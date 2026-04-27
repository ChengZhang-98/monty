"""Tests for tuple slicing metadata propagation.

Tuple slicing (`crates/monty/src/types/tuple.rs::py_getitem` slice path) was
previously losing per-element metadata: it called the overflow-safe
`slice_collect_iterator` for `items` but allocated the result via plain
`allocate_tuple`, so every element of the sliced tuple landed with default
(empty) metadata. The fix mirrors `List::getitem_slice` — slice items and
`item_metadata` in parallel against the same slice spec, then construct via
`allocate_tuple_with_metadata`.

These tests pin the fix the same way `test_metadata_slicing.py` pins list
slicing: forward, reverse, and stepped slices each must carry forward the
metadata of the elements they select.
"""

from typing import Any

from inline_snapshot import snapshot

import pydantic_monty


def _meta(producers: frozenset[str] = frozenset()) -> pydantic_monty.ObjectMetadata:
    return pydantic_monty.ObjectMetadata(producers=producers)


def _capture_print_metadata(
    code: str, inputs: dict[str, pydantic_monty.AnnotatedValue]
) -> list[tuple[Any, pydantic_monty.ObjectMetadata]]:
    """Run code and capture (value, metadata) for each printed object."""
    m = pydantic_monty.Monty(code, inputs=list(inputs.keys()))
    calls: list[tuple[Any, pydantic_monty.ObjectMetadata]] = []

    def cb(stream: str, objects: list[pydantic_monty.AnnotatedValue], sep: str, end: str) -> None:
        for obj in objects:
            calls.append((obj.value, obj.metadata))

    m.run(inputs=inputs, structured_print_callback=cb)
    return calls


# === Forward slice preserves metadata ===


def test_tuple_slice_preserves_element_metadata() -> None:
    """`tup[1:3]` carries forward the metadata of elements at indices 1 and 2."""
    code = """
tup = (a, b, c, d)
sl = tup[1:3]
for x in sl:
    print(x)
"""
    calls = _capture_print_metadata(
        code,
        {
            'a': pydantic_monty.AnnotatedValue('A', _meta(producers=frozenset({'src_a'}))),
            'b': pydantic_monty.AnnotatedValue('B', _meta(producers=frozenset({'src_b'}))),
            'c': pydantic_monty.AnnotatedValue('C', _meta(producers=frozenset({'src_c'}))),
            'd': pydantic_monty.AnnotatedValue('D', _meta(producers=frozenset({'src_d'}))),
        },
    )
    assert [(v, m.producers) for v, m in calls] == snapshot(
        [
            ('B', frozenset({'src_b'})),
            ('C', frozenset({'src_c'})),
        ]
    )


# === Reverse slice preserves metadata in correct order ===


def test_tuple_reverse_slice_preserves_element_metadata() -> None:
    """`tup[::-1]` reverses items AND their parallel metadata."""
    code = """
tup = (a, b, c)
for x in tup[::-1]:
    print(x)
"""
    calls = _capture_print_metadata(
        code,
        {
            'a': pydantic_monty.AnnotatedValue('A', _meta(producers=frozenset({'src_a'}))),
            'b': pydantic_monty.AnnotatedValue('B', _meta(producers=frozenset({'src_b'}))),
            'c': pydantic_monty.AnnotatedValue('C', _meta(producers=frozenset({'src_c'}))),
        },
    )
    assert [(v, m.producers) for v, m in calls] == snapshot(
        [
            ('C', frozenset({'src_c'})),
            ('B', frozenset({'src_b'})),
            ('A', frozenset({'src_a'})),
        ]
    )


# === Stepped slice preserves metadata at correct indices ===


def test_tuple_stepped_slice_preserves_element_metadata() -> None:
    """`tup[::2]` picks elements 0, 2, 4 and the metadata at those exact indices."""
    code = """
tup = (a, b, c, d, e)
for x in tup[::2]:
    print(x)
"""
    calls = _capture_print_metadata(
        code,
        {
            'a': pydantic_monty.AnnotatedValue('A', _meta(producers=frozenset({'src_a'}))),
            'b': pydantic_monty.AnnotatedValue('B', _meta(producers=frozenset({'src_b'}))),
            'c': pydantic_monty.AnnotatedValue('C', _meta(producers=frozenset({'src_c'}))),
            'd': pydantic_monty.AnnotatedValue('D', _meta(producers=frozenset({'src_d'}))),
            'e': pydantic_monty.AnnotatedValue('E', _meta(producers=frozenset({'src_e'}))),
        },
    )
    assert [(v, m.producers) for v, m in calls] == snapshot(
        [
            ('A', frozenset({'src_a'})),
            ('C', frozenset({'src_c'})),
            ('E', frozenset({'src_e'})),
        ]
    )


# === Empty slice yields empty (singleton) tuple — no panic, no orphan metadata ===


def test_tuple_empty_slice_metadata() -> None:
    """`tup[10:20]` on a 3-tuple is empty; allocate_tuple_with_metadata routes
    through the empty-tuple singleton and contributes no metadata."""
    code = """
tup = (a, b, c)
print(len(tup[10:20]))
"""
    calls = _capture_print_metadata(
        code,
        {
            'a': pydantic_monty.AnnotatedValue('A', _meta(producers=frozenset({'src_a'}))),
            'b': pydantic_monty.AnnotatedValue('B', _meta(producers=frozenset({'src_b'}))),
            'c': pydantic_monty.AnnotatedValue('C', _meta(producers=frozenset({'src_c'}))),
        },
    )
    assert [v for v, _ in calls] == snapshot([0])
