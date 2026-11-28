use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

#[test]
fn operator_h_decodes() {
    assert_eq!(
        dm("_$s8PaperKit5ImageV7$bounds9Coherence10CRRegisterVySo6CGRectVAiE7CRValueAAyHCg_Gvg"),
        "PaperKit.Image.$bounds.getter : Coherence.CRRegister<__C.CGRect>"
    );
    assert_eq!(
        dm("_$s8PaperKit5ImageV14contentsBounds9Coherence13TaggedValue_2VySo6CGRectVAA4QuadVAiE7CRValueAAyHCg_GSgvpMV"),
        "property descriptor for PaperKit.Image.contentsBounds : Coherence.TaggedValue_2<__C.CGRect, PaperKit.Quad>?"
    );
}

#[test]
fn malformed_operator_h_is_invalid() {
    let bad = [
        "_$s8PaperKit5ImageV7$bounds9Coherence10CRRegisterVySo6CGRectVAiE7CRValueAAyHCg_GvgX",
        "_$sSiHD0_",
        "_$sSiHn",
        "_$sSig_",
        "_$sSaySiSiSHHPSi_HCg_G",
        "_$sSaySiSiSHyHCg_G",
        "_$sSHHPc",
        "_$sSiSiHO",
        "_$sSaySiyHXG",
    ];
    for b in bad {
        let d = demangle(b);
        assert_eq!(d.status, Status::Invalid, "{b}: {:?}", d.reason);
    }
}
