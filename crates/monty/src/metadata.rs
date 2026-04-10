// TODO(Phase 2): remove this allow once opcodes use LabelSet/Metadata merge operations
#![allow(dead_code)]
//! Metadata propagation for data provenance tracking.
//!
//! Every Python value in Monty can carry metadata with three fields:
//! - **producers**: the set of data sources that contributed to this value (union propagation)
//! - **consumers**: the set of authorized consumers for this value (intersection propagation)
//! - **tags**: arbitrary classification labels on this value (union propagation)
//!
//! When two values combine (e.g. `a + b`), their metadata is merged:
//! - `result.producers = a.producers | b.producers` (union)
//! - `result.consumers = a.consumers & b.consumers` (intersection)
//! - `result.tags = a.tags | b.tags` (union)
//!
//! # Design decisions
//!
//! **Element-level tracking**: containers (list, dict, tuple, set) track metadata per-element,
//! not aggregated at the container level. This avoids false tainting — accessing `lst[0]` returns
//! only element 0's metadata, not metadata merged from all elements. A future enhancement could
//! add hybrid tracking (container-level + element-level) if coarse-grained queries are needed.
//!
//! **Explicit data flow only**: metadata propagates through value operations (arithmetic, string
//! concatenation, function arguments/returns, etc.) but NOT through control flow. An `if` branch
//! condition does not taint values assigned within the branch. This simplifies the implementation
//! and covers the vast majority of practical provenance use cases. A future enhancement could add
//! implicit flow tracking via a "program counter label" if stricter information flow control is needed.
//!
//! # VM integration
//!
//! The VM carries a **parallel metadata stack** (`meta_stack: Vec<MetadataId>`) alongside
//! the operand stack (`stack: Vec<Value>`). Every `push()`/`pop()` mirrors both stacks.
//! Similarly, `meta_globals` parallels `globals`, and `meta_exception_stack` parallels
//! `exception_stack`. The [`MetadataStore`] is owned by the VM and serialized in
//! [`VMSnapshot`](crate::bytecode::VMSnapshot) for pause/resume support.
//!
//! Async tasks also carry their own `meta_stack` and `meta_exception_stack` in the
//! [`Task`](crate::bytecode::vm::scheduler::Task) struct, swapped on context switch.
//!
//! # Interning
//!
//! Label strings are interned as [`LabelId`]s and metadata structs are deduplicated in a
//! [`MetadataStore`]. This keeps the per-value cost to a single `u32` ([`MetadataId`]) and makes
//! merge operations fast through short-circuit checks on the default metadata.
//!
//! # Public API
//!
//! [`ObjectMetadata`] is the public-facing type using plain `BTreeSet<String>` fields.
//! It is converted to/from internal [`MetadataId`] via [`MetadataStore::intern_object_metadata`]
//! and [`MetadataStore::to_object_metadata`]. `None` consumers means universal (no restriction).
//!
//! See `docs/extensions/implemented/metadata-propagation.md` for the full design document.

use std::{cmp::Ordering, collections::BTreeSet};

use ahash::AHashMap;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

// ---------------------------------------------------------------------------
// LabelId — interned label string
// ---------------------------------------------------------------------------

/// An interned identifier for a metadata label string.
///
/// Label strings (e.g. `"user_ssn"`, `"hr_dept"`) are interned once in the
/// [`MetadataStore`] and referenced by this cheap `Copy` handle everywhere else.
/// `LabelId(0)` is reserved for the `"*"` wildcard that represents the universal set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct LabelId(u32);

impl LabelId {
    /// The wildcard label representing the universal set.
    pub const WILDCARD: Self = Self(0);
}

// ---------------------------------------------------------------------------
// LabelSet — compact set of LabelIds with universal-set support
// ---------------------------------------------------------------------------

