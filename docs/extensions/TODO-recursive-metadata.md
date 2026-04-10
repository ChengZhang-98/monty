# TODO: Recursive Metadata on MontyObject

## Goal

Change `MontyObject` container variants to hold `AnnotatedObject` instead of bare
`MontyObject`, so per-element metadata is visible at the API boundary (Python/JS
bindings, external function call args, resume return values).

Currently, metadata exists per-element **inside** the VM (List's `item_metadata`,
Dict's `DictEntry.key_meta`/`value_meta`, etc.) but is **lost** when converting
to `MontyObject` at API boundaries. This task surfaces that internal metadata
through the public `MontyObject` type.

## Current State

```
Internal (VM):
  List { items: [a, b], item_metadata: [meta_a, meta_b] }

Public API (MontyObject):
  AnnotatedObject {
    value: MontyObject::List(vec![MontyObject::Int(1), MontyObject::Int(2)]),
    metadata: None,   // only top-level metadata
  }
  // element metadata is LOST at this boundary
```

## Target State

```
Public API (MontyObject):
  AnnotatedObject {
    value: MontyObject::List(vec![
      AnnotatedObject { value: MontyObject::Int(1), metadata: Some(meta_a) },
      AnnotatedObject { value: MontyObject::Int(2), metadata: Some(meta_b) },
    ]),
    metadata: None,
  }
```

## Step-by-step Plan

### Step 1: Change MontyObject container variants

**File: `crates/monty/src/object.rs`**

Change these variants from `MontyObject` children to `AnnotatedObject` children:

```rust
// BEFORE
pub enum MontyObject {
    List(Vec<MontyObject>),
    Tuple(Vec<MontyObject>),
    Set(Vec<MontyObject>),
    FrozenSet(Vec<MontyObject>),
    Dict(DictPairs),                    // DictPairs = Vec<(MontyObject, MontyObject)>
    NamedTuple { ..., values: Vec<MontyObject> },
    Dataclass { ..., attrs: DictPairs },
    // scalars unchanged: Int, Float, String, Bool, None, etc.
}

// AFTER
pub enum MontyObject {
    List(Vec<AnnotatedObject>),
    Tuple(Vec<AnnotatedObject>),
    Set(Vec<AnnotatedObject>),
    FrozenSet(Vec<AnnotatedObject>),
    Dict(AnnotatedDictPairs),           // new type: Vec<(AnnotatedObject, AnnotatedObject)>
    NamedTuple { ..., values: Vec<AnnotatedObject> },
    Dataclass { ..., attrs: AnnotatedDictPairs },
    // scalars unchanged
}
```

You'll also need to create `AnnotatedDictPairs`:
```rust
pub struct AnnotatedDictPairs(pub Vec<(AnnotatedObject, AnnotatedObject)>);
```

**Important**: `AnnotatedObject` is defined in `crates/monty/src/metadata.rs` and
publicly exported from `monty::AnnotatedObject`. It has:
```rust
pub struct AnnotatedObject {
    pub value: MontyObject,
    pub metadata: Option<ObjectMetadata>,
}
```

This creates a **mutual recursion**: `MontyObject` contains `AnnotatedObject` which
contains `MontyObject`. This is fine in Rust (both are heap-allocated via `Vec`).
Make sure serde derives still work — they should since the recursion goes through
`Vec` indirection.

### Step 2: Update `MontyObject::to_value()` (input path)

**File: `crates/monty/src/object.rs`, method `to_value()`**

When converting `MontyObject::List(elements)` → internal `Value::Ref(List)`:
- Each `AnnotatedObject` in `elements` has `.value` (the MontyObject) and `.metadata`
- Convert each `.value` to a `Value` via recursive `to_value()`
- Convert each `.metadata` to a `MetadataId` via `vm.metadata_store.intern_object_metadata()`
- Pass both to `List::new_with_metadata(values, metadata_ids)`

Same pattern for Tuple, Dict, Set, FrozenSet, NamedTuple, Dataclass.

### Step 3: Update `MontyObject::from_value()` (output path)

**File: `crates/monty/src/object.rs`, method `from_value_inner()`**

When converting internal `List` → `MontyObject::List(elements)`:
- For each element at index `i`, read `list.item_meta(i)`
- Convert the `MetadataId` to `Option<ObjectMetadata>` via `vm.metadata_store.to_object_metadata(id)`
- Wrap in `AnnotatedObject::new(child_monty_obj, child_metadata)`

Same pattern for Tuple (use `tuple.item_meta(i)`), Dict (use `entry.key_meta` /
`entry.value_meta`), Set (use `set.meta_at(i)`).

**Key issue**: `from_value_inner()` currently doesn't have access to the
`MetadataStore`. You'll need to thread it through — either pass `&MetadataStore`
as an additional parameter, or access it via the VM reference. The VM has
`pub(crate) metadata_store: MetadataStore`.

### Step 4: Update `convert_frame_exit()` for external function args

**File: `crates/monty/src/run_progress.rs`**

The function `args.into_py_objects(vm)` converts `ArgValues` → `Vec<MontyObject>`.
This needs to produce `Vec<AnnotatedObject>` with recursive metadata. The
`FunctionCall.args` field should change from `Vec<MontyObject>` to
`Vec<AnnotatedObject>`.

Also update `FunctionCall.kwargs` similarly.

### Step 5: Update `populate_inputs()` for input containers

**File: `crates/monty/src/run.rs`**

When `AnnotatedObject { value: List(annotated_elements), metadata }` is passed as
input, `to_value()` (from Step 2) should automatically handle the recursive
metadata. Just make sure the top-level metadata is still interned and stored in
`meta_globals`.

### Step 6: Update broadcast for external function resume

**File: `crates/monty/src/bytecode/vm/mod.rs`, method `resume()`**

When the host resumes with `ExtFunctionResult::Return(MontyObject::List(...), Some(meta))`:
- The top-level metadata goes on the stack via `push_with_meta(value, meta_id)`
- The element metadata inside the `MontyObject::List` should be converted by `to_value()`
  (Step 2) and stored in the internal `List.item_metadata`
- If the host provides element-level metadata (via `AnnotatedObject` children), use it
- If the host only provides top-level metadata, broadcast it to all elements that
  don't have their own metadata (i.e., `element.metadata.is_none()` → use parent's)

