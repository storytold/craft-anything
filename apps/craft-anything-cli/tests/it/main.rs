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
