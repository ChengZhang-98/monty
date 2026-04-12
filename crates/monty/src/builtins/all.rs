//! Implementation of the all() builtin function.

use crate::{
    args::ArgValues,
    bytecode::VM,
    defer_drop, defer_drop_mut,
    exception_private::RunResult,
    resource::ResourceTracker,
    types::{MontyIter, PyTrait},
    value::Value,
};

/// Implementation of the all() builtin function.
///
/// Returns True if all elements of the iterable are true (or if the iterable is empty).
/// Short-circuits on the first falsy value.
pub fn builtin_all(vm: &mut VM<'_, '_, impl ResourceTracker>, args: ArgValues) -> RunResult<Value> {
    let container_meta = vm.pending_arg_metadata.first().copied().unwrap_or_default();
    let iterable = args.get_one_arg("all", vm.heap)?;
    let iter = MontyIter::new(iterable, vm, container_meta)?;
    defer_drop_mut!(iter, vm);

    while let Some((item, _meta)) = iter.for_next(vm)? {
        defer_drop!(item, vm);
        if !item.py_bool(vm) {
            return Ok(Value::Bool(false));
        }
    }

    Ok(Value::Bool(true))
}
