//! A project's conversation, as data: what the person asked, what the builder did and said, its
//! checklist, questions and checks. It's kept in the project folder (`feed.json`); a rebuild run
//! appends to it as it goes (`ai::rebuild::feed`), and every client reads the same file.

use crate::inbox::{DeliveryState, Inbox, Request};
use crate::workspace::{CheckResult, Status, Step, Workspace};
use craft_anything_evidence::{ErrorCode, ToolError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::io::Read as _;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

mod backups;
mod recovery;
mod transaction;
pub(crate) use backups::{
    classify_open_backup_alias_with_cancel, owned_backups, owned_backups_with_cancel, owns_open_backup_with_cancel, remove_owned_backups,
};
pub(crate) use recovery::sync_directory;
pub use recovery::{RECOVERY_ARCHIVE_DIR, RecoverySnapshot, RecoveryWrite, recovery_notices, write_recovery_snapshot, write_recovery_snapshot_report};
#[cfg(test)]
pub(crate) use transaction::lock_creation_using;
pub(crate) use transaction::{
    CreationLockFailure, PROJECT_LOCK_FILE, ProjectCopyGuard, check_entry as check_coordination_entry, lock_creation, lock_inbox_for_cleanup,
    lock_project_for_copy,
};
pub use transaction::{ProjectFeedGuard, lock_project};

/// Where the conversation is kept in the project folder.
pub const FEED_FILE: &str = "feed.json";

/// These names are reserved for recovery during creation and builder writes. Only copies
/// with matching ownership receipts are excluded from duplication or selected for deletion;
/// a legacy personal file or directory with the same name remains the person's content.
pub(crate) const DAMAGED_BACKUP_NAMES: &[&str] = &[
    "feed.json.broken",
    "feed.json.broken.1",
    "feed.json.broken.2",
    "feed.json.broken.3",
    "feed.json.broken.4",
    "feed.json.broken.5",
    "feed.json.broken.6",
    "feed.json.broken.7",
    "feed.json.broken.8",
    "feed.json.broken.9",
    "feed.json.broken.10",
    "feed.json.broken.11",
    "feed.json.broken.12",
    "feed.json.broken.13",
    "feed.json.broken.14",
    "feed.json.broken.15",
];

/// The format written today (6 retains the history projected during recovery; 5 marks open plan
/// steps stopped at a run's end; 4 saves a dismissed
/// plan card with it; 3 saved a prompt's pictures); files without a version are the earlier bare
/// list. An older Craft Anything won't rewrite a file it can't read (`save` refuses a newer version).
pub const FEED_VERSION: u32 = 6;

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
    let mut feed = load(ws);
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

/// The manifest fields that produced a recovery's history prefix. Later manifest updates must
/// not change the identity of that projected occurrence when a live writer replays its events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecoveryProjection {
    source_sha256: String,
    pub request: String,
    pub checks: Vec<CheckResult>,
    pub last_reply: String,
    pub failure_note: Option<String>,
}

impl RecoveryProjection {
    fn from_workspace(ws: &Workspace, damaged: &[u8]) -> Self {
        let m = &ws.manifest;
        Self {
            source_sha256: craft_anything_evidence::sha256_hex(damaged),
            request: m.request.clone(),
            checks: m.checks.clone(),
            last_reply: m.last_reply.clone(),
            failure_note: (m.status == Status::NeedsYou && !m.note.is_empty()).then(|| m.note.clone()),
        }
    }
}

/// The saved file. Version 6 records a recovery's projected history; older files and the
/// original bare list have no such provenance and are read without inventing it.
#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    items: Vec<FeedItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery: Option<RecoveryProjection>,
}

/// A conversation together with its read state and last successfully saved source. A display
/// fallback must keep that fact through edits: repairing the file later doesn't make the
/// fallback safe to write over it. Load it again after fixing a read error.
#[derive(Debug, Clone)]
pub struct LoadedFeed {
    items: Vec<FeedItem>,
    not_loaded: Option<ToolError>,
    damaged: Option<Vec<u8>>,
    decoded_source: Option<String>,
    recovery: Option<RecoveryProjection>,
    project: Result<std::sync::Arc<transaction::ProjectIdentity>, ToolError>,
    display_source: Option<std::sync::Arc<DisplaySource>>,
    save_needs_sync: bool,
}

/// Keep the opened file alive so an atomic replacement cannot reuse its identity. These
/// small metadata checks let a live display reject stale delivery evidence without reading
/// and hashing the complete conversation on each UI poll.
#[derive(Debug)]
struct DisplaySource {
    file: same_file::Handle,
    length: u64,
    modified: Option<std::time::SystemTime>,
}

impl DisplaySource {
    fn read(path: &Path) -> Option<Self> {
        let (file, metadata) = craft_anything_platform::fs::open_regular_file(path).ok()?;
        let file = same_file::Handle::from_file(file).ok()?;
        Some(Self { file, length: metadata.len(), modified: metadata.modified().ok() })
    }

    fn is_current(&self, path: &Path) -> bool {
        Self::read(path).is_some_and(|current| self.file == current.file && self.length == current.length && self.modified == current.modified)
    }
}

/// The project owning a conversation or run, independent of the path currently naming it.
#[derive(Debug, Clone)]
pub struct ProjectIdentity(std::sync::Arc<transaction::ProjectIdentity>);

impl ProjectIdentity {
    /// Capture a workspace only after confirming its manifest still names the current project.
    pub fn from_workspace(ws: &Workspace) -> Result<Self, ToolError> {
        let project = transaction::ProjectIdentity::from_workspace(ws)?;
        project.check(&ws.root)?;
        Ok(Self(std::sync::Arc::new(project)))
    }

    pub fn is_same_project(&self, other: &Self) -> bool {
        self.0.same(&other.0)
    }

    /// Check directory, creation time and generation without reading the conversation.
    pub fn check_current(&self, root: &Path) -> Result<(), ToolError> {
        self.0.check(root)
    }
}

impl LoadedFeed {
    /// Share only the original project identity, not this conversation's items.
    pub fn project_identity(&self) -> Option<ProjectIdentity> {
        self.project.as_ref().ok().cloned().map(ProjectIdentity)
    }

    /// Prepare an owned lock callback for an inbox transaction. It retains only the project
    /// identity, not this conversation's items, and runs after the request reservation. This
    /// lets the caller validate its loaded project before staging any delivery record.
    pub fn prepare_transaction(&self, root: &Path) -> impl FnOnce() -> Result<ProjectFeedGuard, ToolError> + use<> {
        let root = root.to_path_buf();
        let project = self.project.clone();
        move || {
            let guard = lock_project(&root)?;
            project?.check(&root)?;
            Ok(guard)
        }
    }

    /// The most recent save published these bytes but could not sync the containing folder.
    /// Adopt the visible state before retrying, while continuing to report the sync error.
    /// Every save resets this flag before validation or lock acquisition can fail.
    pub fn save_needs_sync(&self) -> bool {
        self.save_needs_sync
    }

    /// A repaired conversation belongs to this writer only while its original project
    /// directory, manifest creation time and persistent generation remain the same.
    pub fn is_same_project(&self, other: &LoadedFeed) -> bool {
        match (&self.project, &other.project) {
            (Ok(own), Ok(other)) => own.same(other),
            _ => false,
        }
    }

    /// Whether loading found no read or decoding error. A confirmed absent file also starts
    /// a new conversation; recovery callers use `was_decoded` to require saved contents.
    pub fn is_readable(&self) -> bool {
        self.not_loaded.is_none() && self.damaged.is_none()
    }

    /// Whether valid saved bytes were decoded or this value was successfully committed.
    /// An absent-file fallback has no such source until its first save succeeds.
    pub fn was_decoded(&self) -> bool {
        self.decoded_source.is_some()
    }

    /// A live writer may try a fresh load after a repair. The original fallback remains
    /// protected: this does not clear the error or make it directly writable.
    pub fn had_read_error(&self) -> bool {
        self.not_loaded.is_some()
    }

    pub fn recovery_projection(&self) -> Option<&RecoveryProjection> {
        self.recovery.as_ref()
    }

    /// Only the damaged bytes this writer actually loaded can identify its recovery. A plain
    /// read failure, or an unrelated old recovery, cannot authorize deleting saved history.
    pub fn was_recovered_from(&self, earlier: &LoadedFeed) -> bool {
        match (&self.recovery, &earlier.damaged) {
            (Some(projection), Some(damaged)) => projection.source_sha256 == craft_anything_evidence::sha256_hex(damaged),
            _ => false,
        }
    }