/// A set of [`LabelId`]s that can also represent the universal set.
///
/// The universal set (`is_universal = true`) has special algebra:
/// - `union(universal, s) = universal`
/// - `intersection(universal, s) = s`
///
/// When not universal, labels are stored sorted and deduplicated in a [`SmallVec`]
/// that inlines up to 2 elements to avoid heap allocation for the common case of
/// 0-2 labels per field.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct LabelSet {
    /// When `true`, this set contains every possible label (the `"*"` wildcard).
    /// The `labels` vec is empty in this case.
    is_universal: bool,
    /// Sorted, deduplicated label IDs. Empty when `is_universal` is true.
    labels: SmallVec<[LabelId; 2]>,
}

impl LabelSet {
    /// Creates an empty label set (no labels, not universal).
    pub fn empty() -> Self {
        Self {
            is_universal: false,
            labels: SmallVec::new(),
        }
    }

    /// Creates the universal set that contains every label.
    pub fn universal() -> Self {
        Self {
            is_universal: true,
            labels: SmallVec::new(),
        }
    }

    /// Creates a label set from a sorted, deduplicated iterator of [`LabelId`]s.
    ///
    /// The caller must ensure that `ids` yields labels in ascending order with no duplicates.
    /// This is an internal constructor — public callers should use [`MetadataStore::intern`]
    /// which handles sorting and dedup.
    fn from_sorted(ids: SmallVec<[LabelId; 2]>) -> Self {
        Self {
            is_universal: false,
            labels: ids,
        }
    }

    /// Returns `true` if this is the universal set.
    pub fn is_universal(&self) -> bool {
        self.is_universal
    }

    /// Returns `true` if this set contains no labels and is not universal.
    pub fn is_empty(&self) -> bool {
        !self.is_universal && self.labels.is_empty()
    }

    /// Returns the labels in this set, or `None` if this is the universal set.
    pub fn labels(&self) -> Option<&[LabelId]> {
        if self.is_universal { None } else { Some(&self.labels) }
    }

    /// Computes the union of two label sets.
    ///
    /// If either set is universal, the result is universal.
    /// Otherwise, the result is the sorted merge of both label vecs.
    pub fn union(&self, other: &Self) -> Self {
        if self.is_universal || other.is_universal {
            return Self::universal();
        }
        Self::from_sorted(sorted_union(&self.labels, &other.labels))
    }

    /// Computes the intersection of two label sets.
    ///
    /// - Both universal -> universal
    /// - One universal -> the other set (cloned)
    /// - Otherwise -> sorted intersection of both label vecs
    pub fn intersection(&self, other: &Self) -> Self {
        if self.is_universal && other.is_universal {
            return Self::universal();
        }
        if self.is_universal {
            return other.clone();
        }
        if other.is_universal {
            return self.clone();
        }
        Self::from_sorted(sorted_intersection(&self.labels, &other.labels))
    }
}

/// Sorted merge (union) of two sorted slices, producing a deduplicated sorted vec.
fn sorted_union(a: &[LabelId], b: &[LabelId]) -> SmallVec<[LabelId; 2]> {
    let mut result = SmallVec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            Ordering::Less => {
                result.push(a[i]);
                i += 1;
            }
            Ordering::Greater => {
                result.push(b[j]);
                j += 1;
            }
            Ordering::Equal => {
                result.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    result.extend_from_slice(&a[i..]);
    result.extend_from_slice(&b[j..]);
    result
}

/// Sorted intersection of two sorted slices.
fn sorted_intersection(a: &[LabelId], b: &[LabelId]) -> SmallVec<[LabelId; 2]> {
    let mut result = SmallVec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            Ordering::Less => i += 1,
            Ordering::Greater => j += 1,
            Ordering::Equal => {
                result.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Metadata — the three-field provenance record
// ---------------------------------------------------------------------------

/// Provenance metadata attached to a Python value.
///
/// Each field uses set algebra during propagation:
/// - `producers` accumulates via **union** — "this value was derived from all these sources"
/// - `consumers` restricts via **intersection** — "only these consumers may see this value"
/// - `tags` accumulates via **union** — "this value carries all these classification labels"
///
/// Metadata instances are interned in the [`MetadataStore`] and referenced by [`MetadataId`].
/// They are immutable after creation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct Metadata {
    /// Data sources that contributed to this value.
    pub producers: LabelSet,
    /// Authorized consumers of this value. Universal by default (no restriction).
    pub consumers: LabelSet,
    /// Classification labels on this value.
    pub tags: LabelSet,
}

impl Metadata {
    /// Creates the default metadata: empty producers, universal consumers, empty tags.
    ///
    /// This represents a value with no provenance restrictions — it came from nowhere
    /// specific, anyone can see it, and it has no labels.
    pub fn default_metadata() -> Self {
        Self {
            producers: LabelSet::empty(),
            consumers: LabelSet::universal(),
            tags: LabelSet::empty(),
        }
    }

    /// Merges two metadata records according to the propagation rules:
    /// - producers: union
    /// - consumers: intersection
    /// - tags: union
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            producers: self.producers.union(&other.producers),
            consumers: self.consumers.intersection(&other.consumers),
            tags: self.tags.union(&other.tags),
        }
    }
}

