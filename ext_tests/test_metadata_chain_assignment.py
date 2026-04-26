"""Tests for chain assignment (`a = b = c = expr`) preserving metadata.

`e50f8eb` (PR #357) added chain assignment support. The compiler emits
`Opcode::Dup` for each target except the last, then assigns each target
in left-to-right source order. Since `Opcode::Dup` already duplicates the
metadata via `peek_meta()` + `push_with_meta()` (in
`crates/monty/src/bytecode/vm/mod.rs`), every target in the chain receives
the same metadata as the source expression.

These tests pin that invariant down so a future compiler refactor can't
silently start using a different opcode (or a `Dup` variant that doesn't
also dup the meta_stack) without surfacing as a test failure.
"""

from typing import Any

from inline_snapshot import snapshot

import pydantic_monty


def _meta(producers: frozenset[str] = frozenset()) -> pydantic_monty.ObjectMetadata:
    return pydantic_monty.ObjectMetadata(producers=producers)


def _capture_print_metadata(
    code: str, inputs: dict[str, pydantic_monty.AnnotatedValue]
) -> list[tuple[Any, pydantic_monty.ObjectMetadata]]:
    m = pydantic_monty.Monty(code, inputs=list(inputs.keys()))
    calls: list[tuple[Any, pydantic_monty.ObjectMetadata]] = []

    def cb(stream: str, objects: list[pydantic_monty.AnnotatedValue], sep: str, end: str) -> None:
        for obj in objects:
            calls.append((obj.value, obj.metadata))

    m.run(inputs=inputs, structured_print_callback=cb)
    return calls


# === All targets get the source's metadata ===


def test_chain_assignment_propagates_metadata_to_all_targets() -> None:
    """`a = b = c = tainted` makes a, b, and c all carry tainted's metadata."""
    code = """
a = b = c = src
print(a)
print(b)
print(c)
"""
    calls = _capture_print_metadata(
        code,
        {'src': pydantic_monty.AnnotatedValue(42, _meta(producers=frozenset({'src_a'})))},
    )
    assert [(v, m.producers) for v, m in calls] == snapshot(
        [
            (42, frozenset({'src_a'})),
            (42, frozenset({'src_a'})),
            (42, frozenset({'src_a'})),
        ]
    )


# === Two-target chain assignment ===


def test_chain_assignment_two_targets() -> None:
    """`a = b = src` — the most common chain-assignment shape."""
    code = """
a = b = src
print(a)
print(b)
"""
    calls = _capture_print_metadata(
        code,
        {'src': pydantic_monty.AnnotatedValue('hello', _meta(producers=frozenset({'env'})))},
    )
    assert [(v, m.producers) for v, m in calls] == snapshot(
        [
            ('hello', frozenset({'env'})),
            ('hello', frozenset({'env'})),
        ]
    )


# === Chain-assigning a computed value with merged metadata ===


def test_chain_assignment_with_computed_value() -> None:
    """`a = b = x + y` propagates the merged metadata of x and y to both targets."""
    code = """
a = b = x + y
print(a)
print(b)
"""
    calls = _capture_print_metadata(
        code,
        {
            'x': pydantic_monty.AnnotatedValue(10, _meta(producers=frozenset({'src_x'}))),
            'y': pydantic_monty.AnnotatedValue(20, _meta(producers=frozenset({'src_y'}))),
        },
    )
    # Both a and b carry the merged metadata (union of x and y's producers).
    assert [(v, m.producers) for v, m in calls] == snapshot(
        [
            (30, frozenset({'src_x', 'src_y'})),
            (30, frozenset({'src_x', 'src_y'})),
        ]
    )
