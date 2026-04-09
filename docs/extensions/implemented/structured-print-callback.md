# Extension: Structured Print Callback

## Summary

Receive `print()` arguments as native Python objects instead of string fragments.

## Motivation

Tiny Beaver has a **visibility/sanitization system** (SAFE vs TAINTED modes)
that controls how external data flows back to the Planning LLM. When sandboxed
code calls `print()`, the output may contain external data (e.g., web page
content, file contents) that needs sanitization before being included in the
PLLM conversation.

The existing `print_callback` receives `str(obj)` for each argument, losing
type information. Sanitization decisions depend on the structure and type of
the printed objects — for example, a plain string from `web_fetch` needs
different handling than an integer or a list of redacted fields. This requires
the original typed objects, not their string representations.

## API

### New Parameters

```python
result = monty.run(
    ...,
    structured_print_callback=my_handler,
)

# Also available on:
# monty.start(..., structured_print_callback=...)
# monty.run_async(..., structured_print_callback=...)
# repl.feed_run(..., structured_print_callback=...)
# repl.feed_start(..., structured_print_callback=...)
# repl.feed_run_async(..., structured_print_callback=...)
```

### Callback Signature

```python
def my_handler(
    stream: Literal['stdout'],
    objects: list[Any],
    sep: str,
    end: str,
) -> None:
    """
    Called once per print() invocation.

    Args:
        stream: Always 'stdout'
        objects: All positional args as native Python objects.
            JSON-serializable types (int, str, float, bool, None, list, dict,
            tuple) are passed as-is. Non-serializable types (functions,
            iterators, ranges, cyclic references, etc.) are wrapped in
            `NonSerializable(type_name, repr)` objects. Use
            `isinstance(obj, NonSerializable)` to detect them.
        sep: Separator between arguments (default ' ')
        end: String appended after last argument (default '\n')
    """
```

### Behavior

- **Provided**: Each `print()` call triggers one callback invocation with all
  arguments as a list of typed Python objects
