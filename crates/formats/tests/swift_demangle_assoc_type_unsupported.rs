use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

fn invalid(symbol: &str) {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Invalid, "{symbol}: {:?}", d.reason);
}

#[test]
fn associated_type_paths_decode() {
    assert_eq!(
        dm("_$s015_SonicKit_MusicB00cB17StorageOperationsP18mediaAPIAttributes3forSSSg7Content_2IDQZ_tYaKFTj"),
        "dispatch thunk of _SonicKit_MusicKit.MusicKitStorageOperations.mediaAPIAttributes(for: A.Content.ID) async throws -> Swift.String?"
    );
    assert_eq!(
        dm("_$ss14JoinedSequenceV8IteratorV6_inner7Element_ACQZSgvpMV"),
        "property descriptor for Swift.JoinedSequence.Iterator._inner : A.Element.Iterator?"
    );
}

#[test]
fn malformed_associated_type_paths_are_invalid() {
    invalid("$s4main1SV1xQZ");
    invalid("$s4main1SV1x7ElementQYd");
    invalid("$s4main1SV1x7ElementQY__");
    invalid("$s7ElementSi_QZ");
}

#[test]
fn associated_type_base_must_be_a_type() {
    assert_eq!(dm("$sx7ElementSTQx"), "A.Swift.Sequence.Element");
    assert_eq!(dm("$sST7ElementQx"), "Swift.Sequence.Element");
    invalid("$sSiN7ElementSTQx");
    invalid("$sSiN7ElementQx");
}
