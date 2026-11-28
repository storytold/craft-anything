//! The game mechanics oracle: the original's numbers are measured, and the build's self-test is
//! measured against them.
//!
//! The test that matters most is the last one: a build whose self-test says everything works,
//! but whose numbers are not the original's, must be caught.

use craft_anything_engine::workspace::Workspace;
use serde_json::json;

use super::oracle_game::{constants_in, measured_values, numbers_in};

#[test]
fn the_numbers_in_a_text_are_found_in_order() {
    assert_eq!(numbers_in("walked 3.1 m forward"), ["3.1"]);
    assert_eq!(numbers_in("fired 5 shots, 2 hit"), ["5", "2"]);
    assert_eq!(numbers_in("no numbers here"), Vec::<String>::new());
}

#[test]
fn a_signed_number_keeps_its_sign() {
    assert_eq!(numbers_in("damage -7"), ["-7"]);
    assert_eq!(numbers_in("damage 7"), ["7"]);
}

#[test]
fn a_number_with_no_digits_is_not_a_number() {
    assert!(numbers_in("m/s").is_empty());
    assert!(numbers_in("3 m/s").contains(&"3".to_string()));
}

/// The numbers a self-test report says it measured, by step.
#[test]
fn a_report_is_read_as_the_numbers_it_measured() {
    let report = json!({
        "version": 1,
        "steps": [
            {"step": "move", "passed": true, "detail": "walked 3.1 m forward"},
            {"step": "act", "passed": true, "detail": "fired 5 shots, 2 hit"},
            {"step": "hud", "passed": true, "detail": "health, ammo and the objective are drawn"}
        ]
    });
    let values = measured_values(&report);
    assert_eq!(values["move"], json!(["3.1"]));
    assert_eq!(values["act"], json!(["5", "2"]));
    // A step with no number in it is not read: there is nothing to compare.
    assert!(values.get("hud").is_none());
}

/// The constants in decompiled code, without the addresses.
#[test]
fn the_constants_in_code_are_found_and_the_addresses_are_not() {
    let pseudo = "int64_t sub_1000(int64_t x0) {\n  x0 = x0 + 0x68;\n  return f(x0 * 4.5);\n}";
    let constants = constants_in(pseudo);
    assert!(constants.contains(&"0x68".to_string()), "{constants:?}");
    assert!(constants.contains(&"4.5".to_string()), "{constants:?}");
    // The function's own name is not a value.
    assert!(!constants.contains(&"1000".to_string()), "{constants:?}");
}

#[test]
fn a_big_hex_number_is_an_address_not_a_value() {
    let pseudo = "x0 = 0x140012a0;";
    assert!(constants_in(pseudo).is_empty(), "an address is not a value");
}

/// A build that reports the original's numbers is not caught.
#[test]
fn a_build_that_measured_the_originals_numbers_is_not_caught() {
    let script = super::oracle_game::script(json!({"move": ["3.1"]}), json!({"sub_1": ["3.1"]}));
    let build = json!({"steps": [{"step": "move", "passed": true, "detail": "walked 3.1 m forward"}]});
    let observation = vec![craft_anything_engine::oracle::Observation {
        oracle: super::oracle_game::ORACLE_ID.into(),
        version: 1,
        readings: vec![Some(craft_anything_engine::oracle::Reading { value: measured_values(&build), engine: None })],
        gaps: vec![],
    }];
    let differences = craft_anything_engine::oracle::compare(&script, &observation);
    assert!(differences.iter().all(|d| !d.is_failure()), "the same numbers must not differ: {differences:?}");
}

