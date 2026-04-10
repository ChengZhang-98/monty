# Metadata Propagation — User Guide

Track where data comes from, who can see it, and what labels it carries as
values flow through the Monty interpreter.

## Overview

Every value in Monty can carry **metadata** with three fields:

| Field | Type | Default | Merge rule | Meaning |
|-------|------|---------|------------|---------|
| `producers` | `frozenset[str]` | `frozenset()` | **Union** | Data sources that contributed to this value |
| `consumers` | `frozenset[str] \| None` | `None` | **Intersection** | Who may see this value (`None` = anyone) |
| `tags` | `frozenset[str]` | `frozenset()` | **Union** | Classification labels |

When two values combine (e.g. `a + b`), their metadata merges automatically:

- `producers` = union — accumulates all sources
- `consumers` = intersection — most restrictive wins
- `tags` = union — accumulates all labels

## Quick start

```python
from pydantic_monty import AnnotatedValue, Monty, MontyComplete, ObjectMetadata

# 1. Create metadata
meta = ObjectMetadata(
    producers=frozenset({'user_input'}),
    tags=frozenset({'untrusted'}),
)

# 2. Wrap a value with metadata
annotated_input = AnnotatedValue(42, meta)

# 3. Pass it as input
m = Monty('x + 1', inputs=['x'])
result = m.start(inputs={'x': annotated_input})

# 4. Read metadata on the output
assert isinstance(result, MontyComplete)
assert result.output == 43
assert result.metadata.producers == frozenset({'user_input'})
assert result.metadata.tags == frozenset({'untrusted'})
```

## API reference

### `ObjectMetadata`

Immutable metadata record. All fields are keyword-only.

```python
ObjectMetadata(
    *,
    producers: frozenset[str] | None = None,  # defaults to frozenset()
    consumers: frozenset[str] | None = None,  # defaults to None (universal)
    tags: frozenset[str] | None = None,       # defaults to frozenset()
)
```

**Properties:**

| Property | Type | Description |
|----------|------|-------------|
| `.producers` | `frozenset[str]` | Source names |
| `.consumers` | `frozenset[str] \| None` | Allowed consumer names, or `None` for universal |
| `.tags` | `frozenset[str]` | Classification labels |

**Important — `consumers=None` vs `consumers=frozenset()`:**

| Value | Meaning |
|-------|---------|
| `consumers=None` (default) | **Universal** — any consumer may see the value |
| `consumers=frozenset()` | **Empty set** — *no* consumer is allowed to see the value |
| `consumers=frozenset({'admin'})` | Only consumers named `'admin'` may see the value |

When two values with different consumer sets combine, the result gets the
**intersection** (most restrictive). So `{'admin', 'user'} & {'admin'}` = `{'admin'}`.

**Validation:**

Label strings must be non-empty. Passing `''` in any field raises `ValueError`:

```python
ObjectMetadata(producers=frozenset({''}))
# ValueError: producers must not contain empty strings
```

### `AnnotatedValue`

Wraps a Python value with an `ObjectMetadata` record. Used at input/resume
boundaries to attach metadata to values entering the interpreter.

```python
AnnotatedValue(value: Any, metadata: ObjectMetadata)
```

**Properties:**

| Property | Type | Description |
|----------|------|-------------|
| `.value` | `Any` | The wrapped Python value |
| `.metadata` | `ObjectMetadata` | The attached metadata |

### `MontyComplete`

Returned when execution finishes. Now includes a `.metadata` property.

| Property | Type | Description |
|----------|------|-------------|
| `.output` | `Any` | The result value |
| `.metadata` | `ObjectMetadata \| None` | Propagated metadata, or `None` if no metadata was tracked |

## Attaching metadata to inputs

Pass `AnnotatedValue` objects in the `inputs` dict. Plain values (without
`AnnotatedValue`) carry no metadata — they behave exactly as before.

```python
m = Monty('a + b', inputs=['a', 'b'])

meta_a = ObjectMetadata(producers=frozenset({'api'}), tags=frozenset({'external'}))
meta_b = ObjectMetadata(producers=frozenset({'db'}), consumers=frozenset({'admin'}))

result = m.start(inputs={
    'a': AnnotatedValue(10, meta_a),
    'b': AnnotatedValue(20, meta_b),
})

# result.metadata.producers == frozenset({'api', 'db'})     — union
# result.metadata.consumers == frozenset({'admin'})          — intersection (None & {'admin'} = {'admin'})
# result.metadata.tags      == frozenset({'external'})       — union
```

You can mix annotated and plain inputs:

```python
result = m.start(inputs={
    'a': AnnotatedValue(10, meta_a),  # tracked
    'b': 20,                           # no metadata (DEFAULT)
})
# result.metadata == meta_a (merge with DEFAULT is identity)
```

## Attaching metadata on resume

When resuming a `FunctionSnapshot` with an external function's return value,
wrap the return value in `AnnotatedValue` to attach metadata:

```python
m = Monty('fetch(url)', inputs=['url'])
snap = m.start(inputs={'url': 'https://example.com'})

# snap is a FunctionSnapshot — the external function 'fetch' was called
assert isinstance(snap, FunctionSnapshot)

# Resume with a metadata-annotated return value
response_meta = ObjectMetadata(
    producers=frozenset({'web_api'}),
    tags=frozenset({'external', 'untrusted'}),
)
result = snap.resume(
    return_value=AnnotatedValue('response body', response_meta)
)

assert result.metadata.producers == frozenset({'web_api'})
```

