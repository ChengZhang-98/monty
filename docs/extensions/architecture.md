# Monty Architecture Reference

This document describes the Monty codebase architecture relevant to extension
developers. Read this before implementing any extension.

## How Tiny Beaver Uses Monty

Tiny Beaver's primary interface is `MontyRepl`, used in iterative mode:

```
BeaverAgent (orchestrator)
  → PLLM generates Python code
    → MontyRepl.feed_start(code)
      → code runs in sandbox
        → print() → captured via print_callback / structured_print_callback
        → external_fn() → FunctionSnapshot returned to BeaverAgent
          → BeaverAgent dispatches to real Python callable
          → snapshot.resume(return_value=result)
            → execution continues...
              → MontyComplete → result returned to BeaverAgent
```

Key paths to understand for extensions:
- **Output capture**: `print()` → `PrintWriter` → callback. This is where
  `structured_print_callback` hooks in for visibility-aware sanitization.
- **External function dispatch**: `feed_start` → `FunctionSnapshot` →
  `resume`. This is the pause/resume chain that threads `print_callback`
  through snapshots.
- **Session persistence**: `MontyRepl.dump()` / `MontyRepl.load()` for agent
  snapshot serialization. Note: callbacks are **not** serialized.
- **Metadata propagation**: Every value carries a `MetadataId` tracking
  provenance (producers/consumers/tags). Metadata flows through the parallel
  metadata stack in the VM and survives snapshot/resume via `MetadataStore`
  serialization. See [metadata-propagation](implemented/metadata-propagation.md).

## Layer Diagram

```
┌──────────────────────────────────────────┐
│  Python API         (monty-python/src/)  │  PyO3 classes exposed to Python
├──────────────────────────────────────────┤
│  Type Conversion    (convert.rs)         │  py_to_monty / monty_to_py
│                                          │  ObjectMetadata ↔ Python dict
├──────────────────────────────────────────┤
│  Dispatch Layer     (external.rs,        │  External functions, async,
│                      async_dispatch.rs)  │  snapshot resume
├──────────────────────────────────────────┤
│  Core VM            (monty/src/)         │  Bytecode execution, heap,
│                                          │  PrintWriter, builtins
│  Metadata           (metadata.rs)        │  MetadataStore, parallel stacks,
│                                          │  provenance propagation
└──────────────────────────────────────────┘
```

## Key Directories

| Path | Purpose |
|------|---------|
| `crates/monty/src/` | Core Rust VM (no Python dependency) |
| `crates/monty/src/builtins/` | Python builtin function implementations |
| `crates/monty/src/io.rs` | `PrintWriter` and `PrintWriterCallback` trait |
| `crates/monty/src/object.rs` | `MontyObject` enum (bridge between VM and outside) |
| `crates/monty/src/run.rs` | `MontyRun` executor |
| `crates/monty/src/run_progress.rs` | `RunProgress` snapshot types |
| `crates/monty-python/src/monty_cls.rs` | `Monty` class, snapshot classes, `EitherProgress` |
| `crates/monty-python/src/repl.rs` | `MontyRepl` class |
| `crates/monty-python/src/async_dispatch.rs` | Async execution loop, `with_print_writer` |
| `crates/monty-python/src/convert.rs` | `py_to_monty` / `monty_to_py` |
| `crates/monty-python/src/external.rs` | External function dispatch |
| `crates/monty-python/src/metadata.rs` | `PyObjectMetadata`, `PyAnnotatedValue`, metadata validation and conversion |
| `crates/monty/src/metadata.rs` | `MetadataStore`, `MetadataId`, `LabelSet`, provenance types |
| `crates/monty/src/bytecode/vm/mod.rs` | VM struct with parallel metadata stacks |
| `crates/monty-python/python/pydantic_monty/_monty.pyi` | Python type stubs |
| `crates/monty-python/tests/` | Python tests (pytest) |

## Data Flow

### Input to Output

```
Python inputs (dict) + optional ObjectMetadata
  → py_to_monty()        [convert.rs]
    → Vec<MontyObject> + Vec<MetadataId>
      → VM execution     [bytecode, heap, metadata_store]
        → MontyObject + MetadataId
          → monty_to_py() [convert.rs]
            → Python output + optional ObjectMetadata
```

### MontyObject

The intermediate representation between VM `Value` and Python objects. Every
value crossing the VM boundary goes through `MontyObject`:

```
MontyObject::None, Bool, Int, BigInt, Float, String, Bytes,
List, Tuple, Dict, Set, FrozenSet, Date, DateTime, TimeDelta,
TimeZone, Exception, Type, Path, Dataclass, Repr, ...
```

Key methods:
- `MontyObject::from_value(value, vm)` - VM Value → MontyObject
- `MontyObject::to_value(self, heap, interns)` - MontyObject → VM Value
- `py_to_monty(py_obj, dc_registry)` - Python → MontyObject
- `monty_to_py(py, monty_obj, dc_registry)` - MontyObject → Python

### Synchronous Execution (Monty.run)

```
PyMonty::run()
  → extract inputs, limits, print writer
  → run_impl()
    ├─ [no externals] runner.run(inputs, tracker, print_writer)
    │   → VM executes to completion
    │   → monty_to_py(result)
    │
    └─ [has externals] runner.start(inputs, tracker, print_writer)
        → loop over RunProgress snapshots
          → dispatch external calls
          → snapshot.resume(result, writer) → next RunProgress
          → until Complete
```

