//! Tests for the metadata propagation types: `LabelSet`, `Metadata`, `MetadataStore`.

// These types are pub(crate), so we test through the public ObjectMetadata + MetadataStore API
// by making the metadata module visible to tests via a test helper.
// Since the types are pub(crate), we test them through the re-exported ObjectMetadata type
// and test the internal types via a helper module exposed in the crate for testing.

use std::collections::BTreeSet;

use monty::ObjectMetadata;

// === ObjectMetadata construction helpers ===

/// Builds an `ObjectMetadata` from slices. Each field uses `Option`: `None` = universal,
/// `Some(slice)` = explicit set.
fn meta(producers: Option<&[&str]>, consumers: Option<&[&str]>, tags: Option<&[&str]>) -> ObjectMetadata {
    ObjectMetadata {
        producers: producers.map(|p| p.iter().map(ToString::to_string).collect()),
        consumers: consumers.map(|c| c.iter().map(ToString::to_string).collect()),
        tags: tags.map(|t| t.iter().map(ToString::to_string).collect()),
    }
}

fn empty_set() -> BTreeSet<String> {
    BTreeSet::new()
}

// === ObjectMetadata default ===

#[test]
fn object_metadata_default_is_empty() {
    let m = ObjectMetadata::default();
    assert_eq!(m.producers, Some(BTreeSet::new()));
    assert_eq!(m.consumers, None);
    assert_eq!(m.tags, Some(BTreeSet::new()));
}

// === ObjectMetadata equality ===

#[test]
fn object_metadata_equality() {
    let a = meta(Some(&["p1", "p2"]), Some(&["c1"]), Some(&["t1"]));
    let b = meta(Some(&["p1", "p2"]), Some(&["c1"]), Some(&["t1"]));
    assert_eq!(a, b);
}

#[test]
fn object_metadata_inequality_producers() {
    let a = meta(Some(&["p1"]), None, Some(&[]));
    let b = meta(Some(&["p2"]), None, Some(&[]));
    assert_ne!(a, b);
}

#[test]
fn object_metadata_inequality_consumers() {
    let a = meta(Some(&[]), Some(&["c1"]), Some(&[]));
    let b = meta(Some(&[]), Some(&["c2"]), Some(&[]));
    assert_ne!(a, b);
}

#[test]
fn object_metadata_inequality_tags() {
    let a = meta(Some(&[]), None, Some(&["t1"]));
    let b = meta(Some(&[]), None, Some(&["t2"]));
    assert_ne!(a, b);
}

// === Serde round-trip ===

#[test]
fn object_metadata_serde_json_roundtrip() {
    let m = meta(Some(&["p1", "p2"]), Some(&["c1"]), Some(&["t1", "t2"]));
    let json = serde_json::to_string(&m).unwrap();
    let m2: ObjectMetadata = serde_json::from_str(&json).unwrap();
    assert_eq!(m, m2);
}

#[test]
fn object_metadata_serde_json_roundtrip_universal_consumers() {
    let m = meta(Some(&["p1"]), None, Some(&[]));
    let json = serde_json::to_string(&m).unwrap();
    let m2: ObjectMetadata = serde_json::from_str(&json).unwrap();
    assert_eq!(m, m2);
    assert_eq!(m2.consumers, None);
}

#[test]
fn object_metadata_serde_json_roundtrip_empty() {
    let m = ObjectMetadata::default();
    let json = serde_json::to_string(&m).unwrap();
    let m2: ObjectMetadata = serde_json::from_str(&json).unwrap();
    assert_eq!(m, m2);
}

#[test]
fn object_metadata_serde_postcard_roundtrip() {
    let m = meta(
        Some(&["source_a", "source_b"]),
        Some(&["consumer_x"]),
        Some(&["pii", "sensitive"]),
    );
    let bytes = postcard::to_allocvec(&m).unwrap();
    let m2: ObjectMetadata = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(m, m2);
}

// === BTreeSet ordering ===

