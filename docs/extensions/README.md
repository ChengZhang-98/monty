# Tiny Beaver Extensions

This directory documents how to extend the Monty sandboxed Python interpreter
for the [tiny-beaver](https://github.com/ChengZhang-98/tiny-beaver) project.

## What is Tiny Beaver?

Tiny Beaver is a REPL-based LLM agent framework built on a dual-LLM
architecture. Its core execution engine is **MontyRepl** (from this repo):

- A **Planning LLM (PLLM)** generates Python code
- That code runs inside Monty's sandboxed REPL with memory/time limits
- **External functions** (bash, grep, web_search, etc.) are called from within
  the REPL via Monty's pause/resume mechanism (`feed_start` / `snapshot.resume`)
- **Visibility modes** (SAFE/TAINTED) control how external data is sanitized
  before being shown to the PLLM — this is why extensions like
  `structured_print_callback` exist (sanitization needs typed objects, not strings)

Key Monty features used by Tiny Beaver:
- `MontyRepl.feed_start()` / `snapshot.resume()` — iterative execution with
  external function dispatch
- `MontyRepl.dump()` / `MontyRepl.load()` — session persistence for agent
  snapshots
- `print_callback` / `structured_print_callback` — output capture and
  sanitization
- Resource limits — timeout, memory, recursion depth

See the [Tiny Beaver docs](https://github.com/ChengZhang-98/tiny-beaver/tree/main/docs)
for full architecture details, especially `architecture.md` and `visibility.md`.

## For AI Agents

If you are an AI agent tasked with implementing a new extension, read these
docs **in order**:

1. **[architecture.md](architecture.md)** - Understand how Monty is structured
2. **[how-to-extend.md](how-to-extend.md)** - Step-by-step walkthrough
3. **[patterns.md](patterns.md)** - Reusable patterns with code examples
4. **[Existing extensions](implemented/)** - Study prior art for similar work

Then create a doc for your extension using the **[template](_template.md)**.

**Context to keep in mind**: Tiny Beaver's primary interface is `MontyRepl`
(not `Monty` directly). The iterative `feed_start` / `snapshot.resume` path
is the hot path — external function calls happen there. Extensions that modify
callback behavior must work correctly across the snapshot/resume chain.

## Branching Strategy

```
main                      <-- tracks upstream Monty
  └── tiny-beaver-ext     <-- all extensions accumulated here
        ├── feature/foo   <-- branch per extension, merge back
        └── feature/bar
```

- **New extension**: `git checkout tiny-beaver-ext && git checkout -b feature/my-thing`
- **Merge upstream**: `git checkout main && git pull`, then
  `git checkout tiny-beaver-ext && git merge main`
- **Ship extension**: merge feature branch into `tiny-beaver-ext`

## Build & Test Commands

```bash
make format-rs          # format Rust
make lint-rs            # clippy + import checks
make dev-py             # build Python package (required before pytest)
make pytest             # run Python tests
make lint-py            # ruff format + pyright + stubtest
```

## Implemented Extensions

| Extension | Branch | Description |
|-----------|--------|-------------|
| [structured-print-callback](implemented/structured-print-callback.md) | `feature/structured-print-callback` | Receive `print()` args as typed Python objects |
| [metadata-propagation](implemented/metadata-propagation.md) | `tiny-beaver-ext` | Track data provenance (producers/consumers/tags) through every value |
| [exception-type-conversion](implemented/exception-type-conversion.md) | `cz/fix/failure` | Fix `type()` on exceptions returning a string instead of the Python class |
| [module-init-memory-error](implemented/module-init-memory-error.md) | `cz/merge-main/2026-04-17` | Propagate `MemoryError` from stdlib module init instead of panicking |
