"""Tests for the ``structured_print_callback`` extension.

Extension-only tests; see docs/extensions/ for details.
"""

from typing import Callable

import pytest
from inline_snapshot import snapshot

import pydantic_monty

# === structured_print_callback tests ===


StructuredCall = tuple[str, list[object], str, str]
StructuredPrintCallback = Callable[[str, list[pydantic_monty.AnnotatedValue], str, str], None]


def make_structured_collector() -> tuple[list[StructuredCall], StructuredPrintCallback]:
    """Create a structured print callback that collects calls into a list.

    Unwraps ``AnnotatedValue`` objects to plain values so existing tests
    can compare against simple Python literals.
    """
    calls: list[StructuredCall] = []

    def callback(stream: str, objects: list[pydantic_monty.AnnotatedValue], sep: str, end: str) -> None:
        assert stream == 'stdout'
        calls.append((stream, [obj.value for obj in objects], sep, end))

    return calls, callback


def test_structured_print_basic() -> None:
    m = pydantic_monty.Monty('print("hello")')
    calls, callback = make_structured_collector()
    m.run(structured_print_callback=callback)
    assert calls == snapshot([('stdout', ['hello'], ' ', '\n')])


def test_structured_print_multiple_args() -> None:
    m = pydantic_monty.Monty('print(1, "hello", [1, 2])')
    calls, callback = make_structured_collector()
    m.run(structured_print_callback=callback)
    assert calls == snapshot([('stdout', [1, 'hello', [1, 2]], ' ', '\n')])


def test_structured_print_preserves_types() -> None:
    m = pydantic_monty.Monty('print(42, 3.14, True, None, "text")')
    calls, callback = make_structured_collector()
    m.run(structured_print_callback=callback)
    assert len(calls) == snapshot(1)
    _, objects, _, _ = calls[0]
    assert objects == snapshot([42, 3.14, True, None, 'text'])
    assert type(objects[0]) is int
    assert type(objects[1]) is float
    assert type(objects[2]) is bool
    assert objects[3] is None
    assert type(objects[4]) is str


def test_structured_print_nested_containers() -> None:
    m = pydantic_monty.Monty('print({"a": [1, 2]}, (3, 4))')
    calls, callback = make_structured_collector()
    m.run(structured_print_callback=callback)
    assert calls == snapshot([('stdout', [{'a': [1, 2]}, (3, 4)], ' ', '\n')])


def test_structured_print_custom_sep_end() -> None:
    m = pydantic_monty.Monty('print(1, 2, 3, sep="-", end="!")')
    calls, callback = make_structured_collector()
    m.run(structured_print_callback=callback)
    assert calls == snapshot([('stdout', [1, 2, 3], '-', '!')])


def test_structured_print_empty() -> None:
    m = pydantic_monty.Monty('print()')
    calls, callback = make_structured_collector()
    m.run(structured_print_callback=callback)
    assert calls == snapshot([('stdout', [], ' ', '\n')])


def test_structured_print_multiple_calls() -> None:
    code = """
print("line 1")
print(42)
"""
    m = pydantic_monty.Monty(code)
    calls, callback = make_structured_collector()
    m.run(structured_print_callback=callback)
    assert calls == snapshot(
        [
            ('stdout', ['line 1'], ' ', '\n'),
            ('stdout', [42], ' ', '\n'),
        ]
    )


def test_structured_print_non_serializable_fallback() -> None:
    """Non-JSON-serializable types produce NonSerializable objects."""
    code = 'print(range(5))'
    m = pydantic_monty.Monty(code)
    calls, callback = make_structured_collector()
    m.run(structured_print_callback=callback)
    assert len(calls) == snapshot(1)
    _, objects, _, _ = calls[0]
    # range() is not JSON-serializable, so it becomes NonSerializable
    assert len(objects) == 1
    obj = objects[0]
    assert isinstance(obj, pydantic_monty.NonSerializable)
    assert obj.type_name == snapshot('range')
    assert obj.repr == snapshot('range(0, 5)')
    # str() returns the repr string for backward compatibility
    assert str(obj) == snapshot('range(0, 5)')


def test_structured_print_both_callbacks_error() -> None:
    """Providing both print_callback and structured_print_callback should raise."""
    m = pydantic_monty.Monty('print("hello")')
    with pytest.raises(ValueError, match='cannot specify both'):
        m.run(
            print_callback=lambda stream, text: None,
            structured_print_callback=lambda stream, objects, sep, end: None,
        )