#[test]
fn object_metadata_btreeset_deterministic_order() {
    // BTreeSet ensures deterministic serialization regardless of insertion order
    let a = meta(Some(&["z", "a", "m"]), Some(&["b", "a"]), Some(&["x", "c"]));
    let expected_producers: BTreeSet<String> = ["a", "m", "z"].iter().map(ToString::to_string).collect();
    let expected_consumers: BTreeSet<String> = ["a", "b"].iter().map(ToString::to_string).collect();
    let expected_tags: BTreeSet<String> = ["c", "x"].iter().map(ToString::to_string).collect();
    assert_eq!(a.producers, Some(expected_producers));
    assert_eq!(a.consumers, Some(expected_consumers));
    assert_eq!(a.tags, Some(expected_tags));
}

// === ObjectMetadata with empty set vs None (universal) ===

#[test]
fn object_metadata_none_consumers_vs_empty_consumers() {
    // None means universal (no restriction), empty set means no one can consume
    let universal = meta(Some(&[]), None, Some(&[]));
    let no_one = meta(Some(&[]), Some(&[]), Some(&[]));
    assert_ne!(universal, no_one);
    assert_eq!(universal.consumers, None);
    assert_eq!(no_one.consumers, Some(empty_set()));
}

#[test]
fn object_metadata_none_producers_vs_empty_producers() {
    // None means universal (every source), empty set means no known sources
    let universal = meta(None, None, Some(&[]));
    let empty = meta(Some(&[]), None, Some(&[]));
    assert_ne!(universal, empty);
    assert_eq!(universal.producers, None);
    assert_eq!(empty.producers, Some(BTreeSet::new()));
}

#[test]
fn object_metadata_none_tags_vs_empty_tags() {
    // None means universal (every label), empty set means no labels
    let universal = meta(Some(&[]), None, None);
    let empty = meta(Some(&[]), None, Some(&[]));
    assert_ne!(universal, empty);
    assert_eq!(universal.tags, None);
    assert_eq!(empty.tags, Some(BTreeSet::new()));
}

#[test]
fn object_metadata_serde_json_roundtrip_universal_producers() {
    let m = meta(None, Some(&["c"]), Some(&[]));
    let json = serde_json::to_string(&m).unwrap();
    let m2: ObjectMetadata = serde_json::from_str(&json).unwrap();
    assert_eq!(m, m2);
    assert_eq!(m2.producers, None);
}

#[test]
fn object_metadata_serde_json_roundtrip_universal_tags() {
    let m = meta(Some(&[]), Some(&["c"]), None);
    let json = serde_json::to_string(&m).unwrap();
    let m2: ObjectMetadata = serde_json::from_str(&json).unwrap();
    assert_eq!(m, m2);
    assert_eq!(m2.tags, None);
}

// === End-to-end metadata propagation tests (Rust core API) ===

use monty::{AnnotatedObject, MontyObject, MontyRun, NoLimitTracker, PrintWriter};

/// Helper to run code with annotated inputs and return the output's metadata.
fn run_with_meta(
    code: &str,
    input_names: Vec<&str>,
    inputs: Vec<AnnotatedObject>,
) -> (MontyObject, Option<ObjectMetadata>) {
    let names: Vec<String> = input_names.into_iter().map(ToString::to_string).collect();
    let runner = MontyRun::new(code.to_owned(), "test.py", names).unwrap();
    let progress = runner.start(inputs, NoLimitTracker, PrintWriter::Disabled).unwrap();
    let result = progress.into_complete().expect("expected Complete");
    (result.value, result.metadata)
}

#[test]
fn metadata_passthrough_single_input() {
    // An input with metadata passed straight through should retain its metadata
    let input_meta = meta(Some(&["source_a"]), Some(&["consumer_x"]), Some(&["pii"]));
    let input = AnnotatedObject::new(MontyObject::Int(42), Some(input_meta.clone()));
    let (value, out_meta) = run_with_meta("x", vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::Int(42));
    assert_eq!(out_meta, Some(input_meta));
}

#[test]
fn metadata_default_for_no_metadata_input() {
    // An input without explicit metadata should produce None metadata on output
    let input = AnnotatedObject::new(MontyObject::Int(42), None);
    let (value, out_meta) = run_with_meta("x", vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::Int(42));
    assert_eq!(out_meta, None);
}

