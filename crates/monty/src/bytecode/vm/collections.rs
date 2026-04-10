//! Collection building and unpacking helpers for the VM.

use super::VM;
use crate::{
    defer_drop, defer_drop_mut,
    exception_private::{ExcType, RunError, SimpleException},
    heap::{HeapData, HeapGuard, HeapReadOutput},
    intern::StringId,
    metadata::MetadataId,
    resource::ResourceTracker,
    types::{
        Dict, List, PyTrait, Set, Slice, Type, allocate_tuple_with_metadata, slice::value_to_option_i64,
        str::allocate_char,
    },
    value::{VALUE_SIZE, Value},
};

impl<T: ResourceTracker> VM<'_, '_, T> {
    /// Builds a list from the top n stack values, carrying per-element metadata.
    pub(super) fn build_list(&mut self, count: usize) -> Result<(), RunError> {
        let (items, meta) = self.pop_n_with_meta(count);
        let list = List::new_with_metadata(items, meta);
        let heap_id = self.heap.allocate(HeapData::List(list))?;
        self.push(Value::Ref(heap_id));
        Ok(())
    }

    /// Builds a tuple from the top n stack values, carrying per-element metadata.
    ///
    /// Uses the empty tuple singleton when count is 0, and SmallVec
    /// optimization for small tuples (≤2 elements).
    pub(super) fn build_tuple(&mut self, count: usize) -> Result<(), RunError> {
        let (items, meta) = self.pop_n_with_meta(count);
        let value = allocate_tuple_with_metadata(items.into(), meta.into(), self.heap)?;
        self.push(value);
        Ok(())
    }

    /// Builds a dict from the top 2n stack values (key/value pairs), carrying per-entry metadata.
    pub(super) fn build_dict(&mut self, count: usize) -> Result<(), RunError> {
        let (items, meta) = self.pop_n_with_meta(count * 2);
        let mut dict = Dict::new();
        // Use into_iter to consume items by value, avoiding clone and proper ownership transfer
        let mut item_iter = items.into_iter();
        let mut meta_iter = meta.into_iter();
        while let (Some(key), Some(value)) = (item_iter.next(), item_iter.next()) {
            let key_meta = meta_iter.next().unwrap_or_default();
            let value_meta = meta_iter.next().unwrap_or_default();
            dict.set_with_meta(key, key_meta, value, value_meta, self)?;
        }
        let heap_id = self.heap.allocate(HeapData::Dict(dict))?;
        self.push(Value::Ref(heap_id));
        Ok(())
    }

    /// Builds a set from the top n stack values, carrying per-element metadata.
    pub(super) fn build_set(&mut self, count: usize) -> Result<(), RunError> {
        let (items, meta) = self.pop_n_with_meta(count);
        let mut set = Set::new();
        for (item, item_meta) in items.into_iter().zip(meta) {
            set.add_with_meta(item, item_meta, self)?;
        }
        let heap_id = self.heap.allocate(HeapData::Set(set))?;
        self.push(Value::Ref(heap_id));
        Ok(())
    }

    /// Builds a slice object from the top 3 stack values.
    ///
    /// Stack: [start, stop, step] -> [slice]
    /// Each value can be None (for default) or an integer.
    pub(super) fn build_slice(&mut self) -> Result<(), RunError> {
        let this = self;

        let step_val = this.pop();
        defer_drop!(step_val, this);
        let stop_val = this.pop();
        defer_drop!(stop_val, this);
        let start_val = this.pop();
        defer_drop!(start_val, this);

        let start = value_to_option_i64(start_val)?;
        let stop = value_to_option_i64(stop_val)?;
        let step = value_to_option_i64(step_val)?;

        let slice = Slice::new(start, stop, step);
        let heap_id = this.heap.allocate(HeapData::Slice(slice))?;
        this.push(Value::Ref(heap_id));
        Ok(())
    }

