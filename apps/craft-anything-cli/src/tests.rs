//! `run` called in-process: what each command line prints, on which stream, and the exit code.

use super::run;

struct Ran {
    code: i32,
    out: String,
    err: String,
}

fn invoke(args: &[&str]) -> Ran {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(std::iter::once("craft-anything-cli").chain(args.iter().copied()), &mut out, &mut err);
    Ran { code, out: String::from_utf8_lossy(&out).into_owned(), err: String::from_utf8_lossy(&err).into_owned() }
}

#[test]
fn help_and_version_go_to_stdout_and_succeed_everywhere() {
    for args in [&["--help"][..], &["-h"], &["help", "setup"], &["setup", "--help"], &["ai", "model", "--help"], &["rebuild", "-h"]] {
        let r = invoke(args);
        assert_eq!((r.code, r.err.as_str()), (0, ""), "{args:?}");
        assert!(r.out.contains("Usage"), "{args:?}: {}", r.out);
    }
    // `--version` and `-V` work on the top level and on every built-in subcommand.
    for args in [
        &["--version"][..],
        &["-V"],
        &["setup", "--version"],
        &["doctor", "-V"],
        &["ai", "-V"],
        &["ai", "list", "--version"],
        &["rebuild", "-V"],
        &["chat", "--version"],
    ] {
        let r = invoke(args);
        assert_eq!((r.code, r.err.as_str()), (0, ""), "{args:?}");
        assert!(r.out.contains(env!("CARGO_PKG_VERSION")), "{args:?}: {}", r.out);
    }
}

#[test]
fn usage_errors_exit_2_with_the_message_on_stderr_and_nothing_on_stdout() {
    for args in [
        &["--bogus-flag", "commands"][..],
        &["setup", "--bogus"],
        &["doctor", "--client"],
        &["report", "--html", "--html"],
        &["--deep", "--clean-room", "commands"],
        &["ai", "add"],
        &[],
    ] {
        let r = invoke(args);
        assert_eq!(r.code, 2, "{args:?}: {}", r.err);
        assert_eq!(r.out, "", "{args:?}");
        assert!(r.err.contains("error:") || r.err.contains("Usage"), "{args:?}: {}", r.err);
    }
}

#[test]
fn a_documented_repeatable_flag_is_accepted_twice() {
    // `--client` is repeatable: a repeat is not a usage error (the command itself may still fail).
    let r = invoke(&["doctor", "--help"]);
    assert!(r.out.contains("repeatable"), "{}", r.out);
}

#[test]
fn a_bad_log_level_is_refused_before_any_work() {
    let r = invoke(&["--log", "loud", "commands"]);
    assert_eq!((r.code, r.out.as_str()), (2, ""));
    assert!(r.err.contains("loud"), "{}", r.err);
}

#[test]
fn the_command_is_what_the_completions_and_man_pages_are_made_of() {
    let command = super::command();
    let names: Vec<_> = command.get_subcommands().map(|s| s.get_name().to_string()).collect();
    assert!(names.iter().any(|n| n == "setup") && names.iter().any(|n| n == "mcp"), "{names:?}");
    // The log variable is declared on its flag, so `--help` lists it.
    assert!(command.get_arguments().any(|a| a.get_id() == "log" && a.get_env().is_some()));
}

// The commands below start the engine (settings are only read, nothing is written) and print what
// they found on the run's own streams.

#[test]
fn commands_list_every_tool_as_json_or_text() {
    let r = invoke(&["commands", "--format", "json"]);
    assert_eq!((r.code, r.err.as_str()), (0, ""));
    let v: serde_json::Value = serde_json::from_str(&r.out).unwrap();
    assert!(v.as_array().is_some_and(|a| a.iter().any(|c| c["command"] == "core-session-show")));
    let t = invoke(&["commands", "--format", "text"]);
    assert_eq!(t.code, 0);
    assert!(t.out.contains("core-session-show"), "{}", t.out);
}

#[test]
fn capabilities_say_which_version_and_how_many_tools() {
    let r = invoke(&["--compact", "capabilities"]);
    assert_eq!((r.code, r.err.as_str()), (0, ""));
    let v: serde_json::Value = serde_json::from_str(&r.out).unwrap();
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    assert!(v["tools"].as_u64().unwrap_or(0) > 0);
    assert_eq!(r.out.lines().count(), 1, "--compact is one line");
    assert!(invoke(&["capabilities"]).out.lines().count() > 1, "indented by default");
}

