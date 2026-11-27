//! #2425: `engines.ghidra.memory_gb`, set the way a person sets it, reaches Ghidra's start.
//! A test program of its own: Deep mode's analyses share process-wide state.

#![cfg(unix)]

use craft_anything_engine::tools::deep;
use craft_anything_engine::{CallOptions, Caller, Engine};
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// A stand-in `support/analyzeHeadless` that appends the heap it was given to `record`, then fails.
fn recording_launcher(dir: &Path) -> (PathBuf, PathBuf) {
    let support = dir.join("install").join("support");
    std::fs::create_dir_all(&support).unwrap();
    let record = dir.join("heap.txt");
    let launcher = support.join("analyzeHeadless");
    let script = format!("#!/bin/sh\nprintf '%s\\n' \"${{GHIDRA_HEADLESS_MAXMEM-unset}}\" >> '{}'\nexit 1\n", record.display());
    std::fs::write(&launcher, script).unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755)).unwrap();
    (launcher, record)
}

/// Run the analysis a build's study starts, with `memory_gb` set through `settings_set` (or left
/// unset), and return the heap each start of the stand-in was given.
fn heap_given(memory_gb: Option<u64>) -> Vec<String> {
    // A plain prefix: Ghidra's store must not sit under a dot folder, which tempdir()'s names are.
    let d = tempfile::Builder::new().prefix("ca").tempdir().unwrap();
    let (launcher, record) = recording_launcher(d.path());
    let engine = Engine::new(craft_anything_engine::testing::session());
    let set = |key: &str, value: serde_json::Value| {
        engine.call("settings_set", json!({"key": key, "value": value}), CallOptions::new(Caller::Cli)).unwrap();
    };
    set("engines.store_dir", json!(d.path().join("store")));
    set("engines.ghidra.install_dir", json!(launcher));
    set("engines.ghidra.startup_timeout_secs", json!(60));
    if let Some(gb) = memory_gb {
        set("engines.ghidra.memory_gb", json!(gb));
    }
    let settings = engine.session().settings.clone();
    assert_eq!(settings.engines.ghidra.memory_gb, memory_gb);
    let project = d.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join(craft_anything_engine::workspace::MANIFEST), "{}").unwrap();
    let bytes = b"not really a program";
    let err = deep::prepare(&settings, bytes, &craft_anything_evidence::sha256_hex(bytes), &project, &|_| {}).unwrap_err();
    assert!(!err.contains("isn't installed"), "the stand-in is found as Ghidra: {err}");
    std::fs::read_to_string(&record).unwrap_or_default().lines().map(str::to_string).collect()
}

#[test]
fn the_memory_set_for_ghidra_is_the_heap_it_starts_with() {
    assert_eq!(heap_given(Some(1)), ["1G"], "below the automatic floor of 4 GB too");
    assert_eq!(heap_given(Some(3)), ["3G"]);
    assert_eq!(heap_given(Some(12)), ["12G"]);
}

#[test]
fn with_nothing_set_ghidra_gets_the_automatic_amount() {
    let starts = heap_given(None);
    assert_eq!(starts.len(), 1, "{starts:?}");
    let gb: u64 = starts[0].strip_suffix('G').and_then(|n| n.parse().ok()).unwrap_or_else(|| panic!("a heap in GB: {starts:?}"));
    assert!(craft_anything_engines::ghidra::HEAP_GB.contains(&gb), "{gb} GB");
}
