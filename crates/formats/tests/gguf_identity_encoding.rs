use craft_anything_formats::{FormatError, ai_model};

/// Emit integers in either byte order supported by GGUF version 3.
fn integer(bytes: &mut Vec<u8>, value: u64, width: usize, big: bool) {
    let encoded = if big { value.to_be_bytes() } else { value.to_le_bytes() };
    bytes.extend_from_slice(if big { &encoded[8 - width..] } else { &encoded[..width] });
}

/// A length-prefixed string can carry invalid bytes in a damaged model.
fn string(bytes: &mut Vec<u8>, value: &[u8], big: bool) {
    integer(bytes, value.len() as u64, 8, big);
    bytes.extend_from_slice(value);
}

/// Build complete metadata and tensor records without loading model weights.
fn model(keys: &[&[u8]], names: &[&[u8]], big: bool) -> Vec<u8> {
    let mut bytes = b"GGUF".to_vec();
    integer(&mut bytes, 3, 4, big);
    integer(&mut bytes, names.len() as u64, 8, big);
    integer(&mut bytes, keys.len() as u64, 8, big);
    for key in keys {
        string(&mut bytes, key, big);
        integer(&mut bytes, 4, 4, big);
        integer(&mut bytes, 1, 4, big);
    }
    for (i, name) in names.iter().enumerate() {
        string(&mut bytes, name, big);
        integer(&mut bytes, 1, 4, big);
        integer(&mut bytes, 4, 8, big);
        integer(&mut bytes, 0, 4, big);
        integer(&mut bytes, i as u64 * 32, 8, big);
    }
    bytes
}

/// Different invalid byte sequences cannot become the same displayed identity.
#[test]
fn invalid_metadata_keys_are_not_replacement_decoded() {
    for big in [false, true] {
        let bytes = model(&[b"valid.key", b"bad.\xff", b"bad.\xfe"], &[], big);
        let error = ai_model::parse(&bytes, bytes.len() as u64 + 4096).unwrap_err();
        assert!(matches!(error, FormatError::Malformed(_)));
        assert!(error.to_string().contains("metadata key") && error.to_string().contains("UTF-8"));
    }
}

/// An invalid tensor name must not contribute weights under a fabricated name.
#[test]
fn invalid_tensor_names_stop_before_parameter_accounting() {
    for big in [false, true] {
        let mut long = vec![b'x'; 1100];
        long.push(0xff);
        for name in [b"bad.\xff".as_slice(), b"bad.\x80", b"bad.\xc0\xaf", b"bad.\xe2\x82", &long] {
            let bytes = model(&[], &[b"valid", name], big);
            let error = ai_model::parse(&bytes, bytes.len() as u64 + 4096).unwrap_err();
            assert!(matches!(error, FormatError::Malformed(_)));
            assert!(error.to_string().contains("tensor name") && error.to_string().contains("UTF-8"));
        }
    }
}

/// Valid multibyte names, including an actual replacement character, stay intact.
#[test]
fn valid_unicode_tensor_names_are_preserved() {
    for big in [false, true] {
        let bytes = model(&[b"general.test"], &["层.weight".as_bytes(), "actual.�".as_bytes()], big);
        let report = ai_model::parse(&bytes, bytes.len() as u64 + 4096).unwrap();
        assert!(report.complete);
        assert_eq!(report.tensors.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["层.weight", "actual.�"]);
        assert_eq!(report.parameters, 8);
    }
}