/// A build that reports different numbers is caught — even though its self-test says it passed.
#[test]
fn a_build_that_measured_different_numbers_is_caught() {
    let script = super::oracle_game::script(json!({"move": ["3.1"]}), json!({"sub_1": ["3.1"]}));
    // The self-test says everything works; the numbers are wrong.
    let build = json!({"steps": [{"step": "move", "passed": true, "detail": "walked 9.9 m forward"}]});
    let observation = vec![craft_anything_engine::oracle::Observation {
        oracle: super::oracle_game::ORACLE_ID.into(),
        version: 1,
        readings: vec![Some(craft_anything_engine::oracle::Reading { value: measured_values(&build), engine: None })],
        gaps: vec![],
    }];
    let differences = craft_anything_engine::oracle::compare(&script, &observation);
    let failures: Vec<_> = differences.iter().filter(|d| d.is_failure()).collect();
    assert!(!failures.is_empty(), "a different number must be caught: {differences:?}");
    assert!(failures[0].detail.contains("3.1") && failures[0].detail.contains("9.9"), "{}", failures[0].detail);
}

/// A build that measured nothing cannot be said to match.
#[test]
fn a_build_that_measured_nothing_is_not_alike() {
    let script = super::oracle_game::script(json!({"move": ["3.1"]}), json!({"sub_1": ["3.1"]}));
    let observation = vec![craft_anything_engine::oracle::Observation {
        oracle: super::oracle_game::ORACLE_ID.into(),
        version: 1,
        readings: vec![Some(craft_anything_engine::oracle::Reading { value: json!({}), engine: None })],
        gaps: vec![],
    }];
    let differences = craft_anything_engine::oracle::compare(&script, &observation);
    assert!(differences.iter().any(|d| d.is_failure()), "measuring nothing is not measuring the same: {differences:?}");
}

