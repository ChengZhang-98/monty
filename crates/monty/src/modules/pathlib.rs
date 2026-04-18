//! Implementation of the `pathlib` module.
//!
//! Provides a minimal implementation of Python's `pathlib` module with:
//! - `Path`: A class for filesystem path operations
//!
//! The `Path` class supports both pure methods (no I/O, handled directly) and
//! filesystem methods (require I/O, yield external function calls for host resolution).

use crate::{
    builtins::Builtins,
    bytecode::VM,
    exception_private::RunResult,
    heap::{HeapData, HeapId},
    intern::StaticStrings,
    resource::ResourceTracker,
    types::{Module, Type},
    value::Value,
};

/// Creates the `pathlib` module and allocates it on the heap.
///
/// Returns a HeapId pointing to the newly allocated module. Returns `Err` when
/// a heap allocation fails (e.g., the `max_memory` limit is exceeded while
/// populating the module's attribute dict).
///
/// # Panics
///
/// Panics if the required strings have not been pre-interned during prepare phase.
pub fn create_module(vm: &mut VM<'_, '_, impl ResourceTracker>) -> RunResult<HeapId> {
    let mut module = Module::new(StaticStrings::Pathlib);

    // pathlib.Path - the Path class (callable to create Path instances)
    module.set_attr(StaticStrings::PathClass, Value::Builtin(Builtins::Type(Type::Path)), vm)?;

    Ok(vm.heap.allocate(HeapData::Module(module))?)
}
