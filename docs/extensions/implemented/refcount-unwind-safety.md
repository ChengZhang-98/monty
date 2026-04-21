# Fix: Refcount Ops Tolerate Panic Unwinding

## Summary

`Heap::inc_ref` and `Heap::dec_ref` no longer re-panic when called during an active
unwind with a stale or out-of-bounds `HeapId`. A second panic inside a `Drop`
while the thread is unwinding escalates to `abort()`, so the short-circuit
preserves the original panic (and lets pyo3 convert it into `PanicException`).

## Motivation

Heap reference counting is driven from `DropWithHeap` implementations. If the
primary panic leaves a `HeapId` dangling (the original bug corrupted the heap,
or a mid-operation failure left a guard holding a stale id), the destructor
chain runs during unwind and calls `inc_ref` / `dec_ref` with that bad id. The
old behaviour:

1. `inc_ref(bad_id)` → `HeapEntries::get` panics with *"data already freed"*.
2. Rust sees a panic-inside-drop during unwind → `abort()`.
3. The process dies immediately — pyo3 never gets to wrap the original panic as
   a Python `PanicException`, and the user sees a hard crash instead of a
   catchable exception.

The branch `cz/fix/spillway` was prompted by exactly this pattern in the
spillway integration: tight resource limits triggered a primary panic, and the
follow-up refcount cleanup during unwind turned a recoverable failure into a
process abort.

## API

No public API change. The functions still panic on bad ids during normal
operation; the only difference is when `std::thread::panicking()` is already
`true`, in which case they return silently (accepting a refcount leak on the
subtree) rather than re-panicking.

## Implementation

### Files Changed

| File | Change |
|------|--------|
| `crates/monty/src/heap.rs` | `inc_ref` / `dec_ref` gate bad-id / freed-slot / reader-held paths behind `thread::panicking()` |
| `crates/monty/src/heap/heap_entries.rs` | New `HeapEntries::try_get` — non-panicking variant of `get` for freed slots |

### Key Design Decisions

- **Only soft-fail during unwind**: `thread::panicking()` is checked on every
  short-circuit. Outside of an unwind, the usual panic still fires so genuine
  refcount bugs stay loud in tests (including `ref-count-panic`).
- **`try_get` still panics on OOB**: out-of-bounds index indicates a corrupted
  `HeapId` rather than a freed slot. `inc_ref` handles OOB separately by
  comparing the index against `entries.len()` before calling `try_get`.
- **Leak over abort**: when the refcount would drop to zero but readers are
  active during unwind, the entry is leaked rather than freed. The heap is
  already inconsistent from the original panic — the process was going down
  regardless, and a leak is strictly preferable to `abort()` for observability.
- **Work-stack semantics preserved**: `dec_ref`'s iterative traversal over
  nested containers still pops the next id from the work stack when it skips a
  corrupt subtree, so the surrounding cleanup continues where possible.

## Testing

Unit tests live alongside the code they exercise (the heap internals are not
exposed for integration-test consumption):

- `crates/monty/src/heap.rs` → `mod panic_during_unwind_tests`:
  - `inc_ref_panics_on_bad_id_outside_unwind` — baseline: normal panic path is intact
  - `inc_ref_silent_on_oob_id_during_unwind`
  - `inc_ref_silent_on_freed_slot_during_unwind`
  - `dec_ref_silent_on_oob_id_during_unwind`
  - `dec_ref_silent_on_freed_slot_during_unwind`

- `crates/monty/src/heap/heap_entries.rs` → `mod tests`:
  - `try_get_returns_some_for_live_slot`
  - `try_get_returns_none_for_freed_slot`

Each unwind test wraps a `catch_unwind` around a drop guard that fires the
hardened op with a deliberately bad id — without the fix, the secondary panic
inside `Drop` would abort the test process rather than allow `catch_unwind` to
return `Err`.

```bash
cargo test -p monty --lib --features ref-count-panic heap::
```

## Commits

| Hash | Description |
|------|-------------|
| _pending_ | Harden `Heap::{inc_ref,dec_ref}` against panic unwinding + tests |

## Future Considerations

- **Observability of leaked subtrees**: right now a leak during unwind is
  silent. If we ever want to surface these (e.g. in telemetry), a counter on
  the tracker would be a lightweight hook.
- **Any new `DropWithHeap` types** should still use `defer_drop!` / `HeapGuard`
  rather than manual `drop_with_heap` — this fix is a safety net, not a license
  to skip the existing ownership discipline described in `CLAUDE.md`.
