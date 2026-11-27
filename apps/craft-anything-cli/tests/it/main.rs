//! The command line as RFC-0018 pins it, through the built binary: streams, exit codes and the
//! environment variable. (`src/tests.rs` calls `run` in-process for what needs no engine.)

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn cli(config: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_craft-anything-cli"));
    c.env("CRAFT_ANYTHING_CONFIG_DIR", config).env_remove("CRAFT_ANYTHING_LOG");
    c
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn run(config: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    cli(config).args(args).envs(envs.iter().copied()).stdin(Stdio::null()).output().unwrap()
}

#[test]
fn json_is_indented_by_default_and_one_line_with_compact() {
    let cfg = tempfile::tempdir().unwrap();
    let pretty = run(cfg.path(), &["commands", "--json"], &[]);
    let compact = run(cfg.path(), &["--compact", "commands", "--json"], &[]);
    assert!(pretty.status.success() && compact.status.success());
    let (pretty, compact) = (text(&pretty.stdout), text(&compact.stdout));
    assert!(pretty.lines().count() > 1, "indented output has many lines");
    assert_eq!(compact.lines().count(), 1, "--compact is one line");
    // The same JSON either way, and nothing but JSON on stdout.
    let a: serde_json::Value = serde_json::from_str(&pretty).unwrap();
    let b: serde_json::Value = serde_json::from_str(&compact).unwrap();
    assert_eq!(a, b);
}

#[test]
fn a_failed_command_exits_1_with_the_structured_error_on_stdout_and_the_message_on_stderr() {
    let cfg = tempfile::tempdir().unwrap();
    let gone = cfg.path().join("no-such-file.bin");
    let r = run(cfg.path(), &["triage", gone.to_str().unwrap()], &[]);
    assert_eq!(r.status.code(), Some(1), "{}", text(&r.stderr));
    let v: serde_json::Value = serde_json::from_str(&text(&r.stdout)).unwrap();
    assert!(v["error"]["message"].is_string(), "{v}");
    assert!(text(&r.stderr).starts_with("error: "), "{}", text(&r.stderr));
}

#[test]
fn the_log_variable_is_a_fallback_for_its_flag() {
    let cfg = tempfile::tempdir().unwrap();
    // A bad value is reported under the variable's name, not under a flag nobody typed.
    let r = run(cfg.path(), &["commands"], &[("CRAFT_ANYTHING_LOG", "loud")]);
    assert_eq!(r.status.code(), Some(2));
    assert_eq!(text(&r.stdout), "");
    assert!(text(&r.stderr).contains("CRAFT_ANYTHING_LOG") && !text(&r.stderr).contains("--log"), "{}", text(&r.stderr));
    // An empty variable counts as unset.
    assert_eq!(run(cfg.path(), &["commands"], &[("CRAFT_ANYTHING_LOG", "")]).status.code(), Some(0));
    // The flag wins over the variable.
    assert_eq!(run(cfg.path(), &["--log", "off", "commands"], &[("CRAFT_ANYTHING_LOG", "loud")]).status.code(), Some(0));
    // `--help` lists the variable.
    assert!(text(&run(cfg.path(), &["--help"], &[]).stdout).contains("CRAFT_ANYTHING_LOG"));
}

#[test]
fn mcp_writes_nothing_but_protocol_to_stdout() {
    let cfg = tempfile::tempdir().unwrap();
    let mut child = cli(cfg.path()).arg("mcp").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let request =
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
    writeln!(child.stdin.take().unwrap(), "{request}").unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let lines: Vec<String> = text(&out.stdout).lines().map(String::from).collect();
    assert!(!lines.is_empty());
    for line in lines {
        let v: serde_json::Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON-RPC: {line}: {e}"));
        assert_eq!(v["jsonrpc"], "2.0");
    }
}

#[test]
fn tool_options_are_checked_before_any_work() {
    let cfg = tempfile::tempdir().unwrap();
    let project = cfg.path().join("made-by-mistake.cany");
    let project = project.to_str().unwrap();
    for args in [
        vec!["core-session-show", "--bogus"],
        vec!["core-session-show", "--allow-run", "--allow-run"],
        vec!["core-session-show", "--deep", "--clean-room"],
        vec!["core-session-show", "--project", project, "--project", project],
        vec!["core-session-show", "--project"],
        vec!["core-journal-read", "--limit", "1", "--limit", "2"],
        vec!["core-journal-read", "--limit", "--compact"],
        vec!["core-journal-read", "-x"],
        vec!["core-journal-read", "--json", "{\"limit\":1}", "--limit", "2"],
    ] {
        let r = run(cfg.path(), &args, &[]);
        assert_eq!(r.status.code(), Some(2), "{args:?}: {}", text(&r.stderr));
        assert_eq!(text(&r.stdout), "", "{args:?}");
        assert!(text(&r.stderr).starts_with("error: "), "{args:?}: {}", text(&r.stderr));
    }
    assert!(!Path::new(project).exists(), "a refused command line must not create the project");
}

#[test]
fn json_after_a_tool_name_is_its_parameters_with_a_value_and_the_output_format_without() {
    let cfg = tempfile::tempdir().unwrap();
    let project = cfg.path().join("p.cany");
    let project = project.to_str().unwrap();
    let ok = |args: &[&str]| {
        // The journal is kept per project: every call opens (or makes) this one.
        let args: Vec<&str> = ["--project", project].into_iter().chain(args.iter().copied()).collect();
        let r = run(cfg.path(), &args, &[]);
        assert_eq!(r.status.code(), Some(0), "{args:?}: {}", text(&r.stderr));
        serde_json::from_str::<serde_json::Value>(&text(&r.stdout)).unwrap_or_else(|e| panic!("{args:?}: {e}"))
    };
    // With parameters after it: the tool's own `--json`, in any order with other options.
    ok(&["core-journal-read", "--json", "{\"limit\":1}"]);
    ok(&["core-journal-read", "--json={\"limit\":1}", "--kind", "tool_call"]);
    // Bare: the shorthand for `--format json`, even after `--output-format text`'s default would apply.
    ok(&["core-journal-read", "--json"]);
    ok(&["core-journal-read", "--json", "--limit", "1"]);
    // Before the tool name it is the shorthand too, and it cannot be combined with --format.
    ok(&["--json", "core-journal-read"]);
    let both = run(cfg.path(), &["--json", "--format", "text", "commands"], &[]);
    assert_eq!(both.status.code(), Some(2));
}

#[test]
fn a_tool_prints_its_version() {
    let cfg = tempfile::tempdir().unwrap();
    for flag in ["--version", "-V"] {
        let r = run(cfg.path(), &["core-session-show", flag], &[]);
        assert_eq!(r.status.code(), Some(0), "{}", text(&r.stderr));
        assert!(text(&r.stdout).contains(env!("CARGO_PKG_VERSION")), "{}", text(&r.stdout));
    }
}

// Man pages and completions: the example itself, so the test writes the files exactly as the packages get them.
#[allow(dead_code)]
#[path = "../../examples/man-and-completions.rs"]
mod example;

#[test]
fn the_command_is_consistent_and_every_shell_gets_completions() {
    use clap::ValueEnum;
    craft_anything_cli::command().debug_assert();
    for &shell in clap_complete::Shell::value_variants() {
        let mut out = Vec::new();
        clap_complete::generate(shell, &mut craft_anything_cli::command(), "craft-anything-cli", &mut out);
        assert!(!out.is_empty(), "{shell}");
    }
}

/// The example writes the five completion files under the names the packages install, a page for
/// the command and a page per subcommand, and none for the `help` subcommand.
#[test]
fn the_example_writes_the_files_the_packages_install() {
    let dir = tempfile::tempdir().unwrap();
    example::generate(dir.path()).unwrap();
    let mut written: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    written.sort();
    for completion in ["craft-anything-cli.bash", "_craft-anything-cli", "craft-anything-cli.fish", "craft-anything-cli.elv", "_craft-anything-cli.ps1"] {
        assert!(written.iter().any(|w| w == completion), "{completion} in {written:?}");
    }
    let pages: Vec<&String> = written.iter().filter(|w| w.ends_with(".1")).collect();
    assert!(!pages.iter().any(|p| p.contains("-help")), "{pages:?}");
    for sub in craft_anything_cli::command().get_subcommands().filter(|s| s.get_name() != "help" && !s.is_hide_set()) {
        let page = format!("craft-anything-cli-{}.1", sub.get_name());
        assert!(written.contains(&page), "{page} in {written:?}");
    }
    assert!(written.iter().any(|w| w == "craft-anything-cli.1"));
}

/// The FreeBSD package (staged from stubs in a dry run) installs the bash, zsh and fish
/// completions and the man pages.
#[cfg(unix)]
#[test]
fn the_freebsd_package_installs_completions_and_man_pages() {
    let work = tempfile::tempdir().unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/freebsd/package.sh");
    let out = Command::new("bash")
        .arg(&script)
        .arg("--dry-run")
        .env("DIST", work.path().join("dist"))
        .env("CARGO_TARGET_DIR", work.path().join("target"))
        .env_remove("CARGO_BUILD_TARGET")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}{}", text(&out.stdout), text(&out.stderr));
    let listing = text(&out.stdout);
    for path in [
        "share/bash-completion/completions/craft-anything-cli",
        "share/zsh/site-functions/_craft-anything-cli",
        "share/fish/vendor_completions.d/craft-anything-cli.fish",
        "share/man/man1/craft-anything-cli.1",
    ] {
        assert!(listing.contains(path), "{path} missing from the package:\n{listing}");
    }
}
