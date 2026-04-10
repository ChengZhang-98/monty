//! Tests for JSON serialization and deserialization of `MontyObject`.
//!
//! `MontyObject` uses derived serde with externally tagged enum format.
//! This means each variant is wrapped in an object with the variant name as key.

use monty::{AnnotatedObject, ExcType, MontyObject, MontyRun};

// === JSON Serialization Tests ===

#[test]
fn json_output_primitives() {
    // Primitives are wrapped in their variant names
    assert_eq!(serde_json::to_string(&MontyObject::Int(42)).unwrap(), r#"{"Int":42}"#);
    assert_eq!(
        serde_json::to_string(&MontyObject::Float(1.5)).unwrap(),
        r#"{"Float":1.5}"#
    );
    assert_eq!(
        serde_json::to_string(&MontyObject::String("hi".into())).unwrap(),
        r#"{"String":"hi"}"#
    );
    assert_eq!(
        serde_json::to_string(&MontyObject::Bool(true)).unwrap(),
        r#"{"Bool":true}"#
    );
    assert_eq!(serde_json::to_string(&MontyObject::None).unwrap(), r#""None""#);
}

#[test]
fn json_output_list() {
    let ex = MontyRun::new("[1, 'two', 3.0]".to_owned(), "test.py", vec![]).unwrap();
    let result = ex.run_no_limits(Vec::<monty::MontyObject>::new()).unwrap();
    // Container elements are now AnnotatedObject with {value, metadata} wrapping
    assert_eq!(
        serde_json::to_string(&result).unwrap(),
        r#"{"List":[{"value":{"Int":1},"metadata":null},{"value":{"String":"two"},"metadata":null},{"value":{"Float":3.0},"metadata":null}]}"#
    );
}

#[test]
fn json_output_dict() {
    let ex = MontyRun::new("{'a': 1, 'b': 2}".to_owned(), "test.py", vec![]).unwrap();
    let result = ex.run_no_limits(Vec::<monty::MontyObject>::new()).unwrap();
    assert_eq!(
        serde_json::to_string(&result).unwrap(),
        r#"{"Dict":[[{"value":{"String":"a"},"metadata":null},{"value":{"Int":1},"metadata":null}],[{"value":{"String":"b"},"metadata":null},{"value":{"Int":2},"metadata":null}]]}"#
    );
}

#[test]
fn json_output_tuple() {
    let ex = MontyRun::new("(1, 'two')".to_owned(), "test.py", vec![]).unwrap();
    let result = ex.run_no_limits(Vec::<monty::MontyObject>::new()).unwrap();
    assert_eq!(
        serde_json::to_string(&result).unwrap(),
        r#"{"Tuple":[{"value":{"Int":1},"metadata":null},{"value":{"String":"two"},"metadata":null}]}"#
    );
}

#[test]
fn json_output_bytes() {
    let ex = MontyRun::new("b'hi'".to_owned(), "test.py", vec![]).unwrap();
    let result = ex.run_no_limits(Vec::<monty::MontyObject>::new()).unwrap();
    assert_eq!(serde_json::to_string(&result).unwrap(), r#"{"Bytes":[104,105]}"#);
}

#[test]
fn json_output_ellipsis() {
    let ex = MontyRun::new("...".to_owned(), "test.py", vec![]).unwrap();
    let result = ex.run_no_limits(Vec::<monty::MontyObject>::new()).unwrap();
    assert_eq!(serde_json::to_string(&result).unwrap(), r#""Ellipsis""#);
}

#[test]
fn json_output_exception() {
    let obj = MontyObject::Exception {
        exc_type: ExcType::ValueError,
        arg: Some("test".to_string()),
    };
    assert_eq!(
        serde_json::to_string(&obj).unwrap(),
        r#"{"Exception":{"exc_type":"ValueError","arg":"test"}}"#
    );
}

#[test]
fn json_output_repr() {
    let obj = MontyObject::Repr {
        type_name: "function".to_string(),
        repr: "<function foo>".to_string(),
    };
    assert_eq!(
        serde_json::to_string(&obj).unwrap(),
        r#"{"Repr":{"type_name":"function","repr":"<function foo>"}}"#
    );
}

#[test]
fn json_output_cycle_list() {
    // Test JSON serialization of cyclic list
    let ex = MontyRun::new("a = []; a.append(a); a".to_owned(), "test.py", vec![]).unwrap();
    let result = ex.run_no_limits(Vec::<monty::MontyObject>::new()).unwrap();
    // The cyclic reference becomes MontyObject::Cycle, wrapped in AnnotatedObject
    assert_eq!(
        serde_json::to_string(&result).unwrap(),
        r#"{"List":[{"value":{"Cycle":[1,"[...]"]},"metadata":null}]}"#
    );
}

#[test]
fn json_output_cycle_dict() {
    // Test JSON serialization of cyclic dict
    let ex = MontyRun::new("d = {}; d['self'] = d; d".to_owned(), "test.py", vec![]).unwrap();
    let result = ex.run_no_limits(Vec::<monty::MontyObject>::new()).unwrap();
    assert_eq!(
        serde_json::to_string(&result).unwrap(),
        r#"{"Dict":[[{"value":{"String":"self"},"metadata":null},{"value":{"Cycle":[1,"{...}"]},"metadata":null}]]}"#
    );
}

// === JSON Deserialization Tests ===

#[test]
fn json_deserialize_primitives() {
    // Deserialize tagged format
    let int: MontyObject = serde_json::from_str(r#"{"Int":42}"#).unwrap();
    let float: MontyObject = serde_json::from_str(r#"{"Float":2.5}"#).unwrap();
    let string: MontyObject = serde_json::from_str(r#"{"String":"hello"}"#).unwrap();
    let bool_val: MontyObject = serde_json::from_str(r#"{"Bool":true}"#).unwrap();
    let null: MontyObject = serde_json::from_str(r#""None""#).unwrap();

    assert_eq!(int, MontyObject::Int(42));
    assert_eq!(float, MontyObject::Float(2.5));
    assert_eq!(string, MontyObject::String("hello".to_string()));
    assert_eq!(bool_val, MontyObject::Bool(true));
    assert_eq!(null, MontyObject::None);
}

#[test]
fn json_deserialize_list() {
    let list: MontyObject = serde_json::from_str(
        r#"{"List":[{"value":{"Int":1},"metadata":null},{"value":{"String":"two"},"metadata":null},{"value":{"Float":3.0},"metadata":null}]}"#,
    )
    .unwrap();
    assert_eq!(
        list,
        MontyObject::List(vec![
            MontyObject::Int(1).into(),
            MontyObject::String("two".to_string()).into(),
            MontyObject::Float(3.0).into()
        ])
    );
}

#[test]
fn json_deserialize_dict() {
    let dict: MontyObject = serde_json::from_str(
        r#"{"Dict":[[{"value":{"String":"a"},"metadata":null},{"value":{"Int":1},"metadata":null}],[{"value":{"String":"b"},"metadata":null},{"value":{"Int":2},"metadata":null}]]}"#,
    )
    .unwrap();
    if let MontyObject::Dict(pairs) = dict {
        let pairs_vec: Vec<_> = pairs.into_iter().collect();
        assert_eq!(pairs_vec.len(), 2);
        assert_eq!(
            pairs_vec[0],
            (
                AnnotatedObject::from(MontyObject::String("a".to_string())),
                AnnotatedObject::from(MontyObject::Int(1))
            )
        );
        assert_eq!(
            pairs_vec[1],
            (
                AnnotatedObject::from(MontyObject::String("b".to_string())),
                AnnotatedObject::from(MontyObject::Int(2))
            )
        );
    } else {
        panic!("expected Dict");
    }
}

// === Round-trip Tests ===

#[test]
fn json_roundtrip() {
    // Values round-trip through JSON correctly
    let ex = MontyRun::new(
        "{'items': [1, 'two', None], 'flag': True}".to_owned(),
        "test.py",
        vec![],
    )
    .unwrap();
    let result = ex.run_no_limits(Vec::<monty::MontyObject>::new()).unwrap();
    let json = serde_json::to_string(&result).unwrap();
    let parsed: MontyObject = serde_json::from_str(&json).unwrap();
    assert_eq!(result, parsed);
}

#[test]
fn json_roundtrip_empty() {
    // Empty structures round-trip correctly
    let list: MontyObject = serde_json::from_str(r#"{"List":[]}"#).unwrap();
    let dict: MontyObject = serde_json::from_str(r#"{"Dict":[]}"#).unwrap();
    assert_eq!(serde_json::to_string(&list).unwrap(), r#"{"List":[]}"#);
    assert_eq!(serde_json::to_string(&dict).unwrap(), r#"{"Dict":[]}"#);
}

// === Cycle Equality Tests ===

#[test]
fn cycle_equality_same_id() {
    // Multiple references to the same cyclic object should produce equal Cycle values
    let ex = MontyRun::new("a = []; a.append(a); [a, a]".to_owned(), "test.py", vec![]).unwrap();
    let result = ex.run_no_limits(Vec::<monty::MontyObject>::new()).unwrap();

    if let MontyObject::List(outer) = &result {
        assert_eq!(outer.len(), 2, "outer list should have 2 elements");

        if let (MontyObject::List(inner1), MontyObject::List(inner2)) = (&outer[0].value, &outer[1].value) {
            assert_eq!(inner1.len(), 1);
            assert_eq!(inner2.len(), 1);
            assert_eq!(inner1[0], inner2[0], "cycles referencing same object should be equal");
            assert!(matches!(&inner1[0].value, MontyObject::Cycle(..)));
        } else {
            panic!("expected inner lists");
        }
    } else {
        panic!("expected outer list");
    }
}

#[test]
fn cycle_equality_different_ids() {
    // Two separate cyclic objects should produce unequal Cycle values
    let ex = MontyRun::new(
        "a = []; a.append(a); b = []; b.append(b); [a, b]".to_owned(),
        "test.py",
        vec![],
    )
    .unwrap();
    let result = ex.run_no_limits(Vec::<monty::MontyObject>::new()).unwrap();

    if let MontyObject::List(outer) = &result {
        assert_eq!(outer.len(), 2, "outer list should have 2 elements");

        if let (MontyObject::List(inner1), MontyObject::List(inner2)) = (&outer[0].value, &outer[1].value) {
            assert_eq!(inner1.len(), 1);
            assert_eq!(inner2.len(), 1);
            assert_ne!(
                inner1[0], inner2[0],
                "cycles referencing different objects should not be equal"
            );

            if let (MontyObject::Cycle(id1, ph1), MontyObject::Cycle(id2, ph2)) = (&inner1[0].value, &inner2[0].value) {
                assert_ne!(id1, id2, "heap IDs should differ");
                assert_eq!(ph1, ph2, "placeholders should match (both are lists)");
                assert_eq!(*ph1, "[...]");
            } else {
                panic!("expected Cycle variants");
            }
        } else {
            panic!("expected inner lists");
        }
    } else {
        panic!("expected outer list");
    }
}
