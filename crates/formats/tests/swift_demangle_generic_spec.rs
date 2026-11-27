use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

#[test]
fn generic_specialization_symbols_from_the_corpus_decode() {
    assert_eq!(dm("_$sSa5countSivgs5UInt8V_Tg5"), "generic specialization <Swift.UInt8> of Swift.Array.count.getter : Swift.Int");
    assert_eq!(dm("_$sSa15reserveCapacityyySiFSf_Tg5"), "generic specialization <Swift.Float> of Swift.Array.reserveCapacity(Swift.Int) -> ()");
    assert_eq!(dm("_$ss12_ArrayBufferV5countSivsSi_Tg5"), "generic specialization <Swift.Int> of Swift._ArrayBuffer.count.setter : Swift.Int");
}

#[test]
fn constructed_generic_specialization_symbols_decode() {
    assert_eq!(dm("_$s4main3fooyxxFSi_Tg5"), "generic specialization <Swift.Int> of main.foo(A) -> A");
    assert_eq!(dm("_$s4main3fooyxxFSd_Tg5"), "generic specialization <Swift.Double> of main.foo(A) -> A");
}

#[test]
fn malformed_generic_specialization_is_invalid() {
    for symbol in ["_$sSa5countSivgs5UInt8V_Tg", "_$sSa5countSivgs5UInt8V_Tgx", "_$sSa5countSivg_Tg5", "_$s4main3fooyyFSiN_Tg5", "_$s4main3fooyxxFSiNSg_Tg5"] {
        assert_eq!(demangle(symbol).status, Status::Invalid, "{symbol}");
    }
}
