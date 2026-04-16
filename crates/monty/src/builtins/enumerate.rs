//! Implementation of the enumerate() builtin function.

use smallvec::smallvec;

use crate::{
    args::ArgValues,
    bytecode::VM,
    defer_drop, defer_drop_mut,
    exception_private::{ExcType, RunResult, SimpleException},
    heap::HeapData,
    metadata::MetadataId,
    resource::ResourceTracker,
    types::{List, MontyIter, PyTrait, allocate_tuple_with_metadata},
    value::Value,
};

/// Implementation of the enumerate() builtin function.
///
/// Returns a list of `(index, value)` tuples, preserving per-element metadata from the
/// source iterable. When the loop unpacks `for i, r in enumerate(items)`, `r` retains
/// the metadata it carried inside `items` (e.g. `__non_executable` tags), because each
/// tuple's second element is stored with that element's original `MetadataId`.
///
/// Note: In Python this returns a lazy iterator, but here we eagerly collect into a list.
pub fn builtin_enumerate(vm: &mut VM<'_, '_, impl ResourceTracker>, args: ArgValues) -> RunResult<Value> {
    let container_meta = vm.pending_arg_metadata.first().copied().unwrap_or_default();
    let (iterable, start) = args.get_one_two_args("enumerate", vm.heap)?;
    let iter = MontyIter::new(iterable, vm, container_meta)?;
    defer_drop_mut!(iter, vm);
    defer_drop!(start, vm);

    // Get start index (default 0)
    let mut index: i64 = match start {
        Some(Value::Int(n)) => *n,
        Some(Value::Bool(b)) => i64::from(*b),
        Some(v) => {
            let type_name = v.py_type(vm);
            return Err(SimpleException::new_msg(
                ExcType::TypeError,
                format!("'{type_name}' object cannot be interpreted as an integer"),
            )
            .into());
        }
        None => 0,
    };

    let mut result: Vec<Value> = Vec::new();

    while let Some((item, item_meta)) = iter.for_next(vm)? {
        // Preserve the element's metadata in the tuple so that when the loop
        // unpacks `for i, r in enumerate(items)`, `r` keeps its original metadata
        // (e.g. `__non_executable`). The index is a plain integer with no special
        // metadata, so it gets the default.
        let tuple_val = allocate_tuple_with_metadata(
            smallvec![Value::Int(index), item],
            smallvec![MetadataId::DEFAULT, item_meta],
            vm.heap,
        )?;
        result.push(tuple_val);
        index += 1;
    }

    let heap_id = vm.heap.allocate(HeapData::List(List::new(result)))?;
    Ok(Value::Ref(heap_id))
}
