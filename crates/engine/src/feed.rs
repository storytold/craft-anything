//! A project's conversation, as data: what the person asked, what the builder did and said, its
//! checklist, questions and checks. It's kept in the project folder (`feed.json`); a rebuild run
//! appends to it as it goes (`ai::rebuild::feed`), and every client reads the same file.

use crate::inbox::{Inbox, Request};
use crate::workspace::{CheckResult, Status, Step, Workspace};
use craft_anything_evidence::{ErrorCode, ToolError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::io::Write as _;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Where the conversation is kept in the project folder.
pub const FEED_FILE: &str = "feed.json";

/// The format written today (5 marks open plan steps stopped at a run's end; 4 saves a dismissed
/// plan card with it; 3 saved a prompt's pictures); files without a version are the earlier bare
/// list. An older Craft Anything won't rewrite a file it can't read (`save` refuses a newer version).
pub const FEED_VERSION: u32 = 5;

/// What the person sent: as the feed shows it, and the pictures attached to it (their copies in
/// the project's `attachments/`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UserItem {
    /// No pictures: saved as a plain string, as every earlier feed was.
    Text(String),
    WithPictures {
        text: String,
        pictures: Vec<PathBuf>,
    },
}

impl UserItem {
    pub fn new(text: String, pictures: Vec<PathBuf>) -> Self {
        if pictures.is_empty() { UserItem::Text(text) } else { UserItem::WithPictures { text, pictures } }
    }

    pub fn text(&self) -> &str {
        match self {
            UserItem::Text(text) | UserItem::WithPictures { text, .. } => text,
        }
    }

    /// The copies in the project's `attachments/`, in the order they were attached.
    pub fn pictures(&self) -> &[PathBuf] {
        match self {
            UserItem::Text(_) => &[],
            UserItem::WithPictures { pictures, .. } => pictures,
        }
    }
}

impl From<String> for UserItem {
    fn from(text: String) -> Self {
        UserItem::Text(text)
    }
}

/// One step of the builder's own checklist (from its TodoWrite list).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanStep {
    pub text: String,
    /// What it says while doing this step ("Drawing the map").
    pub doing: String,
    pub status: PlanStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanStatus {
    Todo,
    Doing,
    Done,
    /// A run ended with the step still open: stopped, not done (#504).
    Stopped,
}

/// One entry in a project's activity feed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FeedItem {
    /// The builder's checklist, kept up to date in place (shown pinned above the prompt box).
    Plan(Vec<PlanStep>),
    User(UserItem),
    Step {
        step: Step,
        text: String,
    },
    /// The agent's words (streamed).
    Agent {
        text: String,
        streaming: bool,
    },
    /// A run of tool calls, collapsed in Simple mode: (tool, finished ok?).
    Work {
        tools: Vec<(String, Option<bool>)>,
        notes: Vec<String>,
    },
    /// A question or approval; `answer` is the person's reply once given.
    Ask {
        request: Request,
        answer: Option<String>,
    },
    Check(CheckResult),
    Error {
        message: String,
        remediation: Option<String>,
        /// A log with everything known about the failure, to open and inspect.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        log: Option<std::path::PathBuf>,
    },
    /// The person paused the run (not a failure); a message or Continue carries on.
    Paused,
    /// The person folded the plan card above the prompt box (#504). It marks the plan before it as
    /// hidden; the steps are kept, and a later run's checklist shows again.
    PlanDismissed,
}

/// The plan card to show: the checklist, unless the person folded it after it was last updated.
pub fn visible_plan(feed: &[FeedItem]) -> Option<&[PlanStep]> {
    let at = feed.iter().rposition(|i| matches!(i, FeedItem::Plan(_)))?;
    if feed[at + 1..].iter().any(|i| matches!(i, FeedItem::PlanDismissed)) {
        return None;
    }
    match &feed[at] {
        FeedItem::Plan(steps) => Some(steps.as_slice()),
        _ => None,
    }
}

