//! ZCode, Z.ai's coding agent (`zcode.cjs -p <message> --mode <mode>`).
//!
//! ZCode ships as an Electron desktop app with no command on PATH. Its headless runtime is a
//! script inside the app, `resources/glm/zcode.cjs`, which runs under plain Node or under the
//! app's own executable with `ELECTRON_RUN_AS_NODE=1`, so no separate Node install is needed
//! (checked against ZCode 0.17.1 on Windows, #1781). `-p/--prompt <text>` runs one prompt without
//! opening its terminal interface and exits; `--mode build|edit|plan|yolo|guarded` picks what it may
//! do by itself.
//!
//! What isn't there yet, and so isn't used:
//! - The message only goes in argv (`-p <text>`): there is no stdin or file form we know of, so a
//!   message longer than the command line can hold is refused before anything starts
//!   ([`check_argv`](AgentAdapter::check_argv)), rather than failing with os error 206 (#402).
//! - `--json` exists, but no sample of its events has been read, so its output is taken as text.
//! - `--resume <sessionId>` exists, but the text output names no session, so nothing is resumed.
//! - There is no `--model` flag: the model is the one picked once in ZCode's terminal interface.
//!   A machine that has only used the desktop app fails with "Select a model before sending a
//!   message", which is turned into the fix ([`parse_stderr`](AgentAdapter::parse_stderr)).
//! - Our tool server isn't handed over: ZCode reads servers from `~/.zcode/cli/config.json`
//!   (where Setup can register ours), but nothing names this run's project there and no run has
//!   confirmed a headless turn reads it.
//! - Nothing in `-p` asks us before an edit or a command; the mode decides alone.
//!
//! Questions about a build aren't sent to it ([`answers_questions`](AgentAdapter::answers_questions)
//! stays false): `--mode plan` has not been seen to keep it to reading.

use super::*;

/// The headless runtime inside the desktop app's folder.
const RUNTIME: [&str; 3] = ["resources", "glm", "zcode.cjs"];

/// Makes Electron's executable run a script as Node does, with no window.
pub const AS_NODE: (&str, &str) = ("ELECTRON_RUN_AS_NODE", "1");

/// The longest command line we hand ZCode, in the units the system counts: Windows holds 32,767
/// UTF-16 units for the whole line (some go to quoting); Linux holds 131,072 bytes in one argument.
#[cfg(windows)]
const COMMAND_LINE_LIMIT: usize = 32_000;
#[cfg(not(windows))]
const COMMAND_LINE_LIMIT: usize = 131_000;

/// ZCode run as a coding agent: one prompt in, its answer as text out.
pub struct ZCode;

/// How to start ZCode for a chosen program: what to run, the script it runs first (if any), and
/// whether that program is Electron's executable to be run as Node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub program: String,
    pub script: Option<std::path::PathBuf>,
    pub as_node: bool,
}

