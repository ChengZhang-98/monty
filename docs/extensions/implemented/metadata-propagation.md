# Extension: Metadata Propagation

## Summary

Track data provenance metadata (producers, consumers, tags) through every value
in the Monty interpreter, propagating automatically across operations.

## Motivation

Tiny Beaver runs untrusted code that processes data from multiple external
sources (web_search, bash, file reads, etc.). The **visibility/sanitization
system** needs to know *where each piece of data came from* (producers), *who
is allowed to see it* (consumers), and *what classification labels it carries*
(tags) so it can enforce access control before data reaches the Planning LLM.

Without metadata propagation, the agent framework must conservatively treat all
output as potentially tainted. With it, the framework can precisely determine
which outputs contain data from restricted sources and apply fine-grained
sanitization only where needed.

### Example use case

```python
# Inputs provided by host with metadata:
#   api_key  -> producers={"secrets_vault"}, consumers={"internal_tool"}, tags={"credential"}
#   user_msg -> producers={"user_input"},    consumers=UNIVERSAL,         tags={}

result = f"Processing: {user_msg}"
# result.metadata -> producers={"user_input"}, consumers=universal, tags={}
#                    (safe to show to PLLM)

response = call_api(api_key, user_msg)
# response.metadata depends on what the external function returns
# (host controls metadata on resume)
```

## Design

### Three-field model

Every value carries a `Metadata` record with three fields:

| Field | Default | Propagation | Semantics |
|-------|---------|-------------|-----------|
| `producers` | `{}` (empty) | **Union** | Data sources that contributed to this value |
| `consumers` | `UNIVERSAL` | **Intersection** | Who may see this value; `UNIVERSAL` = no restriction |
| `tags` | `{}` (empty) | **Union** | Classification labels (e.g. `"pii"`, `"credential"`) |

When two values combine (e.g. `a + b`), the result's metadata is:
- `result.producers = a.producers | b.producers`
- `result.consumers = a.consumers & b.consumers` (most restrictive wins)
- `result.tags = a.tags | b.tags`

The universal set is represented as `UNIVERSAL` (a singleton of type
`UniversalSet`) at the Python API boundary, `None` in the Rust `ObjectMetadata`
type, and `LabelSet { is_universal: true }` internally. Its algebra:
- `union(universal, s) = universal`
- `intersection(universal, s) = s`

### Two-level container metadata

Containers track metadata at **two levels**:

1. **Container-level metadata** — on the container variable itself (on the
   operand stack). This is the metadata from the container's creation context,
   e.g. metadata attached via `AnnotatedValue` when an external function returns
   a list.
2. **Per-element metadata** — stored alongside each element inside the
   container's internal data structure (e.g. `List.item_metadata`,
   `Dict.DictEntry.value_meta`).

When extracting an element (indexing or iteration), the result's metadata is
the **merge** of both levels:

```python
# External function returns a list with container-level metadata
results = web_search("query")
# results has tags={"__non_executable"} from AnnotatedValue
# results[0] has per-element metadata = DEFAULT (elements from host)

x = results[0]
# x.metadata.tags == {"__non_executable"}
# merge(container_meta, DEFAULT) = container_meta
```

Per-element metadata prevents false tainting when elements are built inside
the sandbox with independent provenance:

```python
secret = get_secret()   # producers={"vault"}, consumers={"admin"}
public = "hello"        # producers={}, consumers=universal

lst = [secret, public]
x = lst[1]              # x gets public's metadata, NOT merged with secret's
```

This works because `lst` has DEFAULT container-level metadata (built in-sandbox),
so `merge(DEFAULT, element_meta) = element_meta`.

When both levels carry non-default metadata, the standard merge rules apply:
union for producers/tags, intersection for consumers.

### Explicit data flow only (for now)

Metadata propagates through **value operations** (arithmetic, string
concatenation, function call arguments/returns, container indexing, etc.) but
**not yet through control flow**:

```python
if secret_condition:     # secret_condition has metadata from "vault"
    x = 1               # x does NOT inherit secret_condition's metadata
else:
    x = 2               # same — no implicit flow
```

> **Planned change: implicit flow tracking.** Control-flow propagation will be
> added in a future release. In strict information flow control (IFC) systems,
> the program counter carries a label that taints all assignments within a
> branch. This would catch the above case but increases complexity (every branch
> point must push/pop a PC label). Will be implemented when the threat model
> requires it.

## Architecture

### Interning and performance

Label strings are interned once in a `MetadataStore` and referenced by
`LabelId` (u32). Metadata records are deduplicated and referenced by
`MetadataId` (u32). This keeps the per-value cost to a single `u32`.

```
┌──────────────────────────────────────────────────────┐
│  MetadataStore (owned by VM, serialized in snapshot) │
│  ├── label_strings: Vec<String>     (LabelId → str)  │
│  ├── label_map: HashMap<str, LabelId>                │
│  ├── entries: Vec<Metadata>    (MetadataId → record) │
│  └── dedup_map: HashMap<Metadata, MetadataId>        │
├──────────────────────────────────────────────────────┤
│  entries[0] = default metadata (always)              │
│    producers: empty, consumers: universal, tags: empty│
└──────────────────────────────────────────────────────┘
```

Merge hot-path short-circuits:
- `merge(DEFAULT, DEFAULT) = DEFAULT` — O(1)
- `merge(a, DEFAULT) = a` — O(1) (empty producers/tags, universal consumers)
- `merge(DEFAULT, b) = b` — O(1)

### Parallel metadata stack

