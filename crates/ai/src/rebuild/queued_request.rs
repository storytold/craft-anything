//! A follow-up waiting for a build slot: the person's words, original request time, copied
//! attachments and identities of waiting notes it already carries. Restarting uses the current
//! builder, mode and limits, and never starts a run by itself.

use super::{RunOptions, Workspace, notes};
use craft_anything_engine::versioned;
use craft_anything_evidence::{ErrorCode, ToolError};
use craft_anything_platform::fs::{read_file_capped, write_atomic};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const FILE: &str = "inbox/queued-request.json";
const VERSION: u32 = 3;
const MAX_BYTES: u64 = 16 * 1024 * 1024;
const STEPS: &[versioned::Step<Value>] = &[v1_to_v2, v2_to_v3];
const _: () = assert!(1 + STEPS.len() == VERSION as usize);

fn v1_to_v2(doc: &mut Value) -> Result<(), String> {
    let doc = doc.as_object_mut().ok_or("the queued request is not an object")?;
    if doc.contains_key("carried_notes") {
        return Err("version 1 did not record carried notes".into());
    }
    // Older requests had no proof that a waiting note was included; keep those notes.
    doc.insert("carried_notes".into(), serde_json::json!([]));
    doc.insert("version".into(), Value::from(2));
    Ok(())
}

fn v2_to_v3(doc: &mut Value) -> Result<(), String> {
    let doc = doc.as_object_mut().ok_or("the queued request is not an object")?;
    if doc.contains_key("asked") {
        return Err("version 2 did not record the request time".into());
    }
    // Its original time is unknown. Restarting or copying the file must not make this older
    // request new enough to clear a later Ghidra stop.
    doc.insert("asked".into(), serde_json::to_value(UNIX_EPOCH).map_err(|e| e.to_string())?);
    doc.insert("version".into(), Value::from(3));
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct QueuedRequest {
    version: u32,
    message: String,
    asked: SystemTime,
    shown: Option<String>,
    #[serde(default)]
    shown_pictures: Vec<PathBuf>,
    #[serde(default)]
    carried_notes: Vec<notes::Carried>,
}

impl QueuedRequest {
    pub(super) fn from_options(opts: &RunOptions) -> Option<Self> {
        let message = opts.message.as_ref().filter(|m| !m.trim().is_empty())?;
        Some(Self {
            version: VERSION,
            message: message.clone(),
            asked: opts.asked,
            shown: opts.shown.clone(),
            shown_pictures: opts.shown_pictures.clone(),
            carried_notes: opts.carried_notes.clone(),
        })
    }
}

fn unreadable(reason: impl std::fmt::Display) -> ToolError {
    ToolError::invalid(format!("Couldn't read the queued request ({reason}); it is left unchanged."))
        .with_remediation(format!("Keep a copy of {FILE} and repair it before starting another build."))
}

fn load(ws: &Workspace) -> Result<Option<QueuedRequest>, ToolError> {
    let path = ws.root.join(FILE);
    let bytes = match read_file_capped(&path, MAX_BYTES) {
        Ok(bytes) => bytes,
        // A dangling link also reads as not found. Only an absent path can be replaced without
        // losing a saved request or the person's link to it.
        Err(e) if e.code == ErrorCode::NotFound && std::fs::symlink_metadata(&path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) => {
            return Ok(None);
        }
        Err(e) => return Err(unreadable(e.message)),
    };
    let mut doc: Value = serde_json::from_slice(&bytes).map_err(unreadable)?;
    let found = doc.get("version").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).ok_or_else(|| unreadable("invalid version"))?;
    if found > VERSION {
        return Err(ToolError::unsupported("The queued request was saved by a newer Craft Anything and is left unchanged.")
            .with_remediation("Update Craft Anything before continuing this request."));
    }
    versioned::upgrade(&mut doc, found, 1, STEPS).map_err(unreadable)?;
    let saved: QueuedRequest = serde_json::from_value(doc).map_err(unreadable)?;
    if saved.message.trim().is_empty() {
        return Err(unreadable("the request has no words"));
    }
    Ok(Some(saved))
}

/// Check the current backup without replacing a queued run's options or cleanup identity.
/// An absent record is fine; malformed, newer or unreadable records still stop a start.
pub(super) fn validate(ws: &Workspace) -> Result<(), ToolError> {
    load(ws).map(|_| ())
}

/// Restore before deciding whether the run needs Study or a queued build slot. A new explicit
/// message takes precedence, but an unreadable older request is never silently discarded.
pub(super) fn restore(ws: &Workspace, opts: &mut RunOptions) -> Result<Option<QueuedRequest>, ToolError> {
    let saved = load(ws)?;
    if opts.message.is_none()
        && let Some(saved) = &saved
    {
        opts.message = Some(saved.message.clone());
        opts.asked = saved.asked;
        opts.shown.clone_from(&saved.shown);
        opts.shown_pictures.clone_from(&saved.shown_pictures);
        opts.carried_notes.clone_from(&saved.carried_notes);
    }
    Ok(saved)
}

