use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

/// Real symbols from the corpus of 1,496,282 distinct Swift symbols, expected text from
/// `swift-demangle` (Swift 6.1 Command Line Tools).
#[test]
fn variadic_parameters_print_their_trailing_ellipsis() {
    assert_eq!(
        dm("_$s6CoreML8MLTensorV3all9alongAxes8keepRankACSid_SbtF"),
        "CoreML.MLTensor.all(alongAxes: Swift.Int..., keepRank: Swift.Bool) -> CoreML.MLTensor"
    );
    assert_eq!(
        dm("_$s14NeuralNetworks6TensorV7product13squeezingAxesACSid_tF"),
        "NeuralNetworks.Tensor.product(squeezingAxes: Swift.Int...) -> NeuralNetworks.Tensor"
    );
    assert_eq!(
        dm("_$s8TipsCore7logInfoyyAA0A3LogV_s12StaticStringVs7CVarArg_pdtF"),
        "TipsCore.logInfo(TipsCore.TipsLog, Swift.StaticString, Swift.CVarArg...) -> ()"
    );
    assert_eq!(
        dm("_$s12NIOIMAPCore216SectionSpecifierV4PartV12arrayLiteralAESid_tcfC"),
        "NIOIMAPCore2.SectionSpecifier.Part.init(arrayLiteral: Swift.Int...) -> NIOIMAPCore2.SectionSpecifier.Part"
    );
    assert_eq!(
        dm("_$s11SwiftSQLite5TableV9dropIndex_8ifExistsSSAA11Expressible_pd_SbtF"),
        "SwiftSQLite.Table.dropIndex(_: SwiftSQLite.Expressible..., ifExists: Swift.Bool) -> Swift.String"
    );
}

#[test]
fn constructed_variadic_types_decode() {
    assert_eq!(dm("_$s4main3fooyySid_tF"), "main.foo(Swift.Int...) -> ()");
}

#[test]
fn malformed_variadic_operands_are_invalid() {
    for symbol in ["_$s4main3fooydF", "_$s4main3foo1xySiydF", "_$s4main3fooWZ", "_$sSiNd"] {
        assert_eq!(demangle(symbol).status, Status::Invalid, "{symbol}");
    }
}