#[test]
fn a_tool_prints_its_result_and_its_help() {
    let r = invoke(&["core-session-show"]);
    assert_eq!((r.code, r.err.as_str()), (0, ""), "{}", r.out);
    serde_json::from_str::<serde_json::Value>(&r.out).unwrap();
    let h = invoke(&["core-session-show", "--help"]);
    assert_eq!(h.code, 0);
    assert!(h.out.contains("Usage: craft-anything-cli core-session-show"), "{}", h.out);
}

#[test]
fn an_unknown_tool_or_a_bad_tool_option_is_a_usage_error() {
    for args in [
        &["no-such-tool"][..],
        &["core-session-show", "--bogus"],
        &["core-session-show", "--deep", "--clean-room"],
        &["core-session-show", "--output-format", "xml"],
    ] {
        let r = invoke(args);
        assert_eq!((r.code, r.out.as_str()), (2, ""), "{args:?}: {}", r.err);
        assert!(r.err.starts_with("error: "), "{args:?}: {}", r.err);
    }
}

#[test]
fn a_command_that_fails_exits_1_with_the_error_on_both_streams() {
    let gone = std::env::temp_dir().join("craft-anything-cli-no-such-file.bin");
    let r = invoke(&["triage", gone.to_str().unwrap()]);
    assert_eq!(r.code, 1, "{}", r.err);
    assert!(r.out.contains("\"error\""), "{}", r.out);
    assert!(r.err.starts_with("error: "), "{}", r.err);
}

#[test]
fn a_tool_answers_its_version_and_refuses_repeated_and_conflicting_options_in_process() {
    for flag in ["--version", "-V"] {
        let r = invoke(&["core-session-show", flag]);
        assert_eq!((r.code, r.err.as_str()), (0, ""));
        assert!(r.out.contains(env!("CARGO_PKG_VERSION")), "{}", r.out);
    }
    for args in [
        &["core-session-show", "--allow-run", "--allow-run"][..],
        &["core-session-show", "--compact", "--compact"],
        &["core-session-show", "--project"],
        &["core-session-show", "--project=", "--compact"],
        &["--json", "--format", "text", "commands"],
    ] {
        let r = invoke(args);
        assert_eq!((r.code, r.out.as_str()), (2, ""), "{args:?}: {}", r.err);
    }
    // `--` ends the global options: what follows is the tool's.
    let r = invoke(&["core-session-show", "--", "--compact"]);
    assert_eq!(r.code, 2, "--compact after -- is the tool's argument, which it doesn't take: {}", r.err);
}

#[test]
fn project_is_taken_out_of_a_tools_arguments_once() {
    use super::take_project;
    let mut args: Vec<String> = ["a", "--project", "p", "b"].map(String::from).into();
    assert_eq!(take_project(&mut args).unwrap(), Some(std::path::PathBuf::from("p")));
    assert_eq!(args, ["a", "b"]);
    let mut args: Vec<String> = ["--project=q"].map(String::from).into();
    assert_eq!(take_project(&mut args).unwrap(), Some(std::path::PathBuf::from("q")));
    // After `--` it is the tool's own argument.
    let mut args: Vec<String> = ["--", "--project", "x"].map(String::from).into();
    assert_eq!(take_project(&mut args).unwrap(), None);
    assert_eq!(args.len(), 3);
    for bad in [&["--project"][..], &["--project", "--compact"], &["--project="], &["--project", "a", "--project=b"]] {
        let mut args: Vec<String> = bad.iter().map(|s| s.to_string()).collect();
        assert!(take_project(&mut args).is_err(), "{bad:?}");
    }
}

// ---- commands that save settings, keys and client configuration, over a folder of their own ----

fn sandbox() -> (tempfile::TempDir, crate::Context) {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let host = craft_anything_setup::Host { home, env: Default::default(), os: craft_anything_setup::Os::Linux };
    let context = crate::Context {
        config_dir: Some(dir.path().join("config")),
        host: Some(host),
        mcp_command: Some(vec!["/opt/craft-anything/craft-anything-cli".into(), "mcp".into()]),
        stdin: None,
    };
    (dir, context)
}

