//! Python types for metadata provenance tracking.
//!
//! Exposes `ObjectMetadata` and `AnnotatedValue` to Python so that hosts can
//! attach and inspect per-value metadata (producers, consumers, tags) when
//! passing data into or receiving data from the Monty interpreter.

use ::monty::ObjectMetadata;
use pyo3::{
    prelude::*,
    types::{PyFrozenSet, PyString},
};

use crate::{convert::py_to_monty, dataclass::DcRegistry};

/// Provenance metadata attached to a value.
///
/// Tracks where a value came from (producers), who may see it (consumers),
/// and classification labels (tags).
///
/// - `producers`: set of source names that contributed to this value
/// - `consumers`: set of allowed consumers, or `None` for universal (no restriction)
/// - `tags`: set of classification labels (e.g. `"pii"`, `"credential"`)
#[pyclass(name = "ObjectMetadata", module = "pydantic_monty._monty", frozen)]
pub struct PyObjectMetadata {
    /// Source names that contributed to this value.
    #[pyo3(get)]
    pub producers: Py<PyFrozenSet>,
    /// Allowed consumer names, or `None` for universal (no restriction).
    #[pyo3(get)]
    pub consumers: Option<Py<PyFrozenSet>>,
    /// Classification labels.
    #[pyo3(get)]
    pub tags: Py<PyFrozenSet>,
}

#[pymethods]
impl PyObjectMetadata {
    #[new]
    #[pyo3(signature = (*, producers=None, consumers=None, tags=None))]
    fn new(
        py: Python<'_>,
        producers: Option<&Bound<'_, PyFrozenSet>>,
        consumers: Option<&Bound<'_, PyFrozenSet>>,
        tags: Option<&Bound<'_, PyFrozenSet>>,
    ) -> PyResult<Self> {
        let producers = match producers {
            Some(fs) => fs.clone().unbind(),
            None => PyFrozenSet::empty(py)?.unbind(),
        };
        let consumers = consumers.map(|fs| fs.clone().unbind());
        let tags = match tags {
            Some(fs) => fs.clone().unbind(),
            None => PyFrozenSet::empty(py)?.unbind(),
        };
        Ok(Self {
            producers,
            consumers,
            tags,
        })
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let producers = self.producers.bind(py).repr()?;
        let tags = self.tags.bind(py).repr()?;
        match &self.consumers {
            Some(c) => {
                let consumers = c.bind(py).repr()?;
                Ok(format!(
                    "ObjectMetadata(producers={producers}, consumers={consumers}, tags={tags})"
                ))
            }
            None => Ok(format!(
                "ObjectMetadata(producers={producers}, consumers=None, tags={tags})"
            )),
        }
    }

    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(false);
        };
        let producers_eq = self.producers.bind(py).eq(other.producers.bind(py))?;
        let tags_eq = self.tags.bind(py).eq(other.tags.bind(py))?;
        let consumers_eq = match (&self.consumers, &other.consumers) {
            (None, None) => true,
            (Some(a), Some(b)) => a.bind(py).eq(b.bind(py))?,
            _ => false,
        };
        Ok(producers_eq && consumers_eq && tags_eq)
    }
}

/// A value paired with provenance metadata.
///
/// Use `AnnotatedValue` to attach metadata when passing inputs to `Monty.run()`
/// or `Monty.start()`, or when resuming a `FunctionSnapshot` with a return value.
#[pyclass(name = "AnnotatedValue", module = "pydantic_monty._monty", frozen)]
pub struct PyAnnotatedValue {
    /// The underlying Python value.
    #[pyo3(get)]
    pub value: Py<PyAny>,
    /// The provenance metadata for this value.
    #[pyo3(get)]
    pub metadata: Py<PyObjectMetadata>,
}

#[pymethods]
impl PyAnnotatedValue {
    #[new]
    fn new(value: Py<PyAny>, metadata: Py<PyObjectMetadata>) -> Self {
        Self { value, metadata }
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let value_repr = self.value.bind(py).repr()?;
        let meta_repr = self.metadata.bind(py).call_method0("__repr__")?;
        Ok(format!("AnnotatedValue({value_repr}, {meta_repr})"))
    }
}

/// Converts a Rust `ObjectMetadata` to a Python `PyObjectMetadata`.
pub fn rust_meta_to_py(py: Python<'_>, meta: &ObjectMetadata) -> PyResult<Py<PyObjectMetadata>> {
    let producers = PyFrozenSet::new(py, meta.producers.iter().map(|s| PyString::new(py, s)))?.unbind();
    let consumers = meta
        .consumers
        .as_ref()
        .map(|c| PyFrozenSet::new(py, c.iter().map(|s| PyString::new(py, s))).map(Bound::unbind))
        .transpose()?;
    let tags = PyFrozenSet::new(py, meta.tags.iter().map(|s| PyString::new(py, s)))?.unbind();
    Py::new(
        py,
        PyObjectMetadata {
            producers,
            consumers,
            tags,
        },
    )
}

/// Converts a Python `PyObjectMetadata` to a Rust `ObjectMetadata`.
pub fn py_meta_to_rust(py: Python<'_>, meta: &PyObjectMetadata) -> PyResult<ObjectMetadata> {
    let producers = meta
        .producers
        .bind(py)
        .iter()
        .map(|item| item.extract::<String>())
        .collect::<PyResult<_>>()?;
    let consumers = meta
        .consumers
        .as_ref()
        .map(|c| {
            c.bind(py)
                .iter()
                .map(|item| item.extract::<String>())
                .collect::<PyResult<_>>()
        })
        .transpose()?;
    let tags = meta
        .tags
        .bind(py)
        .iter()
        .map(|item| item.extract::<String>())
        .collect::<PyResult<_>>()?;
    Ok(ObjectMetadata {
        producers,
        consumers,
        tags,
    })
}

/// Converts a Python input value to an `AnnotatedObject`.
///
/// If the value is an `AnnotatedValue`, extracts the inner value and metadata.
/// Otherwise, converts the value with `py_to_monty` and sets metadata to `None`.
pub fn py_to_annotated(obj: &Bound<'_, PyAny>, dc_registry: &DcRegistry) -> PyResult<::monty::AnnotatedObject> {
    if let Ok(annotated) = obj.extract::<PyRef<'_, PyAnnotatedValue>>() {
        let py = obj.py();
        let value = py_to_monty(annotated.value.bind(py), dc_registry)?;
        let meta_ref = annotated.metadata.borrow(py);
        let rust_meta = py_meta_to_rust(py, &meta_ref)?;
        Ok(::monty::AnnotatedObject::new(value, Some(rust_meta)))
    } else {
        let value = py_to_monty(obj, dc_registry)?;
        Ok(::monty::AnnotatedObject::from(value))
    }
}
