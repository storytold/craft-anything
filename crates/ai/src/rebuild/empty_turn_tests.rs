//! #2431: a builder turn that did nothing (Claude Code's own count of zero tokens both ways, no
//! text, no tool call, no file change) is sent once more, and a second one ends the run with a
//! problem instead of checking an untouched build as if the request had been carried out.

use super::*;
use std::os::unix::fs::PermissionsExt;

/// Claude Code's result for a turn that read and wrote nothing.
const EMPTY: &str =
    r#"{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"","session_id":"s1","usage":{"input_tokens":0,"output_tokens":0}}"#;
/// A turn that worked: a reply and a result with real usage.
const WORKED: &str = concat!(
    r#"{"type":"assistant","session_id":"s1","message":{"content":[{"type":"text","text":"Built it."}]}}"#,
    "\n",
    r#"{"type":"result","subtype":"success","is_error":false,"num_turns":2,"result":"Built it.","session_id":"s1","usage":{"input_tokens":120,"output_tokens":30}}"#,
);

/// A stand-in Claude Code: every run appends its prompt (stdin) to `prompts` and a line to
/// `attempts`, then runs `body`, a shell fragment in which `$n` is this run's number from 1.
fn stand_in(ws: &Workspace, body: &str) -> crate::cli_agents::CliConversation {
    let program = ws.root.join("agent-stand-in.sh");
    let script =
        format!("#!/bin/sh\ncat >> prompts\nprintf '\\n===\\n' >> prompts\nprintf 'attempt\\n' >> attempts\nn=$(wc -l < attempts | tr -d ' ')\n{body}\n");
    std::fs::write(&program, script).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = crate::BackendConfig {
        name: "Scripted CLI".into(),
        kind: BackendKind::AgentCli,
        model: "claude-code".into(),
        base_url: None,
        ssh_host: None,
        enabled: true,
    };
    let options = crate::BackendOptions { command: Some(program.display().to_string()), timeout_secs: Some(20), ..Default::default() };
    let mut spec = CliSpec::new(config, options, false, None);
    spec.project = Some(ws.root.clone());
    spec.preamble = Some(String::new());
    crate::cli_agents::CliConversation::new(spec)
}

/// Print `line` (one JSON line each) from the script.
fn say(lines: &str) -> String {
    lines.lines().map(|l| format!("printf '%s\\n' '{l}'\n")).collect()
}

fn project() -> (tempfile::TempDir, Workspace) {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::create(dir.path(), "x", "Build a thing", TargetRef::None).unwrap();
    std::fs::create_dir_all(ws.app_dir()).unwrap();
    (dir, ws)
}

fn attempts(ws: &Workspace) -> usize {
    std::fs::read_to_string(ws.root.join("attempts")).unwrap_or_default().lines().count()
}

fn prompts(ws: &Workspace) -> Vec<String> {
    let all = std::fs::read_to_string(ws.root.join("prompts")).unwrap_or_default();
    all.split("\n===\n").filter(|p| !p.is_empty()).map(str::to_string).collect()
}

fn said_again(events: &[RunEvent]) -> usize {
    events.iter().filter(|e| matches!(e, RunEvent::Agent(AgentEvent::Status { text }) if text == EMPTY_TURN_AGAIN)).count()
}

/// One builder turn through the worker, with what it emitted.
fn builder_turn(ws: &mut Workspace, cli: &mut crate::cli_agents::CliConversation) -> (Result<String, ToolError>, Vec<RunEvent>) {
    let mut worker = check_worker(ws, false);
    let (tx, rx) = std::sync::mpsc::channel();
    worker.tx = tx;
    let mut fingerprints = AppFingerprintCache::default();
    let result = worker.builder_turn(ws, cli, "Make the buttons blue", false, None, &mut fingerprints);
    (result, rx.try_iter().collect())
}

/// The reporter's case: the first turn ends at once with 0 tokens in and out; the same message
/// goes once more, and the second turn builds.
#[test]
fn an_empty_builder_turn_is_sent_once_more_with_the_same_message() {
    let (_dir, mut ws) = project();
    let mut cli = stand_in(&ws, &format!("if [ \"$n\" = 1 ]; then\n{}else\n{}fi", say(EMPTY), say(WORKED)));
    let (result, events) = builder_turn(&mut ws, &mut cli);
    assert_eq!(result.unwrap(), "Built it.");
    assert_eq!(attempts(&ws), 2);
    assert_eq!(prompts(&ws), ["Make the buttons blue", "Make the buttons blue"], "the same message, not a 'carry on'");
    assert_eq!(said_again(&events), 1, "{events:?}");
    assert_eq!(events.iter().filter(|e| matches!(e, RunEvent::Agent(AgentEvent::TurnDone { .. }))).count(), 2);
    assert!(!cli.last_turn_empty());
}

