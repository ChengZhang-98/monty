# 2026-04-26 upstream merge notes

Completion notes for the `main` → `tiny-beaver-ext` sync planned in
`2026-04-26-merge-plan.md`. Staging branch: `cz/sync-fork/2026-04-26`.

## Final shape

19 upstream commits (post-`v0.0.14` → `v0.0.17`, all of `git log a4be566..main`)
landed across **4 stacked PRs**:

```
tiny-beaver-ext
  └── cz/sync-fork/2026-04-26              ← staging (plan + notes only)
        ├── cz/sync-fork/2026-04-26-A      ← off staging (independent)
        └── cz/sync-fork/2026-04-26-B      ← off staging
              └── cz/sync-fork/2026-04-26-C  ← off PR-B
                    └── cz/sync-fork/2026-04-26-D  ← off PR-C
```

Why stacked rather than independent: PR-B's `4a942bb` (empty-tuple singleton
off memory limit) and PR-C's `08817fc` (move hashing into `PyTrait`) both
rewrite the same block in `crates/monty/src/heap.rs` (the empty-tuple
singleton initialization). Branching PR-C off staging produced a real
conflict; off PR-B's tip auto-merges cleanly. PR-D's `4dc46f2` rename
likewise depends on `e777ebf`'s new `build.rs` (in PR-D itself), but PR-D
also touches the `monty_cls.rs` / `repl.rs` shape PR-C reshaped via
`08817fc`, so stacking PR-D on PR-C avoided redoing that work.

## Per-PR contents

### PR-A — Trivia + JS-only (6 commits)

`2dc8659`, `142807b`, `5c7cf2b` (uprevs to v0.0.15 → v0.0.16 → v0.0.17),
`562507b` (skip WASI tests), `8aa308e` (monty-js limits >`u32::MAX`),
`33e47c6` (`pre-commit` → `prek`).

Conflict: `33e47c6`'s prek migration hit two textual conflicts in
`heap_entries.rs` and `object.rs` — both stale `HeapValue` references
(extension's name) vs upstream's renamed `HeapEntry`, plus an extension-only
`use crate::types::Dataclass;` local import. Resolved by:
- Keeping extension's `HeapValue` in `heap_entries.rs` test fn (rename was
  later reverted by upstream in PR-C, so keeping it here was correct).
- Union of imports in `object.rs`: kept `Tuple` (used by extension's
  `Tuple::new_with_metadata`), added `Dataclass` from local-use hoist, did
  not adopt upstream's `allocate_tuple` (extension code path doesn't use it).

### PR-B — Self-contained core fixes (4 commits)

`d5444c6` (setattr), `4a942bb` (empty-tuple singleton), `9e36ce3`
(`i64::MIN` slice refactor), `3890e83` (GC interval fix). The
`4dc46f2` rename was originally planned here but moved to PR-D — it
modifies `crates/monty-python/src/build.rs`, which is created by
`e777ebf` in PR-D.

Resolutions:
- **`9e36ce3` slice refactor**: extension's pre-existing `get_slice_items`
  + `get_slice_metadata` helpers had the same `i64::MIN.unsigned_abs()`
  panic bug upstream just fixed. Deleted both. `list.rs::getitem_slice`
  now calls upstream's overflow-safe `slice_collect_iterator` twice in
  parallel — once for `items`, once for `item_metadata` — so per-element
  metadata propagation is preserved.
- **`3890e83` resource_limits.rs test rewrite**: upstream rewrote
  `gc_interval_triggers_collection` and added `gc_interval_limit_is_respected`.
  Took upstream's body for the rewritten test (matched the assertions on
  `output.allocations_since_gc` / `output.heap_count` below the conflict).
  Renamed `Executor::run_ref_counts` → `run_ref_counts_with_tracker`
  matching upstream's split, but kept extension's `Vec<AnnotatedObject>`
  input shape; `MontyRun::run_ref_counts_with_tracker` (public) takes
  `Vec<impl Into<AnnotatedObject>>` for inference ergonomics matching
  `MontyRun::run`.
- **Memory-limit byte counts** in `resource_limits.rs`: upstream's
  `4a942bb` reduced the empty-tuple singleton's 104-byte contribution.
  Took upstream's reduced numbers (500000, 3661668, 3661668, 1000000) on
  the assumption that the singleton accounting is shared between extension
  and upstream — confirmed correct by tests passing post-merge.

### PR-C — Internal refactors (5 commits)

