//! Native program sessions: start a program, leave it running, and drive it from outside.
//!
//! The picture tool starts a program, waits for its window, pictures it and closes it. A
//! behaviour oracle needs more: the program must stay up while it is read and acted on, so the
//! original can be watched and the build can be watched the same way.
//!
//! A session is started by one tool, driven by the existing `runtime_ui_*` tools (which take a
//! pid), and closed by another. The program runs through the platform's process layer, so it
//! inherits everything that layer guarantees: a deadline, a cancelled run killed with its process
//! group, and output that never fills a pipe.

use crate::tool::{Ctx, Effects, Requires, ToolOutput, ToolSpec, Toolset, parse_params, schema_of};
use craft_anything_evidence::{Confidence, Coverage, ErrorCode, EvidenceBuilder, Subject, ToolError};
use craft_anything_platform::process::{Launch, Launched};
use craft_anything_platform::sync::Mutex;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Most sessions open at once.
const MAX_SESSIONS: usize = 8;
/// How long a program may take to start before it is given up on.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// The open sessions, kept for as long as the process runs.
struct Sessions(Mutex<HashMap<String, Session>>);

/// One open program.
struct Session {
    program: PathBuf,
    pid: u32,
    running: Launched,
    started: Instant,
}

/// The store of open sessions, in the session's extensions.
fn store(ctx: &mut Ctx<'_>) -> Arc<Sessions> {
    if let Some(s) = ctx.session.ext.get::<Sessions>() {
        return s;
    }
    let s = Arc::new(Sessions(Mutex::new(HashMap::new())));
    ctx.session.ext.insert(s.clone());
    s
}

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            id: "native_program_session_begin",
            title: "Start a program and leave it running",
            summary: "Start a program (an app or a game) and leave it running, so its window can be read and driven from outside with the runtime-ui tools. It runs the program, so it needs the person's OK. The session is closed with native_program_session_end, and every session is closed when the harness ends.",
            beginner: "Open a program on this computer and leave it running.",
            toolset: Toolset::Runtime,
            effects: Effects::MUTATES_SESSION.union(Effects::WRITES_FILESYSTEM).union(Effects::LAUNCHES_PROCESS).union(Effects::EXECUTES_FOREIGN_CODE),
            requires: Requires::Nothing,
            input_schema: schema_of::<BeginParams>,
            run: begin,
        },
        ToolSpec {
            id: "native_program_session_show",
            title: "Show a program's session",
            summary: "Report the open program sessions: the program, its process id, how long it has been running and whether it is still running.",
            beginner: "See which programs are open.",
            toolset: Toolset::Runtime,
            effects: Effects::MUTATES_SESSION.union(Effects::IDEMPOTENT),
            requires: Requires::Nothing,
            input_schema: schema_of::<ShowParams>,
            run: show,
        },
        ToolSpec {
            id: "native_program_session_end",
            title: "Close a program's session",
            summary: "Close a program session: the program is asked to stop, and killed with its process group if it does not. Nothing it started is left running. Every session is closed when the harness ends.",
            beginner: "Close a program this computer opened.",
            toolset: Toolset::Runtime,
            effects: Effects::MUTATES_SESSION.union(Effects::LAUNCHES_PROCESS).union(Effects::EXECUTES_FOREIGN_CODE),
            requires: Requires::Nothing,
            input_schema: schema_of::<EndParams>,
            run: end,
        },
    ]
}

/// Start a program and leave it running.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BeginParams {
    /// The program to start (an absolute path).
    pub program: PathBuf,
    /// Arguments for it.
    #[serde(default)]
    pub args: Vec<String>,
    /// Working folder for the program (an absolute directory). Defaults to the program's folder.
    #[serde(default)]
    pub working_directory: Option<PathBuf>,
}

/// Show the open sessions.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShowParams {
    /// One session to show; every one when it is empty.
    #[serde(default)]
    pub session_id: Option<String>,
}

/// Close one session.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EndParams {
    /// The session to close; every one when it is empty.
    #[serde(default)]
    pub session_id: Option<String>,
}