    /// Extends a list with items from an iterable, for PEP 448 `*expr` literal unpacking.
    ///
    /// Stack: [list, iterable] -> [list]
    /// Pops the iterable, extends the list in place, leaves list on stack.
    /// Per-element metadata is preserved from the source container.
    ///
    /// Raises `TypeError("Value after * must be an iterable, not {type}")` for non-iterables,
    /// matching CPython's message for list/tuple literal unpacking (`[*x]`, `(*x,)`).
    ///
    /// Uses `HeapGuard` for `list_ref` because it is pushed back on success,
    /// and `defer_drop!` for `iterable` because it is always dropped.
    pub(super) fn list_extend(&mut self) -> Result<(), RunError> {
        let this = self;

        let (iterable, iter_meta) = this.pop_with_meta();
        defer_drop!(iterable, this);
        // HeapGuard for list_ref: pushed back on success via into_parts, dropped on error
        let mut list_ref_guard = HeapGuard::new(this.pop(), this);
        let (list_ref, this) = list_ref_guard.as_parts();

        let (copied_items, copied_meta) = extract_items_with_meta(iterable, iter_meta, this, |type_| {
            ExcType::type_error_value_after_star(type_)
        })?;

        // Check if any copied items are refs (for updating contains_refs)
        let has_refs = copied_items.iter().any(|v| matches!(v, Value::Ref(_)));

        // Check memory limit before growing the list
        if let Value::Ref(_) = list_ref {
            this.heap.track_growth(copied_items.len() * VALUE_SIZE)?;
        }

        // Extend the list with items and their metadata
        if let Value::Ref(id) = list_ref {
            let HeapReadOutput::List(mut list) = this.heap.read(*id) else {
                panic!("list_extend: expected List on heap");
            };
            let list = list.get_mut(this.heap);
            if has_refs {
                list.set_contains_refs();
            }
            list.extend_with_meta(copied_items, copied_meta);
        }

        // Mark potential cycle after the mutable borrow ends
        if has_refs {
            this.heap.mark_potential_cycle();
        }

        // Push list_ref back on the stack (don't drop it)
        let (list_ref, this) = list_ref_guard.into_parts();
        this.push(list_ref);
        Ok(())
    }

    /// Converts a list to a tuple, preserving per-element metadata.
    ///
    /// Stack: [list] -> [tuple]
    pub(super) fn list_to_tuple(&mut self) -> Result<(), RunError> {
        let this = self;

        let list_ref = this.pop();
        defer_drop!(list_ref, this);

        let Value::Ref(id) = list_ref else {
            return Err(RunError::internal("ListToTuple: expected list ref"));
        };
        let HeapData::List(list) = this.heap.get(*id) else {
            return Err(RunError::internal("ListToTuple: expected list"));
        };
        let items = list.as_slice().iter().map(|v| v.clone_with_heap(this.heap)).collect();
        let meta = list.item_metadata_slice().to_vec();
        let value = allocate_tuple_with_metadata(items, meta.into(), this.heap)?;
        this.push(value);
        Ok(())
    }

