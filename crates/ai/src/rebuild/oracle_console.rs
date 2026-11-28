//! The console behaviour oracle: run the original with a scripted timeline, keep what it printed
//! and how it exited, then run the same timeline against the build and compare.
//!
//! This is the first oracle, and the cheapest to trust: the observation is a terminal screen and
//! an exit code, the engine already normalises what changes every run (process ids, ports, time
//! buckets), and two runs of the same program compare equal — which the tests in this file check
//! before anything else.
//!
//! The original's side is recorded in Study and kept with the project, so the build is judged
//! against what the original actually did, not against the build's own account of itself.

use craft_anything_engine::evidence::ToolError;
use craft_anything_engine::oracle::{BehaviorOracle, Needs, Observation, OracleCx, Probe, ProbeKind, Reading, SCRIPT_VERSION, Script, TheProgram};
use craft_anything_engine::tool::ToolOutput;
use craft_anything_engine::workspace::{Claim, TargetRef};
use serde_json::{Value, json};

use super::CheckResult;
use super::kinds::runs::{RUN_MS, original_name};
use super::kinds::{Check, CheckCx};
/// The oracle's id, stored with the script so a build is compared the same way.
pub const ORACLE_ID: &str = "console";

/// How long one run may take.
const RUN_MS_LIMIT: u64 = RUN_MS;

/// The probes a console run is read through.
///
/// `exit_code` and `terminal` are what a person would check after running a program: how it
/// ended, and what it printed. Both are compared as text, because a program that prints the same
/// words in another order has behaved the same way.
fn probes() -> Vec<Probe> {
    vec![
        Probe { name: "exit_code".into(), kind: ProbeKind::Exact, tolerance: None },
        Probe { name: "terminal".into(), kind: ProbeKind::Words, tolerance: Some(80.0) },
    ]
}

/// The readings of one run, in the order of [`probes`].
pub fn readings(result: &Value, names: &[String]) -> Vec<Option<Reading>> {
    vec![
        result.pointer("/exit/code").cloned().map(|value| Reading { value, engine: None }),
        Some(Reading { value: Value::String(terminal_text(result, names)), engine: None }),
    ]
}

/// The script for a program that was run with these steps: the steps, and what the original was
/// seen to do.
pub fn script(steps: Vec<Value>, taken: Vec<Observation>) -> Script {
    Script { version: SCRIPT_VERSION, oracle: ORACLE_ID.into(), steps, probes: probes(), original: taken }
}

/// The name of the program, as both sides name it, so a usage line naming the rebuild still
/// matches the original's.
pub fn program_names(original: &std::path::Path, build: &std::path::Path) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(name) = original.file_stem().map(|s| s.to_string_lossy().into_owned()) {
        names.push(name);
    }
    if let Some(name) = build.file_stem().map(|s| s.to_string_lossy().into_owned()) {
        names.push(name);
    }
    names
}

/// The text of a run's terminal, with both programs' names replaced by one word.
fn terminal_text(result: &Value, names: &[String]) -> String {
    let lines = result.pointer("/terminal/lines").and_then(Value::as_array).cloned().unwrap_or_default();
    let text = lines.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n");
    let mut out = text;
    for name in names {
        if !name.is_empty() {
            out = out.replace(name, "program");
        }
    }
    out
}

