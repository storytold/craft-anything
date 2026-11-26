use super::*;
use crate::agent::tests::{Scripted, turn};
use crate::agent::{ApiConversation, EngineHost, LoopConfig, ModelSlot};
use crate::model::{Block, ChatModel, Delta, Msg, Stop, StopReason, TurnRequest, TurnResult};

/// A kind whose one check always says the build isn't there yet (as a website with an empty
/// app/ does, #736). The builder is scripted to answer every turn without writing a file.
struct NeverBuilt;

struct NeverBuiltCheck;

impl Check for NeverBuiltCheck {
    fn id(&self) -> &'static str {
        "never-built"
    }
    fn run(&self, cx: &mut CheckCx) -> Result<CheckResult, ToolError> {
        Ok(CheckResult::new(self.id(), cx.round, false, "No built site found in app/ yet."))
    }
}

impl TargetKind for NeverBuilt {
    fn id(&self) -> &'static str {
        "never-built"
    }
    fn words(&self) -> KindWords {
        KindWords { button: "Build it", icon: "code", ready: "It's ready", change_hint: "Ask for a change" }
    }
    fn detect(&self, _cx: &DetectCx) -> Option<u32> {
        Some(1000)
    }
    fn default_stack(&self, _cx: &DetectCx) -> &'static str {
        "Scripted stack"
    }
    fn goal(&self) -> &'static str {
        "a scripted app"
    }
    fn checks(&self, _ws: &Workspace) -> Vec<Box<dyn Check>> {
        vec![Box::new(NeverBuiltCheck)]
    }
}

/// Runs one build whose builder never writes a file, and returns the run's error and how many
/// builder turns the model was asked for.
fn run_idle_builder(create_app_dir: bool) -> (ToolError, Vec<Vec<Msg>>) {
    let dir = tempfile::tempdir().unwrap();
    let mut ws = Workspace::create(dir.path(), "Idle build", "Build an app", TargetRef::None).unwrap();
    if create_app_dir {
        std::fs::create_dir_all(ws.app_dir()).unwrap();
    }
    ws.manifest.stack = "Scripted stack".into();
    ws.save().unwrap();
    let mut worker = check_worker(&ws, false);
    let seen = Arc::default();
    let turns = (0..20).map(|_| turn(vec![Block::Text { text: "Built it and ran npm run build.".into() }], StopReason::EndTurn)).collect();
    let model = Scripted { turns, seen: Arc::clone(&seen) };
    let slot =
        ModelSlot { name: "Scripted".into(), model: Box::new(model), local: true, effort: None, max_turns: 10, context_tokens: 0, tools: Default::default() };
    let api = ApiConversation::start(Arc::new(EngineHost(worker.engine.clone())), vec![slot], LoopConfig::default()).unwrap();
    let entry = crate::BackendEntry {
        config: crate::BackendConfig {
            name: "Scripted".into(),
            kind: BackendKind::OpenaiCompatible,
            model: "scripted".into(),
            base_url: None,
            ssh_host: None,
            enabled: true,
        },
        options: Default::default(),
    };
    let mut kinds = KindRegistry::built_in();
    kinds.register(Arc::new(NeverBuilt));
    let opts = RunOptions { message: Some("Build the app".into()), kinds, max_rounds: 6, ..RunOptions::default() };
    let error = worker.run(&mut ws, entry, Some(api), None, super::super::KeptHistory::default(), &opts).unwrap_err();
    let requests = seen.lock().unwrap().clone();
    (error, requests)
}

#[test]
fn an_idle_builder_with_an_empty_app_stops_after_two_turns() {
    for create_app_dir in [true, false] {
        let (error, requests) = run_idle_builder(create_app_dir);
        assert!(error.message.contains("has no files yet"), "app dir {create_app_dir}: {error:?}");
        assert!(
            requests.len() <= 3,
            "app dir {create_app_dir}: the builder was asked {} times; it should stop after two turns that changed nothing",
            requests.len()
        );
    }
}

/// #736: with app/ empty, the stop says app/ has no files yet, and the builder is told to write
/// the project's files (not to fix a build) and to show the build's output.
#[test]
fn an_empty_app_is_said_to_have_no_project() {
    let (error, requests) = run_idle_builder(true);
    assert!(error.message.contains("has no files yet"), "{error:?}");
    assert!(error.remediation.as_deref().unwrap_or_default().contains("asks for the project's files"), "{error:?}");
    let second = requests.get(1).map(|m| m.iter().map(Msg::text).collect::<String>()).unwrap_or_default();
    assert!(second.contains("app/ has no files"), "the next prompt says so: {second}");
    assert!(second.contains("exact command you ran"), "and asks for the build's output: {second}");
    assert!(!second.contains("Fix this in app/"), "not a fix for a build that doesn't exist: {second}");
}

