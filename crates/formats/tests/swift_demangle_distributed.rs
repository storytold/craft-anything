use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

/// Distributed thunks and accessors decode to their kind followed by the base function's text.
#[test]
fn distributed_thunks_and_accessors_decode() {
    assert_eq!(
        dm("_$s16GameServicesCore0A10KitServiceC28describeChallengeDefinitions09challengeH0Say0aB00G10DefinitionVSgGSayAF3RefVyAF0gJ6Entity_pGG_tYaKFTE"),
        "distributed thunk GameServicesCore.GameKitService.describeChallengeDefinitions(challengeDefinitions: [GameServices.Ref<GameServices.ChallengeDefinitionEntity>]) async throws -> [GameServices.ChallengeDefinition?]"
    );
    assert_eq!(
        dm("_$s16GameServicesCore0A10KitServiceC28describeChallengeDefinitions09challengeH0Say0aB00G10DefinitionVSgGSayAF3RefVyAF0gJ6Entity_pGG_tYaKFTETu"),
        "async function pointer to distributed thunk GameServicesCore.GameKitService.describeChallengeDefinitions(challengeDefinitions: [GameServices.Ref<GameServices.ChallengeDefinitionEntity>]) async throws -> [GameServices.ChallengeDefinition?]"
    );
}

/// Constructed symbols, expected text checked with `swift-demangle`.
#[test]
fn constructed_distributed_symbols_decode() {
    assert_eq!(dm("_$s4main3fooyyFTE"), "distributed thunk main.foo() -> ()");
    assert_eq!(dm("_$s4main3fooyyFTF"), "distributed accessor for main.foo() -> ()");
    assert_eq!(dm("_$s4main3fooyyFTETu"), "async function pointer to distributed thunk main.foo() -> ()");
    assert_eq!(dm("_$s4main3fooyyFTFTu"), "async function pointer to distributed accessor for main.foo() -> ()");
}

/// Malformed operands are rejected instead of guessed.
#[test]
fn malformed_distributed_symbols_are_invalid() {
    for symbol in ["_$s4main3fooyyFTE_", "_$s4main3fooyyFTY", "_$s4main3fooyyFTZ", "_$s4mainTE", "_$syTE", "_$sSiTE"] {
        assert_eq!(demangle(symbol).status, Status::Invalid, "{symbol}");
    }
}