/// Run the recorded script against the build and read the same probes back.
pub fn replay_build(cx: &OracleCx, script: &Script, build: &std::path::Path) -> Result<Vec<Observation>, ToolError> {
    let mut taken = Vec::new();
    let mut gaps = Vec::new();
    // The original's name, as its printed text may contain it.
    let names = {
        let mut names = vec![original_name(cx.target, cx.deep)];
        if let Some(name) = build.file_stem().map(|s| s.to_string_lossy().into_owned()) {
            names.push(name);
        }
        names
    };

    for step in &script.steps {
        let case = step.get("case").and_then(Value::as_str).unwrap_or("the program");
        let args: Vec<String> =
            step.get("arguments").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
        let mut params = json!({
            "executable": build.display().to_string(),
            "arguments": args,
            "timeout_ms": RUN_MS_LIMIT,
            "idle_timeout_ms": RUN_MS_LIMIT,
            "settle_ms": 200,
        });
        if let Some(cwd) = build.parent() {
            params["working_directory"] = json!(cwd.display().to_string());
        }
        let out = match (cx.call)("runtime_walkthrough_record", params) {
            Ok(out) => out,
            Err(e) => {
                gaps.push(format!("the build couldn't be run with {case}: {}", e.message));
                // An observation with no readings: the comparison knows the step could not be
                // read, rather than that it read nothing.
                taken.push(Observation { oracle: ORACLE_ID.into(), version: SCRIPT_VERSION, readings: vec![None, None], gaps: vec![e.message] });
                continue;
            }
        };
        let result = out.to_json().get("result").cloned().unwrap_or(Value::Null);
        taken.push(Observation { oracle: ORACLE_ID.into(), version: SCRIPT_VERSION, readings: readings(&result, &names), gaps: Vec::new() });
    }

    Ok(taken)
}

/// The console behaviour check: the build is run with the original's timeline and the two are
/// compared.
pub struct ConsoleBehaviourCheck;

impl Check for ConsoleBehaviourCheck {
    fn id(&self) -> &'static str {
        "console-behaviour"
    }

    fn applies(&self, cx: &CheckCx) -> bool {
        cx.ws.manifest.script.is_some()
    }

    fn run(&self, cx: &mut CheckCx) -> Result<CheckResult, ToolError> {
        let Some(script) = cx.ws.manifest.script.as_ref() else {
            return Ok(CheckResult::new(self.id(), cx.round, false, "There's no recorded behaviour to compare with."));
        };
        let godot = cx.godot();
        let Some((program, _base)) = craft_anything_engine::outputs::launch_now(&cx.ws.app_dir(), godot.as_deref()) else {
            // A build with no program to run is a claim the builder can act on, not a check that
            // could not run: it is told what is missing.
            return Ok(CheckResult {
                claims: vec![Claim::new(
                    "The build runs like the original",
                    false,
                    "The build has no program to run, so it can't be compared with the original.",
                )],
                ..CheckResult::new(self.id(), cx.round, true, "The build has no program to run.")
            });
        };
        let program = craft_anything_engine::outputs::absolute_program(&program);
        // The oracle that recorded the original is the one that judges the build.
        let oracles = crate::rebuild::oracles::Oracles::built_in();
        let Some(oracle) = oracles.by_id(ORACLE_ID) else {
            return Ok(CheckResult::new(self.id(), cx.round, false, "There's no console oracle to compare with."));
        };
        let call = |tool: &str, params: Value| cx.call(tool, params);
        let held = TheProgram(program.clone());
        let context = check_context(cx, &call, &held);
        let observation = oracle.replay(&context, script)?;
        let differences = oracle.compare(script, &observation);
        let claims: Vec<Claim> = differences.iter().filter(|d| d.is_failure()).map(|d| Claim::new(d.title(), false, d.detail.clone())).collect();
        let unknowns: Vec<String> = differences.iter().filter(|d| !d.is_failure()).map(|d| d.detail.clone()).collect();
        let mut all = unknowns.clone();
        all.extend(claims.iter().map(|c| format!("{} ({})", c.title, c.detail)));
        let note = if all.is_empty() {
            "It runs like the original with every input.".to_string()
        } else {
            format!("{} of the original's behaviours don't hold yet: {}", claims.len(), all.join("; "))
        };
        Ok(CheckResult { claims, unknowns, ..CheckResult::new(self.id(), cx.round, true, note) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_programs_names_become_one_word() {
        let names = vec!["original".to_string(), "rebuild".to_string()];
        let text = terminal_text(&json!({"terminal": {"lines": ["original 1.0", "rebuild 2.0"]}}), &names);
        assert_eq!(text, "program 1.0\nprogram 2.0");
    }

    #[test]
    fn the_probes_are_the_exit_code_and_what_was_printed() {
        let p = probes();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].name, "exit_code");
        assert_eq!(p[0].kind, ProbeKind::Exact);
        assert_eq!(p[1].name, "terminal");
        assert_eq!(p[1].kind, ProbeKind::Words);
    }

    #[test]
    fn a_run_is_read_through_the_same_two_probes() {
        let result = json!({"exit": {"code": 0}, "terminal": {"lines": ["hello", "world"]}});
        let r = readings(&result, &["ls".to_string()]);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].as_ref().unwrap().value, json!(0));
        // The terminal is read as one piece of text, so the two sides compare by words.
        assert_eq!(r[1].as_ref().unwrap().value, json!("hello\nworld"));
    }
}

