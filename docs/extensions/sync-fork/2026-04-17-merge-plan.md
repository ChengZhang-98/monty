# 2026-04-17 Merge Plan: `main` → `tiny-beaver-ext`

Integrating 17 new upstream commits (post-`v0.0.10` → `v0.0.14`) into the
extension branch. This document is the operational plan — read it before
re-running `git merge main` on `cz/merge-main/2026-04-17`.

## Upstream commits being merged

### Features / behavior

| PR    | Commit     | What it adds                                              |
|-------|------------|-----------------------------------------------------------|
| #319  | `2f93fb4`  | REPL-side static type checking (`type_check`, `skip_type_check`, `type_check_stubs`) |
| #324  | `23ff7eb`  | `zip(..., strict=True)` with per-argument length checks   |
| #322  | `06a12d2`  | `PrintTarget` with `None` / callable / `CollectString` / `CollectStreams` variants |
| #332  | `a8645d8`  | Correct datetime types on OS calls, `NOT_HANDLED` sentinel |
| #337  | `6692c0b`  | Pass `os` / `mount` on `start()` (not just `run()`)       |
| #349  | `b38e220`  | `ExternalExceptionData` — richer external exception payload |
| #348  | `0c61d4d`  | Natural JSON form for `MontyObject` (`JsonMontyObject`, `JsonMontyArray`, `JsonMontyPairs`) |

### Fix

| PR    | Commit     | What it fixes                                              |
|-------|------------|-----------------------------------------------------------|
| #342  | `6fab906`  | Panic when a source file has lines longer than `u16::MAX` |

### Perf / refactor

| PR    | Commit     | What it changes                                            |
|-------|------------|-----------------------------------------------------------|
| #321  | `902d8a3`  | Type-checking performance improvements                    |
| #326  | `b72f306`  | VM cleanup moved from explicit `vm.cleanup()` into `Drop` impl |

### Infra

| PR    | Commit     | What it does                                               |
|-------|------------|-----------------------------------------------------------|
| #320  | `e78c9c3`  | Add JSON load/dump to benchmarks                          |
| #325  | `70b9300`  | Extract `monty-bench` and `monty-datatest` as their own crates |
| #328  | `811ca2c`  | Rust coverage collection for Python tests                 |

### Releases

`2e9df4b`, `dc44b92`, `e02ffc4`, `f2b05cd` — version bumps to 0.0.11, 0.0.12, 0.0.13, 0.0.14.

## Merge approach

We merge `main` → `cz/merge-main/2026-04-17` (branched from
`tiny-beaver-ext`). When resolved and reviewed, fast-forward
`tiny-beaver-ext` to this branch.

The first merge attempt produced **14 conflicted files**, of which ~65 of
~75 conflict hunks need semantic (not textual) reasoning. Below is the
per-file resolution plan.

### Files resolved trivially in the first pass (keep for redo)

| File                                       | Resolution                                                    |
|-------------------------------------------|---------------------------------------------------------------|
| `crates/monty/src/builtins/zip.rs`        | Keep extension's metadata-aware `MontyIter::new(.., meta)` + `for_next` returning `(item, meta)`. Layer upstream strict-mode logic on top. Use `defer_drop_mut!(iterators, vm)` from upstream for iterator cleanup. |
| `crates/monty/src/object.rs`              | Accept upstream's `DictPairs::len()` (new) alongside existing `is_empty()`. |
| `crates/monty/src/lib.rs`                 | Union exports: keep extension's `metadata::{AnnotatedObject, ObjectMetadata}`, `AnnotatedDictPairs`. Add upstream's `PrintStream`, `JsonMontyArray`, `JsonMontyObject`, `JsonMontyPairs`, `object_json` module. |
| `crates/monty/tests/print_writer.rs`      | Rename `PrintWriter::Collect` → `PrintWriter::CollectString` everywhere (upstream renamed). Accept upstream's `vec![]` over extension's `Vec::<MontyObject>::new()`. |

### Files needing a semantic merge

#### `crates/monty/src/bytecode/vm/mod.rs` (3 conflicts)

- Snapshot struct construction: use `mem::take` for every field (scheduler, meta_stack, meta_globals, meta_exception_stack, metadata_store) since `Drop` now touches `self.scheduler`.
- **Remove extension's `VM::cleanup()` method.** Upstream replaced it with a `Drop` impl (#326). Drop already handles exception_stack / scheduler / globals / json cache. Metadata fields (`meta_globals`, `meta_exception_stack`, `metadata_store`) are `Vec<MetadataId>` / `MetadataStore` and don't hold heap refcounts, so their default Drop is sufficient — **verify this by reading `MetadataStore` source before final commit.**
- Gate plain `take_globals()` on `#[cfg(feature = "ref-count-return")]`. Keep `take_globals_with_meta()` as the REPL's primary reclamation path.

