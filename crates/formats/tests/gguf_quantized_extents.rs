use craft_anything_formats::ai_model;

fn tensor_header(kind: u32, shape: &[u64], offset: u64) -> Vec<u8> {
    tensor_header_with_order(kind, shape, offset, false)
}

fn tensor_header_with_order(kind: u32, shape: &[u64], offset: u64, big_endian: bool) -> Vec<u8> {
    let u32_bytes = |value: u32| if big_endian { value.to_be_bytes() } else { value.to_le_bytes() };
    let u64_bytes = |value: u64| if big_endian { value.to_be_bytes() } else { value.to_le_bytes() };
    let mut bytes = b"GGUF".to_vec();
    bytes.extend_from_slice(&u32_bytes(3));
    bytes.extend_from_slice(&u64_bytes(1));
    bytes.extend_from_slice(&u64_bytes(0));
    bytes.extend_from_slice(&u64_bytes(6));
    bytes.extend_from_slice(b"weight");
    bytes.extend_from_slice(&u32_bytes(shape.len() as u32));
    for dim in shape {
        bytes.extend_from_slice(&u64_bytes(*dim));
    }
    bytes.extend_from_slice(&u32_bytes(kind));
    bytes.extend_from_slice(&u64_bytes(offset));
    bytes
}

#[test]
fn a_quantized_tensor_has_a_checked_byte_extent() {
    // Two rows of two Q4_K blocks, each storing 256 weights in 144 bytes.
    let header = tensor_header(12, &[512, 2], 0);
    let start = (header.len() as u64).next_multiple_of(32);
    let exact = ai_model::parse(&header, start + 576).unwrap();
    assert_eq!(exact.tensors[0].data_bytes, Some(576));
    assert_eq!(exact.tensors_past_end, 0);
    assert!(exact.warnings.is_empty(), "{:?}", exact.warnings);
    let cut = ai_model::parse(&header, start + 575).unwrap();
    assert_eq!(cut.tensors_past_end, 1);
    assert!(cut.warnings.iter().any(|w| w.contains("more data")));
}

#[test]
fn every_supported_quantized_layout_counts_its_scale_and_packed_values() {
    // On-disk block geometries, independent of the number of displayable tensors.
    let cases = [
        (2, 32, 18),
        (3, 32, 20),
        (6, 32, 22),
        (7, 32, 24),
        (8, 32, 34),
        (9, 32, 36),
        (10, 256, 84),
        (11, 256, 110),
        (12, 256, 144),
        (13, 256, 176),
        (14, 256, 210),
        (15, 256, 292),
        (16, 256, 66),
        (17, 256, 74),
        (18, 256, 98),
        (19, 256, 50),
        (20, 32, 18),
        (21, 256, 110),
        (22, 256, 82),
        (23, 256, 136),
        (29, 256, 56),
    ];
    for (kind, block, bytes) in cases {
        let header = tensor_header(kind, &[block * 2, 3], 32);
        let start = (header.len() as u64).next_multiple_of(32);
        let report = ai_model::parse(&header, start + 32 + bytes * 6).unwrap();
        assert_eq!(report.tensors[0].data_bytes, Some(bytes * 6), "kind {kind}");
        assert_eq!(report.tensors_past_end, 0, "kind {kind}");
        assert!(report.warnings.is_empty(), "kind {kind}: {:?}", report.warnings);
    }
}

#[test]
fn row_width_must_fit_blocks_even_if_the_total_element_count_does() {
    let header = tensor_header(12, &[128, 2], 0);
    let report = ai_model::parse(&header, 4096).unwrap();
    assert_eq!(report.tensors[0].data_bytes, None);
    assert!(report.warnings.iter().any(|w| w.contains("row") && w.contains("256")));
}

#[test]
fn invalid_or_unrepresentable_extents_never_become_plausible_sizes() {
    let huge = tensor_header(12, &[u64::MAX - 255, 2], 0);
    let report = ai_model::parse(&huge, u64::MAX).unwrap();
    assert_eq!(report.tensors[0].data_bytes, None);
    assert!(report.warnings.iter().any(|w| w.contains("shape")));

    let unknown = tensor_header(999, &[256], 0);
    assert_eq!(ai_model::parse(&unknown, 4096).unwrap().tensors[0].data_bytes, None);
    let plain = tensor_header(0, &[3, 2], 0);
    assert_eq!(ai_model::parse(&plain, 4096).unwrap().tensors[0].data_bytes, Some(24));
    let overflowing_offset = tensor_header(12, &[256], u64::MAX);
    let report = ai_model::parse(&overflowing_offset, u64::MAX).unwrap();
    assert_eq!(report.tensors_past_end, 1);
    assert!(report.warnings.iter().any(|w| w.contains("more data")));
}

#[derive(serde::Deserialize)]
struct OfficialFixture {
    tensor_type: String,
    type_id: u32,
    file_type: String,
    quantized: bool,
    shape: Vec<u64>,
    data_offset: usize,
    data_bytes: u64,
    hex: String,
}

#[derive(serde::Deserialize)]
struct OfficialCorpus {
    inputs: FixtureInputs,
    fixtures: Vec<OfficialFixture>,
}

#[derive(serde::Deserialize)]
struct FixtureInputs {
    rows: u64,
    blocks_per_row: u64,
}

fn official_corpus() -> OfficialCorpus {
    serde_json::from_str(include_str!("fixtures/gguf_current/fixtures.json")).unwrap()
}