/// Fold the plan card in a feed. Returns false when there's no plan to fold or it's folded already.
pub fn dismiss_plan(feed: &mut Vec<FeedItem>) -> bool {
    if visible_plan(feed).is_none() {
        return false;
    }
    feed.push(FeedItem::PlanDismissed);
    true
}

/// Fold the plan card in a project's saved conversation (the UI's write, while no run is going).
pub fn dismiss_plan_saved(ws: &Workspace) -> Result<bool, ToolError> {
    let mut feed = load_for_write(ws);
    let folded = dismiss_plan(&mut feed);
    if folded {
        save(&ws.root, &mut feed)?;
    }
    Ok(folded)
}

/// Mark a question or approval answered, with the words shown for the answer.
pub fn answered(feed: &mut [FeedItem], id: &str, shown: &str) {
    for item in feed {
        if let FeedItem::Ask { request, answer: a @ None } = item
            && request.id == id
        {
            *a = Some(shown.to_string());
        }
    }
}

/// The saved file: `{version, items}`. Earlier files were the bare list.
#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    items: Vec<FeedItem>,
}

/// A writer retains the result of its original read, so a fallback cannot overwrite a file
/// that becomes readable later. Read-only callers use `load`.
#[derive(Debug, Clone)]
pub struct LoadedFeed {
    items: Vec<FeedItem>,
    not_loaded: Option<ToolError>,
    damaged: Option<Vec<u8>>,
}

impl Deref for LoadedFeed {
    type Target = Vec<FeedItem>;

    fn deref(&self) -> &Self::Target {
        &self.items
    }
}

impl DerefMut for LoadedFeed {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.items
    }
}

const MAX_FEED_BYTES: u64 = 64 * 1024 * 1024;
const DAMAGED_NOTICE: &str = "This project's saved conversation is damaged. Showing the history kept in the project.";
const BACKUP_DIR: &str = "feed-recovery";
static BACKUP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Absence must be established separately: a dangling symlink is still an existing entry.
fn read_previous(path: &Path) -> Result<Option<Vec<u8>>, ToolError> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(ToolError::io(path.display(), e)),
        Ok(_) => {}
    }
    craft_anything_platform::fs::read_file_capped(path, MAX_FEED_BYTES).map(Some).map_err(|error| {
        if error.code == ErrorCode::OverLimit {
            error.with_remediation("The original is unchanged. Keep a copy before restoring a smaller conversation, then reopen the project.")
        } else {
            error
        }
    })
}

/// Parse the bytes once. Inspect the version before converting the already parsed value into
/// items, including when a future version's items are no longer readable by this build.
fn decode(bytes: &[u8]) -> Result<Option<Vec<FeedItem>>, ToolError> {
    let Ok(mut value) = serde_json::from_slice::<Value>(bytes) else { return Ok(None) };
    if value.is_array() {
        return Ok(serde_json::from_value(value).ok());
    }
    let Some(version) = value.get("version").and_then(Value::as_u64) else { return Ok(None) };
    if version > u64::from(FEED_VERSION) {
        return Err(ToolError::unsupported("This project's conversation was saved by a newer Craft Anything, so it isn't changed.")
            .with_remediation("Update Craft Anything and reopen the project to keep its conversation up to date."));
    }
    Ok(serde_json::from_value(value["items"].take()).ok())
}

/// The feed for a project: what was saved, else rebuilt from its manifest. Questions still
/// waiting in its inbox are always there.
pub fn load(ws: &Workspace) -> Vec<FeedItem> {
    load_for_write(ws).items
}