/// A builder that changes one file in app/ every turn, so the unchanged-turn limit never fires.
struct Busy {
    app: PathBuf,
    turn: usize,
    seen: Arc<Mutex<usize>>,
}

impl ChatModel for Busy {
    fn turn(&mut self, _request: &TurnRequest<'_>, _delta: &mut dyn FnMut(Delta), _stop: &Stop) -> Result<TurnResult, ToolError> {
        self.turn += 1;
        // Numbered across runs too (a shared count), so each turn writes a file the last didn't.
        let mut count = self.seen.lock().unwrap();
        *count += 1;
        std::fs::create_dir_all(&self.app).unwrap();
        std::fs::write(self.app.join(format!("attempt-{}.txt", *count)), "try").unwrap();
        turn(vec![Block::Text { text: "Changed the app.".into() }], StopReason::EndTurn)
    }
    fn model(&self) -> String {
        "busy".into()
    }
}

/// One run of the build (a request, or "Try again" when `message` is `None`), with a builder
/// that keeps changing files and a check that never passes.
fn run_busy_builder(ws: &mut Workspace, seen: &Arc<Mutex<usize>>, message: Option<&str>) -> ToolError {
    let mut worker = check_worker(ws, false);
    let model = Busy { app: ws.app_dir(), turn: 0, seen: Arc::clone(seen) };
    let slot =
        ModelSlot { name: "Busy".into(), model: Box::new(model), local: true, effort: None, max_turns: 10, context_tokens: 0, tools: Default::default() };
    let api = ApiConversation::start(Arc::new(EngineHost(worker.engine.clone())), vec![slot], LoopConfig::default()).unwrap();
    let entry = crate::BackendEntry {
        config: crate::BackendConfig {
            name: "Busy".into(),
            kind: BackendKind::OpenaiCompatible,
            model: "busy".into(),
            base_url: None,
            ssh_host: None,
            enabled: true,
        },
        options: Default::default(),
    };
    let mut kinds = KindRegistry::built_in();
    kinds.register(Arc::new(NeverBuilt));
    let opts = RunOptions { message: message.map(str::to_string), continuing: message.is_none(), kinds, max_rounds: 6, ..RunOptions::default() };
    worker.run(ws, entry, Some(api), None, super::super::KeptHistory::default(), &opts).unwrap_err()
}

#[test]
fn try_again_does_not_restart_a_build_that_never_works() {
    let dir = tempfile::tempdir().unwrap();
    let mut ws = Workspace::create(dir.path(), "Busy build", "Build an app", TargetRef::None).unwrap();
    ws.manifest.stack = "Scripted stack".into();
    ws.save().unwrap();
    let seen = Arc::new(Mutex::new(0));
    let first = run_busy_builder(&mut ws, &seen, Some("Build the app"));
    assert!(first.message.contains("isn't working yet"), "{first:?}");
    // Try again, many times: each press must not buy another full set of rounds.
    for _ in 0..5 {
        run_busy_builder(&mut ws, &seen, None);
    }
    let total = ws.manifest.checks.len();
    assert!(total <= NOT_WORKING_CHECK_LIMIT as usize, "{total} checks over six runs; the build must stop at its limit");
    assert_eq!(ws.manifest.not_working_checks, NOT_WORKING_CHECK_LIMIT, "the count is kept in the project");

    // Once at the limit, Try again stops before paying for another builder turn.
    let turns = *seen.lock().unwrap();
    let stopped = run_busy_builder(&mut ws, &seen, None);
    assert!(stopped.message.contains("in a row"), "{stopped:?}");
    assert_eq!(*seen.lock().unwrap(), turns, "no builder turn is started at the limit");
    assert_eq!(ws.manifest.checks.len(), total, "and no check runs either");

    // A new request starts the count again: the build gets its checks back.
    let restarted = run_busy_builder(&mut ws, &seen, Some("Use a plain page instead"));
    assert!(restarted.message.contains("isn't working yet"), "{restarted:?}");
    assert!(ws.manifest.checks.len() > total, "a new request runs checks again");
    assert!(ws.manifest.not_working_checks <= NOT_WORKING_CHECK_LIMIT, "{}", ws.manifest.not_working_checks);
}

