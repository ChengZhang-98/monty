"""Tests for the metadata propagation extension Python API.

Verifies that ``ObjectMetadata`` and ``AnnotatedValue`` can be used to
attach and read per-value provenance metadata through the interpreter.
"""

from inline_snapshot import snapshot

import pydantic_monty

# === ObjectMetadata construction ===


def test_object_metadata_construction():
    meta = pydantic_monty.ObjectMetadata(
        producers=frozenset({'vault'}),
        consumers=frozenset({'admin'}),
        tags=frozenset({'pii'}),
    )
    assert meta.producers == snapshot(frozenset({'vault'}))
    assert meta.consumers == snapshot(frozenset({'admin'}))
    assert meta.tags == snapshot(frozenset({'pii'}))


def test_object_metadata_defaults():
    meta = pydantic_monty.ObjectMetadata()
    assert meta.producers == snapshot(frozenset())
    assert meta.consumers is None
    assert meta.tags == snapshot(frozenset())


def test_object_metadata_universal_consumers():
    meta = pydantic_monty.ObjectMetadata(producers=frozenset({'src'}))
    assert meta.consumers is None


def test_object_metadata_equality():
    a = pydantic_monty.ObjectMetadata(producers=frozenset({'src'}), tags=frozenset({'t'}))
    b = pydantic_monty.ObjectMetadata(producers=frozenset({'src'}), tags=frozenset({'t'}))
    assert a == b


def test_object_metadata_inequality():
    a = pydantic_monty.ObjectMetadata(producers=frozenset({'src_a'}))
    b = pydantic_monty.ObjectMetadata(producers=frozenset({'src_b'}))
    assert a != b


def test_object_metadata_repr():
    meta = pydantic_monty.ObjectMetadata(producers=frozenset({'vault'}))
    r = repr(meta)
    assert 'ObjectMetadata' in r
    assert 'vault' in r


# === AnnotatedValue construction ===


def test_annotated_value_construction():
    meta = pydantic_monty.ObjectMetadata(producers=frozenset({'src'}))
    av = pydantic_monty.AnnotatedValue(42, meta)
    assert av.value == snapshot(42)
    assert av.metadata == meta


def test_annotated_value_repr():
    meta = pydantic_monty.ObjectMetadata(producers=frozenset({'src'}))
    av = pydantic_monty.AnnotatedValue('hello', meta)
    r = repr(av)
    assert 'AnnotatedValue' in r
    assert 'hello' in r


# === Input metadata passthrough ===


def test_input_metadata_passthrough():
    """An annotated input passed straight through should retain its metadata."""
    meta = pydantic_monty.ObjectMetadata(
        producers=frozenset({'vault'}),
        consumers=frozenset({'admin'}),
        tags=frozenset({'pii'}),
    )
    av = pydantic_monty.AnnotatedValue(42, meta)
    m = pydantic_monty.Monty('x', inputs=['x'])
    snap = m.start(inputs={'x': av})
    assert isinstance(snap, pydantic_monty.MontyComplete)
    assert snap.output == snapshot(42)
    assert snap.metadata is not None
    assert snap.metadata.producers == snapshot(frozenset({'vault'}))
    assert snap.metadata.consumers == snapshot(frozenset({'admin'}))
    assert snap.metadata.tags == snapshot(frozenset({'pii'}))


def test_input_no_metadata_returns_none():
    """A plain input (no AnnotatedValue) should produce None metadata."""
    m = pydantic_monty.Monty('x', inputs=['x'])
    snap = m.start(inputs={'x': 42})
    assert isinstance(snap, pydantic_monty.MontyComplete)
    assert snap.output == snapshot(42)
    assert snap.metadata is None


def test_metadata_merge_on_binary_op():
    """Binary ops should merge metadata: producers union, consumers intersection."""
    meta_a = pydantic_monty.ObjectMetadata(
        producers=frozenset({'src_a'}),
        consumers=frozenset({'c1', 'c2'}),
    )
    meta_b = pydantic_monty.ObjectMetadata(
        producers=frozenset({'src_b'}),
        consumers=frozenset({'c2', 'c3'}),
    )
    m = pydantic_monty.Monty('a + b', inputs=['a', 'b'])
    snap = m.start(
        inputs={
            'a': pydantic_monty.AnnotatedValue(10, meta_a),
            'b': pydantic_monty.AnnotatedValue(20, meta_b),
        }
    )
    assert isinstance(snap, pydantic_monty.MontyComplete)
    assert snap.output == snapshot(30)
    assert snap.metadata is not None
    # producers: union
    assert snap.metadata.producers == snapshot(frozenset({'src_a', 'src_b'}))
    # consumers: intersection
    assert snap.metadata.consumers == snapshot(frozenset({'c2'}))


def test_metadata_propagates_through_function():
    """Metadata should propagate through function calls."""
    meta = pydantic_monty.ObjectMetadata(
        producers=frozenset({'secret'}),
        tags=frozenset({'classified'}),
    )
    code = 'def double(n):\n    return n * 2\ndouble(x)'
    m = pydantic_monty.Monty(code, inputs=['x'])
    snap = m.start(inputs={'x': pydantic_monty.AnnotatedValue(5, meta)})
    assert isinstance(snap, pydantic_monty.MontyComplete)
    assert snap.output == snapshot(10)
    assert snap.metadata is not None
    assert snap.metadata.producers == snapshot(frozenset({'secret'}))
    assert snap.metadata.tags == snapshot(frozenset({'classified'}))


# === Resume with metadata ===


def test_resume_with_annotated_return_value():
    """Resuming with an AnnotatedValue should carry metadata to the output."""
    m = pydantic_monty.Monty('process(1)')
    snap = m.start()
    assert isinstance(snap, pydantic_monty.FunctionSnapshot)

    meta = pydantic_monty.ObjectMetadata(
        producers=frozenset({'api_call'}),
        tags=frozenset({'external'}),
    )
    result = snap.resume(return_value=pydantic_monty.AnnotatedValue('response', meta))
    assert isinstance(result, pydantic_monty.MontyComplete)
    assert result.output == snapshot('response')
    assert result.metadata is not None
    assert result.metadata.producers == snapshot(frozenset({'api_call'}))
    assert result.metadata.tags == snapshot(frozenset({'external'}))


def test_resume_without_metadata():
    """Resuming with a plain value should produce None metadata."""
    m = pydantic_monty.Monty('process(1)')
    snap = m.start()
    assert isinstance(snap, pydantic_monty.FunctionSnapshot)

    result = snap.resume(return_value='plain_response')
    assert isinstance(result, pydantic_monty.MontyComplete)
    assert result.output == snapshot('plain_response')
    assert result.metadata is None


# === Backward compatibility ===


def test_plain_inputs_still_work():
    """Existing code without metadata should work unchanged."""
    m = pydantic_monty.Monty('x + y', inputs=['x', 'y'])
    assert m.run(inputs={'x': 1, 'y': 2}) == snapshot(3)


def test_mixed_annotated_and_plain_inputs():
    """Mix of annotated and plain inputs should work."""
    meta = pydantic_monty.ObjectMetadata(producers=frozenset({'src'}))
    m = pydantic_monty.Monty('a + b', inputs=['a', 'b'])
    snap = m.start(
        inputs={
            'a': pydantic_monty.AnnotatedValue(10, meta),
            'b': 20,
        }
    )
    assert isinstance(snap, pydantic_monty.MontyComplete)
    assert snap.output == snapshot(30)
    # metadata should come from the annotated input (merge with default = identity)
    assert snap.metadata is not None
    assert snap.metadata.producers == snapshot(frozenset({'src'}))