def test_structured_print_repl_feed_run() -> None:
    """Test structured_print_callback works with MontyRepl.feed_run."""
    repl = pydantic_monty.MontyRepl()
    calls, callback = make_structured_collector()
    repl.feed_run('print(1, "two", [3])', structured_print_callback=callback)
    assert calls == snapshot([('stdout', [1, 'two', [3]], ' ', '\n')])


def test_structured_print_repl_feed_start() -> None:
    """Test structured_print_callback works with MontyRepl.feed_start."""
    repl = pydantic_monty.MontyRepl()
    calls, callback = make_structured_collector()
    result = repl.feed_start('print(1, 2)', structured_print_callback=callback)
    assert isinstance(result, pydantic_monty.MontyComplete)
    assert calls == snapshot([('stdout', [1, 2], ' ', '\n')])


def test_non_serializable_isinstance_check() -> None:
    """NonSerializable can be detected with isinstance() in callback."""
    code = 'print(1, range(3), "hello")'
    m = pydantic_monty.Monty(code)
    calls, callback = make_structured_collector()
    m.run(structured_print_callback=callback)
    _, objects, _, _ = calls[0]
    assert not isinstance(objects[0], pydantic_monty.NonSerializable)
    assert isinstance(objects[1], pydantic_monty.NonSerializable)
    assert not isinstance(objects[2], pydantic_monty.NonSerializable)


def test_non_serializable_equality() -> None:
    """NonSerializable objects support equality comparison."""
    a = pydantic_monty.NonSerializable('range', 'range(0, 5)')
    b = pydantic_monty.NonSerializable('range', 'range(0, 5)')
    c = pydantic_monty.NonSerializable('iterator', '<iterator>')
    assert a == b
    assert a != c


def test_non_serializable_iterator() -> None:
    """Iterators produce NonSerializable with type_name='iterator'."""
    code = 'print(iter([1, 2, 3]))'
    m = pydantic_monty.Monty(code)
    calls, callback = make_structured_collector()
    m.run(structured_print_callback=callback)
    _, objects, _, _ = calls[0]
    obj = objects[0]
    assert isinstance(obj, pydantic_monty.NonSerializable)
    assert obj.type_name == snapshot('iterator')


def test_structured_print_after_resume() -> None:
    """Structured callback works after feed_start + resume (regression test).

    Previously, resume() always created CallbackStringPrint even when the stored
    callback was a StructuredCallbackMarker, causing TypeError when print() was
    called with non-literal arguments after resume.
    """
    repl = pydantic_monty.MontyRepl()
    calls, callback = make_structured_collector()
    code = 'x = get_value()\nprint(f"result: {x}")'
    result = repl.feed_start(code, structured_print_callback=callback)
    assert isinstance(result, pydantic_monty.FunctionSnapshot)
    result = result.resume({'return_value': 42})
    assert isinstance(result, pydantic_monty.MontyComplete)
    assert calls == snapshot([('stdout', ['result: 42'], ' ', '\n')])


def test_structured_print_type_of_dataclass() -> None:
    """type() on a registered dataclass doesn't crash structured_print_callback (regression).

    Previously, printing type() of a dataclass instance via structured_print_callback
    raised AttributeError because the conversion tried to look up 'dataclass' in
    Python's builtins module.
    """
    from dataclasses import dataclass

    @dataclass(frozen=True)
    class WebFetchResult:
        status_code: int
        content: str

    repl = pydantic_monty.MontyRepl(dataclass_registry=[WebFetchResult])
    calls, callback = make_structured_collector()

    # Assign a dataclass instance via external function resume
    result = repl.feed_start(
        'page = fetch_page()',
        structured_print_callback=callback,
    )
    assert isinstance(result, pydantic_monty.FunctionSnapshot)
    result = result.resume({'return_value': WebFetchResult(status_code=200, content='hello')})
    assert isinstance(result, pydantic_monty.MontyComplete)

    # print(type(page)) should not raise
    calls.clear()
    result = repl.feed_start(
        'print(type(page))',
        structured_print_callback=callback,
    )
    assert isinstance(result, pydantic_monty.MontyComplete)
    assert len(calls) == snapshot(1)
    _, objects, _, _ = calls[0]
    obj = objects[0]
    assert isinstance(obj, pydantic_monty.NonSerializable)
    assert obj.type_name == snapshot('type')
    assert obj.repr == snapshot("<class 'dataclass'>")


# === structured_print_callback metadata tests ===


