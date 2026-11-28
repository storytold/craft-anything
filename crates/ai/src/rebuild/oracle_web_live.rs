//! The web behaviour oracle, live: a real page is recorded, a build that behaves differently is
//! compared with it.
//!
//! It needs a browser and a served page, so it is `#[ignore]`d, like the project's other live
//! tests. Run it with `cargo test -p craft-anything-ai --lib oracle_web_live -- --ignored` and
//! `CRAFT_ANYTHING_TEST_CHROMIUM` set to a Chromium to use.

use craft_anything_engine::workspace::{TargetRef, Workspace};

use super::kinds::Check;

/// A page whose button says 42.
const ORIGINAL: &str = "<html><body><button id=b onclick=\"document.getElementById('o').textContent='42'\">Go</button><div id=o>none</div></body></html>";

/// The same page, saying 99: a build that behaves differently.
const DIFFERENT: &str = "<html><body><button id=b onclick=\"document.getElementById('o').textContent='99'\">Go</button><div id=o>none</div></body></html>";

/// Write a page into a folder.
fn page(dir: &std::path::Path, body: &str) {
    std::fs::create_dir_all(dir).expect("the folder");
    std::fs::write(dir.join("index.html"), body).expect("the page");
}

/// Serve a folder on loopback. The server is returned with its address: it stops when dropped,
/// so it must be held for as long as the page is wanted.
fn serve(dir: &std::path::Path) -> (craft_anything_platform::serve::StaticServer, String) {
    let server = craft_anything_platform::serve::serve_dir(dir).expect("serve the folder");
    let url = server.url();
    (server, url)
}

/// Record the original's session, as Study does, and keep it with the project.
fn record(ws: &mut Workspace, url: &str) {
    let worker = super::tests::check_worker(ws, true);
    let script = super::oracle_web::record_session(&|tool, params| worker.call(tool, params), url).expect("record the original");
    ws.manifest.script = Some(script);
}

/// The claims the behaviour check produces for the build.
fn claims_for(ws: &mut Workspace) -> Vec<craft_anything_engine::workspace::Claim> {
    let worker = super::tests::check_worker(ws, true);
    let mut cx = super::kinds::CheckCx::for_test(&worker, ws, 1);
    let result = super::oracle_web::WebBehaviourCheck.run(&mut cx).expect("the check runs");
    result.claims
}

#[ignore = "needs a browser"]
#[test]
fn the_originals_session_is_recorded() {
    let d = tempfile::tempdir().unwrap();
    let original = d.path().join("original");
    page(&original, ORIGINAL);
    let (_server, url) = serve(&original);
    let mut ws = Workspace::create(d.path(), "Site", "Rebuild this site", TargetRef::Website { url: url.clone() }).unwrap();
    record(&mut ws, &url);
    let script = ws.manifest.script.as_ref().expect("the original's session was recorded");
    assert_eq!(script.oracle, "web");
    assert!(!script.steps.is_empty(), "the session has steps: {:?}", script.steps);
    assert_eq!(script.probes.len(), 1);
    assert!(!script.original.is_empty(), "the session was recorded");
    // One observation per step: the session is compared step by step, not only at its end.
    assert_eq!(script.original.len(), script.steps.len(), "one observation per step");
    let value = script.original[0].readings[0].as_ref().expect("a reading").value.clone();
    let shown = value.as_object().expect("labelled values");
    assert!(!shown.is_empty(), "the original's page showed something: {shown:?}");
    // Each step read the page as it was then, so a difference in the middle is visible.
    let middles: Vec<bool> = script.original.iter().map(|o| o.readings[0].is_some()).collect();
    assert!(middles.iter().all(|m| *m), "every step was read: {middles:?}");
}

#[ignore = "needs a browser"]
#[test]
fn a_build_that_behaves_differently_is_caught() {
    let d = tempfile::tempdir().unwrap();
    let original = d.path().join("original");
    page(&original, ORIGINAL);
    let (_server, url) = serve(&original);
    let mut ws = Workspace::create(d.path(), "Site", "Rebuild this site", TargetRef::Website { url: url.clone() }).unwrap();
    record(&mut ws, &url);

    // The build: the same page, saying something else.
    let build = d.path().join("build");
    page(&build, DIFFERENT);
    std::fs::create_dir_all(ws.app_dir()).unwrap();
    std::fs::copy(build.join("index.html"), ws.app_dir().join("index.html")).unwrap();

    let claims = claims_for(&mut ws);
    assert!(!claims.is_empty(), "a different build must fail a claim");
    assert!(claims.iter().any(|c| !c.passed), "the claim must be a failure: {claims:?}");
}

#[ignore = "needs a browser"]
#[test]
fn a_build_that_behaves_the_same_is_not_caught() {
    let d = tempfile::tempdir().unwrap();
    let original = d.path().join("original");
    page(&original, ORIGINAL);
    let (_server, url) = serve(&original);
    let mut ws = Workspace::create(d.path(), "Site", "Rebuild this site", TargetRef::Website { url: url.clone() }).unwrap();
    record(&mut ws, &url);

    // The build: the same page.
    let build = d.path().join("build");
    page(&build, ORIGINAL);
    std::fs::create_dir_all(ws.app_dir()).unwrap();
    std::fs::copy(build.join("index.html"), ws.app_dir().join("index.html")).unwrap();

    let claims = claims_for(&mut ws);
    assert!(claims.iter().all(|c| c.passed), "the same behaviour must hold: {claims:?}");
}
