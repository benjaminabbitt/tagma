//! tagma Python bindings: a thin PyO3 wrapper directly over `tagma-core`
//! (PLAN.md §11, Phase 6 / Y1). No C ABI involved — this links tagma-core
//! in-process via PyO3.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use tagma_core::{infix, Change, Finding, Index as CoreIndex, Tag};

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

    /// Validates a change to item `id` (SPEC.md §13): the findings for the
    /// tag set the item would carry after removing every tag in `remove`
    /// and then adding every tag in `add` (one tag string per element),
    /// against the index's `tagma.*` config as it stands. Pure: the index
    /// is not modified. Returns a list of finding dicts (see
    /// `finding_dict`), sorted by tag spelling, then code, then args.
    /// Raises `ValueError` on an invalid tag.
    #[pyo3(signature = (id, add = Vec::new(), remove = Vec::new()))]
    fn validate<'py>(
        &self,
        py: Python<'py>,
        id: &str,
        add: Vec<String>,
        remove: Vec<String>,
    ) -> PyResult<Bound<'py, PyList>> {
        let change = Change {
            add: parse_tags(&add).map_err(|e| to_value_error(format!("add: {e}")))?,
            remove: parse_tags(&remove).map_err(|e| to_value_error(format!("remove: {e}")))?,
        };
        findings_list(py, &self.inner.validate(id, &change))
    }

    /// Checks the index's own `tagma.*` config (SPEC.md §14), returning
    /// findings as `validate` does, every `origin` `"existing"`.
    fn check<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        findings_list(py, &self.inner.check())
    }
}

/// Parses each element of `tags` as one tag (SPEC.md §2).
fn parse_tags(tags: &[String]) -> Result<Vec<Tag>, String> {
    tags.iter().map(|t| Tag::parse(t)).collect()
}

/// One finding as a dict: `code` and `origin` (`"introduced"` or
/// `"existing"`) as their stable strings, `tag` in its canonical spelling
/// (SPEC.md §2), and `args` as a list of `(name, value)` tuples in order
/// (a multi-valued argument repeated once per value).
fn finding_dict<'py>(py: Python<'py>, f: &Finding) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("code", f.code.as_str())?;
    dict.set_item("tag", f.tag.to_string())?;
    dict.set_item("args", f.args.clone())?;
    dict.set_item("origin", f.origin.as_str())?;
    Ok(dict)
}

/// Findings as a list of `finding_dict`s, order kept.
fn findings_list<'py>(py: Python<'py>, findings: &[Finding]) -> PyResult<Bound<'py, PyList>> {
    let list = PyList::empty(py);
    for f in findings {
        list.append(finding_dict(py, f)?)?;
    }
    Ok(list)
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
