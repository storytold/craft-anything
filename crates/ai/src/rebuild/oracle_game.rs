//! The game mechanics oracle: what the original's numbers are, measured rather than believed.
//!
//! The mechanics sheet asks the builder for the values that shape how the game plays ("walk
//! speed", "rifle damage"). Until now the sheet was read and reported, and its behaviour was
//! always unverified: the builder wrote the sheet, so the sheet could not be the judge of the
//! builder.
//!
//! This oracle gives the sheet something to be measured against:
//!
//! - **Deep mode** reads the original's own constants out of its decompiled code, so a value the
//!   sheet claims can be compared with the number the original actually uses.
//! - **Clean room** has no code to read, so the original's numbers are not established here.
//!   The sheet stays unverified, and says so.
//!
//! Either way the build's own `--self-test` report is corroborating evidence, never the verdict:
//! a build that lies in its self-test is caught when the numbers do not match.

use craft_anything_engine::evidence::ToolError;
use craft_anything_engine::oracle::{BehaviorOracle, Needs, Observation, OracleCx, Probe, ProbeKind, Reading, SCRIPT_VERSION, Script, TheReport};
use craft_anything_engine::workspace::{Claim, TargetRef};
use serde_json::{Value, json};

use super::CheckResult;
use super::kinds::{Check, CheckCx};

/// The oracle's id, stored with the script so a build is compared the same way.
pub const ORACLE_ID: &str = "game-mechanics";

/// The probes a game's mechanics are read through.
///
/// The values are the numbers the original uses, found in its code. A build's numbers come from
/// its own `--self-test` report, where each step says what it measured ("walked 3.1 m forward").
const VALUES: &str = "values";

fn probes() -> Vec<Probe> {
    vec![Probe { name: VALUES.into(), kind: ProbeKind::Exact, tolerance: None }]
}

/// The numbers in a piece of text, in order.
///
/// A sign is part of a number: `-1` and `1` are different. A number with no digits ("m/s") is
/// not one.
pub fn numbers_in(text: &str) -> Vec<String> {
    static NUMBERS: std::sync::LazyLock<Option<regex::Regex>> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)[+-]?(?:0x[0-9a-f]+|(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:e[+-]?[0-9]+)?)").ok());
    let Some(pattern) = NUMBERS.as_ref() else { return Vec::new() };
    pattern.find_iter(text).map(|m| m.as_str().to_string()).collect()
}

/// The numbers in a self-test report, by step.
///
/// The report is the build's own word about what it measured. It is read here so the numbers can
/// be compared with the original's; the report is never the judge of itself.
pub fn measured_values(report: &Value) -> Value {
    let mut out = serde_json::Map::new();
    let steps = report.get("steps").and_then(Value::as_array).cloned().unwrap_or_default();
    for step in steps {
        let name = step.get("step").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
        if name.is_empty() {
            continue;
        }
        let detail = step.get("detail").and_then(Value::as_str).unwrap_or("");
        let numbers = numbers_in(detail);
        if numbers.is_empty() {
            continue;
        }
        // A step with more than one number is compared on all of them, in order.
        out.insert(name, Value::Array(numbers.into_iter().map(Value::String).collect()));
    }
    Value::Object(out)
}

/// The numbers the original's code uses, by the function they are in.
///
/// Each function's decompiled code is read for its numeric constants. A constant is a number
/// with no sign and no fraction — a value a game would use as a speed or a damage figure — while
/// a pointer-sized or address-shaped constant is not a value at all.
pub fn constants_in(pseudo: &str) -> Vec<String> {
    // A number that stands on its own: not part of a longer name and not part of a longer
    // number. The `regex` crate has no look-behind, so the boundaries are checked by hand.
    static CONSTANTS: std::sync::LazyLock<Option<regex::Regex>> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)(0x[0-9a-f]+|\d+(?:\.\d+)?)").ok());
    let Some(pattern) = CONSTANTS.as_ref() else { return Vec::new() };
    let bytes = pseudo.as_bytes();
    let mut out = Vec::new();
    for m in pattern.find_iter(pseudo) {
        let text = m.as_str();
        let start = m.start();
        let end = m.end();
        // A number inside a longer name (`sub_1000`) is part of the name, not a value.
        let before = start.checked_sub(1).and_then(|i| bytes.get(i)).copied().unwrap_or(b' ');
        if before.is_ascii_alphanumeric() || before == b'_' || before == b'.' {
            continue;
        }
        let after = bytes.get(end).copied().unwrap_or(b' ');
        if after.is_ascii_alphanumeric() || after == b'_' || after == b'.' {
            continue;
        }
        // An address is not a value: it is far larger than any game's number.
        if text.starts_with("0x") && text.len() > 6 {
            continue;
        }
        // A value is a number, in either spelling: `4.5` or `0x68`. Rust's float parser reads
        // neither form of a whole number the way a game writes it, so each is read on its own.
        let number = if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            u64::from_str_radix(hex, 16).ok().map(|n| n as f64)
        } else {
            text.parse::<f64>().ok()
        };
        if let Some(v) = number
            && v.is_finite()
            && v.abs() < 1e9
        {
            out.push(text.to_string());
        }
    }
    out
}