#[test]
fn metadata_merge_on_binary_op() {
    // a + b should merge metadata: producers union, consumers intersection, tags union
    let meta_a = meta(Some(&["src_a"]), Some(&["c1", "c2"]), Some(&["tag_a"]));
    let meta_b = meta(Some(&["src_b"]), Some(&["c2", "c3"]), Some(&["tag_b"]));
    let input_a = AnnotatedObject::new(MontyObject::Int(10), Some(meta_a));
    let input_b = AnnotatedObject::new(MontyObject::Int(20), Some(meta_b));

    let (value, out_meta) = run_with_meta("a + b", vec!["a", "b"], vec![input_a, input_b]);
    assert_eq!(value, MontyObject::Int(30));

    let out = out_meta.expect("merged metadata should be present");
    // producers: union of {src_a} and {src_b}
    assert_eq!(
        out.producers,
        Some(BTreeSet::from(["src_a".to_string(), "src_b".to_string()]))
    );
    // consumers: intersection of {c1, c2} and {c2, c3} = {c2}
    assert_eq!(out.consumers, Some(BTreeSet::from(["c2".to_string()])));
    // tags: union of {tag_a} and {tag_b}
    assert_eq!(
        out.tags,
        Some(BTreeSet::from(["tag_a".to_string(), "tag_b".to_string()]))
    );
}

#[test]
fn metadata_propagates_through_function_call() {
    // Metadata should propagate through function arguments and returns
    let input_meta = meta(Some(&["secret"]), None, Some(&["classified"]));
    let input = AnnotatedObject::new(MontyObject::Int(5), Some(input_meta.clone()));
    let code = "def double(n):\n    return n * 2\ndouble(x)";
    let (value, out_meta) = run_with_meta(code, vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::Int(10));
    // n * 2: merge(secret_meta, DEFAULT) = secret_meta
    assert_eq!(out_meta, Some(input_meta));
}

#[test]
fn metadata_propagates_through_variable_assignment() {
    // a = x; b = a + 1; b should carry x's metadata
    let input_meta = meta(Some(&["origin"]), None, Some(&[]));
    let input = AnnotatedObject::new(MontyObject::Int(7), Some(input_meta.clone()));
    let code = "a = x\nb = a + 1\nb";
    let (value, out_meta) = run_with_meta(code, vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::Int(8));
    assert_eq!(out_meta, Some(input_meta));
}

#[test]
fn metadata_merge_with_default_is_identity() {
    // x + 1: merge(x_meta, DEFAULT) should equal x_meta
    let input_meta = meta(Some(&["src"]), Some(&["viewer"]), Some(&["tag"]));
    let input = AnnotatedObject::new(MontyObject::Int(10), Some(input_meta.clone()));
    let (value, out_meta) = run_with_meta("x + 1", vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::Int(11));
    assert_eq!(out_meta, Some(input_meta));
}

#[test]
fn metadata_propagates_through_fstring() {
    // f-string merges metadata from all interpolated values
    let meta_a = meta(Some(&["src_a"]), None, Some(&[]));
    let meta_b = meta(Some(&["src_b"]), None, Some(&[]));
    let input_a = AnnotatedObject::new(MontyObject::String("hello".to_string()), Some(meta_a));
    let input_b = AnnotatedObject::new(MontyObject::String("world".to_string()), Some(meta_b));
    let code = "f'{a} {b}'";
    let (value, out_meta) = run_with_meta(code, vec!["a", "b"], vec![input_a, input_b]);
    assert_eq!(value, MontyObject::String("hello world".to_string()));
    let out = out_meta.expect("merged metadata from f-string parts");
    assert_eq!(
        out.producers,
        Some(BTreeSet::from(["src_a".to_string(), "src_b".to_string()]))
    );
}

#[test]
fn metadata_no_metadata_inputs_produce_none_output() {
    // When no inputs carry metadata, the output should have None metadata
    let (value, out_meta) = run_with_meta("1 + 2", vec![], vec![]);
    assert_eq!(value, MontyObject::Int(3));
    assert_eq!(out_meta, None);
}

// === Recursive element-level metadata tests ===

