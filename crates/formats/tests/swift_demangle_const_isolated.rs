use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

fn invalid(symbol: &str) {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Invalid, "{symbol} should be invalid, got {:?}", d.demangled);
}

#[test]
fn const_parameter_attribute_decodes() {
    assert_eq!(
        dm("_$s10AppIntents20IntentCollectionSizeV7exactlyACSiYt_tcfC"),
        "AppIntents.IntentCollectionSize.init(exactly: _const Swift.Int) -> AppIntents.IntentCollectionSize"
    );
    assert_eq!(
        dm("_$s10AppIntents20IntentCollectionSizeV3min3maxACSiYt_SiYttcfC"),
        "AppIntents.IntentCollectionSize.init(min: _const Swift.Int, max: _const Swift.Int) -> AppIntents.IntentCollectionSize"
    );
}

#[test]
fn isolated_parameter_attribute_decodes() {
    assert_eq!(
        dm("_$sScS8IteratorV4next9isolationxSgScA_pSgYi_tYaFTu"),
        "async function pointer to Swift.AsyncStream.Iterator.next(isolation: isolated Swift.Actor?) async -> A?"
    );
    assert_eq!(
        dm("_$s10TipKitCore7measure9isolation_s8DurationVScA_pSgYi_yyYaKXEtYaKF"),
        "TipKitCore.measure(isolation: isolated Swift.Actor?, _: () async throws -> ()) async throws -> Swift.Duration"
    );
}

#[test]
fn const_and_isolated_behave_as_type_attributes() {
    assert_eq!(dm("_$s1x2f1yyySSYtcF"), "x.f1((_const Swift.String) -> ()) -> ()");
    assert_eq!(dm("_$s1x2f2yyyScA_pSgYicF"), "x.f2((isolated Swift.Actor?) -> ()) -> ()");
}

#[test]
fn a_marker_without_its_type_is_invalid() {
    for s in ["_$s1x2v1yyYtcvp", "_$s1x2v1yyYicvp", "_$s1x2f1yyyYtcF", "_$s1x2f1yyyYicF"] {
        invalid(s);
    }
}
