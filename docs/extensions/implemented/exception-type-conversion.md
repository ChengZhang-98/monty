# Fix: Exception Type Conversion in `monty_to_py`

## Summary

`type()` on exception instances now returns the real Python exception class instead of a string.

## Motivation

When Monty code calls `type()` on an exception instance (e.g., `type(ValueError())`), the
result is a `MontyObject::Type(Type::Exception(ExcType))`. The `monty_to_py` conversion in
the Python bindings only handled types with a `builtin_name()` (like `int`, `str`, `list`).
Exception types were not in that mapping, so they fell through to a string fallback, returning
`"<class 'ValueError'>"` instead of the actual `ValueError` class.

This meant Python code consuming Monty results could not use the returned type for
`isinstance()`, `issubclass()`, or identity checks (`result is ValueError`).

## API

No API changes. This is a fix to existing behavior:

```python
m = pydantic_monty.Monty("x = ValueError()\ntype(x)")
result = m.run()

# Before: result == "<class 'ValueError'>"  (a string)
# After:  result is ValueError               (the actual type)
```

## Implementation

### Files Changed

| File | Change |
|------|--------|
| `crates/monty/src/types/type.rs` | Added `Type::as_exception()` method returning `Option<ExcType>` |
| `crates/monty-python/src/exceptions.rs` | Added `exc_type_to_py_type()` mapping every `ExcType` to its Python type object |
| `crates/monty-python/src/convert.rs` | Added branch in `MontyObject::Type` arm to use `exc_type_to_py_type` for exception types |

### Key Design Decisions

- **`Type::as_exception()` accessor**: The `types` module in the `monty` crate is private, so
  external crates (like `monty-python`) cannot import `Type` directly or pattern-match on
  `Type::Exception(exc_type)`. The `as_exception()` method exposes the inner `ExcType` through
  the public API without changing module visibility.

- **Separate `exc_type_to_py_type` function**: Mirrors the existing `exc_monty_to_py` mapping
  but returns the exception **class** (`Bound<'_, PyType>`) rather than creating an exception
  **instance** (`PyErr`). Placed in `exceptions.rs` alongside the existing mapping for
  discoverability.

- **Conversion order**: The new `as_exception()` branch sits between the `builtin_name()` check
  and the string fallback. This preserves existing behavior for all other types.

## Testing

```bash
uv run pytest crates/monty-python/tests/test_types.py::test_return_exception -v
```

## Future Considerations

If new `ExcType` variants are added, they must be added to both `exc_monty_to_py` (for raising)
and `exc_type_to_py_type` (for type conversion). A non-exhaustive match in either function will
cause a compile error, so this is enforced by the compiler.