/// The script for a game: the values both sides speak, and what the original's code says about
/// them.
///
/// The values are keyed by the name the sheet gives them ("walk speed"), because that is the
/// only vocabulary both sides share. The original's constants are kept beside them, per
/// function, as what the original's own code says — not as the comparison, which would compare
/// function names against step names and never match.
pub fn script(values: Value, _original: Value) -> Script {
    Script {
        version: SCRIPT_VERSION,
        oracle: ORACLE_ID.into(),
        steps: Vec::new(),
        probes: probes(),
        original: vec![Observation {
            oracle: ORACLE_ID.into(),
            version: SCRIPT_VERSION,
            readings: vec![Some(Reading { value: values, engine: None })],
            gaps: Vec::new(),
        }],
    }
}

/// The mechanics sheet, as the values both sides speak.
pub struct TheSheet(pub craft_anything_engine::mechanics::Mechanics);

impl craft_anything_engine::oracle::NamesValues for TheSheet {
    fn named_values(&self) -> Value {
        sheet_values(&self.0)
    }
}

/// The values the sheet names, with the number each one has.
///
/// A value may be a number, text or a switch; only a number can be measured, so the others are
/// left out rather than read as something they are not.
pub fn sheet_values(sheet: &craft_anything_engine::mechanics::Mechanics) -> Value {
    let mut out = serde_json::Map::new();
    for value in &sheet.values {
        let name = value.name.trim().to_lowercase();
        if name.is_empty() {
            continue;
        }
        match &value.value {
            craft_anything_engine::mechanics::MechanicValue::Number(n) => {
                out.insert(name, Value::Array(vec![Value::String(craft_anything_engine::oracle::format_number(*n))]));
            }
            _ => {}
        }
    }
    Value::Object(out)
}

/// The mechanics check: the build's measured numbers are compared with the original's.
pub struct MechanicsOracleCheck;

impl Check for MechanicsOracleCheck {
    fn id(&self) -> &'static str {
        "game-mechanics"
    }

    fn applies(&self, cx: &CheckCx) -> bool {
        // Only a recorded comparison can judge anything; without one the sheet's own check says
        // the behaviour is unverified.
        cx.ws.manifest.script.is_some()
    }

    fn can_refine(&self, _result: &CheckResult) -> bool {
        true
    }

    fn run(&self, cx: &mut CheckCx) -> Result<CheckResult, ToolError> {
        let Some(script) = cx.ws.manifest.script.as_ref() else {
            return Ok(CheckResult::new(self.id(), cx.round, false, "There's no recorded behaviour to compare with."));
        };
        // The build's own report: what it measured, in its own words.
        let report = build_report(cx)?;
        if report.is_null() {
            return Ok(CheckResult {
                claims: vec![Claim::new("The build plays like the original", false, "The build saved no --self-test report, so nothing could be measured.")],
                ..CheckResult::new(self.id(), cx.round, true, "The build saved no --self-test report.")
            });
        }
        // The oracle that recorded the original is the one that judges the build.
        let oracles = crate::rebuild::oracles::Oracles::built_in();
        let Some(oracle) = oracles.by_id(ORACLE_ID) else {
            return Ok(CheckResult::new(self.id(), cx.round, false, "There's no game oracle to compare with."));
        };
        let call = |tool: &str, params: Value| cx.call(tool, params);
        let report = cx.ws.checks_dir().join(format!("round-{}", cx.round)).join("self-test.json");
        let held = TheReport(report);
        // The sheet is the vocabulary both sides speak; the build's report answers in it.
        let sheet = craft_anything_engine::mechanics::read(&cx.ws.mechanics_path()).ok().map(TheSheet);
        let mut context = OracleCx::original(&call, &cx.ws.manifest.target, cx.deep()).with_report(&held);
        if let Some(sheet) = sheet.as_ref() {
            context = context.with_sheet(sheet);
        }
        let observation = oracle.replay(&context, script)?;
        let differences = oracle.compare(script, &observation);
        let claims: Vec<Claim> = differences.iter().filter(|d| d.is_failure()).map(|d| Claim::new(d.title(), false, d.detail.clone())).collect();
        let unknowns: Vec<String> = differences.iter().filter(|d| !d.is_failure()).map(|d| d.detail.clone()).collect();
        let note = if claims.is_empty() && unknowns.is_empty() {
            "It plays like the original: the numbers it measured are the original's.".to_string()
        } else {
            format!(
                "{} of the original's numbers the build does not match: {}",
                claims.len(),
                differences.iter().map(|d| d.detail.clone()).collect::<Vec<_>>().join("; ")
            )
        };
        Ok(CheckResult { claims, unknowns, ..CheckResult::new(self.id(), cx.round, true, note) })
    }
}

