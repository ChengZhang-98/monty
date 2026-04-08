# How to Implement an Extension

Step-by-step guide for adding a new feature to Monty. This assumes you've read
[architecture.md](architecture.md).

## Before You Start

1. Create a feature branch: `git checkout tiny-beaver-ext && git checkout -b feature/my-thing`
2. Read the existing extension docs in [implemented/](implemented/) for prior art
3. Identify which layers your change touches (see architecture.md)

## Step 1: Core VM Changes (if needed)

If your extension requires changes to the core VM (`crates/monty/src/`):

**Prefer additive changes:**
- Add default methods to existing traits (backwards compatible)
- Add new `pub(crate)` methods to existing types
- Make private methods `pub(crate)` if needed from other modules

**Example**: Adding a default trait method
```rust
// io.rs
pub trait PrintWriterCallback {
    // existing methods unchanged...

    fn my_new_capability(&self) -> bool { false }  // default = opt-in
}
```

**Run after changes:**
```bash
make format-rs && make lint-rs
```

## Step 2: Python Binding Implementation

Create new structs/callbacks in `crates/monty-python/src/monty_cls.rs` (or a
new module if large enough).

**If your extension needs data threaded through the snapshot chain**, use the
marker pattern from [patterns.md](patterns.md#marker-pattern).

**If it's a simple parameter that doesn't affect snapshots**, just add it to
the entry point methods and handle it locally.

**Run after changes:**
```bash
cargo check -p pydantic-monty
```

## Step 3: Add Parameter to Python API

Update all six entry point methods:

| Method | File |
|--------|------|
| `Monty.run` | `monty_cls.rs` |
| `Monty.start` | `monty_cls.rs` |
| `Monty.run_async` | `monty_cls.rs` |
| `MontyRepl.feed_run` | `repl.rs` |
| `MontyRepl.feed_start` | `repl.rs` |
| `MontyRepl.feed_run_async` | `repl.rs` |

For each method, update:

1. **`#[pyo3(signature = (...))]`** - add the new parameter with default
2. **Function parameters** - add the Rust parameter
3. **Validation logic** - at the top of the function body
4. **`#[expect(clippy::too_many_arguments)]`** - add if over 7 params

Example pattern:
```rust
#[pyo3(signature = (..., my_new_param=None))]
fn feed_start(
    ...,
    my_new_param: Option<Py<PyAny>>,
) -> PyResult<...> {
    // Validate early
    let resolved = resolve_my_param(py, my_new_param, ...)?;
    // ... rest of method uses resolved
}
```

## Step 4: Update with_print_writer (if applicable)

If your extension changes how callbacks are created from `Option<Py<PyAny>>`,
update `with_print_writer` in `crates/monty-python/src/async_dispatch.rs`.

This function is the bridge between the `Option<Py<PyAny>>` stored in snapshots
and the actual `PrintWriter` used during execution.

## Step 5: Update Type Stubs

Edit `crates/monty-python/python/pydantic_monty/_monty.pyi`:

- Add the new parameter to all relevant method signatures
- **Don't** add module-level type aliases (stubtest rejects them)
- Inline complex types instead

## Step 6: Add Tests

Add tests to `crates/monty-python/tests/`. Follow existing style:

```python
import pytest
from inline_snapshot import snapshot
import pydantic_monty

def test_my_feature_basic() -> None:
    m = pydantic_monty.Monty('...')
    result = m.run(my_new_param=...)
    assert result == snapshot()  # inline-snapshot fills this in

def test_my_feature_error_case() -> None:
    m = pydantic_monty.Monty('...')
    with pytest.raises(ValueError, match='expected message'):
        m.run(my_new_param=bad_value)
```

Rules:
- Use `snapshot()` for assertions (run tests once to fill values)
- Use `pytest.raises` for expected exceptions
- No class-based tests
- Test both `Monty` and `MontyRepl` if applicable

## Step 7: Lint and Test Everything

```bash
make format-rs          # format Rust
make lint-rs            # clippy + import checks (must pass clean)
make dev-py             # build Python package
make pytest             # run Python tests (must all pass)
make lint-py            # ruff + pyright + stubtest (must pass clean)
```

## Step 8: Document the Extension

Copy [_template.md](_template.md) to `implemented/my-extension.md` and fill it
in. Update the table in [README.md](README.md).

## Step 9: Commit and Merge

```bash
git add <files>
git commit -m "Add my-extension: brief description"
git checkout tiny-beaver-ext
git merge feature/my-thing
```

## Common Pitfalls

### Clippy errors after adding parameters
- `too_many_arguments` → add `#[expect(clippy::too_many_arguments)]`
- `needless_pass_by_value` → use `&Bound<'_, PyAny>` or `Py<PyAny>` consistently
- `ref_option` → use `Option<&T>` not `&Option<T>`
- `must_use_candidate` → add `#[must_use]` to pure query methods

### Stubtest failures
- Don't add type aliases to `.pyi` files (they must exist at runtime)
- Every parameter in `.pyi` must match the Rust `#[pyo3(signature)]` exactly

### Datatest runner crashes
The `datatest_runner` may crash with "Failed to import encodings module" if the
Python environment isn't set up. This is pre-existing and unrelated to your
changes. Use `make pytest` for Python tests instead.