The VM carries a **parallel metadata stack** alongside the operand stack.
Every `push()` also pushes a `MetadataId`, every `pop()` also pops one.
The invariant `meta_stack.len() == stack.len()` is maintained at all times.

```
Value stack:     [ Int(1),  Ref(42),  Float(3.14), ... ]
Metadata stack:  [ DEFAULT, Meta(3),  DEFAULT,     ... ]
                   ↑ paired 1:1 ↑
```

Similarly:
- `meta_globals: Vec<MetadataId>` parallels `globals: Vec<Value>`
- `meta_exception_stack: Vec<MetadataId>` parallels `exception_stack: Vec<Value>`
- Async tasks store `meta_stack` and `meta_exception_stack` alongside their value stacks

This approach was chosen over alternatives:
- **Wrapping Value** (`struct Tagged(Value, MetadataId)`) would change the Value
  type used in 100+ files
- **Side table** (`HashMap<SlotId, MetadataId>`) doesn't naturally handle
  stack temporaries

### Public API type

`ObjectMetadata` is the public-facing metadata type. In Rust, it uses
`Option<BTreeSet<String>>` where `None` represents the universal set:

```rust
pub struct ObjectMetadata {
    pub producers: Option<BTreeSet<String>>,  // None = universal
    pub consumers: Option<BTreeSet<String>>,  // None = universal
    pub tags: Option<BTreeSet<String>>,       // None = universal
}
```

`BTreeSet` ensures deterministic serialization order. `Option<BTreeSet>` avoids
the need for a sentinel wildcard string at the Rust API boundary.

In Python, `None` is replaced with an explicit `UNIVERSAL` sentinel (a singleton
of type `UniversalSet`) so that the difference between "universal" and "not
provided" is clear. Each field on the Python `ObjectMetadata` is either a
`frozenset[str]` or `UNIVERSAL`:

```python
from pydantic_monty import ObjectMetadata, UNIVERSAL

meta = ObjectMetadata(producers=frozenset({'vault'}))
assert meta.consumers is UNIVERSAL  # default: no restriction
assert meta.producers == frozenset({'vault'})
```

## Implementation Status

### Phase 0: Core types (complete)

New file `crates/monty/src/metadata.rs` with:
- `LabelId` — interned label handle (u32, Copy)
- `LabelSet` — compact sorted set with universal-set algebra (SmallVec\<\[LabelId; 2\]\>)
- `Metadata` — immutable three-field record
- `MetadataId` — interned metadata handle (u32, Copy, DEFAULT = 0)
- `MetadataStore` — interning, dedup, merge with short-circuits
- `ObjectMetadata` — public API type (re-exported from `monty::ObjectMetadata`)

### Phase 1: VM parallel stack (complete)

Added parallel metadata tracking to the VM without changing any opcode behavior
(all metadata is `MetadataId::DEFAULT`).

| File | Change |
|------|--------|
| `crates/monty/src/metadata.rs` | New module — all core metadata types |
| `crates/monty/src/lib.rs` | Added module declaration and public re-export |
| `crates/monty/src/bytecode/vm/mod.rs` | `meta_stack`, `meta_globals`, `meta_exception_stack`, `metadata_store` on VM and VMSnapshot. Updated `new()`, `restore()`, `snapshot()`, `cleanup()`. Push/pop mirror metadata. Dup/Rot opcodes mirror metadata. Load/store local/global/cell propagate metadata |
| `crates/monty/src/bytecode/vm/call.rs` | `call_sync_function` extends meta_stack for function locals |
| `crates/monty/src/bytecode/vm/async_exec.rs` | Task save/restore mirrors metadata. Coroutine frame init extends meta_stack |
| `crates/monty/src/bytecode/vm/exceptions.rs` | Exception stack push/pop and unwind mirror metadata |
| `crates/monty/src/bytecode/vm/scheduler.rs` | `Task` struct gains `meta_stack`, `meta_exception_stack` |
| `crates/monty/src/run_progress.rs` | `NameLookup::resume` mirrors metadata for cached values |
| `crates/monty/tests/metadata.rs` | 11 integration tests for `ObjectMetadata` |

Backward compatibility: all new snapshot fields use `#[serde(default)]` so old
snapshots deserialize with DEFAULT metadata. On restore, metadata vecs are
resized to match value vec lengths.

### Phase 2: Opcode propagation (complete)

Every value-producing opcode now propagates metadata through the parallel stack.

| Category | Opcodes | Metadata behavior |
|----------|---------|-------------------|
| Binary ops | `BinaryAdd/Sub/Mul/Div/FloorDiv/Mod/Pow`, `BinaryAnd/Or/Xor`, `BinaryBitwise` | `merge(lhs_meta, rhs_meta)` |
| Comparisons | `CompareEq/Ne/Lt/Le/Gt/Ge`, `CompareIs/IsNot`, `CompareIn/NotIn`, `CompareModEq` | `merge(lhs_meta, rhs_meta)` |
| Unary ops | `UnaryNot/Neg/Pos/Invert` | propagate operand metadata |
| In-place ops | `InplaceAdd/Sub/Mul/...` | delegate to binary ops (already propagating) |
| Boolean short-circuit | `JumpIfTrueOrPop`, `JumpIfFalseOrPop` | preserve metadata of whichever operand is returned |
| F-strings | `BuildFString` | merge all parts' metadata |
| Format | `FormatValue` | propagate value's metadata (format spec is just formatting) |
| Attributes | `LoadAttr`, `LoadAttrImport` | propagate object's metadata to attribute |
| Return | `ReturnValue` | propagate return value's metadata to caller's stack |
| Constants/literals | `LoadConst/None/True/False/SmallInt` | `DEFAULT` (via `push()`) |