#[test]
fn metadata_list_input_preserves_element_metadata() {
    // A list input where each element has distinct metadata should preserve
    // per-element metadata through a round-trip: input → interpreter → output.
    let meta_a = meta(Some(&["src_a"]), None, Some(&[]));
    let meta_b = meta(Some(&["src_b"]), Some(&["admin"]), Some(&["pii"]));
    let input_list = MontyObject::List(vec![
        AnnotatedObject::new(MontyObject::Int(1), Some(meta_a.clone())),
        AnnotatedObject::new(MontyObject::Int(2), Some(meta_b.clone())),
    ]);
    let input = AnnotatedObject::from(input_list);

    let (value, _top_meta) = run_with_meta("x", vec!["x"], vec![input]);
    if let MontyObject::List(elements) = value {
        assert_eq!(elements.len(), 2);
        assert_eq!(elements[0].value, MontyObject::Int(1));
        assert_eq!(elements[0].metadata, Some(meta_a));
        assert_eq!(elements[1].value, MontyObject::Int(2));
        assert_eq!(elements[1].metadata, Some(meta_b));
    } else {
        panic!("expected List, got {value:?}");
    }
}

#[test]
fn metadata_list_element_no_metadata_roundtrips_as_none() {
    // Elements without metadata should round-trip as None (not Some(default)).
    let input_list = MontyObject::List(vec![
        AnnotatedObject::new(MontyObject::Int(1), None),
        AnnotatedObject::new(MontyObject::Int(2), None),
    ]);
    let input = AnnotatedObject::from(input_list);

    let (value, _) = run_with_meta("x", vec!["x"], vec![input]);
    if let MontyObject::List(elements) = value {
        assert_eq!(elements[0].metadata, None);
        assert_eq!(elements[1].metadata, None);
    } else {
        panic!("expected List");
    }
}

#[test]
fn metadata_list_mixed_some_none_element_metadata() {
    // A mix of elements with and without metadata should preserve each correctly.
    let meta_a = meta(Some(&["secret"]), Some(&["admin"]), Some(&["classified"]));
    let input_list = MontyObject::List(vec![
        AnnotatedObject::new(MontyObject::Int(1), Some(meta_a.clone())),
        AnnotatedObject::new(MontyObject::Int(2), None),
        AnnotatedObject::new(MontyObject::Int(3), Some(meta_a.clone())),
    ]);
    let input = AnnotatedObject::from(input_list);

    let (value, _) = run_with_meta("x", vec!["x"], vec![input]);
    if let MontyObject::List(elements) = value {
        assert_eq!(elements.len(), 3);
        assert_eq!(elements[0].metadata, Some(meta_a.clone()));
        assert_eq!(elements[1].metadata, None);
        assert_eq!(elements[2].metadata, Some(meta_a));
    } else {
        panic!("expected List");
    }
}

#[test]
fn metadata_tuple_input_preserves_element_metadata() {
    let meta_a = meta(Some(&["src_a"]), None, Some(&[]));
    let meta_b = meta(Some(&["src_b"]), None, Some(&["tag"]));
    let input_tuple = MontyObject::Tuple(vec![
        AnnotatedObject::new(MontyObject::Int(10), Some(meta_a.clone())),
        AnnotatedObject::new(MontyObject::String("hi".to_string()), Some(meta_b.clone())),
    ]);
    let input = AnnotatedObject::from(input_tuple);

    let (value, _) = run_with_meta("x", vec!["x"], vec![input]);
    if let MontyObject::Tuple(elements) = value {
        assert_eq!(elements.len(), 2);
        assert_eq!(elements[0].value, MontyObject::Int(10));
        assert_eq!(elements[0].metadata, Some(meta_a));
        assert_eq!(elements[1].value, MontyObject::String("hi".to_string()));
        assert_eq!(elements[1].metadata, Some(meta_b));
    } else {
        panic!("expected Tuple, got {value:?}");
    }
}

