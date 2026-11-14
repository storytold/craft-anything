use craft_anything_formats::asar::{self, AsarError};

fn archive_with_offset(offset: &str) -> Vec<u8> {
    let json = format!(r#"{{"files":{{"a":{{"size":2,"offset":"{offset}"}}}}}}"#).into_bytes();
    let pad = (4 - json.len() % 4) % 4;
    let payload = json.len() + 4 + pad;
    let mut out = Vec::new();
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&((payload + 4) as u32).to_le_bytes());
    out.extend_from_slice(&(payload as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&json);
    out.extend(std::iter::repeat_n(0u8, pad));
    out.extend_from_slice(b"hi");
    out
}

#[test]
fn string_offsets_must_be_digits_only() {
    for offset in ["+2", "", " 2", "2 ", "0x2", "-1"] {
        let result = asar::parse(&archive_with_offset(offset), 10);
        assert!(matches!(result, Err(AsarError::BadJson(_))), "accepted offset {offset:?}");
    }
    for (offset, expected) in [("0", 0), ("2", 2)] {
        let archive = asar::parse(&archive_with_offset(offset), 10).unwrap();
        assert_eq!(archive.get("a").unwrap().offset, Some(expected));
    }
}
