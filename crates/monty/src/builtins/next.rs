//! Implementation of the next() builtin function.

use crate::{
    args::ArgValues, bytecode::VM, defer_drop, exception_private::RunResult, resource::ResourceTracker,
    types::iter::iterator_next, value::Value,
};

/// Implementation of the next() builtin function.
///
/// Retrieves the next item from an iterator.
///
/// Two forms are supported:
/// - `next(iterator)` - Returns the next item from the iterator. Raises
///   `StopIteration` when the iterator is exhausted.
/// - `next(iterator, default)` - Returns the next item from the iterator, or
///   `default` if the iterator is exhausted.
///
/// Propagates element metadata from the iterator to the return value via
/// `vm.pending_result_metadata`, so the dispatch code pushes it with the correct metadata.
pub fn builtin_next(vm: &mut VM<'_, '_, impl ResourceTracker>, args: ArgValues) -> RunResult<Value> {
    let (iterator, default) = args.get_one_two_args("next", vm.heap)?;
    defer_drop!(iterator, vm);
    let (value, meta) = iterator_next(iterator, default, vm)?;
    vm.pending_result_metadata = meta;
    Ok(value)
}