    /// Merges a mapping into a dict for **kwargs unpacking.
    ///
    /// Stack: [dict, mapping] -> [dict]
    /// Validates that mapping is a dict and that keys are strings.
    ///
    /// Uses `defer_drop!` for `mapping` (always dropped) and `HeapGuard` for
    /// `dict_ref` (pushed back on success, dropped on error).
    pub(super) fn dict_merge(&mut self, func_name_id: u16) -> Result<(), RunError> {
        let this = self;

        let mapping = this.pop();
        defer_drop!(mapping, this);
        // HeapGuard for dict_ref: pushed back on success via into_parts, dropped on error
        let mut dict_ref_guard = HeapGuard::new(this.pop(), this);
        let (dict_ref, this) = dict_ref_guard.as_parts();

        // Get function name for error messages
        let func_name = if func_name_id == 0xFFFF {
            "<unknown>".to_string()
        } else {
            this.interns.get_str(StringId::from_index(func_name_id)).to_string()
        };

        // Check that mapping is a dict and clone key-value pairs with metadata
        let copied_items: Vec<(Value, MetadataId, Value, MetadataId)> = if let Value::Ref(id) = mapping {
            if let HeapData::Dict(dict) = this.heap.get(*id) {
                dict.entries_with_metadata()
                    .map(|(k, km, v, vm_)| (k.clone_with_heap(this), km, v.clone_with_heap(this), vm_))
                    .collect()
            } else {
                let type_name = mapping.py_type(this).to_string();
                return Err(ExcType::type_error_kwargs_not_mapping(&func_name, &type_name));
            }
        } else {
            let type_name = mapping.py_type(this).to_string();
            return Err(ExcType::type_error_kwargs_not_mapping(&func_name, &type_name));
        };

        // Merge into the dict, validating string keys
        let dict_id = if let Value::Ref(id) = dict_ref {
            *id
        } else {
            return Err(RunError::internal("DictMerge: expected dict ref"));
        };

        for (key, key_meta, value, value_meta) in copied_items {
            // Validate key is a string (InternString or heap-allocated Str)
            let is_string = match &key {
                Value::InternString(_) => true,
                Value::Ref(id) => matches!(this.heap.get(*id), HeapData::Str(_)),
                _ => false,
            };
            if !is_string {
                key.drop_with_heap(this);
                value.drop_with_heap(this);
                return Err(ExcType::type_error_kwargs_nonstring_key());
            }

            // Get the string key for error messages (needed before moving key into closure)
            let key_str = match &key {
                Value::InternString(id) => this.interns.get_str(*id).to_string(),
                Value::Ref(id) => {
                    if let HeapData::Str(s) = this.heap.get(*id) {
                        s.as_str().to_string()
                    } else {
                        "<unknown>".to_string()
                    }
                }
                _ => "<unknown>".to_string(),
            };

            let HeapReadOutput::Dict(mut dict) = this.heap.read(dict_id) else {
                unreachable!("DictMerge: entry is not a Dict")
            };

            if let Some(old_value) = dict.set_with_meta(key, key_meta, value, value_meta, this)? {
                old_value.drop_with_heap(this);
                return Err(ExcType::type_error_multiple_values(&func_name, &key_str));
            }
        }

        // Push dict_ref back on the stack (don't drop it)
        let (dict_ref, this) = dict_ref_guard.into_parts();
        this.push(dict_ref);
        Ok(())
    }

    // ========================================================================
    // PEP 448 Literal Building
    // ========================================================================

    /// Silently merges a mapping into the dict literal at `depth` on the stack.
    ///
    /// Used for `{**x, ...}` dict literals where later keys silently overwrite
    /// earlier ones (unlike [`dict_merge`] which raises `TypeError` on duplicate keys
    /// and is used for function-call `**kwargs`).
    ///
    /// Stack (depth = 0): `[..., dict, mapping]` → `[..., dict]`
    ///
    /// # Errors
    ///
    /// Returns `TypeError: '{type}' object is not a mapping` if the TOS is not a dict.
    pub(super) fn dict_update(&mut self, depth: usize) -> Result<(), RunError> {
        let this = self;

        let mapping = this.pop();
        defer_drop!(mapping, this);

        // Clone all key/value pairs with metadata out of the mapping before mutating the target
        let copied_items: Vec<(Value, MetadataId, Value, MetadataId)> = if let Value::Ref(id) = mapping {
            if let HeapData::Dict(dict) = this.heap.get(*id) {
                dict.entries_with_metadata()
                    .map(|(k, km, v, vm_)| (k.clone_with_heap(this), km, v.clone_with_heap(this), vm_))
                    .collect()
            } else {
                let type_ = mapping.py_type(this);
                return Err(ExcType::type_error_not_mapping(type_));
            }
        } else {
            let type_ = mapping.py_type(this);
            return Err(ExcType::type_error_not_mapping(type_));
        };

        // The target dict sits at `depth` positions below TOS (which is now gone after pop)
        let stack_len = this.stack.len();
        let dict_pos = stack_len - 1 - depth;
        // SAFETY: the compiler always emits BuildDict before DictUpdate, so the
        // target is always a Value::Ref.  This is a VM invariant: reaching this else
        // arm means a compiler bug.
        let Value::Ref(dict_id) = this.stack[dict_pos] else {
            unreachable!("DictUpdate: target is always a Ref — compiler invariant")
        };

        for (key, key_meta, value, value_meta) in copied_items {
            let HeapReadOutput::Dict(mut dict) = this.heap.read(dict_id) else {
                unreachable!("DictUpdate: heap entry is always a Dict — compiler invariant")
            };
            let old = dict.set_with_meta(key, key_meta, value, value_meta, this)?;
            // Silently drop any old value — PEP 448 dict literals allow duplicate keys
            if let Some(old_val) = old {
                old_val.drop_with_heap(this);
            }
        }

        Ok(())
    }