/// Load a conversation for later edits. A read failure stays read-only until the project is
/// loaded again. Readable damaged bytes are retained for the required backup before saving.
pub fn load_for_write(ws: &Workspace) -> LoadedFeed {
    let loaded = read_previous(&ws.root.join(FEED_FILE)).and_then(|previous| match previous {
        None => Ok((history(ws), None)),
        Some(bytes) => match decode(&bytes)? {
            Some(items) => Ok((items, None)),
            None => {
                let mut items = history(ws);
                items.insert(
                    0,
                    FeedItem::Error {
                        message: DAMAGED_NOTICE.into(),
                        remediation: Some("The original is unchanged. A complete copy will be kept before saving a new conversation.".into()),
                        log: None,
                    },
                );
                Ok((items, Some(bytes)))
            }
        },
    });
    let (mut feed, damaged) = match loaded {
        Ok(loaded) => loaded,
        Err(error) => {
            let mut items = history(ws);
            items.insert(
                0,
                FeedItem::Error {
                    message: format!("This project's conversation couldn't be loaded: {}", error.message),
                    remediation: Some(
                        error
                            .remediation
                            .clone()
                            .unwrap_or_else(|| "The original is unchanged. Make it readable, then reopen the project before saving.".into()),
                    ),
                    log: None,
                },
            );
            return LoadedFeed { items, not_loaded: Some(error), damaged: None };
        }
    };
    // Nothing is streaming any more after a restart.
    for item in &mut feed {
        if let FeedItem::Agent { streaming, .. } = item {
            *streaming = false;
        }
    }
    let shown: HashSet<String> = feed.iter().filter_map(|i| if let FeedItem::Ask { request, .. } = i { Some(request.id.clone()) } else { None }).collect();
    feed.extend(Inbox::new(ws.inbox_dir()).pending().into_iter().filter(|r| !shown.contains(&r.id)).map(|request| FeedItem::Ask { request, answer: None }));
    LoadedFeed { items: feed, not_loaded: None, damaged }
}

/// A feed rebuilt from what a project saved: the request, the checks, the agent's last words,
/// and anything still waiting for an answer.
pub fn history(ws: &Workspace) -> Vec<FeedItem> {
    let m = &ws.manifest;
    let mut feed = vec![FeedItem::User(UserItem::new(m.request.clone(), Vec::new()))];
    feed.extend(m.checks.iter().cloned().map(FeedItem::Check));
    if !m.last_reply.is_empty() {
        feed.push(FeedItem::Agent { text: m.last_reply.clone(), streaming: false });
    }
    if matches!(m.status, Status::NeedsYou) && !m.note.is_empty() {
        feed.push(FeedItem::Error { message: m.note.clone(), remediation: None, log: None });
    }
    feed.extend(Inbox::new(ws.inbox_dir()).pending().into_iter().map(|request| FeedItem::Ask { request, answer: None }));
    feed
}

/// Write the feed into the project folder (only one that exists; never creates one). A feed
/// saved by a newer Craft Anything is never written over (docs/file-formats.md).
pub fn save(root: &Path, feed: &mut LoadedFeed) -> Result<(), ToolError> {
    if let Some(error) = &feed.not_loaded {
        return Err(error.clone());
    }
    if !root.is_dir() {
        return Err(ToolError::new(ErrorCode::FileError, "The project's folder isn't available, so its conversation wasn't saved."));
    }
    let path = root.join(FEED_FILE);
    let previous = read_previous(&path)?;
    if let Some(original) = &feed.damaged
        && previous.as_deref() != Some(original.as_slice())
    {
        return Err(ToolError::new(ErrorCode::FileChanged, "The damaged conversation changed before it could be saved.")
            .with_remediation("Reopen the project to read the current conversation. Nothing was written."));
    }
    let mut items = feed.items.clone();
    if let Some(bytes) = previous
        && decode(&bytes)?.is_none()
    {
        let backup = keep_damaged(root, &bytes)?;
        let notice =
            FeedItem::Error { message: DAMAGED_NOTICE.into(), remediation: Some(format!("The complete original is kept at {}.", backup.display())), log: None };
        if let Some(item) = items.iter_mut().find(|item| matches!(item, FeedItem::Error { message, .. } if message == DAMAGED_NOTICE)) {
            *item = notice;
        } else {
            items.insert(0, notice);
        }
    }
    let saved = Saved { version: FEED_VERSION, items };
    let json = serde_json::to_vec(&saved).map_err(|e| ToolError::internal(e.to_string()))?;
    if json.len() as u64 > MAX_FEED_BYTES {
        return Err(ToolError::new(ErrorCode::OverLimit, "The conversation is too large to save safely. Its saved file is unchanged."));
    }
    crate::project::write_atomic(&path, &json)?;
    feed.items = saved.items;
    feed.damaged = None;
    Ok(())
}

