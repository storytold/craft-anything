//! The native behaviour oracle: a program watched from outside, and compared.
//!
//! A windowed program (an app or a game) has no page and no report. What it has is a window:
//! the original's window can be read and acted on from outside, and so can the build's. The
//! oracle starts each, reads the same controls, acts on the same ones, and compares what each
//! showed.
//!
//! This is the one oracle that needs a new capability — `DrivesAnApp` — because starting a
//! program and leaving it running is its own thing. The other three oracles are untouched by it.
//!
//! Everything is read from outside: the controls' roles, titles and values, never the program's
//! code. So this oracle works in Clean room too, which is where a person watching a program
//! would be.

use craft_anything_engine::evidence::ToolError;
use craft_anything_engine::oracle::{BehaviorOracle, Needs, Observation, OracleCx, Probe, ProbeKind, Reading, SCRIPT_VERSION, Script};
use craft_anything_engine::workspace::{Claim, TargetRef};
use serde_json::{Value, json};

use super::CheckResult;
use super::kinds::{Check, CheckCx};

/// The oracle's id, stored with the script so a build is compared the same way.
pub const ORACLE_ID: &str = "native";

/// The probes a native run is read through.
///
/// The controls the program shows, by role and title: what a person would read off the screen.
const CONTROLS: &str = "controls";

fn probes() -> Vec<Probe> {
    vec![Probe { name: CONTROLS.into(), kind: ProbeKind::Fields, tolerance: None }]
}

/// The controls a UI tree shows, read as labelled values.
///
/// Each value is a role and a title, so two programs that show the same controls in a different
/// order behave the same way. A control's value is kept as well, when it has one: a button's
/// title says what it is, but a field's value is what it holds.
pub fn controls_of(tree: &Value) -> Value {
    let nodes = tree.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut fields = serde_json::Map::new();
    for node in nodes {
        let role = node.get("role").and_then(Value::as_str).unwrap_or("");
        let title = node.get("title").and_then(Value::as_str).unwrap_or("");
        let identifier = node.get("identifier").and_then(Value::as_str).unwrap_or("");
        if role.is_empty() && title.is_empty() && identifier.is_empty() {
            continue;
        }
        // The role and the title together name a control: the same title under two roles is two
        // controls. A control with no title is named by its identifier, which is how it is found
        // again.
        let key = if !title.is_empty() {
            format!("{role}: {title}")
        } else if !identifier.is_empty() {
            format!("{role}: {identifier}")
        } else {
            role.to_string()
        };
        let value = node.get("value").and_then(Value::as_str).unwrap_or("");
        fields.entry(key).or_insert(Value::String(value.to_string()));
    }
    Value::Object(fields)
}

/// The readings of one run, in the order of [`probes`].
pub fn readings(tree: &Value) -> Vec<Option<Reading>> {
    vec![Some(Reading { value: controls_of(tree), engine: None })]
}

/// The script for a program whose controls were read and acted on: one observation per step.
pub fn script(steps: Vec<Value>, original: Vec<Observation>) -> Script {
    Script { version: SCRIPT_VERSION, oracle: ORACLE_ID.into(), steps, probes: probes(), original }
}

/// The native oracle: a program watched from outside, and compared.
pub struct NativeOracle;