    /// Extends a set literal with all items from an iterable.
    ///
    /// Used for `{*x, ...}` set literals (PEP 448). Follows the same item-copying
    /// pattern as [`list_extend`]; raises `TypeError` for non-iterable sources.
    ///
    /// Stack (depth = 0): `[..., set, iterable]` → `[..., set]`
    ///
    /// # Errors
    ///
    /// Returns `TypeError: '{type}' object is not iterable` if TOS is not iterable.
    pub(super) fn set_extend(&mut self, depth: usize) -> Result<(), RunError> {
        let this = self;

        let (iterable, iter_meta) = this.pop_with_meta();
        defer_drop!(iterable, this);

        let (copied_items, copied_meta) = extract_items_with_meta(iterable, iter_meta, this, |type_| {
            ExcType::type_error_not_iterable(type_)
        })?;

        // The target set sits at `depth` positions below TOS (which is now gone after pop)
        let stack_len = this.stack.len();
        let set_pos = stack_len - 1 - depth;
        // SAFETY: the compiler always emits BuildSet before SetExtend, so the
        // target is always a Value::Ref.  This is a VM invariant: reaching this else
        // arm means a compiler bug.
        let Value::Ref(set_id) = this.stack[set_pos] else {
            unreachable!("SetExtend: target is always a Ref — compiler invariant")
        };

        for (item, item_meta) in copied_items.into_iter().zip(copied_meta) {
            let HeapReadOutput::Set(mut set) = this.heap.read(set_id) else {
                unreachable!("SetExtend: heap entry is always a Set — compiler invariant")
            };
            set.add_with_meta(item, item_meta, this)?;
        }

        Ok(())
    }

    // ========================================================================
    // Comprehension Building
    // ========================================================================

    /// Appends TOS to list for comprehension.
    ///
    /// Stack: [..., list, iter1, ..., iterN, value] -> [..., list, iter1, ..., iterN]
    /// The `depth` parameter is the number of iterators between the list and the value.
    /// List is at stack position: len - 2 - depth (0-indexed from bottom).
    pub(super) fn list_append(&mut self, depth: usize) -> Result<(), RunError> {
        let (value, value_meta) = self.pop_with_meta();
        let stack_len = self.stack.len();
        let list_pos = stack_len - 1 - depth;

        // Get the list reference
        let Value::Ref(list_id) = self.stack[list_pos] else {
            value.drop_with_heap(self);
            return Err(RunError::internal("ListAppend: expected list ref on stack"));
        };

        let HeapReadOutput::List(mut list) = self.heap.read(list_id) else {
            value.drop_with_heap(self);
            return Err(RunError::internal("ListAppend: expected list on heap"));
        };
        list.append_with_meta(self, value, value_meta)?;
        Ok(())
    }

    /// Adds TOS to set for comprehension.
    ///
    /// Stack: [..., set, iter1, ..., iterN, value] -> [..., set, iter1, ..., iterN]
    /// The `depth` parameter is the number of iterators between the set and the value.
    /// May raise TypeError if value is unhashable.
    pub(super) fn set_add(&mut self, depth: usize) -> Result<(), RunError> {
        let (value, value_meta) = self.pop_with_meta();
        let stack_len = self.stack.len();
        let set_pos = stack_len - 1 - depth;

        // Get the set reference
        let Value::Ref(set_id) = self.stack[set_pos] else {
            value.drop_with_heap(self);
            return Err(RunError::internal("SetAdd: expected set ref on stack"));
        };

        let HeapReadOutput::Set(mut set) = self.heap.read(set_id) else {
            value.drop_with_heap(self);
            return Err(RunError::internal("SetAdd: expected set on heap"));
        };
        set.add_with_meta(value, value_meta, self)?;

        Ok(())
    }