/// Keep one new, private copy. These are personal recovery files: never overwrite or delete an
/// existing entry, including a partial copy left by a failed write.
fn keep_damaged(root: &Path, bytes: &[u8]) -> Result<PathBuf, ToolError> {
    let dir = root.join(BACKUP_DIR);
    match std::fs::create_dir(&dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if !std::fs::symlink_metadata(&dir).map_err(|e| ToolError::io(dir.display(), e))?.is_dir() {
                return Err(ToolError::invalid(format!("{} is not a recovery folder", dir.display())));
            }
        }
        Err(error) => return Err(ToolError::io(dir.display(), error)),
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    for _ in 0..64 {
        let id = BACKUP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("feed-{}-{id}.json", std::process::id()));
        let mut file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(ToolError::io(path.display(), error)),
        };
        file.write_all(bytes).and_then(|()| file.sync_all()).map_err(|e| {
            ToolError::io(path.display(), e).with_remediation("The original conversation is unchanged. The incomplete copy is kept at the reported path.")
        })?;
        #[cfg(unix)]
        for parent in [&dir, root] {
            std::fs::File::open(parent).and_then(|file| file.sync_all()).map_err(|e| ToolError::io(parent.display(), e))?;
        }
        return Ok(path);
    }
    Err(ToolError::new(ErrorCode::FileError, "Couldn't reserve a new conversation recovery copy. The original is unchanged."))
}