| File | Change |
|------|--------|
| `crates/monty/src/bytecode/vm/binary.rs` | All binary ops use `pop_with_meta()` + `merge()` + `push_with_meta()` |
| `crates/monty/src/bytecode/vm/compare.rs` | All comparison ops use same pattern |
| `crates/monty/src/bytecode/vm/mod.rs` | Unary ops, boolean short-circuit, LoadAttr stamp, ReturnValue propagation |
| `crates/monty/src/bytecode/vm/format.rs` | `build_fstring` merges all parts, `format_value` propagates value metadata |

**Not yet propagated** (deferred):
- `CellValue` (closure cells): would require breaking `#[repr(transparent)]` on `CellValue`. Cells propagate `DEFAULT` for now. This means closure-captured variables lose their metadata — acceptable for Phase 2, to be addressed when `CellValue` is reworked.
- External call resume values: addressed in Phase 4 (API boundary metadata).

### Phase 3: Container element metadata (complete)

Every container type now stores per-element metadata:

| Container | Storage | Change |
|-----------|---------|--------|
| **List** | `item_metadata: Vec<MetadataId>` (parallel to `items`) | All mutations (append, insert, pop, remove, clear, reverse, sort, extend, copy, slice, iadd) maintain the parallel vec |
| **Tuple** | `item_metadata: SmallVec<[MetadataId; 3]>` (parallel to `items`) | Immutable after creation via `new_with_metadata()` |
| **Dict** | `key_meta: MetadataId` + `value_meta: MetadataId` in `DictEntry` | All entry construction sites updated, `set_with_meta()` passes metadata through |
| **Set** | `meta: MetadataId` in `SetEntry` | `add_with_meta()`, `clone_entries()`, set algebra ops updated |

**Build opcodes** (`BuildList`, `BuildTuple`, `BuildDict`, `BuildSet`) use `pop_n_with_meta()` to capture element metadata from the stack and pass it to container constructors.

**`UnpackSequence`** propagates element metadata from List/Tuple back onto the stack — each unpacked variable receives its element's metadata, not the container's.

| File | Change |
|------|--------|
| `crates/monty/src/types/list.rs` | `item_metadata` field, `new_with_metadata()`, metadata-aware mutations, `get_slice_metadata()` |
| `crates/monty/src/types/tuple.rs` | `item_metadata` field, `new_with_metadata()`, `allocate_tuple_with_metadata()` |
| `crates/monty/src/types/dict.rs` | `key_meta`/`value_meta` on `DictEntry`, `set_with_meta()`, copy/merge propagation |
| `crates/monty/src/types/set.rs` | `meta` on `SetEntry`, `add_with_meta()`, clone/algebra propagation |
| `crates/monty/src/bytecode/vm/collections.rs` | Build opcodes pass element metadata, `UnpackSequence` extracts it |
| `crates/monty/src/bytecode/vm/mod.rs` | `BinarySubscr` propagates container metadata |
| `crates/monty/tests/resource_limits.rs` | Updated memory size assertions (containers slightly larger with metadata fields) |

**Not yet wired** (deferred to follow-up):
- `ForIter` element-level: iterator would need to yield element metadata from the container.
- `StoreSubscr` element-level: `py_setitem` would need to accept metadata to store on the element.

### Phase 4: API boundary (complete)

Metadata now enters and exits the VM through the public API.

**New public types:**
- `AnnotatedObject { value: MontyObject, metadata: Option<ObjectMetadata> }` — a value paired with optional provenance metadata
- `From<MontyObject> for AnnotatedObject` — seamless conversion when metadata is not needed

**Input path:**
- `MontyRun::run()` and `start()` accept `Vec<impl Into<AnnotatedObject>>` — callers can pass `Vec<MontyObject>` (no metadata) or `Vec<AnnotatedObject>` (with metadata)
- `populate_inputs()` interns `ObjectMetadata` into the VM's `MetadataStore` and sets `meta_globals` per input variable

**Output path:**
- `RunProgress::Complete(AnnotatedObject)` — return values carry their propagated metadata
- `FrameExit::Return(Value, MetadataId)` — metadata propagates through the internal frame exit
- `convert_frame_exit` converts `MetadataId` to `ObjectMetadata` via the `MetadataStore`

**External function resume:**
- `ExtFunctionResult::Return(MontyObject, Option<ObjectMetadata>)` — hosts can attach metadata to return values
- `vm.resume(obj, meta)` interns the metadata and pushes it with the value
- `From<MontyObject>` and `From<AnnotatedObject>` impls for `ExtFunctionResult`

**Python bindings:**
- `ObjectMetadata` and `AnnotatedValue` pyclass types for attaching/reading metadata
- `UniversalSet` pyclass and `UNIVERSAL` module-level singleton for representing
  the universal set explicitly (replaces `None` at the Python API boundary)
- `py_to_annotated()` detects `AnnotatedValue` at input boundaries
- `MontyComplete.metadata` property exposes output metadata
- Empty label validation (`validate_no_empty_strings()`)

**JS bindings:**
- Updated to compile with the new API — metadata fields passed as `None` for now

