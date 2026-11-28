//! The behaviour oracle, end to end inside a run: Study records what the original did, and the
//! check compares the build with it.
//!
//! These are the tests that show the line is connected — that a build is judged against the
//! original's own behaviour, not against its own account of itself.

use std::path::Path;

use craft_anything_engine::workspace::{Claim, TargetRef, Workspace};

use super::kinds::Check;

/// A console program that prints one thing.
const ORIGINAL: &str = "#!/bin/sh\necho 'Result 42 completed'\nexit 0\n";

/// The same program saying something else: a build that behaves differently.
const DIFFERENT: &str = "#!/bin/sh\necho 'Result -42 completed'\nexit 0\n";

/// The same program, saying the same thing.
const SAME: &str = "#!/bin/sh\necho 'Result 42 completed'\nexit 0\n";

/// Write a runnable program.
#[cfg(unix)]
fn program(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write the program");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("make it runnable");
    path
}

#[cfg(not(unix))]
fn program(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write the program");
    path
}

/// Record the original's behaviour, as Study does, and keep it with the project.
fn record(ws: &mut Workspace, program: &Path) {
    let capture = ws.capture_dir();
    let worker = super::tests::check_worker(ws, true);
    worker.capture_runs(ws, program, &capture).expect("record the original");
}

/// The build's own program, as a project holds it.
///
/// A Rust project is what `launch_now` looks for first, so the build is laid out as one: a
/// `Cargo.toml` and the program under `target/debug/`.
fn write_build(ws: &Workspace, body: &str) {
    let app = ws.app_dir();
    std::fs::create_dir_all(app.join("target/debug")).expect("the build folder");
    std::fs::write(app.join("Cargo.toml"), "[package]\nname = \"tool\"\n").expect("the manifest");
    let path = app.join("target/debug/tool");
    std::fs::write(&path, body).expect("write the build");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("make it runnable");
    }
}

/// The claims the behaviour check produces for a build.
///
/// The check is run on its own rather than through the run's checks: the build check comes first
/// and stops the rest when the project has no build, which is not what this is about.
fn claims_for(ws: &mut Workspace) -> Vec<Claim> {
    let worker = super::tests::check_worker(ws, true);
    let check = super::kinds::ConsoleBehaviourCheck;
    let mut cx = super::kinds::CheckCx::for_test(&worker, ws, 1);
    let result = check.run(&mut cx).expect("the check runs");
    result.claims
}

#[test]
fn study_records_what_the_original_did() {
    let d = tempfile::tempdir().unwrap();
    let original = program(d.path(), "counter", ORIGINAL);
    let mut ws = Workspace::create(d.path(), "Counter", "Rebuild this console tool", TargetRef::Local { path: original.clone() }).unwrap();
    assert!(ws.manifest.script.is_none(), "nothing is recorded before the study");
    record(&mut ws, &original);
    let script = ws.manifest.script.as_ref().expect("the study recorded the original's behaviour");
    assert_eq!(script.oracle, "console");
    assert_eq!(script.steps.len(), super::kinds::runs::CASES.len(), "one step per input");
    assert_eq!(script.probes.len(), 2, "the exit code and what was printed");
    assert_eq!(script.original.len(), script.steps.len(), "one observation per step");
    assert_eq!(script.original[0].readings.len(), 2, "two readings per step");
    assert!(script.original.iter().all(|o| o.gaps.is_empty()), "the original ran");
}

#[test]
fn a_build_that_behaves_differently_is_caught() {
    let d = tempfile::tempdir().unwrap();
    let original = program(d.path(), "counter", ORIGINAL);
    let mut ws = Workspace::create(d.path(), "Counter", "Rebuild this console tool", TargetRef::Local { path: original.clone() }).unwrap();
    record(&mut ws, &original);
    write_build(&ws, DIFFERENT);
    let claims = claims_for(&mut ws);
    assert!(!claims.is_empty(), "a different build must fail a claim");
    assert!(claims.iter().any(|c| !c.passed), "the claim must be a failure: {claims:?}");
}

#[test]
fn a_build_that_behaves_the_same_is_not_caught() {
    let d = tempfile::tempdir().unwrap();
    let original = program(d.path(), "counter", ORIGINAL);
    let mut ws = Workspace::create(d.path(), "Counter", "Rebuild this console tool", TargetRef::Local { path: original.clone() }).unwrap();
    record(&mut ws, &original);
    write_build(&ws, SAME);
    let claims = claims_for(&mut ws);
    assert!(claims.iter().all(|c| c.passed), "the same behaviour must hold: {claims:?}");
}

#[test]
fn a_build_that_cannot_be_run_is_a_failure_not_a_pass() {
    let d = tempfile::tempdir().unwrap();
    let original = program(d.path(), "counter", ORIGINAL);
    let mut ws = Workspace::create(d.path(), "Counter", "Rebuild this console tool", TargetRef::Local { path: original.clone() }).unwrap();
    record(&mut ws, &original);
    // A build that is not there at all.
    let claims = claims_for(&mut ws);
    assert!(claims.iter().any(|c| !c.passed), "a build that cannot run must not pass: {claims:?}");
}
