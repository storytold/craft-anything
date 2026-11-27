use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

fn invalid(symbol: &str) {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Invalid, "{symbol}: {:?}", d);
}

/// Real symbols from the corpus, expected text from `swift-demangle`.
#[test]
fn owned_parameters_decode() {
    assert_eq!(dm("_$ss10ArraySliceV6appendyyxnF"), "Swift.ArraySlice.append(__owned A) -> ()");
    assert_eq!(dm("_$sScF7enqueueyys11ExecutorJobVnFTq"), "method descriptor for Swift.Executor.enqueue(__owned Swift.ExecutorJob) -> ()");
    assert_eq!(dm("_$ss10SetAlgebraP6update4with7ElementQzSgAFn_tFTj"), "dispatch thunk of Swift.SetAlgebra.update(with: __owned A.Element) -> A.Element?");
}

/// A constructed symbol, checked with `swift-demangle`.
#[test]
fn a_constructed_owned_parameter_decodes() {
    assert_eq!(dm("_$s3Foo1fyyxnF"), "Foo.f(__owned A) -> ()");
}

/// Malformed operands are rejected.
#[test]
fn non_type_operands_are_invalid() {
    invalid("_$s3Foo1fynF");
    // metadata Global node is not a whole type
    invalid("_$sSiNn");
}