| File | Change |
|------|--------|
| `crates/monty/src/metadata.rs` | Added `AnnotatedObject` type with serde support |
| `crates/monty/src/lib.rs` | Re-exported `AnnotatedObject` |
| `crates/monty/src/run.rs` | `run()`/`start()` accept `impl Into<AnnotatedObject>`, `populate_inputs` interns metadata |
| `crates/monty/src/run_progress.rs` | `Complete(AnnotatedObject)`, `ExtFunctionResult::Return` carries metadata, `convert_frame_exit` extracts metadata |
| `crates/monty/src/bytecode/vm/mod.rs` | `FrameExit::Return` carries `MetadataId`, `vm.resume()` accepts metadata |
| `crates/monty/src/repl.rs` | Updated for new `ExtFunctionResult` and `ConvertedExit` |
| `crates/monty-python/src/` | Updated for new API types (metadata=None for now) |
| `crates/monty-js/src/` | Updated for new API types (metadata=None for now) |
| `crates/monty-cli/src/main.rs` | Updated for `AnnotatedObject` output |
| `crates/monty/tests/` | Updated all test files for new API |

### Phase 5: Edge cases (complete)

| Feature | Status | Detail |
|---------|--------|--------|
| **CellValue metadata** | Done | `CellValue` now has `value` + `meta` fields. `LoadCell`/`StoreCell` propagate metadata through closure cells. Removed `#[repr(transparent)]` and `#[serde(transparent)]`. |
| **Comprehensions** | Done | `ListAppend`, `SetAdd`, `DictSetItem` pass element metadata from the stack into containers during comprehension building. |
| **BinarySubscr element-level** | Done | `resolve_subscr_meta()` merges container-level metadata with per-element metadata from List/Tuple (integer indexing) and Dict (key lookup via `value_meta_for_key()`). Uses `lookup_element_meta()` for the per-element lookup. |
| **UnpackEx (star unpacking)** | Done | `a, *rest, b = lst` propagates per-element metadata. The `*rest` list carries each collected element's individual metadata. |
| **Slice operations** | Done (Phase 3) | `get_slice_metadata()` mirrors slice indexing for metadata. |
| **Copy** | Done (Phase 3) | `list.copy()` preserves element metadata. |
| **`*args` unpacking** | Done | `extract_args_tuple_with_meta()` reads per-element metadata from the args tuple and populates `pending_arg_metadata` for propagation to the callee's parameter slots. |
| **`**kwargs` / dict merging** | Done | `dict_merge()` and `dict_update()` preserve per-key and per-value metadata via `entries_with_metadata()` and `set_with_meta()`. |
| **`list_extend` / `set_extend`** | Done | PEP 448 `[*x]` and `{*x}` preserve per-element metadata via `extract_items_with_meta()` helper. |
| **`list_to_tuple`** | Done | Preserves per-element metadata when converting via `allocate_tuple_with_metadata()`. |
| **String unpacking** | Done | Characters inherit the string's metadata in `unpack_sequence`, `unpack_ex`, `list_extend`, `set_extend` (via `extract_items_with_meta()`). |
| **Empty label validation** | Done | Python API rejects empty strings in producers/consumers/tags with `ValueError`. |
| **ForIter element-level** | Done | `MontyIter` now stores `container_meta` from `GET_ITER`. `advance()` returns `(Value, MetadataId)` — the merge of container-level and per-element metadata. `get_heap_item()` returns per-element metadata for List, Tuple, NamedTuple, Dict (keys/values views). `FOR_ITER` uses `push_with_meta()`. |
| **`for_next()` metadata** | Done | `MontyIter::for_next()` now returns `(Value, MetadataId)` — merged container + per-element metadata, consistent with `advance()` on `HeapRead<MontyIter>`. All Rust-side iteration (builtins, `collect()`, set/dict construction) has access to element metadata. |
| **`next()` builtin metadata** | Done | `iterator_next()` returns `(Value, MetadataId)`. `builtin_next` sets `vm.pending_result_metadata` so the dispatch code pushes the result with the correct metadata. |
| **`pending_result_metadata`** | Done | New `MetadataId` field on VM. `CallBuiltinFunction` and `CallBuiltinType` dispatch reads and resets this field, using `push_with_meta()` instead of `push()`. Allows any builtin to propagate metadata to its return value. |
| **Builtin container metadata** | Done | All iteration-creating builtins (`all`, `any`, `sum`, `min`, `max`, `enumerate`, `filter`, `map`, `reversed`, `sorted`, `zip`) now read container metadata from `vm.pending_arg_metadata` and pass it to `MontyIter::new()`. `iter()` type constructor also captures metadata. Scalar-producing builtins (`sum`, `min`, `max`) set `pending_result_metadata`. |
| **`DictItemsView` element metadata** | Done | `get_heap_item()` DictItemsView arm now uses `allocate_tuple_with_metadata()` with `key_meta_at()`/`value_meta_at()`, propagating per-entry metadata to tuple elements. |
| **`collect_iterable_to_set` metadata** | Done | Both paths (fast-path via `advance()` and `for_next()` path) now use `set.add_with_meta()` to preserve per-element metadata. `Set::from_iterator()` also preserves metadata. |
| **Default arguments** | Deferred | When function parameters use defaults, the default's metadata would need to propagate. Requires changes to argument binding. |