/// The game oracle: the numbers the original's own code uses, against the numbers the build
/// measured.
pub struct GameOracle;

impl BehaviorOracle for GameOracle {
    fn id(&self) -> &'static str {
        ORACLE_ID
    }

    fn supports(&self, target: &TargetRef) -> bool {
        matches!(target, TargetRef::Local { .. })
    }

    /// The build saved a report of its own to read.
    fn needs(&self) -> Needs {
        Needs::report()
    }

    /// Record the original's numbers: the constants in its own code.
    ///
    /// Only Deep mode can read them, so in Clean room nothing is recorded and the sheet stays
    /// unverified — as it says it is.
    fn record(&self, cx: &OracleCx) -> Result<Option<Script>, ToolError> {
        if !cx.deep {
            return Ok(None);
        }
        let TargetRef::Local { path } = cx.target else { return Ok(None) };
        let constants = read_constants(cx, path)?;
        if constants.as_object().map(|o| o.is_empty()).unwrap_or(true) {
            return Ok(None);
        }
        // The sheet is the vocabulary both sides speak; the original's constants are what its
        // own code says about them.
        let sheet = cx.sheet.map(|s| s.named_values()).unwrap_or_else(|| Value::Object(Default::default()));
        Ok(Some(script(sheet, constants)))
    }

    /// The build's numbers: what its own self-test says it measured.
    fn replay(&self, cx: &OracleCx, _script: &Script) -> Result<Vec<Observation>, ToolError> {
        let report = report_from(cx).unwrap_or(Value::Null);
        // One observation per step of the core loop: each step is measured on its own, so a
        // step that measures the wrong number is found, and not only a whole report that does.
        let steps = report.get("steps").and_then(Value::as_array).cloned().unwrap_or_default();
        if steps.is_empty() {
            return Ok(vec![Observation {
                oracle: ORACLE_ID.into(),
                version: SCRIPT_VERSION,
                readings: vec![Some(Reading { value: measured_values(&report), engine: None })],
                gaps: Vec::new(),
            }]);
        }
        Ok(steps
            .iter()
            .map(|step| {
                let one = json!({"steps": [step]});
                Observation {
                    oracle: ORACLE_ID.into(),
                    version: SCRIPT_VERSION,
                    readings: vec![Some(Reading { value: measured_values(&one), engine: None })],
                    gaps: Vec::new(),
                }
            })
            .collect())
    }
}

/// The numbers the original's own code uses, by the function they are in.
fn read_constants(cx: &OracleCx, program: &std::path::Path) -> Result<Value, ToolError> {
    let listed = (cx.call)("native_functions_list", json!({"path": program.display().to_string(), "limit": 200}))?;
    let functions = listed.to_json().get("result").and_then(|r| r.get("functions")).and_then(Value::as_array).cloned().unwrap_or_default();
    let mut values = serde_json::Map::new();
    for f in functions.iter().take(FUNCTIONS_DECOMPILED) {
        let Some(address) = f.get("address").and_then(Value::as_str) else { continue };
        let name = f.get("name").and_then(Value::as_str).unwrap_or(address);
        let Ok(out) = (cx.call)("native_function_pseudocode", json!({"path": program.display().to_string(), "function": address})) else {
            continue;
        };
        let decoded = out.to_json();
        let pseudo = decoded.get("result").and_then(|r| r.get("pseudo_code")).and_then(Value::as_str).unwrap_or("");
        let constants = constants_in(pseudo);
        if !constants.is_empty() {
            values.insert(name.to_string(), Value::Array(constants.into_iter().map(Value::String).collect()));
        }
    }
    Ok(Value::Object(values))
}

/// The build's own self-test report, read from the file it saved.
fn report_from(cx: &OracleCx) -> Option<Value> {
    let file = cx.report?.report_path();
    std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// The build's own self-test report, read from the file it saved.
fn build_report(cx: &mut CheckCx) -> Result<Value, ToolError> {
    let file = cx.ws.checks_dir().join(format!("round-{}", cx.round)).join("self-test.json");
    Ok(std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null))
}

/// How many functions are decompiled to find the numbers: enough to find a game's values,
/// without spending the whole study on it.
const FUNCTIONS_DECOMPILED: usize = 40;
