use craft_anything_formats::{keyed_archive, nib, plist};
use craft_anything_testkit::apple as fx;

#[test]
fn pages_preserve_row_contents_order_and_absolute_indices() {
    let archive = keyed_archive::parse(&fx::keyed_archive()).unwrap();
    let all = archive.rows();
    for (offset, limit) in [(0, 0), (0, 1), (1, 3), (all.len() - 1, usize::MAX), (all.len(), 3), (all.len() + 1, 3), (usize::MAX, usize::MAX)] {
        let expected: Vec<_> = all.iter().skip(offset).take(limit).collect();
        let actual = archive.rows_page(offset, limit);
        assert_eq!(serde_json::to_value(actual).unwrap(), serde_json::to_value(expected).unwrap(), "offset {offset}, limit {limit}");
    }
}

#[test]
fn class_names_borrow_the_shared_object() {
    let archive = keyed_archive::parse(&fx::keyed_archive()).unwrap();
    let root = archive.root(None).unwrap().1;
    let class_index = archive.object(root).unwrap().get("$class").and_then(keyed_archive::uid_of).unwrap();
    let stored = archive.object(class_index).unwrap().get("$classname").unwrap().as_str().unwrap();
    let name = archive.class_name(root).unwrap();
    assert!(std::ptr::eq(name.as_ptr(), stored.as_ptr()));
    assert_eq!(archive.class_of(root).as_deref(), Some(name));
    assert!(archive.class_name(u64::MAX).is_none());
}

#[test]
fn a_small_page_copies_only_its_requested_large_class_name() {
    let class = "C".repeat(64 << 10);
    let mut objects = vec![fx::s("$null"), fx::dict(vec![("$classname", fx::s(&class))])];
    objects.extend((0..1024).map(|_| fx::dict(vec![("$class", fx::P::Uid(1))])));
    let bytes = fx::to_bplist(&fx::dict(vec![("$objects", fx::P::Array(objects)), ("$top", fx::dict(vec![("root", fx::P::Uid(2))]))]));
    let archive = keyed_archive::parse(&bytes).unwrap();
    let page = archive.rows_page(513, 1);
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].index, 513);
    assert_eq!(page[0].class.as_deref(), Some(class.as_str()));
    assert_eq!(page[0].references, [1]);
    assert_eq!(page.iter().filter_map(|row| row.class.as_ref()).map(String::len).sum::<usize>(), 64 << 10);
}

#[test]
fn owned_conversion_moves_the_table_and_keeps_borrowed_results() {
    let value = fx::dict(vec![
        ("$archiver", fx::s("NSKeyedArchiver")),
        ("$version", fx::P::Int(100_000)),
        (
            "$objects",
            fx::P::Array(vec![
                fx::s("$null"),
                fx::P::Array(vec![fx::P::Uid(2), fx::P::Uid(3), fx::P::Uid(2)]),
                fx::s("shared text"),
                fx::P::Data(vec![1, 2, 3, 4]),
            ]),
        ),
        ("$top", fx::dict(vec![("root", fx::P::Uid(1)), ("payload", fx::P::Uid(3))])),
    ]);
    for bytes in [fx::to_bplist(&value), fx::to_xml(&value).into_bytes()] {
        let parsed = plist::parse(&bytes).unwrap();
        let stored = parsed.get("$objects").unwrap().as_array().unwrap();
        let table = stored.as_ptr();
        let text = stored[2].as_str().unwrap().as_ptr();
        let plist::Value::Data(payload) = &stored[3] else { panic!("missing payload") };
        let data = payload.as_ptr();
        let borrowed = keyed_archive::from_plist(&parsed).unwrap();
        assert_eq!(stored, borrowed.objects, "the borrowed API leaves the original value usable");
        let owned = keyed_archive::from_owned_plist(parsed).unwrap();
        assert_eq!(owned.objects.as_ptr(), table);
        assert_eq!(owned.objects[2].as_str().unwrap().as_ptr(), text);
        let plist::Value::Data(payload) = &owned.objects[3] else { panic!("missing payload") };
        assert_eq!(payload.as_ptr(), data);
        assert_eq!((owned.archiver.as_str(), owned.version, &owned.top), (borrowed.archiver.as_str(), borrowed.version, &borrowed.top));
        assert_eq!(owned.objects, borrowed.objects);
        for root in [None, Some("root"), Some("payload")] {
            assert_eq!(owned.root(root), borrowed.root(root));
            let index = owned.root(root).unwrap().1;
            assert_eq!(owned.project(index), borrowed.project(index));
        }
        assert_eq!(serde_json::to_value(owned.rows()).unwrap(), serde_json::to_value(borrowed.rows()).unwrap());
        assert_eq!(nib::parse(&bytes).unwrap(), nib::from_keyed_archive(&borrowed));
    }
}

#[test]
fn owned_conversion_preserves_first_fields_and_validation_order() {
    use plist::Value::{self, Array, Dict, Integer, String as Text, Uid};
    let field = |key: &str, value| (key.to_owned(), value);
    let objects = || Array(vec![Text("$null".into()), Text("first table".into())]);
    let top = || Dict(vec![field("root", Uid(1))]);
    let valid = Dict(vec![
        field("$objects", objects()),
        field("$objects", Array(vec![])),
        field("$top", top()),
        field("$top", Value::Null),
        field("$archiver", Text("first archiver".into())),
        field("$archiver", Text("second archiver".into())),
        field("$version", Integer(100_000)),
        field("$version", Integer(2)),
    ]);
    let borrowed = keyed_archive::from_plist(&valid).unwrap();
    let owned = keyed_archive::from_owned_plist(valid).unwrap();
    assert_eq!((owned.archiver, owned.version, owned.top, owned.objects), (borrowed.archiver, borrowed.version, borrowed.top, borrowed.objects));
    for invalid in [
        Value::Null,
        Dict(vec![field("$top", top())]),
        Dict(vec![field("$objects", Value::Null), field("$objects", objects()), field("$top", top())]),
        Dict(vec![field("$objects", objects())]),
        Dict(vec![field("$objects", objects()), field("$top", Value::Null), field("$top", top())]),
        Dict(vec![field("$top", Value::Null), field("$objects", Value::Null)]),
    ] {
        let borrowed = keyed_archive::from_plist(&invalid).unwrap_err();
        let owned = keyed_archive::from_owned_plist(invalid).unwrap_err();
        assert_eq!(owned.to_string(), borrowed.to_string());
    }
}
