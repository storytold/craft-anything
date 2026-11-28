//! Notes the person adds while the builder works, handed over at the builder's next step instead
//! of after its whole turn. The run writes each one to `inbox/notes/<id>.json`; a coding agent
//! that runs a command after every tool call (Claude Code's PostToolUse hook, running
//! `craft-anything-cli builder-notes <dir>`) claims what's there with no-replace publication to
//! `<id>.read.json`, removes the waiting name and gives the text to the model. That is the one moment a note changes hands,
//! so "take back" works until then. The run records the request's state before renaming that file
//! to `<id>.recorded.json` and forgetting it; a claimed version-1 note is still recoverable until
//! that recording completes. A restart may hand it over again, so this is at-least-once recovery.
//! Notes nobody claimed still go in after the turn, as before. A note's file also keeps the
//! person's own words ([`Left`]), so a note still waiting when the app closes isn't lost: the
//! next run takes it as its own waiting note ([`take_left`]).

use craft_anything_engine::versioned;
use craft_anything_evidence::ToolError;
use craft_anything_platform::sync::Mutex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const VERSION: u32 = 1;
const STEPS: &[versioned::Step<Value>] = &[v0_to_v1];
const _: () = assert!(STEPS.len() == VERSION as usize);

fn v0_to_v1(value: &mut Value) -> Result<(), String> {
    if value.is_object() { Ok(()) } else { Err("the note isn't an object".into()) }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct NoteFile {
    version: u32,
    id: u64,
    text: String,
    left: Option<Left>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Waiting,
    Read,
    Recorded,
}

/// Where a project's notes wait for the builder.
pub fn notes_dir(inbox: &Path) -> PathBuf {
    inbox.join("notes")
}

fn pending(dir: &Path, id: u64) -> PathBuf {
    dir.join(format!("{id}.json"))
}

fn read(dir: &Path, id: u64) -> PathBuf {
    dir.join(format!("{id}.read.json"))
}

fn recorded(dir: &Path, id: u64) -> PathBuf {
    dir.join(format!("{id}.recorded.json"))
}

/// A note as the person sent it: what a later run sends if this one ends before handing it over.
/// `text` is without the Just-ask answers the hand-over text may add, so a run in the other mode
/// adds only the ones it may show.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Left {
    pub text: String,
    pub shown: String,
    #[serde(default)]
    pub pictures: Vec<PathBuf>,
    /// When the person added it, so a later run doesn't take it for a request made after a stop
    /// ([`craft_anything_engine::tools::deep::resume_for`]). An older version's note has none.
    #[serde(default)]
    pub added: Option<std::time::SystemTime>,
}

/// A saved waiting note already included in a following request. Both its id and its original
/// contents must match: a later note reusing an old id still belongs to the next run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Carried {
    pub id: u64,
    pub note: Left,
}

impl From<&super::Note> for Carried {
    fn from(note: &super::Note) -> Self {
        Self { id: note.id, note: Left { text: note.text.clone(), shown: note.shown.clone(), pictures: note.pictures.clone(), added: Some(note.added) } }
    }
}

/// A recoverable record and the exact bytes inspected before taking it over. This is only
/// an in-memory handoff; the files remain version 1 and keep their original note id.
#[derive(Debug)]
pub(super) struct LeftNote {
    pub id: u64,
    pub left: Left,
    waiting: Option<Vec<u8>>,
    read: Option<Vec<u8>>,
}

struct Ids {
    next: u64,
}

impl Ids {
    fn reserve(&mut self, used: &BTreeSet<u64>) -> Result<(), ToolError> {
        if let Some(last) = used.last() {
            self.next = self.next.max(last.checked_add(1).ok_or_else(|| ToolError::invalid("The waiting note ids have no space for a later note."))?);
        }
        Ok(())
    }

    fn allocate(&mut self) -> Result<u64, ToolError> {
        let id = self.next;
        self.next = id.checked_add(1).ok_or_else(|| ToolError::invalid("The waiting note ids have no space for a later note."))?;
        Ok(id)
    }
}

static IDS: Mutex<Ids> = Mutex::new(Ids { next: 1 });

fn named_id(name: &str) -> Option<(u64, State)> {
    let name = name.strip_suffix(".json")?;
    let (number, state) = if let Some(number) = name.strip_suffix(".read") {
        (number, State::Read)
    } else if let Some(number) = name.strip_suffix(".recorded") {
        (number, State::Recorded)
    } else {
        (name, State::Waiting)
    };
    number.parse().ok().map(|id| (id, state))
}

/// Unknown files also reserve their names. A process that starts its counter again cannot
/// overwrite a waiting, claimed, completed, or newer-format record.
fn occupied_ids(dir: &Path) -> Result<BTreeSet<u64>, ToolError> {
    let entries = match std::fs::read_dir(dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        result => result.map_err(|error| ToolError::io(dir.display(), error))?,
    };
    let mut ids = BTreeSet::new();
    for entry in entries {
        let entry = entry.map_err(|error| ToolError::io(dir.display(), error))?;
        if let Some((id, _)) = entry.file_name().to_str().and_then(named_id) {
            ids.insert(id);
        }
    }
    Ok(ids)
}

pub(super) fn reserve_ids(dir: &Path) -> Result<(), ToolError> {
    let used = occupied_ids(dir)?;
    IDS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).reserve(&used)
}

pub(super) fn reserve_after(id: u64) -> Result<(), ToolError> {
    IDS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).reserve(&BTreeSet::from([id]))
}

pub(super) fn new_id() -> Result<u64, ToolError> {
    IDS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).allocate()
}