#[test]
fn metadata_dict_input_preserves_key_and_value_metadata() {
    use monty::AnnotatedDictPairs;
    let meta_key = meta(Some(&["key_src"]), None, Some(&[]));
    let meta_val = meta(Some(&["val_src"]), Some(&["viewer"]), Some(&["sensitive"]));
    let input_dict = MontyObject::Dict(AnnotatedDictPairs(vec![(
        AnnotatedObject::new(MontyObject::String("k".to_string()), Some(meta_key.clone())),
        AnnotatedObject::new(MontyObject::Int(42), Some(meta_val.clone())),
    )]));
    let input = AnnotatedObject::from(input_dict);

    let (value, _) = run_with_meta("x", vec!["x"], vec![input]);
    if let MontyObject::Dict(pairs) = value {
        let entries: Vec<_> = pairs.into_iter().collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0.value, MontyObject::String("k".to_string()));
        assert_eq!(entries[0].0.metadata, Some(meta_key));
        assert_eq!(entries[0].1.value, MontyObject::Int(42));
        assert_eq!(entries[0].1.metadata, Some(meta_val));
    } else {
        panic!("expected Dict, got {value:?}");
    }
}

#[test]
fn metadata_nested_list_preserves_inner_element_metadata() {
    // A list containing a list: inner elements should preserve their metadata.
    let meta_inner = meta(Some(&["deep_source"]), None, Some(&["nested"]));
    let inner_list = MontyObject::List(vec![AnnotatedObject::new(
        MontyObject::Int(99),
        Some(meta_inner.clone()),
    )]);
    let input_list = MontyObject::List(vec![AnnotatedObject::new(inner_list, None)]);
    let input = AnnotatedObject::from(input_list);

    let (value, _) = run_with_meta("x", vec!["x"], vec![input]);
    if let MontyObject::List(outer) = value {
        assert_eq!(outer.len(), 1);
        assert_eq!(outer[0].metadata, None); // outer element has no metadata
        if let MontyObject::List(inner) = &outer[0].value {
            assert_eq!(inner.len(), 1);
            assert_eq!(inner[0].value, MontyObject::Int(99));
            assert_eq!(inner[0].metadata, Some(meta_inner));
        } else {
            panic!("expected inner List");
        }
    } else {
        panic!("expected outer List");
    }
}

#[test]
fn metadata_list_indexing_extracts_element_metadata() {
    // Extracting an element from a list via indexing should propagate that
    // element's metadata to the result, not the container's.
    let meta_elem = meta(Some(&["elem_src"]), None, Some(&["tagged"]));
    let input_list = MontyObject::List(vec![
        AnnotatedObject::new(MontyObject::Int(10), None),
        AnnotatedObject::new(MontyObject::Int(20), Some(meta_elem.clone())),
    ]);
    let input = AnnotatedObject::from(input_list);

    let (value, out_meta) = run_with_meta("x[1]", vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::Int(20));
    assert_eq!(out_meta, Some(meta_elem));
}

#[test]
fn metadata_list_append_preserves_existing_element_metadata() {
    // Appending to a list (via Python code) should not disturb existing elements' metadata.
    let meta_a = meta(Some(&["src_a"]), None, Some(&[]));
    let expected_meta = meta_a.clone();
    let input_list = MontyObject::List(vec![AnnotatedObject::new(MontyObject::Int(1), Some(meta_a))]);
    let input = AnnotatedObject::from(input_list);

    let code = "x.append(99)\nx";
    let (value, _) = run_with_meta(code, vec!["x"], vec![input]);
    if let MontyObject::List(elements) = value {
        assert_eq!(elements.len(), 2);
        // Original element keeps its metadata
        assert_eq!(elements[0].value, MontyObject::Int(1));
        assert_eq!(elements[0].metadata, Some(expected_meta));
        // Appended element has no provenance metadata (literal)
        assert_eq!(elements[1].value, MontyObject::Int(99));
        assert_eq!(elements[1].metadata, None);
    } else {
        panic!("expected List");
    }
}

