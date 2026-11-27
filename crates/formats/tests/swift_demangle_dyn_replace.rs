use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

#[test]
fn dynamically_replaceable_key_symbols_decode() {
    assert_eq!(
        dm("_$s16CoreHapticsTools8CHTEventC4timeSo9NSDecimalavsTx"),
        "dynamically replaceable key for CoreHapticsTools.CHTEvent.time.setter : __C.NSDecimal"
    );
    assert_eq!(
        dm("_$s16CoreHapticsTools8CHTEventC4timeSo9NSDecimalavgTx"),
        "dynamically replaceable key for CoreHapticsTools.CHTEvent.time.getter : __C.NSDecimal"
    );
    assert_eq!(
        dm("_$s16CoreHapticsTools19CHTAudioCustomEventC3panSo9NSDecimalaSgvgTx"),
        "dynamically replaceable key for CoreHapticsTools.CHTAudioCustomEvent.pan.getter : __C.NSDecimal?"
    );
}

#[test]
fn constructed_dynamically_replaceable_symbols_decode() {
    assert_eq!(dm("_$s4main3fooyyFTx"), "dynamically replaceable key for main.foo() -> ()");
    assert_eq!(dm("_$s4main3fooyyFTX"), "dynamically replaceable variable for main.foo() -> ()");
    assert_eq!(dm("_$s4main3fooyyFTI"), "dynamically replaceable thunk for main.foo() -> ()");
    assert_eq!(dm("_$s4main6WidgetC5countSivgTX"), "dynamically replaceable variable for main.Widget.count.getter : Swift.Int");
    assert_eq!(dm("_$s4main6WidgetC5countSivgTI"), "dynamically replaceable thunk for main.Widget.count.getter : Swift.Int");
    assert_eq!(dm("_$s4main1SVTX"), "dynamically replaceable variable for main.S");
    assert_eq!(dm("_$s4main1SVTx"), "dynamically replaceable key for main.S");
    assert_eq!(dm("_$s4main1SVTI"), "dynamically replaceable thunk for main.S");
}

#[test]
fn an_operand_that_is_not_valid_is_invalid() {
    for symbol in [
        "_$s4main1xSivp_Tx",
        "_$s3Foo5MyBarV_Tx",
        "_$s3Foo5MyBarV3abcTx",
        "_$s3Foo1A3abcTx",
        "_$s4main1xSivp_TX",
        "_$s4main1xSivp_TI",
        "_$s_Tx",
        "_$s_TX",
        "_$s_TI",
        "_$s3fooTx",
        "_$s3fooTX",
        "_$s3fooTI",
    ] {
        assert_eq!(demangle(symbol).status, Status::Invalid, "{symbol}");
    }
}