Resuming with a plain value (no `AnnotatedValue`) gives the return value
`DEFAULT` metadata — no producers, universal consumers, no tags.

## How metadata propagates

### Automatic propagation

Metadata flows automatically through these operations — no user action needed:

| Operation | Behavior |
|-----------|----------|
| `a + b`, `a - b`, `a * b`, etc. | `merge(a.meta, b.meta)` |
| `a == b`, `a < b`, `a in b`, etc. | `merge(a.meta, b.meta)` |
| `f(a, b)` → `return y` | Arguments' metadata flows into parameters; `y` carries whatever metadata it accumulated from the values used to compute it (not from all arguments) |
| `x.attr` | Attribute inherits the object's metadata |
| `f'{a} {b}'` | Merges all interpolated values' metadata |
| `a, b = lst` | Each variable gets its element's per-element metadata |
| `[*x]`, `(*x,)`, `{*x}` | Elements preserve their per-element metadata |
| `{**d}`, `f(**kwargs)` | Keys and values preserve their per-entry metadata |
| `lst[i]`, `d[key]` | Result carries the specific element's metadata |
| `not x`, `-x` | Result carries `x`'s metadata |

### What does NOT propagate (yet)

Metadata does **not** currently flow through control flow:

```python
if secret_value:   # secret_value has metadata
    x = 1          # x does NOT inherit secret_value's metadata
```

> **Planned change:** Control-flow propagation (implicit flow tracking) is
> planned for a future release. In strict information flow control (IFC), the
> program counter carries a label that taints all assignments within a branch,
> so `x` above would inherit `secret_value`'s metadata. This is not yet
> implemented but will be added when the threat model requires it.

### Merge with DEFAULT is identity

When a tracked value combines with an untracked one (e.g. `x + 1` where `x`
has metadata but `1` is a literal), the result carries `x`'s metadata unchanged.
Literals and constants have `DEFAULT` metadata (empty producers, universal
consumers, empty tags), and merging with DEFAULT is an identity operation.

## Per-element container metadata

Containers track metadata **per element**, not at the container level. This
prevents false tainting:

```python
secret = get_secret()   # producers={'vault'}, consumers={'admin'}
public = 'hello'        # no metadata

lst = [secret, public]
x = lst[1]              # x gets public's metadata (None), NOT merged with secret's
```

This means you can safely put restricted and unrestricted data in the same
container — extracting an element gives you only *that element's* metadata.

## Checking metadata on output

After execution, inspect `result.metadata`:

```python
result = m.start(inputs={'x': AnnotatedValue(42, meta)})

if result.metadata is not None:
    # Value has provenance information
    if 'vault' in result.metadata.producers:
        print('Output contains data from the vault')

    if result.metadata.consumers is not None:
        # Access is restricted
        allowed = result.metadata.consumers
        if current_user not in allowed:
            raise PermissionError(f'User {current_user} not in {allowed}')

    if 'pii' in result.metadata.tags:
        print('Output contains PII — apply sanitization')
else:
    # No metadata tracked — value has no provenance restrictions
    pass
```

## Limitations

- **MontyRepl sync output**: `MontyRepl.feed_run` returns a plain value (no
  `MontyComplete`), so output metadata is not directly accessible on the sync
  path. However, metadata is tracked internally and persists across snippets.
  Use `feed_start` for output metadata via `MontyComplete.metadata`.
- **`for` loop iteration**: The `for x in iterable` loop does not currently
  propagate per-element metadata from the iterable to `x`. This requires
  changes to the iterator protocol.
- **Default arguments**: When a function parameter uses a default value, the
  default's metadata is not propagated. Parameters without an explicit argument
  get `DEFAULT` metadata.
- **JS bindings**: The JavaScript package (`@pydantic/monty`) does not yet
  expose metadata APIs.

## Complete example

```python
from pydantic_monty import AnnotatedValue, Monty, MontyComplete, ObjectMetadata

# Simulate a pipeline with data from multiple sources
code = '''
greeting = f'Hello, {name}!'
result = greeting + ' ' + suffix
result
'''

m = Monty(code, inputs=['name', 'suffix'])

# 'name' comes from user input — anyone can see it
name_meta = ObjectMetadata(
    producers=frozenset({'user_input'}),
    tags=frozenset({'untrusted'}),
)

# 'suffix' comes from a restricted API — only admins can see it
suffix_meta = ObjectMetadata(
    producers=frozenset({'internal_api'}),
    consumers=frozenset({'admin'}),
    tags=frozenset({'confidential'}),
)

result = m.start(inputs={
    'name': AnnotatedValue('Alice', name_meta),
    'suffix': AnnotatedValue('Level 5 clearance', suffix_meta),
})

assert isinstance(result, MontyComplete)
assert result.output == 'Hello, Alice! Level 5 clearance'

meta = result.metadata
assert meta is not None

# Producers: both sources contributed
assert meta.producers == frozenset({'user_input', 'internal_api'})

# Consumers: intersection of None (universal) and {'admin'} = {'admin'}
assert meta.consumers == frozenset({'admin'})

# Tags: union of both
assert meta.tags == frozenset({'untrusted', 'confidential'})
```