def make_annotated_structured_collector() -> tuple[
    list[tuple[str, list[pydantic_monty.AnnotatedValue], str, str]], StructuredPrintCallback
]:
    """Create a structured print callback that keeps full AnnotatedValue objects."""
    calls: list[tuple[str, list[pydantic_monty.AnnotatedValue], str, str]] = []

    def callback(stream: str, objects: list[pydantic_monty.AnnotatedValue], sep: str, end: str) -> None:
        assert stream == 'stdout'
        calls.append((stream, list(objects), sep, end))

    return calls, callback


def test_structured_print_annotated_value_type() -> None:
    """Each object in the callback is an AnnotatedValue with .value and .metadata."""
    m = pydantic_monty.Monty('print("hello")')
    calls, callback = make_annotated_structured_collector()
    m.run(structured_print_callback=callback)
    assert len(calls) == snapshot(1)
    _, objects, _, _ = calls[0]
    assert len(objects) == snapshot(1)
    obj = objects[0]
    assert isinstance(obj, pydantic_monty.AnnotatedValue)
    assert obj.value == snapshot('hello')
    assert isinstance(obj.metadata, pydantic_monty.ObjectMetadata)


def test_structured_print_literal_has_default_metadata() -> None:
    """Literal args have DEFAULT metadata (empty producers, universal consumers, empty tags)."""
    m = pydantic_monty.Monty('print("txt", 42)')
    calls, callback = make_annotated_structured_collector()
    m.run(structured_print_callback=callback)
    _, objects, _, _ = calls[0]
    for obj in objects:
        assert obj.metadata.producers == snapshot(frozenset())
        assert obj.metadata.consumers is pydantic_monty.UNIVERSAL
        assert obj.metadata.tags == snapshot(frozenset())


def test_structured_print_propagates_input_metadata() -> None:
    """Metadata from annotated inputs propagates to print callback args."""
    code = 'print(x)'
    m = pydantic_monty.Monty(code, inputs=['x'])
    meta = pydantic_monty.ObjectMetadata(producers=frozenset({'vault'}), tags=frozenset({'secret'}))
    calls, callback = make_annotated_structured_collector()
    m.run(
        inputs={'x': pydantic_monty.AnnotatedValue(42, meta)},
        structured_print_callback=callback,
    )
    _, objects, _, _ = calls[0]
    assert len(objects) == snapshot(1)
    assert objects[0].value == snapshot(42)
    assert objects[0].metadata.producers == snapshot(frozenset({'vault'}))
    assert objects[0].metadata.tags == snapshot(frozenset({'secret'}))


def test_structured_print_merged_metadata() -> None:
    """When a print arg is computed from multiple tracked values, metadata merges."""
    code = 'print(a + b)'
    m = pydantic_monty.Monty(code, inputs=['a', 'b'])
    meta_a = pydantic_monty.ObjectMetadata(producers=frozenset({'api'}), tags=frozenset({'external'}))
    meta_b = pydantic_monty.ObjectMetadata(
        producers=frozenset({'db'}), consumers=frozenset({'admin'}), tags=frozenset({'internal'})
    )
    calls, callback = make_annotated_structured_collector()
    m.run(
        inputs={
            'a': pydantic_monty.AnnotatedValue(10, meta_a),
            'b': pydantic_monty.AnnotatedValue(20, meta_b),
        },
        structured_print_callback=callback,
    )
    _, objects, _, _ = calls[0]
    assert objects[0].value == snapshot(30)
    # producers: union
    assert objects[0].metadata.producers == snapshot(frozenset({'api', 'db'}))
    # consumers: intersection (None & {'admin'} = {'admin'})
    assert objects[0].metadata.consumers == snapshot(frozenset({'admin'}))
    # tags: union
    assert objects[0].metadata.tags == snapshot(frozenset({'external', 'internal'}))


def test_structured_print_mixed_tracked_and_untracked() -> None:
    """Mix of tracked and untracked args — each carries its own metadata."""
    code = 'print(x, "literal")'
    m = pydantic_monty.Monty(code, inputs=['x'])
    meta = pydantic_monty.ObjectMetadata(producers=frozenset({'sensor'}))
    calls, callback = make_annotated_structured_collector()
    m.run(
        inputs={'x': pydantic_monty.AnnotatedValue(99, meta)},
        structured_print_callback=callback,
    )
    _, objects, _, _ = calls[0]
    # First arg carries input metadata
    assert objects[0].value == snapshot(99)
    assert objects[0].metadata.producers == snapshot(frozenset({'sensor'}))
    # Second arg is a literal — default metadata
    assert objects[1].value == snapshot('literal')
    assert objects[1].metadata.producers == snapshot(frozenset())
