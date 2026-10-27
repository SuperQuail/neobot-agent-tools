//! 对精确 JSON 类型执行有界复查和隔离复制，不承载工具权限或解释器状态。

mod scalar;
mod walk;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

pyo3::create_exception!(
    _native,
    PtcValueError,
    PyValueError,
    "有界 JSON 值处理失败。"
);

fn failure(code: &str, message: &str) -> PyErr {
    PtcValueError::new_err((code.to_owned(), message.to_owned()))
}

fn limit(message: &str) -> PyErr {
    failure("PTC_LIMIT", message)
}

#[derive(Clone, Copy)]
struct Limits {
    nodes: usize,
    depth: usize,
    items: usize,
    string_chars: usize,
    integer_bits: usize,
    bytes: usize,
}

#[pyclass(frozen, module = "neobot_agent_tools._native")]
struct PtcValueKernel {
    limits: Limits,
}

#[pymethods]
impl PtcValueKernel {
    #[new]
    #[pyo3(signature = (*, nodes, depth, items, string_chars, integer_bits, bytes))]
    fn new(
        nodes: usize,
        depth: usize,
        items: usize,
        string_chars: usize,
        integer_bits: usize,
        bytes: usize,
    ) -> PyResult<Self> {
        let limits = Limits {
            nodes,
            depth,
            items,
            string_chars,
            integer_bits,
            bytes,
        };
        if [nodes, depth, items, string_chars, integer_bits, bytes].contains(&0) {
            return Err(PyValueError::new_err("Value limits must be positive"));
        }
        // 宿主对更深的自定义配置使用 Python 回退，避免原生递归耗尽线程栈。
        if depth > 128 {
            return Err(PyValueError::new_err(
                "Native value depth must not exceed 128",
            ));
        }
        Ok(Self { limits })
    }

    #[pyo3(signature = (value, *, byte_limit=None, timeout_seconds=30.0))]
    fn copy<'py>(
        &self,
        value: Bound<'py, PyAny>,
        byte_limit: Option<usize>,
        timeout_seconds: f64,
    ) -> PyResult<Bound<'py, PyAny>> {
        walk::Walker::new(self.limits, byte_limit, timeout_seconds, true)?.run(value)
    }

    #[pyo3(signature = (value, *, byte_limit=None, timeout_seconds=30.0))]
    fn validate(
        &self,
        value: Bound<'_, PyAny>,
        byte_limit: Option<usize>,
        timeout_seconds: f64,
    ) -> PyResult<()> {
        walk::Walker::new(self.limits, byte_limit, timeout_seconds, false)?.run(value)?;
        Ok(())
    }
}

pub(super) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PtcValueKernel>()?;
    module.add("PtcValueError", module.py().get_type::<PtcValueError>())?;
    module.add("PTC_VALUE_API_VERSION", 1)?;
    Ok(())
}
