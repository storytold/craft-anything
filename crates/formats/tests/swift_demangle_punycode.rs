use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

#[test]
fn punycode_identifiers() {
    assert_eq!(
        dm("_$s10Foundation6LocaleV12LanguageCodeV0018norwegianBokml_xibAEvpZMV"),
        "property descriptor for static Foundation.Locale.LanguageCode.norwegianBokmål : Foundation.Locale.LanguageCode"
    );
    assert_eq!(
        dm("_$s10Foundation6LocaleV6RegionV0014cteDIvoire_rbbAEvpZMV"),
        "property descriptor for static Foundation.Locale.Region.côteDIvoire : Foundation.Locale.Region"
    );
    assert_eq!(dm("_$s0012vergenza_JFa"), "vergüenza");
}

#[test]
fn malformed_punycode_is_invalid() {
    for s in ["_$s00", "_$s000", "_$s000abc_", "_$s005x", "_$s0018JJJJJJJJJJJJJJJJJJ", "_$s009lAJACHBGa"] {
        assert_eq!(demangle(s).status, Status::Invalid, "{s}");
    }
}