/// A minimal kind that runs the real [`super::super::check::build_check`] against a test-controlled
/// editor path without touching the host's installed engines.
struct BuildOnlyKind {
    editor: PathBuf,
    check_runs: Arc<std::sync::atomic::AtomicUsize>,
    pause: Option<craft_anything_engine::CancelToken>,
}

struct StandinBuildCheck {
    editor: PathBuf,
    check_runs: Arc<std::sync::atomic::AtomicUsize>,
    /// Cancelled once the build has been checked, as Pause pressed then does.
    pause: Option<craft_anything_engine::CancelToken>,
}

impl Check for StandinBuildCheck {
    fn id(&self) -> &'static str {
        CheckResult::BUILD
    }
    fn run(&self, cx: &mut CheckCx) -> Result<CheckResult, ToolError> {
        self.check_runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (passed, note) = super::super::check::build_check(&cx.ws.app_dir(), cx.ws.app_place(), "Godot", None, Some(&self.editor), &cx.stop(), &[])?;
        if let Some(stop) = &self.pause {
            stop.cancel();
        }
        Ok(CheckResult::new(self.id(), cx.round, passed, note))
    }
}

impl TargetKind for BuildOnlyKind {
    fn id(&self) -> &'static str {
        "build-only"
    }
    fn words(&self) -> KindWords {
        KindWords { button: "Build it", icon: "code", ready: "It's ready", change_hint: "Ask for a change" }
    }
    fn detect(&self, _cx: &DetectCx) -> Option<u32> {
        Some(1000)
    }
    fn default_stack(&self, _cx: &DetectCx) -> &'static str {
        "Scripted stack"
    }
    fn goal(&self) -> &'static str {
        "a Godot project"
    }
    fn checks(&self, _ws: &Workspace) -> Vec<Box<dyn Check>> {
        vec![Box::new(StandinBuildCheck { editor: self.editor.clone(), check_runs: Arc::clone(&self.check_runs), pause: self.pause.clone() })]
    }
}

/// Writes `project.godot` on its first turn, then makes no further edits to `app/`, recording each
/// prompt it receives.
struct WriteOnceBuilder {
    app: PathBuf,
    prompts: Arc<Mutex<Vec<String>>>,
}

impl ChatModel for WriteOnceBuilder {
    fn turn(&mut self, request: &TurnRequest<'_>, _delta: &mut dyn FnMut(Delta), _stop: &Stop) -> Result<TurnResult, ToolError> {
        let mut prompts = self.prompts.lock().unwrap();
        let text = request.messages.last().map(Msg::text).unwrap_or_default();
        prompts.push(text);
        if prompts.len() == 1 {
            std::fs::create_dir_all(&self.app).unwrap();
            std::fs::write(self.app.join("project.godot"), "[application]\nconfig/name=\"Demo\"\n").unwrap();
        }
        turn(vec![Block::Text { text: "Built the project.".into() }], StopReason::EndTurn)
    }
    fn model(&self) -> String {
        "write-once".into()
    }
}

