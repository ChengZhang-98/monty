//! Python types for metadata provenance tracking.
//!
//! Exposes `ObjectMetadata`, `AnnotatedValue`, `UniversalSet`, and the
//! `UNIVERSAL` singleton to Python so that hosts can attach and inspect
//! per-value metadata (producers, consumers, tags) when passing data into
//! or receiving data from the Monty interpreter.

use std::collections::BTreeSet;

use ::monty::ObjectMetadata;
use pyo3::{
    exceptions::{PyTypeError, PyValueError},
    prelude::*,
    sync::PyOnceLock,
    types::{PyFrozenSet, PyString},
};

use crate::{convert::py_to_monty, dataclass::DcRegistry};

// ---------------------------------------------------------------------------
// UniversalSet — the UNIVERSAL singleton type
// ---------------------------------------------------------------------------

/// The universal set — contains every element.
///
/// Used as a metadata field value to indicate "no restrictions" (for consumers)
/// or "all sources/labels" (for producers/tags). Supports `in` membership
/// checks (always `True`) but cannot be iterated or measured.
///
/// Access the singleton via the module-level `UNIVERSAL` constant.
#[pyclass(name = "UniversalSet", module = "pydantic_monty._monty", frozen)]
pub struct PyUniversalSet;

#[pymethods]
#[expect(clippy::unused_self, reason = "PyO3 requires &self for dunder methods")]
impl PyUniversalSet {
    fn __repr__(&self) -> &'static str {
        "UNIVERSAL"
    }

    fn __bool__(&self) -> bool {
        true
    }

    /// Membership test — the universal set contains everything.
    fn __contains__(&self, _item: &Bound<'_, PyAny>) -> bool {
        true
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other.is_instance_of::<Self>()
    }

    fn __hash__(&self) -> u64 {
        // Fixed hash for the singleton — all instances compare equal.
        0x756E_6976_6572_7361 // "universa" as u64
    }

    fn __iter__(&self) -> PyResult<()> {
        Err(PyTypeError::new_err(
            "cannot iterate over UNIVERSAL — it represents the infinite universal set",
        ))
    }

    fn __len__(&self) -> PyResult<usize> {
        Err(PyTypeError::new_err("UNIVERSAL has no finite length"))
    }
}

/// Returns the `UNIVERSAL` singleton, creating it on first call.
///
/// The singleton is cached in a `PyOnceLock` so that identity checks
/// (`meta.consumers is UNIVERSAL`) work correctly across the entire process.
pub fn universal_singleton(py: Python<'_>) -> PyResult<Py<PyUniversalSet>> {
    static INSTANCE: PyOnceLock<Py<PyUniversalSet>> = PyOnceLock::new();
    Ok(INSTANCE
        .get_or_try_init(py, || Py::new(py, PyUniversalSet))?
        .clone_ref(py))
}

// ---------------------------------------------------------------------------
// Helper — validate and parse a metadata field
// ---------------------------------------------------------------------------

/// Validates that a frozenset of strings contains no empty strings.
fn validate_no_empty_strings(fs: &Bound<'_, PyFrozenSet>, field_name: &str) -> PyResult<()> {
    for item in fs.iter() {
        let s = item.extract::<String>()?;
        if s.is_empty() {
            return Err(PyValueError::new_err(format!(
                "{field_name} must not contain empty strings"
            )));
        }
    }
    Ok(())
}