#[test]
fn metadata_unpacking_preserves_element_metadata() {
    // Unpacking a list should propagate each element's metadata to its variable.
    let meta_a = meta(Some(&["src_a"]), None, Some(&[]));
    let meta_b = meta(Some(&["src_b"]), None, Some(&[]));
    let input_list = MontyObject::List(vec![
        AnnotatedObject::new(MontyObject::Int(1), Some(meta_a)),
        AnnotatedObject::new(MontyObject::Int(2), Some(meta_b.clone())),
    ]);
    let input = AnnotatedObject::from(input_list);

    // Unpack into a, b, then return b — should carry meta_b
    let (value, out_meta) = run_with_meta("a, b = x\nb", vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::Int(2));
    assert_eq!(out_meta, Some(meta_b));
}

// === Star-args metadata propagation ===

#[test]
fn metadata_star_args_propagates_to_function_params() {
    // f(*args) should propagate per-element metadata from the args tuple to parameters
    let meta_a = meta(Some(&["src_a"]), None, Some(&[]));
    let meta_b = meta(Some(&["src_b"]), None, Some(&["tag"]));
    let input_a = AnnotatedObject::new(MontyObject::Int(10), Some(meta_a));
    let input_b = AnnotatedObject::new(MontyObject::Int(20), Some(meta_b));

    // *args is constructed from a list that becomes a tuple, then unpacked
    let code = "def add(a, b):\n    return a + b\nadd(*x)";
    let input_list = MontyObject::List(vec![input_a, input_b]);
    let input = AnnotatedObject::from(input_list);

    let (value, out_meta) = run_with_meta(code, vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::Int(30));
    // result should merge meta_a + meta_b
    let out = out_meta.expect("merged metadata from *args");
    assert_eq!(
        out.producers,
        Some(BTreeSet::from(["src_a".to_string(), "src_b".to_string()]))
    );
    assert_eq!(out.tags, Some(BTreeSet::from(["tag".to_string()])));
}

// === dict merge / dict_update metadata ===

#[test]
fn metadata_dict_update_preserves_value_metadata() {
    // {**d} should preserve per-key and per-value metadata from the source dict
    use monty::AnnotatedDictPairs;
    let meta_val = meta(Some(&["api"]), None, Some(&["sensitive"]));
    let input_dict = MontyObject::Dict(AnnotatedDictPairs(vec![(
        AnnotatedObject::new(MontyObject::String("key".to_string()), None),
        AnnotatedObject::new(MontyObject::Int(42), Some(meta_val.clone())),
    )]));
    let input = AnnotatedObject::from(input_dict);

    // {**x} creates a new dict via DictUpdate; value metadata should survive
    let code = "d = {**x}\nd['key']";
    let (value, out_meta) = run_with_meta(code, vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::Int(42));
    assert_eq!(out_meta, Some(meta_val));
}

// === list_extend / set_extend metadata ===

#[test]
fn metadata_list_extend_preserves_element_metadata() {
    // [*x] should preserve per-element metadata from the source list
    let meta_a = meta(Some(&["src_a"]), None, Some(&[]));
    let meta_b = meta(Some(&["src_b"]), None, Some(&[]));
    let input_list = MontyObject::List(vec![
        AnnotatedObject::new(MontyObject::Int(1), Some(meta_a.clone())),
        AnnotatedObject::new(MontyObject::Int(2), Some(meta_b.clone())),
    ]);
    let input = AnnotatedObject::from(input_list);

    // [*x] builds via BuildList(0) then ListExtend
    let code = "y = [*x]\ny";
    let (value, _) = run_with_meta(code, vec!["x"], vec![input]);
    if let MontyObject::List(elements) = value {
        assert_eq!(elements.len(), 2);
        assert_eq!(elements[0].metadata, Some(meta_a));
        assert_eq!(elements[1].metadata, Some(meta_b));
    } else {
        panic!("expected List, got {value:?}");
    }
}

// === list_to_tuple metadata ===

#[test]
fn metadata_list_to_tuple_preserves_element_metadata() {
    // (*x,) goes through ListToTuple, element metadata should survive
    let meta_a = meta(Some(&["src"]), None, Some(&["tagged"]));
    let input_list = MontyObject::List(vec![
        AnnotatedObject::new(MontyObject::Int(1), Some(meta_a.clone())),
        AnnotatedObject::new(MontyObject::Int(2), None),
    ]);
    let input = AnnotatedObject::from(input_list);

    let code = "y = (*x,)\ny";
    let (value, _) = run_with_meta(code, vec!["x"], vec![input]);
    if let MontyObject::Tuple(elements) = value {
        assert_eq!(elements.len(), 2);
        assert_eq!(elements[0].metadata, Some(meta_a));
        assert_eq!(elements[1].metadata, None);
    } else {
        panic!("expected Tuple, got {value:?}");
    }
}