/// Empty twice: exactly one retry, then the run's error says the message didn't reach the
/// builder. It isn't a failure the provider retry loop takes up.
#[test]
fn a_builder_turn_empty_twice_is_retried_once_and_then_reported() {
    let (_dir, mut ws) = project();
    let mut cli = stand_in(&ws, &say(EMPTY));
    let (result, events) = builder_turn(&mut ws, &mut cli);
    let e = result.unwrap_err();
    assert_eq!(e.code, ErrorCode::ProgramFailed);
    assert_eq!(e.message, EMPTY_TURN);
    assert_eq!(attempts(&ws), 2, "exactly one retry");
    assert_eq!(said_again(&events), 1);
    assert!(cli.last_failure().is_none(), "nothing for the provider retry loop to retry");
}

/// Never sent again: a turn that did any work, whatever its count says, and an agent that
/// reports no count at all (its zeros mean "unknown", not "nothing").
#[test]
fn a_turn_that_did_something_or_reports_no_count_is_never_sent_again() {
    let tool = concat!(
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"app/x"}}]}}"#,
        "\n",
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"x"}]}}"#,
    );
    let text = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Nothing to change."}]}}"#;
    let no_count = r#"{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":""}"#;
    let cases = [
        ("a reply", format!("{}{}", say(text), say(EMPTY))),
        ("a tool call", format!("{}{}", say(tool), say(EMPTY))),
        ("a file changed in app/", format!("printf 'body{{}}' > app/style.css\n{}", say(EMPTY))),
        ("no count, but a file changed", format!("printf 'body{{}}' > app/style.css\n{}", say(no_count))),
        ("no count and nothing else", say(no_count)),
        ("no output at all", String::new()),
    ];
    for (what, body) in cases {
        let (_dir, mut ws) = project();
        let mut cli = stand_in(&ws, &body);
        let (result, events) = builder_turn(&mut ws, &mut cli);
        assert!(result.is_ok(), "{what}: {result:?}");
        assert_eq!(attempts(&ws), 1, "{what}: sent again");
        assert_eq!(said_again(&events), 0, "{what}");
    }
}

/// A turn that failed for a reason sending it again can't fix (sign-in, a plan limit, a saved
/// session that isn't here) ends as it did before, with its own error and no empty-turn retry,
/// though its count is zero too.
#[test]
fn a_failed_turn_with_no_tokens_is_not_taken_for_an_empty_one() {
    let sign_in = concat!(
        r#"{"type":"assistant","error":"authentication_failed","message":{"content":[{"type":"text","text":"Not logged in"}]}}"#,
        "\n",
        r#"{"type":"result","subtype":"success","is_error":true,"num_turns":1,"result":"Not logged in","usage":{"input_tokens":0,"output_tokens":0}}"#,
    );
    let limit = concat!(
        r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","rateLimitType":"five_hour","resetsAt":4102444800}}"#,
        "\n",
        r#"{"type":"assistant","error":"rate_limit","message":{"content":[{"type":"text","text":"You have hit your limit"}]}}"#,
        "\n",
        r#"{"type":"result","subtype":"success","is_error":true,"num_turns":1,"result":"You have hit your limit","usage":{"input_tokens":0,"output_tokens":0}}"#,
    );
    let lost = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"num_turns":0,"errors":["No conversation found with session ID: s0"],"usage":{"input_tokens":0,"output_tokens":0}}"#;
    for (what, body) in [("sign-in", say(sign_in)), ("plan limit", say(limit)), ("lost session", format!("{}exit 1", say(lost)))] {
        let (_dir, mut ws) = project();
        let mut cli = stand_in(&ws, &body);
        let (result, events) = builder_turn(&mut ws, &mut cli);
        let e = result.expect_err(what);
        assert_ne!(e.message, EMPTY_TURN, "{what}");
        assert_eq!(attempts(&ws), 1, "{what}: sent again");
        assert_eq!(said_again(&events), 0, "{what}");
        assert!(!cli.last_turn_empty(), "{what}");
    }
}

/// Pause during a turn that then reports nothing: the run stops, and the turn isn't sent again.
#[test]
fn a_paused_turn_is_never_sent_again() {
    let (_dir, mut ws) = project();
    let mut cli = stand_in(&ws, &format!("sleep 3\n{}", say(EMPTY)));
    let mut worker = check_worker(&ws, false);
    let (tx, rx) = std::sync::mpsc::channel();
    worker.tx = tx;
    let stop = worker.stop.clone();
    let pause = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        stop.cancel();
    });
    let mut fingerprints = AppFingerprintCache::default();
    let result = worker.builder_turn(&mut ws, &mut cli, "Make the buttons blue", false, None, &mut fingerprints);
    pause.join().unwrap();
    assert_eq!(result.unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(attempts(&ws), 1);
    assert_eq!(said_again(&rx.try_iter().collect::<Vec<_>>()), 0);
    assert!(!cli.last_turn_empty(), "a stopped turn isn't an empty one");
}