/// Create a tiny cross-platform stand-in executable that optionally prints one line and exits with
/// `code`.
fn write_standin_tool(dir: &Path, stem: &str, line: &str, code: i32) -> PathBuf {
    #[cfg(windows)]
    {
        let path = dir.join(format!("{stem}.cmd"));
        let body = if line.is_empty() { format!("@echo off\r\nexit /b {code}\r\n") } else { format!("@echo off\r\necho {line}\r\nexit /b {code}\r\n") };
        std::fs::write(&path, body).unwrap();
        path
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(stem);
        let body = if line.is_empty() { format!("#!/bin/sh\nexit {code}\n") } else { format!("#!/bin/sh\nprintf '%s\\n' '{line}'\nexit {code}\n") };
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
}

/// Run [`BuildOnlyKind`] with a scripted [`WriteOnceBuilder`] and the given editor path.
fn run_build_only(
    ws: &mut Workspace,
    editor: PathBuf,
    prompts: &Arc<Mutex<Vec<String>>>,
    check_runs: &Arc<std::sync::atomic::AtomicUsize>,
    message: Option<&str>,
    kept: super::super::KeptHistory,
    max_rounds: u32,
) -> Result<String, ToolError> {
    run_build_only_pausing(ws, (editor, false), prompts, check_runs, message, kept, max_rounds)
}

/// As [`run_build_only`]; with `pause`, Pause is pressed as the check finishes.
fn run_build_only_pausing(
    ws: &mut Workspace,
    (editor, pause): (PathBuf, bool),
    prompts: &Arc<Mutex<Vec<String>>>,
    check_runs: &Arc<std::sync::atomic::AtomicUsize>,
    message: Option<&str>,
    kept: super::super::KeptHistory,
    max_rounds: u32,
) -> Result<String, ToolError> {
    let mut worker = check_worker(ws, false);
    let pause = pause.then(|| worker.stop.clone());
    let model = WriteOnceBuilder { app: ws.app_dir(), prompts: Arc::clone(prompts) };
    let slot =
        ModelSlot { name: "WriteOnce".into(), model: Box::new(model), local: true, effort: None, max_turns: 10, context_tokens: 0, tools: Default::default() };
    let api = ApiConversation::start(Arc::new(EngineHost(worker.engine.clone())), vec![slot], LoopConfig::default()).unwrap();
    let entry = crate::BackendEntry {
        config: crate::BackendConfig {
            name: "WriteOnce".into(),
            kind: BackendKind::OpenaiCompatible,
            model: "write-once".into(),
            base_url: None,
            ssh_host: None,
            enabled: true,
        },
        options: Default::default(),
    };
    let mut kinds = KindRegistry::built_in();
    kinds.register(Arc::new(BuildOnlyKind { editor, check_runs: Arc::clone(check_runs), pause }));
    let opts = RunOptions { message: message.map(str::to_string), continuing: message.is_none(), kinds, max_rounds, ..RunOptions::default() };
    worker.run(ws, entry, Some(api), None, kept, &opts)
}

/// #2178: a missing build tool stops verification without asking the builder to refine app/ or
/// tripping the unchanged-turns stop on Try again; once the tool is available, Try again finishes
/// checking without re-running the builder.
#[test]
fn missing_build_tool_stops_without_refining_or_blaming_unchanged_app_files() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let dir = tempfile::tempdir().unwrap();
    let mut ws = Workspace::create(dir.path(), "Missing tool", "Build a Godot game", TargetRef::None).unwrap();
    ws.manifest.stack = "Scripted stack".into();
    ws.save().unwrap();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let check_runs = Arc::new(AtomicUsize::new(0));
    let missing = dir.path().join("missing-godot-editor");

    let first = run_build_only(&mut ws, missing.clone(), &prompts, &check_runs, Some("Build the game"), super::super::KeptHistory::default(), 4).unwrap_err();
    assert_eq!(first.code, ErrorCode::EngineMissing, "{first:?}");
    assert!(first.message.contains("couldn't be checked") && first.message.contains("Couldn't run the build"), "{first:?}");
    assert!(!first.message.contains("changed no files"), "{first:?}");
    assert!(
        first.remediation.as_deref().is_some_and(|r| r.contains("Settings") && r.contains("Try again")),
        "keeps tool setup guidance alongside Try again: {first:?}"
    );
    assert_eq!(first.details.as_ref().and_then(|d| d.get("reason")).and_then(serde_json::Value::as_str), Some("path_missing"), "{first:?}");
    assert_eq!(prompts.lock().unwrap().len(), 1, "missing tool must not trigger a builder refinement turn");
    assert_eq!(check_runs.load(Ordering::SeqCst), 1, "missing tool is not retried immediately");
    assert!(ws.manifest.awaiting_check, "finished build stays awaiting check");
    assert_eq!(ws.manifest.not_working_checks, 0, "an unavailable tool does not count against the build");

    // Repeated Try again while the tool is still missing re-checks only and never trips the
    // unchanged-turns stall error.
    let kept = super::super::KeptHistory { in_mode: true, elsewhere: false };
    for attempt in 1..=2 {
        let retry = run_build_only(&mut ws, missing.clone(), &prompts, &check_runs, None, kept, 4).unwrap_err();
        assert_eq!(retry.code, ErrorCode::EngineMissing, "attempt {attempt}: {retry:?}");
        assert!(!retry.message.contains("changed no files"), "attempt {attempt}: {retry:?}");
        assert_eq!(prompts.lock().unwrap().len(), 1, "attempt {attempt}: Try again must not re-run the builder");
        assert_eq!(check_runs.load(Ordering::SeqCst), 1 + attempt, "attempt {attempt}: one check attempt per Try again");
        assert!(ws.manifest.awaiting_check, "attempt {attempt}: build still awaits check");
    }

    // Once the tool exists and succeeds, Try again checks the existing build and finishes Ready
    // with zero extra builder turns.
    let working = write_standin_tool(dir.path(), "working-godot", "", 0);
    let done = run_build_only(&mut ws, working, &prompts, &check_runs, None, kept, 4).unwrap();
    assert!(done.starts_with("Ready:"), "{done}");
    assert_eq!(prompts.lock().unwrap().len(), 1, "no extra builder turn when check succeeds on Try again");
    assert!(!ws.manifest.awaiting_check);
}