/// Record an answer given while no run is going (straight into the saved feed). `request` is the
/// question, taken before it was answered: one the saved conversation doesn't have yet (posted
/// from outside a run) is added with its answer, rather than lost once it stops waiting (#176).
pub fn record_answer(ws: &Workspace, request: &Request, shown: &str) -> Result<(), ToolError> {
    let mut feed = load_for_write(ws);
    if !feed.iter().any(|i| matches!(i, FeedItem::Ask { request: r, .. } if r.id == request.id)) {
        feed.push(FeedItem::Ask { request: request.clone(), answer: None });
    }
    answered(&mut feed, &request.id, shown);
    save(&ws.root, &mut feed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbox::RequestKind;
    use crate::workspace::TargetRef;

    // Existing format fixtures seed a writable value directly. Production writers retain load state.
    fn save(root: &Path, items: &[FeedItem]) -> Result<(), ToolError> {
        super::save(root, &mut LoadedFeed { items: items.to_vec(), not_loaded: None, damaged: None })
    }

    #[test]
    fn an_unreadable_feed_stays_read_only_after_it_is_repaired() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "Fallback", TargetRef::None).unwrap();
        let path = ws.root.join(FEED_FILE);
        std::fs::create_dir(&path).unwrap();
        let mut loaded = load_for_write(&ws);
        std::fs::remove_dir(&path).unwrap();
        let repaired = br#"{"version":5,"items":[{"User":"Keep this conversation"}]}"#;
        std::fs::write(&path, repaired).unwrap();
        loaded.push(FeedItem::Paused);
        assert!(super::save(&ws.root, &mut loaded).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), repaired);
    }

    #[test]
    fn damaged_bytes_are_kept_and_the_copy_is_reported_before_replacement() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "Fallback", TargetRef::None).unwrap();
        let path = ws.root.join(FEED_FILE);
        let original = b"{\"version\":5,\"items\":[\xff\x00";
        std::fs::write(&path, original).unwrap();
        let dir = ws.root.join(BACKUP_DIR);
        std::fs::create_dir(&dir).unwrap();
        let personal = dir.join("personal-notes.txt");
        std::fs::write(&personal, "Keep my notes").unwrap();
        let mut loaded = load_for_write(&ws);
        loaded.push(FeedItem::Paused);
        super::save(&ws.root, &mut loaded).unwrap();
        let backup = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).find(|p| p != &personal).unwrap();
        assert_eq!(std::fs::read(&backup).unwrap(), original);
        assert_eq!(std::fs::read_to_string(&personal).unwrap(), "Keep my notes");
        assert!(
            load(&ws).iter().any(|item| matches!(item, FeedItem::Error { remediation: Some(message), .. } if message.contains(&backup.display().to_string())))
        );
        super::save(&ws.root, &mut loaded).unwrap();
        assert_eq!(std::fs::read_dir(dir).unwrap().count(), 2, "ordinary saves do not make more backups");
    }

    #[test]
    fn a_failed_backup_does_not_replace_the_source_or_claim_success() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "Fallback", TargetRef::None).unwrap();
        let path = ws.root.join(FEED_FILE);
        let original = b"broken bytes";
        std::fs::write(&path, original).unwrap();
        let blocked = ws.root.join(BACKUP_DIR);
        std::fs::write(&blocked, "An existing personal file").unwrap();
        let mut loaded = load_for_write(&ws);
        let before = loaded.items.clone();
        assert!(super::save(&ws.root, &mut loaded).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(loaded.items, before);
        assert_eq!(std::fs::read_to_string(&blocked).unwrap(), "An existing personal file");
        std::fs::remove_file(blocked).unwrap();
        super::save(&ws.root, &mut loaded).unwrap();
        assert_ne!(std::fs::read(path).unwrap(), original);
    }

    #[test]
    fn a_damaged_feed_replaced_elsewhere_must_be_loaded_again() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "Fallback", TargetRef::None).unwrap();
        let path = ws.root.join(FEED_FILE);
        std::fs::write(&path, "damaged").unwrap();
        let mut loaded = load_for_write(&ws);
        let repaired = br#"[{"User":"Recovered elsewhere"}]"#;
        std::fs::write(&path, repaired).unwrap();
        assert_eq!(super::save(&ws.root, &mut loaded).unwrap_err().code, ErrorCode::FileChanged);
        assert_eq!(std::fs::read(path).unwrap(), repaired);
        assert!(!ws.root.join(BACKUP_DIR).exists());
    }

    #[test]
    fn oversized_and_newer_feeds_cannot_be_replaced_by_fallback_history() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "Fallback", TargetRef::None).unwrap();
        let path = ws.root.join(FEED_FILE);
        std::fs::File::create(&path).unwrap().set_len(MAX_FEED_BYTES + 1).unwrap();
        let mut loaded = load_for_write(&ws);
        assert_eq!(super::save(&ws.root, &mut loaded).unwrap_err().code, ErrorCode::OverLimit);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), MAX_FEED_BYTES + 1);
        let future = br#"{"version":999,"items":{"future":"shape"}}"#;
        std::fs::write(&path, future).unwrap();
        let mut loaded = load_for_write(&ws);
        assert_eq!(super::save(&ws.root, &mut loaded).unwrap_err().code, ErrorCode::NotSupported);
        assert_eq!(std::fs::read(path).unwrap(), future);
        assert!(!ws.root.join(BACKUP_DIR).exists());
    }

    #[test]
    fn file_formats_doc_lists_current_manifest_and_feed_versions() {
        let doc = include_str!("../../../docs/file-formats.md");
        let manifest = format!("`MANIFEST_VERSION` = {} ", crate::workspace::MANIFEST_VERSION);
        let feed = format!("`FEED_VERSION` = {} ", FEED_VERSION);
        assert!(doc.contains(&manifest), "docs/file-formats.md must list {manifest}");
        assert!(doc.contains(&feed), "docs/file-formats.md must list {feed}");
    }

    #[test]
    fn history_rebuilds_from_the_manifest() {
        let d = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(d.path(), "x", "Clone it", TargetRef::None).unwrap();
        ws.manifest.checks.push(CheckResult { similarity: Some(70.0), ..CheckResult::new(CheckResult::WEB_VISUAL, 1, true, "") });
        ws.manifest.last_reply = "Built a page.".into();
        ws.save().unwrap();
        Inbox::new(ws.inbox_dir()).post(RequestKind::Question, "Blue or red?", "", vec![], true, None).unwrap();
        let f = history(&ws);
        assert!(matches!(&f[0], FeedItem::User(r) if r.text() == "Clone it"));
        assert!(matches!(f.last(), Some(FeedItem::Ask { answer: None, .. })), "pending questions come back");
        assert_eq!(f.len(), 4);
    }

    #[test]
    fn a_prompt_keeps_its_pictures_with_it() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "Clone it", TargetRef::None).unwrap();
        let pictures = vec![ws.root.join("attachments/picture-1.png"), ws.root.join("attachments/picture-2.webp")];
        save(&ws.root, &[FeedItem::User(UserItem::new("Make it blue".into(), pictures.clone()))]).unwrap();
        let loaded = load(&ws);
        assert!(matches!(&loaded[0], FeedItem::User(u) if u.text() == "Make it blue" && u.pictures() == pictures.as_slice()));
        // A prompt with no pictures is still saved as the plain string older feeds hold.
        save(&ws.root, &[FeedItem::User(UserItem::new("Add tags".into(), Vec::new()))]).unwrap();
        let raw: Value = serde_json::from_slice(&std::fs::read(ws.root.join(FEED_FILE)).unwrap()).unwrap();
        assert_eq!(raw["items"][0]["User"], "Add tags");
        assert!(raw["items"][0]["User"].is_string());
    }

    /// #504: a plan card can be folded, the steps stay in the conversation, and a folded plan stays
    /// folded after a restart. A file from before folding (version 3) still shows its plan.
    #[test]
    fn a_folded_plan_stays_folded_and_its_steps_are_kept() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "A game", TargetRef::None).unwrap();
        let steps = vec![PlanStep { text: "Lay out the arena".into(), doing: "Laying out the arena".into(), status: PlanStatus::Doing }];
        save(&ws.root, &[FeedItem::User(UserItem::Text("A game".into())), FeedItem::Plan(steps.clone())]).unwrap();
        assert!(visible_plan(&load(&ws)).is_some(), "the plan shows before it's folded");
        assert!(dismiss_plan_saved(&ws).unwrap(), "the plan folds");
        let folded = load(&ws);
        assert!(visible_plan(&folded).is_none(), "folded after a restart too");
        assert!(folded.iter().any(|i| matches!(i, FeedItem::Plan(p) if p == &steps)), "the steps are kept");
        assert!(!dismiss_plan_saved(&ws).unwrap(), "folding again changes nothing");
        save(&ws.root, &[FeedItem::User(UserItem::Text("Hi".into()))]).unwrap();
        assert!(!dismiss_plan_saved(&ws).unwrap(), "a conversation with no plan has nothing to fold");
        std::fs::write(ws.root.join(FEED_FILE), br#"{"version":3,"items":[{"Plan":[{"text":"Lay out","doing":"Laying out","status":"Doing"}]}]}"#).unwrap();
        assert!(visible_plan(&load(&ws)).is_some(), "a version 3 plan still shows");
    }

    /// #504: a step marked stopped when a run ended stays in the saved conversation (it's part of
    /// the plan card's data, not a display flourish).
    #[test]
    fn a_stopped_step_is_kept_across_saves() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "A game", TargetRef::None).unwrap();
        let steps = vec![
            PlanStep { text: "Lay out the arena".into(), doing: "Laying out the arena".into(), status: PlanStatus::Done },
            PlanStep { text: "Add the snake".into(), doing: "Adding the snake".into(), status: PlanStatus::Stopped },
            PlanStep { text: "Keep score".into(), doing: "Keeping score".into(), status: PlanStatus::Todo },
        ];
        save(&ws.root, &[FeedItem::User(UserItem::Text("A game".into())), FeedItem::Plan(steps)]).unwrap();
        let loaded = load(&ws);
        assert!(matches!(visible_plan(&loaded), Some(p) if p.iter().any(|s| s.status == PlanStatus::Stopped)));
    }

    #[test]
    fn older_files_load_answers_are_kept_and_newer_files_are_left_alone() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "Clone it", TargetRef::None).unwrap();
        save(&ws.root, &[FeedItem::User(UserItem::Text("Clone it".into())), FeedItem::Agent { text: "Done.".into(), streaming: true }]).unwrap();
        assert!(matches!(load(&ws).as_slice(), [FeedItem::User(_), FeedItem::Agent { streaming: false, .. }]), "nothing streams after a restart");
        let raw: Value = serde_json::from_slice(&std::fs::read(ws.root.join(FEED_FILE)).unwrap()).unwrap();
        assert_eq!(raw["version"], FEED_VERSION);
        // A feed saved before versioning (the bare list) still loads.
        std::fs::write(ws.root.join(FEED_FILE), br#"[{"User":"Old request"}]"#).unwrap();
        assert!(matches!(load(&ws).as_slice(), [FeedItem::User(r)] if r.text() == "Old request"));
        // Answers given with no run going are saved too.
        let inbox = Inbox::new(ws.inbox_dir());
        let id = inbox.post(RequestKind::Question, "Blue or red?", "", vec![], true, None).unwrap();
        let question = inbox.pending().into_iter().find(|r| r.id == id).unwrap();
        inbox.answer(&id, &crate::inbox::Answer { choice: Some("Blue".into()), ..Default::default() }).unwrap();
        record_answer(&ws, &question, "Blue").unwrap();
        assert!(std::fs::read_to_string(ws.root.join(FEED_FILE)).unwrap().contains("\"Blue\""));
        // #176: one the saved conversation never had (posted from outside a run, answered before
        // anything saved it) is kept with its answer, once, and still after the inbox forgets it.
        save(&ws.root, &[FeedItem::User(UserItem::Text("Clone it".into()))]).unwrap();
        let id = inbox.post(RequestKind::Question, "Which scope?", "", vec![], true, None).unwrap();
        let question = inbox.pending().into_iter().find(|r| r.id == id).unwrap();
        inbox.answer(&id, &crate::inbox::Answer { choice: Some("Minimal".into()), ..Default::default() }).unwrap();
        let _ = std::fs::remove_file(inbox.dir().join(format!("{id}.request.json")));
        let _ = std::fs::remove_file(inbox.dir().join(format!("{id}.answer.json")));
        record_answer(&ws, &question, "Minimal").unwrap();
        let asks = |f: &[FeedItem]| f.iter().filter(|i| matches!(i, FeedItem::Ask { request, answer: Some(a) } if request.id == id && a == "Minimal")).count();
        assert_eq!(asks(&load(&ws)), 1, "kept with its answer");
        record_answer(&ws, &question, "Minimal").unwrap();
        assert_eq!(asks(&load(&ws)), 1, "not twice");
        // One from a newer Craft Anything is shown as far as we can, never written over.
        let newer = br#"{"version":99,"items":[{"User":"From the future"}],"extra":1}"#;
        std::fs::write(ws.root.join(FEED_FILE), newer).unwrap();
        assert!(save(&ws.root, &[FeedItem::User(UserItem::Text("x".into()))]).is_err());
        assert_eq!(std::fs::read(ws.root.join(FEED_FILE)).unwrap(), newer);
    }
}
