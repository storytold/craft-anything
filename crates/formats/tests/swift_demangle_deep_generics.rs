use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

#[test]
fn deep_generic_params_decode_as_indexed_names() {
    assert_eq!(
        dm("_$s19ReminderKitInternal7PromiseC4cast2asACyqd__Gqd__m_tlFTq"),
        "method descriptor for ReminderKitInternal.Promise.cast<A>(as: A1.Type) -> ReminderKitInternal.Promise<A1>"
    );
}

#[test]
fn malformed_deep_generic_params_are_invalid() {
    for sym in ["_$s3Foo1bqd_", "_$s3Foo1bqd_d__", "_$s3Foo1bxqd_"] {
        let d = demangle(sym);
        assert_eq!(d.status, Status::Invalid, "{sym}: {:?}", d.demangled);
    }
}
