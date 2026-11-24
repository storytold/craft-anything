use craft_anything_formats::swift_demangle::{Status, demangle};

fn dm(symbol: &str) -> String {
    let d = demangle(symbol);
    assert_eq!(d.status, Status::Demangled, "{symbol}: {:?}", d.reason);
    d.demangled.unwrap_or_default()
}

#[test]
fn generic_parameter_names_past_z_use_two_letters() {
    assert_eq!(
        dm(
            "_$s15ParsingInternal10Flatten_25Vyxq_q0_q1_q2_q3_q4_q5_q6_q7_q8_q9_q10_q11_q12_q13_q14_q15_q16_q17_q18_q19_q20_q21_q22_q23_q24_GAA16ContextfulParserAAMc"
        ),
        "protocol conformance descriptor for ParsingInternal.Flatten_25<A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, AB> : ParsingInternal.ContextfulParser in ParsingInternal"
    );
    assert_eq!(
        dm(
            "_$s15ParsingInternal10Flatten_29Vyxq_q0_q1_q2_q3_q4_q5_q6_q7_q8_q9_q10_q11_q12_q13_q14_q15_q16_q17_q18_q19_q20_q21_q22_q23_q24_q25_q26_q27_q28_GAA16ContextfulParserAAMc"
        ),
        "protocol conformance descriptor for ParsingInternal.Flatten_29<A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, AB, BB, CB, DB, EB> : ParsingInternal.ContextfulParser in ParsingInternal"
    );
}