// ---------------------------------------------------------------------------
// MetadataId — cheap handle into the MetadataStore
// ---------------------------------------------------------------------------

/// A handle referencing interned [`Metadata`] in the [`MetadataStore`].
///
/// This is the per-value cost of metadata tracking: a single `u32` stored alongside
/// each value on the stack and in containers. `MetadataId(0)` always points to the
/// default metadata (empty producers, universal consumers, empty tags).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct MetadataId(u32);

impl MetadataId {
    /// The default metadata ID (index 0 in the store).
    ///
    /// Points to metadata with empty producers, universal consumers, empty tags.
    pub const DEFAULT: Self = Self(0);

    /// Returns `true` if this is the default metadata.
    pub fn is_default(self) -> bool {
        self.0 == 0
    }
}

impl Default for MetadataId {
    fn default() -> Self {
        Self::DEFAULT
    }
}

// ---------------------------------------------------------------------------
// ObjectMetadata — public API boundary type
// ---------------------------------------------------------------------------

/// Metadata representation used at the public API boundary.
///
/// Unlike the internal [`Metadata`] which uses interned [`LabelId`]s, this type
/// uses plain strings so that callers don't need to interact with the interning system.
///
/// The `consumers` field uses `None` to represent the universal set (no restrictions).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectMetadata {
    /// Data sources that contributed to this value.
    pub producers: BTreeSet<String>,
    /// Authorized consumers. `None` means universal (no restrictions, the default).
    pub consumers: Option<BTreeSet<String>>,
    /// Classification labels.
    pub tags: BTreeSet<String>,
}

// ---------------------------------------------------------------------------
// AnnotatedObject — MontyObject + optional metadata for API boundaries
// ---------------------------------------------------------------------------

/// A [`MontyObject`](crate::MontyObject) paired with optional metadata.
///
/// Used at API boundaries (inputs, outputs, function call args/returns) to carry
/// provenance metadata alongside values. When metadata is `None`, the value has
/// default provenance (no producers, universal consumers, no tags).
///
/// Implements `From<MontyObject>` for easy conversion when metadata is not needed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnnotatedObject {
    /// The Python value.
    pub value: crate::MontyObject,
    /// Optional provenance metadata. `None` means default (no restrictions).
    pub metadata: Option<ObjectMetadata>,
}

impl AnnotatedObject {
    /// Creates a new annotated object with the given metadata.
    #[must_use]
    pub fn new(value: crate::MontyObject, metadata: Option<ObjectMetadata>) -> Self {
        Self { value, metadata }
    }
}

impl From<crate::MontyObject> for AnnotatedObject {
    fn from(value: crate::MontyObject) -> Self {
        Self { value, metadata: None }
    }
}

// ---------------------------------------------------------------------------
// MetadataStore — interning, dedup, and merge
// ---------------------------------------------------------------------------

