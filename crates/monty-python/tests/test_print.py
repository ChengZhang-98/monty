from typing import Callable, Literal

import pytest
from inline_snapshot import snapshot

import pydantic_monty

PrintCallback = Callable[[Literal['stdout'], str], None]


def make_print_collector() -> tuple[list[str], PrintCallback]:
    """Create a print callback that collects output into a list."""
    output: list[str] = []

    def callback(stream: Literal['stdout'], text: str) -> None:
        assert stream == 'stdout'
        output.append(text)

    return output, callback


def test_print_basic() -> None:
    m = pydantic_monty.Monty('print("hello")')
    output, callback = make_print_collector()
    m.run(print_callback=callback)
    assert ''.join(output) == snapshot('hello\n')


def test_print_multiple() -> None:
    code = """
print("line 1")
print("line 2")
"""
    m = pydantic_monty.Monty(code)
    output, callback = make_print_collector()
    m.run(print_callback=callback)
    assert ''.join(output) == snapshot('line 1\nline 2\n')


def test_print_with_values() -> None:
    m = pydantic_monty.Monty('print(1, 2, 3)')
    output, callback = make_print_collector()
    m.run(print_callback=callback)
    assert ''.join(output) == snapshot('1 2 3\n')


def test_print_with_sep() -> None:
    m = pydantic_monty.Monty('print(1, 2, 3, sep="-")')
    output, callback = make_print_collector()
    m.run(print_callback=callback)
    assert ''.join(output) == snapshot('1-2-3\n')


def test_print_with_end() -> None:
    m = pydantic_monty.Monty('print("hello", end="!")')
    output, callback = make_print_collector()
    m.run(print_callback=callback)
    assert ''.join(output) == snapshot('hello!')


def test_print_returns_none() -> None:
    m = pydantic_monty.Monty('print("test")')
    _, callback = make_print_collector()
    result = m.run(print_callback=callback)
    assert result is None


def test_print_empty() -> None:
    m = pydantic_monty.Monty('print()')
    output, callback = make_print_collector()
    m.run(print_callback=callback)
    assert ''.join(output) == snapshot('\n')


def test_print_with_limits() -> None:
    """Verify print_callback works together with resource limits."""
    m = pydantic_monty.Monty('print("with limits")')
    output, callback = make_print_collector()
    limits = pydantic_monty.ResourceLimits(max_duration_secs=5.0)
    m.run(print_callback=callback, limits=limits)
    assert ''.join(output) == snapshot('with limits\n')


def test_print_with_inputs() -> None:
    """Verify print_callback works together with inputs."""
    m = pydantic_monty.Monty('print(x)', inputs=['x'])
    output, callback = make_print_collector()
    m.run(inputs={'x': 42}, print_callback=callback)
    assert ''.join(output) == snapshot('42\n')


def test_print_in_loop() -> None:
    code = """
for i in range(3):
    print(i)
"""
    m = pydantic_monty.Monty(code)
    output, callback = make_print_collector()
    m.run(print_callback=callback)
    assert ''.join(output) == snapshot('0\n1\n2\n')


def test_print_mixed_types() -> None:
    m = pydantic_monty.Monty('print(1, "hello", True, None)')
    output, callback = make_print_collector()
    m.run(print_callback=callback)
    assert ''.join(output) == snapshot('1 hello True None\n')


def make_error_callback(error: Exception) -> PrintCallback:
    """Create a print callback that raises an exception."""

    def callback(stream: Literal['stdout'], text: str) -> None:
        raise error

    return callback


def test_print_callback_raises_value_error() -> None:
    """Test that ValueError raised in callback propagates correctly."""
    m = pydantic_monty.Monty('print("hello")')
    callback = make_error_callback(ValueError('callback error'))
    with pytest.raises(pydantic_monty.MontyRuntimeError) as exc_info:
        m.run(print_callback=callback)
    inner = exc_info.value.exception()
    assert isinstance(inner, ValueError)
    assert inner.args[0] == snapshot('callback error')


def test_print_callback_raises_type_error() -> None:
    """Test that TypeError raised in callback propagates correctly."""
    m = pydantic_monty.Monty('print("hello")')
    callback = make_error_callback(TypeError('wrong type'))
    with pytest.raises(pydantic_monty.MontyRuntimeError) as exc_info:
        m.run(print_callback=callback)
    inner = exc_info.value.exception()
    assert isinstance(inner, TypeError)
    assert inner.args[0] == snapshot('wrong type')


def test_print_callback_raises_in_function() -> None:
    """Test exception from callback when print is called inside a function."""
    code = """
def greet(name):
    print(f"Hello, {name}!")

greet("World")
"""
    m = pydantic_monty.Monty(code)
    callback = make_error_callback(RuntimeError('io error'))
    with pytest.raises(pydantic_monty.MontyRuntimeError) as exc_info:
        m.run(print_callback=callback)
    inner = exc_info.value.exception()
    assert isinstance(inner, RuntimeError)
    assert inner.args[0] == snapshot('io error')


def test_print_callback_raises_in_nested_function() -> None:
    """Test exception from callback when print is called in nested functions."""
    code = """
def outer():
    def inner():
        print("from inner")
    inner()

outer()
"""
    m = pydantic_monty.Monty(code)
    callback = make_error_callback(ValueError('nested error'))
    with pytest.raises(pydantic_monty.MontyRuntimeError) as exc_info:
        m.run(print_callback=callback)
    inner = exc_info.value.exception()
    assert isinstance(inner, ValueError)
    assert inner.args[0] == snapshot('nested error')


def test_print_callback_raises_in_loop() -> None:
    """Test exception from callback when print is called in a loop."""
    code = """
for i in range(5):
    print(i)
"""
    m = pydantic_monty.Monty(code)
    call_count = 0

    def callback(stream: Literal['stdout'], text: str) -> None:
        nonlocal call_count
        call_count += 1
        if call_count >= 3:
            raise ValueError('stopped at 3')

    with pytest.raises(pydantic_monty.MontyRuntimeError) as exc_info:
        m.run(print_callback=callback)
    inner = exc_info.value.exception()
    assert isinstance(inner, ValueError)
    assert inner.args[0] == snapshot('stopped at 3')
    assert call_count == snapshot(3)


def test_map_print() -> None:
    """Test that print can be used inside map."""
    code = """
list(map(print, [1, 2, 3]))
"""
    m = pydantic_monty.Monty(code)
    output, callback = make_print_collector()
    m.run(print_callback=callback)
    assert ''.join(output) == snapshot('1\n2\n3\n')


# === structured_print_callback tests ===


StructuredCall = tuple[str, list[object], str, str]
StructuredPrintCallback = Callable[[str, list[object], str, str], None]


def make_structured_collector() -> tuple[list[StructuredCall], StructuredPrintCallback]:
    """Create a structured print callback that collects calls into a list."""
    calls: list[StructuredCall] = []

    def callback(stream: str, objects: list[object], sep: str, end: str) -> None:
        assert stream == 'stdout'
        calls.append((stream, list(objects), sep, end))

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
