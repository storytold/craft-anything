use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

#[test]
fn associated_conformance_descriptors_decode() {
    assert_eq!(
        dm("_$s9Coherence8Tagged_9P4EnumAC_SYTn"),
        "associated conformance descriptor for Coherence.Tagged_9.Coherence.Tagged_9.Enum: Swift.RawRepresentable"
    );
    assert_eq!(
        dm("_$s8Geometry8BVH2NodeP9BBox2TypeAC_AA0D0Tn"),
        "associated conformance descriptor for Geometry.BVH2Node.Geometry.BVH2Node.BBox2Type: Geometry.BBox2"
    );
    assert_eq!(dm("_$s8Geometry4Ray3P6ScalarAC_s10SIMDScalarTn"), "associated conformance descriptor for Geometry.Ray3.Geometry.Ray3.Scalar: Swift.SIMDScalar");
    assert_eq!(
        dm("_$s4Sage10ThreadableP26ThreadSummarizationOptionsAC_SETn"),
        "associated conformance descriptor for Sage.Threadable.Sage.Threadable.ThreadSummarizationOptions: Swift.Encodable"
    );
}

#[test]
fn a_multi_segment_path_decodes() {
    assert_eq!(
        dm("_$s10Foundation12DataProtocolP7RegionsAC_7ElementST11SubSequenceSlAA15ContiguousBytesTn"),
        "associated conformance descriptor for Foundation.DataProtocol.Foundation.DataProtocol.Regions.Swift.Sequence.Element.Swift.Collection.SubSequence: Foundation.ContiguousBytes"
    );
}

#[test]
fn constructed_symbols_checked_with_swift_demangle() {
    assert_eq!(dm("_$s4main5ProtoP5AssocAC_AA5OtherTn"), "associated conformance descriptor for main.Proto.main.Proto.Assoc: main.Other");
}

#[test]
fn malformed_symbols_are_invalid() {
    for s in ["_$s4main5ProtoTn", "_$s4main5ProtoPTn", "_$s999main5ProtoP5AssocTn", "_$s4main5ProtoP5AssocAC_SiTn"] {
        assert_eq!(demangle(s).status, Status::Invalid, "{s}");
    }
}