pub(super) fn save(ws: &Workspace, opts: &RunOptions) -> Result<(), ToolError> {
    let Some(saved) = QueuedRequest::from_options(opts) else { return Ok(()) };
    // A newer or broken record may have appeared since the caller read it.
    let _ = load(ws)?;
    let bytes = serde_json::to_vec(&saved).map_err(|e| ToolError::internal(e.to_string()))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(ToolError::new(ErrorCode::OverLimit, "The queued request is too large to keep.")
            .with_remediation("Shorten the request or attach its material as files."));
    }
    let path = ws.root.join(FILE);
    std::fs::create_dir_all(ws.inbox_dir()).map_err(|e| ToolError::io(ws.inbox_dir().display(), e))?;
    write_atomic(&path, &bytes)
}

/// The worker has started and owns the request. A later request or an unreadable record stays;
/// failure to remove this backup is reported, since a later Continue could offer it again.
pub(super) fn finished(ws: &Workspace, saved: Option<&QueuedRequest>) {
    let Some(saved) = saved else { return };
    let remove = || -> Result<(), ToolError> {
        if load(ws)?.as_ref() == Some(saved) {
            let path = ws.root.join(FILE);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(ToolError::io(path.display(), e)),
            }
        }
        Ok(())
    };
    if let Err(e) = remove() {
        log::warn!("The build started, but its queued request couldn't be cleared and may be offered again: {}", e.message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_anything_engine::workspace::TargetRef;

    fn project() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::create(dir.path(), "queued", "A notes app", TargetRef::None).unwrap();
        (dir, ws)
    }

    fn request(message: &str) -> RunOptions {
        RunOptions { message: Some(message.into()), ..Default::default() }
    }

    #[test]
    fn a_literal_v1_record_restores_words_and_pictures_without_old_run_settings() {
        let (_dir, ws) = project();
        std::fs::create_dir_all(ws.inbox_dir()).unwrap();
        std::fs::write(ws.root.join(FILE), br#"{"version":1,"message":"Add a search box","shown":"Add search","shown_pictures":["attachments/search.png"]}"#)
            .unwrap();
        let mut opts = RunOptions { backend: Some("Current builder".into()), max_rounds: 2, clean_room: Some(true), ..Default::default() };
        restore(&ws, &mut opts).unwrap();
        assert_eq!(opts.message.as_deref(), Some("Add a search box"));
        assert_eq!(opts.shown.as_deref(), Some("Add search"));
        assert_eq!(opts.shown_pictures, [PathBuf::from("attachments/search.png")]);
        assert_eq!(opts.backend.as_deref(), Some("Current builder"));
        assert_eq!(opts.max_rounds, 2);
        assert_eq!(opts.clean_room, Some(true));
        assert!(opts.carried_notes.is_empty(), "a version-1 request proves no waiting notes were carried");
    }

    #[test]
    fn a_saved_request_restores_its_carried_note_identities() {
        let (_dir, ws) = project();
        let asked = std::time::UNIX_EPOCH + std::time::Duration::from_nanos(1_234_567_890);
        let note = notes::Carried {
            id: 7,
            note: notes::Left {
                text: "Add a search box".into(),
                shown: "Add search".into(),
                pictures: vec![PathBuf::from("attachments/search.png")],
                added: Some(asked),
            },
        };
        let opts = RunOptions { carried_notes: vec![note.clone()], asked, ..request("Add a search box") };
        save(&ws, &opts).unwrap();
        let mut restored = RunOptions::default();
        restore(&Workspace::open(&ws.root).unwrap(), &mut restored).unwrap();
        assert_eq!(restored.carried_notes, [note]);
        assert_eq!(restored.message, opts.message);
        assert_eq!(restored.asked, asked, "the queued request keeps its original time, including subsecond precision");
        assert_eq!(load(&ws).unwrap().unwrap().version, VERSION);
    }

    #[test]
    fn older_requests_without_a_time_do_not_clear_a_later_stop() {
        use craft_anything_engine::tools::deep;
        for bytes in [
            r#"{"version":1,"message":"Old request","shown":null,"shown_pictures":[]}"#,
            r#"{"version":2,"message":"Old request","shown":null,"shown_pictures":[],"carried_notes":[]}"#,
        ] {
            let (_dir, ws) = project();
            std::fs::create_dir_all(ws.inbox_dir()).unwrap();
            let path = ws.root.join(FILE);
            std::fs::write(&path, bytes).unwrap();
            let root = ws.root.clone();
            drop(ws);
            deep::stop_for(&root).unwrap();
            let marker = std::fs::read(root.join(deep::STOPPED)).unwrap();
            let ws = Workspace::open(&root).unwrap();
            let after_stop = std::time::SystemTime::now() + std::time::Duration::from_secs(1);
            let mut restored = RunOptions { asked: after_stop, ..Default::default() };

            restore(&ws, &mut restored).unwrap();
            deep::resume_for(&root, restored.asked).unwrap();

            assert!(deep::stopped(&root), "an unknown original time must not become a new request after restart");
            assert_eq!(std::fs::read(root.join(deep::STOPPED)).unwrap(), marker);
            assert_eq!(restored.message.as_deref(), Some("Old request"));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), bytes, "migration on read does not consume or rewrite the backup");

            let mut fresh = RunOptions { asked: after_stop, ..request("A genuinely new request") };
            restore(&ws, &mut fresh).unwrap();
            deep::resume_for(&root, fresh.asked).unwrap();
            assert!(!deep::stopped(&root), "a new explicit request still supersedes the earlier stop");
            assert_eq!(fresh.message.as_deref(), Some("A genuinely new request"));
            assert_eq!(fresh.asked, after_stop);
        }
    }

    #[test]
    fn an_explicit_new_request_wins_but_failed_starts_leave_the_backup() {
        let (_dir, ws) = project();
        save(&ws, &request("Earlier request")).unwrap();
        let before = std::fs::read(ws.root.join(FILE)).unwrap();
        let mut opts = request("New request");
        let saved = restore(&ws, &mut opts).unwrap();
        assert_eq!(opts.message.as_deref(), Some("New request"));
        assert_eq!(std::fs::read(ws.root.join(FILE)).unwrap(), before, "reading is not consumption");
        finished(&ws, saved.as_ref());
        assert!(!ws.root.join(FILE).exists(), "a successful explicit replacement retires the old request");
    }

    #[test]
    fn finishing_an_older_start_keeps_a_later_request() {
        let (_dir, ws) = project();
        save(&ws, &request("Earlier request")).unwrap();
        let saved = restore(&ws, &mut RunOptions::default()).unwrap();
        save(&ws, &request("Later request")).unwrap();
        finished(&ws, saved.as_ref());
        let mut next = RunOptions::default();
        restore(&ws, &mut next).unwrap();
        assert_eq!(next.message.as_deref(), Some("Later request"));
    }

    #[test]
    fn bad_or_newer_records_are_left_unchanged_even_with_an_explicit_request() {
        let (_dir, ws) = project();
        std::fs::create_dir_all(ws.inbox_dir()).unwrap();
        for bytes in [
            "{unfinished",
            r#"{"message":"No version"}"#,
            r#"{"version":0,"message":"No such old format"}"#,
            r#"{"version":4,"message":"Keep the newer data","future":true}"#,
            r#"{"version":1,"message":"Known format","unexpected":true}"#,
            r#"{"version":1,"message":"   "}"#,
            r#"{"version":1,"message":12}"#,
            r#"{"version":2,"message":"No time in this format","asked":{"secs_since_epoch":1,"nanos_since_epoch":0}}"#,
            r#"{"version":3,"message":"Missing request time"}"#,
            r#"{"version":3,"message":"Invalid request time","asked":null}"#,
            r#"{"version":3,"message":"Invalid request time","asked":{"secs_since_epoch":-1,"nanos_since_epoch":0}}"#,
        ] {
            std::fs::write(ws.root.join(FILE), bytes).unwrap();
            let mut opts = request("Do not replace unreadable words");
            assert!(restore(&ws, &mut opts).is_err(), "{bytes}");
            assert!(save(&ws, &opts).is_err(), "{bytes}");
            assert_eq!(std::fs::read_to_string(ws.root.join(FILE)).unwrap(), bytes);
        }
    }

    #[test]
    fn oversized_and_non_file_records_are_not_replaced() {
        let (_dir, ws) = project();
        save(&ws, &request("Keep me")).unwrap();
        let before = std::fs::read(ws.root.join(FILE)).unwrap();
        let oversized = request(&"x".repeat(MAX_BYTES as usize));
        assert_eq!(save(&ws, &oversized).unwrap_err().code, ErrorCode::OverLimit);
        assert_eq!(std::fs::read(ws.root.join(FILE)).unwrap(), before);
        std::fs::write(ws.root.join(FILE), vec![b'x'; MAX_BYTES as usize + 1]).unwrap();
        assert!(restore(&ws, &mut RunOptions::default()).is_err());
        assert!(save(&ws, &request("Replacement")).is_err());
        assert_eq!(std::fs::metadata(ws.root.join(FILE)).unwrap().len(), MAX_BYTES + 1);
        std::fs::remove_file(ws.root.join(FILE)).unwrap();
        std::fs::create_dir(ws.root.join(FILE)).unwrap();
        assert!(restore(&ws, &mut RunOptions::default()).is_err());
        assert!(save(&ws, &request("Replacement")).is_err());
        assert!(ws.root.join(FILE).is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_request_link_is_not_treated_as_an_absent_request() {
        let (_dir, ws) = project();
        std::fs::create_dir_all(ws.inbox_dir()).unwrap();
        let path = ws.root.join(FILE);
        std::os::unix::fs::symlink("missing-request.json", &path).unwrap();
        assert!(restore(&ws, &mut RunOptions::default()).is_err());
        assert!(save(&ws, &request("Replacement")).is_err());
        assert!(std::fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_link(&path).unwrap(), PathBuf::from("missing-request.json"));
    }
}
