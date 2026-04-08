# Tiny Beaver Extensions

This directory documents how to extend the Monty sandboxed Python interpreter
for the tiny-beaver project.

## For AI Agents

If you are an AI agent tasked with implementing a new extension, read these
docs **in order**:

1. **[architecture.md](architecture.md)** - Understand how Monty is structured
2. **[how-to-extend.md](how-to-extend.md)** - Step-by-step walkthrough
3. **[patterns.md](patterns.md)** - Reusable patterns with code examples
4. **[Existing extensions](implemented/)** - Study prior art for similar work

Then create a doc for your extension using the **[template](_template.md)**.

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