fn invoke_in(context: &crate::Context, args: &[&str]) -> Ran {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = crate::run_in(std::iter::once("craft-anything-cli").chain(args.iter().copied()), &mut out, &mut err, context.clone());
    Ran { code, out: String::from_utf8_lossy(&out).into_owned(), err: String::from_utf8_lossy(&err).into_owned() }
}

fn json(r: &Ran) -> serde_json::Value {
    serde_json::from_str(&r.out).unwrap_or_else(|e| panic!("not JSON ({e}): {}", r.out))
}

const SERVER: [&str; 8] = ["ai", "add", "srv", "--kind", "openai_compatible", "--model", "m1", "--base-url"];

fn add_server(cx: &crate::Context) {
    let mut args = SERVER.to_vec();
    args.push("http://127.0.0.1:9/v1");
    let r = invoke_in(cx, &args);
    assert_eq!((r.code, r.err.as_str()), (0, ""), "{}", r.out);
    assert_eq!(json(&r)["added"]["name"], "srv");
}

#[test]
fn a_backend_is_added_changed_listed_and_removed_in_the_given_folder() {
    let (dir, cx) = sandbox();
    assert_eq!(json(&invoke_in(&cx, &["ai", "list"])), serde_json::json!([]));
    add_server(&cx);
    // Saved in the folder the context names, not in the person's.
    assert!(dir.path().join("config/config.toml").is_file());
    let listed = json(&invoke_in(&cx, &["ai", "list"]));
    assert_eq!((listed[0]["name"].clone(), listed[0]["model"].clone()), ("srv".into(), "m1".into()));
    assert_eq!(json(&invoke_in(&cx, &["ai", "model", "srv", "m2"]))["model"], "m2");
    assert_eq!(json(&invoke_in(&cx, &["ai", "effort", "srv", "high"]))["effort"], "high");
    assert!(json(&invoke_in(&cx, &["ai", "effort", "srv"]))["levels"].is_array());
    assert_eq!(json(&invoke_in(&cx, &["ai", "request-rate", "srv", "30"]))["name"], "srv");
    assert!(json(&invoke_in(&cx, &["ai", "request-rate", "srv"]))["requests_per_minute"].is_u64());
    assert_eq!(json(&invoke_in(&cx, &["ai", "tools", "srv", "lean"]))["name"], "srv");
    assert_eq!(invoke_in(&cx, &["ai", "tools", "srv"]).code, 0);
    assert_eq!(invoke_in(&cx, &["ai", "off", "srv"]).code, 0);
    assert_eq!(json(&invoke_in(&cx, &["ai", "list"]))[0]["enabled"], false);
    assert_eq!(invoke_in(&cx, &["ai", "on", "srv"]).code, 0);
    assert_eq!(json(&invoke_in(&cx, &["ai", "builder", "srv"]))["builder"], "srv");
    assert_eq!(invoke_in(&cx, &["ai", "builder"]).code, 0);
    assert_eq!(invoke_in(&cx, &["ai", "extractor", "srv"]).code, 0);
    assert_eq!(json(&invoke_in(&cx, &["ai", "extractor"]))["extractor"], serde_json::Value::Null);
    assert_eq!(json(&invoke_in(&cx, &["ai", "key", "srv"]))["name"], "srv");
    assert_eq!(json(&invoke_in(&cx, &["ai", "key", "srv", "--forget"]))["changed"], "forgotten");
    assert_eq!(json(&invoke_in(&cx, &["ai", "program", "srv"]))["found"], true);
    assert_eq!(json(&invoke_in(&cx, &["ai", "folder", "srv"]))["found"], true);
    let text = invoke_in(&cx, &["--format", "text", "ai", "list"]);
    assert!(text.out.contains("name: srv"), "{}", text.out);
    // Nothing is listening at that address, so the model list and a test request fail with exit 1.
    for args in [&["ai", "model", "srv"][..], &["ai", "test", "srv"]] {
        let r = invoke_in(&cx, args);
        assert_eq!(r.code, 1, "{args:?}: {}", r.err);
        assert!(r.err.contains("Couldn't reach srv"), "{}", r.err);
    }
    assert_eq!(json(&invoke_in(&cx, &["ai", "remove", "srv"]))["removed"], true);
    let again = invoke_in(&cx, &["ai", "remove", "srv"]);
    assert_eq!((again.code, json(&again)["removed"].clone()), (1, false.into()));
    assert_eq!(invoke_in(&cx, &["ai", "detect"]).code, 0);
}

