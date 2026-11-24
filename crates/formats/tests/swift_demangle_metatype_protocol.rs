use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

/// Real symbols from the corpus: a protocol metatype `pm` now reads `.Protocol`.
#[test]
fn protocol_metatypes_read_protocol() {
    assert_eq!(
        dm("_$s9RealityIO8MeshPrimC4prim4typeAcA0D0C_AA0C0_pmtcfc"),
        "RealityIO.MeshPrim.init(prim: RealityIO.Prim, type: RealityIO.Mesh.Protocol) -> RealityIO.MeshPrim"
    );
    assert_eq!(
        dm("_$s9RealityIO8CubePrimC4prim4typeAcA0D0C_AA0C0_pmtcfc"),
        "RealityIO.CubePrim.init(prim: RealityIO.Prim, type: RealityIO.Cube.Protocol) -> RealityIO.CubePrim"
    );
    assert_eq!(
        dm("_$s19CompilerSwiftSyntax0C0V10isProtocolySbAA0cE0_pmF"),
        "CompilerSwiftSyntax.Syntax.isProtocol(CompilerSwiftSyntax.SyntaxProtocol.Protocol) -> Swift.Bool"
    );
    assert_eq!(
        dm("_$s9RealityIO9XformPrimC4prim4typeAcA0D0C_AA0C0_pmtcfc"),
        "RealityIO.XformPrim.init(prim: RealityIO.Prim, type: RealityIO.Xform.Protocol) -> RealityIO.XformPrim"
    );
    assert_eq!(
        dm("_$s20_CompilerSwiftSyntax0C0V10asProtocolyAA0cE0_pAaE_pmF"),
        "_CompilerSwiftSyntax.Syntax.asProtocol(_CompilerSwiftSyntax.SyntaxProtocol.Protocol) -> _CompilerSwiftSyntax.SyntaxProtocol"
    );
}

/// Constructed symbols, expected text checked with `swift-demangle`.
#[test]
fn constructed_protocol_metatypes_decode() {
    assert_eq!(dm("$s1a1vSayypmGvp"), "a.v : [Any.Protocol]");
}

/// Malformed operands are rejected.
#[test]
fn a_non_existential_metatype_is_not_a_protocol_metatype() {
    // `m` of a plain type still reads `.Type`, not `.Protocol`.
    assert_eq!(dm("$s1a1vSaySiXmTGvp"), "a.v : [@thick Swift.Int.Type]");
}

/// Constructed malformed symbols are invalid.
#[test]
fn malformed_protocol_metatypes_are_invalid() {
    for symbol in ["_$s1a1vSayypm", "_$s1a1vSaypmGvp"] {
        assert_eq!(demangle(symbol).status, Status::Invalid, "{symbol}");
    }
}
