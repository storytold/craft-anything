use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

#[test]
fn same_type_requirements_print_with_double_equals() {
    assert_eq!(
        dm("_$s10FontParser13PSInterpreterV8evaluate9procedure9argumentsAA0C10ExitReasonOSgAA15PSObjectLocatorV_xtSTRzSi7ElementRtzlF"),
        "FontParser.PSInterpreter.evaluate<A where A: Swift.Sequence, A.Element == Swift.Int>(procedure: FontParser.PSObjectLocator, arguments: A) -> FontParser.PSInterpreterExitReason?"
    );
    assert_eq!(
        dm("_$s10Foundation16AttributedStringV14removeSubrangeyyxSXRzAC5IndexV5BoundRtzlF"),
        "Foundation.AttributedString.removeSubrange<A where A: Swift.RangeExpression, A.Bound == Foundation.AttributedString.Index>(A) -> ()"
    );
}

#[test]
fn malformed_same_type_requirements_are_invalid() {
    assert_eq!(demangle("_$s4main3fooxxXRzlF").status, Status::Invalid);
    assert_eq!(demangle("_$s4main3fooyyxSiRtzlF").status, Status::Invalid);
    assert_eq!(demangle("_$s4main3fooyyxSiN7ElementQzRSlF").status, Status::Invalid);
    assert_eq!(demangle("_$s4main3fooyyxSi_RTzlF").status, Status::Invalid);
    assert_eq!(demangle("_$s4main3fooyyxSi_1YRTzlF").status, Status::Invalid);
}

#[test]
fn protocol_qualified_associated_type_name_decodes() {
    assert_eq!(dm("_$s4main3fooyyxSi7ElementSTRtzlF"), "main.foo<A where A.Swift.Sequence.Element == Swift.Int>(A) -> ()");
}

#[test]
fn substitution_keeps_the_parameter_depth() {
    assert_eq!(dm("_$s4main3fooyyxSi1XRtd__AD1YRtzlF"), "main.foo<A where A1.X == Swift.Int, A.Y == A1.X>(A) -> ()");
}
