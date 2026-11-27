//! Stopping Ghidra's analysis for one project and keeping the project (#948). A test program of its
//! own: which analyses are under way is the whole process's.

use craft_anything_engine::settings::Settings;
use craft_anything_engine::tools::deep;
use craft_anything_engine::workspace::MANIFEST;
use craft_anything_engine::{CallOptions, Caller, Engine};
use craft_anything_engines::ghidra;
use craft_anything_testkit as tk;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Settings with a stand-in Ghidra whose analysis of `bytes` waits for another process's lock, so
/// anything that starts it runs until it's stopped (or 40 s pass).
fn waiting_ghidra(d: &Path, bytes: &[u8]) -> Settings {
    let launcher = d.join("analyzeHeadless");
    std::fs::write(&launcher, "").unwrap();
    let store = d.join("store");
    let mut settings = craft_anything_engine::testing::session().settings.clone();
    settings.engines.store_dir = Some(store.clone());
    settings.engines.ghidra.install_dir = Some(launcher);
    settings.engines.ghidra.startup_timeout_secs = Some(40);
    let sha = craft_anything_evidence::sha256_hex(bytes);
    let digest = craft_anything_evidence::sha256_hex(b"craft-anything-ghidra-fast-profile-v2|lang=");
    let cache = ghidra::project_cache_dir(&store, &sha, &digest);
    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    std::fs::write(cache.with_extension("analysing"), "").unwrap();
    settings
}

fn project(d: &Path) -> PathBuf {
    let root = d.join("game");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(MANIFEST), "{}").unwrap();
    root
}

