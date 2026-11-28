//! A build in a runner process of its own (#1970): started from a staged copy of the built CLI,
//! it says why when it can't start, holds its project while it builds, and outlives the process
//! that started it.
//!
//! The builder is an OpenAI-compatible backend on a loopback server that accepts and never answers,
//! so a run that starts stays in its first request: no network, no coding agent, no key. Each
//! test has its own config folder, projects and staging folder.

use craft_anything_ai::rebuild::RunOptions;
use craft_anything_ai::rebuild::runner::{self, RunnerOptions};
use craft_anything_engine::workspace::{TargetRef, Workspace, build_lock};
use craft_anything_platform::process::{Launch, Process};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const CLI: &str = env!("CARGO_BIN_EXE_craft-anything-cli");
/// Set only for the parent process the survival test starts (the guard that keeps the fixture from
/// doing anything when ignored tests are run by hand).
const FIXTURE_ENV: &str = "CRAFT_ANYTHING_TEST_RUNNER_PARENT";

/// A loopback server that takes connections and never answers, so a builder's first request waits.
struct Silent {
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Silent {
    fn start() -> Silent {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut held = Vec::new();
            while !s.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((conn, _)) => held.push(conn),
                    Err(_) => std::thread::sleep(Duration::from_millis(20)),
                }
            }
        });
        Silent { port, stop, thread: Some(thread) }
    }
}

