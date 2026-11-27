use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

fn invalid(symbol: &str) {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Invalid, "{symbol}: {:?}", d);
}

#[test]
fn h_parameter_ownership_decodes_as_shared() {
    assert_eq!(
        dm("_$s10Foundation12CharacterSetV12charactersInACSSh_tcfC"),
        "Foundation.CharacterSet.init(charactersIn: __shared Swift.String) -> Foundation.CharacterSet"
    );
    assert_eq!(
        dm("_$s10Foundation12CharacterSetV14contentsOfFileACSgSSh_tcfC"),
        "Foundation.CharacterSet.init(contentsOfFile: __shared Swift.String) -> Foundation.CharacterSet?"
    );
    assert_eq!(
        dm("_$s10Foundation12CharacterSetV20bitmapRepresentationAcA4DataVh_tcfC"),
        "Foundation.CharacterSet.init(bitmapRepresentation: __shared Foundation.Data) -> Foundation.CharacterSet"
    );
    assert_eq!(
        dm("_$s10Foundation12URLQueryItemV4name5valueACSSh_SSSghtcfC"),
        "Foundation.URLQueryItem.init(name: __shared Swift.String, value: __shared Swift.String?) -> Foundation.URLQueryItem"
    );
    assert_eq!(
        dm("_$s10Foundation13URLComponentsV3url23resolvingAgainstBaseURLACSgAA0G0Vh_SbtcfC"),
        "Foundation.URLComponents.init(url: __shared Foundation.URL, resolvingAgainstBaseURL: Swift.Bool) -> Foundation.URLComponents?"
    );
}

#[test]
fn malformed_shared_is_invalid() {
    invalid("_$s3Foo3bar1xyhF");
    invalid("_$s3Foo3barF");
    // `SiN` (type metadata for Swift.Int) is a Global, so `h` must not wrap it;
    // swift-demangle leaves the symbol unchanged.
    invalid("_$sSiNhMD");
}
