use craft_anything_engine::{CallOptions, Caller, Engine};
use craft_anything_formats::{keyed_archive, nib, plist};
use craft_anything_testkit::apple as fx;
use serde_json::{Value, json};

fn call(engine: &Engine, path: &std::path::Path, offset: usize, limit: usize) -> Value {
    engine.call("apple_archive_decode", json!({"path": path, "offset": offset, "limit": limit}), CallOptions::new(Caller::Cli)).unwrap().to_json()
}

#[test]
fn archive_tool_pages_keep_global_counts_and_root_projection() {
    let directory = craft_anything_engine::testing::temp_dir();
    let path = directory.join("sample.archive");
    std::fs::write(&path, fx::keyed_archive()).unwrap();
    let engine = Engine::new(craft_anything_engine::testing::session());
    let full = call(&engine, &path, 0, 20_000);
    let rows = full["result"]["objects"].as_array().unwrap();
    for (offset, limit) in [(0, 1), (1, 3), (rows.len() - 1, 20_000), (rows.len(), 1), (rows.len() + 1, 1), (usize::MAX, 1)] {
        let output = call(&engine, &path, offset, limit);
        let expected: Vec<_> = rows.iter().skip(offset).take(limit).cloned().collect();
        assert_eq!(output["result"]["objects"], json!(expected));
        for field in ["objects_total", "class_counts", "root", "root_object", "tree", "top"] {
            assert_eq!(output["result"][field], full["result"][field], "{field} at offset {offset}");
        }
        assert_eq!(output["coverage"]["examined"], expected.len());
        if offset < rows.len() && offset + expected.len() < rows.len() {
            assert_eq!(output["coverage"]["next_offset"], offset + expected.len());
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_small_tool_page_keeps_shared_classes_counted_globally() {
    let class = "C".repeat(64 << 10);
    let mut objects = vec![fx::s("$null"), fx::dict(vec![("$classname", fx::s(&class))])];
    objects.extend((0..1024).map(|_| fx::dict(vec![("$class", fx::P::Uid(1))])));
    let bytes = fx::to_bplist(&fx::dict(vec![
        ("$archiver", fx::s("NSKeyedArchiver")),
        ("$objects", fx::P::Array(objects)),
        ("$top", fx::dict(vec![("root", fx::P::Uid(2))])),
    ]));
    let directory = craft_anything_engine::testing::temp_dir();
    let path = directory.join("shared.archive");
    std::fs::write(&path, bytes).unwrap();
    let engine = Engine::new(craft_anything_engine::testing::session());
    let output = call(&engine, &path, 513, 1);
    assert_eq!(output["result"]["objects_total"], 1026);
    assert_eq!(output["result"]["objects"].as_array().unwrap().len(), 1);
    assert_eq!(output["result"]["objects"][0]["index"], 513);
    assert_eq!(output["result"]["objects"][0]["class"], class);
    assert_eq!(output["result"]["class_counts"][&class], 1024);
    assert_eq!(output["coverage"]["next_offset"], 514);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn archive_and_nib_routes_keep_the_borrowed_binary_and_xml_results() {
    let directory = craft_anything_engine::testing::temp_dir();
    let engine = Engine::new(craft_anything_engine::testing::session());
    let value = fx::keyed_archive_value();
    for (name, bytes) in [("binary.nib", fx::to_bplist(&value)), ("xml.nib", fx::to_xml(&value).into_bytes())] {
        let path = directory.join(name);
        std::fs::write(&path, &bytes).unwrap();
        let parsed = plist::parse(&bytes).unwrap();
        let borrowed = keyed_archive::from_plist(&parsed).unwrap();
        let root = borrowed.root(None).unwrap();
        let archive = call(&engine, &path, 0, 20_000);
        assert_eq!(archive["result"]["tree"], borrowed.project(root.1));
        assert_eq!(archive["result"]["root"], root.0);
        assert_eq!(archive["result"]["root_object"], root.1);
        assert_eq!(archive["result"]["objects"], serde_json::to_value(borrowed.rows()).unwrap());

        let expected = nib::graph(&nib::from_keyed_archive(&borrowed), 20_000, 40_000);
        let result = engine
            .call("apple_nib_decode", json!({"path": path, "max_objects": 20_000, "max_connections": 40_000}), CallOptions::new(Caller::Cli))
            .unwrap()
            .to_json();
        assert_eq!(result["result"]["failures"], json!([]));
        assert_eq!(result["result"]["documents_total"], 1);
        let actual = &result["result"]["documents"][0];
        assert_eq!(actual["objects"], json!(expected.objects));
        assert_eq!(actual["connections"], json!(expected.connections));
        assert_eq!(actual["hierarchy"], json!(expected.hierarchy));
        assert_eq!(actual["class_counts"], json!(expected.class_counts));
        assert_eq!(actual["root_objects"], json!(expected.root_objects));
        assert_eq!(actual["coder_version"], 100_000);
    }
    std::fs::remove_dir_all(directory).unwrap();
}