- **Not provided**: Falls back to existing behavior (no callback, or string
  `print_callback` if that's set)
- **Both provided**: Raises `ValueError("cannot specify both 'print_callback'
  and 'structured_print_callback'")`

### Examples

```python
from pydantic_monty import Monty, NonSerializable

calls = []
def handler(stream, objects, sep, end):
    calls.append((objects, sep, end))

m = Monty('print(1, "hello", [1, 2], sep="-")')
m.run(structured_print_callback=handler)
# calls == [([1, "hello", [1, 2]], "-", "\n")]
#           ^^^^^^^^^^^^^^^^^^^ native types, not strings

# Non-serializable types become NonSerializable objects:
m2 = Monty('print(range(5))')
m2.run(structured_print_callback=handler)
obj = calls[-1][0][0]
assert isinstance(obj, NonSerializable)
assert obj.type_name == 'range'
assert obj.repr == 'range(0, 5)'
assert str(obj) == 'range(0, 5)'  # backward-compatible in string contexts
```

## Implementation

### Files Changed

| File | Change |
|------|--------|
| `crates/monty/src/io.rs` | Added `wants_structured()` and `stdout_write_structured()` default methods to `PrintWriterCallback` trait and `PrintWriter` enum |
| `crates/monty/src/builtins/print.rs` | Branch: if `wants_structured()`, convert Values to MontyObjects and call `stdout_write_structured` once; else existing string path |
| `crates/monty/src/object.rs` | Made `MontyObject::from_value` `pub(crate)` (was private); changed `Repr(String)` to `Repr { type_name, repr }` |
| `crates/monty-python/src/non_serializable.rs` | New file: `NonSerializable` pyclass with `type_name` and `repr` fields |
| `crates/monty-python/src/convert.rs` | Added `monty_to_py_structured()` that wraps `Repr`/`Cycle`/`Function`/non-builtin `Type` in `NonSerializable`; `monty_to_py()` falls back to string repr for non-builtin types |
| `crates/monty-python/src/monty_cls.rs` | Added `StructuredCallbackMarker` pyclass, `CallbackStructuredPrint` struct, `resolve_print_callback()`, `wrap/unwrap_structured_callback()` |
| `crates/monty-python/src/async_dispatch.rs` | Updated `with_print_writer` to detect marker and create `CallbackStructuredPrint` |
| `crates/monty-python/src/repl.rs` | Added `structured_print_callback` param to `feed_run`, `feed_start`, `feed_run_async`; added `make_print_writer_from_callback` helper |
| `crates/monty-python/python/pydantic_monty/_monty.pyi` | Added parameter to all method signatures |
| `crates/monty-python/tests/test_print.py` | 12 new tests |

### Patterns Used

1. **[Marker Pattern](../patterns.md#marker-pattern)**: `StructuredCallbackMarker`
   wraps the real callback + `DcRegistry` so it can travel through the snapshot
   chain as a regular `Py<PyAny>`. Zero changes to snapshot class signatures.

2. **Default Trait Method Pattern**: `wants_structured()` and
   `stdout_write_structured()` are default methods on `PrintWriterCallback`,
   so existing implementations don't need changes.

3. **Mutual Exclusion Pattern**: `resolve_print_callback()` validates that
   only one of `print_callback`/`structured_print_callback` is provided.

### Key Design Decisions

1. **One call per `print()` vs one call per argument**: We chose one call per
   `print()` with all args as a list. This gives the callback full context for
   sanitization decisions across all arguments.

2. **sep/end as parameters**: Passed through to the callback so it can
   reconstruct the full output if needed, or ignore them.

3. **MontyObject → Python conversion**: Uses `monty_to_py_structured()`, which
   delegates to `monty_to_py()` for serializable types but wraps `Repr`, `Cycle`,
   `Function`, and non-builtin `Type` variants in
   `NonSerializable(type_name, repr)` objects instead of plain strings. This lets
   consumers use `isinstance(obj, NonSerializable)` to distinguish
   non-serializable values. The `type_name` field carries the Python type name
   (e.g. `"range"`, `"iterator"`, `"cycle_list"`, `"type"`), and the `repr`
   field carries the repr string (e.g. `"<class 'dataclass'>"`). `str(obj)`
   returns the repr for backward compatibility. The general `monty_to_py()` path
   (used by `run()` return values etc.) still converts these to plain strings.
   For `MontyObject::Type`, builtin types (e.g. `int`, `str`) are looked up from
   Python's `builtins` module, while non-builtin types (e.g. `Dataclass`,
   `DateTime`) return a `"<class 'name'>"` string.

4. **DcRegistry in marker**: The structured callback needs `DcRegistry` to
   convert dataclass MontyObjects back to proper Python dataclass instances.
   Bundling it in the marker avoids adding a parameter to `with_print_writer`.

## Testing

```bash
make dev-py
uv run python -m pytest crates/monty-python/tests/test_print.py -v -k "structured or non_serializable"
```

Tests cover:
- Basic types (int, str, float, bool, None)
- Nested containers (list, dict, tuple)
- Custom sep/end
- Empty print()
- Multiple print() calls
- Non-serializable fallback (range → `NonSerializable`)
- `isinstance()` detection of `NonSerializable` alongside native types
- `NonSerializable` equality comparison
- Iterator → `NonSerializable` with `type_name='iterator'`
- Both-callbacks error
- MontyRepl.feed_run
- MontyRepl.feed_start
- `type()` on dataclass instance → `NonSerializable` (regression test)

## Commits

| Hash | Description |
|------|-------------|
| `caeeeaf` | Add structured_print_callback for typed print output |
| `8f00eae` | Document structured_print_callback in all .pyi docstrings |

## Known Issues (Fixed)

### `StructuredCallbackMarker` not callable after resume

**Bug**: When `structured_print_callback` was passed to `feed_start()`, calling
`print()` with non-literal arguments (e.g. f-strings with variables) after
`resume()` raised `TypeError: 'builtins.StructuredCallbackMarker' object is not
callable`.

**Root cause**: The `resume()` methods on `PyFunctionSnapshot`,
`PyNameLookupSnapshot`, and `PyFutureSnapshot` unconditionally created
`CallbackStringPrint` from the stored callback, ignoring that it might be a
`StructuredCallbackMarker`. The marker was passed directly as a Python callable,
which failed.

**Fix**: All three `resume()` methods now check `unwrap_structured_callback()`
first, matching the pattern already used in `run()`/`start()` and
`with_print_writer()`.

### Non-builtin `Type` crashes `monty_to_py`

**Bug**: Printing `type()` of a dataclass instance (or any non-builtin type like
`DateTime`) via `structured_print_callback` raised `AttributeError` because
`monty_to_py()` unconditionally looked up the type name in Python's `builtins`
module. Non-builtin types like `"dataclass"` don't exist there.

**Root cause**: `monty_to_py()` handled `MontyObject::Type(t)` with
`import_builtins(py)?.getattr(py, t.to_string())`, which fails for any type
not present in `builtins` (e.g. `Dataclass`, `DateTime`, `JsonValue`).

**Fix**:
- `monty_to_py()` now checks `t.builtin_name()`: builtins are looked up as
  before; non-builtins return a `"<class 'name'>"` string.
- `monty_to_py_structured()` adds an explicit match arm for non-builtin types,
  wrapping them as `NonSerializable(type_name="type", repr="<class 'name'>")`.

## Future Considerations

- The `load_snapshot` and `load_repl_snapshot` functions also accept
  `print_callback` but don't yet support `structured_print_callback`. This
  could be added if snapshot serialization + structured output is needed.