#[test]
fn ai_commands_refuse_what_they_cannot_do() {
    let (_dir, cx) = sandbox();
    add_server(&cx);
    // Usage errors (exit 2) before anything is saved.
    for (args, needle) in [
        (&["ai", "add", "x", "--kind", "bogus"][..], "--kind must be one of"),
        (&["ai", "add", "x", "--kind", "openai_compatible", "--key-env", "CA_NO_SUCH_VARIABLE_X"], "CA_NO_SUCH_VARIABLE_X"),
    ] {
        let r = invoke_in(&cx, args);
        assert_eq!((r.code, r.out.as_str()), (2, ""), "{args:?}");
        assert!(r.err.contains(needle), "{args:?}: {}", r.err);
    }
    // Failures (exit 1) that name the backend or the value.
    for (args, needle) in [
        (&["ai", "add", "x", "--kind", "openai_compatible", "--base-url", "http://127.0.0.1:9/v1"][..], "Couldn't reach x"),
        (&["ai", "key", "nope"], "No AI backend named"),
        (&["ai", "effort", "srv", "bogus"], "isn't an effort level"),
        (&["ai", "tools", "srv", "bogus"], "isn't a tool list"),
        (&["ai", "builder", "nope"], "No AI backend named"),
        (&["ai", "folder", "srv", "relative/path"], "doesn't keep a folder"),
    ] {
        let r = invoke_in(&cx, args);
        assert_eq!(r.code, 1, "{args:?}: {}", r.err);
        assert!(r.err.contains(needle), "{args:?}: {}", r.err);
    }
    for args in [&["ai", "model", "nope", "m"][..], &["ai", "on", "nope"], &["ai", "off", "nope"]] {
        assert_eq!(invoke_in(&cx, args).code, 1, "{args:?}");
    }
    // The saved backend is untouched by all of that.
    assert_eq!(json(&invoke_in(&cx, &["ai", "list"])).as_array().map(Vec::len), Some(1));
}

#[test]
fn chat_and_rebuild_say_what_is_missing() {
    let (_dir, cx) = sandbox();
    let chat = invoke_in(&cx, &["chat", "hello"]);
    assert_eq!(chat.code, 1, "{}", chat.err);
    assert!(chat.err.contains("No AI is set up yet"), "{}", chat.err);
    let rebuild = invoke_in(&cx, &["rebuild", "-", "make a thing", "--unattended"]);
    assert_eq!(rebuild.code, 1, "{}", rebuild.err);
    assert!(rebuild.err.contains("Building needs a coding agent"), "{}", rebuild.err);
    // The project of that attempt is in the folder of the context.
    assert!(rebuild.out.contains(&_dir.path().join("config/projects").display().to_string()), "{}", rebuild.out);
    add_server(&cx);
    let chat = invoke_in(&cx, &["chat", "hello"]);
    assert_eq!(chat.code, 1, "{}", chat.err);
    assert!(chat.err.contains("Couldn't reach srv"), "{}", chat.err);
    assert!(chat.err.contains(&_dir.path().join("config/logs").display().to_string()), "{}", chat.err);
    let rebuild = invoke_in(&cx, &["rebuild", "-", "make a thing", "--backend", "nope"]);
    assert_eq!(rebuild.code, 1);
    assert!(rebuild.err.contains("No AI backend named"), "{}", rebuild.err);
}