#### `crates/monty/src/repl.rs` (11 conflicts — all mechanically identical)

Every conflict is the same pattern:
```
<<<<<<< HEAD
    let (g, mg, ms) = vm.take_globals_with_meta();
    repl.globals = g;
    repl.meta_globals = mg;
    repl.metadata_store = ms;
    vm.cleanup();
=======
    repl.globals = vm.take_globals();
>>>>>>> main
```

Resolution: keep HEAD, delete the `vm.cleanup();` line (Drop now does it).
Use a script:

```python
import re
path = "crates/monty/src/repl.rs"
with open(path) as f: text = f.read()
pattern = re.compile(r"<<<<<<< HEAD\n(.*?)=======\n(.*?)>>>>>>> main\n", re.DOTALL)
def keep_head_drop_cleanup(m):
    lines = m.group(1).splitlines(keepends=True)
    return "".join(ln for ln in lines if ln.strip() != "vm.cleanup();")
with open(path, 'w') as f:
    f.write(pattern.sub(keep_head_drop_cleanup, text))
```

Also check `run.rs:404` — it's inside a `#[cfg(feature = "ref-count-return")]` function so `take_globals()` is OK there.

#### `crates/monty-python/src/lib.rs` (5 conflicts)

Union everything: keep all of extension's exports (`PyNonSerializable`,
`PyObjectMetadata`, `PyUniversalSet`, `PyAnnotatedValue`) **and** all of
upstream's (`PyCollectStreams`, `PyCollectString`, `get_not_handled`,
`NotHandledSentinel`). Module-init adds both `UNIVERSAL` and `NOT_HANDLED`.

Keep both `mod non_serializable;` (extension) and `mod print_target;`
(upstream — holds `PyCollectStreams`/`PyCollectString`).

Update import: `use pyo3::{prelude::*, sync::PyOnceLock, types::PyAny};`
(upstream needs those for the `NOT_HANDLED` singleton).

#### `crates/monty-python/src/monty_cls.rs` (30 conflicts) — THE HARD ONE

This is where upstream's `PrintTarget` and extension's
`structured_print_callback` collide. Every `Monty.run` / `Monty.start` /
`resume` entry point is touched by both sides.

**Integration design** — both systems stay, but the parameter shape changes:

- `print_callback` parameter accepts: `None`, a callable, `CollectString()`, or `CollectStreams()` (per upstream). Upstream's `PrintTarget::from_py` handles this dispatch. **Keep upstream's PrintTarget as the mechanism for the `print_callback` parameter.**
- `structured_print_callback` remains a **separate, orthogonal** parameter (per extension). It's only ever a callable returning typed objects with metadata. When present, it short-circuits `print_callback` routing and is wired via extension's `CallbackStructuredPrint`.
- `resolve_print_callback(py, print_cb, structured_cb, dc_registry)` stays as the extension's single entry point — but internally it now produces a `PrintTarget` (upstream's) when `structured_cb` is absent, and a structured-callback writer when it's present. This preserves a single call site in every REPL/Monty method.

Per-method resolution recipe:

1. **Signature**: union the parameter lists from both sides. Order:
   `code, *, inputs, external_functions, print_callback, structured_print_callback, mount, os, type_check, skip_type_check, type_check_stubs` (exact set per method).
2. **Body prelude**: call `resolve_print_callback` to get a `PrintTarget`-or-structured-writer; call `run_type_check_if_enabled` (upstream); call `OsHandler::from_run_args` (upstream).
3. **External exceptions**: adopt upstream's `ExternalExceptionData` shape for external-function error paths (#349). Extension has no competing design here.
4. **Imports**: union every side. Extension adds `annotated_to_py_structured`, `py_to_annotated`, `rust_meta_to_py`, `CallbackStructuredPrint`, `resolve_print_callback`, `unwrap_structured_callback`, `with_print_writer`. Upstream adds `ExcType`, `JsonMontyArray`, `JsonMontyObject`, `JsonMontyPairs`, `py_type_check`, `call_os_callback_parts`, `PrintTarget`.

Do not delete either side's helper functions. They serve different purposes.

#### `crates/monty-python/src/repl.rs` (11 conflicts)

Mirrors `monty_cls.rs` — same parameter-union recipe, same
print-callback integration. Additionally:

