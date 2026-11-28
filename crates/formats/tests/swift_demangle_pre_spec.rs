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
fn generic_pre_specialization_symbols() {
    assert_eq!(dm("_$sSa12_endMutationyyFyXl_Ts5"), "generic pre-specialization <Swift.AnyObject> of Swift.Array._endMutation() -> ()");
    assert_eq!(dm("_$ss10_NativeSetV4copyyyFSS_Ts5"), "generic pre-specialization <Swift.String> of Swift._NativeSet.copy() -> ()");
    assert_eq!(
        dm("_$ss17_NativeDictionaryV4copyyyFSS_SSTs5"),
        "generic pre-specialization <Swift.String, Swift.String> of Swift._NativeDictionary.copy() -> ()"
    );
    assert_eq!(
        dm("_$ss12_ArrayBufferV16beginCOWMutationSbyFSS_Ts5"),
        "generic pre-specialization <Swift.String> of Swift._ArrayBuffer.beginCOWMutation() -> Swift.Bool"
    );
    assert_eq!(
        dm("_$sSD8_VariantV20isUniquelyReferencedSbyFs11AnyHashableV_ypTs5"),
        "generic pre-specialization <Swift.AnyHashable, Any> of Swift.Dictionary._Variant.isUniquelyReferenced() -> Swift.Bool"
    );
}

#[test]
fn more_pre_specialization_symbols() {
    assert_eq!(
        dm("_$ss12_ArrayBufferV30isMutableAndUniquelyReferencedSbyFSS_Ts5"),
        "generic pre-specialization <Swift.String> of Swift._ArrayBuffer.isMutableAndUniquelyReferenced() -> Swift.Bool"
    );
    assert_eq!(
        dm("_$ss12_ArrayBufferV30isMutableAndUniquelyReferencedSbyFyp_Ts5"),
        "generic pre-specialization <Any> of Swift._ArrayBuffer.isMutableAndUniquelyReferenced() -> Swift.Bool"
    );
}

#[test]
fn malformed_pre_specialization_is_invalid() {
    invalid("_$sSa12_endMutationyyFyXl_Ts");
    invalid("_$sSa12_endMutationyyFyXl_Tsx");
    invalid("_$sSa12_endMutationyyFyXl_Ts10");
    invalid("_$sSa12_endMutationyyFyXl_");
    invalid("_$s4main3fooyyF_Ts5");
    invalid("_$s4main3fooyyFSiTs5");
    invalid("_$s4main3fooyyFK_Ts5");
    invalid("_$sKSi_Ts5");
}

/// Each `A64` repeat copies the pre-specialization before it 64 times, so every further stage
/// multiplies the rendered text. The text counts against the expansion budget: a short chain still
/// decodes and a long one is rejected instead of building gigabytes of output.
#[test]
fn repeated_pre_specializations_are_bounded() {
    let two = dm("_$sSiSi_Ts0fi1xVA64B_Ts0");
    assert_eq!(two.matches("generic pre-specialization <Swift.Int> of Swift.Int").count(), 65);
    let five = demangle("_$sSiSi_Ts0fi1xVA64B_Ts0fi1xVA64D_Ts0fi1xVA64F_Ts0fi1xVA64H_Ts0fi1xVA64J_Ts0");
    assert_eq!(five.status, Status::Invalid, "{} bytes", five.demangled.unwrap_or_default().len());
}