fn ends_within<T>(h: &std::thread::JoinHandle<T>, limit: Duration) -> bool {
    let until = Instant::now() + limit;
    while !h.is_finished() {
        if Instant::now() > until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

/// `engines_analysis_stop` (the project's Stop link) ends the analysis within moments and keeps
/// the project; a new request lets Ghidra be used again.
#[test]
fn stopping_it_for_its_project_ends_it_and_keeps_the_project() {
    let d = tempfile::Builder::new().prefix("ca").tempdir().unwrap();
    let bytes: &'static [u8] = b"a program its project stopped the analysis of";
    let settings = waiting_ghidra(d.path(), bytes);
    let root = project(d.path());
    let (began, waiting) = std::sync::mpsc::channel();
    let a = {
        let root = root.clone();
        std::thread::spawn(move || {
            deep::prepare(&settings, bytes, &craft_anything_evidence::sha256_hex(bytes), &root, &move |_| {
                let _ = began.send(());
            })
        })
    };
    assert!(waiting.recv_timeout(Duration::from_secs(30)).is_ok(), "the analysis never began waiting");
    assert!(deep::analysing_for(&root));

    let e = Engine::new(craft_anything_engine::testing::session());
    let stop = |p: &Path| e.call("engines_analysis_stop", json!({"project": p}), CallOptions::new(Caller::Cli));
    assert_eq!(stop(&root).unwrap().to_json()["result"]["stopped"], true);
    assert!(!deep::analysing_for(&root), "the screen stops offering Stop at once");
    assert!(ends_within(&a, Duration::from_secs(30)), "the analysis kept going");
    assert!(a.join().unwrap().is_err(), "it stopped, it didn't finish");
    assert!(root.join(MANIFEST).is_file(), "the project is kept");
    assert!(deep::stopped(&root));
    assert_eq!(stop(&root).unwrap().to_json()["result"]["stopped"], false, "nothing left to stop");
    assert!(stop(d.path()).is_err(), "only a project folder is written to");

    deep::resume(&root).unwrap();
    assert!(!deep::stopped(&root));
    deep::resume(&root).unwrap();
}

/// #1118 review: once stopped, the build's next decompile (here the builder's tool server, which has
/// the project's investigation open) uses the built-in engine instead of starting the analysis again.
#[test]
fn a_decompile_after_stop_does_not_start_the_analysis_again() {
    let d = tempfile::Builder::new().prefix("ca").tempdir().unwrap();
    let (bytes, layout) = tk::elf64(tk::ElfArch::X86_64, false);
    let program = d.path().join("game.elf");
    std::fs::write(&program, &bytes).unwrap();
    let root = project(d.path());
    let mut session = craft_anything_engine::testing::session();
    session.settings = waiting_ghidra(d.path(), &bytes);
    session.settings.deep_mode = true;
    let investigation = craft_anything_engine::project::Project::create(&root.join("investigation.cany"), "Game", Default::default()).unwrap();
    session.open_project(investigation).unwrap();
    let e = Engine::new(session);
    e.call("core_target_open", json!({"path": program}), CallOptions::new(Caller::Cli)).unwrap();
    assert!(!deep::analysing_for(&root), "nothing under way in this process");
    assert_eq!(e.call("engines_analysis_stop", json!({"project": root}), CallOptions::new(Caller::Cli)).unwrap().to_json()["result"]["stopped"], false);

    let call = std::thread::spawn(move || {
        e.call("native_function_pseudocode", json!({"function": format!("{:#x}", layout.code_va)}), CallOptions::new(Caller::Cli)).map(|o| o.to_json())
    });
    assert!(ends_within(&call, Duration::from_secs(20)), "the decompile started Ghidra's analysis again");
    let out = call.join().unwrap().unwrap().to_string();
    assert!(out.contains(deep::STOPPED_NOTE), "the builder is told why: {out}");
}

/// #1903 review: a request made before the stop leaves Ghidra off, one made after it lets it run,
/// and once it runs again nothing of the stop is left in the project.
#[test]
fn a_request_from_before_the_stop_keeps_it() {
    let d = tempfile::Builder::new().prefix("ca").tempdir().unwrap();
    let root = project(d.path());
    let before = std::time::SystemTime::now();
    std::thread::sleep(Duration::from_millis(5));
    deep::stop_for(&root).unwrap();
    deep::resume_for(&root, before).unwrap();
    assert!(deep::stopped(&root), "a request from before the stop doesn't undo it");
    std::thread::sleep(Duration::from_millis(5));
    deep::resume_for(&root, std::time::SystemTime::now()).unwrap();
    assert!(!deep::stopped(&root), "a later request lets Ghidra run");
    deep::resume_for(&root, std::time::SystemTime::now()).unwrap();
    let names: Vec<_> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert!(!names.iter().any(|n| n.to_string_lossy().starts_with("ghidra-stop")), "the stop and its lock file are gone: {names:?}");
    // Stopped again after that: the lock file comes back with the stop.
    deep::stop_for(&root).unwrap();
    assert!(deep::stopped(&root) && root.join(deep::STOP_LOCK).is_file());
}

/// #1903 review: while older requests judge the stop, it never disappears for readers, and a
/// newer request racing them isn't undone by one of them putting the stop back.
#[test]
fn the_stop_stays_visible_while_requests_judge_it() {
    let d = tempfile::Builder::new().prefix("ca").tempdir().unwrap();
    let root = project(d.path());
    let before = std::time::SystemTime::now();
    std::thread::sleep(Duration::from_millis(5));
    deep::stop_for(&root).unwrap();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let judges: Vec<_> = (0..4)
        .map(|_| {
            let (root, done) = (root.clone(), done.clone());
            std::thread::spawn(move || {
                while !done.load(std::sync::atomic::Ordering::SeqCst) {
                    deep::resume_for(&root, before).unwrap();
                }
            })
        })
        .collect();
    for _ in 0..2000 {
        assert!(deep::stopped(&root), "an older request hid the stop");
    }
    std::thread::sleep(Duration::from_millis(5));
    deep::resume_for(&root, std::time::SystemTime::now()).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    for j in judges {
        j.join().unwrap();
    }
    assert!(!deep::stopped(&root), "the newer request's resume held");
}

/// #1903 review: a stop that took its time but hasn't written it yet doesn't outlast a request made
/// after that time: the request waits for the stop's lock and then judges it.
#[test]
fn a_request_waits_for_a_stop_being_written() {
    let d = tempfile::Builder::new().prefix("ca").tempdir().unwrap();
    let root = project(d.path());
    deep::resume_for(&root, std::time::SystemTime::now()).unwrap();
    assert!(!root.join("ghidra-stop.lock").exists(), "a project never stopped gets no lock file");
    // Stand in for a stop between taking its time and writing it.
    let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(root.join("ghidra-stop.lock")).unwrap();
    lock.lock().unwrap();
    let stopped_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    std::thread::sleep(Duration::from_millis(5));
    let asked = std::time::SystemTime::now();
    let r = root.clone();
    let request = std::thread::spawn(move || deep::resume_for(&r, asked).unwrap());
    std::thread::sleep(Duration::from_millis(100));
    std::fs::write(root.join(deep::STOPPED), stopped_at.to_string()).unwrap();
    lock.unlock().unwrap();
    request.join().unwrap();
    assert!(!deep::stopped(&root), "the request came after the stop's time");
}