/// Central store for interning label strings and deduplicating metadata records.
///
/// The store guarantees that:
/// - Each unique label string is stored once and referenced by [`LabelId`]
/// - Each unique [`Metadata`] combination is stored once and referenced by [`MetadataId`]
/// - Index 0 always holds the default metadata and the `"*"` wildcard label
///
/// The store grows monotonically during execution — entries are never removed.
/// This is acceptable because the number of unique metadata combinations is typically
/// very small (bounded by the product of unique label sets, usually < 100).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MetadataStore {
    /// `LabelId(i)` -> label string. Index 0 is always `"*"`.
    label_strings: Vec<String>,
    /// Reverse map: label string -> `LabelId`.
    label_map: AHashMap<String, LabelId>,
    /// `MetadataId(i)` -> interned metadata. Index 0 is always the default.
    entries: Vec<Metadata>,
    /// Reverse map: metadata -> `MetadataId` for dedup.
    dedup_map: AHashMap<Metadata, MetadataId>,
}

impl Default for MetadataStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MetadataStore {
    /// Creates a new store with the default metadata and wildcard label pre-registered.
    pub fn new() -> Self {
        let wildcard = "*".to_string();
        let default_meta = Metadata::default_metadata();

        let mut label_map = AHashMap::new();
        label_map.insert(wildcard.clone(), LabelId::WILDCARD);

        let mut dedup_map = AHashMap::new();
        dedup_map.insert(default_meta.clone(), MetadataId::DEFAULT);

        Self {
            label_strings: vec![wildcard],
            label_map,
            entries: vec![default_meta],
            dedup_map,
        }
    }

    /// Interns a label string, returning its [`LabelId`].
    ///
    /// If the string was already interned, returns the existing ID.
    pub fn intern_label(&mut self, name: &str) -> LabelId {
        if let Some(&id) = self.label_map.get(name) {
            return id;
        }
        let id = LabelId(u32::try_from(self.label_strings.len()).expect("label count overflow"));
        self.label_strings.push(name.to_string());
        self.label_map.insert(name.to_string(), id);
        id
    }

    /// Returns the string for a [`LabelId`].
    pub fn label_name(&self, id: LabelId) -> &str {
        &self.label_strings[id.0 as usize]
    }

    /// Interns a [`Metadata`] record, returning its [`MetadataId`].
    ///
    /// If an identical record was already interned, returns the existing ID (dedup).
    pub fn intern(&mut self, metadata: Metadata) -> MetadataId {
        if let Some(&id) = self.dedup_map.get(&metadata) {
            return id;
        }
        let id = MetadataId(u32::try_from(self.entries.len()).expect("metadata count overflow"));
        self.dedup_map.insert(metadata.clone(), id);
        self.entries.push(metadata);
        id
    }

    /// Returns the [`Metadata`] for a [`MetadataId`].
    pub fn get(&self, id: MetadataId) -> &Metadata {
        &self.entries[id.0 as usize]
    }

    /// Merges two metadata records and returns the interned result.
    ///
    /// Short-circuits for common cases:
    /// - `merge(DEFAULT, DEFAULT) -> DEFAULT` (O(1))
    /// - `merge(a, DEFAULT) -> a` (O(1), since default has empty producers/tags and universal consumers)
    /// - `merge(DEFAULT, b) -> b` (O(1))
    pub fn merge(&mut self, a: MetadataId, b: MetadataId) -> MetadataId {
        if a.is_default() {
            return b;
        }
        if b.is_default() {
            return a;
        }
        let merged = self.entries[a.0 as usize].merge(&self.entries[b.0 as usize]);
        self.intern(merged)
    }

    /// Converts an [`ObjectMetadata`] (public API type) into a [`MetadataId`] by
    /// interning all label strings and the resulting metadata record.
    pub fn intern_object_metadata(&mut self, obj_meta: &ObjectMetadata) -> MetadataId {
        let producers = self.intern_label_set_from_strings(&obj_meta.producers, false);
        let consumers = match &obj_meta.consumers {
            None => LabelSet::universal(),
            Some(set) => self.intern_label_set_from_strings(set, false),
        };
        let tags = self.intern_label_set_from_strings(&obj_meta.tags, false);

        self.intern(Metadata {
            producers,
            consumers,
            tags,
        })
    }

