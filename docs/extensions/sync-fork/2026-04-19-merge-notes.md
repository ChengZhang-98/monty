# 2026-04-19 upstream merge notes

Completion notes for the `main` → `cz/fork-sync/2026-04-18` merge.
Merge commit: `bd9752f`. Second parent: upstream `a4be566`.

## Upstream PRs bundled

Of the 19 upstream commits since the last sync, 17 were already cherry-picked
into `tiny-beaver-ext` via the squash-merged PR #11 and were recorded as
already-integrated by `2672e15` (`git merge -s ours` against upstream
`v0.0.14` tag `f2b05cd`). The remaining 6 PRs were merged in this sync:

| PR   | Commit    | Summary                                                 |
| ---- | --------- | ------------------------------------------------------- |
| #66  | `8d62cb9` | Implement `hasattr` builtin                             |
| #251 | `8d68341` | Fix partial-future resolution panics in mixed gathers   |
| #353 | `49de74a` | Add benchmark for parsing 1000 lines                    |
| #354 | `1496d1a` | Cheap sourcemaps (reworked `parse.rs` source location)  |
| #355 | `3929368` | Raise `MontySyntaxError` for source with lone surrogates |
| #271 | `a4be566` | `MontyRepl.call_function` for calling Python from Rust  |

## Conflict resolutions

Five files had real conflicts (all driven by tiny-beaver's metadata
propagation extension):

### `crates/monty/src/bytecode/vm/call.rs`

Upstream PR #251 refactored `FrameExit::Return` handling to an explicit
`return Ok(v)`. Tiny-beaver's `FrameExit::Return` carries a metadata tuple
(`Return(Value, Option<Metadata>)`). Resolution kept both:

```rust
FrameExit::Return(v, _meta) => return Ok(v),
```

### `crates/monty/src/bytecode/vm/async_exec.rs`

PR #251 introduced `VM::resume_with_resolved_futures`. Tiny-beaver's
`ExtFunctionResult::Return` has a second metadata field. Resolution: adopted
the new method and updated the one call site to destructure
`ExtFunctionResult::Return(obj, _meta)`.

### `crates/monty/src/run_progress.rs`

Same refactor — replaced the inline future-resolution block with
`vm.resume_with_resolved_futures(results)`.

### `crates/monty/src/repl.rs`

Two conflicts:

1. Imports — merged upstream's new imports (`ArgValues`, `KwargsValues`,
   `ExcType`, `MontyException`) with tiny-beaver's `AnnotatedObject`.
2. `call_function` implementation (PR #271) — upstream uses `take_globals()`
   which only exists under `#[cfg(feature = "ref-count-return")]`.
   Tiny-beaver has `take_globals_with_meta()` that also returns the parallel
   `meta_globals` and `metadata_store`. Resolution:

   ```rust
   let (g, mg, ms) = vm.take_globals_with_meta();
   self.globals = g;
   self.meta_globals = mg;
   self.metadata_store = ms;
   ```

   This keeps metadata and the metadata store in sync across calls —
   required for the extension's propagation invariants.

### `crates/monty-python/src/repl.rs`

- Imports: merged `metadata::py_to_annotated` with upstream's
  `monty_cls::{EitherProgress, call_os_callback_parts, extract_source_code,
  py_type_check}`.
- Kept tiny-beaver's `PrintTarget::from_py_args(print_callback,
  structured_print_callback, &this.dc_registry)?;` over upstream's
  single-callback path (already established in the 2026-04-17 merge).
- Removed obsolete `code_owned` binding.

### `crates/monty/tests/repl.rs`

Both sides added tests. Concatenated HEAD's metadata-propagation tests with
upstream's `call_function` tests, and updated the upstream test snippets to
the extension's signatures:

- `MontyObject::Repr("bad".into())` → `MontyObject::Repr { type_name:
  "test".into(), repr: "bad".into() }` (tiny-beaver uses the struct form).
- `MontyObject::List(vec![MontyObject::Int(1)])` →
  `MontyObject::List(vec![MontyObject::Int(1).into()])` (list elements are
  `AnnotatedObject`).

Same `.into()` / `MontyObject::Repr` shape updates applied to
`tests/asyncio.rs` (5 sites) and `tests/name_lookup.rs` (1 site) — these
were auto-merged by git but compilation required matching tiny-beaver types.

In `tests/asyncio.rs`, `runner.start(vec![], ...)` was also changed to
`runner.start(Vec::<MontyObject>::new(), ...)` to resolve E0283 (type
inference failure on the `impl Into<AnnotatedObject>` bound with an empty
vec).

## Validation

- `make format-rs` — clean
- `make lint-rs` — clean
- `make test-ref-count-panic` — passing
- `make test-py` — passing
- `make test-cases` — 930 passed (baseline before merge: 928)