/// Parses a metadata field value from the constructor argument.
///
/// Accepted inputs:
/// - `None` (Rust `Option::None`, meaning the Python arg was omitted) → applies the default
/// - `UniversalSet` → returns the `UNIVERSAL` singleton as `Py<PyAny>`
/// - `frozenset[str]` → validates and returns the frozenset as `Py<PyAny>`
///
/// Anything else raises `TypeError`.
fn parse_metadata_field(
    py: Python<'_>,
    value: Option<&Bound<'_, PyAny>>,
    field_name: &str,
    default_fn: impl FnOnce(Python<'_>) -> PyResult<Py<PyAny>>,
) -> PyResult<Py<PyAny>> {
    let Some(val) = value else {
        return default_fn(py);
    };
    if val.is_instance_of::<PyUniversalSet>() {
        return Ok(universal_singleton(py)?.into_any());
    }
    if let Ok(fs) = val.cast::<PyFrozenSet>() {
        validate_no_empty_strings(fs, field_name)?;
        return Ok(fs.clone().unbind().into_any());
    }
    Err(PyTypeError::new_err(format!(
        "{field_name} must be a frozenset[str] or UNIVERSAL, got {type_name}",
        type_name = val.get_type().qualname()?
    )))
}

/// Returns a `Py<PyAny>` that is an empty frozenset.
fn empty_frozenset_any(py: Python<'_>) -> PyResult<Py<PyAny>> {
    Ok(PyFrozenSet::empty(py)?.unbind().into_any())
}

/// Returns a `Py<PyAny>` that is the UNIVERSAL singleton.
fn universal_any(py: Python<'_>) -> PyResult<Py<PyAny>> {
    Ok(universal_singleton(py)?.into_any())
}

// ---------------------------------------------------------------------------
// Helper — convert a Py<PyAny> field to repr / eq
// ---------------------------------------------------------------------------

/// Returns the repr string for a metadata field value.
///
/// - If the value is a `UniversalSet`, returns `"UNIVERSAL"`.
/// - Otherwise returns the frozenset's repr.
fn field_repr(py: Python<'_>, field: &Py<PyAny>) -> PyResult<String> {
    if field.bind(py).is_instance_of::<PyUniversalSet>() {
        Ok("UNIVERSAL".to_owned())
    } else {
        Ok(field.bind(py).repr()?.to_string())
    }
}

/// Checks equality of two metadata field values.
///
/// Two `UNIVERSAL` values are equal. Two frozensets are compared by value.
/// A `UNIVERSAL` vs frozenset is never equal.
fn field_eq(py: Python<'_>, a: &Py<PyAny>, b: &Py<PyAny>) -> PyResult<bool> {
    let a_univ = a.bind(py).is_instance_of::<PyUniversalSet>();
    let b_univ = b.bind(py).is_instance_of::<PyUniversalSet>();
    match (a_univ, b_univ) {
        (true, true) => Ok(true),
        (false, false) => a.bind(py).eq(b.bind(py)),
        _ => Ok(false),
    }
}

// ---------------------------------------------------------------------------
// PyObjectMetadata
// ---------------------------------------------------------------------------

/// Provenance metadata attached to a value.
///
/// Tracks where a value came from (producers), who may see it (consumers),
/// and classification labels (tags). Each field is either a `frozenset[str]`
/// (explicit set) or `UNIVERSAL` (the infinite universal set).
///
/// - `producers`: accumulates via **union** — `UNIVERSAL` means "every source"
/// - `consumers`: restricts via **intersection** — `UNIVERSAL` means "no restrictions"
/// - `tags`: accumulates via **union** — `UNIVERSAL` means "every label"
#[pyclass(name = "ObjectMetadata", module = "pydantic_monty._monty", frozen)]
pub struct PyObjectMetadata {
    /// Source names that contributed to this value, or `UNIVERSAL`.
    #[pyo3(get)]
    pub producers: Py<PyAny>,
    /// Allowed consumer names, or `UNIVERSAL` (no restriction).
    #[pyo3(get)]
    pub consumers: Py<PyAny>,
    /// Classification labels, or `UNIVERSAL`.
    #[pyo3(get)]
    pub tags: Py<PyAny>,
}

#[pymethods]
impl PyObjectMetadata {
    /// Construct a new `ObjectMetadata`.
    ///
    /// Each field accepts `frozenset[str]` or `UNIVERSAL`. Omitting a field
    /// uses the default:
    /// - `producers`: defaults to `frozenset()` (empty set)
    /// - `consumers`: defaults to `UNIVERSAL` (no restriction)
    /// - `tags`: defaults to `frozenset()` (empty set)
    #[new]
    #[pyo3(signature = (*, producers=None, consumers=None, tags=None))]
    pub(crate) fn new(
        py: Python<'_>,
        producers: Option<&Bound<'_, PyAny>>,
        consumers: Option<&Bound<'_, PyAny>>,
        tags: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let producers = parse_metadata_field(py, producers, "producers", empty_frozenset_any)?;
        let consumers = parse_metadata_field(py, consumers, "consumers", universal_any)?;
        let tags = parse_metadata_field(py, tags, "tags", empty_frozenset_any)?;
        Ok(Self {
            producers,
            consumers,
            tags,
        })
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let producers = field_repr(py, &self.producers)?;
        let consumers = field_repr(py, &self.consumers)?;
        let tags = field_repr(py, &self.tags)?;
        Ok(format!(
            "ObjectMetadata(producers={producers}, consumers={consumers}, tags={tags})"
        ))
    }

    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(false);
        };
        let producers_eq = field_eq(py, &self.producers, &other.producers)?;
        let consumers_eq = field_eq(py, &self.consumers, &other.consumers)?;
        let tags_eq = field_eq(py, &self.tags, &other.tags)?;
        Ok(producers_eq && consumers_eq && tags_eq)
    }
}