    /// Sets dict[key] = value for comprehension.
    ///
    /// Stack: [..., dict, iter1, ..., iterN, key, value] -> [..., dict, iter1, ..., iterN]
    /// The `depth` parameter is the number of iterators between the dict and the key-value pair.
    /// May raise TypeError if key is unhashable.
    pub(super) fn dict_set_item(&mut self, depth: usize) -> Result<(), RunError> {
        let (value, value_meta) = self.pop_with_meta();
        let (key, key_meta) = self.pop_with_meta();
        let stack_len = self.stack.len();
        let dict_pos = stack_len - 1 - depth;

        // Get the dict reference
        let Value::Ref(dict_id) = self.stack[dict_pos] else {
            key.drop_with_heap(self);
            value.drop_with_heap(self);
            return Err(RunError::internal("DictSetItem: expected dict ref on stack"));
        };

        let HeapReadOutput::Dict(mut dict) = self.heap.read(dict_id) else {
            key.drop_with_heap(self);
            value.drop_with_heap(self);
            return Err(RunError::internal("DictSetItem: expected dict on heap"));
        };
        let old_value = dict.set_with_meta(key, key_meta, value, value_meta, self)?;

        // Drop old value if key already existed
        if let Some(old) = old_value {
            old.drop_with_heap(self);
        }

        Ok(())
    }

    // ========================================================================
    // Unpacking
    // ========================================================================

    /// Unpacks a sequence into n values on the stack.
    ///
    /// Supports lists, tuples, and strings. For strings, each character becomes
    /// a separate single-character string.
    pub(super) fn unpack_sequence(&mut self, count: usize) -> Result<(), RunError> {
        let this = self;

        let (value, value_meta) = this.pop_with_meta();
        defer_drop!(value, this);

        // Copy values without incrementing refcounts (avoids borrow conflict with heap.get).
        // For strings, we allocate new string values for each character.
        // Returns (items, metadata) pairs for element-level metadata propagation.
        let items_and_meta: (Vec<Value>, Vec<MetadataId>) = match value {
            // Interned strings (string literals stored inline, not on heap)
            Value::InternString(string_id) => {
                let s = this.interns.get_str(*string_id);
                let str_len = s.chars().count();
                if str_len != count {
                    return Err(unpack_size_error(count, str_len));
                }
                let mut items = Vec::with_capacity(str_len);
                for c in s.chars() {
                    items.push(allocate_char(c, this.heap)?);
                }
                // Characters inherit the string's metadata
                let meta = vec![value_meta; items.len()];
                (items, meta)
            }
            // Heap-allocated sequences
            Value::Ref(heap_id) => match this.heap.get(*heap_id) {
                HeapData::List(list) => {
                    let list_len = list.len();
                    if list_len != count {
                        return Err(unpack_size_error(count, list_len));
                    }
                    let meta = list.item_metadata_slice().to_vec();
                    let items = list.as_slice().iter().map(|v| v.clone_with_heap(this)).collect();
                    (items, meta)
                }
                HeapData::Tuple(tuple) => {
                    let tuple_len = tuple.as_slice().len();
                    if tuple_len != count {
                        return Err(unpack_size_error(count, tuple_len));
                    }
                    let meta = tuple.item_metadata_slice().to_vec();
                    let items = tuple.as_slice().iter().map(|v| v.clone_with_heap(this)).collect();
                    (items, meta)
                }
                HeapData::Str(s) => {
                    let str_len = s.as_str().chars().count();
                    if str_len != count {
                        return Err(unpack_size_error(count, str_len));
                    }
                    let chars: Vec<char> = s.as_str().chars().collect();
                    let mut items = Vec::with_capacity(chars.len());
                    for c in chars {
                        items.push(allocate_char(c, this.heap)?);
                    }
                    // Characters inherit the string's metadata
                    let meta = vec![value_meta; items.len()];
                    (items, meta)
                }
                _ => {
                    let type_name = value.py_type(this);
                    return Err(unpack_type_error(type_name));
                }
            },
            // Non-iterable types
            _ => {
                let type_name = value.py_type(this);
                return Err(unpack_type_error(type_name));
            }
        };

        let (items, meta) = items_and_meta;
        // Push items in reverse order so first item is on top, with their element metadata
        for (item, m) in items.into_iter().zip(meta).rev() {
            this.push_with_meta(item, m);
        }
        Ok(())
    }

