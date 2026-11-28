use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

#[test]
fn protocol_conformance_descriptors_with_generic_requirements_decode() {
    assert_eq!(
        dm("_$s11TabularData6ColumnVyxGSQAASQRzlMc"),
        "protocol conformance descriptor for <A where A: Swift.Equatable> TabularData.Column<A> : Swift.Equatable in TabularData"
    );
    assert_eq!(
        dm("_$s7SwiftUI16IndirectOptionalOyxGSQAASQRzlMc"),
        "protocol conformance descriptor for <A where A: Swift.Equatable> SwiftUI.IndirectOptional<A> : Swift.Equatable in SwiftUI"
    );
}

#[test]
fn property_descriptors_with_function_typed_properties_decode() {
    assert_eq!(
        dm("_$ss18LazyFilterSequenceV10_predicateySb7ElementQzcvpMV"),
        "property descriptor for Swift.LazyFilterSequence._predicate : (A.Element) -> Swift.Bool"
    );
    assert_eq!(
        dm("_$ss18LazyFilterSequenceV8IteratorV10_predicateySb7ElementQzcvg"),
        "Swift.LazyFilterSequence.Iterator._predicate.getter : (A.Element) -> Swift.Bool"
    );
}

#[test]
fn getters_of_async_methods_decode() {
    assert_eq!(
        dm("_$s12AppleIDSetup28AutomaticRepairModelExchangeV15receiveFinishedyyACYacvg"),
        "AppleIDSetup.AutomaticRepairModelExchange.receiveFinished.getter : (AppleIDSetup.AutomaticRepairModelExchange) async -> ()"
    );
}

#[test]
fn an_operand_without_a_name_or_context_is_invalid() {
    let symbol = "_$s4mainvpMV"; // bare contextless property descriptor, no name marker
    assert_eq!(demangle(symbol).status, Status::Invalid, "{symbol}");
}
