"""Tests for design decision §1 from 2026-04-26 sync: metadata is orthogonal
to hash and equality.

Two values that differ only in their `producers`/`consumers`/`tags` metadata
must hash equal and compare equal under Python semantics. This is required for
the `a == b => hash(a) == hash(b)` invariant — without it, dict/set lookups
become non-deterministic in the presence of metadata.

Provenance is observability metadata, not identity. The visibility/sanitization
layer (see TinyBeaver `docs/visibility.md`) operates on metadata downstream of
equality, not as part of it.

Tests run code inside the sandbox so they exercise the same `py_hash` / `py_eq`
paths the bytecode VM uses.
"""

from inline_snapshot import snapshot

import pydantic_monty


def _meta(producers: frozenset[str] = frozenset(), tags: frozenset[str] = frozenset()) -> pydantic_monty.ObjectMetadata:
    return pydantic_monty.ObjectMetadata(producers=producers, tags=tags)


# === String identity under differing metadata ===


def test_string_eq_ignores_metadata() -> None:
    """`a == b` for two equal strings must be True regardless of metadata."""
    code = 'a == b'
    m = pydantic_monty.Monty(code, inputs=['a', 'b'])
    result = m.run(
        inputs={
            'a': pydantic_monty.AnnotatedValue('hello', _meta(producers=frozenset({'src_a'}))),
            'b': pydantic_monty.AnnotatedValue('hello', _meta(producers=frozenset({'src_b'}))),
        },
    )
    assert result == snapshot(True)


def test_string_hash_ignores_metadata() -> None:
    """`hash(a) == hash(b)` for two equal strings, regardless of metadata.

    Verified via set deduplication — if hashes differ, the set keeps both.
    """
    code = 's = {a, b}; len(s)'
    m = pydantic_monty.Monty(code, inputs=['a', 'b'])
    result = m.run(
        inputs={
            'a': pydantic_monty.AnnotatedValue('hello', _meta(producers=frozenset({'src_a'}))),
            'b': pydantic_monty.AnnotatedValue('hello', _meta(producers=frozenset({'src_b'}))),
        },
    )
    assert result == snapshot(1)


# === Integer identity under differing metadata ===


def test_int_set_dedupe_across_metadata() -> None:
    """Two equal ints with different metadata dedupe in a set."""
    code = 'len({a, b})'
    m = pydantic_monty.Monty(code, inputs=['a', 'b'])
    result = m.run(
        inputs={
            'a': pydantic_monty.AnnotatedValue(42, _meta(tags=frozenset({'untrusted'}))),
            'b': pydantic_monty.AnnotatedValue(42, _meta(producers=frozenset({'cache'}))),
        },
    )
    assert result == snapshot(1)


# === Dict lookup ignores key metadata ===


def test_dict_lookup_ignores_key_metadata() -> None:
    """`d[k_a] = v` then `d[k_b]` must succeed when `k_a == k_b` but metadata differs.

    This is the load-bearing case: a dict storing values keyed by user-supplied
    strings must not become unreachable after a metadata change on the lookup key.
    """
    code = "d = {}; d[k_store] = 'value'; d[k_lookup]"
    m = pydantic_monty.Monty(code, inputs=['k_store', 'k_lookup'])
    result = m.run(
        inputs={
            'k_store': pydantic_monty.AnnotatedValue('key', _meta(producers=frozenset({'env'}))),
            'k_lookup': pydantic_monty.AnnotatedValue('key', _meta(producers=frozenset({'user_input'}))),
        },
    )
    assert result == snapshot('value')


# === Tuple identity under differing element metadata ===


def test_tuple_eq_ignores_element_metadata() -> None:
    """Tuples of equal elements compare equal regardless of per-element metadata."""
    code = "(a, 1) == (b, 1)"
    m = pydantic_monty.Monty(code, inputs=['a', 'b'])
    result = m.run(
        inputs={
            'a': pydantic_monty.AnnotatedValue('x', _meta(tags=frozenset({'pii'}))),
            'b': pydantic_monty.AnnotatedValue('x', _meta(tags=frozenset({'public'}))),
        },
    )
    assert result == snapshot(True)