impl BehaviorOracle for NativeOracle {
    fn id(&self) -> &'static str {
        ORACLE_ID
    }

    fn supports(&self, target: &TargetRef) -> bool {
        matches!(target, TargetRef::Local { .. })
    }

    /// The build is a program to start and watch.
    fn needs(&self) -> Needs {
        Needs::app()
    }

    /// Record the original's controls: start it, read its window, close it.
    fn record(&self, cx: &OracleCx) -> Result<Option<Script>, ToolError> {
        let TargetRef::Local { path } = cx.target else { return Ok(None) };
        let program = path.clone();
        let started = start_program(cx, &program, &[])?;
        let first = read_tree(cx, started.pid)?;
        let controls = controls_of(&first);
        if controls.as_object().map(|o| o.is_empty()).unwrap_or(true) {
            end_program(cx, &started.session_id)?;
            return Ok(None);
        }
        // The steps are the controls the program showed, grouped by the container they are in:
        // a panel's fields and its button are one group. The fields are filled before the
        // buttons are pressed, because a control that depends on what was typed only shows its
        // effect when the field is filled first.
        let steps: Vec<Value> = groups_of(&first).into_iter().flatten().collect();
        // Each step is run on its own, and what the program showed afterwards is kept: a sequence's
        // value is in what each step changed, not only in where it ended up.
        let mut observations = vec![observation(&first)];
        for step in steps.iter() {
            let title = step.get("title").and_then(Value::as_str).unwrap_or("");
            if title.is_empty() {
                continue;
            }
            let acted = match step.get("action").and_then(Value::as_str) {
                Some("fill") => fill(cx, started.pid, title),
                _ => walkthrough(cx, started.pid, title),
            };
            let after = read_tree(cx, started.pid)?;
            observations.push(observation(&after));
            // A control that could not be acted on is recorded: the build must fail it the same
            // way.
            if !acted {
                let last = observations.last_mut().expect("an observation");
                last.gaps.push(format!("the original's {title} could not be acted on"));
            }
        }
        end_program(cx, &started.session_id)?;
        Ok(Some(script(steps, observations)))
    }

    /// Start the build, read the same controls, and compare.
    fn replay(&self, cx: &OracleCx, script: &Script) -> Result<Vec<Observation>, ToolError> {
        let app = cx.app.ok_or_else(|| ToolError::invalid("There's no program to watch."))?;
        let started = start_program(cx, app.app_program(), app.app_args())?;
        let first = read_tree(cx, started.pid)?;
        // The same sequence, step by step, so what each step changed is compared.
        let mut observations = vec![observation(&first)];
        for step in &script.steps {
            let title = step.get("title").and_then(Value::as_str).unwrap_or("");
            if title.is_empty() {
                continue;
            }
            let acted = match step.get("action").and_then(Value::as_str) {
                Some("fill") => fill(cx, started.pid, title),
                _ => walkthrough(cx, started.pid, title),
            };
            let after = read_tree(cx, started.pid)?;
            observations.push(observation(&after));
            if !acted {
                observations.last_mut().expect("an observation").gaps.push(format!("the build's {title} could not be acted on"));
            }
        }
        end_program(cx, &started.session_id)?;
        Ok(observations)
    }
}

/// A program that was started and is running.
struct Started {
    session_id: String,
    pid: u32,
}

/// Start a program and leave it running.
fn start_program(cx: &OracleCx, program: &std::path::Path, args: &[String]) -> Result<Started, ToolError> {
    let mut params = json!({
        "program": program.display().to_string(),
        "args": args,
    });
    // The program is started from its own folder, so it finds what it keeps beside it.
    if let Some(dir) = program.parent() {
        params["working_directory"] = json!(dir.display().to_string());
    }
    let out = (cx.call)("native_program_session_begin", params)?;
    let result = out.to_json().get("result").cloned().unwrap_or(Value::Null);
    let session_id = result.get("session_id").and_then(Value::as_str).ok_or_else(|| ToolError::internal("The program session has no id."))?.to_string();
    let pid = result.get("pid").and_then(Value::as_u64).ok_or_else(|| ToolError::internal("The program session has no process id."))? as u32;
    let started = Started { session_id, pid };
    // The program is running, but its window may not be there yet: a program takes a moment to
    // draw, and a window read too early finds nothing. The oracle waits for the window, so a
    // program that draws slowly is still read.
    wait_for_window(cx, started.pid)?;
    Ok(started)
}

/// Wait until the program's window shows something, or the wait is over.
///
/// A program that shows no window at all — a console program, say — is not an error: the caller
/// reads the tree and finds nothing, which is what the program looks like.
fn wait_for_window(cx: &OracleCx, pid: u32) -> Result<(), ToolError> {
    let deadline = std::time::Instant::now() + WINDOW_WAIT;
    loop {
        if let Ok(tree) = read_tree(cx, pid) {
            let shown = tree.get("nodes").and_then(Value::as_array).map(|n| n.len()).unwrap_or(0);
            // The window itself is one node; anything more means it has drawn.
            if shown > 1 {
                return Ok(());
            }
        }
        if std::time::Instant::now() >= deadline {
            return Ok(());
        }
        std::thread::sleep(WINDOW_POLL);
    }
}

/// How long the oracle waits for a program's window to draw.
const WINDOW_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// How often the oracle looks while it waits.
const WINDOW_POLL: std::time::Duration = std::time::Duration::from_millis(250);

/// One observation of a window.
fn observation(tree: &Value) -> Observation {
    Observation { oracle: ORACLE_ID.into(), version: SCRIPT_VERSION, readings: readings(tree), gaps: Vec::new() }
}