    /// Converts a [`MetadataId`] back to an [`ObjectMetadata`] (public API type)
    /// by resolving all label IDs to their strings.
    ///
    /// Returns `None` for the default metadata (no provenance information).
    pub fn to_object_metadata(&self, id: MetadataId) -> Option<ObjectMetadata> {
        if id.is_default() {
            return None;
        }
        let meta = self.get(id);
        Some(ObjectMetadata {
            producers: self.label_set_to_strings(&meta.producers),
            consumers: if meta.consumers.is_universal() {
                None
            } else {
                Some(self.label_set_to_strings(&meta.consumers))
            },
            tags: self.label_set_to_strings(&meta.tags),
        })
    }

    /// Interns a set of string labels into a [`LabelSet`].
    fn intern_label_set_from_strings(&mut self, strings: &BTreeSet<String>, universal: bool) -> LabelSet {
        if universal {
            return LabelSet::universal();
        }
        let mut ids: SmallVec<[LabelId; 2]> = strings.iter().map(|s| self.intern_label(s)).collect();
        ids.sort_unstable();
        ids.dedup();
        LabelSet {
            is_universal: false,
            labels: ids,
        }
    }

    /// Resolves a [`LabelSet`] to a set of strings.
    fn label_set_to_strings(&self, set: &LabelSet) -> BTreeSet<String> {
        if let Some(ids) = set.labels() {
            ids.iter().map(|&id| self.label_name(id).to_string()).collect()
        } else {
            BTreeSet::from(["*".to_string()])
        }
    }
}

// Tests for internal types that can't be accessed from integration tests.
#[cfg(test)]
mod tests {
    use super::*;

    // === LabelSet algebra ===

    #[test]
    fn label_set_empty_is_empty() {
        let s = LabelSet::empty();
        assert!(s.is_empty());
        assert!(!s.is_universal());
        assert_eq!(s.labels(), Some(&[] as &[LabelId]));
    }

    #[test]
    fn label_set_universal_is_universal() {
        let s = LabelSet::universal();
        assert!(s.is_universal());
        assert!(!s.is_empty());
        assert_eq!(s.labels(), None);
    }

    #[test]
    fn label_set_union_both_empty() {
        let result = LabelSet::empty().union(&LabelSet::empty());
        assert!(result.is_empty());
    }

    #[test]
    fn label_set_union_with_universal() {
        let a = LabelSet::from_sorted(SmallVec::from_slice(&[LabelId(1), LabelId(2)]));
        assert!(a.union(&LabelSet::universal()).is_universal());
        assert!(LabelSet::universal().union(&a).is_universal());
    }

    #[test]
    fn label_set_union_disjoint() {
        let a = LabelSet::from_sorted(SmallVec::from_slice(&[LabelId(1), LabelId(3)]));
        let b = LabelSet::from_sorted(SmallVec::from_slice(&[LabelId(2), LabelId(4)]));
        let result = a.union(&b);
        assert_eq!(
            result.labels().unwrap(),
            &[LabelId(1), LabelId(2), LabelId(3), LabelId(4)]
        );
    }

    #[test]
    fn label_set_union_overlapping() {
        let a = LabelSet::from_sorted(SmallVec::from_slice(&[LabelId(1), LabelId(2), LabelId(3)]));
        let b = LabelSet::from_sorted(SmallVec::from_slice(&[LabelId(2), LabelId(3), LabelId(4)]));
        let result = a.union(&b);
        assert_eq!(
            result.labels().unwrap(),
            &[LabelId(1), LabelId(2), LabelId(3), LabelId(4)]
        );
    }

    #[test]
    fn label_set_intersection_both_universal() {
        let result = LabelSet::universal().intersection(&LabelSet::universal());
        assert!(result.is_universal());
    }

    #[test]
    fn label_set_intersection_one_universal() {
        let a = LabelSet::from_sorted(SmallVec::from_slice(&[LabelId(1), LabelId(2)]));
        let result = a.intersection(&LabelSet::universal());
        assert_eq!(result.labels().unwrap(), &[LabelId(1), LabelId(2)]);

        let result2 = LabelSet::universal().intersection(&a);
        assert_eq!(result2.labels().unwrap(), &[LabelId(1), LabelId(2)]);
    }