    /// Unpacks a sequence with a starred target.
    ///
    /// `before` is the number of targets before the star, `after` is the number after.
    /// The starred target collects all middle items into a list.
    ///
    /// For example, `first, *rest, last = [1, 2, 3, 4, 5]` has before=1, after=1.
    /// After execution, the stack has: first (top), rest_list, last.
    pub(super) fn unpack_ex(&mut self, before: usize, after: usize) -> Result<(), RunError> {
        let this = self;

        let (value, value_meta) = this.pop_with_meta();
        defer_drop_mut!(value, this);

        let min_items = before + after;

        // Extract items and their metadata from the sequence.
        // String characters inherit the string's metadata.
        let (items, meta): (Vec<Value>, Vec<MetadataId>) = match value {
            Value::InternString(string_id) => {
                let s = this.interns.get_str(*string_id);
                let chars: Vec<char> = s.chars().collect();
                if chars.len() < min_items {
                    return Err(unpack_ex_too_few_error(min_items, chars.len()));
                }
                let mut items = Vec::with_capacity(chars.len());
                for c in chars {
                    items.push(allocate_char(c, this.heap)?);
                }
                let meta = vec![value_meta; items.len()];
                (items, meta)
            }
            Value::Ref(heap_id) => match this.heap.get(*heap_id) {
                HeapData::List(list) => {
                    let list_len = list.len();
                    if list_len < min_items {
                        return Err(unpack_ex_too_few_error(min_items, list_len));
                    }
                    let meta = list.item_metadata_slice().to_vec();
                    let items = list.as_slice().iter().map(|v| v.clone_with_heap(this)).collect();
                    (items, meta)
                }
                HeapData::Tuple(tuple) => {
                    let tuple_len = tuple.as_slice().len();
                    if tuple_len < min_items {
                        return Err(unpack_ex_too_few_error(min_items, tuple_len));
                    }
                    let meta = tuple.item_metadata_slice().to_vec();
                    let items = tuple.as_slice().iter().map(|v| v.clone_with_heap(this)).collect();
                    (items, meta)
                }
                HeapData::Str(s) => {
                    let chars: Vec<char> = s.as_str().chars().collect();
                    if chars.len() < min_items {
                        return Err(unpack_ex_too_few_error(min_items, chars.len()));
                    }
                    let mut items = Vec::with_capacity(chars.len());
                    for c in chars {
                        items.push(allocate_char(c, this.heap)?);
                    }
                    let meta = vec![value_meta; items.len()];
                    (items, meta)
                }
                _ => {
                    let type_name = value.py_type(this);
                    return Err(unpack_type_error(type_name));
                }
            },
            _ => {
                let type_name = value.py_type(this);
                return Err(unpack_type_error(type_name));
            }
        };

        this.push_unpack_ex_results(items, &meta, before, after)
    }

    /// Helper to push unpacked items with starred target onto the stack.
    ///
    /// Takes items and their metadata, creates the middle list for the starred target.
    fn push_unpack_ex_results(
        &mut self,
        items: Vec<Value>,
        meta: &[MetadataId],
        before: usize,
        after: usize,
    ) -> Result<(), RunError> {
        let this = self;

        defer_drop_mut!(items, this);

        // Items get pushed onto the stack backwards, so a lot of .rev() calls

        // After items (from the end)
        let after_start = items.len() - after;
        for (item, m) in items
            .drain(after_start..)
            .zip(meta[after_start..].iter().copied())
            .rev()
        {
            this.push_with_meta(item, m);
        }

        // Middle items as a list (starred target) — each element carries its metadata
        let middle_start = before;
        let middle_meta: Vec<MetadataId> = meta[middle_start..after_start].to_vec();
        let middle_list: Vec<Value> = items.drain(middle_start..).collect();
        let list_id = this
            .heap
            .allocate(HeapData::List(List::new_with_metadata(middle_list, middle_meta)))?;
        this.push(Value::Ref(list_id));

        // Before items
        for (item, m) in items.drain(..).zip(meta[..before].iter().copied()).rev() {
            this.push_with_meta(item, m);
        }

        Ok(())
    }
}

