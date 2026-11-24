use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

/// Symbols from macOS 15 SDK `.tbd` files and Command Line Tools dylibs, expected text from
/// `swift-demangle`. A protocol at module level prints its module and its name.
#[test]
fn a_protocol_at_module_level_decodes() {
    assert_eq!(dm("_$s7SwiftUI10ShapeStyleTL"), "protocol requirements base descriptor for SwiftUI.ShapeStyle");
    assert_eq!(dm("_$s11NotesShared8CommandsTL"), "protocol requirements base descriptor for NotesShared.Commands");
    assert_eq!(dm("_$s8MusicKit11GenreFilterTL"), "protocol requirements base descriptor for MusicKit.GenreFilter");
    assert_eq!(dm("_$s11TeaSettings10ResettableTL"), "protocol requirements base descriptor for TeaSettings.Resettable");
    assert_eq!(dm("_$s4VDAF20AggregatableProtocolTL"), "protocol requirements base descriptor for VDAF.AggregatableProtocol");
    assert_eq!(dm("_$s9JetEngine15AccountProviderTL"), "protocol requirements base descriptor for JetEngine.AccountProvider");
    assert_eq!(dm("_$s12GRDBInternal12TableRequestTL"), "protocol requirements base descriptor for GRDBInternal.TableRequest");
}

/// A protocol nested in another type keeps every name of the path, from the module down.
#[test]
fn a_protocol_nested_in_a_type_decodes() {
    assert_eq!(dm("_$s12HMFoundation12StateMachineO14EngineProtocolTL"), "protocol requirements base descriptor for HMFoundation.StateMachine.EngineProtocol");
    assert_eq!(dm("_$s10Accelerate9BNNSGraphO15PointerArgumentTL"), "protocol requirements base descriptor for Accelerate.BNNSGraph.PointerArgument");
    assert_eq!(
        dm("_$s11PassKitCore17NearbyPeerPaymentO14AnalyticsErrorTL"),
        "protocol requirements base descriptor for PassKitCore.NearbyPeerPayment.AnalyticsError"
    );
    assert_eq!(
        dm("_$s23HomeKitDaemonFoundation3MTRO7ClusterO10DefinitionTL"),
        "protocol requirements base descriptor for HomeKitDaemonFoundation.MTR.Cluster.Definition"
    );
    assert_eq!(
        dm("_$s5UIKit14IntelligenceUIO15PromptEntryViewC8DelegateTL"),
        "protocol requirements base descriptor for UIKit.IntelligenceUI.PromptEntryView.Delegate"
    );
}

/// Names that use word substitutions from earlier identifiers, and names that contain `_`.
#[test]
fn a_name_with_word_substitutions_decodes() {
    assert_eq!(
        dm("_$s013CompilerSwiftA21PluginMessageHandling0cD7HandlerTL"),
        "protocol requirements base descriptor for CompilerSwiftCompilerPluginMessageHandling.PluginMessageHandler"
    );
    assert_eq!(dm("_$s012_JetUI_SwiftB00A17ViewRepresentableTL"), "protocol requirements base descriptor for _JetUI_SwiftUI.JetViewRepresentable");
}

/// A standard library protocol is one operand (`SB`, `Sl`, `Sz`), so the name and context are not
/// separate: `swift-demangle` prints `Swift.` in front of the standard name.
#[test]
fn standard_library_protocols_decode() {
    assert_eq!(dm("_$sSBTL"), "protocol requirements base descriptor for Swift.BinaryFloatingPoint");
    assert_eq!(dm("_$sSlTL"), "protocol requirements base descriptor for Swift.Collection");
    assert_eq!(dm("_$sSzTL"), "protocol requirements base descriptor for Swift.BinaryInteger");
}

/// Constructed symbols, expected text checked with `swift-demangle`.
#[test]
fn constructed_protocol_requirement_descriptors_decode() {
    assert_eq!(dm("_$s3Foo3BarTL"), "protocol requirements base descriptor for Foo.Bar");
    assert_eq!(dm("_$s3Foo3BarV3BazTL"), "protocol requirements base descriptor for Foo.Bar.Baz");
    assert_eq!(dm("_$s3Foo3BarPTL"), "protocol requirements base descriptor for Foo.Bar");
    assert_eq!(dm("_$s3Foo2eeoiTL"), "protocol requirements base descriptor for Foo.== infix");
    assert_eq!(dm("_$s3Foo3Bar1_LLTL"), "protocol requirements base descriptor for Foo.(Bar in _)");
    assert_eq!(dm("_$s3Foo3BarTLTm"), "merged protocol requirements base descriptor for Foo.Bar");
}

/// `swift-demangle` rejects the first two as well. For the others it prints the symbol back, or
/// runs the leftover `Foo` into the phrase, or prints `Baz in Foo.bar() -> Swift.Int` for a
/// function in place of the context; all of them are invalid here.
#[test]
fn an_operand_that_is_not_a_protocol_is_invalid() {
    for symbol in
        ["_$s3Foo3BarVTL", "_$s3Foo3BarP3BazVTL", "_$s3FooTL", "_$sTL", "_$s3Foo3BarTLTL", "_$s3Foo3BarTL3Baz", "_$s3Foo3BarTL_", "_$s3Foo3barSiyF3BazTL"]
    {
        assert_eq!(demangle(symbol).status, Status::Invalid, "{symbol}");
    }
}