    #[test]
    fn label_set_intersection_disjoint() {
        let a = LabelSet::from_sorted(SmallVec::from_slice(&[LabelId(1), LabelId(3)]));
        let b = LabelSet::from_sorted(SmallVec::from_slice(&[LabelId(2), LabelId(4)]));
        let result = a.intersection(&b);
        assert!(result.is_empty());
    }

    #[test]
    fn label_set_intersection_overlapping() {
        let a = LabelSet::from_sorted(SmallVec::from_slice(&[LabelId(1), LabelId(2), LabelId(3)]));
        let b = LabelSet::from_sorted(SmallVec::from_slice(&[LabelId(2), LabelId(3), LabelId(4)]));
        let result = a.intersection(&b);
        assert_eq!(result.labels().unwrap(), &[LabelId(2), LabelId(3)]);
    }

    #[test]
    fn label_set_intersection_with_empty() {
        let a = LabelSet::from_sorted(SmallVec::from_slice(&[LabelId(1), LabelId(2)]));
        let result = a.intersection(&LabelSet::empty());
        assert!(result.is_empty());
    }

    // === MetadataStore interning ===

    #[test]
    fn store_new_has_default_at_index_0() {
        let store = MetadataStore::new();
        let meta = store.get(MetadataId::DEFAULT);
        assert!(meta.producers.is_empty());
        assert!(meta.consumers.is_universal());
        assert!(meta.tags.is_empty());
    }

    #[test]
    fn store_wildcard_label_at_index_0() {
        let store = MetadataStore::new();
        assert_eq!(store.label_name(LabelId::WILDCARD), "*");
    }

    #[test]
    fn store_intern_label_returns_same_id() {
        let mut store = MetadataStore::new();
        let id1 = store.intern_label("foo");
        let id2 = store.intern_label("foo");
        assert_eq!(id1, id2);
    }

    #[test]
    fn store_intern_label_different_strings() {
        let mut store = MetadataStore::new();
        let id1 = store.intern_label("foo");
        let id2 = store.intern_label("bar");
        assert_ne!(id1, id2);
        assert_eq!(store.label_name(id1), "foo");
        assert_eq!(store.label_name(id2), "bar");
    }

    #[test]
    fn store_intern_metadata_dedup() {
        let mut store = MetadataStore::new();
        let p = store.intern_label("p1");
        let meta1 = Metadata {
            producers: LabelSet::from_sorted(SmallVec::from_slice(&[p])),
            consumers: LabelSet::universal(),
            tags: LabelSet::empty(),
        };
        let meta2 = meta1.clone();
        let id1 = store.intern(meta1);
        let id2 = store.intern(meta2);
        assert_eq!(id1, id2);
    }

    #[test]
    fn store_intern_default_returns_default_id() {
        let mut store = MetadataStore::new();
        let id = store.intern(Metadata::default_metadata());
        assert_eq!(id, MetadataId::DEFAULT);
    }

    // === MetadataStore merge ===

    #[test]
    fn store_merge_default_default() {
        let mut store = MetadataStore::new();
        let result = store.merge(MetadataId::DEFAULT, MetadataId::DEFAULT);
        assert_eq!(result, MetadataId::DEFAULT);
    }

    #[test]
    fn store_merge_a_default_returns_a() {
        let mut store = MetadataStore::new();
        let p = store.intern_label("p1");
        let meta = Metadata {
            producers: LabelSet::from_sorted(SmallVec::from_slice(&[p])),
            consumers: LabelSet::universal(),
            tags: LabelSet::empty(),
        };
        let a = store.intern(meta);
        let result = store.merge(a, MetadataId::DEFAULT);
        assert_eq!(result, a);
    }

