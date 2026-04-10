# Reusable Extension Patterns

Patterns proven to minimize merge conflicts with upstream Monty.

## Marker Pattern (threading data through snapshots) {#marker-pattern}

**Problem**: You need to thread custom data through the snapshot/resume chain,
but changing `print_callback: Option<Py<PyAny>>` in ~20 locations causes
massive merge conflicts.

**Solution**: Wrap your data in an internal `#[pyclass]` and pass it through the
existing `Option<Py<PyAny>>` field. Detect and unwrap it at usage points.

### Implementation

```rust
// 1. Define a marker pyclass (internal only, not exported to Python)
#[pyclass]
pub(crate) struct MyMarker {
    pub real_callback: Py<PyAny>,
    pub extra_data: MyExtraData,
}

// 2. Wrap at entry points
pub(crate) fn wrap_my_callback(
    py: Python<'_>,
    callback: Py<PyAny>,
    extra: MyExtraData,
) -> PyResult<Py<PyAny>> {
    let marker = MyMarker {
        real_callback: callback,
        extra_data: extra,
    };
    Ok(Py::new(py, marker)?.into_any())
}

// 3. Detect and unwrap at usage points
pub(crate) fn unwrap_my_callback(
    py: Python<'_>,
    cb: &Py<PyAny>,
) -> Option<(Py<PyAny>, MyExtraData)> {
    let bound = cb.bind(py);
    if let Ok(marker) = bound.cast::<MyMarker>() {
        let borrowed = marker.borrow();
        Some((
            borrowed.real_callback.clone_ref(py),
            borrowed.extra_data.clone_ref(py),
        ))
    } else {
        None
    }
}
```

### Usage at entry points

```rust
fn feed_start(..., my_callback: Option<Py<PyAny>>) {
    // Wrap in marker before passing to the snapshot chain
    let print_callback = if let Some(cb) = my_callback {
        Some(wrap_my_callback(py, cb, extra_data)?)
    } else {
        None
    };
    // print_callback is now Option<Py<PyAny>> — fits existing chain
}
```

### Usage at with_print_writer

```rust
pub(crate) fn with_print_writer<R>(
    print_callback: Option<Py<PyAny>>,
    f: impl FnOnce(PrintWriter<'_>) -> R,
) -> R {
    match print_callback {
        Some(cb) => {
            let my_data = Python::attach(|py| unwrap_my_callback(py, &cb));
            if let Some((real_cb, extra)) = my_data {
                // Create your custom callback
                let mut my_cb = MyCustomCallback::new(real_cb, extra);
                f(PrintWriter::Callback(&mut my_cb))
            } else {
                // Existing string callback
                let mut print_cb = CallbackStringPrint::from_py(cb);
                f(PrintWriter::Callback(&mut print_cb))
            }
        }
        None => f(PrintWriter::Stdout),
    }
}
```

**Result**: Zero changes to snapshot class signatures. The marker travels as a
regular `Py<PyAny>` through the entire chain.

## Mutual Exclusion Pattern (conflicting parameters)

**Problem**: Two parameters can't be used together.

**Solution**: Validate at entry points with `resolve_*` functions.

```rust
pub(crate) fn resolve_print_callback(
    py: Python<'_>,
    print_callback: Option<Py<PyAny>>,
    structured_print_callback: Option<Py<PyAny>>,
    dc_registry: &DcRegistry,
) -> PyResult<Option<Py<PyAny>>> {
    if print_callback.is_some() && structured_print_callback.is_some() {
        return Err(PyValueError::new_err(
            "cannot specify both 'print_callback' and 'structured_print_callback'",
        ));
    }
    if let Some(cb) = structured_print_callback {
        // Wrap in marker for the chain
        Ok(Some(wrap_structured_callback(py, cb, dc_registry.clone_ref(py))?))
    } else {
        Ok(print_callback)
    }
}
```

Call this at the top of every entry point method.

## Default Trait Method Pattern (backwards-compatible VM changes)

**Problem**: You need to add behavior to a core trait without breaking existing
implementations.

**Solution**: Add methods with default implementations.

```rust
pub trait PrintWriterCallback {
    // Existing required methods (don't change these)
    fn stdout_write(&mut self, output: Cow<str>) -> Result<(), MontyException>;
    fn stdout_push(&mut self, end: char) -> Result<(), MontyException>;

    // New opt-in capability with defaults
    fn wants_structured(&self) -> bool { false }
    fn stdout_write_structured(
        &mut self,
        objects: Vec<MontyObject>,
        sep: &str,
        end: &str,
    ) -> Result<(), MontyException> {
        // Default: fall back to string-based output
        for (i, obj) in objects.iter().enumerate() {
            if i > 0 { self.stdout_write(Cow::Borrowed(sep))?; }
            self.stdout_write(Cow::Owned(obj.to_string()))?;
        }
        self.stdout_write(Cow::Borrowed(end))
    }
}
```

Then in the builtin implementation, check the capability:
```rust
if vm.print_writer.wants_structured() {
    // new path
} else {
    // existing path (unchanged)
}
```

## Sync Entry Point Pattern (creating PrintWriter without snapshots)

For synchronous methods that don't go through the snapshot chain
(`Monty.run`, `MontyRepl.feed_run`), create the callback directly:

```rust
fn run(..., my_param: Option<&Bound<'_, PyAny>>) {
    let mut string_cb;
    let mut structured_cb;
    let print_writer = if let Some(cb) = structured_param {
        structured_cb = CallbackStructuredPrint::new(cb, dc_registry);
        PrintWriter::Callback(&mut structured_cb)
    } else if let Some(cb) = print_callback {
        string_cb = CallbackStringPrint::new(cb);
        PrintWriter::Callback(&mut string_cb)
    } else {
        PrintWriter::Stdout
    };
    // use print_writer...
}
```

Note: both `_cb` variables must be declared before the `if` chain so they
live long enough. Only one is initialized, but both must be in scope.

## Helper Function Pattern (shared across monty_cls.rs and repl.rs)

Define shared helpers in `monty_cls.rs` as `pub(crate)` and import in `repl.rs`:

```rust
// monty_cls.rs
pub(crate) fn resolve_print_callback(...) -> PyResult<...> { ... }
pub(crate) fn wrap_structured_callback(...) -> PyResult<...> { ... }
pub(crate) fn unwrap_structured_callback(...) -> Option<...> { ... }

// repl.rs
use crate::monty_cls::{resolve_print_callback, unwrap_structured_callback, ...};
```

## Conflict-Risk Summary

| Change type | Conflict risk | Recommendation |
|-------------|---------------|----------------|
| New file | None | Always safe |
| Default trait method | Very low | Preferred for VM changes |
| New `pub(crate)` function | Very low | Preferred for helpers |
| New parameter on entry points | Low | Required for new features |
| Change `with_print_writer` | Low-medium | Contains detection logic |
| Change snapshot field types | **High** | Avoid (use marker pattern) |
| Change existing method signatures | **High** | Avoid if possible |
| Change `builtin_print` loop body | Medium | May conflict if upstream modifies print |