pub(super) fn sync_directory(dir: &Path) -> Result<(), ToolError> {
    #[cfg(unix)]
    std::fs::File::open(dir).and_then(|file| file.sync_all()).map_err(|error| ToolError::io(dir.display(), error))?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// Leave a note for the builder's next step: `text` is what the agent's hook hands over.
/// Recovery requires every persistence step to succeed. An error can leave this same id
/// published; retain it for retry instead of allocating another id for the same request.
pub fn hand_over(dir: &Path, id: u64, text: &str, left: Option<&Left>) -> Result<(), craft_anything_evidence::ToolError> {
    match publish(dir, id, text, left, sync_directory)?.persistence_warning {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

pub(super) struct Published {
    pub persistence_warning: Option<ToolError>,
}

/// Publication lets an independent hook claim the note, even before the last directory sync
/// returns. Report that sync separately: a new sender cannot safely retry an already visible id.
/// Recovery callers keep using hand_over(), which requires every persistence step to succeed.
pub(super) fn publish(dir: &Path, id: u64, text: &str, left: Option<&Left>, sync: impl Fn(&Path) -> Result<(), ToolError>) -> Result<Published, ToolError> {
    publish_checking_claim(dir, id, text, left, sync, |source, destination| publish_claim(source, destination, |from, to| std::fs::hard_link(from, to)))
}

fn publish_checking_claim(
    dir: &Path,
    id: u64,
    text: &str,
    left: Option<&Left>,
    sync: impl Fn(&Path) -> Result<(), ToolError>,
    claim: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> Result<Published, ToolError> {
    std::fs::create_dir_all(dir).map_err(|e| craft_anything_evidence::ToolError::io(dir.display(), e))?;
    // The no-clobber rename can itself fall back to a hard link on some systems. Exercise the
    // actual publication path on this drive before accepting any of the person's words.
    let probe = tempfile::Builder::new().prefix("claim-probe-").tempdir_in(dir).map_err(|e| ToolError::io(dir.display(), e))?;
    let source = probe.path().join("waiting");
    std::fs::write(&source, b"").map_err(|e| ToolError::io(source.display(), e))?;
    claim(&source, &probe.path().join("claimed")).map_err(|e| {
        ToolError::io(dir.display(), e)
            .with_remediation("The note was not accepted. Restore access or move the project to a drive that supports atomic note delivery, then try again.")
    })?;
    probe.close().map_err(|e| ToolError::io(dir.display(), e))?;
    if let Some(parent) = dir.parent() {
        sync(parent)?;
    }
    let bytes = serde_json::to_vec(&NoteFile { version: VERSION, id, text: text.into(), left: left.cloned() })
        .map_err(|e| craft_anything_evidence::ToolError::internal(e.to_string()))?;
    craft_anything_engine::project::write_atomic(&pending(dir, id), &bytes)?;
    Ok(Published { persistence_warning: sync(dir).err() })
}

/// The notes an earlier run left waiting (the app closed before the builder took them, #1620),
/// oldest first. A file without the person's words (from before they were kept) can't be sent
/// safely and is left out, as is one that isn't a note. A version-1 read note still needs its
/// request state recorded; a legacy read note has no such evidence and stays excluded. A
/// folder or file that can't be read, or a newer format, is an error, not "no notes".
fn left(dir: &Path) -> std::io::Result<Vec<LeftNote>> {
    let entries = match std::fs::read_dir(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        r => r?,
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry?;
        if let Some((id, state)) = entry.file_name().to_str().and_then(named_id) {
            files.push((id, entry.path(), state));
        }
    }
    files.sort_unstable_by_key(|(id, _, _)| *id);
    let mut notes: BTreeMap<u64, LeftNote> = BTreeMap::new();
    let mut completed = BTreeSet::new();
    let mut active = BTreeSet::new();
    for (id, path, state) in files {
        if state != State::Recorded {
            active.insert(id);
        }
        if !std::fs::symlink_metadata(&path)?.file_type().is_file() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "the note isn't a regular file"));
        }
        let bytes = std::fs::read(path)?;
        let Some((found, file)) = decode(&bytes)? else {
            if state != State::Waiting {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "the claimed or completed note isn't a recognized record"));
            }
            continue;
        };
        if state != State::Waiting && file.id != id {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "the claimed or completed note's identity doesn't match its name"));
        }
        if state == State::Recorded {
            completed.insert(id);
        }
        if file.id == id
            && (state == State::Waiting || (state == State::Read && found == VERSION))
            && let Some(left) = file.left.filter(|left| !left.text.trim().is_empty())
        {
            let note = notes.entry(id).or_insert_with(|| LeftNote { id, left: left.clone(), waiting: None, read: None });
            if note.left != left {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "the waiting and claimed records disagree about the note"));
            }
            if state == State::Waiting { note.waiting = Some(bytes) } else { note.read = Some(bytes) }
        }
    }
    if active.iter().any(|id| completed.contains(id)) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "a completed note also has a waiting or claimed record"));
    }
    Ok(notes.into_values().collect())
}

fn decode(bytes: &[u8]) -> std::io::Result<Option<(u32, NoteFile)>> {
    let Ok(mut value) = serde_json::from_slice::<Value>(bytes) else { return Ok(None) };
    let found = match value.get("version") {
        None => 0,
        Some(version) => version
            .as_u64()
            .and_then(|version| u32::try_from(version).ok())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "the waiting note's version isn't a whole number"))?,
    };
    if found > VERSION {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "the waiting note was saved by a newer Craft Anything"));
    }
    versioned::upgrade(&mut value, found, 0, STEPS).map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    if let Some(value) = value.as_object_mut() {
        value.insert("version".into(), Value::from(VERSION));
    }
    Ok(serde_json::from_value(value).ok().map(|file| (found, file)))
}

/// Inspect the whole batch before changing any record. A failed or interrupted startup keeps
/// the original files; unreadable or newer records stop startup and remain for inspection.
pub(super) fn take_left(dir: &Path) -> Result<Vec<LeftNote>, ToolError> {
    reserve_ids(dir)?;
    left(dir).map_err(|error| {
        ToolError::io(dir.display(), error)
            .with_remediation("The waiting notes were kept. Restore access or inspect their records before starting the build again.")
    })
}

fn unchanged(path: &Path, expected: Option<&[u8]>) -> Result<(), ToolError> {
    let matches = match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => expected.is_none(),
        Err(error) => return Err(ToolError::io(path.display(), error)),
        Ok(metadata) if metadata.file_type().is_file() => expected.is_some_and(|expected| std::fs::read(path).is_ok_and(|bytes| bytes == expected)),
        Ok(_) => false,
    };
    if matches { Ok(()) } else { Err(ToolError::invalid("A waiting note changed during recovery; its records were kept.")) }
}

/// Prefer a link so a failed directory sync can retain both names. A drive without hard links
/// can use the pinned library's no-clobber rename instead. Neither operation replaces an existing
/// claim, and moving the source also keeps withdrawal and delivery mutually exclusive.
fn publish_claim(source: &Path, destination: &Path, link: impl FnOnce(&Path, &Path) -> std::io::Result<()>) -> std::io::Result<()> {
    match link(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if matches!(error.kind(), std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::NotFound) => Err(error),
        Err(_) => {
            let mut source = tempfile::TempPath::try_from_path(source)?;
            // This is the person's durable waiting record, not a disposable temporary file.
            // In particular, a refused destination must never remove the original on drop.
            source.disable_cleanup(true);
            source.persist_noclobber(destination).map_err(|error| error.error)
        }
    }
}

/// Reuse the persistent id. A claimed note becomes waiting only after its new current-mode
/// copy is synced. Explicitly carried notes stay claimed until their initial turn succeeds.
pub(super) fn restore(dir: &Path, note: &LeftNote, text: &str, carried: bool) -> Result<(), ToolError> {
    restore_using(dir, note, text, carried, || Ok(()))
}

