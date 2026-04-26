"""Tests for the `setattr` builtin (`d5444c6`, PR #67) interacting with
extension metadata.

**Status: pins down current behavior — `setattr` does NOT propagate value metadata.**

The new `builtin_setattr` calls `Value::py_set_attr` → `Dataclass::set_attr` →
`Dict::set` (NOT `Dict::set_with_meta`). So `setattr(obj, "x", tainted)`
currently produces an attribute with default (empty) metadata, dropping the
value's metadata on the floor.

This is consistent with extension's existing `obj.attr = tainted` codepath
(also calls plain `Dict::set`) — so it's not a setattr-specific regression,
but a pre-existing extension gap that the new builtin inherits.

When the gap is closed in a follow-up extension (wiring metadata through
`Dict::set` for dataclass attrs), the `_drops_metadata` test below will
fail. That failure is a feature: it forces flipping the assertion to
"preserves" and confirming the new behavior intentionally.

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


# === Current behavior: setattr drops the value's metadata ===


def test_setattr_drops_value_metadata() -> None:
    """Documents current behavior: `setattr(b, 'x', tainted)` then `print(b.x)`
    shows the value but with default (empty) metadata, not the tainted's metadata.

    This is the gap. When closed, this test will fail and should be flipped to
    `frozenset({'src_a'})`.
    """
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
    # Current behavior: metadata is lost in Dict::set (not Dict::set_with_meta).
    # When the gap is closed, this should become `frozenset({'src_a'})`.
    assert meta.producers == snapshot(frozenset())