// ---------------------------------------------------------------------------
// PyAnnotatedValue
// ---------------------------------------------------------------------------

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
    pub(crate) fn new(value: Py<PyAny>, metadata: Py<PyObjectMetadata>) -> Self {
        Self { value, metadata }
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let value_repr = self.value.bind(py).repr()?;
        let meta_repr = self.metadata.bind(py).call_method0("__repr__")?;
        Ok(format!("AnnotatedValue({value_repr}, {meta_repr})"))
    }
}

// ---------------------------------------------------------------------------
// Rust ↔ Python conversion helpers
// ---------------------------------------------------------------------------

/// Converts an `Option<BTreeSet<String>>` metadata field to a `Py<PyAny>`.
///
/// `None` → `UNIVERSAL` singleton, `Some(set)` → `PyFrozenSet`.
fn option_set_to_py(py: Python<'_>, field: Option<&BTreeSet<String>>) -> PyResult<Py<PyAny>> {
    match field {
        None => Ok(universal_singleton(py)?.into_any()),
        Some(set) => {
            let fs = PyFrozenSet::new(py, set.iter().map(|s| PyString::new(py, s)))?;
            Ok(fs.unbind().into_any())
        }
    }
}

/// Converts a `Py<PyAny>` metadata field to an `Option<BTreeSet<String>>`.
///
/// `UniversalSet` → `None`, `frozenset` → `Some(set)`.
fn py_field_to_option_set(py: Python<'_>, field: &Py<PyAny>) -> PyResult<Option<BTreeSet<String>>> {
    if field.bind(py).is_instance_of::<PyUniversalSet>() {
        return Ok(None);
    }
    let fs: &Bound<'_, PyFrozenSet> = field.bind(py).cast()?;
    let set = fs
        .iter()
        .map(|item| item.extract::<String>())
        .collect::<PyResult<_>>()?;
    Ok(Some(set))
}

/// Converts a Rust [`ObjectMetadata`] to a Python [`PyObjectMetadata`].
///
/// Each `None` field (universal) becomes the `UNIVERSAL` singleton; each
/// `Some(set)` becomes a `PyFrozenSet`.
pub fn rust_meta_to_py(py: Python<'_>, meta: &ObjectMetadata) -> PyResult<Py<PyObjectMetadata>> {
    let producers = option_set_to_py(py, meta.producers.as_ref())?;
    let consumers = option_set_to_py(py, meta.consumers.as_ref())?;
    let tags = option_set_to_py(py, meta.tags.as_ref())?;
    Py::new(
        py,
        PyObjectMetadata {
            producers,
            consumers,
            tags,
        },
    )
}

/// Converts a Python [`PyObjectMetadata`] to a Rust [`ObjectMetadata`].
///
/// `UNIVERSAL` fields become `None`; frozenset fields become `Some(BTreeSet)`.
pub fn py_meta_to_rust(py: Python<'_>, meta: &PyObjectMetadata) -> PyResult<ObjectMetadata> {
    let producers = py_field_to_option_set(py, &meta.producers)?;
    let consumers = py_field_to_option_set(py, &meta.consumers)?;
    let tags = py_field_to_option_set(py, &meta.tags)?;
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
