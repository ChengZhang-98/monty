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
    assert meta.consumers is pydantic_monty.UNIVERSAL
    assert meta.tags == snapshot(frozenset())


def test_object_metadata_universal_consumers():
    meta = pydantic_monty.ObjectMetadata(producers=frozenset({'src'}))
    assert meta.consumers is pydantic_monty.UNIVERSAL


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


def test_object_metadata_rejects_empty_string_producers():
    import pytest

    with pytest.raises(ValueError) as exc_info:
        pydantic_monty.ObjectMetadata(producers=frozenset({'valid', ''}))
    assert exc_info.value.args[0] == snapshot('producers must not contain empty strings')


def test_object_metadata_rejects_empty_string_consumers():
    import pytest

    with pytest.raises(ValueError) as exc_info:
        pydantic_monty.ObjectMetadata(consumers=frozenset({''}))
    assert exc_info.value.args[0] == snapshot('consumers must not contain empty strings')


def test_object_metadata_rejects_empty_string_tags():
    import pytest

    with pytest.raises(ValueError) as exc_info:
        pydantic_monty.ObjectMetadata(tags=frozenset({''}))
    assert exc_info.value.args[0] == snapshot('tags must not contain empty strings')


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
    result = snap.resume({'return_value': pydantic_monty.AnnotatedValue('response', meta)})
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

    result = snap.resume({'return_value': 'plain_response'})
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


# === MontyRepl metadata propagation ===


def test_repl_metadata_input_passthrough():
    """MontyRepl.feed_start with AnnotatedValue input should preserve metadata."""
    meta = pydantic_monty.ObjectMetadata(
        producers=frozenset({'vault'}),
        tags=frozenset({'secret'}),
    )
    repl = pydantic_monty.MontyRepl(script_name='repl.py')
    result = repl.feed_start('x', inputs={'x': pydantic_monty.AnnotatedValue(42, meta)})
    assert isinstance(result, pydantic_monty.MontyComplete)
    assert result.output == snapshot(42)
    assert result.metadata is not None
    assert result.metadata.producers == snapshot(frozenset({'vault'}))
    assert result.metadata.tags == snapshot(frozenset({'secret'}))


def test_repl_metadata_merge_on_binary_op():
    """MontyRepl should merge metadata on binary ops just like Monty."""
    meta_a = pydantic_monty.ObjectMetadata(producers=frozenset({'src_a'}))
    meta_b = pydantic_monty.ObjectMetadata(producers=frozenset({'src_b'}))
    repl = pydantic_monty.MontyRepl(script_name='repl.py')
    result = repl.feed_start(
        'a + b',
        inputs={
            'a': pydantic_monty.AnnotatedValue(10, meta_a),
            'b': pydantic_monty.AnnotatedValue(20, meta_b),
        },
    )
    assert isinstance(result, pydantic_monty.MontyComplete)
    assert result.output == snapshot(30)
    assert result.metadata is not None
    assert result.metadata.producers == snapshot(frozenset({'src_a', 'src_b'}))


def test_repl_no_metadata_returns_none():
    """MontyRepl without AnnotatedValue inputs should return None metadata."""
    repl = pydantic_monty.MontyRepl(script_name='repl.py')
    result = repl.feed_start('1 + 2')
    assert isinstance(result, pydantic_monty.MontyComplete)
    assert result.output == snapshot(3)
    assert result.metadata is None


# === FunctionSnapshot annotated_args / annotated_kwargs ===