    /// A writer that has reconciled the projected occurrences keeps its replayed history.
    pub fn clear_recovery_projection(&mut self) {
        self.recovery = None;
    }

    /// Take the items for a read-only view. Writers keep the `LoadedFeed` and pass it to `save`.
    pub fn into_items(self) -> Vec<FeedItem> {
        self.items
    }
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

const DAMAGED_NOTICE: &str = "This project's saved conversation couldn't be read. Showing what the project could recover.";

fn newer_version(bytes: &[u8]) -> bool {
    serde_json::from_slice::<Value>(bytes).ok().and_then(|v| v.get("version").and_then(Value::as_u64)).is_some_and(|v| v > u64::from(FEED_VERSION))
}

fn newer_error() -> ToolError {
    ToolError::new(ErrorCode::NotSupported, "This project's conversation was saved by a newer Craft Anything, so it isn't changed.")
        .with_remediation("Update Craft Anything and reopen the project to keep its conversation up to date.")
}

fn decode(bytes: &[u8]) -> Option<Saved> {
    serde_json::from_slice::<Saved>(bytes)
        .ok()
        .filter(|saved| saved.version <= FEED_VERSION)
        .or_else(|| serde_json::from_slice::<Vec<FeedItem>>(bytes).ok().map(|items| Saved { version: 0, items, recovery: None }))
}

/// Only a path with nothing at it is a new conversation. In particular, a dangling link is
/// still the person's file and must not be replaced with a fallback.
fn read_previous(path: &Path) -> Result<Option<Vec<u8>>, ToolError> {
    match craft_anything_platform::fs::open_regular_file(path) {
        Ok((mut file, _)) => {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).map_err(|error| ToolError::io(path.display(), error))?;
            Ok(Some(bytes))
        }
        Err(read_error) if read_error.code == ErrorCode::NotFound => match std::fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(ToolError::io(path.display(), e)),
            Ok(_) => Err(read_error),
        },
        Err(error) => Err(error),
    }
}

/// The feed for a project: what was saved, else rebuilt from its manifest. Questions still
/// waiting in its inbox are always there.
pub fn load(ws: &Workspace) -> LoadedFeed {
    load_inner(ws)
}

fn load_inner(ws: &Workspace) -> LoadedFeed {
    let project = transaction::ProjectIdentity::from_workspace(ws).map(std::sync::Arc::new);
    let path = ws.root.join(FEED_FILE);
    let display_source = DisplaySource::read(&path);
    let saved = read_previous(&path).and_then(|bytes| match bytes {
        Some(bytes) if newer_version(&bytes) => Err(newer_error()),
        bytes => Ok(bytes),
    });
    let display_source = display_source.filter(|source| source.is_current(&path)).map(std::sync::Arc::new);
    let saved = match saved {
        Ok(None) => {
            return LoadedFeed {
                items: history(ws),
                not_loaded: None,
                damaged: None,
                decoded_source: None,
                recovery: None,
                project,
                display_source,
                save_needs_sync: false,
            };
        }
        Err(error) => {
            let error = if error.remediation.is_some() {
                error
            } else {
                error.with_remediation("The original is unchanged. Make it readable, then reopen the project before saving or answering a question.")
            };
            let mut items = history(ws);
            items.push(FeedItem::Error {
                message: format!("This project's saved conversation couldn't be read: {}", error.message),
                remediation: error.remediation.clone(),
                log: None,
            });
            return LoadedFeed {
                items,
                not_loaded: Some(error),
                damaged: None,
                decoded_source: None,
                recovery: None,
                project,
                display_source,
                save_needs_sync: false,
            };
        }
        Ok(Some(bytes)) => match decode(&bytes) {
            Some(saved) => (saved, craft_anything_evidence::sha256_hex(&bytes)),
            None => {
                let mut items = history(ws);
                items.push(FeedItem::Error {
                    message: DAMAGED_NOTICE.into(),
                    remediation: Some("The original is unchanged. A complete copy will be kept beside feed.json before saving a new conversation.".into()),
                    log: None,
                });
                let recovery = Some(RecoveryProjection::from_workspace(ws, &bytes));
                return LoadedFeed {
                    items,
                    not_loaded: None,
                    damaged: Some(bytes),
                    decoded_source: None,
                    recovery,
                    project,
                    display_source,
                    save_needs_sync: false,
                };
            }
        },
    };
    let (saved, decoded_source) = saved;
    let mut feed = saved.items;
    feed.retain(|item| !recovery::is_recovery_notice(item));
    // Nothing is streaming any more after a restart.
    for item in &mut feed {
        if let FeedItem::Agent { streaming, .. } = item {
            *streaming = false;
        }
    }
    let shown: HashSet<String> = feed.iter().filter_map(|i| if let FeedItem::Ask { request, .. } = i { Some(request.id.clone()) } else { None }).collect();
    feed.extend(Inbox::new(ws.inbox_dir()).pending().into_iter().filter(|r| !shown.contains(&r.id)).map(|request| FeedItem::Ask { request, answer: None }));
    LoadedFeed {
        items: feed,
        not_loaded: None,
        damaged: None,
        decoded_source: Some(decoded_source),
        recovery: saved.recovery,
        project,
        display_source,
        save_needs_sync: false,
    }
}

/// A read-only conversation view, including retryable questions whose answer was saved but
/// couldn't yet be delivered. Writers use `load` so a save between recording and delivery
/// cannot erase the recorded answer.
pub fn for_display(ws: &Workspace) -> Vec<FeedItem> {
    for_display_using(ws, || {})
}

fn for_display_using(ws: &Workspace, after_load: impl FnOnce()) -> Vec<FeedItem> {
    let feed = load(ws);
    let mut items = feed.items.clone();
    after_load();
    show_pending_for_feed(&mut items, &Inbox::new(ws.inbox_dir()), &feed);
    items.extend(recovery_notices(&ws.root));
    items
}

/// Receipt state can change after the conversation snapshot was loaded. It confirms an
/// answered row only while that snapshot's opened source is still current. A changed source
/// leaves the earlier words visible as unconfirmed, rather than assigning a later reply's
/// delivery to them. A subsequent display load picks up the new conversation.
pub fn show_pending_for_feed(items: &mut [FeedItem], inbox: &Inbox, source: &LoadedFeed) {
    let answered: Vec<_> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| match item {
            FeedItem::Ask { request, answer: Some(answer) } => Some((index, request.clone(), answer.clone())),
            _ => None,
        })
        .collect();
    show_pending(items, inbox);
    let root = inbox.dir().parent();
    let same_project = root.is_some_and(|root| source.project.as_ref().is_ok_and(|project| project.check(root).is_ok()));
    let current = same_project && root.is_some_and(|root| source.display_source.as_ref().is_some_and(|saved| saved.is_current(&root.join(FEED_FILE))));
    if !current {
        for (index, mut original, answer) in answered {
            if let Some(item) = items.get_mut(index) {
                // The overlay has just checked that this question is still waiting and
                // removed delivery confirmation. An unrelated feed write must not take
                // away its retry control. A replaced project must not reuse that control.
                if same_project && matches!(item, FeedItem::Ask { request, answer: None } if request.id == original.id) {
                    // The same request may have a newer undelivered reply on disk. Preserve
                    // the control, but do not describe this snapshot's words as that reply.
                    original.detail.push_str("\n\nA saved reply has not been delivered, but the conversation changed. Refresh to see the latest reply before retrying.\n\nEarlier saved reply:\n");
                    original.detail.push_str(&answer);
                    *item = FeedItem::Ask { request: original, answer: None };
                    continue;
                }
                *item = FeedItem::Error {
                    message: format!(
                        "Delivery of your saved reply to \"{}\" could not be confirmed because the conversation changed.\n\nEarlier saved reply:\n{answer}",
                        original.title
                    ),
                    remediation: Some("Refresh the conversation to see the latest reply and delivery state before answering again.".into()),
                    log: None,
                };
            }
        }
    }
}

