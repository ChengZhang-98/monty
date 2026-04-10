//! Tests for the metadata propagation types: `LabelSet`, `Metadata`, `MetadataStore`.

// These types are pub(crate), so we test through the public ObjectMetadata + MetadataStore API
// by making the metadata module visible to tests via a test helper.
// Since the types are pub(crate), we test them through the re-exported ObjectMetadata type
// and test the internal types via a helper module exposed in the crate for testing.

use std::collections::BTreeSet;

use monty::ObjectMetadata;

// === ObjectMetadata construction helpers ===

fn meta(producers: &[&str], consumers: Option<&[&str]>, tags: &[&str]) -> ObjectMetadata {
    ObjectMetadata {
        producers: producers.iter().map(ToString::to_string).collect(),
        consumers: consumers.map(|c| c.iter().map(ToString::to_string).collect()),
        tags: tags.iter().map(ToString::to_string).collect(),
    }
}

fn empty_set() -> BTreeSet<String> {
    BTreeSet::new()
}

// === ObjectMetadata default ===

#[test]
fn object_metadata_default_is_empty() {
    let m = ObjectMetadata::default();
    assert!(m.producers.is_empty());
    assert_eq!(m.consumers, None);
    assert!(m.tags.is_empty());
}

// === ObjectMetadata equality ===

#[test]
fn object_metadata_equality() {
    let a = meta(&["p1", "p2"], Some(&["c1"]), &["t1"]);
    let b = meta(&["p1", "p2"], Some(&["c1"]), &["t1"]);
    assert_eq!(a, b);
}

#[test]
fn object_metadata_inequality_producers() {
    let a = meta(&["p1"], None, &[]);
    let b = meta(&["p2"], None, &[]);
    assert_ne!(a, b);
}

#[test]
fn object_metadata_inequality_consumers() {
    let a = meta(&[], Some(&["c1"]), &[]);
    let b = meta(&[], Some(&["c2"]), &[]);
    assert_ne!(a, b);
}

#[test]
fn object_metadata_inequality_tags() {
    let a = meta(&[], None, &["t1"]);
    let b = meta(&[], None, &["t2"]);
    assert_ne!(a, b);
}

// === Serde round-trip ===

#[test]
fn object_metadata_serde_json_roundtrip() {
    let m = meta(&["p1", "p2"], Some(&["c1"]), &["t1", "t2"]);
    let json = serde_json::to_string(&m).unwrap();
    let m2: ObjectMetadata = serde_json::from_str(&json).unwrap();
    assert_eq!(m, m2);
}

#[test]
fn object_metadata_serde_json_roundtrip_universal_consumers() {
    let m = meta(&["p1"], None, &[]);
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
    let m = meta(&["source_a", "source_b"], Some(&["consumer_x"]), &["pii", "sensitive"]);
    let bytes = postcard::to_allocvec(&m).unwrap();
    let m2: ObjectMetadata = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(m, m2);
}

// === BTreeSet ordering ===

#[test]
fn object_metadata_btreeset_deterministic_order() {
    // BTreeSet ensures deterministic serialization regardless of insertion order
    let a = meta(&["z", "a", "m"], Some(&["b", "a"]), &["x", "c"]);
    let expected_producers: BTreeSet<String> = ["a", "m", "z"].iter().map(ToString::to_string).collect();
    let expected_consumers: BTreeSet<String> = ["a", "b"].iter().map(ToString::to_string).collect();
    let expected_tags: BTreeSet<String> = ["c", "x"].iter().map(ToString::to_string).collect();
    assert_eq!(a.producers, expected_producers);
    assert_eq!(a.consumers, Some(expected_consumers));
    assert_eq!(a.tags, expected_tags);
}

// === ObjectMetadata with empty consumers vs None ===

#[test]
fn object_metadata_none_consumers_vs_empty_consumers() {
    // None means universal (no restriction), empty set means no one can consume
    let universal = meta(&[], None, &[]);
    let no_one = meta(&[], Some(&[]), &[]);
    assert_ne!(universal, no_one);
    assert_eq!(universal.consumers, None);
    assert_eq!(no_one.consumers, Some(empty_set()));
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
    let input_meta = meta(&["source_a"], Some(&["consumer_x"]), &["pii"]);
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
    let meta_a = meta(&["src_a"], Some(&["c1", "c2"]), &["tag_a"]);
    let meta_b = meta(&["src_b"], Some(&["c2", "c3"]), &["tag_b"]);
    let input_a = AnnotatedObject::new(MontyObject::Int(10), Some(meta_a));
    let input_b = AnnotatedObject::new(MontyObject::Int(20), Some(meta_b));

    let (value, out_meta) = run_with_meta("a + b", vec!["a", "b"], vec![input_a, input_b]);
    assert_eq!(value, MontyObject::Int(30));

    let out = out_meta.expect("merged metadata should be present");
    // producers: union of {src_a} and {src_b}
    assert_eq!(
        out.producers,
        BTreeSet::from(["src_a".to_string(), "src_b".to_string()])
    );
    // consumers: intersection of {c1, c2} and {c2, c3} = {c2}
    assert_eq!(out.consumers, Some(BTreeSet::from(["c2".to_string()])));
    // tags: union of {tag_a} and {tag_b}
    assert_eq!(out.tags, BTreeSet::from(["tag_a".to_string(), "tag_b".to_string()]));
}

#[test]
fn metadata_propagates_through_function_call() {
    // Metadata should propagate through function arguments and returns
    let input_meta = meta(&["secret"], None, &["classified"]);
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
    let input_meta = meta(&["origin"], None, &[]);
    let input = AnnotatedObject::new(MontyObject::Int(7), Some(input_meta.clone()));
    let code = "a = x\nb = a + 1\nb";
    let (value, out_meta) = run_with_meta(code, vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::Int(8));
    assert_eq!(out_meta, Some(input_meta));
}

#[test]
fn metadata_merge_with_default_is_identity() {
    // x + 1: merge(x_meta, DEFAULT) should equal x_meta
    let input_meta = meta(&["src"], Some(&["viewer"]), &["tag"]);
    let input = AnnotatedObject::new(MontyObject::Int(10), Some(input_meta.clone()));
    let (value, out_meta) = run_with_meta("x + 1", vec!["x"], vec![input]);
    assert_eq!(value, MontyObject::Int(11));
    assert_eq!(out_meta, Some(input_meta));
}

#[test]
fn metadata_propagates_through_fstring() {
    // f-string merges metadata from all interpolated values
    let meta_a = meta(&["src_a"], None, &[]);
    let meta_b = meta(&["src_b"], None, &[]);
    let input_a = AnnotatedObject::new(MontyObject::String("hello".to_string()), Some(meta_a));
    let input_b = AnnotatedObject::new(MontyObject::String("world".to_string()), Some(meta_b));
    let code = "f'{a} {b}'";
    let (value, out_meta) = run_with_meta(code, vec!["a", "b"], vec![input_a, input_b]);
    assert_eq!(value, MontyObject::String("hello world".to_string()));
    let out = out_meta.expect("merged metadata from f-string parts");
    assert_eq!(
        out.producers,
        BTreeSet::from(["src_a".to_string(), "src_b".to_string()])
    );
}

#[test]
fn metadata_no_metadata_inputs_produce_none_output() {
    // When no inputs carry metadata, the output should have None metadata
    let (value, out_meta) = run_with_meta("1 + 2", vec![], vec![]);
    assert_eq!(value, MontyObject::Int(3));
    assert_eq!(out_meta, None);
}