def test_annotated_args_on_function_snapshot():
    """FunctionSnapshot.annotated_args carries per-argument metadata."""
    meta = pydantic_monty.ObjectMetadata(
        producers=frozenset({'vault'}),
        consumers=frozenset({'internal'}),
    )
    code = 'fetch(api_key, url)'
    m = pydantic_monty.Monty(code, inputs=['api_key', 'url'])
    snap = m.start(
        inputs={
            'api_key': pydantic_monty.AnnotatedValue('secret', meta),
            'url': 'https://example.com',
        }
    )
    assert isinstance(snap, pydantic_monty.FunctionSnapshot)

    annotated = snap.annotated_args
    assert len(annotated) == snapshot(2)

    # First arg carries metadata
    assert annotated[0].value == snapshot('secret')
    assert annotated[0].metadata.producers == snapshot(frozenset({'vault'}))
    assert annotated[0].metadata.consumers == snapshot(frozenset({'internal'}))

    # Second arg has default (empty) metadata
    assert annotated[1].value == snapshot('https://example.com')
    assert annotated[1].metadata.producers == snapshot(frozenset())

    # Plain .args still works
    assert snap.args == snapshot(('secret', 'https://example.com'))


def test_annotated_kwargs_on_function_snapshot():
    """FunctionSnapshot.annotated_kwargs carries per-kwarg metadata."""
    meta = pydantic_monty.ObjectMetadata(
        producers=frozenset({'db'}),
        tags=frozenset({'pii'}),
    )
    code = 'fetch(timeout=t)'
    m = pydantic_monty.Monty(code, inputs=['t'])
    snap = m.start(
        inputs={
            't': pydantic_monty.AnnotatedValue(30, meta),
        }
    )
    assert isinstance(snap, pydantic_monty.FunctionSnapshot)

    annotated_kw = snap.annotated_kwargs
    assert 'timeout' in annotated_kw
    assert annotated_kw['timeout'].value == snapshot(30)
    assert annotated_kw['timeout'].metadata.producers == snapshot(frozenset({'db'}))
    assert annotated_kw['timeout'].metadata.tags == snapshot(frozenset({'pii'}))

    # Plain .kwargs still works
    assert snap.kwargs == snapshot({'timeout': 30})


def test_annotated_args_no_metadata():
    """FunctionSnapshot.annotated_args with plain inputs yields default metadata."""
    code = 'fetch(x)'
    m = pydantic_monty.Monty(code, inputs=['x'])
    snap = m.start(inputs={'x': 42})
    assert isinstance(snap, pydantic_monty.FunctionSnapshot)

    annotated = snap.annotated_args
    assert len(annotated) == snapshot(1)
    assert annotated[0].value == snapshot(42)
    # default metadata: empty producers, universal consumers (UNIVERSAL)
    assert annotated[0].metadata.producers == snapshot(frozenset())
    assert annotated[0].metadata.consumers is pydantic_monty.UNIVERSAL


def test_annotated_args_metadata_survives_serialization():
    """Metadata on FunctionSnapshot args should survive dump/load round-trip."""
    meta = pydantic_monty.ObjectMetadata(
        producers=frozenset({'vault'}),
        consumers=frozenset({'internal'}),
        tags=frozenset({'secret'}),
    )
    code = 'fetch(key)'
    m = pydantic_monty.Monty(code, inputs=['key'])
    snap = m.start(
        inputs={
            'key': pydantic_monty.AnnotatedValue('s3cr3t', meta),
        }
    )
    assert isinstance(snap, pydantic_monty.FunctionSnapshot)

    # Round-trip through serialization
    data = snap.dump()
    loaded = pydantic_monty.load_snapshot(data)
    assert isinstance(loaded, pydantic_monty.FunctionSnapshot)

    annotated = loaded.annotated_args
    assert len(annotated) == snapshot(1)
    assert annotated[0].value == snapshot('s3cr3t')
    assert annotated[0].metadata.producers == snapshot(frozenset({'vault'}))
    assert annotated[0].metadata.consumers == snapshot(frozenset({'internal'}))
    assert annotated[0].metadata.tags == snapshot(frozenset({'secret'}))


# === UNIVERSAL sentinel ===


def test_universal_singleton_identity():
    """UNIVERSAL is a singleton — identity check should pass."""
    assert pydantic_monty.UNIVERSAL is pydantic_monty.UNIVERSAL


