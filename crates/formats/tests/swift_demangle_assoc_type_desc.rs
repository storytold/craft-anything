use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

/// Real symbols from macOS 15 SDK / Command Line Tools binaries, expected text from `swift-demangle`.
#[test]
fn associated_type_descriptors_decode() {
    assert_eq!(dm("_$s5Input5Human11ComputeNodePTl"), "associated type descriptor for Human.ComputeNode.Input");
    assert_eq!(dm("_$s5Type69Coherence8Tagged_7PTl"), "associated type descriptor for Coherence.Tagged_7.Type6");
    assert_eq!(dm("_$s6Images10ProductKit7FeaturePTl"), "associated type descriptor for ProductKit.Feature.Images");
    assert_eq!(dm("_$s4Kind7JetCore15JSONTypeClusterPTl"), "associated type descriptor for JetCore.JSONTypeCluster.Kind");
    assert_eq!(dm("_$s4Body7SwiftUI26ImageAccessibilityProviderPTl"), "associated type descriptor for SwiftUI.ImageAccessibilityProvider.Body");
}

/// Constructed symbols, expected text checked with `swift-demangle`.
#[test]
fn constructed_associated_type_descriptors_decode() {
    assert_eq!(dm("_$s3Bar3Foo5ProtoPTl"), "associated type descriptor for Foo.Proto.Bar");
    assert_eq!(dm("_$s3Baz3Foo5ProtoPTl"), "associated type descriptor for Foo.Proto.Baz");
}

/// Malformed operands are rejected.
#[test]
fn malformed_associated_type_descriptors_are_invalid() {
    for symbol in
        ["_$sPTl", "_$s3FooPTl", "_$s3Foo5ProtoPTl", "_$s3Foo5ProtoP3BarTlX", "_$s3Foo5ProtoP3BarT", "_$s3Bar3Foo5ProtoVTl", "_$s3Bar4_ABCLL3Foo5ProtoPTl"]
    {
        assert_eq!(demangle(symbol).status, Status::Invalid, "{symbol}");
    }
}