/// The desktop app's executable beside a runtime script at `resources/glm/zcode.cjs`.
fn app_beside(script: &Path) -> Option<std::path::PathBuf> {
    let dir = script.parent()?.parent()?.parent()?;
    ["ZCode.exe", "ZCode", "zcode"].iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

/// Whether `program` is the desktop app's executable: named ZCode, and either a Windows program
/// or one with the app's `resources` folder beside it. A `zcode` command with neither is run as
/// it is.
fn is_app(program: &Path) -> bool {
    let named = program.file_stem().is_some_and(|s| s.eq_ignore_ascii_case("zcode"));
    let exe = program.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"));
    let beside = program.parent().is_some_and(|d| !d.as_os_str().is_empty() && d.join(RUNTIME[0]).is_dir());
    named && (exe || beside)
}

/// How the chosen `program` starts ZCode's headless runtime. The desktop app is always run as Node
/// with its runtime script first, even when that script is missing (an older ZCode): Node then
/// fails to find it and nothing opens a window, and the app never reads our `-p` as its own.
pub fn launch(program: &str) -> Launch {
    let path = Path::new(program);
    let script_named = path.extension().is_some_and(|e| ["cjs", "js", "mjs"].iter().any(|x| e.eq_ignore_ascii_case(x)));
    if script_named {
        return match app_beside(path) {
            Some(app) => Launch { program: app.display().to_string(), script: Some(path.to_path_buf()), as_node: true },
            None => Launch { program: "node".into(), script: Some(path.to_path_buf()), as_node: false },
        };
    }
    if is_app(path) {
        let dir = path.parent().unwrap_or(Path::new(""));
        let script = RUNTIME.iter().fold(dir.to_path_buf(), |p, part| p.join(part));
        return Launch { program: program.to_string(), script: Some(script), as_node: true };
    }
    Launch { program: program.to_string(), script: None, as_node: false }
}

/// The length of the command line that starts `launch` with `args`, as the system counts it, with
/// room for a space and quotes around each part.
fn command_line_len(launch: &Launch, args: &[String]) -> usize {
    #[cfg(windows)]
    let len = |s: &str| s.encode_utf16().count();
    #[cfg(not(windows))]
    let len = |s: &str| s.len();
    let script = launch.script.as_ref().map(|s| s.display().to_string());
    std::iter::once(launch.program.as_str()).chain(script.as_deref()).chain(args.iter().map(String::as_str)).map(|part| len(part) + 3).sum()
}

/// Refuse a command line over `limit`, saying why and what to do instead.
fn fits(launch: &Launch, args: &[String], limit: usize) -> Result<(), ToolError> {
    let needed = command_line_len(launch, args);
    if needed <= limit {
        return Ok(());
    }
    Err(ToolError::new(
        ErrorCode::OverLimit,
        format!(
            "This message is too long for ZCode: ZCode takes its message on the command line, which holds about {limit} characters here, and this one needs {needed}."
        ),
    )
    .with_remediation(
        "Ask in fewer words, or use a coding agent that reads its message on stdin (Claude Code or Codex). A build's brief is usually longer than ZCode can take this way.",
    ))
}

/// What ZCode says when no model has been picked on this computer yet.
const NO_MODEL: &str = "select a model before sending a message";

impl AgentAdapter for ZCode {
    fn ids(&self) -> &'static [&'static str] {
        &["zcode"]
    }

    fn args(&self, cx: &ArgsCx) -> Vec<String> {
        let mut v = cx.options.args.clone();
        // A mode the person chose in the backend's arguments is theirs; otherwise a builder builds
        // and anything else only plans.
        if !v.iter().any(|a| a == "--mode" || a.starts_with("--mode=")) {
            v.extend(["--mode".into(), if cx.builder { "build" } else { "plan" }.into()]);
        }
        v.extend(["-p".into(), format!("{}{}", cx.preamble, cx.prompt)]);
        v
    }

    fn streams_json(&self) -> bool {
        false
    }

    /// `zcode.cjs --help` lists `-p, --prompt <text>` (ZCode 0.17.1).
    fn required_flags(&self) -> (&'static [&'static str], &'static [&'static str]) {
        (&["--help"], &["--prompt"])
    }

    /// The plain adapter would start the desktop app with the message as its argument.
    fn plain_fallback(&self) -> bool {
        false
    }

    fn missing_flags_error(&self) -> Option<(&'static str, &'static str)> {
        Some((
            "This ZCode has no headless mode: its help doesn't list --prompt.",
            "Update ZCode (0.17.1 has it), or choose its resources/glm/zcode.cjs as the program in Settings → AI.",
        ))
    }

    /// Its connection test reads the help, which ZCode is known to answer; `--version` isn't.
    fn version_args(&self) -> &'static [&'static str] {
        &["--help"]
    }

    /// The desktop app runs as Node with its runtime script first; a script chosen by itself runs
    /// under the app beside it, else under Node.
    fn process(&self, program: &str) -> Process {
        let l = launch(program);
        let mut p = Process::new(&l.program);
        if l.as_node {
            p = p.env(AS_NODE.0, AS_NODE.1);
        }
        match l.script {
            Some(script) => p.arg(script),
            None => p,
        }
    }

    fn check_argv(&self, program: &str, args: &[String]) -> Result<(), ToolError> {
        fits(&launch(program), args, COMMAND_LINE_LIMIT)
    }

    /// Its runtime lives inside the desktop app on this computer; a run over SSH would start that
    /// app's executable without it.
    fn check_remote_options(&self, _options: &BackendOptions, host: &str, _stop: &Stop) -> Result<(), ToolError> {
        Err(ToolError::new(ErrorCode::BadSettings, format!("ZCode can't be run on {host} over SSH here.")).with_remediation(
            "Remove the SSH host from this AI in Settings → AI and run ZCode on this computer, or use another coding agent on that computer.",
        ))
    }

    fn parse_stderr(&self, _p: &mut StreamParser, line: &str) -> Vec<AgentEvent> {
        if !line.to_ascii_lowercase().contains(NO_MODEL) {
            return Vec::new();
        }
        vec![AgentEvent::Error {
            message: "ZCode hasn't been given a model on this computer yet.".into(),
            remediation: Some(
                "Start ZCode's terminal interface once (its resources/glm/zcode.cjs, without -p) and pick a model, then try again. ZCode has no --model flag for runs like this one yet."
                    .into(),
            ),
            log: None,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn mcp() -> McpCommand {
        McpCommand { command: "craft-anything-cli".into(), args: vec!["mcp".into()], notes: None }
    }

    fn args_of(options: &BackendOptions, access: Access, prompt: &str) -> Vec<String> {
        build_args(&ZCode, options, prompt, "PRE ", &mcp(), None, access)
    }

    /// A desktop app layout in a temp folder: the executable, and its runtime unless `runtime` is false.
    fn app(dir: &Path, exe: &str, runtime: bool) -> (PathBuf, PathBuf) {
        let exe = dir.join(exe);
        std::fs::write(&exe, b"").unwrap();
        let script = dir.join("resources").join("glm").join("zcode.cjs");
        std::fs::create_dir_all(script.parent().unwrap()).unwrap();
        if runtime {
            std::fs::write(&script, b"").unwrap();
        }
        (exe, script)
    }

    #[test]
    fn registered_and_never_run_plain() {
        let adapter = AgentAdapters::built_in().for_agent("zcode");
        assert_eq!(adapter.ids(), ["zcode"], "registered, not the plain adapter");
        assert_eq!(adapter.required_flags(), (&["--help"][..], &["--prompt"][..]));
        assert!(!adapter.plain_fallback() && !adapter.streams_json() && !adapter.resumes() && !adapter.answers_questions());
        assert!(!adapter.prompt_on_stdin() && !adapter.reads_message_file(), "only -p <text> is known");
        assert!(adapter.missing_flags_error().is_some());
        assert_eq!(program_for("zcode"), "zcode");
        assert_eq!(display_for("zcode"), "ZCode");
    }

    #[test]
    fn a_builder_builds_and_anything_else_plans() {
        let o = BackendOptions::default();
        assert_eq!(args_of(&o, Access::Build, "make a game"), ["--mode", "build", "-p", "PRE make a game"]);
        assert_eq!(args_of(&o, Access::CleanRoomBuild, "make a game"), ["--mode", "build", "-p", "PRE make a game"]);
        assert_eq!(args_of(&o, Access::Analyse, "what is it?"), ["--mode", "plan", "-p", "PRE what is it?"]);
        // A leading dash isn't read as a flag.
        assert_eq!(args_of(&o, Access::Analyse, "-x"), ["--mode", "plan", "-p", "PRE  -x"]);
    }

    #[test]
    fn a_mode_in_the_backends_arguments_is_kept() {
        for chosen in [vec!["--mode".to_string(), "guarded".to_string()], vec!["--mode=guarded".to_string()]] {
            let o = BackendOptions { args: chosen.clone(), ..Default::default() };
            let a = args_of(&o, Access::Build, "go");
            assert_eq!(a[..chosen.len()], chosen[..], "{a:?}");
            assert_eq!(a.iter().filter(|x| x.starts_with("--mode")).count(), 1, "{a:?}");
            assert_eq!(a[a.len() - 2..], ["-p", "PRE go"], "{a:?}");
        }
    }

    #[test]
    fn the_desktop_app_runs_its_runtime_as_node() {
        let d = tempfile::tempdir().unwrap();
        let (exe, script) = app(d.path(), "ZCode.exe", true);
        let l = launch(&exe.display().to_string());
        assert_eq!(l, Launch { program: exe.display().to_string(), script: Some(script), as_node: true });
    }

    /// An app without its runtime still runs as Node with the missing script first: Node fails to
    /// find it rather than the window opening, or Node reading `-p` as code to print.
    #[test]
    fn an_app_without_its_runtime_still_runs_as_node() {
        let d = tempfile::tempdir().unwrap();
        let (exe, script) = app(d.path(), "ZCode.exe", false);
        let l = launch(&exe.display().to_string());
        assert!(l.as_node && l.script.as_deref() == Some(script.as_path()) && !script.exists(), "{l:?}");
    }

    #[test]
    fn a_chosen_runtime_runs_under_the_app_beside_it_or_node() {
        let d = tempfile::tempdir().unwrap();
        let (exe, script) = app(d.path(), "ZCode.exe", true);
        let l = launch(&script.display().to_string());
        assert_eq!(l, Launch { program: exe.display().to_string(), script: Some(script.clone()), as_node: true });
        // Copied somewhere on its own: plain Node.
        let alone = tempfile::tempdir().unwrap();
        let copy = alone.path().join("zcode.cjs");
        std::fs::write(&copy, b"").unwrap();
        assert_eq!(launch(&copy.display().to_string()), Launch { program: "node".into(), script: Some(copy), as_node: false });
    }

    /// A `zcode` command (on PATH, or a wrapper) with no app folder beside it runs as it is.
    #[test]
    fn a_zcode_command_runs_as_it_is() {
        assert_eq!(launch("zcode"), Launch { program: "zcode".into(), script: None, as_node: false });
        let d = tempfile::tempdir().unwrap();
        let wrapper = d.path().join("zcode");
        std::fs::write(&wrapper, b"").unwrap();
        assert!(!launch(&wrapper.display().to_string()).as_node);
        let other = d.path().join("other.exe");
        assert!(!launch(&other.display().to_string()).as_node, "only ZCode's executable");
    }

    #[test]
    fn a_message_too_long_for_the_command_line_is_refused() {
        let l = Launch { program: "ZCode.exe".into(), script: Some(PathBuf::from("zcode.cjs")), as_node: true };
        let short = args_of(&BackendOptions::default(), Access::Build, "make a game");
        assert!(fits(&l, &short, 1_000).is_ok());
        let long = args_of(&BackendOptions::default(), Access::Build, &"x".repeat(2_000));
        let e = fits(&l, &long, 1_000).unwrap_err();
        assert_eq!(e.code, ErrorCode::OverLimit);
        assert!(e.message.contains("too long for ZCode") && e.message.contains("1000"), "{}", e.message);
        assert!(e.remediation.as_deref().is_some_and(|r| r.contains("stdin")), "{e:?}");
        // The real limit: a 40,000-character brief is refused on Windows, where #402 happened.
        let brief = args_of(&BackendOptions::default(), Access::Build, &"x".repeat(40_000));
        assert_eq!(ZCode.check_argv("ZCode.exe", &brief).is_err(), cfg!(windows));
        assert!(ZCode.check_argv("ZCode.exe", &short).is_ok());
    }

    #[test]
    fn no_model_yet_says_how_to_pick_one() {
        let mut p = StreamParser::new(Arc::new(ZCode));
        let events = ZCode.parse_stderr(&mut p, "Error: Select a model before sending a message");
        assert!(
            matches!(events.as_slice(), [AgentEvent::Error { message, remediation: Some(fix), .. }] if message.contains("model") && fix.contains("pick a model")),
            "{events:?}"
        );
        assert!(ZCode.parse_stderr(&mut p, "warning: something else").is_empty());
    }

    #[test]
    fn ssh_is_refused() {
        let e = ZCode.check_remote_options(&BackendOptions::default(), "box", &Stop::default()).unwrap_err();
        assert!(e.message.contains("box"), "{}", e.message);
    }

    /// A missing program is an error with a fix, not a hang or a crash.
    #[test]
    fn a_missing_program_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let missing = d.path().join("ZCode.exe");
        let config =
            BackendConfig { name: "ZCode".into(), kind: crate::BackendKind::AgentCli, model: "zcode".into(), base_url: None, ssh_host: None, enabled: true };
        let options = BackendOptions { command: Some(missing.display().to_string()), ..Default::default() };
        let mut c = CliConversation::new(CliSpec::new(config, options, false, None));
        let all = turn(&mut c, "hello");
        assert!(all.iter().any(|e| matches!(e, AgentEvent::Error { .. })), "{all:?}");
        assert!(all.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. })), "{all:?}");
    }

    fn turn(c: &mut CliConversation, text: &str) -> Vec<AgentEvent> {
        c.send(text);
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let mut all = Vec::new();
        while std::time::Instant::now() < deadline && !all.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. })) {
            all.extend(c.poll());
            std::thread::sleep(Duration::from_millis(10));
        }
        all
    }

    /// A stand-in desktop app: its executable is a script named `ZCode` with the runtime beside it.
    /// It answers `--help` like ZCode 0.17.1, and otherwise records how it was started.
    #[cfg(unix)]
    fn stand_in(dir: &Path, run: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let (exe, _) = app(dir, "ZCode", true);
        let script = format!(
            "#!/bin/sh\nd=$(dirname \"$0\")\nif [ \"$2\" = --help ]; then echo '  -p, --prompt <text>  Run a single prompt without opening the TUI'; exit 0; fi\n\
             printf '%s\\n' \"ELECTRON_RUN_AS_NODE=$ELECTRON_RUN_AS_NODE\" \"$@\" > \"$d/argv\"\n{run}\n"
        );
        std::fs::write(&exe, script).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        exe
    }

    #[cfg(unix)]
    fn conversation(exe: &Path) -> CliConversation {
        let config =
            BackendConfig { name: "ZCode".into(), kind: crate::BackendKind::AgentCli, model: "zcode".into(), base_url: None, ssh_host: None, enabled: true };
        let options = BackendOptions { command: Some(exe.display().to_string()), ..Default::default() };
        CliConversation::new(CliSpec::new(config, options, false, None))
    }

    /// One turn through the real runner: the app starts as Node with its runtime first, the help
    /// probe goes the same way, and its printed answer is the reply.
    #[cfg(unix)]
    #[test]
    fn a_turn_runs_the_runtime_as_node_and_reads_its_answer() {
        let d = tempfile::tempdir().unwrap();
        let exe = stand_in(d.path(), "echo 'It is a game.'");
        let mut c = conversation(&exe);
        let all = turn(&mut c, "What does this program do?");
        let text: String = all.iter().filter_map(|e| if let AgentEvent::TextDelta { text } = e { Some(text.clone()) } else { None }).collect();
        assert_eq!(text, "It is a game.\n", "{all:?}");
        assert!(!all.iter().any(|e| matches!(e, AgentEvent::Error { .. })), "{all:?}");
        let argv = std::fs::read_to_string(d.path().join("argv")).unwrap();
        let lines: Vec<&str> = argv.lines().collect();
        assert_eq!(lines[0], "ELECTRON_RUN_AS_NODE=1", "{argv}");
        assert!(lines[1].ends_with("zcode.cjs"), "the runtime comes first: {argv}");
        assert_eq!(lines[2..5], ["--mode", "plan", "-p"], "{argv}");
        assert!(argv.contains("What does this program do?"), "{argv}");
    }

    /// A turn that fails because no model was picked says how to pick one, and still ends.
    #[cfg(unix)]
    #[test]
    fn a_failed_turn_says_why_and_ends() {
        let d = tempfile::tempdir().unwrap();
        let exe = stand_in(d.path(), "echo 'Error: Select a model before sending a message' >&2\nexit 1");
        let mut c = conversation(&exe);
        let all = turn(&mut c, "hello");
        assert!(all.iter().any(|e| matches!(e, AgentEvent::Error { remediation: Some(fix), .. } if fix.contains("pick a model"))), "{all:?}");
        assert!(all.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. })), "{all:?}");
    }

    /// A help without `--prompt` (a ZCode with no headless mode) stops the run with the fix, and
    /// never falls back to starting the app with the message.
    #[cfg(unix)]
    #[test]
    fn a_zcode_without_headless_mode_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let (exe, _) = app(d.path(), "ZCode", true);
        std::fs::write(&exe, "#!/bin/sh\nd=$(dirname \"$0\")\nif [ \"$2\" = --help ]; then echo 'Usage: zcode [options]'; exit 0; fi\ntouch \"$d/ran\"\n")
            .unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut c = conversation(&exe);
        let all = turn(&mut c, "hello");
        assert!(all.iter().any(|e| matches!(e, AgentEvent::Error { message, .. } if message.contains("no headless mode"))), "{all:?}");
        assert!(!d.path().join("ran").exists(), "{all:?}");
    }
}
