use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

fn invalid(symbol: &str) {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Invalid, "{symbol}: {:?}", d.demangled);
}

#[test]
fn protocol_self_conformance_descriptor_decodes() {
    assert_eq!(dm("_$ss5ErrorMS"), "protocol self-conformance descriptor for Swift.Error");
    assert_eq!(dm("_$ss4Int8MS"), "protocol self-conformance descriptor for Swift.Int8");
}

#[test]
fn protocol_self_conformance_witness_table_decodes() {
    assert_eq!(dm("_$ss5ErrorWS"), "protocol self-conformance witness table for Swift.Error");
    assert_eq!(dm("_$ss4Int8WS"), "protocol self-conformance witness table for Swift.Int8");
}

#[test]
fn protocol_self_conformance_witness_decodes() {
    assert_eq!(dm("_$ss5ErrorP5_codeSivgTS"), "protocol self-conformance witness for Swift.Error._code.getter : Swift.Int");
    assert_eq!(dm("_$ss5ErrorP7_domainSSvgTS"), "protocol self-conformance witness for Swift.Error._domain.getter : Swift.String");
    assert_eq!(dm("_$ss5ErrorP9_userInfoyXlSgvgTS"), "protocol self-conformance witness for Swift.Error._userInfo.getter : Swift.AnyObject?");
    assert_eq!(dm("_$ss5ErrorP19_getEmbeddedNSErroryXlSgyFTS"), "protocol self-conformance witness for Swift.Error._getEmbeddedNSError() -> Swift.AnyObject?");
}

#[test]
fn constructed_symbols_decode() {
    assert_eq!(dm("_$s3Foo5ProtoPMS"), "protocol self-conformance descriptor for Foo.Proto");
    assert_eq!(dm("_$s3Foo5ProtoPWS"), "protocol self-conformance witness table for Foo.Proto");
}

#[test]
fn malformed_operands_are_invalid() {
    invalid("_$ss5ErrorW");
    invalid("_$ss5ErrorM");
    invalid("_$ss5ErrorP5_codeSivgT");
    invalid("_$ss5ErrorP5_codeSivgTSx");
    invalid("_$s3Foo5ProtoPMSx");
    invalid("_$ssMS");
    invalid("_$sSiMS");
    invalid("_$syTS");
    invalid("_$s3FooTS");
    invalid("_$sSiWS");
    invalid("_$sSiMaTS");
}