// === String unpacking metadata inheritance ===

#[test]
fn metadata_string_unpack_inherits_string_metadata() {
    // Unpacking a string should give each char the string's metadata
    let input_meta = meta(Some(&["api_response"]), None, Some(&["external"]));
    let input = AnnotatedObject::new(MontyObject::String("ab".to_string()), Some(input_meta.clone()));

    let code = "a, b = x\na";
    let (value, out_meta) = run_with_meta(code, vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::String("a".to_string()));
    assert_eq!(out_meta, Some(input_meta));
}

#[test]
fn metadata_string_star_unpack_inherits_string_metadata() {
    // first, *rest = string should give each char the string's metadata
    let input_meta = meta(Some(&["src"]), None, Some(&[]));
    let input = AnnotatedObject::new(MontyObject::String("abc".to_string()), Some(input_meta.clone()));

    let code = "first, *rest = x\nfirst";
    let (value, out_meta) = run_with_meta(code, vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::String("a".to_string()));
    assert_eq!(out_meta, Some(input_meta));
}

// === Merge with one-sided metadata ===

#[test]
fn metadata_merge_only_one_operand_has_metadata() {
    // When only one operand has metadata, result should carry that metadata
    // (merge with DEFAULT is identity)
    let input_meta = meta(Some(&["src"]), Some(&["viewer"]), Some(&["tag"]));
    let input_a = AnnotatedObject::new(MontyObject::Int(10), Some(input_meta.clone()));
    let input_b = AnnotatedObject::new(MontyObject::Int(5), None);

    let (value, out_meta) = run_with_meta("a + b", vec!["a", "b"], vec![input_a, input_b]);
    assert_eq!(value, MontyObject::Int(15));
    assert_eq!(out_meta, Some(input_meta));
}

// === Container-level metadata propagation through indexing ===

#[test]
fn metadata_container_level_propagates_through_indexing() {
    // When a list has container-level metadata (e.g. from an external function return)
    // but elements have DEFAULT per-element metadata, indexing should propagate the
    // container's metadata to the extracted element.
    let container_meta = meta(Some(&["web_api"]), Some(&["admin"]), Some(&["untrusted"]));
    let input_list = MontyObject::List(vec![
        AnnotatedObject::new(MontyObject::Int(10), None),
        AnnotatedObject::new(MontyObject::Int(20), None),
    ]);
    let input = AnnotatedObject::new(input_list, Some(container_meta.clone()));

    let (value, out_meta) = run_with_meta("x[0]", vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::Int(10));
    assert_eq!(out_meta, Some(container_meta));
}

#[test]
fn metadata_container_level_merges_with_element_metadata_on_indexing() {
    // When both the container and element have metadata, indexing should merge them:
    // producers = union, consumers = intersection, tags = union.
    let container_meta = meta(Some(&["api"]), Some(&["admin", "user"]), Some(&["external"]));
    let elem_meta = meta(Some(&["db"]), Some(&["admin"]), Some(&["pii"]));
    let input_list = MontyObject::List(vec![AnnotatedObject::new(MontyObject::Int(42), Some(elem_meta))]);
    let input = AnnotatedObject::new(input_list, Some(container_meta));

    let (value, out_meta) = run_with_meta("x[0]", vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::Int(42));
    let out = out_meta.expect("merged metadata should be present");
    assert_eq!(
        out.producers,
        Some(BTreeSet::from(["api".to_string(), "db".to_string()]))
    );
    assert_eq!(out.consumers, Some(BTreeSet::from(["admin".to_string()])));
    assert_eq!(
        out.tags,
        Some(BTreeSet::from(["external".to_string(), "pii".to_string()]))
    );
}