def test_universal_isinstance():
    """UNIVERSAL is an instance of UniversalSet."""
    assert isinstance(pydantic_monty.UNIVERSAL, pydantic_monty.UniversalSet)


def test_universal_contains():
    """Membership in UNIVERSAL always returns True."""
    assert 'anything' in pydantic_monty.UNIVERSAL
    assert '' in pydantic_monty.UNIVERSAL


def test_universal_bool():
    """bool(UNIVERSAL) is True."""
    assert bool(pydantic_monty.UNIVERSAL) is True


def test_universal_repr():
    """repr(UNIVERSAL) is 'UNIVERSAL'."""
    assert repr(pydantic_monty.UNIVERSAL) == snapshot('UNIVERSAL')


def test_universal_equality():
    """UNIVERSAL compares equal to itself but not to other types."""
    assert pydantic_monty.UNIVERSAL == pydantic_monty.UNIVERSAL
    assert pydantic_monty.UNIVERSAL != frozenset()
    assert pydantic_monty.UNIVERSAL != None  # noqa: E711


def test_universal_iter_raises():
    """Iterating UNIVERSAL raises TypeError."""
    import pytest

    with pytest.raises(TypeError):
        iter(pydantic_monty.UNIVERSAL)  # pyright: ignore[reportCallIssue,reportArgumentType]


def test_universal_len_raises():
    """len(UNIVERSAL) raises TypeError."""
    import pytest

    with pytest.raises(TypeError):
        len(pydantic_monty.UNIVERSAL)  # pyright: ignore[reportArgumentType]


def test_object_metadata_explicit_universal_consumers():
    """Passing UNIVERSAL explicitly for consumers should work."""
    meta = pydantic_monty.ObjectMetadata(consumers=pydantic_monty.UNIVERSAL)
    assert meta.consumers is pydantic_monty.UNIVERSAL


def test_object_metadata_explicit_universal_producers():
    """Passing UNIVERSAL for producers should work."""
    meta = pydantic_monty.ObjectMetadata(producers=pydantic_monty.UNIVERSAL)
    assert meta.producers is pydantic_monty.UNIVERSAL
    assert meta.consumers is pydantic_monty.UNIVERSAL  # default
    assert meta.tags == snapshot(frozenset())


def test_object_metadata_explicit_universal_tags():
    """Passing UNIVERSAL for tags should work."""
    meta = pydantic_monty.ObjectMetadata(tags=pydantic_monty.UNIVERSAL)
    assert meta.tags is pydantic_monty.UNIVERSAL
    assert meta.producers == snapshot(frozenset())


def test_object_metadata_rejects_invalid_type():
    """Passing an invalid type raises TypeError."""
    import pytest

    with pytest.raises(TypeError):
        pydantic_monty.ObjectMetadata(producers='not_a_frozenset')  # pyright: ignore[reportArgumentType]


def test_object_metadata_universal_repr():
    """Repr with UNIVERSAL fields shows UNIVERSAL, not None."""
    meta = pydantic_monty.ObjectMetadata(producers=pydantic_monty.UNIVERSAL)
    r = repr(meta)
    assert 'UNIVERSAL' in r


def test_object_metadata_universal_equality():
    """Two ObjectMetadata with UNIVERSAL in same positions are equal."""
    a = pydantic_monty.ObjectMetadata(producers=pydantic_monty.UNIVERSAL)
    b = pydantic_monty.ObjectMetadata(producers=pydantic_monty.UNIVERSAL)
    assert a == b


def test_object_metadata_universal_vs_empty_not_equal():
    """UNIVERSAL and frozenset() are not equal for any field."""
    a = pydantic_monty.ObjectMetadata(producers=pydantic_monty.UNIVERSAL)
    b = pydantic_monty.ObjectMetadata(producers=frozenset())
    assert a != b