/// #2430: Pause pressed while the last check runs ends the run paused, not Ready, and carrying on
/// checks the build again.
#[test]
fn a_pause_during_the_last_check_doesnt_end_in_ready() {
    let dir = tempfile::tempdir().unwrap();
    let mut ws = Workspace::create(dir.path(), "Paused check", "Build a Godot game", TargetRef::None).unwrap();
    ws.manifest.stack = "Scripted stack".into();
    ws.save().unwrap();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let check_runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    // One that fails as well: a check the pause cut short isn't counted against the build.
    for code in [0, 1] {
        let editor = write_standin_tool(dir.path(), &format!("paused-godot-{code}"), "", code);
        let (message, kept) = match code {
            0 => (Some("Build the game"), super::super::KeptHistory::default()),
            _ => (None, super::super::KeptHistory { in_mode: true, elsewhere: false }),
        };
        let paused = run_build_only_pausing(&mut ws, (editor, true), &prompts, &check_runs, message, kept, 4).unwrap_err();
        assert_eq!(paused.code, ErrorCode::Cancelled, "{paused:?}");
        assert_ne!(ws.manifest.status, Status::Ready);
        assert!(ws.manifest.awaiting_check && ws.manifest.checks.is_empty(), "the paused round isn't kept");
        assert_eq!(ws.manifest.not_working_checks, 0);
    }
    let working = write_standin_tool(dir.path(), "resumed-godot", "", 0);
    let kept = super::super::KeptHistory { in_mode: true, elsewhere: false };
    let done = run_build_only(&mut ws, working, &prompts, &check_runs, None, kept, 4).unwrap();
    assert!(done.starts_with("Ready:"), "{done}");
    assert_eq!(prompts.lock().unwrap().len(), 1, "only the check runs again");
}

/// Companion to #2178: a genuine compile/build failure (the tool ran and failed) still enters a
/// refinement turn so the builder can fix `app/`.
#[test]
fn real_build_failure_still_refines_in_a_second_builder_turn() {
    let dir = tempfile::tempdir().unwrap();
    let mut ws = Workspace::create(dir.path(), "Broken build", "Build a Godot game", TargetRef::None).unwrap();
    ws.manifest.stack = "Scripted stack".into();
    ws.save().unwrap();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let check_runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let failing = write_standin_tool(dir.path(), "failing-godot", "ERROR: syntax broken", 1);

    let err = run_build_only(&mut ws, failing, &prompts, &check_runs, Some("Build the game"), super::super::KeptHistory::default(), 1).unwrap_err();
    assert_eq!(err.code, ErrorCode::ProgramFailed, "{err:?}");
    let seen = prompts.lock().unwrap();
    assert_eq!(seen.len(), 2, "a real build failure must still trigger refinement: {seen:?}");
    assert!(seen[1].contains("ERROR: syntax broken"), "the refinement prompt carries the build error: {}", seen[1]);
}

/// Companion to #2178: an own-project build check also surfaces a missing tool as `EngineMissing`
/// while leaving a real non-zero build failure as a failed check result.
#[test]
fn own_project_build_check_separates_missing_tool_from_real_failure() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("project.godot"), "[application]\nconfig/name=\"Own\"\n").unwrap();
    let mut ws =
        Workspace::create_at(dir.path(), "Own game", "Fix a level", TargetRef::None, Some(craft_anything_engine::workspace::ExistingFiles::Edit)).unwrap();
    ws.manifest.stack = "Godot".into();
    ws.save().unwrap();

    let mut worker = check_worker(&ws, false);
    let missing = dir.path().join("missing-own-godot");
    worker.editor = Some(missing);
    let err = worker.run_checks(&ws, &GameKind, 1).unwrap_err();
    assert_eq!(err.code, ErrorCode::EngineMissing, "{err:?}");

    let failing = write_standin_tool(dir.path(), "failing-own-godot", "ERROR: own build failed", 1);
    worker.editor = Some(failing);
    let checks = worker.run_checks(&ws, &GameKind, 1).unwrap();
    assert_eq!(checks.len(), 1);
    assert!(!checks[0].passed && checks[0].note.contains("ERROR: own build failed"), "{checks:?}");
}