- **`TypeCheckState`** (upstream): keep the struct and `Option<Mutex<TypeCheckState>>` field on `PyMontyRepl`.
- **`dump()` / `load()`**: serialize `type_check_state` alongside extension's metadata-aware REPL state. Preserve upstream's `SerializedReplOwnedLegacy` fallback so REPL snapshots from 0.0.13 still load.
- **`put_repl_after_commit` / `put_repl_after_rollback`** (upstream): keep both. Extension's code path currently uses `put_repl` directly — switch to `put_repl_after_commit` on success paths and `put_repl_after_rollback` on restore-from-error paths.

#### `crates/monty-python/src/async_dispatch.rs` (4 conflicts)

Upstream adds `ReplCleanupNotifier`, `await_repl_transition`,
`dispatch_loop_repl`. Extension adds `with_print_writer`. Keep all.
Conflicts are mostly in function signatures that thread a print writer
through async resume paths; union them.

#### `crates/monty-python/src/serialization.rs` (1 conflict)

Small — extension adds metadata to the REPL serialization schema,
upstream adds `type_check_state`. Combine into a single new schema
version.

#### Python surface — `__init__.py` (2), `_monty.pyi` (12), `test_print.py` (1)

- **`__init__.py`**: union exports — `CollectString`, `CollectStreams`, `NOT_HANDLED` (upstream) + `NonSerializable`, `ObjectMetadata`, `AnnotatedValue`, `UNIVERSAL` (extension).
- **`_monty.pyi`**: union declarations. Stub each upstream class and each extension class. For `MontyRepl.__new__` and each `feed_*` method, use the unified signature from the Rust resolution.
- **`test_print.py`**: **move extension-only tests to `ext_tests/`** (see below). Accept upstream's changes to the file and re-add our specific test cases under `ext_tests/test_structured_print_callback.py`.

## New rule: extension tests live in `ext_tests/`

Starting with this merge, any Python test that **only** exercises
tiny-beaver-ext functionality (structured print callback, metadata,
NonSerializable, universal-set sentinel) belongs in the top-level
`ext_tests/` directory — not in `crates/monty-python/tests/`.

Rationale:
- Reduces conflict surface on future upstream merges.
- Makes extension coverage visible as a separate test target.
- Upstream tests stay verbatim from `main`, making future merges nearly conflict-free in `crates/monty-python/tests/`.

Action items for this merge:
1. Create `ext_tests/` with a `conftest.py` if fixtures are needed.
2. Move extension-specific assertions out of `test_print.py`,
   `test_repl.py`, `test_serialize.py`, `test_start.py` into
   `ext_tests/test_<feature>.py` files.
3. Add a `make ext-test` target (or extend `make pytest`) that runs
   both `crates/monty-python/tests/` and `ext_tests/`.
4. Document the split in `docs/extensions/README.md` under "Build & Test".

## Execution order when we re-run the merge

1. `git checkout cz/merge-main/2026-04-17 && git merge main`
2. Resolve in this order (low risk → high risk):
   1. `crates/monty/src/lib.rs`, `object.rs`, `tests/print_writer.rs` — mechanical unions.
   2. `crates/monty/src/builtins/zip.rs` — small, reviewed above.
   3. `crates/monty/src/bytecode/vm/mod.rs` — remove `cleanup()`, verify Drop sufficiency.
   4. `crates/monty/src/repl.rs` — run the Python script above.
   5. `crates/monty-python/src/lib.rs` — union exports.
   6. `crates/monty-python/src/serialization.rs` — single schema merge.
   7. `crates/monty-python/src/async_dispatch.rs` — signature unions.
   8. `crates/monty-python/src/monty_cls.rs` — the big one; follow per-method recipe above; commit in logical sub-steps (e.g., one commit per method family).
   9. `crates/monty-python/src/repl.rs` — same recipe as `monty_cls.rs`.
   10. Python surface: `_monty.pyi`, `__init__.py`.
   11. Tests: move extension-only tests to `ext_tests/`, adopt upstream's test files verbatim elsewhere.
3. Run, in order:
   - `make format-rs && make lint-rs` — fix until clean.
   - `make dev-py && make pytest` — upstream tests must pass.
   - `make ext-test` (or equivalent) — extension tests must pass.
   - `make test-ref-count-panic` — sanity check core.
4. Write a follow-up doc: `docs/extensions/sync-fork/2026-04-17-merge-notes.md` capturing any surprises and their resolutions, so the next merge is faster.

## Known risks to watch for