fn restore_using(dir: &Path, note: &LeftNote, text: &str, carried: bool, after_publish: impl FnOnce() -> Result<(), ToolError>) -> Result<(), ToolError> {
    restore_with_link(dir, note, text, carried, after_publish, |source, destination| std::fs::hard_link(source, destination))
}

fn restore_with_link(
    dir: &Path,
    note: &LeftNote,
    text: &str,
    carried: bool,
    after_publish: impl FnOnce() -> Result<(), ToolError>,
    link: impl Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), ToolError> {
    let waiting = pending(dir, note.id);
    let claimed = read(dir, note.id);
    unchanged(&waiting, note.waiting.as_deref())?;
    unchanged(&claimed, note.read.as_deref())?;
    if carried {
        if note.read.is_none() {
            // A legacy waiting note can be recovered, but a legacy read note cannot. Write
            // the current version durably before publishing its claimed successor.
            hand_over(dir, note.id, text, Some(&note.left))?;
            publish_claim(&waiting, &claimed, link).map_err(|error| ToolError::io(claimed.display(), error))?;
            after_publish()?;
            sync_directory(dir)?;
            remove_if_present(&waiting)?;
            sync_directory(dir)?;
        } else if note.waiting.is_some() {
            sync_directory(dir)?;
            std::fs::remove_file(&waiting).map_err(|error| ToolError::io(waiting.display(), error))?;
            sync_directory(dir)?;
        }
    } else {
        hand_over(dir, note.id, text, Some(&note.left))?;
        after_publish()?;
        if note.read.is_some() {
            unchanged(&claimed, note.read.as_deref())?;
            std::fs::remove_file(&claimed).map_err(|error| ToolError::io(claimed.display(), error))?;
            sync_directory(dir)?;
        }
    }
    Ok(())
}

/// Keep a note out of the hook while a follow-up prompt waits for its builder turn.
/// False means a read claim already exists; the caller remembers its own prepared claims
/// to distinguish them from hook delivery. Startup recovery uses restore() separately.
pub(super) fn carry_for_prompt(dir: &Path, id: u64, left: &Left, claimed: impl FnOnce() -> Result<(), ToolError>) -> Result<bool, ToolError> {
    let mut batch = take_left(dir)?;
    let note = match batch.iter().position(|note| note.id == id) {
        Some(index) => batch.swap_remove(index),
        None => {
            for path in [pending(dir, id), read(dir, id), recorded(dir, id)] {
                unchanged(&path, None)?;
            }
            hand_over(dir, id, &left.text, Some(left))?;
            take_left(dir)?.into_iter().find(|note| note.id == id).ok_or_else(|| ToolError::invalid("The waiting note changed before delivery."))?
        }
    };
    // Legacy notes have no saved time: recovery still treats them as a new request, but that
    // in-memory time is not a change to the old record. Known times remain part of its identity.
    if note.left.text != left.text
        || note.left.shown != left.shown
        || note.left.pictures != left.pictures
        || note.left.added.is_some_and(|added| Some(added) != left.added)
    {
        return Err(ToolError::invalid("The waiting note changed before delivery; its records were kept."));
    }
    if note.read.is_some() {
        return Ok(false);
    }
    restore_using(dir, &note, &left.text, true, claimed)?;
    Ok(true)
}

/// Take a note back before the builder read it: true when it was still waiting.
pub fn take_back(dir: &Path, id: u64) -> Result<bool, ToolError> {
    // A claimer can have published its read link just before the waiting name is removed.
    // The caller holds the queue lock, so completion cannot erase that evidence meanwhile.
    Ok(remove_if_present(&pending(dir, id))? && !was_read(dir, id))
}

/// The builder has read the note (it was claimed).
pub fn was_read(dir: &Path, id: u64) -> bool {
    read(dir, id).is_file() || recorded(dir, id).is_file()
}

/// The target's request state is saved. Keep that distinction even if removing the completed
/// note fails; restart must not replay successfully recorded claimed notes. No directory is
/// created here, and an error keeps claimed or completed evidence for another attempt.
pub(super) fn mark_recorded(dir: &Path, id: u64) -> Result<(), ToolError> {
    mark_recorded_using(dir, id, sync_directory, || {})
}

/// Notify only after this call has published or fully validated completion. A subsequent
/// directory-sync error still propagates, but the live handoff must not replay that request.
pub(super) fn mark_recorded_using(dir: &Path, id: u64, sync: impl Fn(&Path) -> Result<(), ToolError>, published: impl FnOnce()) -> Result<(), ToolError> {
    let source = read(dir, id);
    let destination = recorded(dir, id);
    let waiting = pending(dir, id);
    if !check_record(&source, id)? {
        // A partial batch may already have acknowledged this note. Validate that evidence
        // before allowing forget() to remove it; an unread note has neither entry.
        if check_record(&destination, id)? {
            unchanged(&waiting, None)?;
            published();
            sync(dir)?;
        }
        return Ok(());
    }
    match std::fs::symlink_metadata(&destination) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(ToolError::io(destination.display(), error)),
        Ok(_) => return Err(ToolError::invalid("The completed note's name is already occupied; both entries were kept for inspection.")),
    }
    if check_record(&waiting, id)? {
        let bytes = std::fs::read(&source).map_err(|error| ToolError::io(source.display(), error))?;
        unchanged(&waiting, Some(&bytes))?;
    }
    // A failed claim sync can leave both links. Persist the read name before removing
    // its waiting alias, and persist that removal before publishing completion. A stop
    // at either barrier leaves a recoverable claim, never waiting beside recorded.
    sync(dir)?;
    remove_if_present(&waiting)?;
    sync(dir)?;
    std::fs::rename(&source, &destination).map_err(|error| ToolError::io(source.display(), error))?;
    published();
    sync(dir)
}

fn check_record(path: &Path, id: u64) -> Result<bool, ToolError> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(ToolError::io(path.display(), error)),
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
        Ok(_) => return Err(ToolError::invalid("The note is not a regular file; it was kept for inspection.")),
    }
    let bytes = std::fs::read(path).map_err(|error| ToolError::io(path.display(), error))?;
    let decoded = decode(&bytes).map_err(|error| ToolError::io(path.display(), error))?;
    if decoded.is_none_or(|(_, file)| file.id != id) {
        return Err(ToolError::invalid("The note's record changed; it was kept for inspection."));
    }
    Ok(true)
}

fn remove_if_present(path: &Path) -> Result<bool, ToolError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(ToolError::io(path.display(), error)),
    }
}

/// Done with a note: its files go. Keep completion evidence if an earlier removal fails.
pub fn forget(dir: &Path, id: u64) -> Result<(), ToolError> {
    forget_using(dir, id, sync_directory)
}

