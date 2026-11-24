use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

/// Symbols from macOS 15 SDK `.tbd` files and Command Line Tools dylibs, expected text from `swift-demangle`.
#[test]
fn opaque_type_descriptors_name_the_opaque_return_type() {
    assert_eq!(
        dm("_$s7MediaML13CellularModelC9estimatorQrvpQOMQ"),
        "opaque type descriptor for <<opaque return type of MediaML.CellularModel.estimator : some>>"
    );
    assert_eq!(
        dm("_$s12FinanceKitUI16OrderPaymentViewV4bodyQrvpQOMQ"),
        "opaque type descriptor for <<opaque return type of FinanceKitUI.OrderPaymentView.body : some>>"
    );
    assert_eq!(
        dm("_$s9SnippetUI30SimpleItemRichSearchResultViewV4bodyQrvpQOMQ"),
        "opaque type descriptor for <<opaque return type of SnippetUI.SimpleItemRichSearchResultView.body : some>>"
    );
    assert_eq!(
        dm("_$s15ConversationKit24CaptionsSizeToggleButtonV4bodyQrvpQOMQ"),
        "opaque type descriptor for <<opaque return type of ConversationKit.CaptionsSizeToggleButton.body : some>>"
    );
    assert_eq!(
        dm("_$s10WellnessUI30SpecificMedCompletedLogSnippetV4bodyQrvpQOMQ"),
        "opaque type descriptor for <<opaque return type of WellnessUI.SpecificMedCompletedLogSnippet.body : some>>"
    );
    assert_eq!(
        dm("_$s17PhotosSwiftUICore0A12PrefetchableO8RootViewV4bodyQrvpQOMQ"),
        "opaque type descriptor for <<opaque return type of PhotosSwiftUICore.PhotosPrefetchable.RootView.body : some>>"
    );
    assert_eq!(
        dm("_$s12PhotosUICore17CreateAlbumIntentV16parameterSummaryQrvpZQOMQ"),
        "opaque type descriptor for <<opaque return type of static PhotosUICore.CreateAlbumIntent.parameterSummary : some>>"
    );
    assert_eq!(
        dm("_$s6TipKit4TipsO14OptionsBuilderV15buildExpressionyQrSayAA0A6Option_pSgGFZQOMQ"),
        "opaque type descriptor for <<opaque return type of static TipKit.Tips.OptionsBuilder.buildExpression([TipKit.TipOption?]) -> some>>"
    );
}

/// Constructed symbols, expected text checked with `swift-demangle`.
#[test]
fn constructed_opaque_type_descriptors_decode() {
    assert_eq!(dm("_$s1a2mkQryFQOMQ"), "opaque type descriptor for <<opaque return type of a.mk() -> some>>");
    assert_eq!(dm("_$s1a1HV4bodyQrvpQOMQ"), "opaque type descriptor for <<opaque return type of a.H.body : some>>");
}

/// A symbol whose metadata kind is `Q` but whose operand is not shaped as an opaque return
/// type is invalid here.
#[test]
fn a_malformed_opaque_type_descriptor_is_invalid() {
    for symbol in ["_$s1a2mkQryFQOMQ_", "_$s1a2mkQryFQOMQ0_", "_$s1a2mkQryFQQOMQ", "_$s1a2mkQryFOMQ", "_$s1a2mkQrFQOMQ"] {
        assert_eq!(demangle(symbol).status, Status::Invalid, "{symbol}");
    }
}