def test_universal_producers_roundtrip_through_interpreter():
    """UNIVERSAL producers should survive a round-trip through the interpreter."""
    meta = pydantic_monty.ObjectMetadata(
        producers=pydantic_monty.UNIVERSAL,
        consumers=frozenset({'admin'}),
    )
    m = pydantic_monty.Monty('x', inputs=['x'])
    snap = m.start(inputs={'x': pydantic_monty.AnnotatedValue(42, meta)})
    assert isinstance(snap, pydantic_monty.MontyComplete)
    assert snap.metadata is not None
    assert snap.metadata.producers is pydantic_monty.UNIVERSAL
    assert snap.metadata.consumers == snapshot(frozenset({'admin'}))


# === Container-level metadata propagation through indexing ===


def test_container_metadata_propagates_through_indexing():
    """Indexing a list that has container-level metadata (from input) should
    propagate the container's metadata to the extracted element."""
    container_meta = pydantic_monty.ObjectMetadata(
        producers=frozenset({'web_api'}),
        consumers=frozenset({'admin'}),
        tags=frozenset({'untrusted'}),
    )
    m = pydantic_monty.Monty('x[0]', inputs=['x'])
    snap = m.start(inputs={'x': pydantic_monty.AnnotatedValue([10, 20], container_meta)})
    assert isinstance(snap, pydantic_monty.MontyComplete)
    assert snap.output == snapshot(10)
    assert snap.metadata is not None
    assert snap.metadata.producers == snapshot(frozenset({'web_api'}))
    assert snap.metadata.consumers == snapshot(frozenset({'admin'}))
    assert snap.metadata.tags == snapshot(frozenset({'untrusted'}))


def test_container_metadata_propagates_through_negative_indexing():
    """Negative indexing should also propagate container metadata."""
    container_meta = pydantic_monty.ObjectMetadata(
        tags=frozenset({'non_executable'}),
    )
    m = pydantic_monty.Monty('x[-1]', inputs=['x'])
    snap = m.start(inputs={'x': pydantic_monty.AnnotatedValue([1, 2, 3], container_meta)})
    assert isinstance(snap, pydantic_monty.MontyComplete)
    assert snap.output == snapshot(3)
    assert snap.metadata is not None
    assert snap.metadata.tags == snapshot(frozenset({'non_executable'}))


def test_container_metadata_on_resume_propagates_through_indexing():
    """Metadata from an external function return (via resume) should propagate
    through indexing — the primary scenario from the tiny-beaver bug."""
    code = 'results = fetch()\nresults[0]'
    m = pydantic_monty.Monty(code)
    snap = m.start()
    assert isinstance(snap, pydantic_monty.FunctionSnapshot)

    container_meta = pydantic_monty.ObjectMetadata(
        tags=frozenset({'__non_executable'}),
    )
    result = snap.resume({'return_value': pydantic_monty.AnnotatedValue(['item_a', 'item_b'], container_meta)})
    assert isinstance(result, pydantic_monty.MontyComplete)
    assert result.output == snapshot('item_a')
    assert result.metadata is not None
    assert result.metadata.tags == snapshot(frozenset({'__non_executable'}))


# === Container-level metadata propagation through iteration ===


def test_container_metadata_propagates_through_for_loop():
    """Iterating a list with container-level metadata should propagate metadata
    to each yielded element."""
    container_meta = pydantic_monty.ObjectMetadata(
        producers=frozenset({'api'}),
        tags=frozenset({'untrusted'}),
    )
    code = 'total = 0\nfor item in x:\n    total = total + item\ntotal'
    m = pydantic_monty.Monty(code, inputs=['x'])
    snap = m.start(inputs={'x': pydantic_monty.AnnotatedValue([10, 20], container_meta)})
    assert isinstance(snap, pydantic_monty.MontyComplete)
    assert snap.output == snapshot(30)
    assert snap.metadata is not None
    assert snap.metadata.producers == snapshot(frozenset({'api'}))
    assert snap.metadata.tags == snapshot(frozenset({'untrusted'}))