pub(super) fn forget_using(dir: &Path, id: u64, sync: impl Fn(&Path) -> Result<(), ToolError>) -> Result<(), ToolError> {
    let mut removed = false;
    for path in [pending(dir, id), read(dir, id), recorded(dir, id)] {
        removed |= remove_if_present(&path)?;
    }
    if removed {
        sync(dir)?;
    }
    Ok(())
}

/// Start a run with no notes left over from an earlier one (ids restart with the process).
pub fn clear(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

/// For the agent's hook: claim every waiting note, oldest first, and return their texts. A note
/// another claimer (or "take back") got first is skipped.
pub fn claim(dir: &Path) -> Vec<String> {
    claim_using(dir, sync_directory)
}

fn claim_using(dir: &Path, sync: impl Fn(&Path) -> Result<(), ToolError>) -> Vec<String> {
    claim_with_link(dir, sync, |source, destination| std::fs::hard_link(source, destination))
}

fn claim_with_link(dir: &Path, sync: impl Fn(&Path) -> Result<(), ToolError>, link: impl Fn(&Path, &Path) -> std::io::Result<()>) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut ids: Vec<u64> =
        entries.flatten().filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".json")).and_then(|n| n.parse::<u64>().ok())).collect();
    ids.sort_unstable();
    ids.into_iter()
        .filter_map(|id| {
            // An occupied completion name is not a waiting note, even if it is unreadable.
            if !matches!(std::fs::symlink_metadata(recorded(dir, id)), Err(error) if error.kind() == std::io::ErrorKind::NotFound) {
                return None;
            }
            let (_, file) = std::fs::read(pending(dir, id)).ok().and_then(|bytes| decode(&bytes).ok().flatten())?;
            if file.id != id {
                return None;
            }
            publish_claim(&pending(dir, id), &read(dir, id), &link).ok()?;
            // The read name owns delivery. Keep the waiting alias when available if syncing
            // the claim fails; a later hook cannot replace the claim or deliver it again.
            if sync(dir).is_ok() {
                let _ = std::fs::remove_file(pending(dir, id));
            }
            Some(file.text)
        })
        .collect()
}

/// The notes as context for the builder's next step, or nothing: what a coding agent's hook
/// prints, and what the API-backend loop adds after a step's tool results.
pub fn context(texts: &[String]) -> Option<String> {
    if texts.is_empty() {
        return None;
    }
    let list = texts.iter().map(|t| format!("- {t}")).collect::<Vec<_>>().join("\n");
    // Stated as facts: the agent reads it as context, not as an order from the tool.
    Some(format!(
        "The person added this while you were working:\n{list}\nIt applies to the work from here on, and your reply can say how you took it into account."
    ))
}