fn fixture_bytes(fixture: &OfficialFixture) -> Vec<u8> {
    fixture.hex.as_bytes().as_chunks::<2>().0.iter().map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect()
}

// Reconstruct a big-endian header around the official fixture's unchanged byte payload.
// These variants are made by this test helper, not by the official writer.
fn big_endian_fixture(fixture: &OfficialFixture) -> Vec<u8> {
    let mut bytes = tensor_header_with_order(fixture.type_id, &fixture.shape, 0, true);
    bytes.resize(bytes.len().next_multiple_of(32), 0);
    let original = fixture_bytes(fixture);
    bytes.extend_from_slice(&original[fixture.data_offset..fixture.data_offset + fixture.data_bytes as usize]);
    bytes
}

// Complete files and measured payload sizes from the pinned official writers;
// the generator also verifies the files and their truncations with GGUFReader.
#[test]
fn official_weight_files_keep_their_identity_and_detect_short_payloads() {
    let corpus = official_corpus();
    for fixture in corpus.fixtures {
        let bytes = fixture_bytes(&fixture);
        let report = ai_model::parse(&bytes, bytes.len() as u64).unwrap();
        let tensor = &report.tensors[0];
        assert_eq!(tensor.dtype, fixture.tensor_type);
        assert_eq!(tensor.shape, fixture.shape);
        assert_eq!(tensor.data_bytes, Some(fixture.data_bytes), "{}", fixture.tensor_type);
        assert_eq!(report.quantization.as_deref(), fixture.quantized.then_some(fixture.tensor_type.as_str()));
        let file_type = format!("mostly {}", fixture.file_type.strip_prefix("MOSTLY_").unwrap().replace("_MOE", " MoE"));
        assert_eq!(report.file_type.as_deref(), Some(file_type.as_str()));
        assert!(report.warnings.is_empty(), "{}: {:?}", fixture.tensor_type, report.warnings);

        // The official writer pads to its alignment. Cut the tensor, not just that padding.
        let data_end = fixture.data_offset + fixture.data_bytes as usize;
        let exact = ai_model::parse(&bytes[..data_end], data_end as u64).unwrap();
        assert_eq!(exact.tensors_past_end, 0);
        let cut = ai_model::parse(&bytes[..data_end - 1], data_end as u64 - 1).unwrap();
        assert_eq!(cut.tensors_past_end, 1, "{}", fixture.tensor_type);
        assert!(cut.warnings.iter().any(|warning| warning.contains("more data")));
    }
}

#[test]
fn reconstructed_headers_preserve_byte_order_and_extent_checks() {
    let corpus = official_corpus();
    for fixture in &corpus.fixtures {
        let big = big_endian_fixture(fixture);
        let report = ai_model::parse(&big, big.len() as u64).unwrap();
        assert_eq!(report.tensors[0].dtype, fixture.tensor_type);
        assert_eq!(report.tensors[0].shape, fixture.shape);
        assert_eq!(report.tensors[0].data_bytes, Some(fixture.data_bytes));
        assert_eq!(report.tensors_past_end, 0);
        assert!(report.complete && report.warnings.is_empty(), "{}: {:?}", fixture.tensor_type, report.warnings);

        // Infer block width from the official corpus's input dimensions, not the parser's table.
        let block = fixture.shape[0] / corpus.inputs.blocks_per_row;
        if block > 1 {
            let half_rows = tensor_header(fixture.type_id, &[block / 2, 2], 0);
            let report = ai_model::parse(&half_rows, 4096).unwrap();
            assert_eq!(report.tensors[0].data_bytes, None, "{}", fixture.tensor_type);
            assert!(report.warnings.iter().any(|warning| warning.contains("row width")), "{}", fixture.tensor_type);
        }
        let overflow = tensor_header_with_order(fixture.type_id, &[u64::MAX - (block - 1), 2], 0, true);
        let report = ai_model::parse(&overflow, u64::MAX).unwrap();
        assert_eq!(report.tensors[0].data_bytes, None, "{}", fixture.tensor_type);
        assert!(report.warnings.iter().any(|warning| warning.contains("shape")), "{}", fixture.tensor_type);
    }

    // Removed storage IDs and an unassigned ID keep their unknown-type behavior.
    for kind in [31, 32, 33, 36, 37, 38, 999] {
        let report = ai_model::parse(&tensor_header(kind, &[32], 0), 4096).unwrap();
        assert_eq!(report.tensors[0].dtype, format!("type {kind}"));
        assert_eq!(report.tensors[0].data_bytes, None);
        assert_eq!(report.tensors_past_end, 0);
        assert!(report.warnings.is_empty());
    }
    let plain = ai_model::parse(&tensor_header(0, &[3, 2], 0), 4096).unwrap();
    assert_eq!(plain.tensors[0].dtype, "F32");
    assert_eq!(plain.tensors[0].data_bytes, Some(24));
    assert_eq!(plain.tensors_past_end, 0);
    assert!(plain.warnings.is_empty());

    let fixture = corpus.fixtures.iter().find(|fixture| fixture.tensor_type == "MXFP4").unwrap();
    let block = fixture.shape[0] / corpus.inputs.blocks_per_row;
    let bytes = fixture.data_bytes / (corpus.inputs.rows * corpus.inputs.blocks_per_row);
    let report = ai_model::parse(&tensor_header(fixture.type_id, &[block], u64::MAX), u64::MAX).unwrap();
    assert_eq!(report.tensors[0].data_bytes, Some(bytes));
    assert_eq!(report.tensors_past_end, 1);
    assert!(report.warnings.iter().any(|warning| warning.contains("more data")));
}
