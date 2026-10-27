//! Python bindings for immutable, file-system independent Agent text snapshots.
#![deny(missing_docs)]

mod core;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use pyo3::exceptions::{PyIndexError, PyMemoryError, PyUnicodeDecodeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::core::{CoreError, Page, SnapshotData};

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
        // PyBytes is immutable and the bound argument owns its reference for the
        // duration of detach. Only its immutable slice crosses this boundary.
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
    ) -> PyResult<Page> {
        py.detach(|| self.inner.page(offset, limit, column, budget))
            .map_err(python_error)
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
    module.add("API_VERSION", 1)?;
    module.add("VERSION", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