/// The text a field is filled with when the original's own content is not known.
///
/// It is the same text the web oracle uses, so both sides are filled the same way.
pub const PROBE_TEXT: &str = "craft-anything";

/// Whether a control takes text: a field, a box or a combo.
pub fn takes_text(role: &str) -> bool {
    matches!(role, "AXTextField" | "AXTextArea" | "AXComboBox" | "AXSearchField" | "AXSecureTextField")
}

/// Put text into a field, and say whether it worked.
fn fill(cx: &OracleCx, pid: u32, title: &str) -> bool {
    let params = json!({
        "pid": pid,
        "steps": [{"action": "AXSetValue", "element": title, "value": PROBE_TEXT}],
        "duration_ms": 4000,
    });
    match (cx.call)("runtime_ui_walkthrough_record", params) {
        Ok(out) => {
            let result = out.to_json().get("result").cloned().unwrap_or(Value::Null);
            result
                .get("steps")
                .and_then(Value::as_array)
                .and_then(|s| s.first())
                .and_then(|s| s.get("outcome"))
                .and_then(Value::as_str)
                .is_some_and(|o| o == "dispatched")
        }
        Err(_) => false,
    }
}

/// Act on a control, and say whether it worked.
fn walkthrough(cx: &OracleCx, pid: u32, title: &str) -> bool {
    let params = json!({
        "pid": pid,
        "steps": [{"action": "AXPress", "element": title}],
        "duration_ms": 4000,
    });
    match (cx.call)("runtime_ui_walkthrough_record", params) {
        Ok(out) => {
            let result = out.to_json().get("result").cloned().unwrap_or(Value::Null);
            result
                .get("steps")
                .and_then(Value::as_array)
                .and_then(|s| s.first())
                .and_then(|s| s.get("outcome"))
                .and_then(Value::as_str)
                .is_some_and(|o| o == "dispatched")
        }
        Err(_) => false,
    }
}

/// Read a running program's window.
pub fn read_tree(cx: &OracleCx, pid: u32) -> Result<Value, ToolError> {
    let out = (cx.call)("runtime_ui_read", json!({"pid": pid, "max_nodes": 1000}))?;
    let result = out.to_json().get("result").cloned().unwrap_or(Value::Null);
    // A program with no window yet: the tree is empty, which the comparison reads as unreadable.
    Ok(result)
}

/// Close a program session.
fn end_program(cx: &OracleCx, session_id: &str) -> Result<(), ToolError> {
    let _ = (cx.call)("native_program_session_end", json!({"session_id": session_id}))?;
    Ok(())
}

/// The native behaviour check: the build is watched the way the original was, and the two are
/// compared.
pub struct NativeBehaviourCheck;

impl Check for NativeBehaviourCheck {
    fn id(&self) -> &'static str {
        "native-behaviour"
    }

    fn applies(&self, cx: &CheckCx) -> bool {
        // Only a recorded comparison can judge anything.
        cx.ws.manifest.script.is_some() && matches!(cx.ws.manifest.target, TargetRef::Local { .. })
    }

    fn run(&self, cx: &mut CheckCx) -> Result<CheckResult, ToolError> {
        let Some(script) = cx.ws.manifest.script.as_ref() else {
            return Ok(CheckResult::new(self.id(), cx.round, false, "There's no recorded behaviour to compare with."));
        };
        let Some(app) = super::output::built_program(&cx.ws.app_dir()) else {
            return Ok(CheckResult {
                claims: vec![Claim::new("The build shows what the original shows", false, "The build has no program to watch.")],
                ..CheckResult::new(self.id(), cx.round, true, "The build has no program to watch.")
            });
        };
        let oracle = NativeOracle;
        let call = |tool: &str, params: Value| cx.call(tool, params);
        let held = craft_anything_engine::oracle::TheApp { program: app.clone(), args: Vec::new() };
        let context = OracleCx::original(&call, &cx.ws.manifest.target, cx.deep()).with_app(&held);
        let observation = match oracle.replay(&context, script) {
            Ok(o) => o,
            Err(e) => {
                let detail = format!("The build's window could not be watched: {}", e.message);
                return Ok(CheckResult {
                    claims: vec![Claim::new("The build shows what the original shows", false, detail.clone())],
                    ..CheckResult::new(self.id(), cx.round, true, detail)
                });
            }
        };
        let mut claims = Vec::new();
        let mut unknowns = Vec::new();
        for difference in oracle.compare(script, &observation) {
            if difference.is_failure() {
                claims.push(Claim::new(difference.title(), false, difference.detail.clone()));
            } else {
                unknowns.push(difference.detail.clone());
            }
        }
        let note = if claims.is_empty() && unknowns.is_empty() {
            "It shows what the original shows.".to_string()
        } else {
            let mut parts = Vec::new();
            if !claims.is_empty() {
                parts.push(format!("{} of the original's controls the build does not show", claims.len()));
            }
            if !unknowns.is_empty() {
                parts.push(format!("{} could not be compared", unknowns.len()));
            }
            format!("{}.", parts.join(", "))
        };
        Ok(CheckResult { claims, unknowns, ..CheckResult::new(self.id(), cx.round, true, note) })
    }
}

