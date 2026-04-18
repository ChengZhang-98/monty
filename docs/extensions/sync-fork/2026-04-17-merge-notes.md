# 2026-04-17 upstream merge notes

Completion notes for the `main` → `tiny-beaver-ext` merge prepared in
`2026-04-17-merge-plan.md`. Merge branch: `cz/merge-main/2026-04-17`.

## Final resolutions

### `PrintTarget` unification

Merged the extension's `StructuredCallbackMarker` / `CallbackStructuredPrint`
pipeline into upstream's `PrintTarget`:

- Added `PrintTarget::StructuredCallback { callback, dc_registry }` variant
  alongside the pre-existing `Stdout`, `Callback`, `CollectStreams`,
  `CollectString` variants.
- Added `PrintTarget::from_py_args(print_callback, structured_print_callback,
  dc_registry)` which errors (`ValueError`) when both callbacks are set,
  routes through `StructuredCallback` when only the structured variant is
  supplied, and falls back to `from_py(print_callback)` otherwise.
- Moved the `CallbackStructuredPrint` writer implementation into
  `print_target.rs`; deleted the extension helpers
  `CallbackStringPrint`, `CallbackStructuredPrint`, `StructuredCallbackMarker`,
  `wrap_structured_callback`, `unwrap_structured_callback`,
  `resolve_print_callback`, and the REPL's `make_print_writer_from_callback`.
- Every `#[pymethods]` `run`/`start`/`run_async`/`resume`/`feed_*` signature
  now takes `structured_print_callback: Option<&Bound<'_, PyAny>>` alongside
  `print_callback`, and routes through `PrintTarget::from_py_args`. No
  callsite still constructs a `PrintWriter` by hand.

### Snapshot storage

`PyFunctionSnapshot` now stores `args: Vec<MontyObject>` plus
`args_metadata: Vec<Option<ObjectMetadata>>` (and the same split for kwargs).
Lazy getters `args` / `kwargs` reconstruct the `Py` side on demand (upstream's
approach); `annotated_args` / `annotated_kwargs` still expose the
extension's metadata-aware `PyAnnotatedValue` tuples by zipping stored values
with stored metadata. `PyMontyComplete` stores `monty_output: AnnotatedObject`
(extension metadata preserved) with lazy output / metadata / output_json
getters (upstream's lazy-only shape).

### `call_os_callback_parts` signature

Changed to accept `&[AnnotatedObject]` / `&[(AnnotatedObject, AnnotatedObject)]`
because `OsCall.args` / `ReplOsCall.args` hold `Vec<AnnotatedObject>` in the
extension branch. Callers pass `&call.args` / `&call.kwargs` directly; the
function strips `.value` when building the Python-side tuple/dict.

### `object_json.rs` — natural JSON with annotated containers

Upstream's `object_json.rs` assumed `MontyObject::List/Tuple/Set/FrozenSet`
hold `Vec<MontyObject>` and that `Dict`/dataclass attrs hold `DictPairs`. In
the extension those all hold `AnnotatedObject` variants. Rewrote the internal
helpers:

- `serialize_annotated_seq` / `serialize_tagged_annotated_seq` /
  `serialize_annotated_dict` — walk annotated containers, drop metadata (no
  place for it in natural JSON), and recurse via `JsonMontyObject(&x.value)`.
- `FieldsBody` and `AttrsBody` now borrow `&[AnnotatedObject]` /
  `&AnnotatedDictPairs`.
- `JsonMontyArray` / `JsonMontyPairs` (public API) still take `&[MontyObject]`
  and are used by `PyFunctionSnapshot::args_json` / `kwargs_json`, which
  themselves store plain `Vec<MontyObject>`. Dropped the now-unused
  `serialize_seq`-variant helpers (`serialize_tagged_seq`, `serialize_dict`,
  `DictPairsBody`).

Added `AnnotatedDictPairs::len()` so the new dict helper can size its
`serialize_map` call up-front.

### `MontyRun::run_no_limits` signature

Changed from `Vec<impl Into<AnnotatedObject>>` to `Vec<MontyObject>`. The old
signature is fine at callsites that pass real values but fails type inference
on the extremely common `run_no_limits(vec![])` idiom: with no elements the
compiler cannot select the `impl Into<AnnotatedObject>` bound. Taking
`Vec<MontyObject>` resolves inference and costs us nothing — the body still
maps through `Into::into` to get `AnnotatedObject`s before delegating to
`run`.

Callers passing raw `Vec<MontyObject>` (all tests, `monty-bench`,
`monty-cli`, JS bindings) were already valid; no update required.

## Fixed during merge: `Module::set_attr` propagates allocation errors

4 Python tests (`test_start::test_run_multiple_threads_with_shared_mount_under_memory_limit`,
`test_start::test_start_mount_released_after_resource_error`,
`test_mount_table::test_repl_feed_run_mount_released_after_resource_error`,
`test_mount_table::test_feed_start_mount_released_after_resource_error`)
panicked in `crates/monty/src/types/module.rs:65` — `Module::set_attr` called
`self.attrs.set(...).unwrap()`, but after an upstream change `Dict::set` can
return `Err(UncatchableExc(MemoryError))` when `max_memory` is exceeded during
dict growth. Lazy stdlib module construction under a tight limit then panicked
instead of propagating the exception.

Fix: changed `Module::set_attr` to return `RunResult<()>` (dropping any
previous value defensively) and propagated through every `create_module`
function (`sys`, `typing`, `asyncio`, `pathlib`, `os`, `math`, `json`, `re`,
`datetime`) — each now returns `RunResult<HeapId>` instead of
`Result<HeapId, ResourceError>`. `StandardLib::create` was updated to match;
its sole caller (`VM::load_module` in `bytecode/vm/mod.rs`) already returned
`RunResult<()>`, so no change there. `ResourceError: Into<RunError>` means
`vm.heap.allocate(...)?` still works unchanged.

## Validation

- `make format-rs` — clean
- `make lint-rs` — clean (clippy + import check workspace-wide)
- `make dev-py` — builds cleanly
- `make pytest` — **1061 passed, 1 skipped, 1 xfailed** (the 4 previously
  failing mount/memory tests now pass after the `Module::set_attr` fix)
- `cargo test -p monty --features ref-count-panic` — clean
- `uv run cargo run -p monty-datatest --features ref-count-panic` —
  928 passed