fn begin(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: BeginParams = parse_params(params)?;
    if !p.program.is_absolute() || !p.program.is_file() {
        return Err(
            ToolError::invalid(format!("{} isn't a program file.", p.program.display())).with_remediation("Pass the full path of the program to start.")
        );
    }
    let sessions = store(ctx);
    let mut open = sessions.0.lock().unwrap_or_else(|e| e.into_inner());
    if open.len() >= MAX_SESSIONS {
        return Err(ToolError::new(ErrorCode::OverLimit, format!("{MAX_SESSIONS} program sessions are already open"))
            .with_remediation("Close one with native_program_session_end, or list them with native_program_session_show."));
    }
    let dir = p.working_directory.clone().unwrap_or_else(|| p.program.parent().map(PathBuf::from).unwrap_or_default());
    ctx.report(None, format!("Starting {}…", p.program.display()));
    // The program is a window that is watched afterwards, so it must outlive the tool call that
    // started it. A [`Process`] is supervised and killed when its caller exits, which would take
    // the window with it; a [`Launch`] is fire-and-forget by design: nothing is captured and
    // nothing is killed, and the child is reaped when it ends.
    let running = Launch::new(&p.program).args(&p.args).cwd(&dir).label("the program").start_watched()?;
    let pid = running.pid();
    let id = format!("program-{pid}");
    let started = Instant::now();
    open.insert(id.clone(), Session { program: p.program.clone(), pid, running, started });
    drop(open);
    let ev = EvidenceBuilder::new(
        "native_program_session_begin",
        Subject { kind: "program".into(), sha256: None, name: Some(p.program.display().to_string()) },
        super::artifact_tools::env(ctx),
    )
    .params(json!({"program": p.program.display().to_string(), "args": p.args}))
    .result(json!({"session_id": id, "pid": pid}))
    .summary(format!("Started {} (process {pid}).", p.program.display()))
    .confidence(Confidence::Observed)
    .coverage(Coverage::complete(1))
    .build()?;
    Ok(ToolOutput::evidence(ev))
}

fn show(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: ShowParams = parse_params(params)?;
    let sessions = store(ctx);
    let mut open = sessions.0.lock().unwrap_or_else(|e| e.into_inner());
    let rows: Vec<Value> = open
        .iter_mut()
        .filter(|(id, _)| p.session_id.as_ref().is_none_or(|want| want == *id))
        .map(|(id, s)| {
            json!({
                "session_id": id,
                "program": s.program.display().to_string(),
                "pid": s.pid,
                "running_for_ms": s.started.elapsed().as_millis() as u64,
                "finished": !s.running.is_running(),
            })
        })
        .collect();
    drop(open);
    let ev = EvidenceBuilder::new("native_program_session_show", Subject { kind: "program".into(), sha256: None, name: None }, super::artifact_tools::env(ctx))
        .result(json!({"sessions": rows}))
        .summary(format!("{} program session(s) open.", rows.len()))
        .confidence(Confidence::Observed)
        .coverage(Coverage::complete(rows.len() as u64))
        .build()?;
    Ok(ToolOutput::evidence(ev))
}

fn end(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: EndParams = parse_params(params)?;
    let sessions = store(ctx);
    let mut open = sessions.0.lock().unwrap_or_else(|e| e.into_inner());
    let wanted: Vec<String> = match &p.session_id {
        Some(id) => vec![id.clone()],
        None => open.keys().cloned().collect(),
    };
    let mut closed = Vec::new();
    for id in wanted {
        let Some(mut session) = open.remove(&id) else { continue };
        // The program is asked to stop, then killed if it does not.
        session.running.stop();
        closed.push(json!({"session_id": id, "program": session.program.display().to_string(), "pid": session.pid}));
    }
    drop(open);
    let ev = EvidenceBuilder::new("native_program_session_end", Subject { kind: "program".into(), sha256: None, name: None }, super::artifact_tools::env(ctx))
        .result(json!({"closed": closed}))
        .summary(format!("Closed {} program session(s).", closed.len()))
        .confidence(Confidence::Observed)
        .coverage(Coverage::complete(closed.len() as u64))
        .build()?;
    Ok(ToolOutput::evidence(ev))
}
