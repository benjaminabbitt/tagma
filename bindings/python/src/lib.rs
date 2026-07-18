//! tagma Python bindings: a thin PyO3 wrapper directly over `tagma-core`
//! (PLAN.md §11, Phase 6 / Y1). No C ABI involved — this links tagma-core
//! in-process via PyO3.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use tagma_core::{infix, Index as CoreIndex, Tag};

fn to_value_error(msg: String) -> PyErr {
    PyValueError::new_err(msg)
}

/// An in-memory tagma index: item id -> tags, queryable via infix or
/// postfix (wraps `tagma_core::Index`).
#[pyclass(name = "Index")]
struct Index {
    inner: CoreIndex,
}

#[pymethods]
impl Index {
    /// Creates an empty index.
    #[new]
    fn new() -> Self {
        Index {
            inner: CoreIndex::new(),
        }
    }

    /// Parses and adds a `<id> <tag> <tag>...` line. Raises `ValueError`
    /// on an invalid tag.
    fn add(&mut self, line: &str) -> PyResult<()> {
        self.inner.add_line(line).map_err(to_value_error)
    }

    /// Compiles `q` (infix) and evaluates it, returning sorted matching
    /// ids. Raises `ValueError` on compile or evaluation failure.
    fn query(&self, q: &str) -> PyResult<Vec<String>> {
        self.inner.query(q).map_err(to_value_error)
    }

    /// Evaluates an already-compiled postfix query directly, returning
    /// sorted matching ids. Raises `ValueError` on evaluation failure.
    fn query_postfix(&self, q: &str) -> PyResult<Vec<String>> {
        self.inner.query_postfix(q).map_err(to_value_error)
    }
}

/// Compiles an infix query to its postfix wire form. Raises `ValueError`
/// on a compilation failure.
#[pyfunction]
fn compile(q: &str) -> PyResult<String> {
    infix::compile(q).map_err(to_value_error)
}

/// Parses a single tag string into a dict with keys `namespace`, `key`,
/// `value` (each a `str` or `None`). Raises `ValueError` on an invalid tag.
#[pyfunction]
fn parse_tag(py: Python<'_>, s: &str) -> PyResult<Py<PyDict>> {
    let tag = Tag::parse(s).map_err(to_value_error)?;
    let dict = PyDict::new(py);
    dict.set_item("namespace", tag.namespace)?;
    dict.set_item("key", tag.key)?;
    dict.set_item("value", tag.value)?;
    Ok(dict.into())
}

/// tagma: PyO3 bindings over tagma-core.
#[pymodule]
fn tagma(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Index>()?;
    m.add_function(wrap_pyfunction!(compile, m)?)?;
    m.add_function(wrap_pyfunction!(parse_tag, m)?)?;
    Ok(())
}
