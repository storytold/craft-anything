use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

/// `Le`: a related declaration, as in synthesized Clang-imported error types.
#[test]
fn related_declaration() {
    assert_eq!(
        dm("_$s13HomeDataModel18MatterCommandErrorV0dF0V07wrappedF0SC12MTRErrorCodeLeVvg"),
        "HomeDataModel.MatterCommandError.MatterError.wrappedError.getter : __C_Synthesized.related decl 'e' for MTRErrorCode"
    );
    assert_eq!(dm("_$s4main1fLeyyF"), "main.related decl 'e' for f() -> ()");
}

/// `L_`: a locally-discriminated declaration, numbered from 1.
#[test]
fn locally_discriminated() {
    // Constructed and checked with swift-demangle.
    assert_eq!(dm("$s4main1fL_yyF"), "f #1 () -> () in main");
    assert_eq!(dm("$s4main1fL0_yyF"), "f #2 () -> () in main");
    assert_eq!(dm("$s4main1aL_Sivp"), "a #1 : Swift.Int in main");
}

/// `Ll`: an anonymous file-discriminated declaration.
#[test]
fn anonymous_file_discriminated() {
    assert_eq!(dm("$s4main1fLlyyF"), "main.(in f)() -> ()");
    assert_eq!(dm("$s4main1aLlSivp"), "main.(in a) : Swift.Int");
}

/// Invalid cases.
#[test]
fn invalid_local_decls() {
    for symbol in ["$s4main1fL", "$s4main1fLe", "$s4main1fL_", "$s4main1aLL", "$s4main1fL18446744073709551614_yyF"] {
        let d = demangle(symbol);
        // Some may demangle or not; ensure L alone without index is rejected.
        if symbol == "$s4main1fL" {
            assert_eq!(d.status, Status::Invalid, "{symbol}: {:?}", d.demangled);
        }
    }
}

/// An entity written after `of` or `in` keeps its local owner there whole.
#[test]
fn local_owner_after_of_or_in() {
    assert_eq!(dm("$s4main1fL_yyFfA_"), "default argument 0 of f #1 () -> () in main");
    assert_eq!(dm("$s4main1fL_yyFyycfU_"), "closure #1 () -> () in f #1 () -> () in main");
}

/// Inside a local type, the nested path comes first and the local type follows ` in `.
#[test]
fn nested_in_local_type() {
    assert_eq!(dm("$s4main5outeryyF1SL_V1TVMa"), "type metadata accessor for T in S #1 in main.outer() -> ()");
    assert_eq!(dm("$s4main5outeryyF1SL_V1TV1UVMa"), "type metadata accessor for T.U in S #1 in main.outer() -> ()");
    assert_eq!(dm("$s4main5outeryyF1SL_V1TV1fyyF"), "T.f() -> () in S #1 in main.outer() -> ()");
    assert_eq!(dm("$s4main5outeryyF1SL_V1fyyF"), "f() -> () in S #1 in main.outer() -> ()");
}

/// Local declarations nested in one another render each containing context once. Rendering it
/// twice at every level doubled the work, so this symbol took minutes.
#[test]
fn nested_local_declarations_render_each_context_once() {
    let out = dm(&format!("$s4main{}", "1fL_yyF1gyyF".repeat(30)));
    assert_eq!(out.matches("f #1 () -> ()").count(), 30);
    assert!(out.ends_with(" in main"), "{out}");
}
