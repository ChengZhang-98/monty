# Fix: `MemoryError` Propagation in Stdlib Module Init

## Summary

When a sandboxed program imports a stdlib module (`sys`, `os`, `pathlib`,
`typing`, `asyncio`, `math`, `json`, `re`, `datetime`) under a tight
`max_memory` limit, allocation failures during module attribute setup are now
propagated as `MemoryError` instead of panicking the host.

## Motivation

Stdlib modules are created lazily when first imported inside the sandbox
(`VM::load_module` calls `StandardLib::create`, which populates the module's
attribute `Dict`). After the 2026-04-17 upstream merge, `Dict::set` can return
`Err(UncatchableExc(MemoryError))` when growing the dict exceeds
`max_memory` — but `Module::set_attr` was a `fn(..) -> ()` that unwrapped,
causing a Rust panic rather than a clean resource-limit error.

This tripped four tests that exercise the mount-release path under a low
memory limit:

- `test_start::test_run_multiple_threads_with_shared_mount_under_memory_limit`
- `test_start::test_start_mount_released_after_resource_error`
- `test_mount_table::test_repl_feed_run_mount_released_after_resource_error`
- `test_mount_table::test_feed_start_mount_released_after_resource_error`

Each test mounts a directory, starts code that does
`Path('/mnt/...').read_text()`, and sets `max_memory` low enough that the
subsequent module-init allocation trips. The expected behavior is a
`MontyRuntimeError` wrapping `MemoryError`; the actual behavior was a host
panic.

## API

No public API changes. A program under a tight `max_memory` limit now sees
`MemoryError: memory limit exceeded ...` instead of the host panicking.

## Implementation

### Files Changed

| File | Change |
|------|--------|
| `crates/monty/src/types/module.rs` | `Module::set_attr` now returns `RunResult<()>`; drops any previous value defensively |
| `crates/monty/src/modules/mod.rs` | `StandardLib::create` returns `RunResult<HeapId>` (was `Result<_, ResourceError>`) |
| `crates/monty/src/modules/{sys,typing,asyncio,pathlib,os,math,json/mod,re,datetime}.rs` | Each `create_module` returns `RunResult<HeapId>`; every `set_attr` call `?`-propagates |

### Key Design Decisions

- **`RunResult` instead of `Result<_, ResourceError>`**: `Dict::set` returns
  `RunResult`, and `ResourceError: Into<RunError>` already exists — so
  `vm.heap.allocate(...)?` at the tail of each `create_module` still works
  without conversion. `VM::load_module` already returned `RunResult<()>`, so
  its `module.create(self)?` call needed no change.

- **Defensive drop of previous value**: `Module::set_attr` now drops any
  `Some` returned from `Dict::set`. In practice module attributes are only
  assigned once during init, so `previous` is always `None` — but the drop
  prevents refcount leaks if a caller ever re-assigns an attribute.

- **`InternString` keys are always hashable**: the docstring now states this
  explicitly so readers know that the only error path is allocation failure
  inside `Dict::set`, never a hash-time error.

## Testing

```bash
uv run pytest crates/monty-python/tests/test_start.py \
              crates/monty-python/tests/test_mount_table.py
```

All four previously-panicking tests now pass.

## Future Considerations

Any future stdlib module (a new `StandardLib::<Name>` variant) must likewise
`?`-propagate `set_attr` results, and its `create_module` must return
`RunResult<HeapId>`. Returning `Result<_, ResourceError>` would still compile
(via `?` at `StandardLib::create`), but the panic surface of a bare
`.unwrap()` on `set_attr` would reappear — hence the signature change is
enforced at the module boundary.
