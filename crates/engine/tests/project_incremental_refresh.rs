use craft_anything_engine::project::{Project, Purpose, write_atomic};
use craft_anything_engine::session::{MAX_LOADED_EVIDENCE, Session};
use craft_anything_engine::tools::investigation_tools;
use craft_anything_evidence::{Environment, Evidence, EvidenceBuilder, Subject};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

fn evidence(tag: usize, bytes: usize) -> Evidence {
    EvidenceBuilder::new("refresh-fixture", Subject::default(), Environment::builtin(true))
        .result(json!({"tag": tag, "payload": "x".repeat(bytes)}))
        .summary("old label")
        .build()
        .unwrap()
}

fn project(root: &Path) -> Project {
    Project::create(root, "Refresh fixture", Purpose::Education).unwrap()
}

fn evidence_path(project: &Project, id: &str) -> PathBuf {
    project.root.join("evidence").join(&id[3..5]).join(format!("{id}.json"))
}

fn payload_address(session: &Session, id: &str) -> *const u8 {
    session.evidence(id).unwrap().result["payload"].as_str().unwrap().as_ptr()
}

fn set_modified(path: &Path, modified: SystemTime) {
    std::fs::OpenOptions::new().write(true).open(path).unwrap().set_modified(modified).unwrap();
}

#[test]
fn unchanged_refresh_reuses_large_evidence_and_still_loads_new_records() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(&dir.path().join("project.cany"));
    let ids: Vec<_> = (0..16)
        .map(|tag| {
            let ev = evidence(tag, 512 * 1024);
            project.store_evidence(&ev).unwrap();
            ev.id
        })
        .collect();
    let mut session = Session::isolated(dir.path());
    let started = Instant::now();
    session.open_project(project.clone()).unwrap();
    let first = started.elapsed();
    let addresses: Vec<_> = ids.iter().map(|id| payload_address(&session, id)).collect();
    let started = Instant::now();
    for _ in 0..5 {
        session.open_project(project.clone()).unwrap();
    }
    eprintln!("refresh benchmark: 16 records / 8 MiB payload, first={first:?}, five unchanged={:?}", started.elapsed());
    for (id, address) in ids.iter().zip(&addresses) {
        assert_eq!(payload_address(&session, id), *address, "an unchanged refresh must reuse the verified payload allocation");
    }

    let added = evidence(16, 128);
    project.store_evidence(&added).unwrap();
    session.open_project(project).unwrap();
    assert_eq!(session.evidence(&added.id), Some(&added));
    assert_eq!(session.ledger_len(), ids.len() + 1);
    for (id, address) in ids.iter().zip(addresses) {
        assert_eq!(payload_address(&session, id), address, "new evidence must not reload the unchanged records beside it");
    }
}

#[test]
fn preserved_metadata_cannot_hide_changed_or_partially_written_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(&dir.path().join("project.cany"));
    let mut ev = evidence(0, 256);
    project.store_evidence(&ev).unwrap();
    let path = evidence_path(&project, &ev.id);
    let metadata = std::fs::metadata(&path).unwrap();
    let modified = metadata.modified().unwrap();
    let mut session = Session::isolated(dir.path());
    session.open_project(project.clone()).unwrap();

    // Display fields are outside the canonical evidence id. Rewrites must still become visible,
    // even when a writer preserves both the byte count and the modification time.
    for (summary, replace) in [("new label", false), ("next name", true)] {
        ev.summary = summary.into();
        assert!(ev.verify());
        let bytes = serde_json::to_vec_pretty(&ev).unwrap();
        assert_eq!(bytes.len() as u64, metadata.len());
        if replace {
            write_atomic(&path, &bytes).unwrap();
        } else {
            std::fs::write(&path, &bytes).unwrap();
        }
        set_modified(&path, modified);
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), modified);
        session.open_project(project.clone()).unwrap();
        assert_eq!(session.evidence(&ev.id), Some(&ev));
    }

    let valid = serde_json::to_vec_pretty(&ev).unwrap();
    let mut tampered = ev.clone();
    tampered.result["payload"] = json!("y".repeat(256));
    let tampered = serde_json::to_vec_pretty(&tampered).unwrap();
    assert_eq!(tampered.len(), valid.len());
    for bad in [tampered, valid[..valid.len() / 2].to_vec()] {
        std::fs::write(&path, bad).unwrap();
        set_modified(&path, modified);
        session.open_project(project.clone()).unwrap();
        assert_eq!(session.evidence(&ev.id), Some(&ev), "a bad disk record does not replace previously verified evidence");
        std::fs::write(&path, &valid).unwrap();
        set_modified(&path, modified);
        session.open_project(project.clone()).unwrap();
        assert_eq!(session.evidence(&ev.id), Some(&ev), "a failed read must be retried after repair");
    }
}

