//! Python-visible wrapper for values that cannot be represented as standard
//! Python types during structured print callback output.
//!
//! When the structured print callback encounters a non-serializable Monty value
//! (e.g. iterators, ranges, modules, coroutines, cyclic references), it wraps
//! the type name and repr string in a [`PyNonSerializable`] instance instead of
//! returning a plain Python string. This lets consumers like Tiny Beaver
//! distinguish non-serializable objects from genuine strings via `isinstance()`.

use pyo3::prelude::*;

/// A marker object representing a Monty value that could not be converted to a
/// native Python type.
///
/// Returned by the `structured_print_callback` for values like iterators,
/// ranges, modules, coroutines, and cyclic references. Consumers can use
/// `isinstance(obj, NonSerializable)` to detect these and inspect the
/// `type_name` and `repr` attributes for sanitization or display decisions.
///
/// `str()` and `repr()` both return the original repr string, so the object
/// is backward-compatible in string contexts (e.g. f-strings, logging).
#[pyclass(
    name = "NonSerializable",
    module = "pydantic_monty._monty",
    frozen,
    skip_from_py_object
)]
#[derive(Debug, Clone)]
pub struct PyNonSerializable {
    /// The Python type name of the original value (e.g. `"range"`, `"iterator"`,
    /// `"cycle_list"`).
    #[pyo3(get)]
    pub type_name: String,
    /// The `repr()` string of the original value.
    #[pyo3(get)]
    pub repr: String,
}

#[pymethods]
impl PyNonSerializable {
    #[new]
    fn new(type_name: String, repr: String) -> Self {
        Self { type_name, repr }
    }

    fn __repr__(&self) -> String {
        format!("NonSerializable(type_name='{}', repr='{}')", self.type_name, self.repr)
    }

    fn __str__(&self) -> &str {
        &self.repr
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.type_name == other.type_name && self.repr == other.repr
    }
}