/// What the hook prints for the agent: the notes as context for its next step, or nothing.
pub fn hook_output(texts: &[String]) -> Option<String> {
    let context = context(texts)?;
    Some(serde_json::json!({"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": context}}).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drive_without_safe_claim_publication_refuses_a_new_note_before_accepting_it() {
        let d = tempfile::tempdir().unwrap();
        let dir = notes_dir(d.path());
        let original = Left { text: "Keep the older request".into(), ..Default::default() };
        hand_over(&dir, 7, &original.text, Some(&original)).unwrap();
        let before = std::fs::read(pending(&dir, 7)).unwrap();
        let error = publish_checking_claim(&dir, 8, "New request", None, sync_directory, |source, destination| {
            assert!(source.is_file(), "the capability check uses a real disposable file on the project drive");
            assert!(!destination.exists());
            Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "neither hard links nor exclusive rename are available"))
        })
        .err()
        .unwrap();

        assert!(error.message.contains("neither hard links nor exclusive rename"));
        assert!(error.remediation.unwrap().contains("not accepted"));
        assert!(!pending(&dir, 8).exists(), "a rejected note cannot be observed by the builder's hook");
        assert_eq!(std::fs::read(pending(&dir, 7)).unwrap(), before);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "the disposable probe is cleaned after failure");
    }

    #[test]
    fn only_the_exact_waiting_note_carried_by_a_request_is_omitted() {
        for (id, picture, copies) in [(7, "blue.png", 1), (8, "blue.png", 2), (7, "another-blue.png", 2)] {
            let d = tempfile::tempdir().unwrap();
            let dir = notes_dir(d.path());
            let original = Left { text: "make it blue".into(), shown: "Blue".into(), pictures: vec![PathBuf::from("blue.png")], added: None };
            let waiting = Left { pictures: vec![PathBuf::from(picture)], ..original.clone() };
            hand_over(&dir, id, &waiting.text, Some(&waiting)).unwrap();
            let mut opts = super::super::RunOptions {
                message: Some("make it blue and keep the current layout".into()),
                carried_notes: vec![Carried { id: 7, note: original }],
                ..Default::default()
            };

            super::super::prepend_older_notes(&mut opts, &take_left(&dir).unwrap()).unwrap();

            let message = opts.message.unwrap();
            assert_eq!(message.matches("make it blue").count(), copies, "a different id or saved content remains a separate request");
            assert!(message.contains("and keep the current layout"), "the carried identity does not replace the whole message");
            assert!(pending(&dir, id).is_file(), "preparing the request leaves its durable note recoverable");
        }
    }

    fn left_words(dir: &Path) -> std::io::Result<Vec<Left>> {
        left(dir).map(|notes| notes.into_iter().map(|note| note.left).collect())
    }

    fn no_hard_links(_: &Path, _: &Path) -> std::io::Result<()> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "this drive does not support hard links"))
    }

    #[test]
    fn a_hook_claim_can_deliver_without_hard_links() {
        let d = tempfile::tempdir().unwrap();
        let dir = notes_dir(d.path());
        let original = Left { text: "Use Spanish".into(), shown: "Use Spanish".into(), ..Default::default() };
        hand_over(&dir, 71, "Use Spanish", Some(&original)).unwrap();
        let bytes = std::fs::read(pending(&dir, 71)).unwrap();

        assert_eq!(claim_with_link(&dir, sync_directory, no_hard_links), ["Use Spanish"]);
        assert!(was_read(&dir, 71));
        assert!(!pending(&dir, 71).exists());
        assert_eq!(std::fs::read(read(&dir, 71)).unwrap(), bytes);
        assert!(claim_with_link(&dir, sync_directory, no_hard_links).is_empty(), "a second hook cannot claim the note again");
        assert_eq!(left_words(&dir).unwrap(), [original], "the claimed note remains recoverable until recording succeeds");
        mark_recorded(&dir, 71).unwrap();
        assert!(take_left(&dir).unwrap().is_empty());
        forget(&dir, 71).unwrap();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn a_carried_legacy_note_can_start_without_hard_links() {
        let d = tempfile::tempdir().unwrap();
        let dir = notes_dir(d.path());
        std::fs::create_dir_all(&dir).unwrap();
        let original = Left { text: "Use Spanish".into(), shown: "Use Spanish".into(), ..Default::default() };
        let bytes = serde_json::to_vec(&serde_json::json!({"id":71,"text":"Earlier mode context","left":original})).unwrap();
        std::fs::write(pending(&dir, 71), bytes).unwrap();
        let batch = take_left(&dir).unwrap();

        restore_with_link(&dir, &batch[0], "Current mode context", true, || Ok(()), no_hard_links).unwrap();

        assert!(!pending(&dir, 71).exists());
        let (_, claimed) = decode(&std::fs::read(read(&dir, 71)).unwrap()).unwrap().unwrap();
        assert_eq!(claimed.version, VERSION);
        assert_eq!(claimed.text, "Current mode context");
        assert_eq!(claimed.left, Some(original.clone()));
        assert_eq!(left_words(&dir).unwrap(), [original]);
        assert!(claim_with_link(&dir, sync_directory, no_hard_links).is_empty(), "the initial prompt owns the carried note");
        mark_recorded(&dir, 71).unwrap();
        forget(&dir, 71).unwrap();
        assert!(take_left(&dir).unwrap().is_empty());
    }

    #[test]
    fn a_no_link_claim_preserves_a_competing_claim_or_withdrawal() {
        for change in ["file", "directory", "withdrawn"] {
            let d = tempfile::tempdir().unwrap();
            let dir = notes_dir(d.path());
            hand_over(&dir, 71, "Waiting change", None).unwrap();
            let before = std::fs::read(pending(&dir, 71)).unwrap();
            let got = claim_with_link(&dir, sync_directory, |source, destination| {
                // These changes happen after the hook has read the waiting record.
                match change {
                    "file" => std::fs::write(destination, b"Keep this earlier claim").unwrap(),
                    "directory" => {
                        std::fs::create_dir(destination).unwrap();
                        std::fs::write(destination.join("personal.txt"), b"Keep this unexpected entry").unwrap();
                    }
                    _ => std::fs::remove_file(source).unwrap(),
                }
                no_hard_links(source, destination)
            });
            assert!(got.is_empty(), "a competing claim or completed withdrawal owns delivery");
            if change == "withdrawn" {
                assert!(!pending(&dir, 71).exists());
                assert!(!read(&dir, 71).exists());
            } else {
                assert_eq!(std::fs::read(pending(&dir, 71)).unwrap(), before, "failed publication must not drop the durable source");
                let retained = if change == "file" { read(&dir, 71) } else { read(&dir, 71).join("personal.txt") };
                assert_eq!(
                    std::fs::read(retained).unwrap(),
                    if change == "file" { b"Keep this earlier claim".as_slice() } else { b"Keep this unexpected entry".as_slice() }
                );
            }
        }
    }

    #[test]
    fn a_no_link_claim_keeps_recovery_evidence_after_failed_directory_sync() {
        let d = tempfile::tempdir().unwrap();
        let dir = notes_dir(d.path());
        let original = Left { text: "Keep my change".into(), shown: "Keep my change".into(), ..Default::default() };
        hand_over(&dir, 71, "Keep my change", Some(&original)).unwrap();
        let delivered = claim_with_link(&dir, |_| Err(ToolError::invalid("Injected claim directory sync failure.")), no_hard_links);
        assert_eq!(delivered, ["Keep my change"]);
        assert!(claim_with_link(&dir, sync_directory, no_hard_links).is_empty());
        assert_eq!(left_words(&dir).unwrap(), [original], "the claimed record survives until the request is saved");
        mark_recorded(&dir, 71).unwrap();
        assert!(take_left(&dir).unwrap().is_empty());
    }

    #[test]
    fn a_claim_preserves_an_occupied_read_record() {
        let d = tempfile::tempdir().unwrap();
        let dir = notes_dir(d.path());
        hand_over(&dir, 71, "waiting", None).unwrap();
        let waiting = std::fs::read(pending(&dir, 71)).unwrap();
        std::fs::write(read(&dir, 71), b"Keep this newer or damaged read record").unwrap();
        assert!(claim(&dir).is_empty());
        assert_eq!(std::fs::read(pending(&dir, 71)).unwrap(), waiting);
        assert_eq!(std::fs::read(read(&dir, 71)).unwrap(), b"Keep this newer or damaged read record");
    }

    #[test]
    fn a_completed_id_blocks_even_a_note_without_recovery_words() {
        for (claimed, legacy) in [(false, false), (true, false), (false, true), (true, true)] {
            let d = tempfile::tempdir().unwrap();
            let dir = notes_dir(d.path());
            hand_over(&dir, 71, "already recorded", None).unwrap();
            assert_eq!(claim(&dir), ["already recorded"]);
            mark_recorded(&dir, 71).unwrap();
            hand_over(&dir, 71, "must not be delivered again", None).unwrap();
            let active = if claimed {
                std::fs::rename(pending(&dir, 71), read(&dir, 71)).unwrap();
                read(&dir, 71)
            } else {
                pending(&dir, 71)
            };
            if legacy {
                let mut value: Value = serde_json::from_slice(&std::fs::read(&active).unwrap()).unwrap();
                value.as_object_mut().unwrap().remove("version");
                value["left"] = serde_json::json!({"text": "legacy words", "shown": "legacy words", "pictures": []});
                std::fs::write(&active, serde_json::to_vec(&value).unwrap()).unwrap();
            }
            let before = std::fs::read(&active).unwrap();
            let completed = std::fs::read(recorded(&dir, 71)).unwrap();
            assert!(take_left(&dir).unwrap_err().message.contains("completed note"));
            assert!(claim(&dir).is_empty());
            assert_eq!(std::fs::read(&active).unwrap(), before);
            assert_eq!(std::fs::read(recorded(&dir, 71)).unwrap(), completed);
            std::fs::remove_file(active).unwrap();
            assert!(take_left(&dir).unwrap().is_empty(), "a lone completed record does not stop startup");
        }
    }

    #[test]
    fn a_published_claim_is_not_retractable_before_waiting_cleanup() {
        let d = tempfile::tempdir().unwrap();
        let dir = notes_dir(d.path());
        hand_over(&dir, 71, "already claimed", None).unwrap();
        std::fs::hard_link(pending(&dir, 71), read(&dir, 71)).unwrap();
        assert!(!take_back(&dir, 71).unwrap(), "removing the waiting name cannot retract the published claim");
        assert!(was_read(&dir, 71));
        assert!(claim(&dir).is_empty());
        assert!(std::fs::read_to_string(read(&dir, 71)).unwrap().contains("already claimed"));
    }

    #[test]
    fn a_note_changes_hands_once_and_can_be_taken_back_until_then() {
        let d = tempfile::tempdir().unwrap();
        let dir = notes_dir(d.path());
        assert!(claim(&dir).is_empty(), "no folder, nothing to claim");
        hand_over(&dir, 2, "make the header blue", None).unwrap();
        hand_over(&dir, 10, "add a contact form", None).unwrap();
        hand_over(&dir, 3, "never mind the footer", None).unwrap();
        assert!(take_back(&dir, 3).unwrap(), "still waiting: it can be taken back");
        assert_eq!(claim(&dir), ["make the header blue", "add a contact form"], "oldest first, by number");
        assert!(was_read(&dir, 2) && was_read(&dir, 10) && !was_read(&dir, 3));
        assert!(!take_back(&dir, 2).unwrap(), "read notes can't be taken back");
        assert!(claim(&dir).is_empty(), "each note is handed over once");
        forget(&dir, 2).unwrap();
        assert!(!was_read(&dir, 2));
        clear(&dir);
        assert!(!dir.exists());
    }

    #[test]
    fn notes_nobody_took_are_left_for_the_next_run() {
        let d = tempfile::tempdir().unwrap();
        let dir = notes_dir(d.path());
        assert!(left_words(&dir).unwrap().is_empty(), "no folder, nothing left");
        let note = |text: &str| Left { text: text.into(), shown: format!("{text} (shown)"), pictures: vec![PathBuf::from("pictures/a.png")], added: None };
        hand_over(&dir, 10, "add a form, with what was asked in Just ask", Some(&note("add a form"))).unwrap();
        hand_over(&dir, 2, "make it blue", Some(&note("make it blue"))).unwrap();
        // A legacy read record has no proof that request-state recording remained pending.
        std::fs::write(
            dir.join("3.json"),
            br#"{"id":3,"text":"taken by the hook","left":{"text":"taken by the hook","shown":"taken by the hook (shown)","pictures":[]}}"#,
        )
        .unwrap();
        hand_over(&dir, 4, "from an older version", None).unwrap();
        hand_over(&dir, 5, " ", Some(&note(" "))).unwrap();
        std::fs::write(dir.join("6.json"), b"{not json").unwrap();
        std::fs::rename(dir.join("3.json"), dir.join("3.read.json")).unwrap();
        assert_eq!(left_words(&dir).unwrap(), [note("make it blue"), note("add a form")], "oldest first, the person's words without the answers");
        assert_eq!(claim(&dir)[0], "make it blue", "the hook still reads the hand-over text");
    }

    #[test]
    fn a_run_inspects_left_notes_without_erasing_their_durable_records() {
        let d = tempfile::tempdir().unwrap();
        let dir = notes_dir(d.path());
        // When it was added goes with it, so a run after a restart doesn't take it for a request
        // made after a stop of Ghidra (#1903 review).
        let added = std::time::UNIX_EPOCH + std::time::Duration::from_nanos(1_794_000_000_123_456_789);
        let note = Left { text: "make it blue".into(), shown: "make it blue".into(), pictures: Vec::new(), added: Some(added) };
        assert!(take_left(&dir).unwrap().is_empty(), "no folder, nothing left");
        hand_over(&dir, 1, "make it blue", Some(&note)).unwrap();
        let original = std::fs::read(pending(&dir, 1)).unwrap();
        let recovered = take_left(&dir).unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].id, 1);
        assert_eq!(recovered[0].left, note);
        assert_eq!(std::fs::read(pending(&dir, 1)).unwrap(), original, "inspection is not acknowledgement");
        drop(recovered);
        assert_eq!(take_left(&dir).unwrap().len(), 1, "another process can still recover the same note");

        std::fs::create_dir_all(dir.join("2.json")).unwrap();
        assert!(left_words(&dir).is_err());
        assert!(take_left(&dir).is_err());
        assert_eq!(std::fs::read(pending(&dir, 1)).unwrap(), original);
        assert!(dir.join("2.json").is_dir(), "an unreadable batch stays intact for inspection");
    }

    #[test]
    fn a_prompt_claim_reports_ownership_before_later_persistence_can_fail() {
        let directory = tempfile::tempdir().unwrap();
        let dir = notes_dir(directory.path());
        let left = Left { text: "Keep this prepared request".into(), shown: "Keep this prepared request".into(), pictures: Vec::new(), added: None };
        hand_over(&dir, 71, &left.text, Some(&left)).unwrap();
        let owned = std::cell::Cell::new(false);

        let error = carry_for_prompt(&dir, 71, &left, || {
            owned.set(true);
            assert!(read(&dir, 71).is_file(), "the callback observes an actually published claim");
            Err(ToolError::internal("interrupted after claim publication, before its sync"))
        })
        .unwrap_err();

        assert!(error.message.contains("interrupted after claim publication"));
        assert!(owned.get(), "the prompt's ownership must survive a later persistence failure");
        assert!(claim(&dir).is_empty(), "the hook cannot also deliver the prepared request");
        assert_eq!(take_left(&dir).unwrap()[0].left, left, "the undelivered claim stays recoverable");
        assert!(!carry_for_prompt(&dir, 71, &left, || panic!("an existing claim cannot be acquired again")).unwrap());
    }

    #[test]
    fn prompt_handoff_preserves_known_times_and_accepts_a_legacy_recovery_time() {
        for known in [false, true] {
            let d = tempfile::tempdir().unwrap();
            let dir = notes_dir(d.path());
            let added = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1);
            let original = Left { text: "Make it blue".into(), shown: "Make it blue".into(), pictures: Vec::new(), added: known.then_some(added) };
            hand_over(&dir, 71, &original.text, Some(&original)).unwrap();
            let bytes = std::fs::read(pending(&dir, 71)).unwrap();
            let mut recovered = original.clone();
            recovered.added = Some(added + std::time::Duration::from_secs(1));
            if known {
                assert!(carry_for_prompt(&dir, 71, &recovered, || Ok(())).is_err(), "a known request time cannot change during delivery");
                assert_eq!(std::fs::read(pending(&dir, 71)).unwrap(), bytes);
                recovered.added = original.added;
            }

            assert!(carry_for_prompt(&dir, 71, &recovered, || Ok(())).unwrap());

            assert!(!pending(&dir, 71).exists());
            assert_eq!(std::fs::read(read(&dir, 71)).unwrap(), bytes, "the persistent time stays exact, including a legacy absent time");
            assert_eq!(take_left(&dir).unwrap()[0].left, original);
        }
    }

    #[test]
    fn an_interrupted_takeover_keeps_both_copies_and_restarts_with_the_same_id() {
        let d = tempfile::tempdir().unwrap();
        let dir = notes_dir(d.path());
        let raw = Left { text: "Keep the title blue".into(), shown: "Keep the title blue".into(), pictures: Vec::new(), added: None };
        hand_over(&dir, 71, "Old mode context and the blue title", Some(&raw)).unwrap();
        assert_eq!(claim(&dir), ["Old mode context and the blue title"]);
        let original = std::fs::read(read(&dir, 71)).unwrap();
        let batch = take_left(&dir).unwrap();
        let error =
            restore_using(&dir, &batch[0], "Only current mode context", false, || Err(ToolError::invalid("Stop after the new copy is durable."))).unwrap_err();
        assert!(error.message.contains("new copy is durable"));
        assert_eq!(std::fs::read(read(&dir, 71)).unwrap(), original, "the old record is not discarded before the handoff completes");
        assert!(pending(&dir, 71).is_file());
        drop(batch);

        let again = take_left(&dir).unwrap();
        assert_eq!(again.len(), 1, "two durable copies describe one note, not two requests");
        assert_eq!((again[0].id, &again[0].left), (71, &raw));
        restore(&dir, &again[0], "New process current mode context", false).unwrap();
        assert!(!was_read(&dir, 71), "the new worker cannot mistake an old claim for a fresh delivery");
        assert_eq!(claim(&dir), ["New process current mode context"]);
        assert!(claim(&dir).is_empty());
    }

    #[test]
    fn carried_notes_stay_recoverable_without_being_claimed_twice() {
        for legacy in [false, true] {
            let d = tempfile::tempdir().unwrap();
            let dir = notes_dir(d.path());
            let raw = Left { text: "Keep the title blue".into(), shown: "Keep the title blue".into(), pictures: Vec::new(), added: None };
            hand_over(&dir, 72, "Old mode context", Some(&raw)).unwrap();
            if legacy {
                let mut old: Value = serde_json::from_slice(&std::fs::read(pending(&dir, 72)).unwrap()).unwrap();
                old.as_object_mut().unwrap().remove("version");
                std::fs::write(pending(&dir, 72), serde_json::to_vec(&old).unwrap()).unwrap();
            }
            let batch = take_left(&dir).unwrap();
            restore(&dir, &batch[0], "This note is in the initial message", true).unwrap();
            assert!(claim(&dir).is_empty(), "the initial message already carries this id");
            assert!(read(&dir, 72).is_file());
            drop(batch);
            let after_failed_start = take_left(&dir).unwrap();
            assert_eq!(after_failed_start.len(), 1);
            assert_eq!(after_failed_start[0].left, raw);
            restore(&dir, &after_failed_start[0], &raw.text, false).unwrap();
            assert_eq!(claim(&dir), [raw.text]);
        }
    }

    #[test]
    fn takeover_refuses_a_changed_or_newer_record_without_overwriting_it() {
        let d = tempfile::tempdir().unwrap();
        let dir = notes_dir(d.path());
        let raw = Left { text: "Keep the title blue".into(), shown: "Keep the title blue".into(), pictures: Vec::new(), added: None };
        hand_over(&dir, 73, &raw.text, Some(&raw)).unwrap();
        let batch = take_left(&dir).unwrap();
        let future = br#"{"version":99,"id":73,"text":"Newer private record"}"#;
        std::fs::write(pending(&dir, 73), future).unwrap();
        assert!(restore(&dir, &batch[0], "Replacement context", false).is_err());
        assert_eq!(std::fs::read(pending(&dir, 73)).unwrap(), future);
        assert!(take_left(&dir).is_err());
        assert_eq!(std::fs::read(pending(&dir, 73)).unwrap(), future);
    }

    #[test]
    fn a_fresh_counter_reserves_unknown_waiting_claimed_and_completed_names() {
        let d = tempfile::tempdir().unwrap();
        let dir = notes_dir(d.path());
        std::fs::create_dir(&dir).unwrap();
        for name in ["1.json", "2.read.json", "73.recorded.json"] {
            std::fs::write(dir.join(name), b"Unknown contents must not be overwritten").unwrap();
        }
        let mut fresh = Ids { next: 1 };
        fresh.reserve(&occupied_ids(&dir).unwrap()).unwrap();
        assert_eq!(fresh.allocate().unwrap(), 74, "a new process keeps the order after every occupied state, even unrecognized bytes");
        std::fs::write(dir.join("18446744073709551615.json"), b"Keep this extreme id").unwrap();
        assert!(fresh.reserve(&occupied_ids(&dir).unwrap()).is_err());
        fresh.next = u64::MAX;
        assert!(fresh.allocate().is_err(), "id exhaustion cannot wrap into an earlier request");
        for name in ["1.json", "2.read.json", "73.recorded.json"] {
            assert_eq!(std::fs::read(dir.join(name)).unwrap(), b"Unknown contents must not be overwritten");
        }
        assert_eq!(std::fs::read(dir.join("18446744073709551615.json")).unwrap(), b"Keep this extreme id");
    }

    #[test]
    fn claimed_notes_wait_for_recording_and_completed_notes_do_not_replay() {
        let directory = tempfile::tempdir().unwrap();
        let dir = notes_dir(directory.path());
        let note = Left { text: "Translate into German".into(), shown: "Translate into German".into(), pictures: Vec::new(), added: None };
        hand_over(&dir, 7, &note.text, Some(&note)).unwrap();
        assert_eq!(claim(&dir), std::slice::from_ref(&note.text));
        assert_eq!(left_words(&dir).unwrap(), std::slice::from_ref(&note), "handed to the builder is not yet saved request state");

        mark_recorded(&dir, 7).unwrap();

        assert!(!read(&dir, 7).exists());
        assert!(recorded(&dir, 7).is_file());
        assert!(was_read(&dir, 7), "an in-memory retry still knows that the builder took it");
        assert!(left_words(&dir).unwrap().is_empty(), "keeping the completed file after cleanup failure cannot replay its request");
        assert!(take_left(&dir).unwrap().is_empty());
    }

    #[test]
    fn completion_removes_the_waiting_link_after_a_failed_claim_sync() {
        let directory = tempfile::tempdir().unwrap();
        let dir = notes_dir(directory.path());
        let note = Left { text: "Translate into German".into(), shown: "Translate into German".into(), pictures: Vec::new(), added: None };
        hand_over(&dir, 7, &note.text, Some(&note)).unwrap();
        assert_eq!(claim_using(&dir, |_| Err(ToolError::internal("injected claim sync failure"))), std::slice::from_ref(&note.text));
        assert!(pending(&dir, 7).is_file() && read(&dir, 7).is_file());
        assert!(claim(&dir).is_empty(), "the read link still owns delivery");
        assert_eq!(left_words(&dir).unwrap(), std::slice::from_ref(&note));

        mark_recorded(&dir, 7).unwrap();

        assert!(!pending(&dir, 7).exists() && !read(&dir, 7).exists());
        assert!(recorded(&dir, 7).is_file());
        assert!(take_left(&dir).unwrap().is_empty(), "restart accepts completion even if cleanup never runs");
        assert!(claim(&dir).is_empty());
    }

    #[test]
    fn interrupted_completion_keeps_a_claim_recoverable_until_its_marker_is_published() {
        for fail_at in 1..=3 {
            let directory = tempfile::tempdir().unwrap();
            let dir = notes_dir(directory.path());
            let note = Left { text: "Translate into German".into(), shown: "Translate into German".into(), pictures: Vec::new(), added: None };
            hand_over(&dir, 7, &note.text, Some(&note)).unwrap();
            assert_eq!(claim_using(&dir, |_| Err(ToolError::internal("injected claim sync failure"))), std::slice::from_ref(&note.text));
            let calls = std::cell::Cell::new(0);

            let result = mark_recorded_using(
                &dir,
                7,
                |path| {
                    calls.set(calls.get() + 1);
                    if calls.get() == fail_at { Err(ToolError::internal("injected completion sync failure")) } else { sync_directory(path) }
                },
                || {},
            );

            assert!(result.is_err());
            assert_eq!(left_words(&dir).unwrap(), if fail_at < 3 { vec![note] } else { Vec::new() });
            assert!(claim(&dir).is_empty(), "an interrupted completion must not permit a second live delivery");
            mark_recorded(&dir, 7).unwrap();
            assert!(take_left(&dir).unwrap().is_empty(), "completion can be retried without leaving colliding states");
        }
    }

    #[test]
    fn completion_preserves_a_replaced_waiting_record() {
        let directory = tempfile::tempdir().unwrap();
        let dir = notes_dir(directory.path());
        hand_over(&dir, 7, "original request", None).unwrap();
        assert_eq!(claim_using(&dir, |_| Err(ToolError::internal("injected claim sync failure"))), ["original request"]);
        let original = std::fs::read(read(&dir, 7)).unwrap();
        std::fs::remove_file(pending(&dir, 7)).unwrap();
        hand_over(&dir, 7, "changed request", None).unwrap();
        let changed = std::fs::read(pending(&dir, 7)).unwrap();

        assert!(mark_recorded(&dir, 7).is_err());

        assert_eq!(std::fs::read(read(&dir, 7)).unwrap(), original);
        assert_eq!(std::fs::read(pending(&dir, 7)).unwrap(), changed);
        assert!(!recorded(&dir, 7).exists());
    }

    #[test]
    fn forgetting_reports_removal_failures_and_keeps_completion_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let dir = notes_dir(directory.path());
        hand_over(&dir, 7, "recorded request", None).unwrap();
        assert_eq!(claim(&dir), ["recorded request"]);
        mark_recorded(&dir, 7).unwrap();
        let completed = std::fs::read(recorded(&dir, 7)).unwrap();
        std::fs::create_dir(pending(&dir, 7)).unwrap();

        assert!(forget(&dir, 7).is_err());

        assert_eq!(std::fs::read(recorded(&dir, 7)).unwrap(), completed);
        std::fs::remove_dir(pending(&dir, 7)).unwrap();
        assert!(take_left(&dir).unwrap().is_empty());
        forget(&dir, 7).unwrap();
        assert!(!was_read(&dir, 7));
        assert!(take_left(&dir).unwrap().is_empty());
        forget(&dir, 7).unwrap();
    }

    #[test]
    fn failed_recording_cleanup_preserves_all_notes_for_inspection() {
        let directory = tempfile::tempdir().unwrap();
        let dir = notes_dir(directory.path());
        let note = Left { text: "Translate into German".into(), shown: "Translate into German".into(), pictures: Vec::new(), added: None };
        for id in [1, 2] {
            hand_over(&dir, id, &note.text, Some(&note)).unwrap();
        }
        assert_eq!(claim(&dir).len(), 2);
        mark_recorded(&dir, 1).unwrap();
        let original = std::fs::read(read(&dir, 2)).unwrap();
        std::fs::create_dir(recorded(&dir, 2)).unwrap();
        std::fs::write(recorded(&dir, 2).join("keep.txt"), b"Keep this unexpected entry").unwrap();

        assert!(mark_recorded(&dir, 2).is_err());
        assert_eq!(std::fs::read(read(&dir, 2)).unwrap(), original);
        assert!(left_words(&dir).is_err(), "an unreadable completed entry prevents clearing the notes directory");
        assert!(take_left(&dir).is_err());

        let aside = dir.clone();
        assert!(aside.is_dir(), "failed recovery preserves the records at their original paths");
        assert!(recorded(&aside, 1).is_file());
        assert_eq!(std::fs::read(read(&aside, 2)).unwrap(), original);
        assert_eq!(std::fs::read(recorded(&aside, 2).join("keep.txt")).unwrap(), b"Keep this unexpected entry");
    }

    #[test]
    fn newer_notes_are_not_claimed_or_cleared() {
        let directory = tempfile::tempdir().unwrap();
        let dir = notes_dir(directory.path());
        std::fs::create_dir(&dir).unwrap();
        let future = br#"{"version":99,"id":1,"text":"Keep this future note","left":{"text":"Keep this future note","shown":"Keep it","pictures":[]}}"#;
        std::fs::write(pending(&dir, 1), future).unwrap();

        assert!(claim(&dir).is_empty());
        assert_eq!(std::fs::read(pending(&dir, 1)).unwrap(), future);
        assert!(take_left(&dir).is_err());

        let aside = dir.clone();
        assert_eq!(std::fs::read(pending(&aside, 1)).unwrap(), future);
    }

    #[test]
    fn a_claimed_note_with_changed_identity_is_preserved_on_restart() {
        let directory = tempfile::tempdir().unwrap();
        let dir = notes_dir(directory.path());
        let note = Left { text: "Translate into German".into(), shown: "Translate into German".into(), pictures: Vec::new(), added: None };
        hand_over(&dir, 71, &note.text, Some(&note)).unwrap();
        assert_eq!(claim(&dir).len(), 1);
        let mut changed: Value = serde_json::from_slice(&std::fs::read(read(&dir, 71)).unwrap()).unwrap();
        changed["id"] = Value::from(72);
        let changed = serde_json::to_vec(&changed).unwrap();
        std::fs::write(read(&dir, 71), &changed).unwrap();

        assert!(mark_recorded(&dir, 71).is_err());
        assert!(left_words(&dir).is_err());
        assert!(take_left(&dir).is_err());

        let aside = dir.clone();
        assert!(aside.is_dir(), "failed recovery preserves the records at their original paths");
        assert_eq!(std::fs::read(read(&aside, 71)).unwrap(), changed);
    }

    #[test]
    fn the_hook_hands_the_notes_to_the_model_as_context() {
        assert!(hook_output(&[]).is_none(), "nothing printed when nothing waits");
        let out: serde_json::Value = serde_json::from_str(&hook_output(&["make it blue".into()]).unwrap()).unwrap();
        assert_eq!(out["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        let context = out["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert!(context.contains("- make it blue") && context.contains("applies to the work from here on"), "{context}");
    }
}