/// Extracts items and their per-element metadata from an iterable value.
///
/// Container elements carry their stored metadata. String characters and dict keys
/// inherit the iterable's own stack metadata (`iter_meta`), since they don't have
/// independent provenance.
///
/// The `make_error` closure produces the appropriate `TypeError` for non-iterable types.
fn extract_items_with_meta<T: ResourceTracker>(
    iterable: &Value,
    iter_meta: MetadataId,
    vm: &mut VM<'_, '_, T>,
    make_error: impl FnOnce(Type) -> RunError,
) -> Result<(Vec<Value>, Vec<MetadataId>), RunError> {
    match iterable {
        Value::Ref(id) => match vm.heap.get(*id) {
            HeapData::List(list) => {
                let meta = list.item_metadata_slice().to_vec();
                let items = list.as_slice().iter().map(|v| v.clone_with_heap(vm)).collect();
                Ok((items, meta))
            }
            HeapData::Tuple(tuple) => {
                let meta = tuple.item_metadata_slice().to_vec();
                let items = tuple.as_slice().iter().map(|v| v.clone_with_heap(vm)).collect();
                Ok((items, meta))
            }
            HeapData::Set(set) => {
                let (items, meta): (Vec<_>, Vec<_>) = set
                    .entries_with_metadata()
                    .map(|(v, m)| (v.clone_with_heap(vm), m))
                    .unzip();
                Ok((items, meta))
            }
            HeapData::Dict(dict) => {
                // Dict iteration yields keys; each key carries its key_meta
                let (items, meta): (Vec<_>, Vec<_>) = dict
                    .entries_with_metadata()
                    .map(|(k, km, _, _)| (k.clone_with_heap(vm), km))
                    .unzip();
                Ok((items, meta))
            }
            HeapData::Str(s) => {
                let chars: Vec<char> = s.as_str().chars().collect();
                let mut items = Vec::with_capacity(chars.len());
                for c in chars {
                    items.push(allocate_char(c, vm.heap)?);
                }
                // Characters inherit the string's metadata
                let meta = vec![iter_meta; items.len()];
                Ok((items, meta))
            }
            _ => {
                let type_ = iterable.py_type(vm);
                Err(make_error(type_))
            }
        },
        Value::InternString(id) => {
            let s = vm.interns.get_str(*id);
            let chars: Vec<char> = s.chars().collect();
            let mut items = Vec::with_capacity(chars.len());
            for c in chars {
                items.push(allocate_char(c, vm.heap)?);
            }
            // Characters inherit the string's metadata
            let meta = vec![iter_meta; items.len()];
            Ok((items, meta))
        }
        _ => {
            let type_ = iterable.py_type(vm);
            Err(make_error(type_))
        }
    }
}

/// Creates the ValueError for star unpacking when there are too few values.
fn unpack_ex_too_few_error(min_needed: usize, actual: usize) -> RunError {
    let message = format!("not enough values to unpack (expected at least {min_needed}, got {actual})");
    SimpleException::new_msg(ExcType::ValueError, message).into()
}

/// Creates the appropriate ValueError for unpacking size mismatches.
///
/// Python uses different messages depending on whether there are too few or too many values:
/// - Too few: "not enough values to unpack (expected X, got Y)"
/// - Too many: "too many values to unpack (expected X, got Y)"
fn unpack_size_error(expected: usize, actual: usize) -> RunError {
    let message = if actual < expected {
        format!("not enough values to unpack (expected {expected}, got {actual})")
    } else {
        format!("too many values to unpack (expected {expected}, got {actual})")
    };
    SimpleException::new_msg(ExcType::ValueError, message).into()
}

/// Creates a TypeError for attempting to unpack a non-iterable type.
fn unpack_type_error(type_name: Type) -> RunError {
    SimpleException::new_msg(
        ExcType::TypeError,
        format!("cannot unpack non-iterable {type_name} object"),
    )
    .into()
}