def test_container_metadata_on_resume_propagates_through_iteration():
    """Metadata from external function return should propagate through a for loop."""
    code = 'results = fetch()\nfirst = None\nfor r in results:\n    first = r\n    break\nfirst'
    m = pydantic_monty.Monty(code)
    snap = m.start()
    assert isinstance(snap, pydantic_monty.FunctionSnapshot)

    container_meta = pydantic_monty.ObjectMetadata(
        tags=frozenset({'__non_executable'}),
    )
    result = snap.resume({'return_value': pydantic_monty.AnnotatedValue(['a', 'b'], container_meta)})
    assert isinstance(result, pydantic_monty.MontyComplete)
    assert result.output == snapshot('a')
    assert result.metadata is not None
    assert result.metadata.tags == snapshot(frozenset({'__non_executable'}))


# === Container metadata propagation through indexing then further ops ===


def test_container_metadata_propagates_through_indexing_then_fstring():
    """f"info: {x[0]}" should carry the container's metadata."""
    container_meta = pydantic_monty.ObjectMetadata(
        tags=frozenset({'secret'}),
    )
    code = "f'info: {x[0]}'"
    m = pydantic_monty.Monty(code, inputs=['x'])
    snap = m.start(inputs={'x': pydantic_monty.AnnotatedValue(['data'], container_meta)})
    assert isinstance(snap, pydantic_monty.MontyComplete)
    assert snap.output == snapshot('info: data')
    assert snap.metadata is not None
    assert snap.metadata.tags == snapshot(frozenset({'secret'}))


def test_container_metadata_propagates_through_indexing_then_concat():
    """x[0] + " suffix" should carry the container's metadata."""
    container_meta = pydantic_monty.ObjectMetadata(
        tags=frozenset({'non_executable'}),
    )
    code = "x[0] + ' suffix'"
    m = pydantic_monty.Monty(code, inputs=['x'])
    snap = m.start(inputs={'x': pydantic_monty.AnnotatedValue(['hello'], container_meta)})
    assert isinstance(snap, pydantic_monty.MontyComplete)
    assert snap.output == snapshot('hello suffix')
    assert snap.metadata is not None
    assert snap.metadata.tags == snapshot(frozenset({'non_executable'}))


# === Container metadata in structured print callback ===


def test_container_metadata_visible_in_structured_print_callback_after_indexing():
    """structured_print_callback should see container metadata on indexed elements."""
    container_meta = pydantic_monty.ObjectMetadata(
        tags=frozenset({'__non_executable'}),
    )
    print_log: list[pydantic_monty.AnnotatedValue] = []

    def on_print(stream: str, objects: list[pydantic_monty.AnnotatedValue], sep: str, end: str) -> None:
        for obj in objects:
            print_log.append(obj)

    code = 'results = fetch()\nprint(results[0])'
    m = pydantic_monty.Monty(code)
    snap = m.start(structured_print_callback=on_print)
    assert isinstance(snap, pydantic_monty.FunctionSnapshot)

    snap.resume({'return_value': pydantic_monty.AnnotatedValue(['item_a', 'item_b'], container_meta)})
    assert len(print_log) == snapshot(1)
    assert print_log[0].value == snapshot('item_a')
    assert print_log[0].metadata.tags == snapshot(frozenset({'__non_executable'}))


def test_container_metadata_visible_in_structured_print_callback_after_iteration():
    """structured_print_callback should see container metadata on iterated elements."""
    container_meta = pydantic_monty.ObjectMetadata(
        tags=frozenset({'__non_executable'}),
    )
    print_log: list[pydantic_monty.AnnotatedValue] = []

    def on_print(stream: str, objects: list[pydantic_monty.AnnotatedValue], sep: str, end: str) -> None:
        for obj in objects:
            print_log.append(obj)

    code = 'results = fetch()\nfor r in results:\n    print(r)'
    m = pydantic_monty.Monty(code)
    snap = m.start(structured_print_callback=on_print)
    assert isinstance(snap, pydantic_monty.FunctionSnapshot)

    snap.resume({'return_value': pydantic_monty.AnnotatedValue(['a', 'b'], container_meta)})
    assert len(print_log) == snapshot(2)
    assert print_log[0].value == snapshot('a')
    assert print_log[0].metadata.tags == snapshot(frozenset({'__non_executable'}))
    assert print_log[1].value == snapshot('b')
    assert print_log[1].metadata.tags == snapshot(frozenset({'__non_executable'}))