`08817fc` (move hashing into `PyTrait`), `bbf6313` (gc coverage),
`c026c34` (more gc coverage, including `ref-count-panic` →
`memory-model-checks` rename), `f10735f` (insta migration), plus a new
ext_tests commit (`test_metadata_hash_invariant.py`,
`test_metadata_dict_keys.py`).

Resolutions:
- **`08817fc` `HeapValue` → `HeapEntry` rename**: upstream standardized
  on `HeapEntry`. Extension's `1e758b5` (panic-tolerant `try_get` from
  PR #13) had a stale `HeapValue` return type. Bulk-renamed all
  `HeapValue` → `HeapEntry` references in `heap.rs` and `heap_entries.rs`
  (one method signature + 8 doc comments). Folded into the cherry-pick.
- **`08817fc` heap.rs:355 safety comment**: union — adopted upstream's
  added "or `collect_garbage`" guarantee, kept extension's `HeapEntry`
  name.
- **`c026c34` monty-datatest signature change**: upstream introduced
  `build_test_limits(gc_interval)` and added `gc_interval: Option<usize>`
  parameter to `run_iter_loop` / `run_mount_fs_iter_loop`. Took upstream's
  signatures + helper, kept extension's `Vec::<monty::MontyObject>::new()`
  over upstream's `vec![]` (the merge-plan-noted type-inference workaround
  for `impl Into<AnnotatedObject>`).
- **`f10735f` insta migration**: 35 hunks across `tests/{json_output,
  json_serde, print_writer}.rs`. Each test had two divergences — extension's
  metadata-aware container shapes (`Vec<AnnotatedObject>`,
  `MontyObject::Repr` struct form, JSON envelopes with `{"value":..., "metadata":null}`)
  AND upstream's `assert_snapshot!` macro. Took union: kept extension's
  shapes / metadata envelopes, adopted `assert_snapshot!` and the `eval()`
  helper. `print_writer.rs` (20 hunks, mechanical) resolved via Python
  script: take upstream's side everywhere, then patch the helper's
  `vec![]` → `Vec::<monty::MontyObject>::new()`.

### PR-D — Public-API reshape (7 commits)

`e425e31` (unicode error wrapping), `e50f8eb` (chain assignment),
`e777ebf` (async Monty build), `ba26c2e` (input safety), `4dc46f2`
(rename), plus two ext_tests commits
(`test_metadata_slicing.py` + `test_metadata_setattr.py`,
`test_metadata_chain_assignment.py`).

Resolutions:
- **`e425e31` `py_to_annotated` enhancement**: extension's `py_to_annotated`
  was using raw `py_to_monty` and would leak `UnicodeEncodeError` for
  lone-surrogate strings. Updated `metadata.rs::py_to_annotated` to route
  through `py_to_monty_value` internally — extension's input path now has
  the same error-wrapping safety as upstream's, without changing
  `py_to_annotated`'s callsite signature. Single-source of truth.
- **`e425e31` `ExtFunctionResult::Return` shape**: kept extension's
  `(obj, None)` shape (where `None` is the metadata field) at all
  conversion sites in `external.rs` and `async_dispatch.rs`. Removed
  obsolete `py: Python<'_>` arg from `py_obj_to_ext_result` since
  `py_to_monty_value` handles error wrapping internally.
- **`e777ebf` `build.rs` extraction**: upstream moved `extract_source_code`
  and `py_type_check` from `monty_cls.rs` to a new `build.rs`. Adopted
  the new import path everywhere. Did not adopt upstream's
  `convert::py_to_monty_value` import in `repl.rs` — extension's
  `metadata::py_to_annotated` is the canonical input-conversion entry
  point and now wraps via `py_to_monty_value` internally.
- **`ba26c2e` `convert::py_to_monty` depth parameter** (the largest
  semantic conflict): upstream added `depth: u8` (max 200) AND switched
  `MontyObject::List/Tuple/Set/FrozenSet` element types from
  `Vec<AnnotatedObject>` (extension) back to `Vec<MontyObject>` (upstream's
  revert). Per merge-plan §"input metadata threading", kept extension's
  `Vec<AnnotatedObject>` shape AND threaded the new `depth` parameter:
  every container conversion is now
  `.map(|item| py_to_monty(&item, dc_registry, depth).map(AnnotatedObject::from))`.