#[test]
fn a_local_record_with_the_same_id_does_not_impersonate_the_disk_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(&dir.path().join("project.cany"));
    let ev = evidence(0, 64);
    project.store_evidence(&ev).unwrap();
    let mut session = Session::isolated(dir.path());
    session.open_project(project.clone()).unwrap();
    session.open_project(project.clone()).unwrap();
    let mut local = ev.clone();
    local.summary = "local display".into();
    assert!(local.verify());
    session.record(local.clone()).unwrap();
    assert_eq!(session.evidence(&ev.id), Some(&local));
    assert_eq!(project.load_evidence(&ev.id).unwrap(), ev, "storing an existing id keeps its file");
    session.open_project(project).unwrap();
    assert_eq!(session.evidence(&ev.id), Some(&ev), "refresh must restore the actual disk record");
}

#[test]
fn project_switches_and_a_cleared_ledger_require_their_own_records() {
    let dir = tempfile::tempdir().unwrap();
    let a = project(&dir.path().join("a.cany"));
    let b = project(&dir.path().join("b.cany"));
    let first = evidence(0, 64);
    let mut second = first.clone();
    second.summary = "other project".into();
    a.store_evidence(&first).unwrap();
    b.store_evidence(&second).unwrap();
    let mut session = Session::isolated(dir.path());
    session.open_project(a.clone()).unwrap();
    session.open_project(b).unwrap();
    assert_eq!(session.evidence(&first.id), Some(&second));
    session.open_project(a.clone()).unwrap();
    assert_eq!(session.evidence(&first.id), Some(&first));
    session.clear_ledger();
    session.open_project(a).unwrap();
    assert_eq!(session.evidence(&first.id), Some(&first));
}

#[test]
fn unchanged_records_still_consume_the_record_budget() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(&dir.path().join("project.cany"));
    let mut oldest = None;
    for tag in 0..=MAX_LOADED_EVIDENCE {
        let ev = evidence(tag, 0);
        let path = evidence_path(&project, &ev.id);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_vec_pretty(&ev).unwrap()).unwrap();
        set_modified(&path, SystemTime::UNIX_EPOCH + Duration::from_secs(tag as u64 + 1));
        if tag == 0 {
            oldest = Some(ev.id);
        }
    }
    let oldest = oldest.unwrap();
    let mut session = Session::isolated(dir.path());
    for _ in 0..2 {
        session.open_project(project.clone()).unwrap();
        assert_eq!(session.ledger_len(), MAX_LOADED_EVIDENCE);
        assert_eq!(session.unloaded_evidence(), 1);
        assert!(session.evidence(&oldest).is_none());
        assert_eq!(session.resolve_evidence(&oldest).unwrap().id, oldest, "older evidence remains available by id");
    }
}

#[test]
fn unchanged_records_still_consume_the_byte_budget() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(&dir.path().join("project.cany"));
    let mut oldest = None;
    for tag in 0..3 {
        let ev = evidence(tag, 22 * 1024 * 1024);
        project.store_evidence(&ev).unwrap();
        set_modified(&evidence_path(&project, &ev.id), SystemTime::UNIX_EPOCH + Duration::from_secs(tag as u64 + 1));
        if tag == 0 {
            oldest = Some(ev.id);
        }
    }
    let oldest = oldest.unwrap();
    let mut session = Session::isolated(dir.path());
    for _ in 0..2 {
        session.open_project(project.clone()).unwrap();
        assert_eq!(session.ledger_len(), 2);
        assert_eq!(session.unloaded_evidence(), 1);
        assert!(session.evidence(&oldest).is_none());
    }
}

#[test]
fn external_investigation_rewrites_remain_visible_with_identical_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(&dir.path().join("project.cany"));
    let path = project.root.join("investigation/state.json");
    let state = |revision, statement| {
        serde_json::to_vec(&json!({"revision": revision, "findings": [{"id": "fnd_1", "statement": statement, "evidence_ids": [], "confidence": "observed"}]}))
            .unwrap()
    };
    write_atomic(&path, &state(1, "first")).unwrap();
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    let mut session = Session::isolated(dir.path());
    session.open_project(project.clone()).unwrap();
    session.open_project(project.clone()).unwrap();
    assert_eq!(investigation_tools::cached(&session).findings[0].statement, "first");
    write_atomic(&path, &state(2, "later")).unwrap();
    set_modified(&path, modified);
    session.open_project(project.clone()).unwrap();
    assert_eq!(investigation_tools::cached(&session).revision, 2);
    assert_eq!(investigation_tools::cached(&session).findings[0].statement, "later");
    std::fs::write(&path, b"{").unwrap();
    session.open_project(project.clone()).unwrap();
    assert!(investigation_tools::cached(&session).findings.is_empty(), "a corrupt state is not a cache hit");
    write_atomic(&path, &state(3, "fixed")).unwrap();
    session.open_project(project).unwrap();
    assert_eq!(investigation_tools::cached(&session).findings[0].statement, "fixed");
}