# === Fix #4: next() propagates metadata ===


def test_next_propagates_iterator_metadata():
    """next(iter(x)) should propagate the container's metadata to the returned element."""
    container_meta = pydantic_monty.ObjectMetadata(
        producers=frozenset({'api'}),
        tags=frozenset({'untrusted'}),
    )
    code = 'it = iter(x)\nnext(it)'
    m = pydantic_monty.Monty(code, inputs=['x'])
    snap = m.start(inputs={'x': pydantic_monty.AnnotatedValue([10, 20], container_meta)})
    assert isinstance(snap, pydantic_monty.MontyComplete)
    assert snap.output == snapshot(10)
    assert snap.metadata is not None
    assert snap.metadata.producers == snapshot(frozenset({'api'}))
    assert snap.metadata.tags == snapshot(frozenset({'untrusted'}))


def test_next_on_resume_propagates_metadata():
    """next() on an iterator created from an external function return
    should propagate the container's metadata."""
    code = 'results = fetch()\nit = iter(results)\nnext(it)'
    m = pydantic_monty.Monty(code)
    snap = m.start()
    assert isinstance(snap, pydantic_monty.FunctionSnapshot)

    container_meta = pydantic_monty.ObjectMetadata(
        tags=frozenset({'__non_executable'}),
    )
    result = snap.resume({'return_value': pydantic_monty.AnnotatedValue(['a', 'b'], container_meta)})
    assert isinstance(result, pydantic_monty.MontyComplete)
    assert result.output == snapshot('a')
    assert result.metadata is not None
    assert result.metadata.tags == snapshot(frozenset({'__non_executable'}))


# === Fix #6: Builtins propagate container metadata ===


def test_sum_propagates_container_metadata():
    """sum(x) where x has container metadata should propagate it."""
    container_meta = pydantic_monty.ObjectMetadata(
        producers=frozenset({'api'}),
        tags=frozenset({'financial'}),
    )
    m = pydantic_monty.Monty('sum(x)', inputs=['x'])
    snap = m.start(inputs={'x': pydantic_monty.AnnotatedValue([10, 20], container_meta)})
    assert isinstance(snap, pydantic_monty.MontyComplete)
    assert snap.output == snapshot(30)
    assert snap.metadata is not None
    assert snap.metadata.producers == snapshot(frozenset({'api'}))
    assert snap.metadata.tags == snapshot(frozenset({'financial'}))


def test_min_propagates_container_metadata():
    """min(x) where x has container metadata should propagate it."""
    container_meta = pydantic_monty.ObjectMetadata(
        tags=frozenset({'measurement'}),
    )
    m = pydantic_monty.Monty('min(x)', inputs=['x'])
    snap = m.start(inputs={'x': pydantic_monty.AnnotatedValue([5, 3, 8], container_meta)})
    assert isinstance(snap, pydantic_monty.MontyComplete)
    assert snap.output == snapshot(3)
    assert snap.metadata is not None
    assert snap.metadata.tags == snapshot(frozenset({'measurement'}))


def test_max_propagates_container_metadata():
    """max(x) where x has container metadata should propagate it."""
    container_meta = pydantic_monty.ObjectMetadata(
        tags=frozenset({'measurement'}),
    )
    m = pydantic_monty.Monty('max(x)', inputs=['x'])
    snap = m.start(inputs={'x': pydantic_monty.AnnotatedValue([5, 3, 8], container_meta)})
    assert isinstance(snap, pydantic_monty.MontyComplete)
    assert snap.output == snapshot(8)
    assert snap.metadata is not None
    assert snap.metadata.tags == snapshot(frozenset({'measurement'}))