| File | Change |
|------|--------|
| `crates/monty/src/heap_data.rs` | `CellValue` now a struct with `value` + `meta` fields |
| `crates/monty/src/heap.rs` | Updated `cell.0` → `cell.value` references |
| `crates/monty/src/object.rs` | Updated `cell.0` → `cell.value` reference |
| `crates/monty/src/bytecode/vm/mod.rs` | `load_cell`/`store_cell` propagate metadata. `BinarySubscr` uses `resolve_subscr_meta()` which merges container + element metadata via `lookup_element_meta()`. `GET_ITER` captures container metadata with `pop_with_meta()` and passes to `MontyIter::new()`. `FOR_ITER` uses `push_with_meta()` with merged metadata from `advance()`. Added `pending_result_metadata: MetadataId` field. `CallBuiltinFunction`/`CallBuiltinType` dispatch uses `push_with_meta(result, pending_result_metadata)`. |
| `crates/monty/src/bytecode/vm/call.rs` | Updated `CellValue` construction with struct syntax. `extract_args_tuple_with_meta()` propagates `*args` element metadata via `pending_arg_metadata`. |
| `crates/monty/src/bytecode/vm/collections.rs` | `list_append`/`set_add`/`dict_set_item` pass metadata. `unpack_ex`/`unpack_sequence` propagate per-element metadata (including string chars inheriting the string's metadata). `list_extend`/`set_extend` preserve element metadata via `extract_items_with_meta()`. `dict_merge`/`dict_update` preserve key/value metadata. `list_to_tuple` preserves element metadata. |
| `crates/monty/src/types/iter.rs` | `MontyIter` now stores `container_meta: MetadataId`. `new()` accepts `container_meta`. `advance()` and `for_next()` both return `(Value, MetadataId)` with merged container+element metadata. `get_heap_item()` returns per-element metadata for all container types. `DictItemsView` uses `allocate_tuple_with_metadata()` for key/value metadata. `iterator_next()` returns `(Value, MetadataId)`. `init()` (iter constructor) captures metadata from `pending_arg_metadata`. |
| `crates/monty/src/types/dict.rs` | Added `value_meta_for_key()` for subscript metadata resolution without `&mut VM`. Removed `#[expect(dead_code)]` from `value_meta_at`/`key_meta_at` (now used by `get_heap_item()`). |
| `crates/monty/src/types/dict_view.rs` | `collect_iterable_to_set()` now uses `add_with_meta()` to preserve per-element metadata in both fast-path and `for_next()` paths. |
| `crates/monty/src/types/set.rs` | `Set::from_iterator()` now uses `add_with_meta()` to preserve per-element metadata. |
| `crates/monty/src/builtins/*.rs` | All iteration-creating builtins (`all`, `any`, `sum`, `min`/`max`, `enumerate`, `filter`, `map`, `reversed`, `sorted`, `zip`) read container metadata from `vm.pending_arg_metadata` and pass it to `MontyIter::new()`. `sum`, `min`, `max` set `pending_result_metadata`. `next` propagates via `pending_result_metadata`. |
| `crates/monty/src/types/list.rs` | Added `extend_with_meta()` for metadata-preserving list extension. |
| `crates/monty-python/src/metadata.rs` | Added `validate_no_empty_strings()` — rejects empty strings in metadata labels. |

### Phase 6: External function call arg metadata (complete)

Metadata on the **arguments** of external function calls (and OS calls) is now
preserved and exposed to the host. Previously, `FunctionCall.args` was
`Vec<MontyObject>` (no metadata); now it is `Vec<AnnotatedObject>`, and the
Python `FunctionSnapshot` exposes `annotated_args` and `annotated_kwargs`
properties that return `AnnotatedValue` objects — the same type used for inputs.

**Core changes:**

- `pending_kwarg_metadata: Vec<MetadataId>` added to the VM alongside the
  existing `pending_arg_metadata`. Populated by `exec_call_function_kw` and
  `exec_call_attr_kw` via `pop_n_with_meta()`.
- `ArgValues::into_annotated_objects()` replaces `into_py_objects()`. Reads
  positional metadata from `pending_arg_metadata`, inline kwarg metadata from
  `pending_kwarg_metadata`, and dict kwarg metadata from the dict's own
  per-entry metadata.
- `Dict::into_iter_with_metadata()` consuming iterator added for
  metadata-preserving dict kwargs conversion.
- `FunctionCall`, `OsCall`, `ReplFunctionCall`, `ReplOsCall` args/kwargs fields
  changed from `Vec<MontyObject>` to `Vec<AnnotatedObject>` (and likewise for
  kwargs pairs).
- `ConvertedExit::FunctionCall` and `ConvertedExit::OsCall` updated accordingly.
- All consumers updated: `OsFunction::on_no_handler`, `parse_fs_request`,
  `MountTable::handle_os_call`, dispatch functions in Python/JS bindings.

**Python API:**

- `FunctionSnapshot.annotated_args` → `tuple[AnnotatedValue, ...]` — each
  positional arg bundled with its metadata.
- `FunctionSnapshot.annotated_kwargs` → `dict[str, AnnotatedValue]` — each
  kwarg value bundled with its metadata.
- `FunctionSnapshot.args` and `.kwargs` still return plain values (backwards
  compatible).

**JS API:**

- `MontySnapshot.args` and `.kwargs` still return plain values (backwards
  compatible). The underlying `Vec<AnnotatedObject>` carries metadata
  internally; annotated accessors can be added when the JS metadata API is
  designed.

**Serialization:**

- `SERIALIZATION_VERSION` bumped from 1 to 2 (inner `FunctionCall<T>` now
  serializes `Vec<AnnotatedObject>` args).

| File | Change |
|------|--------|
| `crates/monty/src/bytecode/vm/mod.rs` | Added `pending_kwarg_metadata` field |
| `crates/monty/src/bytecode/vm/call.rs` | KW call opcodes capture metadata via `pop_n_with_meta()` |
| `crates/monty/src/args.rs` | `into_annotated_objects()` replaces `into_py_objects()` |
| `crates/monty/src/types/dict.rs` | `DictIntoIterWithMeta`, `Dict::into_iter_with_metadata()` |
| `crates/monty/src/run_progress.rs` | `FunctionCall`, `OsCall`, `ConvertedExit` use `AnnotatedObject` |
| `crates/monty/src/repl.rs` | `ReplFunctionCall`, `ReplOsCall` use `AnnotatedObject` |
| `crates/monty/src/os.rs` | `on_no_handler` accepts `&[AnnotatedObject]` |
| `crates/monty/src/fs/dispatch.rs` | `parse_fs_request` accepts `&[AnnotatedObject]` |
| `crates/monty/src/fs/mount_table.rs` | `handle_os_call` accepts `&[AnnotatedObject]` |
| `crates/monty-python/src/monty_cls.rs` | `annotated_args`/`annotated_kwargs` getters on `PyFunctionSnapshot` |
| `crates/monty-python/src/external.rs` | Dispatch functions accept `&[AnnotatedObject]` |
| `crates/monty-python/src/async_dispatch.rs` | Dispatch functions accept `&[AnnotatedObject]` |
| `crates/monty-python/src/serialization.rs` | `SERIALIZATION_VERSION` bumped to 2 |
| `crates/monty-js/src/monty_cls.rs` | `MontySnapshot` and dispatch updated for `AnnotatedObject` |
| `crates/monty-cli/src/main.rs` | Helper functions updated for `AnnotatedObject` |

### Phase 7: Explicit universal set sentinel (complete)

The Python API previously used `None` to represent the universal set for metadata
fields. This was ambiguous — `None` could mean "not provided" or "universal".

This phase introduces an explicit `UniversalSet` type and `UNIVERSAL` singleton
so the Python API has a clear, self-documenting sentinel:

```python
from pydantic_monty import ObjectMetadata, UNIVERSAL

meta = ObjectMetadata()
assert meta.consumers is UNIVERSAL  # explicitly universal, not None
```

**Changes:**

- `PyUniversalSet` frozen pyclass with `__contains__` (always `True`),
  `__bool__` (`True`), `__repr__` (`"UNIVERSAL"`), `__eq__`, `__hash__`,
  `__iter__` (raises `TypeError`), `__len__` (raises `TypeError`)
- `UNIVERSAL` module-level singleton via `PyOnceLock` — identity checks
  (`meta.consumers is UNIVERSAL`) work process-wide
- `ObjectMetadata` field storage changed from `Option<Py<PyFrozenSet>>` to
  `Py<PyAny>` (either `frozenset` or `UniversalSet`) with `parse_metadata_field`
  validation
- Constructor accepts `frozenset[str] | UniversalSet | None` for each field;
  `None` applies the field-specific default
- `field_repr` / `field_eq` helpers for repr and equality that handle both types
- Conversion helpers `option_set_to_py` / `py_field_to_option_set` bridge
  between Rust `Option<BTreeSet>` and Python `frozenset | UNIVERSAL`

The Rust core `ObjectMetadata` type is **unchanged** — it still uses
`Option<BTreeSet<String>>` where `None` = universal.

| File | Change |
|------|--------|
| `crates/monty-python/src/metadata.rs` | `PyUniversalSet`, `universal_singleton()`, field parsing/repr/eq helpers, updated `PyObjectMetadata` to use `Py<PyAny>` fields |
| `crates/monty-python/src/lib.rs` | Registered `UniversalSet` class and `UNIVERSAL` constant on the module |
| `crates/monty-python/python/pydantic_monty/__init__.py` | Exported `UniversalSet` and `UNIVERSAL` |
| `crates/monty-python/python/pydantic_monty/_monty.pyi` | Added `UniversalSet` class, `UNIVERSAL` constant, updated `ObjectMetadata` field types |
| `crates/monty-python/tests/test_ext_metadata.py` | Added tests for `UNIVERSAL` singleton, `UniversalSet` behavior, explicit universal fields, round-trip |

## Testing

```bash
# Run metadata unit + integration tests
cargo test -p monty --features ref-count-panic metadata

# Run full test suite (metadata infrastructure must not break anything)
make test-ref-count-panic
```

Tests cover:
- `LabelSet` algebra: union, intersection, universal set, empty set, disjoint, overlapping
- `MetadataStore`: label interning/dedup, metadata interning/dedup, merge short-circuits, commutativity, associativity
- `ObjectMetadata`: serde round-trip (JSON + postcard), equality, BTreeSet ordering, `None` vs empty consumers
- `MetadataStore` serde: postcard round-trip preserves labels and entries
- `ObjectMetadata` ↔ `MetadataId` round-trip through `intern_object_metadata` / `to_object_metadata`

**End-to-end propagation tests** (Rust core API, `crates/monty/tests/metadata.rs`):
- Input passthrough: single input with metadata returned unchanged
- Default metadata: input without metadata produces `None` on output
- Binary op merge: `a + b` merges producers (union), consumers (intersection), tags (union)
- Function call: `double(x)` propagates `x`'s metadata through argument binding and return
- Variable assignment: `a = x; b = a + 1; b` carries `x`'s metadata
- Merge with default: `x + 1` preserves `x`'s metadata (DEFAULT is identity)
- F-string: `f'{a} {b}'` merges metadata from all interpolated values
- No metadata: `1 + 2` with no inputs produces `None` metadata
- `*args`: `f(*x)` propagates per-element metadata to function parameters
- `{**x}`: dict literal merging preserves per-key/per-value metadata
- `[*x]`: list extend preserves per-element metadata
- `(*x,)`: list-to-tuple conversion preserves per-element metadata
- String unpacking: `a, b = x` and `first, *rest = x` inherit the string's metadata
- One-sided merge: `a + b` where only one operand has metadata preserves it

**Python binding tests** (`crates/monty-python/tests/test_ext_metadata.py`):
- `ObjectMetadata` construction, defaults, equality, repr, and validation
- `UNIVERSAL` singleton identity, `isinstance`, `__contains__`, `__bool__`, `__repr__`, `__eq__`
- `UNIVERSAL` raises `TypeError` on `iter()` and `len()`
- Explicit `UNIVERSAL` on each field (producers, consumers, tags)
- `UNIVERSAL` vs `frozenset()` inequality
- `UNIVERSAL` round-trip through the interpreter
- Empty string rejection: producers, consumers, and tags must not contain empty strings
- Invalid type rejection for metadata fields

## Future Considerations

- **Implicit flow tracking** (planned): program counter label for control-flow
  tainting — assignments inside `if`/`else`/`while` branches would inherit the
  condition's metadata
- **`StoreSubscr` metadata propagation**: `d[key] = value` does not currently
  propagate `value`'s metadata to the dict entry. This would require changes
  to the `StoreSubscr` opcode handler.
- **Method call metadata propagation**: `obj.method()` does not propagate
  `obj`'s metadata through the method call result. For example,
  `results.items()` produces a DictItemsView with DEFAULT metadata even if
  `results` has container metadata.
- **Metadata-aware type conversions**: `str()`, `repr()`, `int()`, `float()`
  currently produce `DEFAULT` metadata; they should propagate the operand's
  metadata since they are data conversions.
- **Default argument metadata**: function parameter defaults should propagate
  their metadata when the argument is not provided by the caller
- **REPL sync output metadata**: `MontyRepl.feed_run` returns `MontyObject`
  (no metadata on the output). Metadata is tracked internally and persists
  across snippets; use `feed_start` for output metadata via `MontyComplete`

## Porting Upstream Features — Metadata Checklist

This codebase is forked from `pydantic/monty`. When merging new features from
upstream (e.g. new opcodes, new builtins, new container types, new
`py_getattr`/`py_setattr` behaviors), the metadata system must be extended to
cover the new code paths. This section explains the invariants, the patterns to
follow, and the common pitfalls.

### Core invariant

```
meta_stack.len() == stack.len()    (always, on every code path)
```

Every call to `self.stack.push(v)` must have a corresponding
`self.meta_stack.push(m)`. Every pop from one must pop from the other. If this
invariant breaks, the VM will panic on the next metadata access (index out of
bounds) or silently misattribute metadata to the wrong value.

### Decision tree for new code

When you add or port code that **produces a new value** on the stack, ask:

1. **Where does the value come from?**
   - **Constant/literal** (e.g. `LoadConst`, `LoadNone`) → push `MetadataId::DEFAULT`
   - **Derived from one operand** (e.g. unary op, type conversion) → propagate the operand's metadata
   - **Derived from two operands** (e.g. binary op, comparison) → `metadata_store.merge(lhs_meta, rhs_meta)`
   - **From a container element** (e.g. subscript, iteration) → merge the container's stack-level metadata with the element's per-element metadata
   - **From host/external** (e.g. external function return, input) → use the `ObjectMetadata` from the API boundary
   - **Structural/internal** (e.g. creating an iterator, building a class) → `MetadataId::DEFAULT`

2. **Does the code push directly onto `self.stack`?**
   - If yes, you MUST also push onto `self.meta_stack`. Use `push_with_meta(value, meta)` instead of `push(value)`.
   - If no (the code calls `self.push()` which already handles both), you're fine.

3. **Does the code pop from the stack?**
   - If you need the metadata: use `pop_with_meta()` → `(Value, MetadataId)`
   - If you don't need it: `pop()` already pops from both stacks

4. **Does the code directly extend/drain/truncate `self.stack`?**
   - Mirror the operation on `self.meta_stack`. E.g.:
     ```rust
     self.stack.extend(namespace);
     self.meta_stack.extend(iter::repeat_n(MetadataId::DEFAULT, ns_len));
     ```

### Patterns by feature type

#### New opcode

Every value-producing opcode needs metadata handling. The pattern depends on the
opcode category (see Phase 2 table in this doc). Common patterns:

```rust
// Binary op pattern
let (rhs, rhs_meta) = self.pop_with_meta();
defer_drop!(rhs, self);
let (lhs, lhs_meta) = self.pop_with_meta();
defer_drop!(lhs, self);
let result_meta = self.metadata_store.merge(lhs_meta, rhs_meta);
// ... compute result ...
self.push_with_meta(result, result_meta);

// Unary op pattern
let (value, meta) = self.pop_with_meta();
// ... compute result ...
self.push_with_meta(result, meta);

// Attribute access pattern (LoadAttr)
let obj_meta = self.peek_meta();  // capture before pop
// ... py_getattr pushes via handle_call_result! ...
// stamp metadata on the pushed result:
if let Some(last) = self.meta_stack.last_mut() {
    *last = obj_meta;
}
```

#### New container type

If upstream adds a new container type (e.g. `OrderedDict`, `deque`):

1. Add `MetadataId` storage per element (parallel vec or inline in entry struct)
2. Add `#[serde(default)]` on the new field for backward compatibility
3. Update all element mutation methods to maintain the metadata parallel vec
4. Update `py_estimate_size` to include metadata in the size calculation
5. Update the `Build*` opcode to use `pop_n_with_meta()` and pass to the constructor
6. Update `UnpackSequence`/`UnpackEx` if the type is iterable
7. Update `BinarySubscr` if the type supports indexing — add a branch in
   `resolve_subscr_meta()`

#### New builtin function

Builtins that consume iterables now read `vm.pending_arg_metadata` to capture
the iterable's container metadata and pass it to `MontyIter::new()`. Builtins
that produce scalar results (`sum`, `min`, `max`, `next`) set
`vm.pending_result_metadata` so the dispatch code pushes the result with the
correct metadata. The dispatch code for `CallBuiltinFunction`/`CallBuiltinType`
reads and resets `pending_result_metadata`, using `push_with_meta()`.

| Builtin | Metadata behavior |
|---------|-------------------|
| `len()`, `type()`, `isinstance()`, `id()` | `DEFAULT` (informational, not data-derived) |
| `str()`, `repr()`, `int()`, `float()` | Propagate operand's metadata (data conversion) |
| `sum()`, `min()`, `max()` | Propagate container metadata via `pending_result_metadata`; `sum` also merges per-element metadata across items |
| `next()` | Propagates element metadata from iterator via `pending_result_metadata` |
| `sorted()`, `reversed()` | Create new containers; container metadata passed to `MontyIter::new()` for iteration |
| `list()`, `tuple()`, `set()`, `dict()` | Constructor — elements keep their metadata |
| `all()`, `any()` | Iterate with container metadata; return `bool` with `DEFAULT` |
| `enumerate()`, `filter()`, `map()`, `zip()` | Iterate with container metadata; produce new containers |
| `iter()` | Captures container metadata from `pending_arg_metadata` into `MontyIter.container_meta` |

To add metadata propagation to a new builtin:
1. Capture metadata early: `let meta = vm.pending_arg_metadata.first().copied().unwrap_or_default();`
2. Pass it to `MontyIter::new(iterable, vm, meta)` if iterating
3. Set `vm.pending_result_metadata = meta;` before returning if the result should carry metadata

#### New trait method (e.g. `py_getattr` changes)

If upstream changes how `py_getattr`, `py_setattr`, `py_getitem`, or
`py_setitem` work, the metadata handling is at the **opcode level**, not the
trait level. The trait methods don't know about metadata — the VM dispatch code
handles it:

- `LoadAttr`: captures `obj_meta` before the call, stamps it on the result after
- `BinarySubscr`: calls `resolve_subscr_meta()` which merges container-level
  metadata with per-element metadata (via `lookup_element_meta()`)
- `GetIter`: captures container metadata with `pop_with_meta()` and stores it
  in `MontyIter.container_meta`
- `ForIter`: uses `push_with_meta()` with the merged result from `advance()`
  (container-level + per-element metadata)
- `StoreSubscr`: currently doesn't propagate metadata to the stored element
  (deferred)

If a trait method gains a new code path that pushes values (e.g. property
getters), check that the push goes through the opcode handler's metadata logic.

#### New `FrameExit` variant

If upstream adds a new `FrameExit` variant (e.g. for a new kind of external
call), update:

1. `convert_frame_exit()` in `run_progress.rs` — extract metadata if the exit
   carries a return value
2. `build_run_progress()` — wrap in `AnnotatedObject` if producing `Complete`
3. The resume method — accept `Option<ObjectMetadata>` and intern it via
   `vm.resume(obj, meta.as_ref())`

#### New async/task feature

If upstream adds new async patterns, ensure:

1. `save_task_context()` saves `meta_stack` and `meta_exception_stack`
2. `load_or_init_task()` restores them (with `.resize()` for old snapshots)
3. Any direct `self.stack.push/extend` in async code has a matching
   `self.meta_stack` operation

### Common pitfalls when merging upstream

1. **New `self.stack.extend(...)` without `meta_stack.extend(...)`** — causes
   immediate panic on next `pop()`. Search for `self.stack.extend` in the diff
   and verify each has a metadata counterpart.

2. **New `self.push(v)` in a hot loop** — `push()` defaults to `DEFAULT`, which
   is correct for constants but wrong for values derived from operands. Check if
   the pushed value should carry metadata from its source.

3. **New `HeapData` variant without `CellValue`-style metadata** — won't cause a
   compile error but will lose metadata for values stored in the new type.

4. **New `ConvertedExit` or `RunProgress` variant** — if it carries a
   `MontyObject`, it should probably carry `Option<ObjectMetadata>` too.

5. **New `ExtFunctionResult` match arms** — pattern matches on `Return` now need
   two fields: `Return(obj, meta)`. The compiler will catch this (exhaustive
   match), but the new code needs to handle the metadata correctly.

6. **Snapshot format changes** — new metadata fields should use
   `#[serde(default)]` so old snapshots deserialize with `DEFAULT` metadata.
   Test with `make test-ref-count-panic` which exercises the serde path.

### Quick merge verification

After merging upstream changes, run:

```bash
# Must pass — metadata invariant failures cause panics
make test-ref-count-panic

# Must pass — checks for metadata-related lint issues
make lint-rs

# Search for direct stack manipulation that might miss metadata
grep -rn 'self\.stack\.\(push\|extend\|drain\)' crates/monty/src/bytecode/vm/
# Each hit should have a corresponding meta_stack operation nearby
```