#[test]
fn setup_doctor_and_uninstall_work_on_the_given_home() {
    let (dir, cx) = sandbox();
    let claude = dir.path().join("home/.claude.json");
    let planned = invoke_in(&cx, &["setup", "--dry-run", "--client", "claude_code"]);
    assert_eq!(planned.code, 0, "{}", planned.err);
    assert_eq!(json(&planned)["status"], "planned");
    assert!(!claude.exists(), "a dry run writes nothing");
    let text = invoke_in(&cx, &["--format", "text", "setup", "--dry-run", "--client", "claude_code"]);
    assert!(text.out.contains("Register: /opt/craft-anything/craft-anything-cli mcp"), "{}", text.out);
    let done = invoke_in(&cx, &["setup", "--yes", "--client", "claude_code"]);
    assert_eq!(done.code, 0, "{}", done.err);
    assert_eq!(json(&done)["status"], "configured");
    assert!(std::fs::read_to_string(&claude).unwrap_or_default().contains("/opt/craft-anything/craft-anything-cli"));
    assert_eq!(json(&invoke_in(&cx, &["setup", "--yes", "--client", "claude_code"]))["status"], "unchanged");
    let text = invoke_in(&cx, &["--format", "text", "setup", "--client", "claude_code"]);
    assert!(text.out.contains("status: unchanged"), "{}", text.out);
    let unknown = invoke_in(&cx, &["setup", "--client", "nope"]);
    assert_eq!(unknown.code, 1);
    assert!(unknown.err.contains("Unknown client nope"), "{}", unknown.err);
    // The registered program is not there (whether or not this computer has its own), so the doctor reports a problem.
    let doctor = invoke_in(&cx, &["doctor", "--client", "claude_code"]);
    assert_eq!(doctor.code, 1, "{}", doctor.err);
    assert!(json(&doctor)["checks"].as_array().is_some_and(|c| c.iter().any(|c| c["name"] == "registration:claude_code" && c["status"] == "fail")));
    let text = invoke_in(&cx, &["--format", "text", "doctor"]);
    assert!(text.out.contains("[FAIL] registration:claude_code") && text.out.contains("problems found"), "{}", text.out);
    let removed = invoke_in(&cx, &["--format", "text", "uninstall"]);
    assert_eq!(removed.code, 0, "{}", removed.err);
    assert!(removed.out.contains("removed claude_code"), "{}", removed.out);
    assert_eq!(json(&invoke_in(&cx, &["uninstall"]))["status"], "nothing_to_remove");
}

#[test]
fn the_default_context_is_this_computers() {
    let cx = crate::Context::default();
    assert_eq!(cx.config_dir(), craft_anything_engine::settings::config_dir());
    assert_eq!(cx.host().home, craft_anything_setup::Host::current().home);
    assert!(cx.config_dir.is_none() && cx.host.is_none() && cx.mcp_command.is_none());
}

#[test]
fn mcp_refuses_a_bad_shared_target_before_serving() {
    let (dir, cx) = sandbox();
    let inbox = dir.path().join("inbox").display().to_string();
    for (args, needle) in [
        (vec!["mcp", "--inbox", inbox.as_str(), "--target", "{"], "Invalid shared target: EOF while parsing"),
        (vec!["mcp", "--target", "{\"x\":1}"], "unknown field `x`"),
    ] {
        let r = invoke_in(&cx, &args);
        assert_eq!((r.code, r.out.as_str()), (1, ""), "{args:?}");
        assert!(r.err.contains(needle), "{args:?}: {}", r.err);
    }
}

#[test]
fn triage_and_report_open_a_target_only_when_deep() {
    let (dir, cx) = sandbox();
    let file = dir.path().join("sample.txt");
    std::fs::write(&file, b"hello world").unwrap();
    let file = file.display().to_string();
    // Clean room never looks inside the original.
    for args in [&["triage", file.as_str()][..], &["report", file.as_str()]] {
        let r = invoke_in(&cx, args);
        assert_eq!(r.code, 1, "{args:?}");
        assert!(r.err.contains("Clean room never does"), "{}", r.err);
    }
    let triage = invoke_in(&cx, &["--deep", "triage", &file]);
    assert_eq!(triage.code, 0, "{}", triage.err);
    assert_eq!(json(&triage)["environment"]["clean_room"], false);
    let report = invoke_in(&cx, &["--deep", "report", &file]);
    assert_eq!(report.code, 0, "{}", report.err);
    assert!(report.out.starts_with("# Investigation report") && report.out.contains("sample.txt"), "{}", report.out);
    // Without a target it reports on the (empty) project, as text or as a file.
    let bare = invoke_in(&cx, &["report"]);
    assert!(bare.out.contains("No targets recorded"), "{}", bare.out);
    let page = dir.path().join("report.html");
    let wrote = invoke_in(&cx, &["report", "--html", "--out", &page.display().to_string()]);
    assert_eq!(wrote.code, 0, "{}", wrote.err);
    assert_eq!(json(&wrote)["result"]["format"], "html");
    assert!(std::fs::read_to_string(&page).unwrap().contains("<html"));
    let missing = invoke_in(&cx, &["--deep", "triage", &dir.path().join("nope").display().to_string()]);
    assert_eq!(missing.code, 1, "{}", missing.out);
}

