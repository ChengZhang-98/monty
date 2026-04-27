"""Tests for the `setattr` builtin (`d5444c6`, PR #67) interacting with
extension metadata.

`builtin_setattr` and the `StoreAttr` opcode now thread `value_meta` through
`Value::py_set_attr` → `Dataclass::set_attr` → `Dict::set_with_meta`, so
`setattr(obj, "x", tainted)` and `obj.x = tainted` both preserve the value's
metadata (producers / consumers / tags) on the resulting attribute.

These tests cover both entry points — the `setattr` builtin and the
`StoreAttr` opcode (`obj.x = v` syntax) — because they share the same fix
site (`Dataclass::set_attr` calling `Dict::set_with_meta`).

Note: Monty's parser does not support inline `class` definitions, so the
test dataclass is defined at module scope on the host side and registered
via `dataclass_registry`, then passed as an input.

See `docs/extensions/sync-fork/2026-04-26-merge-notes.md` for context.
"""

from dataclasses import dataclass
from typing import Any

from inline_snapshot import snapshot

import pydantic_monty


@dataclass
class Box:
    x: int


def _meta(producers: frozenset[str] = frozenset()) -> pydantic_monty.ObjectMetadata:
    return pydantic_monty.ObjectMetadata(producers=producers)


# === setattr basic round-trip (no metadata involvement) ===


def test_setattr_value_roundtrips() -> None:
    """`setattr(obj, "x", v)` then `obj.x` returns v unchanged.

    Verifies the new `setattr` builtin works at all — orthogonal to metadata.
    """
    code = "setattr(b, 'x', 99); b.x"
    m = pydantic_monty.Monty(code, inputs=['b'], dataclass_registry=[Box])
    result = m.run(inputs={'b': Box(x=0)})
    assert result == snapshot(99)


# === setattr preserves the value's metadata ===


def test_setattr_preserves_value_metadata() -> None:
    """`setattr(b, 'x', tainted)` followed by `print(b.x)` propagates `tainted`'s
    metadata onto the read-back value, end-to-end through `Dict::set_with_meta`."""
    code = "setattr(b, 'x', tainted); print(b.x)"
    m = pydantic_monty.Monty(code, inputs=['b', 'tainted'], dataclass_registry=[Box])
    calls: list[tuple[Any, pydantic_monty.ObjectMetadata]] = []

    def cb(stream: str, objects: list[pydantic_monty.AnnotatedValue], sep: str, end: str) -> None:
        for obj in objects:
            calls.append((obj.value, obj.metadata))

    m.run(
        inputs={
            'b': Box(x=0),
            'tainted': pydantic_monty.AnnotatedValue(99, _meta(producers=frozenset({'src_a'}))),
        },
        structured_print_callback=cb,
    )
    assert len(calls) == snapshot(1)
    _, meta = calls[0]
    assert meta.producers == snapshot(frozenset({'src_a'}))


# === StoreAttr opcode (`obj.x = tainted`) preserves the value's metadata ===


def test_store_attr_preserves_value_metadata() -> None:
    """The `obj.x = tainted` syntax (compiled to `StoreAttr`) shares the
    `Dataclass::set_attr` fix site with `setattr`, so it must propagate metadata
    the same way."""
    code = "b.x = tainted; print(b.x)"
    m = pydantic_monty.Monty(code, inputs=['b', 'tainted'], dataclass_registry=[Box])
    calls: list[tuple[Any, pydantic_monty.ObjectMetadata]] = []

    def cb(stream: str, objects: list[pydantic_monty.AnnotatedValue], sep: str, end: str) -> None:
        for obj in objects:
            calls.append((obj.value, obj.metadata))

    m.run(
        inputs={
            'b': Box(x=0),
            'tainted': pydantic_monty.AnnotatedValue(99, _meta(producers=frozenset({'src_b'}))),
        },
        structured_print_callback=cb,
    )
    assert len(calls) == snapshot(1)
    _, meta = calls[0]
    assert meta.producers == snapshot(frozenset({'src_b'}))
