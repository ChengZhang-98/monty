"""Tests for design decision §2 from 2026-04-26 sync: dict key metadata
is preserved (separately from value metadata).

The extension stores `key_meta` alongside `value_meta` in `DictEntry`
(see `crates/monty/src/types/dict.rs`). Keys carry their own metadata that
survives storage and is independent of value metadata; lookup itself
ignores key metadata (see `test_metadata_hash_invariant.py` for the lookup
invariant).

These tests pin down the storage semantics so future merges can't quietly
regress them. They focus on the round-trip: write a key with metadata, then
extract it via iteration / `.keys()` and confirm metadata survived.
"""

from inline_snapshot import snapshot

import pydantic_monty


def _meta(producers: frozenset[str] = frozenset(), tags: frozenset[str] = frozenset()) -> pydantic_monty.ObjectMetadata:
    return pydantic_monty.ObjectMetadata(producers=producers, tags=tags)


# === Per-element key metadata survives storage and iteration ===


def test_dict_key_metadata_survives_iteration() -> None:
    """Key metadata is preserved when iterating over `d.keys()`.

    The dict is built inside the sandbox from a tagged input, then we read
    back the keys. The print callback exposes per-element metadata.
    """
    from typing import Any

    code = """
d = {k: 1}
for key in d:
    print(key)
"""
    m = pydantic_monty.Monty(code, inputs=['k'])
    calls: list[tuple[Any, pydantic_monty.ObjectMetadata]] = []

    def cb(stream: str, objects: list[pydantic_monty.AnnotatedValue], sep: str, end: str) -> None:
        for obj in objects:
            calls.append((obj.value, obj.metadata))

    m.run(
        inputs={'k': pydantic_monty.AnnotatedValue('mykey', _meta(producers=frozenset({'env'})))},
        structured_print_callback=cb,
    )
    assert len(calls) == snapshot(1)
    value, meta = calls[0]
    assert value == snapshot('mykey')
    # Iterating .keys() should produce the same metadata that was attached on insert.
    assert meta.producers == snapshot(frozenset({'env'}))


# === Re-insert with different key metadata: behavior pinned (not changed) ===


def test_dict_reinsert_same_key_different_meta() -> None:
    """Re-inserting an equal key with different metadata: lookup still succeeds.

    This pins down the *lookup* semantic regardless of what `set_with_meta`
    chooses to do about the stored `key_meta` on re-insert (overwrite vs.
    keep original — either is defensible). What must not break: the value
    looked up via the second key reference returns whatever was last set.
    """
    code = "d = {}; d[k1] = 1; d[k2] = 2; d[k1]"
    m = pydantic_monty.Monty(code, inputs=['k1', 'k2'])
    result = m.run(
        inputs={
            'k1': pydantic_monty.AnnotatedValue('shared', _meta(producers=frozenset({'src_a'}))),
            'k2': pydantic_monty.AnnotatedValue('shared', _meta(producers=frozenset({'src_b'}))),
        },
    )
    # k1 == k2 (same string content), so d[k1] and d[k2] target the same slot;
    # the second assignment overwrites the first regardless of metadata.
    assert result == snapshot(2)


# === Current behavior: dict subscript-store drops value metadata ===


def test_dict_subscript_store_drops_value_metadata() -> None:
    """Documents current behavior: `d[k] = v` stores v but loses v's metadata.

    Same `Dict::set` vs `Dict::set_with_meta` gap as `setattr` (see
    `test_metadata_setattr.py`). The compiler-emitted `StoreSubscript` opcode
    routes through `Dict::set`, which doesn't take metadata.

    When this gap is closed (compiler routes through `Dict::set_with_meta`),
    the assertion should flip to `frozenset({'val_src'})`.
    """
    from typing import Any

    code = """
d = {}
d[k] = v
print(d[k])
"""
    m = pydantic_monty.Monty(code, inputs=['k', 'v'])
    calls: list[tuple[Any, pydantic_monty.ObjectMetadata]] = []

    def cb(stream: str, objects: list[pydantic_monty.AnnotatedValue], sep: str, end: str) -> None:
        for obj in objects:
            calls.append((obj.value, obj.metadata))

    m.run(
        inputs={
            'k': pydantic_monty.AnnotatedValue('mykey', _meta(producers=frozenset({'key_src'}))),
            'v': pydantic_monty.AnnotatedValue('myval', _meta(producers=frozenset({'val_src'}))),
        },
        structured_print_callback=cb,
    )
    assert len(calls) == snapshot(1)
    value, meta = calls[0]
    assert value == snapshot('myval')
    # Current behavior: metadata is lost in Dict::set (not Dict::set_with_meta).
    # When the gap is closed, this should become `frozenset({'val_src'})`.
    assert meta.producers == snapshot(frozenset())