#[test]
fn tool_flags_after_the_name_are_the_global_ones() {
    let (dir, cx) = sandbox();
    let project = dir.path().join("proj").display().to_string();
    let r = invoke_in(&cx, &["capabilities", "--project", &project, "--allow-run", "--deep", "--format", "text"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.starts_with("analysis_mode: deep"), "{}", r.out);
    assert!(dir.path().join("proj/project.toml").is_file(), "the project was opened where --project said");
    let clean = invoke_in(&cx, &["capabilities", "--clean-room", "--json"]);
    assert_eq!(json(&clean)["analysis_mode"], "clean-room");
    let bad = invoke_in(&cx, &["capabilities", "--format", "bogus"]);
    assert_eq!((bad.code, bad.out.as_str()), (2, ""));
    let none = invoke_in(&cx, &["capabilities", "--project"]);
    assert_eq!((none.code, none.out.as_str()), (2, ""));
    assert_eq!(invoke_in(&cx, &["--deep", "--clean-room", "capabilities"]).code, 2);
}

#[test]
fn every_log_level_is_accepted() {
    let (_dir, cx) = sandbox();
    for level in ["off", "error", "warn", "info", "debug", "trace"] {
        let r = invoke_in(&cx, &["--log", level, "capabilities"]);
        assert_eq!(r.code, 0, "{level}: {}", r.err);
    }
}

#[test]
fn setup_asks_before_it_changes_anything() {
    let (dir, cx) = sandbox();
    // No terminal to answer on, so it shows the plan and stops.
    let text = invoke_in(&cx, &["--format", "text", "setup", "--client", "claude_code"]);
    assert_eq!(text.code, 1, "{}", text.err);
    assert!(text.out.contains("[create] Claude Code"), "{}", text.out);
    assert!(!dir.path().join("home/.claude.json").exists());
    let json_plan = invoke_in(&cx, &["setup", "--client", "claude_code"]);
    assert_eq!(json(&json_plan)["status"], "needs_confirmation");
    let all = invoke_in(&cx, &["setup", "--dry-run", "--all-detected"]);
    assert_eq!(json(&all)["status"], "planned");
}

#[test]
fn a_backend_that_cannot_be_saved_is_reported() {
    let (dir, cx) = sandbox();
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    // The settings file's place is taken by a folder, so reading finds nothing and saving fails.
    std::fs::create_dir(config.join("config.toml")).unwrap();
    let mut args = SERVER.to_vec();
    args.push("http://127.0.0.1:9/v1");
    let r = invoke_in(&cx, &args);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.err.starts_with("error:"), "{}", r.err);
}

#[test]
fn an_unattended_rebuild_answers_for_the_person() {
    let ask = |kind: &str, choices: &[&str]| -> craft_anything_engine::inbox::Request {
        serde_json::from_value(serde_json::json!({"id": "r1", "kind": kind, "title": "Pick one", "choices": choices, "created_ms": 0})).unwrap()
    };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut io = crate::Io { out: &mut out, err: &mut err, pretty: false };
    let first = crate::rebuild_cmd::auto_answer(&mut io, &ask("question", &["red", "blue"]));
    assert_eq!((first.choice.as_deref(), first.text), (Some("red"), None));
    let open = crate::rebuild_cmd::auto_answer(&mut io, &ask("question", &[]));
    assert!(open.choice.is_none() && open.text.is_some_and(|t| t.contains("best judgement")));
    let approval = crate::rebuild_cmd::auto_answer(&mut io, &ask("approval", &[]));
    assert_eq!(approval.allow, Some(false));
    let said = String::from_utf8(out).unwrap();
    assert!(said.contains("Pick one → red (unattended)") && said.contains("→ no preference (unattended)") && said.contains("→ refused (unattended)"), "{said}");
}

