//! 每次遍历独立计费；只记录当前祖先，重复别名仍分别计费和复制。

use std::collections::HashSet;
use std::time::Instant;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString};

use super::{Limits, failure, limit, scalar};

pub(super) struct Walker {
    limits: Limits,
    budget: usize,
    nodes: usize,
    active: HashSet<usize>,
    started: Instant,
    timeout: f64,
    copying: bool,
}

impl Walker {
    pub(super) fn new(
        limits: Limits,
        bytes: Option<usize>,
        timeout: f64,
        copying: bool,
    ) -> PyResult<Self> {
        if !timeout.is_finite() || timeout < 0.0 {
            return Err(PyValueError::new_err(
                "timeout_seconds must be finite and non-negative",
            ));
        }
        Ok(Self {
            limits,
            budget: bytes.unwrap_or(limits.bytes),
            nodes: 0,
            active: HashSet::new(),
            started: Instant::now(),
            timeout,
            copying,
        })
    }

    fn charge(&mut self, bytes: usize) -> PyResult<()> {
        self.budget = self
            .budget
            .checked_sub(bytes)
            .ok_or_else(|| limit("JSON byte limit exceeded"))?;
        Ok(())
    }

    fn check_time(&self) -> PyResult<()> {
        if self.started.elapsed().as_secs_f64() >= self.timeout {
            return Err(failure("PTC_TIMEOUT", "Program wall-clock limit exceeded"));
        }
        Ok(())
    }

    pub(super) fn run<'py>(mut self, value: Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let result = self.visit(value, 0)?;
        // 少量大标量也可能消耗时间，返回前必须再次检查墙钟预算。
        self.check_time()?;
        Ok(result)
    }

    pub(super) fn visit<'py>(
        &mut self,
        value: Bound<'py, PyAny>,
        depth: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.nodes += 1;
        if self.nodes > self.limits.nodes || depth > self.limits.depth {
            return Err(limit("JSON node/depth limit exceeded"));
        }
        // 读取可变 Python 容器时保持 GIL；定期检查时间和信号，不调用任意宿主对象。
        if self.nodes % 64 == 1 {
            self.check_time()?;
            value.py().check_signals()?;
        }
        if value.is_exact_instance_of::<PyString>() {
            let text = value.cast::<PyString>()?;
            if text.len()? > self.limits.string_chars {
                return Err(limit("String limit exceeded"));
            }
            self.charge(scalar::string_bytes(text)?)?;
            return Ok(value);
        }
        if value.is_none() || value.is_exact_instance_of::<PyBool>() {
            self.charge(5)?;
            return Ok(value);
        }
        if value.is_exact_instance_of::<PyInt>() {
            let bits = value.call_method0("bit_length")?.extract::<usize>()?;
            if bits > self.limits.integer_bits {
                return Err(limit("Integer limit exceeded"));
            }
            self.charge(value.str()?.len()?)?;
            return Ok(value);
        }
        if value.is_exact_instance_of::<PyFloat>() {
            if !value.extract::<f64>()?.is_finite() {
                return Err(failure("PTC_VALUE", "Non-finite number is not JSON"));
            }
            // Python 的浮点 repr 与 json.dumps 一致，保留指数和负零的额度语义。
            self.charge(value.repr()?.len()?)?;
            return Ok(value);
        }
        if value.is_exact_instance_of::<PyList>() {
            return self.list(value, depth);
        }
        if value.is_exact_instance_of::<PyDict>() {
            return self.dict(value, depth);
        }
        Err(failure(
            "PTC_VALUE",
            "Only exact JSON primitives are allowed",
        ))
    }

    fn enter(&mut self, value: &Bound<'_, PyAny>, overhead: usize) -> PyResult<usize> {
        let length = value.len()?;
        if length > self.limits.items {
            return Err(limit("Collection limit exceeded"));
        }
        let identity = value.as_ptr() as usize;
        if self.active.contains(&identity) {
            return Err(failure("PTC_VALUE", "Cyclic values are forbidden"));
        }
        let bytes = length
            .checked_mul(overhead)
            .and_then(|size| size.checked_add(2));
        self.charge(bytes.ok_or_else(|| limit("JSON byte limit exceeded"))?)?;
        self.active.insert(identity);
        Ok(identity)
    }

    fn list<'py>(&mut self, value: Bound<'py, PyAny>, depth: usize) -> PyResult<Bound<'py, PyAny>> {
        let identity = self.enter(&value, 2)?;
        let source = value.cast::<PyList>()?;
        let output = if self.copying {
            PyList::empty(value.py())
        } else {
            source.clone()
        };
        for child in source.iter() {
            let copied = self.visit(child, depth + 1)?;
            if self.copying {
                output.append(copied)?;
            }
        }
        self.active.remove(&identity);
        Ok(output.into_any())
    }

    fn dict<'py>(&mut self, value: Bound<'py, PyAny>, depth: usize) -> PyResult<Bound<'py, PyAny>> {
        let identity = self.enter(&value, 4)?;
        let source = value.cast::<PyDict>()?;
        let output = if self.copying {
            PyDict::new(value.py())
        } else {
            source.clone()
        };
        for (key, child) in source.iter() {
            if !key.is_exact_instance_of::<PyString>() {
                return Err(failure("PTC_VALUE", "JSON object keys must be strings"));
            }
            // 对齐 Python 下标赋值的求值顺序：先检查右侧值，再检查键。
            let copied = self.visit(child, depth + 1)?;
            let checked_key = self.visit(key, depth + 1)?;
            if self.copying {
                output.set_item(checked_key, copied)?;
            }
        }
        self.active.remove(&identity);
        Ok(output.into_any())
    }
}