/// #2178: `checks_with_a_retry_now` still retries transient check errors (such as a dropped
/// browser WebSocket with no missing-tool reason), never retries cancellation, and
/// `unchecked_build_error` keeps tool remediation cleanly punctuated while filtering blank or
/// non-tool remediations.
#[test]
fn retry_policy_and_unchecked_build_remediation_cover_transient_and_cancelled_errors() {
    struct ScriptedCheck {
        calls: Arc<std::sync::atomic::AtomicUsize>,
        outcomes: Mutex<std::collections::VecDeque<Result<CheckResult, ToolError>>>,
    }
    impl Check for ScriptedCheck {
        fn id(&self) -> &'static str {
            "scripted"
        }
        fn run(&self, _cx: &mut CheckCx) -> Result<CheckResult, ToolError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.outcomes.lock().unwrap().pop_front().unwrap()
        }
    }
    struct ScriptedKind {
        calls: Arc<std::sync::atomic::AtomicUsize>,
        outcomes: Mutex<std::collections::VecDeque<Result<CheckResult, ToolError>>>,
    }
    impl TargetKind for ScriptedKind {
        fn id(&self) -> &'static str {
            "scripted"
        }
        fn words(&self) -> KindWords {
            KindWords { button: "Open", icon: "box", ready: "Ready", change_hint: "Ask" }
        }
        fn detect(&self, _cx: &DetectCx) -> Option<u32> {
            None
        }
        fn default_stack(&self, _cx: &DetectCx) -> &'static str {
            "Scripted"
        }
        fn goal(&self) -> &'static str {
            "a scripted build"
        }
        fn checks(&self, _ws: &Workspace) -> Vec<Box<dyn Check>> {
            let next = self.outcomes.lock().unwrap().pop_front().unwrap();
            vec![Box::new(ScriptedCheck { calls: Arc::clone(&self.calls), outcomes: Mutex::new(std::collections::VecDeque::from([next])) })]
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::create(dir.path(), "RetryCheck", "Build it", TargetRef::None).unwrap();
    let worker = check_worker(&ws, false);

    // Transient `EngineMissing` (e.g. dropped browser WebSocket, no missing-tool `reason`) gets
    // an immediate retry and succeeds on the second attempt.
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let kind = ScriptedKind {
        calls: Arc::clone(&calls),
        outcomes: Mutex::new(std::collections::VecDeque::from([
            Err(ToolError::new(ErrorCode::EngineMissing, "the WebSocket is closed")),
            Ok(CheckResult::new("scripted", 1, true, "Fine.")),
        ])),
    };
    let checks = worker.checks_with_a_retry_now(&ws, &kind, 1).unwrap();
    assert_eq!(checks.len(), 1);
    assert!(checks[0].passed);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2, "transient check error must retry once");

    // Cancellation stops immediately with no retry.
    let cancel_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let cancel_kind = ScriptedKind { calls: Arc::clone(&cancel_calls), outcomes: Mutex::new(std::collections::VecDeque::from([Err(ToolError::cancelled())])) };
    let err = worker.checks_with_a_retry_now(&ws, &cancel_kind, 1).unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled);
    assert_eq!(cancel_calls.load(std::sync::atomic::Ordering::SeqCst), 1, "cancelled check must not retry");

    // `unchecked_build_error` punctuates unpunctuated tool remediation, ignores blank or
    // non-unavailable-tool remediation, and preserves `details`.
    let no_dot = ToolError::new(ErrorCode::NoPermission, "denied").with_remediation("Make it executable");
    let wrapped = unchecked_build_error(no_dot);
    assert_eq!(wrapped.remediation.as_deref(), Some("Make it executable. Press Try again to check it again. Nothing needs building again."));

    let blank = ToolError::new(ErrorCode::NoPermission, "denied").with_remediation("   ");
    assert_eq!(unchecked_build_error(blank).remediation.as_deref(), Some("Press Try again to check it again. Nothing needs building again."));

    let transient =
        ToolError::new(ErrorCode::EngineMissing, "WebSocket closed").with_remediation("Transient hint.").with_details(serde_json::json!({"reason": "dropped"}));
    let wrapped_transient = unchecked_build_error(transient);
    assert_eq!(wrapped_transient.remediation.as_deref(), Some("Press Try again to check it again. Nothing needs building again."));
    assert_eq!(wrapped_transient.details.as_ref().and_then(|d| d["reason"].as_str()), Some("dropped"));
}