/// The console oracle: a program run with the same inputs, compared by what it printed and how
/// it exited.
pub struct ConsoleOracle;

impl BehaviorOracle for ConsoleOracle {
    fn id(&self) -> &'static str {
        ORACLE_ID
    }

    fn supports(&self, target: &TargetRef) -> bool {
        matches!(target, TargetRef::Local { .. })
    }

    /// The build is a program to start.
    fn needs(&self) -> Needs {
        Needs::program()
    }

    /// Record the original's behaviour: run it with each input and keep what it printed.
    ///
    /// The recording is the same pass that captures the runs for the study's notes, so the
    /// original is not run twice.
    fn record(&self, cx: &OracleCx) -> Result<Option<Script>, ToolError> {
        let TargetRef::Local { path } = cx.target else { return Ok(None) };
        let program = path.clone();
        let mut steps = Vec::new();
        let mut taken = Vec::new();
        for (case, args) in super::kinds::runs::CASES {
            let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
            let mut params = json!({
                "executable": program.display().to_string(),
                "arguments": args,
                "timeout_ms": RUN_MS_LIMIT,
                "idle_timeout_ms": RUN_MS_LIMIT,
                "settle_ms": 200,
            });
            if let Some(cwd) = program.parent() {
                params["working_directory"] = json!(cwd.display().to_string());
            }
            let Ok(out) = (cx.call)("runtime_walkthrough_record", params) else { continue };
            let result = out.to_json().get("result").cloned().unwrap_or(Value::Null);
            steps.push(json!({"case": case, "arguments": args, "step_id": case}));
            taken.push(Observation {
                oracle: ORACLE_ID.into(),
                version: SCRIPT_VERSION,
                readings: readings(&result, &program_names(&program, &program)),
                gaps: Vec::new(),
            });
        }
        if steps.is_empty() {
            return Ok(None);
        }
        Ok(Some(script(steps, taken)))
    }

    fn replay(&self, cx: &OracleCx, script: &Script) -> Result<Vec<Observation>, ToolError> {
        let program = cx.program.ok_or_else(|| ToolError::invalid("There's no build to compare."))?;
        replay_build(cx, script, program.program())
    }
}

/// A context for an oracle, from a check's context.
///
/// The oracle reaches the engine through the check's own call, so it inherits the check's time
/// limit and the person's pause. The build is a program to start.
///
/// The caller owns the closure and the program holder, so both outlive the context.
pub fn check_context<'a>(cx: &'a CheckCx, call: &'a dyn Fn(&str, Value) -> Result<ToolOutput, ToolError>, program: &'a TheProgram) -> OracleCx<'a> {
    OracleCx::original(call, &cx.ws.manifest.target, cx.deep()).with_program(program)
}

/// A context for recording the original's side: no build yet, only the original.
pub fn original_context<'a>(call: &'a dyn Fn(&str, Value) -> Result<ToolOutput, ToolError>, target: &'a TargetRef, deep: bool) -> OracleCx<'a> {
    OracleCx::original(call, target, deep)
}