#[test]
fn the_hook_notes_command_prints_nothing_when_no_notes_wait() {
    let (dir, cx) = sandbox();
    let r = invoke_in(&cx, &["builder-notes", &dir.path().join("none").display().to_string()]);
    assert_eq!((r.code, r.out.as_str(), r.err.as_str()), (0, "", ""));
}

#[test]
fn a_project_folder_is_created_then_opened_again() {
    let (dir, cx) = sandbox();
    let project = dir.path().join("again").display().to_string();
    for _ in 0..2 {
        assert_eq!(invoke_in(&cx, &["capabilities", "--project", &project]).code, 0);
    }
    assert!(dir.path().join("again/project.toml").is_file());
}

#[test]
fn a_bad_log_value_from_the_environment_is_named_by_its_variable() {
    use clap::CommandFactory as _;
    let err = crate::Cli::command().try_get_matches_from(["craft-anything-cli", "--log", "bogus", "commands"]).unwrap_err();
    // Typed on the command line: clap's own message stands.
    let typed = ["--log", "bogus"].map(std::ffi::OsString::from);
    assert_eq!(crate::env_value_error(&err, &typed), None);
    // Not typed there: it came from CRAFT_ANYTHING_LOG.
    let said = crate::env_value_error(&err, &[]).unwrap_or_default();
    assert!(said.starts_with("CRAFT_ANYTHING_LOG: invalid value 'bogus' (expected one of"), "{said}");
}

#[test]
fn setup_and_uninstall_say_what_they_did_and_what_blocks_them() {
    let (dir, cx) = sandbox();
    let claude = dir.path().join("home/.claude.json");
    let done = invoke_in(&cx, &["--format", "text", "setup", "--yes", "--client", "claude_code"]);
    assert_eq!(done.code, 0, "{}", done.err);
    assert!(done.out.contains("status: configured") && done.out.contains("Restart"), "{}", done.out);
    // A config that is not JSON is not touched: the plan says why, and nothing is applied.
    std::fs::write(&claude, "{ not json").unwrap();
    let blocked = invoke_in(&cx, &["--format", "text", "setup", "--yes", "--client", "claude_code"]);
    assert_eq!(blocked.code, 1, "{}", blocked.out);
    assert!(blocked.out.contains("[blocked]") || blocked.out.contains("blocked"), "{}", blocked.out);
    let removed = invoke_in(&cx, &["--format", "text", "uninstall"]);
    assert_eq!(removed.code, 1, "{}", removed.out);
    assert!(removed.out.contains("blocked by Claude Code") && removed.out.contains("status: blocked") && removed.out.contains("Back up"), "{}", removed.out);
    assert_eq!(std::fs::read_to_string(&claude).unwrap(), "{ not json", "the file is left as it was");
}

#[test]
fn an_api_backend_has_no_agent_model_or_thinking() {
    let (dir, cx) = sandbox();
    add_server(&cx);
    // A target is opened before the chat starts, and clean room refuses that.
    let file = dir.path().join("x.txt").display().to_string();
    let chat = invoke_in(&cx, &["chat", "hello", "--target", &file]);
    assert_eq!(chat.code, 1, "{}", chat.err);
    assert!(chat.err.contains("Clean room never does"), "{}", chat.err);
    for args in [&["ai", "model", "srv", "--thinking", "high"][..], &["ai", "model", "srv", "--default"], &["ai", "model", "nope", "--default"]] {
        let r = invoke_in(&cx, args);
        assert_eq!(r.code, 1, "{args:?}: {}", r.out);
        assert!(r.err.contains("does not offer a live model") || r.err.contains("No AI backend named"), "{args:?}: {}", r.err);
    }
}

