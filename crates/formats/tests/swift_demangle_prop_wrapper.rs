use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

/// Symbols from macOS 15 SDK `.tbd` files and Command Line Tools dylibs, expected text from `swift-demangle`.
#[test]
fn property_wrapper_backing_initializers_decode() {
    assert_eq!(
        dm("_$s10SiriFindMy8SnippetsO33SetSharedLocationVisibilityResultV07alreadyE0SbvpfP"),
        "property wrapper backing initializer of SiriFindMy.Snippets.SetSharedLocationVisibilityResult.alreadySet : Swift.Bool"
    );
    assert_eq!(
        dm("_$s10SiriMailUI0aB5ModelC11attachmentsSay0aB7UIModel01_aB7MessageV0B10AttachmentVGvpfP"),
        "property wrapper backing initializer of SiriMailUI.SiriMailModel.attachments : [SiriMailUIModel._SiriMailMessage.MailAttachment]"
    );
    assert_eq!(dm("_$s10SiriMailUI0aB5ModelC4bodySSvpfP"), "property wrapper backing initializer of SiriMailUI.SiriMailModel.body : Swift.String");
    assert_eq!(
        dm("_$s14PhoneSnippetUI18PlayVoicemailModelV04openA4TextSSvpfP"),
        "property wrapper backing initializer of PhoneSnippetUI.PlayVoicemailModel.openPhoneText : Swift.String"
    );
}

/// A private property keeps its file discriminator.
#[test]
fn a_private_property_keeps_its_discriminator() {
    assert_eq!(
        dm("_$s10SiriFindMy8SnippetsO33SetSharedLocationVisibilityResultV11_visibility33_73FB5C2BEA92CD04A3E6B3D7F5D842E5LLSivpfP"),
        "property wrapper backing initializer of SiriFindMy.Snippets.SetSharedLocationVisibilityResult.(_visibility in _73FB5C2BEA92CD04A3E6B3D7F5D842E5) : Swift.Int"
    );
}

/// The projected-value initializer form.
#[test]
fn init_from_projected_value_decodes() {
    assert_eq!(dm("_$s3Foo3abcSSvpfW"), "property wrapper init from projected value of Foo.abc : Swift.String");
    assert_eq!(dm("_$s3Foo5MyBarV3abcSSvpfW"), "property wrapper init from projected value of Foo.MyBar.abc : Swift.String");
}

/// Constructed symbols, expected text checked with `swift-demangle`.
#[test]
fn constructed_property_wrapper_symbols_decode() {
    assert_eq!(dm("_$s3Foo3abcSSvpfP"), "property wrapper backing initializer of Foo.abc : Swift.String");
    assert_eq!(dm("_$s3Foo5MyBarV3abcSSvpfP"), "property wrapper backing initializer of Foo.MyBar.abc : Swift.String");
    assert_eq!(dm("_$s3Foo5MyBarV3abcSSvpfW"), "property wrapper init from projected value of Foo.MyBar.abc : Swift.String");
}

/// Malformed operands are rejected instead of guessed.
#[test]
fn a_malformed_property_wrapper_operand_is_invalid() {
    for symbol in [
        "_$s3Foo5MyBarVpfP",
        "_$s3Foo3abcSSfP",
        "_$s3Foo3abcSSvpfP_",
        "_$s3Foo3abcSSvpfPX",
        "_$s3Foo3abcSSvpf",
        "_$s3Foo3abcSSvpfP0_",
        "_$s3FoofP",
        "_$s3FoofW",
        "_$s3Foo3barfP",
        "_$s3Foo3abcSSvfP",
        "_$s3Foo3abcSSvfW",
    ] {
        assert_eq!(demangle(symbol).status, Status::Invalid, "{symbol}");
    }
}
