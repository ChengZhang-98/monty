//! Comparison operation helpers for the VM.

use std::cmp::Ordering;

use super::VM;
use crate::{
    defer_drop,
    exception_private::{ExcType, RunError},
    resource::ResourceTracker,
    types::{LongInt, PyTrait},
    value::Value,
};

impl<T: ResourceTracker> VM<'_, '_, T> {
    /// Equality comparison.
    pub(super) fn compare_eq(&mut self) -> Result<(), RunError> {
        let this = self;

        let (rhs, rhs_meta) = this.pop_with_meta();
        defer_drop!(rhs, this);
        let (lhs, lhs_meta) = this.pop_with_meta();
        defer_drop!(lhs, this);
        let result_meta = this.metadata_store.merge(lhs_meta, rhs_meta);

        let result = lhs.py_eq(rhs, this)?;
        this.push_with_meta(Value::Bool(result), result_meta);
        Ok(())
    }

    /// Inequality comparison.
    pub(super) fn compare_ne(&mut self) -> Result<(), RunError> {
        let this = self;

        let (rhs, rhs_meta) = this.pop_with_meta();
        defer_drop!(rhs, this);
        let (lhs, lhs_meta) = this.pop_with_meta();
        defer_drop!(lhs, this);
        let result_meta = this.metadata_store.merge(lhs_meta, rhs_meta);

        let result = !lhs.py_eq(rhs, this)?;
        this.push_with_meta(Value::Bool(result), result_meta);
        Ok(())
    }

    /// Ordering comparison with a predicate.
    pub(super) fn compare_ord<F>(&mut self, check: F) -> Result<(), RunError>
    where
        F: FnOnce(Ordering) -> bool,
    {
        let this = self;

        let (rhs, rhs_meta) = this.pop_with_meta();
        defer_drop!(rhs, this);
        let (lhs, lhs_meta) = this.pop_with_meta();
        defer_drop!(lhs, this);
        let result_meta = this.metadata_store.merge(lhs_meta, rhs_meta);

        let result = lhs.py_cmp(rhs, this)?.is_some_and(check);
        this.push_with_meta(Value::Bool(result), result_meta);
        Ok(())
    }

    /// Identity comparison (is/is not).
    ///
    /// Compares identity using `Value::is()` which compares IDs.
    ///
    /// Identity is determined by `Value::id()` which uses:
    /// - Fixed IDs for singletons (None, True, False, Ellipsis)
    /// - Interned string/bytes index for InternString/InternBytes
    /// - HeapId for heap-allocated values (Ref)
    /// - Value-based hashing for immediate types (Int, Float, Function, etc.)
    pub(super) fn compare_is(&mut self, negate: bool) {
        let this = self;

        let (rhs, rhs_meta) = this.pop_with_meta();
        defer_drop!(rhs, this);
        let (lhs, lhs_meta) = this.pop_with_meta();
        defer_drop!(lhs, this);
        let result_meta = this.metadata_store.merge(lhs_meta, rhs_meta);

        let result = lhs.is(rhs);
        this.push_with_meta(Value::Bool(if negate { !result } else { result }), result_meta);
    }

    /// Membership test (in/not in).
    pub(super) fn compare_in(&mut self, negate: bool) -> Result<(), RunError> {
        let this = self;

        let (container, container_meta) = this.pop_with_meta(); // container (rhs)
        defer_drop!(container, this);
        let (item, item_meta) = this.pop_with_meta(); // item to find (lhs)
        defer_drop!(item, this);
        let result_meta = this.metadata_store.merge(item_meta, container_meta);

        let contained = container.py_contains(item, this)?;
        this.push_with_meta(Value::Bool(if negate { !contained } else { contained }), result_meta);
        Ok(())
    }

    /// Modulo equality comparison: a % b == k
    ///
    /// This is an optimization for patterns like `x % 3 == 0`. The constant k
    /// is provided by the caller (fetched from the constant pool using the
    /// cached code reference in the run loop).
    ///
    /// Uses a fast path for Int/Float types via `py_mod_eq`, and falls back to
    /// computing `py_mod` then comparing with `py_eq` for other types (e.g., LongInt).
    pub(super) fn compare_mod_eq(&mut self, k: &Value) -> Result<(), RunError> {
        let this = self;

        let (rhs, rhs_meta) = this.pop_with_meta(); // divisor (b)
        defer_drop!(rhs, this);
        let (lhs, lhs_meta) = this.pop_with_meta(); // dividend (a)
        defer_drop!(lhs, this);
        let result_meta = this.metadata_store.merge(lhs_meta, rhs_meta);

        // Try fast path for Int/Float types
        let mod_result = match k {
            Value::Int(k_val) => lhs.py_mod_eq(rhs, *k_val),
            _ => None,
        };

        if let Some(is_equal) = mod_result {
            // Fast path succeeded
            this.push_with_meta(Value::Bool(is_equal), result_meta);
            Ok(())
        } else {
            // Fallback: compute py_mod then compare with py_eq
            // This handles LongInt and other Ref types
            let mod_value = lhs.py_mod(rhs, this);

            match mod_value {
                Ok(Some(v)) => {
                    defer_drop!(v, this);

                    // Handle InternLongInt by converting to heap LongInt for comparison
                    let k_value = if let Value::InternLongInt(id) = k {
                        let bi = this.interns.get_long_int(*id).clone();
                        LongInt::new(bi).into_value(this.heap)?
                    } else {
                        // k is from the constant pool and is always an immediate value
                        k.clone_immediate()
                    };
                    defer_drop!(k_value, this);

                    let is_equal = v.py_eq(k_value, this)?;
                    this.push_with_meta(Value::Bool(is_equal), result_meta);
                    Ok(())
                }
                Ok(None) => Err(ExcType::type_error("unsupported operand type(s) for %")),
                Err(e) => Err(e),
            }
        }
    }
}