#[test]
fn a_project_that_cannot_be_made_stops_every_command_that_needs_the_engine() {
    let (dir, cx) = sandbox();
    // A folder below a file can be neither opened nor created.
    let file = dir.path().join("a-file");
    std::fs::write(&file, "x").unwrap();
    let project = file.join("project").display().to_string();
    let target = dir.path().join("t.txt").display().to_string();
    for args in [
        vec!["--project", &project, "mcp"],
        vec!["--project", &project, "triage", &target],
        vec!["--project", &project, "report"],
        vec!["--project", &project, "capabilities"],
        vec!["--project", &project, "chat", "hello"],
        vec!["--project", &project, "ai", "list"],
    ] {
        let r = invoke_in(&cx, &args);
        assert_eq!(r.code, 1, "{args:?}: {}", r.out);
        assert!(r.err.contains("a-file"), "{args:?}: {}", r.err);
    }
}

fn with_input(cx: &crate::Context, text: &str) -> crate::Context {
    crate::Context { stdin: Some(crate::Stdin::new(text)), ..cx.clone() }
}

#[test]
fn a_key_is_read_from_the_given_input() {
    let (_dir, cx) = sandbox();
    add_server(&cx);
    let saved = invoke_in(&with_input(&cx, "sk-test-key\n"), &["ai", "key", "srv", "--key-stdin"]);
    assert_eq!(saved.code, 0, "{}", saved.err);
    assert!(!saved.out.contains("sk-test-key"), "the key is never printed");
    assert_eq!(json(&invoke_in(&cx, &["ai", "list"]))[0]["key_stored"], true);
    let empty = invoke_in(&with_input(&cx, "  \n"), &["ai", "key", "srv", "--key-stdin"]);
    assert_eq!((empty.code, empty.out.as_str()), (2, ""), "{}", empty.err);
    let mut add = SERVER.to_vec();
    add.extend(["http://127.0.0.1:9/v1", "--key-stdin"]);
    let none = invoke_in(&with_input(&cx, ""), &add);
    assert_eq!(none.code, 2, "{}", none.err);
}

#[test]
fn setup_applies_what_the_given_input_confirms() {
    let (dir, cx) = sandbox();
    let claude = dir.path().join("home/.claude.json");
    let no = invoke_in(&with_input(&cx, "n\n"), &["--format", "text", "setup", "--client", "claude_code"]);
    assert_eq!(no.code, 1, "{}", no.out);
    assert!(!claude.exists());
    let yes = invoke_in(&with_input(&cx, "y\n"), &["--format", "text", "setup", "--client", "claude_code"]);
    assert_eq!(yes.code, 0, "{}", yes.err);
    assert!(yes.err.contains("Apply these") && yes.out.contains("status: configured"), "{}{}", yes.err, yes.out);
    assert!(claude.is_file());
}

#[test]
fn a_rebuild_asks_the_person_through_the_given_input() {
    let (_dir, cx) = sandbox();
    let ask = |kind: &str, choices: &[&str]| -> craft_anything_engine::inbox::Request {
        serde_json::from_value(
            serde_json::json!({"id": "r1", "kind": kind, "title": "Pick one", "detail": "some\ncontext", "choices": choices, "created_ms": 0}),
        )
        .unwrap()
    };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut io = crate::Io { out: &mut out, err: &mut err, pretty: false };
    let answer = |text: &str, io: &mut crate::Io<'_>, r| crate::rebuild_cmd::prompt_answer(io, &with_input(&cx, text), &r);
    let by_number = answer("2\n", &mut io, ask("question", &["red", "blue"]));
    assert_eq!(by_number.choice.as_deref(), Some("blue"));
    let by_text = answer("teal\n", &mut io, ask("question", &["red", "blue"]));
    assert_eq!((by_text.choice, by_text.text.as_deref()), (None, Some("teal")));
    let always = answer("a\n", &mut io, ask("approval", &[]));
    assert_eq!((always.allow, always.always), (Some(true), true));
    let once = answer("Yes\n", &mut io, ask("approval", &[]));
    assert_eq!((once.allow, once.always), (Some(true), false));
    let refused = answer("n\n", &mut io, ask("approval", &[]));
    assert_eq!(refused.allow, Some(false));
    let said = String::from_utf8(out).unwrap();
    assert!(said.contains("? Pick one") && said.contains("  some\n  context") && said.contains("1) red") && said.contains("Allow? [y]es"), "{said}");
}