def test_sum_on_resume_propagates_container_metadata():
    """sum() on an external function return should propagate container metadata."""
    code = 'results = fetch()\nsum(results)'
    m = pydantic_monty.Monty(code)
    snap = m.start()
    assert isinstance(snap, pydantic_monty.FunctionSnapshot)

    container_meta = pydantic_monty.ObjectMetadata(
        tags=frozenset({'__non_executable'}),
    )
    result = snap.resume({'return_value': pydantic_monty.AnnotatedValue([10, 20, 30], container_meta)})
    assert isinstance(result, pydantic_monty.MontyComplete)
    assert result.output == snapshot(60)
    assert result.metadata is not None
    assert result.metadata.tags == snapshot(frozenset({'__non_executable'}))


# === Fix #5: dict.items() propagates key/value metadata to tuple elements ===


# Note: DictItemsView per-entry metadata propagation (key_meta/value_meta flowing into
# tuple elements) is tested at the Rust level in crates/monty/tests/metadata.rs.
# The Python API can't easily construct annotated dicts with per-entry metadata, and
# container-level metadata doesn't flow through .items() method calls (method call
# metadata propagation is a separate deferred feature).


# === Fix #9: enumerate() preserves element metadata when unpacking ===


def test_enumerate_propagates_metadata_to_unpacked_value():
    """for i, r in enumerate(items) — the value variable r must carry the
    container's metadata.  Previously the intermediate (index, value) tuple was
    built without per-element metadata so `r` arrived with DEFAULT metadata."""
    container_meta = pydantic_monty.ObjectMetadata(
        tags=frozenset({'__non_executable'}),
    )
    print_log: list[pydantic_monty.AnnotatedValue] = []

    def on_print(stream: str, objects: list[pydantic_monty.AnnotatedValue], sep: str, end: str) -> None:
        for obj in objects:
            print_log.append(obj)

    code = 'results = fetch()\nfor i, r in enumerate(results):\n    print(r)'
    m = pydantic_monty.Monty(code)
    snap = m.start(structured_print_callback=on_print)
    assert isinstance(snap, pydantic_monty.FunctionSnapshot)

    snap.resume({'return_value': pydantic_monty.AnnotatedValue(['a', 'b'], container_meta)})
    assert len(print_log) == snapshot(2)
    assert print_log[0].value == snapshot('a')
    assert print_log[0].metadata.tags == snapshot(frozenset({'__non_executable'}))
    assert print_log[1].value == snapshot('b')
    assert print_log[1].metadata.tags == snapshot(frozenset({'__non_executable'}))


def test_enumerate_index_has_no_metadata():
    """The index variable i in `for i, r in enumerate(items)` should have
    default (empty) metadata — it is a synthesised integer, not data from the
    tagged container."""
    container_meta = pydantic_monty.ObjectMetadata(
        tags=frozenset({'__non_executable'}),
    )
    print_log: list[pydantic_monty.AnnotatedValue] = []

    def on_print(stream: str, objects: list[pydantic_monty.AnnotatedValue], sep: str, end: str) -> None:
        for obj in objects:
            print_log.append(obj)

    code = 'results = fetch()\nfor i, r in enumerate(results):\n    print(i)'
    m = pydantic_monty.Monty(code)
    snap = m.start(structured_print_callback=on_print)
    assert isinstance(snap, pydantic_monty.FunctionSnapshot)

    snap.resume({'return_value': pydantic_monty.AnnotatedValue(['a', 'b'], container_meta)})
    assert len(print_log) == snapshot(2)
    assert print_log[0].value == snapshot(0)
    assert print_log[0].metadata.tags == snapshot(frozenset())
    assert print_log[1].value == snapshot(1)
    assert print_log[1].metadata.tags == snapshot(frozenset())
