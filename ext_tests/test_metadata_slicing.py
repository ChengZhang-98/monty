"""Tests for list slicing metadata propagation after the `9e36ce3`
(`i64::MIN` overflow fix) merge.

`9e36ce3` rewrote sequence slicing across `bytes`/`list`/`range`/`str`/
`tuple` to use the shared overflow-safe `slice_collect_iterator` helper.
The extension's previous `get_slice_items` + `get_slice_metadata` helpers
(which had the same `i64::MIN.unsigned_abs()` panic upstream just fixed)
were deleted; list slicing now calls `slice_collect_iterator` twice in
parallel — once for `items`, once for `item_metadata` — to preserve
per-element metadata propagation.

These tests pin down that the parallel slice produces the right metadata
indices for forward, negative-index, and stepped slices. If the alignment
between `items` and `item_metadata` ever drifts, these tests will catch
the wrong metadata coming out of `lst[i:j:k]`.
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


def test_list_slice_preserves_element_metadata() -> None:
    """`lst[1:3]` carries forward the metadata of elements at indices 1 and 2."""
    code = """
lst = [a, b, c, d]
sl = lst[1:3]
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
    # Slice [1:3] should yield B and C, with their respective metadata.
    assert [(v, m.producers) for v, m in calls] == snapshot(
        [
            ('B', frozenset({'src_b'})),
            ('C', frozenset({'src_c'})),
        ]
    )


# === Reverse slice preserves metadata in correct order ===


def test_list_reverse_slice_preserves_element_metadata() -> None:
    """`lst[::-1]` reverses items AND their parallel metadata."""
    code = """
lst = [a, b, c]
for x in lst[::-1]:
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


def test_list_stepped_slice_preserves_element_metadata() -> None:
    """`lst[::2]` picks elements 0, 2, 4 and the metadata at those exact indices."""
    code = """
lst = [a, b, c, d, e]
for x in lst[::2]:
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
    # Step 2: indices 0, 2, 4 → A, C, E
    assert [(v, m.producers) for v, m in calls] == snapshot(
        [
            ('A', frozenset({'src_a'})),
            ('C', frozenset({'src_c'})),
            ('E', frozenset({'src_e'})),
        ]
    )


# === i64::MIN regression — slice doesn't panic on extreme step ===


def test_list_slice_extreme_negative_step_does_not_panic() -> None:
    """`9e36ce3` fix: slicing with step near `i64::MIN` must not panic.

    Before the fix, `get_slice_items` did `i64::try_from(start).expect(...)`
    and `usize::try_from(-step).expect(...)`, which would panic on
    `i64::MIN`. The new `slice_collect_iterator` saturates safely.
    Empty result is fine — the test asserts no panic.
    """
    code = """
lst = [1, 2, 3]
sl = lst[::-9223372036854775808]
print(len(sl))
"""
    calls = _capture_print_metadata(code, {})
    # Whatever the resulting length is, the test passes if no panic occurred.
    assert len(calls) == snapshot(1)