    #[test]
    fn store_merge_default_b_returns_b() {
        let mut store = MetadataStore::new();
        let t = store.intern_label("tag1");
        let meta = Metadata {
            producers: LabelSet::empty(),
            consumers: LabelSet::universal(),
            tags: LabelSet::from_sorted(SmallVec::from_slice(&[t])),
        };
        let b = store.intern(meta);
        let result = store.merge(MetadataId::DEFAULT, b);
        assert_eq!(result, b);
    }

    #[test]
    fn store_merge_two_non_default() {
        let mut store = MetadataStore::new();
        let p1 = store.intern_label("p1");
        let p2 = store.intern_label("p2");
        let c1 = store.intern_label("c1");
        let c2 = store.intern_label("c2");
        let t1 = store.intern_label("t1");
        let t2 = store.intern_label("t2");

        let meta_a = Metadata {
            producers: LabelSet::from_sorted(SmallVec::from_slice(&[p1])),
            consumers: LabelSet::from_sorted(SmallVec::from_slice(&[c1, c2])),
            tags: LabelSet::from_sorted(SmallVec::from_slice(&[t1])),
        };
        let meta_b = Metadata {
            producers: LabelSet::from_sorted(SmallVec::from_slice(&[p2])),
            consumers: LabelSet::from_sorted(SmallVec::from_slice(&[c2])),
            tags: LabelSet::from_sorted(SmallVec::from_slice(&[t2])),
        };

        let a = store.intern(meta_a);
        let b = store.intern(meta_b);
        let result_id = store.merge(a, b);
        let result = store.get(result_id);

        // producers: union of {p1} and {p2} = {p1, p2}
        assert_eq!(result.producers.labels().unwrap(), &[p1, p2]);
        // consumers: intersection of {c1, c2} and {c2} = {c2}
        assert_eq!(result.consumers.labels().unwrap(), &[c2]);
        // tags: union of {t1} and {t2} = {t1, t2}
        assert_eq!(result.tags.labels().unwrap(), &[t1, t2]);
    }

    #[test]
    fn store_merge_is_commutative() {
        let mut store = MetadataStore::new();
        let p1 = store.intern_label("p1");
        let p2 = store.intern_label("p2");

        let meta_a = Metadata {
            producers: LabelSet::from_sorted(SmallVec::from_slice(&[p1])),
            consumers: LabelSet::universal(),
            tags: LabelSet::empty(),
        };
        let meta_b = Metadata {
            producers: LabelSet::from_sorted(SmallVec::from_slice(&[p2])),
            consumers: LabelSet::universal(),
            tags: LabelSet::empty(),
        };

        let a = store.intern(meta_a);
        let b = store.intern(meta_b);
        let ab = store.merge(a, b);
        let ba = store.merge(b, a);
        assert_eq!(ab, ba);
    }

    #[test]
    fn store_merge_is_associative() {
        let mut store = MetadataStore::new();
        let p1 = store.intern_label("p1");
        let p2 = store.intern_label("p2");
        let p3 = store.intern_label("p3");

        let meta_a = Metadata {
            producers: LabelSet::from_sorted(SmallVec::from_slice(&[p1])),
            consumers: LabelSet::universal(),
            tags: LabelSet::empty(),
        };
        let meta_b = Metadata {
            producers: LabelSet::from_sorted(SmallVec::from_slice(&[p2])),
            consumers: LabelSet::universal(),
            tags: LabelSet::empty(),
        };
        let meta_c = Metadata {
            producers: LabelSet::from_sorted(SmallVec::from_slice(&[p3])),
            consumers: LabelSet::universal(),
            tags: LabelSet::empty(),
        };

        let a = store.intern(meta_a);
        let b = store.intern(meta_b);
        let c = store.intern(meta_c);

        let ab_c = {
            let ab = store.merge(a, b);
            store.merge(ab, c)
        };
        let a_bc = {
            let bc = store.merge(b, c);
            store.merge(a, bc)
        };
        assert_eq!(ab_c, a_bc);
    }

