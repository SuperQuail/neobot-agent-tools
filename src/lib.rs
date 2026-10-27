//! 将文本快照及 PTC 值内核绑定到 Python，不提供文件系统访问或工具调度权限。
#![deny(missing_docs)]

mod core;
mod ptc_values;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use pyo3::exceptions::{PyIndexError, PyMemoryError, PyUnicodeDecodeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::core::{CoreError, Row, SnapshotData};

// 只在 Python 绑定边界沿用已有五元组协议，内核内部使用具名分页结果。
type PythonPage = (Vec<Row>, usize, usize, Option<usize>, Option<usize>);

fn python_error(error: CoreError) -> PyErr {
    match error {
        CoreError::LineIndex => PyIndexError::new_err(error.to_string()),
        CoreError::Allocation => PyMemoryError::new_err(error.to_string()),
        _ => PyValueError::new_err(error.to_string()),
    }
}

#[pyclass(frozen, module = "neobot_agent_tools._native")]
struct Snapshot {
    inner: Arc<SnapshotData>,
}

#[pymethods]
impl Snapshot {
    #[new]
    fn new(py: Python<'_>, data: &Bound<'_, PyBytes>) -> PyResult<Self> {
        let bytes = data.as_bytes();
        // PyBytes 不可变，绑定参数在 detach 期间持有引用；跨越 GIL 边界的仅为只读字节切片。
        let result = py.detach(|| SnapshotData::create(bytes));
        match result {
            Ok(inner) => Ok(Self { inner }),
            Err(CoreError::Utf8(error)) => {
                Err(PyUnicodeDecodeError::new_err_from_utf8(py, bytes, error))
            }
            Err(error) => Err(python_error(error)),
        }
    }

    #[getter]
    fn size(&self) -> usize {
        self.inner.size()
    }
    #[getter]
    fn char_len(&self) -> usize {
        self.inner.characters()
    }
    #[getter]
    fn line_count(&self) -> usize {
        self.inner.line_count()
    }
    #[getter]
    fn memory_size(&self) -> usize {
        self.inner.memory_size()
    }
    #[getter]
    fn text(&self) -> &str {
        self.inner.text()
    }

    fn start(&self, index: usize) -> usize {
        self.inner.start(index)
    }
    #[pyo3(signature = (index, *, keepends=true))]
    fn line(&self, index: usize, keepends: bool) -> PyResult<&str> {
        self.inner.line(index, keepends).map_err(python_error)
    }
    fn read_fragment(
        &self,
        py: Python<'_>,
        index: usize,
        column: usize,
        char_limit: usize,
    ) -> PyResult<(String, usize)> {
        py.detach(|| self.inner.fragment(index, column, char_limit))
            .map_err(python_error)
    }
    fn read_page(
        &self,
        py: Python<'_>,
        offset: usize,
        limit: usize,
        column: usize,
        budget: usize,
    ) -> PyResult<PythonPage> {
        let page = py
            .detach(|| self.inner.page(offset, limit, column, budget))
            .map_err(python_error)?;
        Ok((
            page.rows,
            page.start_character,
            page.end_character,
            page.next_offset,
            page.next_column,
        ))
    }
    fn normalized<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let bytes = py
            .detach(|| self.inner.normalized())
            .map_err(python_error)?;
        Ok(PyBytes::new(py, &bytes))
    }
    fn share(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
    fn shares_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Snapshot>()?;
    ptc_values::register(module)?;
    module.add("API_VERSION", 1)?;
    module.add("VERSION", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
