# 2026-04-26 Merge Plan: `main` → `tiny-beaver-ext`

Integrating 19 new upstream commits (post-`v0.0.14` → `v0.0.17`) into the
extension branch. This document is the operational plan — read it before
re-running any merge step on the staging branch.

## Upstream commits being merged

`git log a4be566..main` on `tiny-beaver-ext`. The previous sync (PR #12)
landed through PR #271 / commit `a4be566`; everything below is new.

### Features / behavior

| PR    | Commit     | What it adds                                                              |
|-------|------------|---------------------------------------------------------------------------|
| #356  | `e425e31`  | Wrap input unicode-conversion errors as `MontyRuntimeError`                |
| #357  | `e50f8eb`  | Support chain assignment (`a = b = c = expr`)                              |
| #358  | `e777ebf`  | Async `Monty` instance build without holding the GIL                       |
| #360  | `ba26c2e`  | Input safety — typed validation in `convert.rs` / `build.rs`               |
| #67   | `d5444c6`  | Implement `setattr` builtin                                                 |
| #344  | `8aa308e`  | monty-js: limits above `u32::MAX` (max memory, allocations, recursion, GC) |

### Fixes

| PR    | Commit     | What it fixes                                                                  |
|-------|------------|--------------------------------------------------------------------------------|
| #361  | `4dc46f2`  | Rename `prefix_code` → `type_check_stubs` (pyi + Rust + Python tests)           |
| #363  | `4a942bb`  | Stop empty-tuple singleton from contributing to memory-limit accounting         |
| #368  | `9e36ce3`  | Fix panics on overflow when negating `i64::MIN` (slice/bytes/list/range/str)    |
| #371  | `3890e83`  | GC interval ignored in `ResourceTracker` and `Heap`                             |
| #376  | `bbf6313`  | GC coverage tracking through scheduler / args / run_progress                    |
| #379  | `562507b`  | Skip a few monty-js tests on WASI                                               |

### Refactor / perf

| PR    | Commit     | What it changes                                                                  |
|-------|------------|----------------------------------------------------------------------------------|
| #373  | `08817fc`  | Move hashing into `PyTrait` (delete 200 lines from `heap.rs`, add per-type impls) |
| #375  | `f10735f`  | Switch test assertions to `insta` for `tests/{repl,json_output,json_serde,print_writer}.rs` |
| #381  | `c026c34`  | More GC coverage — touches `vm/mod.rs`, `vm/async_exec.rs`, `heap.rs`, type files |

### Infra

| PR    | Commit     | What it does                                                                     |
|-------|------------|----------------------------------------------------------------------------------|
| #392  | `33e47c6`  | Switch `pre-commit` → `prek` in CI + Makefile + `check_imports.py` + tiny touch-ups |

### Releases

`2dc8659` (v0.0.15), `142807b` (v0.0.16), `5c7cf2b` (v0.0.17) — Cargo /
package-lock version bumps.

## Two design decisions for this merge

### 1. Metadata is orthogonal to hash and equality

Locked in by the user. Implications:

- Every per-type `py_hash` impl from `08817fc` lands verbatim — none of them
  read `MetadataId`, `meta_globals`, or `meta_stack`, and that property must
  not regress.
- Every per-type `py_eq` impl in the extension must also ignore metadata
  (otherwise the `a == b ⇒ hash(a) == hash(b)` invariant breaks). Audit grep
  during PR-C: zero hits for metadata reads inside `py_eq` / `py_hash`
  bodies.
- Two values with different `producers` / `consumers` / `tags` are equal,
  hash equal, dedupe in sets, and find each other in dicts. Provenance is
  observability metadata, not identity.
- Lock in via a new `ext_tests/test_metadata_hash_invariant.py`:
  `hash("hi"@A) == hash("hi"@B)`, `len({"hi"@A, "hi"@B}) == 1`,
  `d["k"@A] = v; d["k"@B]` returns `v`.

### 2. Dict key metadata is preserved

Locked in by the user. Already implemented today: `crates/monty/src/types/dict.rs`
stores `key_meta: MetadataId` alongside `value_meta` in `DictEntry`, and
`Dict::set_with_meta` writes both. The merge must not regress this.

Verify during PR-C:

- `c026c34` touches `types/dict.rs` for ~4 lines — read the diff before
  applying; confirm `set_with_meta` and `value_meta_for_key` paths are
  untouched.
- Lock in via `ext_tests/test_metadata_dict_keys.py`: after `d["k"@A] = v`,
  the stored key metadata returns `A`; subsequent `d["k"@B]` *lookup*
  succeeds and returns `v` (lookup ignores key metadata). Pin down whatever
  `set_with_meta` does on re-insert — keep current behavior, don't change it.

## Branch model

```
tiny-beaver-ext
  └── cz/sync-fork/2026-04-26          ← staging branch
        ├── cz/sync-fork/2026-04-26-A  ← PR-A: trivia + JS-only
        ├── cz/sync-fork/2026-04-26-B  ← PR-B: self-contained core fixes
        ├── cz/sync-fork/2026-04-26-C  ← PR-C: internal refactors
        └── cz/sync-fork/2026-04-26-D  ← PR-D: public-API reshape
```

(Dash-separated, not slash-separated — git refuses `foo/bar` as a child of an
existing `foo` ref.)

Each feature branch merges (squash) into the staging branch. When all four
land green and `make ext-test` passes on staging, fast-forward
`tiny-beaver-ext` to the staging tip in one step. Mirrors the prior sync
(`cz/fork-sync/2026-04-18` → `tiny-beaver-ext`).

## Per-PR contents

### PR-A — Trivia + JS-only

Commits (low-risk, no extension touch points):

- `2dc8659` v0.0.15 uprev
- `142807b` v0.0.16 uprev
- `5c7cf2b` v0.0.17 uprev (this is the version we land at)
- `562507b` skip some tests on WASI
- `8aa308e` monty-js limits above `u32::MAX`
- `33e47c6` `pre-commit` → `prek`

Approach: cherry-pick in the order above. The three uprev commits collapse
naturally into the v0.0.17 endpoint. `33e47c6` touches comments scattered
across many files (`heap_entries.rs`, `vm/op.rs`, `fstring.rs`, etc.) — none
should conflict with extension code, but eyeball the diff before committing.

**Validation gate (per PR):**

```bash
make format-rs
make lint-rs                    # zero warnings
make test-ref-count-panic
make dev-py
make pytest
make test-cases
uv run pytest ext_tests/
```

### PR-B — Self-contained core fixes

Commits (no parameter-shape changes, narrow file scope):

- `4a942bb` empty-tuple singleton off memory limit (16 lines, `heap.rs` only)
- `4dc46f2` `prefix_code` → `type_check_stubs` (pure rename)
- `d5444c6` `setattr` builtin
- `3890e83` GC interval fix (heap/resource/run/limits)
- `9e36ce3` `i64::MIN` negation refactor (slice/bytes/list/range/str/tuple)

Watch-points:

- **`9e36ce3`** rewrote slicing across 5 type files. The extension's
  metadata propagation lives in `__getitem__` slice paths — audit each site
  to confirm `result_meta = container_meta ∪ index_meta` (or the extension's
  precise rule) still holds. Add a test to `ext_tests/test_metadata_slicing.py`
  if one doesn't already exist.
- **`d5444c6`** (`setattr`): the new builtin must thread metadata. Pattern:
  the `value` operand's `MetadataId` should land on the attribute target
  exactly as `__setitem__` does. Add a test asserting
  `setattr(obj, "x", tainted)` makes `obj.x.metadata == tainted.metadata`.
- **`3890e83`** touches `heap.rs`. The extension's panic-tolerant
  `inc_ref/dec_ref` (commit `1e758b5` on `tiny-beaver-ext`) lives there;
  verify the GC-interval check is orthogonal to the unwind-safety logic.
- **`4dc46f2`** rename and **`4a942bb`** empty-tuple are mechanical.

### PR-C — Internal refactors

Commits (interface-changing but not user-facing):

- `08817fc` move hashing into `PyTrait`
- `bbf6313` fix gc coverage (vm/mod.rs, scheduler.rs, args.rs, run_progress.rs)
- `c026c34` more gc coverage (vm/mod.rs, vm/async_exec.rs, heap.rs, types/*)
- `f10735f` switch tests to `insta`

Watch-points:

- **`08817fc`** — see "Two design decisions" §1. Zero metadata reads in
  any new `py_hash` impl.
- **`bbf6313` + `c026c34`** both touch `vm/mod.rs` and `run_progress.rs` —
  same files where the 2026-04-19 sync had to destructure
  `ExtFunctionResult::Return(obj, _meta)` and use `take_globals_with_meta`.
  Reuse that recipe. Specifically `bbf6313` adds entries to `args.rs` and
  `vm/scheduler.rs` — confirm the parallel meta stacks (`meta_stack`,
  `meta_globals`, `meta_exception_stack`) remain invariants under any new
  scheduler entry/exit point.
- **`c026c34`** touches `dict.rs` (4 lines) — see "Two design decisions" §2,
  do not regress `set_with_meta` / `key_meta` storage.
- **`f10735f`** — adopt insta in upstream-owned tests, but preserve the
  extension's bespoke tests in `tests/repl.rs`
  (`call_function_preserves_global_metadata` etc.). Convert those to insta
  too if convenient, otherwise leave as plain `assert_eq!`.

### PR-D — Public-API reshape

Commits (parameters thread through every Monty/MontyRepl entry point):

- `e425e31` wrap input unicode errors as `MontyRuntimeError`
- `e50f8eb` chain assignment
- `e777ebf` async Monty build without GIL
- `ba26c2e` input safety

Approach: this is the equivalent of the 2026-04-17 `monty_cls.rs` reshape.
Reuse the per-method recipe from `2026-04-17-merge-plan.md` §"Mechanical
conflict-resolution recipe":

1. Take main's signature as the base.
2. Add back `structured_print_callback: Option<&Bound<'_, PyAny>>` next to
   `print_callback` in every `#[pymethods]` signature.
3. Replace `PrintTarget::from_py(print_callback)?` with
   `PrintTarget::from_py_args(print_callback, structured_print_callback,
   &self.dc_registry)?`.
4. Where the method signature uses `Vec<MontyObject>` for input values,
   change to `Vec<::monty::AnnotatedObject>` (extension's metadata-aware
   shape).
5. For the `RunProgress::Complete` match arm, use the annotated form
   (`&annotated.value`).
6. The `.pyi` should already match — no further changes needed there.

Watch-points:

- **`e777ebf` (async build)** adds `build.rs` with a 203-line async
  constructor + reshapes `monty_cls.rs` (~144) and `repl.rs` (~46). The new
  constructor must accept and store the extension's params:
  `structured_print_callback`, metadata-aware `dc_registry`, and seed
  `MetadataStore` from input metadata. Land this commit *first* in PR-D so
  every later commit slots into the new constructor shape.
- **`ba26c2e` (input safety)** rewrites `convert.rs` (`py_to_monty`) and
  validation in `build.rs`. Extension uses `py_to_annotated` for
  metadata-aware conversion — preserve that pathway. Validation logic should
  layer on top of the annotated path; do not regress to `py_to_monty`.
- **`e50f8eb` (chain assignment)** is parser/compiler. Audit: does the new
  compiler path emit ops that propagate the same `MetadataId` to all
  assignment targets? `a = b = c = expr` should give all three the same
  metadata. Add `crates/monty/test_cases/assign__chain_metadata.py` to lock
  this in (use the extension's metadata test harness — see
  `ext_tests/`).
- **`e425e31` (unicode error wrapping)** changes error variants across
  `convert.rs`, `external.rs`, `monty_cls.rs`, `repl.rs`, `async_dispatch.rs`.
  Mechanical union with the extension's existing error sites; no design
  conflict.

## Execution order

For each PR:

1. `git checkout cz/sync-fork/2026-04-26 && git checkout -b cz/sync-fork/2026-04-26-<X>`
2. Cherry-pick (or merge) the planned commits in the order listed in the
   "Per-PR contents" section.
3. Resolve conflicts per the watch-points above.
4. Run the validation gate (see PR-A).
5. Open PR against `cz/sync-fork/2026-04-26` (NOT `tiny-beaver-ext`).
6. Squash-merge into staging.

After PR-D is merged, run the validation gate one more time on the staging
tip, then fast-forward `tiny-beaver-ext`:

```bash
git checkout tiny-beaver-ext
git merge --ff-only cz/sync-fork/2026-04-26
git push origin tiny-beaver-ext
```

## Deliverables on completion

- One squash-merge commit per PR on `cz/sync-fork/2026-04-26`.
- Fast-forward of `tiny-beaver-ext` to the staging tip.
- New `ext_tests/`:
  - `test_metadata_hash_invariant.py` (PR-C)
  - `test_metadata_dict_keys.py` (PR-C)
  - `test_metadata_slicing.py` (PR-B, if missing)
  - `test_metadata_setattr.py` (PR-B)
  - `test_metadata_chain_assignment.py` (PR-D, or as a Monty `test_cases/` file)
- `docs/extensions/sync-fork/2026-04-26-merge-notes.md` after the merge,
  capturing actual decisions vs. this plan, surprises, and deferred items.
- `docs/extensions/README.md` "Implemented Extensions" table updated if any
  extension's branch / surface changed.