- **`ba26c2e` `extract_input_values` rewrite**: upstream cleaned the
  function to a pure if/else. Adopted upstream's structure, kept
  extension's `py_to_annotated` (instead of upstream's
  `py_to_monty_value` + manual error wrap) since it now wraps internally.
- **`4dc46f2` rename**: clean auto-merge after `e777ebf` and `ba26c2e`
  landed.

## Two design decisions confirmed under execution

### §1 Metadata is orthogonal to hash and equality

Locked in by user. Pinned by `ext_tests/test_metadata_hash_invariant.py`
(5 tests, all passing): two strings with different metadata are equal,
hash-equal, dedupe in sets, and find each other in dicts. Two ints with
different metadata dedupe in sets. Tuple `(a, 1) == (b, 1)` when `a == b`
regardless of element metadata.

Programmatic AST-style scan over every `py_hash` and `py_eq` body in
`crates/monty/src/types/*.rs` and `heap_data.rs` after PR-C: zero
references to `MetadataId`, `meta_globals`, `meta_stack`, `metadata_store`,
or `.item_metadata` inside any hash/equality body.

### §2 Dict key metadata is preserved

Locked in by user. Pinned by `ext_tests/test_metadata_dict_keys.py`
(3 tests, all passing): key metadata survives `.keys()` iteration; lookup
ignores key metadata; re-insert behavior pinned (last write wins).

`Dict::set_with_meta` still callable at 8 sites in `dict.rs`. `c026c34`'s
4-line touch on `dict.rs` was a refcount-iteration tweak unrelated to key
metadata storage.

## Validation results

After final rebase and squash on each branch:

| Suite | Result |
|---|---|
| `make test-memory-model-checks` (cargo + datatest) | **1689 passed, 0 failed** |
| `make test-ref-count-return` | **1708 passed, 0 failed** |
| `make test-no-features` | **1704 passed, 0 failed** |
| `make test-type-checking` | **21 passed, 0 failed** |
| `make test-cases` (datatest) | **934 passed, 0 failed** |
| `make pytest` (upstream Python tests) | **1100 passed, 1 skipped, 1 xfailed** |
| `uv run pytest ext_tests/` | **38 passed, 0 failed** |
| `make lint-rs` (clippy + import-check) | clean |
| `cargo fmt --check` | clean |
| `ruff format --check` + `ruff check` | clean |

`make lint-py` failed on a `nodejs-wheel` permission issue (pyright
stubtest binary) unrelated to any code change in this sync. Direct ruff
runs both pass.

## Real bugs caught by validation

Three test bugs slipped past structural review and surfaced only at first
execution. All three were folded back into the PR they belong to via
`git rebase --autosquash`:

