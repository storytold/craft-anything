use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

#[test]
fn pack_types_render() {
    assert_eq!(
        dm("_$s10Foundation10ExpressionVyxxQp_QPq_GAA26DecodableWithConfigurationAAMc"),
        "protocol conformance descriptor for Foundation.Expression<Pack{repeat A}, B> : Foundation.DecodableWithConfiguration in Foundation"
    );
    assert_eq!(
        dm("_$s12AVFoundation15AVMergedMetricsVyxq_q0_q0_Qp_QPGSciAAMc"),
        "protocol conformance descriptor for AVFoundation.AVMergedMetrics<A, B, Pack{repeat C}> : Swift.AsyncSequence in AVFoundation"
    );
}

#[test]
fn pack_expansions_render() {
    assert_eq!(dm("_$s10Foundation10ExpressionV8evaluateyq_xxQpKF"), "Foundation.Expression.evaluate(repeat A) throws -> B");
}

#[test]
fn pack_elements_render_with_level() {
    assert_eq!(dm("_$sxQe_"), "/* level: 0 */ each A");
    assert_eq!(dm("_$sxQe0_"), "/* level: 1 */ each A");
}

#[test]
fn malformed_packs_are_invalid() {
    let d = demangle("_$s4main3fooVyQp");
    assert_eq!(d.status, Status::Invalid);
    let d = demangle("$sSi_QPSg_A63At_QPSg_A63Bt_QPSg_A63Ct_QPSg_A63Dt_QPSg_A63Et_QPSg");
    assert_eq!(d.status, Status::Invalid);
}
