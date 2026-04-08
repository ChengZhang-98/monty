# Extension: [Name]

## Summary

One-line description of what this extension does.

## Motivation

Why this extension exists. What problem does it solve for tiny-beaver?

## API

### New Parameters

```python
# Show the new parameter(s) on the method signature
result = monty.run(
    ...,
    my_new_param=...,  # describe
)
```

### Callback Signature (if applicable)

```python
def my_callback(arg1: type1, arg2: type2) -> None:
    ...
```

### Behavior

- What happens when the parameter is provided
- What happens when it's not provided (default behavior)
- Any mutual exclusion with other parameters

## Implementation

### Files Changed

| File | Change |
|------|--------|
| `crates/monty/src/...` | What changed |
| `crates/monty-python/src/...` | What changed |

### Patterns Used

Which patterns from [patterns.md](../patterns.md) were used and why.

### Key Design Decisions

Any non-obvious choices and their rationale.

## Testing

How to run the tests for this extension:

```bash
uv run python -m pytest crates/monty-python/tests/test_... -v -k "test_name"
```

## Commits

After each commit, record its hash here in a separate follow-up commit
(to avoid a chicken-and-egg problem where amending changes the hash).

| Hash | Description |
|------|-------------|
| `abcdef0` | Initial implementation |

## Future Considerations

Any known limitations or planned improvements.