    #[test]
    fn store_merge_consumers_intersection_restricts() {
        let mut store = MetadataStore::new();
        let c1 = store.intern_label("c1");
        let c2 = store.intern_label("c2");

        // a: consumers = universal (anyone can see)
        let meta_a = Metadata {
            producers: LabelSet::empty(),
            consumers: LabelSet::universal(),
            tags: LabelSet::empty(),
        };
        // b: consumers = {c1, c2} (restricted)
        let meta_b = Metadata {
            producers: LabelSet::empty(),
            consumers: LabelSet::from_sorted(SmallVec::from_slice(&[c1, c2])),
            tags: LabelSet::empty(),
        };

        let a = store.intern(meta_a);
        let b = store.intern(meta_b);
        let result = store.merge(a, b);
        let result_meta = store.get(result);

        // intersection of universal and {c1, c2} = {c1, c2}
        assert_eq!(result_meta.consumers.labels().unwrap(), &[c1, c2]);
    }

    // === ObjectMetadata round-trip through MetadataStore ===

    #[test]
    fn store_object_metadata_roundtrip() {
        let mut store = MetadataStore::new();
        let obj = ObjectMetadata {
            producers: BTreeSet::from(["source_a".to_string(), "source_b".to_string()]),
            consumers: Some(BTreeSet::from(["consumer_x".to_string()])),
            tags: BTreeSet::from(["pii".to_string()]),
        };
        let id = store.intern_object_metadata(&obj);
        let roundtripped = store.to_object_metadata(id).unwrap();
        assert_eq!(obj, roundtripped);
    }

    #[test]
    fn store_object_metadata_universal_consumers_roundtrip() {
        let mut store = MetadataStore::new();
        let obj = ObjectMetadata {
            producers: BTreeSet::from(["src".to_string()]),
            consumers: None, // universal
            tags: BTreeSet::new(),
        };
        let id = store.intern_object_metadata(&obj);
        let roundtripped = store.to_object_metadata(id).unwrap();
        assert_eq!(obj, roundtripped);
    }

    #[test]
    fn store_default_metadata_returns_none() {
        let store = MetadataStore::new();
        assert_eq!(store.to_object_metadata(MetadataId::DEFAULT), None);
    }

    // === MetadataStore serde round-trip ===

    #[test]
    fn store_serde_roundtrip() {
        let mut store = MetadataStore::new();
        let p = store.intern_label("producer_1");
        let c = store.intern_label("consumer_1");
        let meta = Metadata {
            producers: LabelSet::from_sorted(SmallVec::from_slice(&[p])),
            consumers: LabelSet::from_sorted(SmallVec::from_slice(&[c])),
            tags: LabelSet::empty(),
        };
        let id = store.intern(meta);

        let bytes = postcard::to_allocvec(&store).unwrap();
        let store2: MetadataStore = postcard::from_bytes(&bytes).unwrap();

        // Verify the round-tripped store has the same data
        assert_eq!(store2.label_name(p), "producer_1");
        assert_eq!(store2.label_name(c), "consumer_1");
        let meta2 = store2.get(id);
        assert_eq!(meta2.producers.labels().unwrap(), &[p]);
        assert_eq!(meta2.consumers.labels().unwrap(), &[c]);
        assert!(meta2.tags.is_empty());
    }

    // === Metadata merge algebra ===

    #[test]
    fn metadata_merge_default_is_identity() {
        let p1 = LabelId(1);
        let c1 = LabelId(2);
        let t1 = LabelId(3);
        let meta = Metadata {
            producers: LabelSet::from_sorted(SmallVec::from_slice(&[p1])),
            consumers: LabelSet::from_sorted(SmallVec::from_slice(&[c1])),
            tags: LabelSet::from_sorted(SmallVec::from_slice(&[t1])),
        };
        let default = Metadata::default_metadata();

        let result = meta.merge(&default);
        // producers: union with empty = same
        assert_eq!(result.producers.labels().unwrap(), &[p1]);
        // consumers: intersection with universal = same
        assert_eq!(result.consumers.labels().unwrap(), &[c1]);
        // tags: union with empty = same
        assert_eq!(result.tags.labels().unwrap(), &[t1]);
    }
}
