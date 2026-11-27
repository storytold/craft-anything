use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

/// Real symbols from macOS 15 SDK / Command Line Tools: `fD` on a non-class type is `deinit`.
#[test]
fn f_d_on_a_non_class_type_is_deinit() {
    assert_eq!(dm("_$s13HomeDataModel8StreamedVfD"), "HomeDataModel.Streamed.deinit");
    assert_eq!(dm("_$s13AppleKeyStore18SensitiveRawBufferVfD"), "AppleKeyStore.SensitiveRawBuffer.deinit");
    assert_eq!(dm("_$s15Synchronization19AtomicLazyReferenceVfD"), "Synchronization.AtomicLazyReference.deinit");
    assert_eq!(dm("_$s6System4MachO4PortVfD"), "System.Mach.Port.deinit");
}

/// Constructed symbols: any non-class context reads `deinit`, a class still `__deallocating_deinit`.
#[test]
fn constructed_deinitializer_suffixes_decode() {
    assert_eq!(dm("_$s4main3FooVfD"), "main.Foo.deinit");
    assert_eq!(dm("_$s4main3BarOfD"), "main.Bar.deinit");
    assert_eq!(dm("_$s4main3BazPfD"), "main.Baz.deinit");
    assert_eq!(dm("_$s4main6WidgetCfD"), "main.Widget.__deallocating_deinit");
    // Extension context: Swift selects `deinit` even for a class base.
    assert_eq!(dm("_$s4main6WidgetC4mainEfD"), "(extension in main):main.Widget.deinit");
}

/// Malformed `fD` symbols are invalid.
#[test]
fn malformed_deinitializer_symbols_are_invalid() {
    for symbol in ["_$s4main3FooVf", "_$s4main3FooVfDx", "_$s3FooVfD", "_$s4main3FooVfD_", "_$s4main3FooVWfD", "_$s4mainKfD"] {
        assert_eq!(demangle(symbol).status, Status::Invalid, "{symbol}");
    }
}