/// Offer undelivered answers for retry in a cloned display list, never in a writer's feed.
/// If the request expired, keep its saved words visible without claiming they were delivered.
pub fn show_pending(items: &mut [FeedItem], inbox: &Inbox) {
    let waiting: HashSet<String> = inbox.pending().into_iter().map(|request| request.id).collect();
    for item in items {
        if let FeedItem::Ask { request, answer } = item {
            let delivery = inbox.delivery_state(&request.id);
            if matches!(delivery, Ok(DeliveryState::Delivered)) {
                continue;
            }
            if let Err(error) = &delivery {
                if let Some(shown) = answer {
                    *item = FeedItem::Error {
                        message: format!("Delivery of your saved reply to \"{}\" could not be confirmed.\n\nSaved reply:\n{shown}", request.title),
                        remediation: Some(format!(
                            "{} Keep these words and restore access to the project's inbox to confirm delivery before sending another reply.",
                            error.message
                        )),
                        log: None,
                    };
                }
                continue;
            }
            if waiting.contains(&request.id) {
                if let Some(shown) = answer.take() {
                    request.detail.push_str("\n\nYour saved reply has not been delivered. You can retry it:\n");
                    request.detail.push_str(&shown);
                }
            } else if let Some(shown) = answer
                && !matches!(delivery, Ok(DeliveryState::Untracked))
            {
                *item = FeedItem::Error {
                    message: format!("Your reply to \"{}\" was saved but has not been delivered.\n\nSaved reply:\n{shown}", request.title),
                    remediation: Some("This question is no longer waiting. Keep these words and copy them into a new request from the builder.".into()),
                    log: None,
                };
            }
        }
    }
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
/// saved by a newer Craft Anything is never written over (docs/file-formats.md). A read failure
/// also leaves the path unchanged; only a missing path can be created. A readable damaged
/// conversation is preserved byte-for-byte in a verified recovery copy before replacement.
/// The loaded source is compared under the project lock on every save.
pub fn save(root: &Path, feed: &mut LoadedFeed) -> Result<(), ToolError> {
    save_inner(root, feed, false)
}

/// Save replayed events only if the replacement still matches the bytes that were decoded.
/// All application feed writers share the project lock through comparison and replacement.
pub fn save_if_unchanged(root: &Path, feed: &mut LoadedFeed) -> Result<(), ToolError> {
    save_inner(root, feed, true)
}

/// Save inside a transaction already held by an inbox submission. Request locks must have
/// been acquired before this guard; the caller keeps the guard through answer publication.
pub fn save_locked(guard: &ProjectFeedGuard, feed: &mut LoadedFeed) -> Result<(), ToolError> {
    save_locked_using(guard, feed, false, crate::project::write_atomic)
}

/// Commit a replay against its decoded source without reacquiring the project lock.
pub fn save_if_unchanged_locked(guard: &ProjectFeedGuard, feed: &mut LoadedFeed) -> Result<(), ToolError> {
    save_locked_using(guard, feed, true, crate::project::write_atomic)
}

fn save_inner(root: &Path, feed: &mut LoadedFeed, require_decoded_source: bool) -> Result<(), ToolError> {
    save_using(root, feed, require_decoded_source, crate::project::write_atomic)
}

fn save_using(
    root: &Path,
    feed: &mut LoadedFeed,
    require_decoded_source: bool,
    write: impl FnOnce(&Path, &[u8]) -> Result<(), ToolError>,
) -> Result<(), ToolError> {
    feed.save_needs_sync = false;
    if let Some(error) = &feed.not_loaded {
        return Err(error.clone());
    }
    let guard = lock_project(root)?;
    save_locked_using(&guard, feed, require_decoded_source, write)
}

fn save_locked_using(
    guard: &ProjectFeedGuard,
    feed: &mut LoadedFeed,
    require_decoded_source: bool,
    write: impl FnOnce(&Path, &[u8]) -> Result<(), ToolError>,
) -> Result<(), ToolError> {
    save_locked_with_directory_sync(guard, feed, require_decoded_source, write, recovery::sync_directory)
}

pub(crate) fn save_locked_with_directory_sync(
    guard: &ProjectFeedGuard,
    feed: &mut LoadedFeed,
    require_decoded_source: bool,
    write: impl FnOnce(&Path, &[u8]) -> Result<(), ToolError>,
    sync: impl FnOnce(&Path) -> Result<(), ToolError>,
) -> Result<(), ToolError> {
    feed.save_needs_sync = false;
    if let Some(error) = &feed.not_loaded {
        return Err(error.clone());
    }
    guard.check_project()?;
    let root = &guard.root;
    feed.project.as_ref().map_err(Clone::clone)?.check(root)?;
    let path = root.join(FEED_FILE);
    let previous = read_previous(&path)?;
    if previous.as_deref().is_some_and(newer_version) {
        return Err(newer_error());
    }
    // An ordinary caller can hold an old snapshot too. Compare every loaded source while
    // holding the same lock as replacement, including a confirmed-absent original.
    if (require_decoded_source && feed.decoded_source.is_none())
        || feed.decoded_source.as_ref().is_some_and(|source| previous.as_deref().map(craft_anything_evidence::sha256_hex).as_ref() != Some(source))
        || (feed.decoded_source.is_none() && feed.damaged.is_none() && previous.is_some())
    {
        return Err(ToolError::new(ErrorCode::FileChanged, "The saved conversation changed before its pending events could be saved.")
            .with_remediation("Load the current conversation again before replaying the pending events. Nothing was written."));
    }
    if let Some(original) = &feed.damaged
        && previous.as_deref() != Some(original.as_slice())
    {
        return Err(ToolError::new(ErrorCode::FileChanged, "The saved conversation changed after it couldn't be read.")
            .with_remediation("Reopen the project to read the current conversation before retrying. Nothing was written."));
    }
    let mut items = feed.items.clone();
    if let Some(bytes) = previous.filter(|bytes| decode(bytes).is_none()) {
        let backup = keep_damaged(root, &bytes)?;
        let notice = FeedItem::Error {
            message: DAMAGED_NOTICE.into(),
            remediation: Some(format!(
                "The complete original is kept at {}. This conversation continues from the project's recoverable history.",
                backup.display()
            )),
            log: None,
        };
        if let Some(item) = items.iter_mut().find(|item| matches!(item, FeedItem::Error { message, .. } if message == DAMAGED_NOTICE)) {
            *item = notice;
        } else {
            items.insert(0, notice);
        }
    }
    let saved = Saved { version: FEED_VERSION, items, recovery: feed.recovery.clone() };
    let json = serde_json::to_vec(&saved).map_err(|e| ToolError::internal(e.to_string()))?;
    write(&path, &json)?;
    feed.items = saved.items;
    feed.damaged = None;
    feed.decoded_source = Some(craft_anything_evidence::sha256_hex(&json));
    feed.display_source = DisplaySource::read(&path).map(std::sync::Arc::new);
    feed.save_needs_sync = true;
    sync(root).map_err(|mut error| {
        error.message = format!("The conversation was written, but its folder could not be synced: {}", error.message);
        if error.remediation.is_none() {
            error.remediation = Some(
                "Keep the project open and restore access to its folder, then retry saving. Answer delivery remains pending until the save completes.".into(),
            );
        }
        error
    })?;
    feed.save_needs_sync = false;
    Ok(())
}

/// Every backup recovery can create. Deletion additionally verifies a generated receipt and
/// the current regular file's bytes, rather than taking ownership from its name alone.
fn damaged_backups(root: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    DAMAGED_BACKUP_NAMES.iter().map(move |name| root.join(name))
}

/// Keep an exact copy without ever opening an existing backup for replacement. A failed copy
/// leaves the source untouched; a failed later save reuses only a verified copy of these bytes.
fn keep_damaged(root: &Path, bytes: &[u8]) -> Result<PathBuf, ToolError> {
    if let Some(backup) = backups::matching_copy(root, bytes)? {
        return Ok(backup);
    }
    let mut unverifiable = None;
    for backup in damaged_backups(root) {
        match std::fs::symlink_metadata(&backup) {
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(ToolError::io(backup.display(), error)),
        }
        match backups::publish(root, &backup, bytes) {
            Ok(true) => return Ok(backup),
            Ok(false) => {}
            Err(error) if backups::is_unverifiable_staging(&error) => {
                unverifiable.get_or_insert(error);
            }
            Err(error) => return Err(error),
        }
    }
    if let Some(error) = unverifiable {
        return Err(error);
    }
    Err(ToolError::new(
        ErrorCode::FileError,
        format!("The conversation wasn't saved: all {} conversation recovery slots are already in use.", DAMAGED_BACKUP_NAMES.len()),
    )
    .with_remediation(
        "Move an earlier .broken backup and its matching inbox/feed-backups/<backup-name>.json ownership record, including its retained identity link and receipt in inbox/feed-backups/pending, to another safe location, then retry. The original conversation is unchanged.",
    ))
}

/// Record an answer given while no run is going (straight into the saved feed). `request` is the
/// question, taken before it was answered: one the saved conversation doesn't have yet (posted
/// from outside a run) is added with its answer, rather than lost once it stops waiting (#176).
pub fn record_answer(ws: &Workspace, request: &Request, shown: &str) -> Result<(), ToolError> {
    record_answer_using(ws, request, shown, || {})
}

pub(crate) fn record_answer_locked(guard: &ProjectFeedGuard, ws: &Workspace, request: &Request, shown: &str) -> Result<(), ToolError> {
    record_answer_locked_using(guard, ws, request, shown, || {})
}

fn record_answer_using(ws: &Workspace, request: &Request, shown: &str, before_save: impl FnOnce()) -> Result<(), ToolError> {
    let guard = lock_project(&ws.root)?;
    record_answer_locked_using(&guard, ws, request, shown, before_save)
}

fn record_answer_locked_using(guard: &ProjectFeedGuard, ws: &Workspace, request: &Request, shown: &str, before_save: impl FnOnce()) -> Result<(), ToolError> {
    // Inbox submission already owns its request lock. Do not reacquire it while holding
    // this feed lock; load only reads pending requests and delivery evidence.
    guard.check_workspace(ws)?;
    let mut feed = load(ws);
    record_pending_answer(&mut feed, request, shown);
    before_save();
    save_locked_using(guard, &mut feed, false, crate::project::write_atomic)
}

/// Record the latest answer to a request captured from the pending inbox, including a retry
/// after the earlier answer was saved but couldn't be delivered.
pub fn record_pending_answer(feed: &mut Vec<FeedItem>, request: &Request, shown: &str) {
    if let Some(FeedItem::Ask { answer, .. }) = feed.iter_mut().find(|item| matches!(item, FeedItem::Ask { request: r, .. } if r.id == request.id)) {
        *answer = Some(shown.to_owned());
    } else {
        feed.push(FeedItem::Ask { request: request.clone(), answer: Some(shown.to_owned()) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbox::RequestKind;
    use crate::workspace::TargetRef;

    /// Seed a fixture against its current bytes; production writers use the value from `load`.
    fn save(root: &Path, items: &[FeedItem]) -> Result<(), ToolError> {
        let previous = read_previous(&root.join(FEED_FILE))?;
        let damaged = previous.as_ref().filter(|bytes| decode(bytes).is_none()).cloned();
        let decoded_source = previous.as_ref().filter(|bytes| decode(bytes).is_some()).map(|bytes| craft_anything_evidence::sha256_hex(bytes));
        let project = transaction::ProjectIdentity::from_workspace(&Workspace::open(root)?).map(std::sync::Arc::new);
        let display_source = DisplaySource::read(&root.join(FEED_FILE)).map(std::sync::Arc::new);
        super::save(
            root,
            &mut LoadedFeed {
                items: items.to_vec(),
                not_loaded: None,
                damaged,
                decoded_source,
                recovery: None,
                project,
                display_source,
                save_needs_sync: false,
            },
        )
    }

    fn question(ws: &Workspace, title: &str) -> Request {
        let inbox = Inbox::new(ws.inbox_dir());
        let id = inbox.post(RequestKind::Question, title, "", vec![], true, None).unwrap();
        inbox.pending().into_iter().find(|request| request.id == id).unwrap()
    }

    #[test]
    fn a_directory_sync_failure_keeps_the_visible_feed_as_the_next_source() {
        for damaged in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let ws = Workspace::create(directory.path(), "Site", "A website", TargetRef::None).unwrap();
            let path = ws.root.join(FEED_FILE);
            if damaged {
                std::fs::write(&path, b"damaged conversation").unwrap();
            }
            let mut feed = load(&ws);
            feed.push(FeedItem::Paused);
            let guard = lock_project(&ws.root).unwrap();
            let error = save_locked_with_directory_sync(&guard, &mut feed, false, crate::project::write_atomic, |_| {
                Err(ToolError::new(ErrorCode::FileError, "injected directory sync failure"))
            })
            .unwrap_err();

            assert!(error.message.contains("was written"));
            assert!(error.message.contains("injected directory sync failure"));
            assert!(feed.save_needs_sync());
            assert!(feed.was_decoded());
            assert_eq!(feed.decoded_source, Some(craft_anything_evidence::sha256_hex(&std::fs::read(&path).unwrap())));
            assert_eq!(feed.items, load(&ws).items);

            save_if_unchanged_locked(&guard, &mut feed).unwrap();
            assert!(!feed.save_needs_sync());
            assert_eq!(load(&ws).iter().filter(|item| matches!(item, FeedItem::Paused)).count(), 1);
        }
    }

    #[test]
    fn a_later_prewrite_failure_clears_the_previous_directory_sync_state() {
        for failure in ["source", "project", "lock"] {
            let directory = tempfile::tempdir().unwrap();
            let ws = Workspace::create(directory.path(), "Site", "A website", TargetRef::None).unwrap();
            let mut feed = load(&ws);
            let guard = lock_project(&ws.root).unwrap();
            assert!(
                save_locked_with_directory_sync(&guard, &mut feed, false, crate::project::write_atomic, |_| {
                    Err(ToolError::new(ErrorCode::FileError, "injected directory sync failure"))
                })
                .is_err()
            );
            assert!(feed.save_needs_sync());
            let path = ws.root.join(FEED_FILE);
            feed.push(FeedItem::Paused);
            match failure {
                "source" => {
                    std::fs::write(&path, br#"{"version":6,"items":[{"User":"Another saved conversation"}]}"#).unwrap();
                    assert_eq!(save_locked(&guard, &mut feed).unwrap_err().code, ErrorCode::FileChanged);
                }
                "project" => {
                    std::fs::remove_file(ws.root.join(crate::workspace::MANIFEST)).unwrap();
                    assert!(save_locked(&guard, &mut feed).is_err());
                }
                _ => {
                    drop(guard);
                    std::fs::write(ws.inbox_dir().join("feed.lock"), b"personal entry").unwrap();
                    assert!(super::save(&ws.root, &mut feed).is_err());
                }
            }
            assert!(!feed.save_needs_sync(), "{failure}");
            assert!(!decode(&std::fs::read(&path).unwrap()).unwrap().items.iter().any(|item| matches!(item, FeedItem::Paused)));
        }
    }

    #[test]
    fn a_stale_snapshot_does_not_assign_a_retry_delivery_to_earlier_words() {
        use crate::inbox::Answer;
        for consumed in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let ws = Workspace::create(directory.path(), "Colour", "Pick a colour", TargetRef::None).unwrap();
            let inbox = Inbox::new(ws.inbox_dir());
            let request = question(&ws, "Which colour?");
            let answer_path = inbox.dir().join(format!("{}.answer.json", request.id));
            std::fs::create_dir(&answer_path).unwrap();
            assert!(inbox.answer_recorded(&ws, &request.id, &Answer { text: Some("Blue".into()), ..Default::default() }, "Blue").is_err());
            std::fs::remove_dir(&answer_path).unwrap();

            let shown = for_display_using(&ws, || {
                inbox.answer_recorded(&ws, &request.id, &Answer { text: Some("Red".into()), ..Default::default() }, "Red").unwrap();
                if consumed {
                    assert_eq!(inbox.wait(&request.id, std::time::Duration::ZERO, &|| false).unwrap().as_text(), "Red");
                }
            });

            assert!(
                shown
                    .iter()
                    .any(|item| matches!(item, FeedItem::Error { message, .. } if message.contains("could not be confirmed") && message.contains("Blue")))
            );
            assert!(!shown.iter().any(|item| matches!(item, FeedItem::Ask { answer: Some(answer), .. } if answer == "Blue")));
            assert!(for_display(&ws).iter().any(|item| matches!(item, FeedItem::Ask { answer: Some(answer), .. } if answer == "Red")));
        }
    }

    #[test]
    fn a_superseded_pending_reply_is_labelled_as_earlier_text() {
        use crate::inbox::Answer;
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Colour", "Pick a colour", TargetRef::None).unwrap();
        let inbox = Inbox::new(ws.inbox_dir());
        let request = question(&ws, "Which colour?");
        let blocked = inbox.dir().join(format!("{}.answer.json", request.id));
        std::fs::create_dir(&blocked).unwrap();
        assert!(inbox.answer_recorded(&ws, &request.id, &Answer { text: Some("Blue".into()), ..Default::default() }, "Blue").is_err());
        let source = load(&ws);
        assert!(inbox.answer_recorded(&ws, &request.id, &Answer { text: Some("Red".into()), ..Default::default() }, "Red").is_err());
        let mut shown = source.items.clone();
        show_pending_for_feed(&mut shown, &inbox, &source);
        let card = shown
            .iter()
            .find_map(|item| match item {
                FeedItem::Ask { request: current, answer: None } if current.id == request.id => Some(current),
                _ => None,
            })
            .expect("the pending request stays actionable");
        assert!(card.detail.contains("Earlier saved reply:\nBlue"), "stale words must not be offered as the current saved retry: {}", card.detail);
        assert!(!card.detail.contains("You can retry it"));
        assert!(for_display(&ws).iter().any(|item| matches!(item,
            FeedItem::Ask { request: current, answer: None } if current.id == request.id && current.detail.ends_with("Red"))));
        assert!(source.iter().any(|item| matches!(item, FeedItem::Ask { answer: Some(answer), .. } if answer == "Blue")));
    }

    #[test]
    fn an_unrelated_answer_keeps_a_pending_reply_available_for_retry() {
        use crate::inbox::Answer;
        for consumed in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let ws = Workspace::create(directory.path(), "Colour", "Pick a colour", TargetRef::None).unwrap();
            let inbox = Inbox::new(ws.inbox_dir());
            let first = question(&ws, "Which colour?");
            let second = question(&ws, "Which size?");
            let blocked = inbox.dir().join(format!("{}.answer.json", first.id));
            std::fs::create_dir(&blocked).unwrap();
            assert!(inbox.answer_recorded(&ws, &first.id, &Answer { text: Some("Blue".into()), ..Default::default() }, "Blue").is_err());
            std::fs::remove_dir(&blocked).unwrap();
            let source = load(&ws);

            inbox.answer_recorded(&ws, &second.id, &Answer { text: Some("Large".into()), ..Default::default() }, "Large").unwrap();
            if consumed {
                assert_eq!(inbox.wait(&second.id, std::time::Duration::ZERO, &|| false).unwrap().as_text(), "Large");
            }
            let mut shown = source.items.clone();
            show_pending_for_feed(&mut shown, &inbox, &source);

            assert!(
                shown.iter().any(|item| {
                    matches!(item, FeedItem::Ask { request, answer: None }
                    if request.id == first.id && request.detail.contains("has not been delivered") && request.detail.contains("Blue"))
                }),
                "an unrelated conversation change must keep the pending question actionable"
            );
            assert!(!shown.iter().any(|item| matches!(item, FeedItem::Ask { request, answer: Some(_) } if request.id == first.id)));
            assert!(source.iter().any(|item| matches!(item, FeedItem::Ask { request, answer: Some(answer) } if request.id == first.id && answer == "Blue")));
            assert!(inbox.pending().iter().any(|request| request.id == first.id));

            inbox.answer_recorded(&ws, &first.id, &Answer { text: Some("Red".into()), ..Default::default() }, "Red").unwrap();
            assert_eq!(inbox.wait(&first.id, std::time::Duration::ZERO, &|| false).unwrap().as_text(), "Red");
            assert!(for_display(&ws).iter().any(|item| {
                matches!(item, FeedItem::Ask { request, answer: Some(answer) }
                    if request.id == first.id && answer == "Red" && !request.detail.contains("has not been delivered"))
            }));
        }
    }

    #[test]
    fn a_replaced_project_does_not_keep_an_old_question_retry_control() {
        use crate::inbox::Answer;
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Colour", "Pick a colour", TargetRef::None).unwrap();
        let inbox = Inbox::new(ws.inbox_dir());
        let request = question(&ws, "Which colour?");
        let blocked = inbox.dir().join(format!("{}.answer.json", request.id));
        std::fs::create_dir(&blocked).unwrap();
        assert!(inbox.answer_recorded(&ws, &request.id, &Answer { text: Some("Blue".into()), ..Default::default() }, "Blue").is_err());
        std::fs::remove_dir(&blocked).unwrap();
        let source = load(&ws);
        let mut changed = serde_json::to_value(&ws.manifest).unwrap();
        changed["generation"] = serde_json::json!("ffffffffffffffffffffffffffffffff");
        crate::project::write_atomic(&ws.root.join(crate::workspace::MANIFEST), &serde_json::to_vec(&changed).unwrap()).unwrap();

        let mut shown = source.items.clone();
        show_pending_for_feed(&mut shown, &inbox, &source);

        assert!(!shown.iter().any(|item| matches!(item, FeedItem::Ask { request: current, .. } if current.id == request.id)));
        assert!(
            shown.iter().any(|item| matches!(item, FeedItem::Error { message, .. } if message.contains("could not be confirmed") && message.contains("Blue")))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_live_display_does_not_block_on_a_named_pipe() {
        use crate::inbox::Answer;
        use std::time::Duration;
        const FIXTURE: &str = "CRAFT_TEST_DISPLAY_FIFO_ROOT";
        if let Some(root) = std::env::var_os(FIXTURE) {
            let ws = Workspace::open(Path::new(&root)).unwrap();
            let inbox = Inbox::new(ws.inbox_dir());
            let source = load(&ws);
            let mut unchanged = source.items.clone();
            show_pending_for_feed(&mut unchanged, &inbox, &source);
            assert!(unchanged.iter().any(|item| matches!(item, FeedItem::Ask { answer: Some(answer), .. } if answer == "Blue")));
            let path = ws.root.join(FEED_FILE);
            std::fs::remove_file(&path).unwrap();
            craft_anything_platform::Process::new("mkfifo").args([path.as_os_str()]).timeout(Duration::from_secs(2)).run_checked().unwrap();

            let mut shown = source.items.clone();
            show_pending_for_feed(&mut shown, &inbox, &source);

            assert!(
                shown
                    .iter()
                    .any(|item| matches!(item, FeedItem::Error { message, .. } if message.contains("could not be confirmed") && message.contains("Blue")))
            );
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Colour", "Pick a colour", TargetRef::None).unwrap();
        let inbox = Inbox::new(ws.inbox_dir());
        let request = question(&ws, "Which colour?");
        inbox.answer_recorded(&ws, &request.id, &Answer { text: Some("Blue".into()), ..Default::default() }, "Blue").unwrap();
        craft_anything_platform::Process::new(std::env::current_exe().unwrap())
            .args(["--exact", "feed::tests::a_live_display_does_not_block_on_a_named_pipe", "--test-threads=1"])
            .env(FIXTURE, &ws.root)
            .timeout(Duration::from_secs(5))
            .stdout_cap(64 * 1024)
            .stderr_cap(64 * 1024)
            .run_checked()
            .expect("a named pipe replacing the saved feed must not block display refresh");
    }

    #[cfg(unix)]
    #[test]
    fn a_guarded_save_rejects_a_named_pipe_and_releases_the_project_lock() {
        use std::os::unix::fs::FileTypeExt as _;
        use std::time::Duration;
        const FIXTURE: &str = "CRAFT_TEST_SAVE_FIFO_ROOT";
        if let Some(root) = std::env::var_os(FIXTURE) {
            let ws = Workspace::open(Path::new(&root)).unwrap();
            let mut source = load(&ws);
            assert!(source.was_decoded());
            source.push(FeedItem::Paused);
            let path = ws.root.join(FEED_FILE);
            let original = ws.root.join("original-conversation.json");
            std::fs::rename(&path, &original).unwrap();
            let before = std::fs::read(&original).unwrap();
            craft_anything_platform::Process::new("mkfifo").args([path.as_os_str()]).timeout(Duration::from_secs(2)).run_checked().unwrap();

            assert_eq!(super::save(&ws.root, &mut source).unwrap_err().code, ErrorCode::BadInput);
            assert!(std::fs::symlink_metadata(&path).unwrap().file_type().is_fifo());
            assert_eq!(std::fs::read(&original).unwrap(), before);
            assert!(load(&ws).had_read_error(), "a fresh load also refuses the pipe without waiting");
            let guard = lock_project(&ws.root).expect("the rejected save releases its project guard");
            guard.check_workspace(&ws).unwrap();
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Project", "Keep my conversation", TargetRef::None).unwrap();
        super::save(&ws.root, &mut load(&ws)).unwrap();
        craft_anything_platform::Process::new(std::env::current_exe().unwrap())
            .args(["--exact", "feed::tests::a_guarded_save_rejects_a_named_pipe_and_releases_the_project_lock", "--test-threads=1"])
            .env(FIXTURE, &ws.root)
            .timeout(Duration::from_secs(5))
            .stdout_cap(64 * 1024)
            .stderr_cap(64 * 1024)
            .run_checked()
            .expect("a pipe must not block a guarded save or leave the project locked");
    }

    #[test]
    fn ordinary_saves_preserve_a_concurrently_recorded_answer() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Site", "A website", TargetRef::None).unwrap();
        let request = question(&ws, "Which colour?");
        super::save(&ws.root, &mut load(&ws)).unwrap();
        let mut older = load(&ws);
        older.push(FeedItem::Paused);
        record_answer(&ws, &request, "Blue").unwrap();
        let path = ws.root.join(FEED_FILE);
        let before = std::fs::read(&path).unwrap();

        assert_eq!(super::save(&ws.root, &mut older).unwrap_err().code, ErrorCode::FileChanged);

        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(matches!(older.last(), Some(FeedItem::Paused)), "the caller retains its unsaved change for replay");
    }

    #[test]
    fn an_absent_feed_snapshot_cannot_replace_a_new_conversation() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Site", "A website", TargetRef::None).unwrap();
        let request = question(&ws, "Which colour?");
        let mut absent = load(&ws);
        record_answer(&ws, &request, "Blue").unwrap();
        let path = ws.root.join(FEED_FILE);
        let before = std::fs::read(&path).unwrap();
        absent.push(FeedItem::Paused);

        assert_eq!(super::save(&ws.root, &mut absent).unwrap_err().code, ErrorCode::FileChanged);

        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn a_successful_first_save_establishes_the_next_source_guard() {
        for damaged in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let ws = Workspace::create(directory.path(), "Site", "A website", TargetRef::None).unwrap();
            let path = ws.root.join(FEED_FILE);
            if damaged {
                std::fs::write(&path, b"damaged conversation").unwrap();
            }
            let mut feed = load(&ws);
            super::save(&ws.root, &mut feed).unwrap();
            feed.push(FeedItem::Paused);

            save_if_unchanged(&ws.root, &mut feed).unwrap();
            super::save(&ws.root, &mut feed).unwrap();

            assert_eq!(feed.decoded_source, Some(craft_anything_evidence::sha256_hex(&std::fs::read(&path).unwrap())));
            assert_eq!(load(&ws).iter().filter(|item| matches!(item, FeedItem::Paused)).count(), 1);
        }
    }

    #[test]
    fn an_answer_cannot_commit_between_recovery_comparison_and_replacement() {
        use std::sync::mpsc;
        use std::time::Duration;
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Site", "A website", TargetRef::None).unwrap();
        let request = question(&ws, "Which colour?");
        super::save(&ws.root, &mut load(&ws)).unwrap();
        let mut replayed = load(&ws);
        replayed.push(FeedItem::Paused);

        std::thread::scope(|scope| {
            let (release, go) = mpsc::channel();
            let (entered, ready) = mpsc::channel();
            let (completed, done) = mpsc::channel();
            let ws = &ws;
            let request = &request;
            let other = scope.spawn(move || {
                go.recv_timeout(Duration::from_secs(5)).unwrap();
                entered.send(()).unwrap();
                let result = record_answer(ws, request, "Blue");
                completed.send(()).unwrap();
                result
            });
            save_using(&ws.root, &mut replayed, true, |path, bytes| {
                release.send(()).unwrap();
                ready.recv_timeout(Duration::from_secs(5)).unwrap();
                // The old uncoordinated answer completes here; a coordinated writer waits
                // for this comparison/replacement transaction to release its lock.
                let _ = done.recv_timeout(Duration::from_millis(200));
                crate::project::write_atomic(path, bytes)
            })
            .unwrap();
            other.join().unwrap().unwrap();
        });

        let saved = load(&ws);
        assert!(saved.iter().any(|item| matches!(item, FeedItem::Paused)));
        assert!(saved.iter().any(|item| matches!(item, FeedItem::Ask { request: saved, answer: Some(answer) } if saved.id == request.id && answer == "Blue")));
    }

    #[test]
    fn concurrent_answer_transactions_keep_both_replies() {
        use std::sync::mpsc;
        use std::time::Duration;
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Site", "A website", TargetRef::None).unwrap();
        let first = question(&ws, "Which colour?");
        let second = question(&ws, "Which size?");
        super::save(&ws.root, &mut load(&ws)).unwrap();

        std::thread::scope(|scope| {
            let (release, go) = mpsc::channel();
            let (entered, ready) = mpsc::channel();
            let (completed, done) = mpsc::channel();
            let ws = &ws;
            let second = &second;
            let other = scope.spawn(move || {
                go.recv_timeout(Duration::from_secs(5)).unwrap();
                entered.send(()).unwrap();
                let result = record_answer(ws, second, "Large");
                completed.send(()).unwrap();
                result
            });
            record_answer_using(ws, &first, "Blue", || {
                release.send(()).unwrap();
                ready.recv_timeout(Duration::from_secs(5)).unwrap();
                let _ = done.recv_timeout(Duration::from_millis(200));
            })
            .unwrap();
            other.join().unwrap().unwrap();
        });

        let saved = load(&ws);
        for (request, shown) in [(&first, "Blue"), (&second, "Large")] {
            assert!(
                saved.iter().any(|item| matches!(item, FeedItem::Ask { request: saved, answer: Some(answer) } if saved.id == request.id && answer == shown))
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn failed_replacements_reuse_one_verified_recovery_copy() {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Site", "A website", TargetRef::None).unwrap();
        let permissions = std::fs::metadata(&ws.root).unwrap().permissions();
        std::fs::set_permissions(&ws.root, std::fs::Permissions::from_mode(0o500)).unwrap();
        let probe = ws.root.join("write-probe");
        let denied = std::fs::write(&probe, b"probe");
        std::fs::set_permissions(&ws.root, permissions.clone()).unwrap();
        if denied.is_ok() {
            std::fs::remove_file(&probe).unwrap();
            eprintln!("permission test skipped: this process can create entries in a read-only directory");
            return;
        }
        let path = ws.root.join(FEED_FILE);
        let original = b"damaged conversation";
        std::fs::write(&path, original).unwrap();
        let mut feed = load(&ws);
        for _ in 0..20 {
            let result = save_using(&ws.root, &mut feed, false, |path, bytes| {
                std::fs::set_permissions(&ws.root, std::fs::Permissions::from_mode(0o500)).unwrap();
                let result = crate::project::write_atomic(path, bytes);
                std::fs::set_permissions(&ws.root, permissions.clone()).unwrap();
                result
            });
            assert!(result.is_err());
            assert_eq!(std::fs::read(&path).unwrap(), original);
            assert_eq!(damaged_backups(&ws.root).filter(|backup| backup.exists()).count(), 1, "retry reuses the complete verified copy");
        }
        super::save(&ws.root, &mut feed).unwrap();
        assert!(load(&ws).was_decoded());
        assert_eq!(damaged_backups(&ws.root).filter(|backup| backup.exists()).count(), 1);
        assert_eq!(std::fs::read(ws.root.join("feed.json.broken")).unwrap(), original);
    }

    #[test]
    fn only_decoded_saved_bytes_establish_a_recovery_base() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "x", "Fallback", TargetRef::None).unwrap();
        let path = ws.root.join(FEED_FILE);
        let missing = load(&ws);
        assert!(missing.is_readable());
        assert!(!missing.was_decoded());
        std::fs::create_dir(&path).unwrap();
        let unreadable = load(&ws);
        assert!(!unreadable.was_decoded());
        assert!(unreadable.had_read_error());
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, b"damaged").unwrap();
        let damaged = load(&ws);
        assert!(!damaged.was_decoded());
        assert!(!damaged.is_readable());
        std::fs::write(&path, br#"{"version":5,"items":[{"User":"Actual saved words"}]}"#).unwrap();
        let decoded = load(&ws);
        assert!(decoded.was_decoded());
        assert!(decoded.is_readable());
        assert!(decoded.recovery_projection().is_none());
        std::fs::write(&path, br#"{"version":999,"items":[]}"#).unwrap();
        let future = load(&ws);
        assert!(!future.was_decoded());
        assert!(future.had_read_error());
    }

    #[test]
    fn recovery_save_rejects_a_replacement_changed_after_loading() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "x", "Fallback", TargetRef::None).unwrap();
        let path = ws.root.join(FEED_FILE);
        let decoded = br#"{"version":5,"items":[{"User":"Decoded replacement"}]}"#;
        let changed = br#"{"version":5,"items":[{"User":"Another client's saved answer"}]}"#;
        for replacement in [Some(changed.as_slice()), Some(b"damaged again".as_slice()), None] {
            std::fs::write(&path, decoded).unwrap();
            let mut feed = load(&ws);
            feed.push(FeedItem::Paused);
            let pending = feed.items.clone();
            match replacement {
                Some(bytes) => std::fs::write(&path, bytes).unwrap(),
                None => std::fs::remove_file(&path).unwrap(),
            }
            assert_eq!(save_if_unchanged(&ws.root, &mut feed).unwrap_err().code, ErrorCode::FileChanged);
            assert_eq!(read_previous(&path).unwrap().as_deref(), replacement);
            assert_eq!(feed.items, pending, "the replayed events remain available for another load");
            assert!(!ws.root.join("feed.json.broken").exists());
        }
    }

    #[test]
    fn recovery_save_requires_decoded_bytes_and_keeps_its_updated_identity() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "x", "Fallback", TargetRef::None).unwrap();
        let path = ws.root.join(FEED_FILE);
        let mut absent = load(&ws);
        assert_eq!(save_if_unchanged(&ws.root, &mut absent).unwrap_err().code, ErrorCode::FileChanged);
        assert!(!path.exists());
        std::fs::write(&path, b"damaged").unwrap();
        let mut damaged = load(&ws);
        assert_eq!(save_if_unchanged(&ws.root, &mut damaged).unwrap_err().code, ErrorCode::FileChanged);
        assert_eq!(std::fs::read(&path).unwrap(), b"damaged");
        save(&ws.root, &[FeedItem::User("Saved replacement".to_owned().into())]).unwrap();
        let mut feed = load(&ws);
        feed.push(FeedItem::Paused);
        save_if_unchanged(&ws.root, &mut feed).unwrap();
        feed.push(FeedItem::Agent { text: "Another pending event".into(), streaming: false });
        save_if_unchanged(&ws.root, &mut feed).unwrap();
        assert_eq!(load(&ws).items, feed.items);
    }

    #[test]
    fn unknown_delivery_keeps_saved_words_without_suggesting_a_duplicate_reply() {
        use crate::inbox::Answer;
        for consumed in [false, true] {
            for unreadable in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let ws = Workspace::create(directory.path(), "x", "A game", TargetRef::None).unwrap();
                let inbox = Inbox::new(ws.inbox_dir());
                let id = inbox.post(RequestKind::Question, "Blue or red?", "", vec![], true, None).unwrap();
                let answer_path = inbox.dir().join(format!("{id}.answer.json"));
                let answer = Answer { text: Some("Keep these saved words".into()), ..Default::default() };
                if consumed {
                    inbox.answer_recorded(&ws, &id, &answer, "Keep these saved words").unwrap();
                    assert!(inbox.wait(&id, std::time::Duration::ZERO, &|| false).is_some());
                } else {
                    std::fs::create_dir(&answer_path).unwrap();
                    assert!(inbox.answer_recorded(&ws, &id, &answer, "Keep these saved words").is_err());
                    std::fs::remove_dir(&answer_path).unwrap();
                }
                let receipt = inbox.dir().join(format!("{id}.delivery.json"));
                std::fs::remove_file(&receipt).unwrap();
                if unreadable {
                    std::fs::create_dir(&receipt).unwrap();
                } else {
                    std::fs::write(&receipt, b"unrecognized receipt").unwrap();
                }
                let before = std::fs::read(ws.root.join(FEED_FILE)).unwrap();
                let shown = for_display(&ws);
                assert!(!shown.iter().any(|item| matches!(item, FeedItem::Ask { request, .. } if request.id == id)));
                let (message, remediation) = shown
                    .iter()
                    .find_map(|item| match item {
                        FeedItem::Error { message, remediation, .. } if message.contains("Keep these saved words") => {
                            Some((message, remediation.as_deref().unwrap_or("")))
                        }
                        _ => None,
                    })
                    .expect("the saved words remain visible");
                assert!(message.contains("could not be confirmed"));
                assert!(!message.contains("not been delivered"));
                assert!(!remediation.contains("copy them into a new request"));
                assert_eq!(std::fs::read(ws.root.join(FEED_FILE)).unwrap(), before);
                assert!(load(&ws).iter().any(|item| matches!(item, FeedItem::Ask { answer: Some(text), .. } if text == "Keep these saved words")));
            }
        }
    }

    #[test]
    fn recovery_projection_survives_a_later_manifest_update() {
        let directory = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(directory.path(), "x", "Original request", TargetRef::None).unwrap();
        ws.manifest.last_reply = "Turn A".into();
        std::fs::write(ws.root.join(FEED_FILE), b"damaged").unwrap();
        let mut recovered = load(&ws);
        ws.manifest.last_reply = "Turn B".into();
        super::save(&ws.root, &mut recovered).unwrap();
        let saved = load(&ws);
        assert!(saved.was_decoded());
        assert_eq!(saved.recovery_projection().unwrap().last_reply, "Turn A");
        assert!(saved.iter().any(|item| matches!(item, FeedItem::Agent { text, .. } if text == "Turn A")));
    }

    #[test]
    fn old_feed_formats_keep_their_items_without_inventing_recovery_provenance() {
        for original in [br#"[{"User":"Saved before recovery metadata"}]"#.as_slice(), br#"{"version":5,"items":[{"User":"Saved before recovery metadata"}]}"#]
        {
            let directory = tempfile::tempdir().unwrap();
            let ws = Workspace::create(directory.path(), "x", "Fallback", TargetRef::None).unwrap();
            let path = ws.root.join(FEED_FILE);
            std::fs::write(&path, original).unwrap();
            let mut feed = load(&ws);
            let before = feed.items.clone();
            assert!(feed.was_decoded());
            assert!(feed.recovery_projection().is_none());
            super::save(&ws.root, &mut feed).unwrap();
            let updated: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(updated["version"], FEED_VERSION);
            assert!(updated.get("recovery").is_none());
            assert_eq!(load(&ws).items, before);
        }
    }

    #[test]
    fn a_feed_that_was_not_loaded_cannot_replace_a_later_readable_file() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "Fallback", TargetRef::None).unwrap();
        let path = ws.root.join(FEED_FILE);
        std::fs::create_dir(&path).unwrap();
        let mut feed = load(&ws);
        std::fs::remove_dir(&path).unwrap();
        let original = br#"{"version":5,"items":[{"User":"Keep the actual conversation"}]}"#;
        std::fs::write(&path, original).unwrap();
        feed.push(FeedItem::Paused);
        assert!(super::save(&ws.root, &mut feed).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn malformed_feed_is_kept_under_an_unused_name_and_shown_in_the_conversation() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "Fallback", TargetRef::None).unwrap();
        let path = ws.root.join(FEED_FILE);
        let original = b"{\"version\":5,\"items\":[\xff\x00";
        std::fs::write(&path, original).unwrap();
        let previous_backup = ws.root.join("feed.json.broken");
        std::fs::write(&previous_backup, b"an earlier conversation").unwrap();
        let mut feed = load(&ws);
        feed.push(FeedItem::Paused);
        super::save(&ws.root, &mut feed).unwrap();
        let backup = ws.root.join("feed.json.broken.1");
        assert_eq!(std::fs::read(&backup).unwrap(), original);
        assert_eq!(std::fs::read(&previous_backup).unwrap(), b"an earlier conversation");
        let saved = load(&ws);
        assert!(saved.iter().any(|item| matches!(item, FeedItem::Error { remediation: Some(message), .. } if message.contains(&backup.display().to_string()))));
        assert!(matches!(saved.last(), Some(FeedItem::Paused)));
    }

    #[test]
    fn a_failed_backup_keeps_the_original_and_can_be_retried() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "Fallback", TargetRef::None).unwrap();
        let path = ws.root.join(FEED_FILE);
        let original = b"{\"version\":";
        std::fs::write(&path, original).unwrap();
        for index in 0..16 {
            let backup = path.with_extension(if index == 0 { "json.broken".into() } else { format!("json.broken.{index}") });
            std::fs::write(backup, b"keep an earlier backup").unwrap();
        }
        let mut feed = load(&ws);
        let before = feed.items.clone();
        let error = super::save(&ws.root, &mut feed).unwrap_err();
        assert!(error.remediation.as_deref().unwrap().contains("inbox/feed-backups/<backup-name>.json"));
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(feed.items, before, "a failed save must not claim that a backup was kept");
        let free = path.with_extension("json.broken.15");
        std::fs::remove_file(&free).unwrap();
        super::save(&ws.root, &mut feed).unwrap();
        assert_eq!(std::fs::read(&free).unwrap(), original);
        assert_eq!(std::fs::read(path.with_extension("json.broken")).unwrap(), b"keep an earlier backup");
        feed.push(FeedItem::Paused);
        super::save(&ws.root, &mut feed).unwrap();
        assert_eq!(std::fs::read(&free).unwrap(), original, "later saves keep the recovery copy");
    }

    #[test]
    fn a_damaged_feed_replaced_before_saving_must_be_loaded_again() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "Fallback", TargetRef::None).unwrap();
        let path = ws.root.join(FEED_FILE);
        std::fs::write(&path, b"broken").unwrap();
        let mut feed = load(&ws);
        let repaired = br#"{"version":5,"items":[{"User":"Recovered elsewhere"}]}"#;
        std::fs::write(&path, repaired).unwrap();
        assert_eq!(super::save(&ws.root, &mut feed).unwrap_err().code, ErrorCode::FileChanged);
        assert_eq!(std::fs::read(&path).unwrap(), repaired);
        assert!(!path.with_extension("json.broken").exists());
    }

    #[test]
    fn an_answer_saved_before_inbox_delivery_stays_available_for_retry() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "A game", TargetRef::None).unwrap();
        let inbox = Inbox::new(ws.inbox_dir());
        let id = inbox.post(RequestKind::Question, "Blue or red?", "", vec![], true, None).unwrap();
        let question = inbox.pending().pop().unwrap();
        record_answer(&ws, &question, "Blue").unwrap();
        // Delivery hasn't happened (or failed): reopening must still offer this request.
        assert!(for_display(&ws).iter().any(|item| matches!(item, FeedItem::Ask { request, answer: None } if request.id == id)));
        // A concurrent writer keeps the recorded answer during the delivery interval.
        let mut other = load(&ws);
        other.push(FeedItem::Paused);
        super::save(&ws.root, &mut other).unwrap();
        assert!(load(&ws).iter().any(|item| matches!(item, FeedItem::Ask { answer: Some(answer), .. } if answer == "Blue")));
        record_answer(&ws, &question, "Red").unwrap();
        inbox.answer(&id, &crate::inbox::Answer { choice: Some("Red".into()), ..Default::default() }).unwrap();
        let saved = load(&ws);
        assert_eq!(saved.iter().filter(|item| matches!(item, FeedItem::Ask { request, .. } if request.id == id)).count(), 1);
        assert!(saved.iter().any(|item| matches!(item, FeedItem::Ask { request, answer: Some(answer) } if request.id == id && answer == "Red")));
    }

    #[cfg(windows)]
    #[test]
    fn a_feed_open_without_sharing_is_preserved_after_the_handle_closes() {
        use std::os::windows::fs::OpenOptionsExt as _;
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "Fallback", TargetRef::None).unwrap();
        let path = ws.root.join(FEED_FILE);
        let original = br#"{"version":5,"items":[{"User":"Keep the actual conversation"}]}"#;
        std::fs::write(&path, original).unwrap();
        let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
        assert!(std::fs::read(&path).is_err());
        let mut feed = load(&ws);
        assert!(super::save(&ws.root, &mut feed).is_err());
        drop(held);
        assert!(super::save(&ws.root, &mut feed).is_err(), "the fallback still wasn't loaded");
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let mut reread = load(&ws);
        reread.push(FeedItem::Paused);
        super::save(&ws.root, &mut reread).unwrap();
        assert!(matches!(&load(&ws)[0], FeedItem::User(user) if user.text() == "Keep the actual conversation"));
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

    /// A writable parent lets the atomic writer replace even a write-only feed. The read must
    /// actually fail before these tests exercise the save, rather than mocking the filesystem.
    #[cfg(unix)]
    fn deny_feed_read(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o200)).unwrap();
        match std::fs::read(path) {
            Err(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
                true
            }
            Ok(_) => {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
                eprintln!("permission test skipped: this process can read a write-only file");
                false // Run as root: nothing is unreadable.
            }
        }
    }

    #[cfg(unix)]
    fn assert_unreadable_feed_is_preserved(version: u32) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::create(dir.path(), "Project", "Test", TargetRef::None).unwrap();
        let path = ws.root.join(FEED_FILE);
        let original = serde_json::to_vec(&serde_json::json!({
            "version": version,
            "items": [{"User": "Keep the saved conversation"}],
            "extra": {"keep": true}
        }))
        .unwrap();
        std::fs::write(&path, &original).unwrap();
        if !deny_feed_read(&path) {
            return;
        }
        let replacement = [FeedItem::User(UserItem::Text("New message".into()))];
        let saved = save(&ws.root, &replacement);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original, "save must not replace a feed it could not read");
        assert_eq!(saved.unwrap_err().code, ErrorCode::NoPermission);
        // Restoring access allows a normal save, but never a downgrade from a newer version.
        if version > FEED_VERSION {
            assert_eq!(save(&ws.root, &replacement).unwrap_err().code, ErrorCode::NotSupported);
            assert_eq!(std::fs::read(&path).unwrap(), original);
        } else {
            save(&ws.root, &replacement).unwrap();
            let saved: Saved = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(saved.items, replacement);
        }
    }

    #[cfg(unix)]
    #[test]
    fn save_keeps_an_unreadable_feed() {
        assert_unreadable_feed_is_preserved(FEED_VERSION);
    }

    #[cfg(unix)]
    #[test]
    fn save_keeps_an_unreadable_newer_feed() {
        assert_unreadable_feed_is_preserved(FEED_VERSION + 1);
    }

    #[cfg(unix)]
    #[test]
    fn record_answer_keeps_an_unreadable_feed_and_can_retry() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::create(dir.path(), "x", "Original request", TargetRef::None).unwrap();
        let conversation =
            vec![FeedItem::User(UserItem::Text("Original request".into())), FeedItem::Agent { text: "Keep this completed work.".into(), streaming: false }];
        save(&ws.root, &conversation).unwrap();
        let path = ws.root.join(FEED_FILE);
        let original = std::fs::read(&path).unwrap();
        let inbox = Inbox::new(ws.inbox_dir());
        let id = inbox.post(RequestKind::Question, "Blue or red?", "", vec![], true, None).unwrap();
        let question = inbox.pending().into_iter().find(|r| r.id == id).unwrap();
        inbox.answer(&id, &crate::inbox::Answer { choice: Some("Blue".into()), ..Default::default() }).unwrap();
        if !deny_feed_read(&path) {
            return;
        }
        let saved = record_answer(&ws, &question, "Blue");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original, "the display fallback must not replace the saved conversation");
        assert_eq!(saved.unwrap_err().code, ErrorCode::NoPermission);
        record_answer(&ws, &question, "Blue").unwrap();
        let loaded = load(&ws);
        assert_eq!(&loaded[..conversation.len()], conversation.as_slice());
        assert_eq!(loaded.len(), conversation.len() + 1);
        assert!(matches!(loaded.last(), Some(FeedItem::Ask { request, answer: Some(answer) }) if request.id == id && answer == "Blue"));
    }

    #[cfg(unix)]
    #[test]
    fn save_leaves_a_dangling_feed_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::create(dir.path(), "Project", "Test", TargetRef::None).unwrap();
        let path = ws.root.join(FEED_FILE);
        let target = Path::new("missing-feed.json");
        std::os::unix::fs::symlink(target, &path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap_err().kind(), std::io::ErrorKind::NotFound);
        let saved = save(&ws.root, &[FeedItem::Paused]);
        assert!(std::fs::symlink_metadata(&path).unwrap().file_type().is_symlink(), "save replaced the dangling link");
        assert_eq!(std::fs::read_link(&path).unwrap(), target);
        assert!(!ws.root.join(target).exists());
        assert_eq!(saved.unwrap_err().code, ErrorCode::NotFound);
    }

    #[cfg(unix)]
    #[test]
    fn save_leaves_a_looping_feed_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::create(dir.path(), "Project", "Test", TargetRef::None).unwrap();
        let path = ws.root.join(FEED_FILE);
        std::os::unix::fs::symlink(FEED_FILE, &path).unwrap();
        assert!(std::fs::read(&path).is_err());
        let saved = save(&ws.root, &[FeedItem::Paused]);
        assert!(std::fs::symlink_metadata(&path).unwrap().file_type().is_symlink(), "save replaced the looping link");
        assert_eq!(std::fs::read_link(&path).unwrap(), Path::new(FEED_FILE));
        assert_eq!(saved.unwrap_err().code, ErrorCode::FileError);
    }

    #[test]
    fn save_creates_absent_feeds_and_recovers_readable_malformed_json() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::create(dir.path(), "Project", "Test", TargetRef::None).unwrap();
        let path = ws.root.join(FEED_FILE);
        let conversation = [FeedItem::User(UserItem::Text("Keep building".into()))];
        assert_eq!(std::fs::symlink_metadata(&path).unwrap_err().kind(), std::io::ErrorKind::NotFound);
        save(&ws.root, &conversation).unwrap();
        let saved: Saved = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved.items, conversation);
        // A readable parse failure can recover only after its exact bytes have been kept.
        std::fs::write(&path, b"{\"version\":").unwrap();
        save(&ws.root, &conversation).unwrap();
        let saved: Saved = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved.version, FEED_VERSION);
        assert_eq!(std::fs::read(path.with_extension("json.broken")).unwrap(), b"{\"version\":");
        assert!(matches!(saved.items.first(), Some(FeedItem::Error { .. })));
        assert_eq!(&saved.items[1..], &conversation);
    }
}