### Iterative Execution (Monty.start / snapshot.resume)

```
Monty.start()
  → runner.start() → RunProgress
    → EitherProgress::progress_or_complete()
      → run_progress_to_py()
        → PyFunctionSnapshot | PyNameLookupSnapshot | PyFutureSnapshot | PyMontyComplete

PyFunctionSnapshot.resume(return_value=...)
  → with_print_writer(callback, |writer| inner.resume(result, writer))
    → next RunProgress
      → run_progress_to_py() → next snapshot or MontyComplete
```

### Async Execution (Monty.run_async)

```
future_into_py(async {
  runner.start() → RunProgress
  dispatch_loop_run(progress):
    loop {
      FunctionCall → dispatch_function_call()
        → CallResult::Sync → spawn_blocking(resume)
        → CallResult::Coroutine → spawn_coroutine_task() + resume with Future(id)
      ResolveFutures → wait on JoinSet → resume with results
      Complete → monty_to_py(result)
    }
})
```

## The Snapshot Threading Chain

**This is the most important concept for extensions.**

When execution pauses (external function call, name lookup, etc.), the VM state
becomes a snapshot object. These snapshots carry metadata that must persist
across the entire chain:

```rust
// Every snapshot stores these fields:
pub struct PyFunctionSnapshot {
    snapshot: Mutex<EitherFunctionSnapshot>,  // VM state
    print_callback: Option<Py<PyAny>>,       // ← threaded through chain
    dc_registry: DcRegistry,                 // ← threaded through chain
    script_name: String,                     // ← threaded through chain
    // ... public fields (function_name, args, kwargs, etc.)
}
```

The `print_callback: Option<Py<PyAny>>` appears in **~20 locations** across
`monty_cls.rs`, `repl.rs`, and `async_dispatch.rs`. Any change to its type
causes merge conflicts with upstream in all 20 places.

The chain flows through:
```
Entry point (run/start/feed_start)
  → EitherProgress::progress_or_complete()
    → run_progress_to_py() / repl_progress_to_py()
      → PyFunctionSnapshot { print_callback, dc_registry, ... }
        → snapshot.resume()
          → with_print_writer(print_callback, |writer| ...)
            → next RunProgress
              → next snapshot (carries same print_callback)
```

See [patterns.md](patterns.md) for how to thread new data through this chain
without changing the types.

## PrintWriter (monty/src/io.rs)

How `print()` output reaches the host:

```rust
pub enum PrintWriter<'a> {
    Disabled,                                       // discard
    Stdout,                                         // terminal
    Collect(&'a mut String),                        // buffer
    Callback(&'a mut dyn PrintWriterCallback),      // custom handler
}

pub trait PrintWriterCallback {
    fn stdout_write(&mut self, output: Cow<str>) -> Result<(), MontyException>;
    fn stdout_push(&mut self, end: char) -> Result<(), MontyException>;
    fn wants_structured(&self) -> bool { false }
    fn stdout_write_structured(&mut self, objects: Vec<MontyObject>, sep: &str, end: &str) -> ...;
}
```

`with_print_writer` in `async_dispatch.rs` creates the right callback from an
`Option<Py<PyAny>>` and invokes a closure with the `PrintWriter`.

## The EitherX Pattern

PyO3 classes can't be generic. The codebase uses enum wrappers:

```rust
// Core: generic over resource tracker
pub struct FunctionCall<T: ResourceTracker> { ... }

// PyO3-facing: enum over both tracker types
enum EitherFunctionSnapshot {
    NoLimit(FunctionCall<PySignalTracker<NoLimitTracker>>),
    Limited(FunctionCall<PySignalTracker<LimitedTracker>>),
    ReplNoLimit(ReplFunctionCall<...>, Py<PyMontyRepl>),
    ReplLimited(ReplFunctionCall<...>, Py<PyMontyRepl>),
}
```

Conversion uses traits like `FromFunctionCall<T>` to avoid boilerplate.

## GIL Patterns

```rust
// Release GIL for blocking Rust work (from Python context)
py.detach(|| { /* no GIL, pure Rust */ })

// Acquire GIL from Rust context (in callbacks, worker threads)
Python::attach(|py| { /* GIL held */ })

// Py<PyAny> is GIL-independent (Send + Sync), stores across threads
// Bound<'py, PyAny> is GIL-bound, cannot leave the closure
let stored = callback.clone().unbind();  // → Py<PyAny>
Python::attach(|py| stored.bind(py).call1(args));  // re-acquire later
```

## Python API Entry Points

These are the methods that accept user-facing parameters:

| Class | Method | File |
|-------|--------|------|
| `Monty` | `run` | `monty_cls.rs` |
| `Monty` | `start` | `monty_cls.rs` |
| `Monty` | `run_async` | `monty_cls.rs` |
| `MontyRepl` | `feed_run` | `repl.rs` |
| `MontyRepl` | `feed_start` | `repl.rs` |
| `MontyRepl` | `feed_run_async` | `repl.rs` |

When adding a new parameter, it must be added to **all six** methods (plus
their `#[pyo3(signature)]` attributes and the `.pyi` type stubs).