/// A build that saves a self-test report with the given steps, as a project holds it.
#[cfg(unix)]
fn build_with_self_test(ws: &Workspace, steps: &str) {
    use std::os::unix::fs::PermissionsExt;
    let app = ws.app_dir();
    std::fs::create_dir_all(app.join("target/debug")).unwrap();
    std::fs::write(app.join("Cargo.toml"), "[package]\nname = \"game\"\n").unwrap();
    let program = app.join("target/debug/game");
    // The check runs the program with `--self-test <file>`, so the file is the second argument.
    let script = format!("#!/bin/sh\ncat > \"$2\" <<'EOF'\n{}\nEOF\n", steps);
    std::fs::write(&program, script).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The self-test is the build's own word: with a comparison recorded, its passing steps are
/// corroboration, and its failing steps still fail.
#[cfg(unix)]
#[test]
fn the_self_test_is_corroboration_when_a_comparison_was_recorded() {
    use super::kinds::Check;
    let d = tempfile::tempdir().unwrap();
    let mut ws = Workspace::create(d.path(), "Game", "Rebuild this game", craft_anything_engine::workspace::TargetRef::None).unwrap();
    build_with_self_test(
        &ws,
        r#"{"version":1,"steps":[{"step":"move","passed":true,"detail":"walked 3.1 m forward"},{"step":"act","passed":true,"detail":"fired"},{"step":"hud","passed":true,"detail":"drawn"}]}"#,
    );

    // Without a comparison, the self-test's own claims hold the check up.
    let worker = super::tests::check_worker(&ws, true);
    let mut cx = super::kinds::CheckCx::for_test(&worker, &ws, 1);
    let without = super::kinds::SelfTestCheck.run(&mut cx).expect("the check runs");
    assert!(without.claims.iter().all(|c| c.passed), "a passing self-test holds up on its own: {:?}", without.claims);
    assert_eq!(without.claims.len(), 3, "the core loop's three steps");

    // With a comparison recorded, its passing steps are corroboration: they are reported, but
    // they do not hold the check up.
    ws.manifest.script = Some(super::oracle_game::script(json!({}), json!({})));
    let worker = super::tests::check_worker(&ws, true);
    let mut cx = super::kinds::CheckCx::for_test(&worker, &ws, 1);
    let with = super::kinds::SelfTestCheck.run(&mut cx).expect("the check runs");
    assert!(with.claims.is_empty(), "a passing step is corroboration, not a claim: {:?}", with.claims);
    assert!(with.note.contains("own account"), "{}", with.note);
}

/// A self-test step that failed still fails, with or without a comparison.
#[cfg(unix)]
#[test]
fn a_failing_self_test_step_still_fails() {
    use super::kinds::Check;
    let d = tempfile::tempdir().unwrap();
    let mut ws = Workspace::create(d.path(), "Game", "Rebuild this game", craft_anything_engine::workspace::TargetRef::None).unwrap();
    build_with_self_test(
        &ws,
        r#"{"version":1,"steps":[{"step":"move","passed":false,"detail":"the player did not move"},{"step":"act","passed":true,"detail":"fired"},{"step":"hud","passed":true,"detail":"drawn"}]}"#,
    );
    ws.manifest.script = Some(super::oracle_game::script(json!({}), json!({})));
    let worker = super::tests::check_worker(&ws, true);
    let mut cx = super::kinds::CheckCx::for_test(&worker, &ws, 1);
    let with = super::kinds::SelfTestCheck.run(&mut cx).expect("the check runs");
    assert_eq!(with.claims.len(), 1, "only the failing step holds the check up: {:?}", with.claims);
    assert!(!with.claims[0].passed);
}

/// The sheet: the vocabulary both sides speak.
fn a_sheet() -> craft_anything_engine::mechanics::Mechanics {
    craft_anything_engine::mechanics::Mechanics {
        version: 1,
        values: vec![
            craft_anything_engine::mechanics::Mechanic {
                name: "walk speed".into(),
                group: "movement".into(),
                value: craft_anything_engine::mechanics::MechanicValue::Number(4.5),
                unit: "m/s".into(),
                source: "function 0x140012a0".into(),
                estimated: false,
            },
            craft_anything_engine::mechanics::Mechanic {
                name: "rifle damage".into(),
                group: "rifle".into(),
                value: craft_anything_engine::mechanics::MechanicValue::Number(7.0),
                unit: String::new(),
                source: String::new(),
                estimated: true,
            },
        ],
    }
}

/// The values the sheet names, keyed by the name it gives them.
#[test]
fn the_sheet_names_the_values() {
    let values = super::oracle_game::sheet_values(&a_sheet());
    let obj = values.as_object().expect("an object of values");
    assert_eq!(obj.get("walk speed").and_then(|v| v.as_array()).map(|a| a.len()), Some(1));
    // A whole number is written without its fraction, so it matches what a self-test says.
    assert_eq!(obj.get("rifle damage").and_then(|v| v.as_array()).and_then(|a| a.first()).and_then(|v| v.as_str()), Some("7"));
}

/// A build that measured the sheet's numbers is not caught.
#[test]
fn a_build_that_measured_the_sheets_numbers_is_not_caught() {
    let script = super::oracle_game::script(super::oracle_game::sheet_values(&a_sheet()), json!({"sub_1": ["4.5"]}));
    // The build's report says the same numbers, in the sheet's words.
    let report = json!({"steps": [
        {"step": "walk speed", "passed": true, "detail": "walked 4.5 m"},
        {"step": "rifle damage", "passed": true, "detail": "did 7 damage"}
    ]});
    let build = vec![craft_anything_engine::oracle::Observation {
        oracle: super::oracle_game::ORACLE_ID.into(),
        version: 1,
        readings: vec![Some(craft_anything_engine::oracle::Reading { value: super::oracle_game::measured_values(&report), engine: None })],
        gaps: vec![],
    }];
    let differences = craft_anything_engine::oracle::compare(&script, &build);
    assert!(differences.iter().all(|d| !d.is_failure()), "the sheet's numbers must match: {differences:?}");
}

/// A build that measured different numbers is caught — the sheet's names are the vocabulary, so
/// the difference is found rather than every function reported as missing.
#[test]
fn a_build_that_measured_different_numbers_than_the_sheet_is_caught() {
    let script = super::oracle_game::script(super::oracle_game::sheet_values(&a_sheet()), json!({"sub_1": ["4.5"]}));
    let report = json!({"steps": [
        {"step": "walk speed", "passed": true, "detail": "walked 9.9 m"},
        {"step": "rifle damage", "passed": true, "detail": "did 7 damage"}
    ]});
    let build = vec![craft_anything_engine::oracle::Observation {
        oracle: super::oracle_game::ORACLE_ID.into(),
        version: 1,
        readings: vec![Some(craft_anything_engine::oracle::Reading { value: super::oracle_game::measured_values(&report), engine: None })],
        gaps: vec![],
    }];
    let differences = craft_anything_engine::oracle::compare(&script, &build);
    let failures: Vec<_> = differences.iter().filter(|d| d.is_failure()).collect();
    assert!(!failures.is_empty(), "a different number must be caught: {differences:?}");
    // And it names the value, not the original's function.
    assert!(failures[0].detail.contains("walk speed"), "{}", failures[0].detail);
}

/// The original's function names are not compared against the build's step names: they are
/// different vocabularies, and comparing them would report every function as missing.
#[test]
fn the_originals_function_names_are_not_compared_against_step_names() {
    let script = super::oracle_game::script(super::oracle_game::sheet_values(&a_sheet()), json!({"sub_1": ["4.5"], "sub_2": ["2"]}));
    let report = json!({"steps": [{"step": "walk speed", "passed": true, "detail": "walked 4.5 m"}]});
    let build = vec![craft_anything_engine::oracle::Observation {
        oracle: super::oracle_game::ORACLE_ID.into(),
        version: 1,
        readings: vec![Some(craft_anything_engine::oracle::Reading { value: super::oracle_game::measured_values(&report), engine: None })],
        gaps: vec![],
    }];
    let differences = craft_anything_engine::oracle::compare(&script, &build);
    let names: Vec<&str> = differences.iter().map(|d| d.detail.as_str()).collect();
    assert!(!names.iter().any(|n| n.contains("sub_1")), "a function name is not a value: {names:?}");
}

/// The end-to-end shape: a sheet, the original's code, and a build's report.
///
/// The three speak one vocabulary — the sheet's names — so a build that measures the sheet's
/// numbers is alike and one that does not is not.
#[test]
fn the_sheet_the_original_and_the_build_speak_one_vocabulary() {
    // The sheet: what the values are called and what they are.
    let sheet = a_sheet();
    let values = super::oracle_game::sheet_values(&sheet);
    assert!(values.get("walk speed").is_some(), "the sheet names its values: {values:?}");

    // The original's code: numbers, under function names that say nothing about what they are.
    let constants = json!({"sub_1000": ["4.5", "7"], "sub_2000": ["60"]});
    let script = super::oracle_game::script(values, constants);

    // The build's report: steps named after the sheet's values.
    let good = json!({"steps": [
        {"step": "walk speed", "passed": true, "detail": "walked 4.5 m"},
        {"step": "rifle damage", "passed": true, "detail": "did 7 damage"}
    ]});
    let bad = json!({"steps": [
        {"step": "walk speed", "passed": true, "detail": "walked 9.9 m"},
        {"step": "rifle damage", "passed": true, "detail": "did 7 damage"}
    ]});

    let observation = |report: &serde_json::Value| {
        vec![craft_anything_engine::oracle::Observation {
            oracle: super::oracle_game::ORACLE_ID.into(),
            version: 1,
            readings: vec![Some(craft_anything_engine::oracle::Reading { value: super::oracle_game::measured_values(report), engine: None })],
            gaps: vec![],
        }]
    };

    // The build that measures the sheet's numbers is alike.
    let differences = craft_anything_engine::oracle::compare(&script, &observation(&good));
    assert!(differences.iter().all(|d| !d.is_failure()), "{differences:?}");

    // The build that measures another number is not, and the difference names the value.
    let differences = craft_anything_engine::oracle::compare(&script, &observation(&bad));
    let failures: Vec<_> = differences.iter().filter(|d| d.is_failure()).collect();
    assert_eq!(failures.len(), 1, "{differences:?}");
    assert!(failures[0].detail.contains("walk speed"), "{}", failures[0].detail);
    assert!(failures[0].detail.contains("4.5") && failures[0].detail.contains("9.9"), "{}", failures[0].detail);
}
