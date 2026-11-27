use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

#[test]
fn base_conformance_descriptors_name_the_type_and_protocol() {
    assert_eq!(dm("_$s10TipKitCore0cA8ProtocolPSQTb"), "base conformance descriptor for TipKitCore.CoreTipProtocol: Swift.Equatable");
    assert_eq!(dm("_$s10SnippetKit0A11PluginModelPSETb"), "base conformance descriptor for SnippetKit.SnippetPluginModel: Swift.Encodable");
    assert_eq!(dm("_$ss8_PointerPs17CustomReflectableTb"), "base conformance descriptor for Swift._Pointer: Swift.CustomReflectable");
    assert_eq!(dm("_$s15FindMyMessaging11MessageTypePSYTb"), "base conformance descriptor for FindMyMessaging.MessageType: Swift.RawRepresentable");
    assert_eq!(dm("_$s9Coherence15CRStruct_Base_4PAA0b1_C0Tb"), "base conformance descriptor for Coherence.CRStruct_Base_4: Coherence.CRStruct_Base");
    assert_eq!(dm("_$s11SwiftSQLite10SchemaTypePAA05QueryD0Tb"), "base conformance descriptor for SwiftSQLite.SchemaType: SwiftSQLite.QueryType");
    assert_eq!(dm("_$s16MusicKitInternal08PickableA4ItemPSHTb"), "base conformance descriptor for MusicKitInternal.PickableMusicItem: Swift.Hashable");
    assert_eq!(dm("_$s8RemoteUI17ScriptableElementPs12IdentifiableTb"), "base conformance descriptor for RemoteUI.ScriptableElement: Swift.Identifiable");
}

#[test]
fn a_standard_protocol_on_the_right_decodes() {
    assert_eq!(
        dm("_$ss32ExpressibleByStringInterpolationPs0abC7LiteralTb"),
        "base conformance descriptor for Swift.ExpressibleByStringInterpolation: Swift.ExpressibleByStringLiteral"
    );
    assert_eq!(
        dm("_$s5UIKit24UIBridgedAnimationVectorPs18AdditiveArithmeticTb"),
        "base conformance descriptor for UIKit.UIBridgedAnimationVector: Swift.AdditiveArithmetic"
    );
}

#[test]
fn a_constructed_symbol_decodes() {
    assert_eq!(dm("_$s3Foo3BarPSQTb"), "base conformance descriptor for Foo.Bar: Swift.Equatable");
}

#[test]
fn an_operand_that_is_not_a_type_or_protocol_is_invalid() {
    for symbol in ["_$sTb", "_$s3FooTb", "_$s3Foo3BarTb", "_$sSiSiTb", "_$sSiNSQTb", "_$s3foo3baryyFQOSQTb"] {
        assert_eq!(demangle(symbol).status, Status::Invalid, "{symbol}");
    }
}