1. **`tests/resource_limits.rs::memory_limit_zero` `vec![]` → E0283**
   (origin: PR-B's `9e36ce3` cherry-pick; folded into `2e847bb`).
   Empty `vec![]` cannot infer element type for extension's
   `Vec<impl Into<AnnotatedObject>>` signature. Fixed to
   `Vec::<MontyObject>::new()`.

2. **`tests/resource_limits.rs::gc_interval_limit_is_respected` `vec![]`
   → E0283** (origin: PR-B's `3890e83` cherry-pick; folded into
   `2e847bb`). Same root cause as #1, on the
   `run_ref_counts_with_tracker` callsite.

3. **`tests/asyncio.rs::suspended_task_stack_survives_forced_gc` shape**
   (origin: PR-C's `bbf6313` cherry-pick; folded into `a2bf0c6`). New test
   from upstream used `ExtFunctionResult::Return(obj)` (single-arg) and
   `MontyObject::List(vec![MontyObject::Int(3), ...])` (bare elements).
   Updated to extension's `ExtFunctionResult::Return(obj, None)` and
   `Vec<AnnotatedObject>` shape, plus `assert_eq!(result.value, ...)`
   since `into_complete()` returns `AnnotatedObject`. Also fixed
   `runner.start(vec![], ...)` → `runner.start(Vec::<MontyObject>::new(), ...)`.

Two ext_tests authoring bugs also surfaced and were folded back into the
ext_tests commits where they originated:

4. **`ext_tests/test_metadata_setattr.py` inline `class` syntax**
   (folded into `1047b50`). Test defined the dataclass inside sandboxed
   code; Monty's parser doesn't support `class` definitions. Switched to
   module-scope dataclass + `dataclass_registry=[Box]` + Box() instance
   as input, matching the pattern in `test_dataclasses.py`.

5. **`ext_tests/test_metadata_dict_keys.py::test_value_metadata_independent_of_key_metadata`
   wrong assertion** (folded into `66e3033`). The test asserted that
   `d[k] = v` then `d[k]` returns v with v's metadata. It does not —
   same `Dict::set` vs `Dict::set_with_meta` gap as setattr (the
   compiler's `StoreSubscript` opcode routes through plain `Dict::set`).
   Renamed test to `test_dict_subscript_store_drops_value_metadata` and
   pinned the actual current behavior (empty metadata) so the test suite
   turns green and the gap is visible in the docstring.

## Two known extension gaps now explicitly documented

These are pre-existing extension issues that this sync surfaced via the
new ext_tests, not regressions introduced by the merge:

1. **`setattr(b, 'x', tainted)` drops value metadata** —
   `Dataclass::set_attr` calls `Dict::set` (no metadata thread). Pinned by
   `test_setattr_drops_value_metadata`.

2. **`d[k] = v` drops value metadata** — the compiler's `StoreSubscript`
   opcode routes through `Dict::set` (no metadata thread). Same root
   cause as #1; this is the broader gap. Pinned by
   `test_dict_subscript_store_drops_value_metadata`.

Both should be fixed in a follow-up extension that wires the compiler's
`StoreSubscript` opcode and `Dataclass::set_attr` through
`Dict::set_with_meta`. When that ships, both pinning tests fail and
force flipping to assert metadata propagation.

## Notable changes vs. plan

- **PR-D had +1 commit** (`4dc46f2` rename moved here from PR-B because
  it depends on `build.rs` from `e777ebf`).
- **PR-C and PR-D each gained an ext_tests commit** added during PR-C/PR-D
  authoring, which the original plan listed as a single "ext_tests/
  populated" deliverable — split per PR for clearer attribution and
  easier squash-merge into staging.
- **`extension/test_metadata_chain_assignment.py`** was added on PR-D
  while authoring (the merge plan called for `test_cases/assign__chain_metadata.py`
  but the metadata test harness lives in `ext_tests/`, not Monty
  test_cases — so it landed there).
- **The `ref-count-panic` → `memory-model-checks` feature flag rename**
  (upstream's `c026c34`) flowed into `CLAUDE.md` and the Makefile; any
  local notes referencing `--features ref-count-panic` should switch to
  `--features memory-model-checks` going forward.

## Setup for offline cargo builds (sandbox)

To build offline (e.g. in agent sandboxes without external network),
populate `vendor/` with `cargo vendor` (run on a machine with PyPI +
GitHub access), then add to `~/.cargo/config.toml` (NOT the project's
`.cargo/config.toml`, which stays clean for other developers):

```toml
[source.crates-io]
replace-with = "vendored-sources"

[source."git+https://github.com/astral-sh/ruff.git?rev=6ded4bed1651e30b34dd04cdaa50c763036abb0d"]
git = "https://github.com/astral-sh/ruff.git"
rev = "6ded4bed1651e30b34dd04cdaa50c763036abb0d"
replace-with = "vendored-sources"

[source."git+https://github.com/salsa-rs/salsa.git?rev=53421c2fff87426fa0bb51cab06632b87646de13"]
git = "https://github.com/salsa-rs/salsa.git"
rev = "53421c2fff87426fa0bb51cab06632b87646de13"
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "/absolute/path/to/monty/vendor"
```

`vendor/` is in `.gitignore` (well, it should be — add `/vendor/` if
not already there before committing the next merge that needs it).

## Deliverables

- 4 PR branches: `cz/sync-fork/2026-04-26-{A,B,C,D}` — each builds and
  tests cleanly in isolation.
- Staging branch: `cz/sync-fork/2026-04-26` — holds the plan + this
  notes doc only.
- 5 new `ext_tests/` files locking in design decisions and watch-points
  (38 tests total).
- 2 known gaps documented in test files for follow-up extension work.

## Squash-merge order

```bash
# In order: A, B, C, D into staging
gh pr merge --squash cz/sync-fork/2026-04-26-A
gh pr merge --squash cz/sync-fork/2026-04-26-B
gh pr merge --squash cz/sync-fork/2026-04-26-C
gh pr merge --squash cz/sync-fork/2026-04-26-D

# Then fast-forward tiny-beaver-ext to staging
git checkout tiny-beaver-ext
git merge --ff-only cz/sync-fork/2026-04-26
git push origin tiny-beaver-ext
```
