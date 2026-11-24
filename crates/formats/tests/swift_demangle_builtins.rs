use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

fn invalid(symbol: &str) {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Invalid, "{symbol}: expected invalid, got {:?}", d.demangled);
}

/// Built-in types decode like `swift-demangle`.
#[test]
fn builtin_types_decode() {
    assert_eq!(dm("_$sBi64_Bv8_N"), "type metadata for Builtin.Vec8xInt64");
    assert_eq!(dm("_$sBi8_Bv4_WV"), "value witness table for Builtin.Vec4xInt8");
    assert_eq!(dm("_$sBi16_Bv4_WV"), "value witness table for Builtin.Vec4xInt16");
    assert_eq!(dm("_$sBi8_Bv64_WV"), "value witness table for Builtin.Vec64xInt8");
    assert_eq!(dm("_$sBi32_Bv64_WV"), "value witness table for Builtin.Vec64xInt32");
    assert_eq!(dm("_$sBi64_Bv64_WV"), "value witness table for Builtin.Vec64xInt64");
    assert_eq!(dm("_$sBi8_Bv32_WV"), "value witness table for Builtin.Vec32xInt8");
    assert_eq!(dm("_$sScc7contextBcvs"), "Swift.UnsafeContinuation.context.setter : Builtin.RawUnsafeContinuation");
    assert_eq!(dm("_$sSce8executorBevg"), "Swift.UnownedSerialExecutor.executor.getter : Builtin.Executor");
    assert_eq!(dm("_$sSi12SIMD4StorageV6_valueBi64_Bv4_vs"), "Swift.Int.SIMD4Storage._value.setter : Builtin.Vec4xInt64");
    assert_eq!(dm("_$sBi32_Bv3_WV"), "value witness table for Builtin.Vec3xInt32");
    assert_eq!(dm("_$sBf16_Bv8_N"), "type metadata for Builtin.Vec8xFPIEEE16");
}

#[test]
fn constructed_simple_builtin_types_decode() {
    assert_eq!(dm("_$sBbN"), "type metadata for Builtin.BridgeObject");
    assert_eq!(dm("_$sBcN"), "type metadata for Builtin.RawUnsafeContinuation");
    assert_eq!(dm("_$sBdN"), "type metadata for Builtin.NonDefaultDistributedActorStorage");
    assert_eq!(dm("_$sBeN"), "type metadata for Builtin.Executor");
    assert_eq!(dm("_$sBjN"), "type metadata for Builtin.Job");
    assert_eq!(dm("_$sBoN"), "type metadata for Builtin.NativeObject");
    assert_eq!(dm("_$sBpN"), "type metadata for Builtin.RawPointer");
    assert_eq!(dm("_$sBtN"), "type metadata for Builtin.SILToken");
    assert_eq!(dm("_$sBwN"), "type metadata for Builtin.Word");
    assert_eq!(dm("_$sBAN"), "type metadata for Builtin.ImplicitActor");
    assert_eq!(dm("_$sBBN"), "type metadata for Builtin.UnsafeValueBuffer");
    assert_eq!(dm("_$sBDN"), "type metadata for Builtin.DefaultActorStorage");
    assert_eq!(dm("_$sBIN"), "type metadata for Builtin.IntLiteral");
    assert_eq!(dm("_$sBON"), "type metadata for Builtin.UnknownObject");
    assert_eq!(dm("_$sBPN"), "type metadata for Builtin.PackIndex");
    assert_eq!(dm("_$sBTN"), "type metadata for Builtin.TheTupleType");
}

#[test]
fn malformed_builtin_types_are_invalid() {
    invalid("_$sBN");
    invalid("_$sBiN");
    invalid("_$sBfN");
    invalid("_$sBzN");
    invalid("_$sBi64_Bv0_N");
    invalid("_$sBi64_Bv4097_N");
    invalid("_$sBi64_Bv8N");
}