impl Drop for Silent {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Bench {
    _dir: tempfile::TempDir,
    config: PathBuf,
    staging: PathBuf,
    ws: Workspace,
}

/// A project, a config folder (with an API builder on `port`, or no AI at all) and a staging folder.
fn bench(port: Option<u16>) -> Bench {
    let dir = tempfile::Builder::new().prefix("runner").tempdir().unwrap();
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    if let Some(port) = port {
        std::fs::write(
            config.join("config.toml"),
            format!(
                "[[ai.backends]]\nname = \"API\"\nkind = \"openai_compatible\"\nmodel = \"test-model\"\nbase_url = \"http://127.0.0.1:{port}/v1\"\nenabled = true\n"
            ),
        )
        .unwrap();
    }
    let ws = Workspace::create(&dir.path().join("projects"), "Notes", "A small notes page", TargetRef::None).unwrap();
    let staging = dir.path().join("staging");
    Bench { _dir: dir, config, staging, ws }
}

fn options() -> RunnerOptions {
    RunnerOptions::new(&RunOptions { max_rounds: 0, ..RunOptions::default() }, false)
}

/// Wait up to `limit` for `f`.
fn eventually(limit: Duration, f: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    f()
}

/// End a process we started, by its id (a runner we have no handle on), and what it started.
fn end_pid(pid: u32) {
    let p = if cfg!(windows) {
        Process::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"])
    } else {
        Process::new("kill").args(["-KILL", &pid.to_string()])
    };
    let _ = p.timeout(Duration::from_secs(20)).run();
}

#[test]
fn a_runner_that_cant_start_says_why_in_its_own_words() {
    let b = bench(None);
    let err = runner::spawn_in(Path::new(CLI), &b.staging, &b.ws, &options(), &b.config).unwrap_err();
    assert!(err.message.contains("coding agent"), "the runner's own message: {err:?}");
    assert!(err.remediation.is_some(), "and its remediation: {err:?}");
    let inbox = b.ws.inbox_dir();
    assert!(inbox.join(runner::ERROR_FILE).is_file(), "kept for the app");
    assert!(!inbox.join(runner::OPTIONS_FILE).exists(), "the runner read and removed its options");
    assert!(!build_lock::held(&b.ws.root), "nothing holds the project");
    // The staged copy keeps the CLI's name.
    let staged: Vec<PathBuf> =
        std::fs::read_dir(&b.staging).unwrap().flatten().map(|e| e.path().join(craft_anything_platform::which::cli_file_name())).collect();
    assert!(staged.len() == 1 && staged[0].is_file(), "{staged:?}");
}

#[test]
fn a_runner_holds_its_project_until_it_is_stopped() {
    let server = Silent::start();
    let b = bench(Some(server.port));
    let launched = runner::spawn_in(Path::new(CLI), &b.staging, &b.ws, &options(), &b.config).unwrap();
    let inbox = b.ws.inbox_dir();
    // Whether this test's own job (Cargo's, when run through it) let the runner leave it.
    eprintln!("runner {}, outside this test's job: {}", launched.pid(), launched.outside_our_job());
    assert!(build_lock::held(&b.ws.root), "its build holds the project");
    assert_eq!(runner::read_started(&inbox).map(|s| s.pid), Some(launched.pid()));
    assert!(!inbox.join(runner::ERROR_FILE).exists());
    // A second one is refused before anything starts.
    let err = runner::spawn_in(Path::new(CLI), &b.staging, &b.ws, &options(), &b.config).unwrap_err();
    assert_eq!(err.message, craft_anything_ai::rebuild::BUILDING_ELSEWHERE);
    assert!(launched.stop(), "stopped");
    assert!(eventually(Duration::from_secs(10), || !build_lock::held(&b.ws.root)), "the project is let go when it ends");
    assert!(!launched.is_running(), "nothing left running");
    drop(server);
}

/// The point of the runner: the process that started it ends (here it is killed, as by the task
/// manager) and the build goes on.
#[test]
fn a_runner_outlives_the_process_that_started_it() {
    let server = Silent::start();
    let b = bench(Some(server.port));
    let report = b.staging.with_file_name("parent.json");
    // The parent is this test program, running only the fixture below.
    let me = std::env::current_exe().unwrap();
    let parent = Launch::new(me)
        .args(["--exact", "survival_parent_fixture", "--ignored", "--nocapture", "--test-threads=1"])
        .env(FIXTURE_ENV, &report)
        .env("FIXTURE_PROJECT", &b.ws.root)
        .env("FIXTURE_STAGING", &b.staging)
        .env("FIXTURE_CONFIG", &b.config)
        .log_to(b.staging.with_file_name("parent.log"))
        .hide_console()
        .start_watched()
        .unwrap();
    let started = eventually(Duration::from_secs(30), || report.is_file());
    let text = std::fs::read_to_string(&report).unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let runner_pid = v["runner"].as_u64().and_then(|p| u32::try_from(p).ok());
    if !started || runner_pid.is_none() {
        parent.stop();
        panic!("the parent didn't start a runner: {text} / {}", std::fs::read_to_string(b.staging.with_file_name("parent.log")).unwrap_or_default());
    }
    let runner_pid = runner_pid.unwrap_or_default();
    eprintln!("runner {runner_pid}, outside the parent's job: {}", v["outside_our_job"]);
    assert!(build_lock::held(&b.ws.root));

    // Kill the parent only (not its tree or job).
    if cfg!(windows) {
        let _ = Process::new("taskkill").args(["/PID", &parent.pid().to_string(), "/F"]).timeout(Duration::from_secs(20)).run();
    } else {
        let _ = Process::new("kill").args(["-KILL", &parent.pid().to_string()]).timeout(Duration::from_secs(20)).run();
    }
    let parent_gone = parent.wait_timeout(Duration::from_secs(10)).is_some();
    std::thread::sleep(Duration::from_secs(2));
    let alive = build_lock::held(&b.ws.root);

    end_pid(runner_pid);
    parent.stop();
    let released = eventually(Duration::from_secs(15), || !build_lock::held(&b.ws.root));
    drop(server);
    assert!(parent_gone, "the parent was killed");
    assert!(alive, "the runner's build still holds its project after its parent was killed");
    assert!(released, "and ends when it is stopped: nothing left running");
}

/// The parent for the test above. Does nothing unless that test started it.
#[test]
#[ignore = "started by a_runner_outlives_the_process_that_started_it"]
fn survival_parent_fixture() {
    let Some(report) = std::env::var_os(FIXTURE_ENV).map(PathBuf::from) else { return };
    let var = |k: &str| PathBuf::from(std::env::var_os(k).unwrap());
    let ws = Workspace::open(&var("FIXTURE_PROJECT")).unwrap();
    let launched = runner::spawn_in(Path::new(CLI), &var("FIXTURE_STAGING"), &ws, &options(), &var("FIXTURE_CONFIG")).unwrap();
    let v = serde_json::json!({"runner": launched.pid(), "outside_our_job": launched.outside_our_job()});
    std::fs::write(&report, v.to_string()).unwrap();
    // Wait to be killed; end by itself in the end, so a parent the test lost doesn't stay forever.
    std::thread::sleep(Duration::from_secs(120));
}