#[test]
fn metadata_container_level_propagates_through_negative_indexing() {
    // Negative indexing (x[-1]) should also propagate container metadata.
    let container_meta = meta(Some(&["src"]), None, Some(&["tagged"]));
    let input_list = MontyObject::List(vec![
        AnnotatedObject::new(MontyObject::Int(1), None),
        AnnotatedObject::new(MontyObject::Int(2), None),
    ]);
    let input = AnnotatedObject::new(input_list, Some(container_meta.clone()));

    let (value, out_meta) = run_with_meta("x[-1]", vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::Int(2));
    assert_eq!(out_meta, Some(container_meta));
}

// === Container-level metadata propagation through iteration ===

#[test]
fn metadata_container_level_propagates_through_for_loop() {
    // When iterating a list with container-level metadata, each yielded element
    // should carry the container's metadata.
    let container_meta = meta(Some(&["web_api"]), Some(&["admin"]), Some(&["untrusted"]));
    let input_list = MontyObject::List(vec![
        AnnotatedObject::new(MontyObject::Int(10), None),
        AnnotatedObject::new(MontyObject::Int(20), None),
    ]);
    let input = AnnotatedObject::new(input_list, Some(container_meta.clone()));

    // Sum the elements via iteration — metadata should propagate through the loop variable
    let code = "total = 0\nfor item in x:\n    total = total + item\ntotal";
    let (value, out_meta) = run_with_meta(code, vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::Int(30));
    assert_eq!(out_meta, Some(container_meta));
}

#[test]
fn metadata_container_level_propagates_through_iteration_single_element() {
    // Single-element iteration should also propagate container metadata.
    let container_meta = meta(Some(&["vault"]), Some(&["internal"]), Some(&["secret"]));
    let input_list = MontyObject::List(vec![AnnotatedObject::new(MontyObject::Int(42), None)]);
    let input = AnnotatedObject::new(input_list, Some(container_meta.clone()));

    let code = "result = None\nfor item in x:\n    result = item\nresult";
    let (value, out_meta) = run_with_meta(code, vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::Int(42));
    assert_eq!(out_meta, Some(container_meta));
}

// === Container-level metadata propagates through indexing then further operations ===

#[test]
fn metadata_container_level_propagates_through_indexing_then_concatenation() {
    // x[0] + " world" should carry x's container metadata through the binary op.
    // This tests the chain: container meta → index → binary operation.
    let container_meta = meta(Some(&["api"]), None, Some(&["non_executable"]));
    let input_list = MontyObject::List(vec![AnnotatedObject::new(
        MontyObject::String("hello".to_string()),
        None,
    )]);
    let input = AnnotatedObject::new(input_list, Some(container_meta.clone()));

    let code = "x[0] + ' world'";
    let (value, out_meta) = run_with_meta(code, vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::String("hello world".to_string()));
    assert_eq!(out_meta, Some(container_meta));
}

// === Container-level metadata propagates through f-string after indexing ===

#[test]
fn metadata_container_level_propagates_through_indexing_then_fstring() {
    // f"info: {x[0]}" should carry x's container metadata.
    let container_meta = meta(Some(&["vault"]), Some(&["admin"]), Some(&["secret"]));
    let input_list = MontyObject::List(vec![AnnotatedObject::new(
        MontyObject::String("data".to_string()),
        None,
    )]);
    let input = AnnotatedObject::new(input_list, Some(container_meta.clone()));

    let code = "f'info: {x[0]}'";
    let (value, out_meta) = run_with_meta(code, vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::String("info: data".to_string()));
    assert_eq!(out_meta, Some(container_meta));
}

// === Tuple container-level metadata ===

#[test]
fn metadata_tuple_container_level_propagates_through_indexing() {
    // Same as list test but with a tuple container.
    let container_meta = meta(Some(&["src"]), None, Some(&["tagged"]));
    let input_tuple = MontyObject::Tuple(vec![
        AnnotatedObject::new(MontyObject::Int(1), None),
        AnnotatedObject::new(MontyObject::Int(2), None),
    ]);
    let input = AnnotatedObject::new(input_tuple, Some(container_meta.clone()));

    let (value, out_meta) = run_with_meta("x[1]", vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::Int(2));
    assert_eq!(out_meta, Some(container_meta));
}