/// The controls a window shows, grouped by the container they are in.
///
/// The accessibility tree says how deep each control sits, and the controls that share a
/// container belong together: a panel's fields and its button are one group, and a panel that
/// only appears once something else is chosen is a group of its own.
///
/// The groups are what the session is built from, so a control is acted on together with the
/// ones it sits beside it.
pub fn groups_of(tree: &Value) -> Vec<Vec<Value>> {
    let nodes = tree.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
    // The controls, in the order the tree lists them, each with the depth it sits at.
    let mut controls: Vec<(u64, Value)> = Vec::new();
    for node in &nodes {
        let role = node.get("role").and_then(Value::as_str).unwrap_or("");
        let title = node.get("title").and_then(Value::as_str).unwrap_or("");
        let identifier = node.get("identifier").and_then(Value::as_str).unwrap_or("");
        if !takes_text(role) && !is_pressed(role) {
            continue;
        }
        // A control with no title is still a control: it is named by its identifier.
        if title.is_empty() && identifier.is_empty() {
            continue;
        }
        let depth = node.get("depth").and_then(Value::as_u64).unwrap_or(0);
        // The element is named by its title, or by its identifier when it has none: that is how
        // the walkthrough finds it again.
        let identifier = node.get("identifier").and_then(Value::as_str).unwrap_or("");
        let element = if !title.is_empty() { title } else { identifier };
        let action = if takes_text(role) {
            json!({"action": "fill", "role": role, "title": element, "value": PROBE_TEXT})
        } else {
            json!({"action": "press", "role": role, "title": element})
        };
        controls.push((depth, action));
    }
    // A group is a run of controls that sit at the same depth: the tree lists a container's
    // children together, so a control that is deeper than the one before it has entered a
    // container of its own and starts a group.
    let mut groups: Vec<Vec<Value>> = Vec::new();
    let mut current: Vec<Value> = Vec::new();
    let mut current_depth: Option<u64> = None;
    for (depth, action) in controls {
        if let Some(previous) = current_depth
            && depth != previous
        {
            groups.push(std::mem::take(&mut current));
        }
        current_depth = Some(depth);
        current.push(action);
    }
    if !current.is_empty() {
        groups.push(current);
    }
    // Within a group, the fields are filled before the buttons are pressed: a control that
    // depends on what was typed only shows its effect when the field is filled first.
    groups.iter_mut().for_each(|group| {
        let (fields, buttons): (Vec<Value>, Vec<Value>) = group.iter().cloned().partition(|a| a.get("action").and_then(Value::as_str) == Some("fill"));
        *group = fields.into_iter().chain(buttons).collect();
    });
    // The groups are bounded, and the session as a whole: a group that would take it past the
    // budget is left out, and the groups before it are kept.
    let mut kept: Vec<Vec<Value>> = Vec::new();
    let mut count = 0;
    for group in groups.into_iter().take(MAX_GROUPS) {
        if count + group.len() > MAX_SESSION_ACTIONS {
            break;
        }
        count += group.len();
        kept.push(group);
    }
    kept
}

/// Most groups a native session is split into.
///
/// The native path opens no browser session, so a group costs little; the bound is here so a
/// window with a great many panels does not make the study endless.
pub const MAX_GROUPS: usize = 16;

/// Most actions one native session may have.
///
/// A session is replayed against the build, so every action runs twice.
pub const MAX_SESSION_ACTIONS: usize = 60;

/// Whether a control is pressed: a button, a link, a checkbox, a radio, a tab.
pub fn is_pressed(role: &str) -> bool {
    matches!(role, "AXButton" | "AXLink" | "AXCheckBox" | "AXRadioButton" | "AXTabButton" | "AXMenuButton" | "AXPopUpButton")
}