- **Scheduler + Drop + snapshot()**: upstream's `Drop for VM` runs after `snapshot()` returns, so every field moved out of `self` in `snapshot()` must be replaced via `mem::take`, not direct move. Double-check extension's metadata fields are handled this way.
- **REPL `put_repl` → commit/rollback split**: using the wrong variant on an error path will commit a type-check snippet that shouldn't have committed. The snippet stays visible to later `type_check` calls even after the code that defined it failed.
- **`structured_print_callback` + `CollectString`**: the collectors are upstream's classes; our structured callback is extension's. If both are supplied, we should fail loudly — they're incompatible routes. Add a clear `TypeError` at `resolve_print_callback`.
- **External function error path (#349)**: extension's external-function-error handling predates `ExternalExceptionData`. Audit every `ExtFunctionResult::Error` construction site after the merge to confirm it produces the new shape.
- **`MontyIter::new` signature**: extension added a `meta` parameter. Upstream has not touched it, but check no upstream-auto-merged call site calls `MontyIter::new(value, vm)` with two arguments.

## Deliverables on completion

- Merge commit on `cz/merge-main/2026-04-17`.
- Updated `docs/extensions/implemented/*.md` for each extension whose
  surface changed (e.g., `structured-print-callback.md` must note its
  interaction with upstream's `CollectString`/`CollectStreams`).
- `ext_tests/` populated per the rule above.
- `docs/extensions/sync-fork/2026-04-17-merge-notes.md` summarizing
  actual decisions vs. this plan.

---

# Fresh-session handoff (2026-04-17, pause point)

## Working-tree state at pause

Merge is in progress on `cz/merge-main/2026-04-17`; do NOT `git merge --abort`.
`git status` should show these unmerged paths (**only these two remain**):

- `crates/monty-python/src/monty_cls.rs` — 28 conflict hunks
- `crates/monty-python/src/repl.rs` — 11 conflict hunks

All other previously conflicted files are already conflict-free in the working tree
(but not yet `git add`ed). Do not run `git add` on them until `monty_cls.rs` +
`repl.rs` compile — keeping them unstaged makes it trivial to diff against `HEAD`
and `main` for cross-checking.

Already-resolved files:
- `crates/monty/src/builtins/zip.rs`
- `crates/monty/src/object.rs`
- `crates/monty/src/lib.rs`
- `crates/monty/src/bytecode/vm/mod.rs`
- `crates/monty/src/repl.rs`
- `crates/monty/tests/print_writer.rs`
- `crates/monty-python/src/lib.rs`
- `crates/monty-python/src/async_dispatch.rs`
- `crates/monty-python/src/serialization.rs`
- `crates/monty-python/python/pydantic_monty/__init__.py`
- `crates/monty-python/python/pydantic_monty/_monty.pyi`
- `crates/monty-python/tests/test_print.py` (extension-only tests moved out)

New files created (untracked):
- `ext_tests/test_print_structured.py` — the extension structured-print tests
  extracted from `tests/test_print.py`. Verify after merge compiles that
  `pytest ext_tests/` works from the repo root.

## Required design decision (apply before touching any remaining hunk)

Upstream introduced `PrintTarget` in `crates/monty-python/src/print_target.rs` as
the single thread-through value for print routing. Extension's
`structured_print_callback` predates it and uses a parallel
`CallbackStructuredPrint` / `StructuredCallbackMarker` / `resolve_print_callback`
/ `unwrap_structured_callback` pipeline embedded in `monty_cls.rs`.

**Decision: extend `PrintTarget` to be the single source of truth for both string
and structured callbacks.** Concretely, in `print_target.rs`:

1. Add a new variant:
   ```rust
   StructuredCallback { callback: Py<PyAny>, dc_registry: DcRegistry },
   ```
2. Add a new constructor that replaces `from_py` at the public API boundary:
   ```rust
   pub fn from_py_args(
       print_callback: Option<&Bound<'_, PyAny>>,
       structured_print_callback: Option<&Bound<'_, PyAny>>,
       dc_registry: &DcRegistry,
   ) -> PyResult<Self>
   ```
   - If both are `Some`: `ValueError("cannot specify both 'print_callback' and 'structured_print_callback'")`.
   - If `structured_print_callback` is `Some`: `StructuredCallback { … }`.
   - Else: delegate to the existing `from_py(print_callback)` logic.
3. Extend `PrintStorage` + `with_writer` + `storage` + `clone_handle` /
   `clone_handle_detached` to handle the new variant. The storage variant holds
   the same `CallbackStructuredPrint` struct that currently lives in
   `monty_cls.rs` (move it into `print_target.rs`).
4. Delete from `monty_cls.rs`: `CallbackStringPrint`, `CallbackStructuredPrint`,
   `StructuredCallbackMarker`, `wrap_structured_callback`,
   `unwrap_structured_callback`, `resolve_print_callback`. (Upstream already
   removed the need for them; the new `PrintTarget` variant subsumes all of
   them.) `async_dispatch.rs` and `repl.rs` currently import these — update
   those imports to the new `PrintTarget` method calls.
5. For the `with_print_writer` helper in `async_dispatch.rs`: replace the
   `Option<Py<PyAny>>` + `unwrap_structured_callback` dance with a
   `&PrintTarget`-taking helper, or remove it entirely in favor of
   `PrintTarget::with_writer`.

Why this is better than preserving the extension's parallel pipeline:
- Single surface; no `StructuredCallbackMarker` wrapper class threaded through
  snapshots as `Option<Py<PyAny>>`.
- `CollectStreams` / `CollectString` and `StructuredCallback` live in the same
  enum, so "both set" validation is natural (`from_py_args` checks it once).
- Snapshot serialization already has to skip `PrintTarget` — the new variant
  inherits that. The old `StructuredCallbackMarker` `Py<PyAny>` threading
  conflicts with upstream's `print_target: PrintTarget` field shape anyway.

## Second design decision: input metadata threading

Upstream changed `run_impl` / `runner.run` / `runner.start` to take
`Vec<MontyObject>`. Extension passes `Vec<AnnotatedObject>` to preserve input
metadata. **Keep extension's `Vec<AnnotatedObject>`** — input metadata is a
first-class extension feature (see `docs/extensions/implemented/metadata-propagation.md`).

Where upstream's signature appears in already-resolved code, revert to the
extension's form:
- `fn run_impl` — `input_values: Vec<::monty::AnnotatedObject>`
- In the `RunProgress::Complete(annotated)` branch, use `&annotated.value` (not
  a bare `&result`).

This is straightforward because the surrounding code already has both names in
scope; the hunks are short local swaps.

## Mechanical conflict-resolution recipe (after the two decisions above are coded)

For each remaining conflict hunk in `monty_cls.rs` and `repl.rs`:

1. Take **main's** side as the base (it has mount/os/type_check/PrintTarget).
2. Add back the `structured_print_callback: Option<&Bound<'_, PyAny>>` parameter
   alongside `print_callback` (same position, right after `print_callback`).
3. Replace `PrintTarget::from_py(print_callback)?` with
   `PrintTarget::from_py_args(print_callback, structured_print_callback, &self.dc_registry)?`.
4. Where the method signature uses `Vec<MontyObject>` for input values, change
   to `Vec<::monty::AnnotatedObject>`.
5. For the `RunProgress::Complete` match arm, use the annotated form.
6. Update the `.pyi` already matches — no further changes needed there.

The final monty_cls.rs conflict (lines 2284–2450, the whole block of extension
callback structs) should be resolved by taking **main's side** (empty), because
step 4 of the "Required design decision" moves those structs into
`print_target.rs`.

## Callsite audit to run after conflicts are resolved

Grep and verify:
- `CallbackStringPrint` — should only exist inside `print_target.rs`.
- `CallbackStructuredPrint` — same.
- `StructuredCallbackMarker` / `wrap_structured_callback` / `unwrap_structured_callback` — **zero hits**; removed entirely.
- `resolve_print_callback` — replaced by `PrintTarget::from_py_args`.
- `Vec<MontyObject>` in `monty_cls.rs` / `repl.rs` run/start paths —
  **zero hits**; use `Vec<::monty::AnnotatedObject>`.
- `with_print_writer` — either removed or takes `&PrintTarget`.
- `MontyIter::new` — called with 3 args everywhere (`value, vm, meta`).

## Post-merge validation checklist

```bash
make format-rs
make lint-rs                    # expect zero warnings
make test-ref-count-panic       # core Rust tests
make dev-py                     # build Python package
make pytest                     # main Python test suite (unchanged tests)
uv run pytest ext_tests/        # extension-only tests
make test-cases
```

If any `ext_tests/` test references symbols not present in `pydantic_monty`
(e.g. `AnnotatedValue` alias), that's expected — `ext_tests/test_print_structured.py`
was extracted verbatim from the HEAD block; update imports if needed.

## Follow-up docs to write after merge is green

- `docs/extensions/sync-fork/2026-04-17-merge-notes.md` — actual decisions vs.
  this plan, surprises, deferred items.
- Update `docs/extensions/implemented/structured-print-callback.md` to reflect
  the new `PrintTarget::StructuredCallback` implementation and the `ValueError`
  when both callbacks are set.
- Update `docs/extensions/implemented/metadata-propagation.md` if input-threading
  details changed.