### Step 7: Fix all MontyObject construction sites

Search the codebase for every place that constructs `MontyObject::List(...)`,
`MontyObject::Tuple(...)`, `MontyObject::Dict(...)`, `MontyObject::Set(...)`, etc.
and update to use `AnnotatedObject` children.

Key files to check:
- `crates/monty/src/object.rs` — `from_value_inner()`, various constructors
- `crates/monty/src/io.rs` — structured print callback creates `Vec<MontyObject>`
- `crates/monty/src/run_progress.rs` — `into_py_objects()` for function call args
- `crates/monty/src/modules/json.rs` — JSON parsing creates MontyObject trees
- `crates/monty/src/repl.rs` — REPL progress types
- `crates/monty-python/src/convert.rs` — `py_to_monty()` and `monty_to_py()`
- `crates/monty-js/src/convert.rs` — `js_to_monty()` and `monty_to_js()`
- `crates/monty/tests/` — many tests construct MontyObject containers

**Tip**: After changing the enum variants, `cargo check` will show every broken
call site. Fix them mechanically:
- `MontyObject::List(vec![MontyObject::Int(1)])` becomes
  `MontyObject::List(vec![AnnotatedObject::from(MontyObject::Int(1))])`
  or use the `From<MontyObject>` impl which sets metadata to `None`.

### Step 8: Update Python bindings

**File: `crates/monty-python/src/convert.rs`**

- `py_to_monty()`: When converting a Python list to `MontyObject::List`, wrap
  each element in `AnnotatedObject` (with `None` metadata for now, since Python
  callers don't pass per-element metadata yet)
- `monty_to_py()`: When converting `MontyObject::List(Vec<AnnotatedObject>)` to
  Python, extract `.value` from each element (ignore metadata for now)
- Later: expose metadata as a Python dict on each element for policy checking

### Step 9: Update JS bindings

**File: `crates/monty-js/src/convert.rs`**

Same pattern as Python bindings.

### Step 10: Update tests

Many tests in `crates/monty/tests/` construct `MontyObject::List(vec![...])` etc.
These all need wrapping with `AnnotatedObject::from()`. This is the most tedious
part but fully mechanical.

Also add new tests in `crates/monty/tests/metadata.rs`:
```rust
#[test]
fn metadata_element_level_on_external_function_args() {
    // ext_fn([a, b]) where a and b have different metadata
    // → FunctionCall.args[0] should be List with per-element metadata
}

#[test]
fn metadata_resume_with_annotated_container() {
    // Resume ext_fn with List([Annotated(1, meta_a), Annotated(2, meta_b)])
    // → elements should carry their metadata through subsequent operations
}
```

## Files Changed (estimated)

| File | Change |
|------|--------|
| `crates/monty/src/object.rs` | Container variants → `AnnotatedObject`, `to_value`/`from_value` carry metadata |
| `crates/monty/src/metadata.rs` | `AnnotatedDictPairs` type, possibly helpers |
| `crates/monty/src/run_progress.rs` | `FunctionCall.args` → `Vec<AnnotatedObject>` |
| `crates/monty/src/io.rs` | Structured print callback with annotated objects |
| `crates/monty/src/modules/json.rs` | JSON parsing wraps children in AnnotatedObject |
| `crates/monty/src/run.rs` | `populate_inputs` handles recursive metadata |
| `crates/monty/src/repl.rs` | REPL types updated |
| `crates/monty-python/src/convert.rs` | Recursive Python ↔ AnnotatedObject conversion |
| `crates/monty-js/src/convert.rs` | Recursive JS ↔ AnnotatedObject conversion |
| `crates/monty/tests/*.rs` | Update MontyObject container constructions |
| `crates/monty-python/tests/*.py` | May need updates if return types change |

## Verification

```bash
# After each step, verify:
cargo check -p monty              # core compiles
cargo check --workspace           # all crates compile
make format-rs && make lint-rs    # formatting and lint clean
make test-ref-count-panic         # all tests pass

# Specifically test metadata round-trip:
cargo test -p monty --features ref-count-panic --test metadata
```

## Important Invariants

1. `From<MontyObject> for AnnotatedObject` must still work — sets metadata to `None`
2. Serde round-trip must work — `AnnotatedObject` with `None` metadata should be
   compact (don't use `skip_serializing_if` with postcard — it breaks binary deser)
3. The `DictPairs` type is public — if changing to `AnnotatedDictPairs`, ensure
   backward compatibility or update all consumers
4. Cycle detection in `from_value_inner()` must still work with `AnnotatedObject`
5. `PartialEq` on `MontyObject` must still work for test assertions

## Reference

- Extension docs: `docs/extensions/implemented/metadata-propagation.md`
- Upstream merge guide: see "Porting Upstream Features" section in the extension doc
- `AnnotatedObject` definition: `crates/monty/src/metadata.rs`
- Internal element metadata: `List.item_metadata`, `Tuple.item_metadata`,
  `DictEntry.key_meta`/`value_meta`, `SetEntry.meta`
