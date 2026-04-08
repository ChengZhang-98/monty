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
            iterators, ranges, etc.) are passed as their repr() string.
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
from pydantic_monty import Monty

calls = []
def handler(stream, objects, sep, end):
    calls.append((objects, sep, end))

m = Monty('print(1, "hello", [1, 2], sep="-")')
m.run(structured_print_callback=handler)
# calls == [([1, "hello", [1, 2]], "-", "\n")]
#           ^^^^^^^^^^^^^^^^^^^ native types, not strings
```

## Implementation

### Files Changed

| File | Change |
|------|--------|
| `crates/monty/src/io.rs` | Added `wants_structured()` and `stdout_write_structured()` default methods to `PrintWriterCallback` trait and `PrintWriter` enum |
| `crates/monty/src/builtins/print.rs` | Branch: if `wants_structured()`, convert Values to MontyObjects and call `stdout_write_structured` once; else existing string path |
| `crates/monty/src/object.rs` | Made `MontyObject::from_value` `pub(crate)` (was private) |
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

3. **MontyObject → Python conversion**: Uses the existing `monty_to_py()` path,
   which means types like `Type(int)` become the actual Python `int` type
   object (not the string `"<class 'int'>"`). Only `MontyObject::Repr` variants
   (ranges, iterators, closures, etc.) become strings.

4. **DcRegistry in marker**: The structured callback needs `DcRegistry` to
   convert dataclass MontyObjects back to proper Python dataclass instances.
   Bundling it in the marker avoids adding a parameter to `with_print_writer`.

## Testing

```bash
make dev-py
uv run python -m pytest crates/monty-python/tests/test_print.py -v -k "structured"
```

Tests cover:
- Basic types (int, str, float, bool, None)
- Nested containers (list, dict, tuple)
- Custom sep/end
- Empty print()
- Multiple print() calls
- Non-serializable fallback (range → repr string)
- Both-callbacks error
- MontyRepl.feed_run
- MontyRepl.feed_start

## Commits

| Hash | Description |
|------|-------------|
| `caeeeaf` | Add structured_print_callback for typed print output |
| `e822822` | Document structured_print_callback in all .pyi docstrings |

## Future Considerations

- The `load_snapshot` and `load_repl_snapshot` functions also accept
  `print_callback` but don't yet support `structured_print_callback`. This
  could be added if snapshot serialization + structured output is needed.
