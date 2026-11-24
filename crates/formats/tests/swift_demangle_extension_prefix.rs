use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

/// Real symbols from macOS 15 SDK `.tbd` files and Command Line Tools dylibs, expected text from
/// `swift-demangle`.
#[test]
fn extension_prefix_is_printed_for_extension_contexts() {
    assert_eq!(
        dm("_$s10Foundation20NSUTF8StringEncodingSSAAE0D0Vvg"),
        "Foundation.NSUTF8StringEncoding.getter : (extension in Foundation):Swift.String.Encoding"
    );
    assert_eq!(dm("_$sSo13GKAccessPointC12GameCenterUIE06updateE0yyF"), "(extension in GameCenterUI):__C.GKAccessPoint.updateUI() -> ()");
    assert_eq!(
        dm("_$sSo9PXGLayoutC12PhotosUICoreE27SpriteConfigurationSnapshotVN"),
        "type metadata for (extension in PhotosUICore):__C.PXGLayout.SpriteConfigurationSnapshot"
    );
    assert_eq!(
        dm("_$s22PrivateSearchProtocols11DocumentKeyV0aB4CoreE8toStringSSyF"),
        "(extension in PrivateSearchCore):PrivateSearchProtocols.DocumentKey.toString() -> Swift.String"
    );
    assert_eq!(
        dm("_$sSo22UIToolTipConfigurationC5UIKitE10sourceRectSo6CGRectVSgvpMV"),
        "property descriptor for (extension in UIKit):__C.UIToolTipConfiguration.sourceRect : __C.CGRect?"
    );
    assert_eq!(
        dm("_$s7SwiftUI4ViewPAAE31defaultAdaptableTabBarPlacementyQrAA0efgH0VF"),
        "(extension in SwiftUI):SwiftUI.View.defaultAdaptableTabBarPlacement(SwiftUI.AdaptableTabBarPlacement) -> some"
    );
    assert_eq!(
        dm("_$sSo22NSAttributeDescriptionC8CoreDataE13AttributeTypeV13transformableAEvgZ"),
        "static (extension in CoreData):__C.NSAttributeDescription.AttributeType.transformable.getter : (extension in CoreData):__C.NSAttributeDescription.AttributeType"
    );
}

/// Malformed operands are rejected rather than guessed.
#[test]
fn malformed_extension_prefix_symbols_are_invalid() {
    for symbol in ["_$sE", "_$s3FooE", "_$s3Foo3BarE"] {
        assert_eq!(demangle(symbol).status, Status::Invalid, "{symbol}");
    }
}
