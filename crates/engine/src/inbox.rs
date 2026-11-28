//! The inbox: how an agent working on a project asks the person something — a question with
//! choices, or approval for an action — and waits for the answer the app writes back.
//!
//! It is a folder, so it works across processes (the app, our MCP server started by a coding
//! agent, the in-process agent loop): `<id>.request.json` is written by the asker and
//! `<id>.answer.json` by the app. Both are written atomically, owner-only, because a request can
//! carry the person's own words and an answer can carry a typed reply or an approval; the asker
//! removes both once it has read the answer. Ids are generated here and validated on every path
//! join.

use crate::project::write_atomic;
use craft_anything_evidence::{ErrorCode, ToolError};
use serde::{Deserialize, Serialize};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod delivery;
pub(crate) use delivery::DeliveryState;

/// The session's inbox folder (kept in `Session::ext`). Set by `mcp --inbox <dir>` or by the
/// workflow that runs the in-process agent.
#[derive(Debug, Clone)]
pub struct InboxDir(pub PathBuf);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestKind {
    /// A question; the answer is a choice and/or free text.
    Question,
    /// Permission for an action; the answer is allow/deny.
    Approval,
}

/// Something waiting for the person.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub kind: RequestKind,
    /// The question, or a one-line description of the action ("Run `npm install`").
    pub title: String,
    /// Optional context in plain words.
    #[serde(default)]
    pub detail: String,
    /// Suggested answers, shown as buttons.
    #[serde(default)]
    pub choices: Vec<String>,
    /// Whether a typed answer is accepted too.
    #[serde(default)]
    pub allow_text: bool,
    /// For approvals: the action's kind, so "always allow" can be remembered per kind.
    #[serde(default)]
    pub action: Option<String>,
    pub created_ms: u64,
}

/// The person's answer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    #[serde(default)]
    pub choice: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    /// Approvals: true to allow.
    #[serde(default)]
    pub allow: Option<bool>,
    /// Approvals: allow this kind of action for the rest of the project.
    #[serde(default)]
    pub always: bool,
}

impl Answer {
    /// The answer as one line of text for an agent.
    pub fn as_text(&self) -> String {
        match (&self.choice, self.text.as_deref().map(str::trim).filter(|t| !t.is_empty())) {
            (Some(c), Some(t)) => format!("{c} — {t}"),
            (Some(c), None) => c.clone(),
            (None, Some(t)) => t.to_string(),
            (None, None) => String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Inbox {
    dir: PathBuf,
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// A number that never repeats within this process, for names that must stay unique even when
/// two are made in the same millisecond (or a name was just freed by deletion).
pub fn next_counter() -> u64 {
    COUNTER.fetch_add(1, Ordering::SeqCst)
}

/// Ids are `[0-9a-z-]` only, so they are always safe as file names.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn no_longer_waiting() -> ToolError {
    ToolError::new(ErrorCode::NotFound, "This question is no longer waiting.").with_remediation("Keep your reply and answer a new request from the builder.")
}

fn request_entry_exists(path: &Path) -> Result<bool, ToolError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(ToolError::io(path.display(), error)),
    }
}

/// The caller holds the request lock. If the directory cannot unlink the request, its
/// writable handle can still leave an empty, synced tombstone that no submitter can parse.
fn withdraw_request(path: &Path, request: &std::fs::File) -> Result<(), ToolError> {
    let removal_error = match std::fs::remove_file(path) {
        Ok(()) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !request_entry_exists(path)? => return Ok(()),
        Err(error) => error,
    };
    request.set_len(0).and_then(|()| request.sync_all()).map_err(|error| {
        let mut error = ToolError::io(path.display(), error);
        error.message = format!("This question could not be withdrawn: {}", error.message);
        error.with_remediation(format!(
            "Removing the request also failed: {removal_error}. It may still be waiting. Restore access to the inbox and withdraw it before submitting another reply."
        ))
    })
}

fn read_answer(path: &Path) -> Result<Option<Answer>, ToolError> {
    let unreadable = |mut error: ToolError| {
        // The entry exists even when its link target disappeared. NotFound is reserved for
        // withdrawal at the submission boundary, so callers must not swallow this failure.
        if error.code == ErrorCode::NotFound {
            error.code = ErrorCode::FileError;
        }
        error.with_remediation("Make the saved answer readable before retrying. This submission has not changed it.")
    };
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(unreadable(ToolError::io(path.display(), error))),
    };
    // A directory contains no decoded answer; the existing atomic delivery will reject it.
    if metadata.is_dir() {
        return Ok(None);
    }
    let (file, metadata) = craft_anything_platform::fs::open_regular_file(path).map_err(unreadable)?;
    let length = metadata.len();
    let limit = length.checked_add(1).ok_or_else(|| unreadable(ToolError::new(ErrorCode::OverLimit, "The saved answer is too large to read safely.")))?;
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes).map_err(|error| unreadable(ToolError::io(path.display(), error)))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > length {
        return Err(unreadable(ToolError::new(ErrorCode::FileChanged, "The saved answer grew while it was being read.")));
    }
    Ok(serde_json::from_slice(&bytes).ok())
}

// A second submitter should report a busy request instead of waiting indefinitely for the
// first. The waiter itself must let an already accepted submission finish before withdrawing.
const ANSWER_LOCK_WAIT: Duration = Duration::from_secs(2);
const LOCK_POLL: Duration = Duration::from_millis(5);

impl Inbox {
    pub fn new(dir: impl Into<PathBuf>) -> Inbox {
        Inbox { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, id: &str, suffix: &str) -> Result<PathBuf, ToolError> {
        if !valid_id(id) {
            return Err(ToolError::invalid(format!("{id:?} is not an inbox id")));
        }
        Ok(self.dir.join(format!("{id}.{suffix}.json")))
    }

    /// A durable answer file is also evidence of delivery when its receipt could not be
    /// updated. Recorded submissions retain the receipt after the waiter consumes the file.
    pub(crate) fn delivery_state(&self, id: &str) -> Result<DeliveryState, ToolError> {
        if read_answer(&self.path(id, "answer")?)?.is_some() {
            return Ok(DeliveryState::Delivered);
        }
        delivery::state(&self.dir, id)
    }

    /// Post a request; returns its id.
    pub fn post(
        &self,
        kind: RequestKind,
        title: &str,
        detail: &str,
        choices: Vec<String>,
        allow_text: bool,
        action: Option<String>,
    ) -> Result<String, ToolError> {
        std::fs::create_dir_all(&self.dir).map_err(|e| ToolError::io(self.dir.display(), e))?;
        let n = next_counter();
        let id = format!("{}-{}-{n}", now_ms(), std::process::id());
        let req = Request { id: id.clone(), kind, title: title.to_string(), detail: detail.to_string(), choices, allow_text, action, created_ms: now_ms() };
        let bytes = serde_json::to_vec_pretty(&req).map_err(|e| ToolError::internal(e.to_string()))?;
        craft_anything_platform::fs::write_atomic_private(&self.path(&id, "request")?, &bytes)?;
        Ok(id)
    }

    /// Requests without an answer yet, oldest first. Unreadable files are skipped.
    pub fn pending(&self) -> Vec<Request> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut out: Vec<Request> = rd
            .flatten()
            .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".request.json")).map(str::to_string))
            .filter(|id| valid_id(id) && !matches!(self.delivery_state(id), Ok(DeliveryState::Delivered)))
            .filter_map(|id| std::fs::read(self.dir.join(format!("{id}.request.json"))).ok())
            .filter_map(|b| serde_json::from_slice::<Request>(&b).ok())
            .collect();
        out.sort_by_key(|r| r.created_ms);
        out
    }

    /// Write the person's answer.
    pub fn answer(&self, id: &str, answer: &Answer) -> Result<(), ToolError> {
        self.answer_inner(id, answer, false, |_| Ok(()))
    }

    /// Save an offline answer in the project's conversation before delivering it. Desktop,
    /// CLI and MCP callers share the same validation, persistence and submission transaction.
    pub fn answer_recorded(&self, ws: &crate::workspace::Workspace, id: &str, answer: &Answer, shown: &str) -> Result<(), ToolError> {
        self.answer_with_guard(
            id,
            answer,
            || {
                let guard = crate::feed::lock_project(&ws.root)?;
                guard.check_workspace(ws)?;
                Ok(guard)
            },
            |guard, request| crate::feed::record_answer_locked(guard, ws, request, shown),
        )
    }

    /// Accept one pending request, persist its reply, then deliver it. The request file's OS
    /// lock also guards withdrawal in `wait`, across handles and processes. `persist` runs only
    /// after accepting a still-pending request; it must not submit to this inbox again.
    /// A persistence failure releases the reservation and leaves the request available to retry.
    /// Project-backed writers use `answer_with_guard` to coordinate deletion before staging.
    pub fn answer_with(&self, id: &str, answer: &Answer, persist: impl FnOnce(&Request) -> Result<(), ToolError>) -> Result<(), ToolError> {
        self.answer_inner(id, answer, true, persist)
    }

    /// Reserve the request, prepare and validate its project guard, stage delivery, save the
    /// conversation and publish the answer. The guard survives every step after preparation.
    /// `prepare` must acquire the project guard here, after the request lock, not beforehand.
    pub fn answer_with_guard<G>(
        &self,
        id: &str,
        answer: &Answer,
        prepare: impl FnOnce() -> Result<G, ToolError>,
        persist: impl FnOnce(&G, &Request) -> Result<(), ToolError>,
    ) -> Result<(), ToolError> {
        self.answer_inner_using(id, answer, true, prepare, persist, || {})
    }

    fn answer_inner(&self, id: &str, answer: &Answer, recorded: bool, persist: impl FnOnce(&Request) -> Result<(), ToolError>) -> Result<(), ToolError> {
        self.answer_inner_using(id, answer, recorded, || Ok(()), |_, request| persist(request), || {})
    }

    fn answer_inner_using<G>(
        &self,
        id: &str,
        answer: &Answer,
        recorded: bool,
        prepare: impl FnOnce() -> Result<G, ToolError>,
        persist: impl FnOnce(&G, &Request) -> Result<(), ToolError>,
        before_publish: impl FnOnce(),
    ) -> Result<(), ToolError> {
        let request_path = self.path(id, "request")?;
        let answer_path = self.path(id, "answer")?;
        let bytes = serde_json::to_vec(answer).map_err(|e| ToolError::internal(e.to_string()))?;
        // Requests are created once with unique ids; neither submitters nor waiters recreate
        // them. Keep this handle open until delivery, and recheck the path after taking its
        // lock: a waiter may have unlinked the request while this handle was waiting for it.
        // Exclusive locks are emulated as write locks on some network filesystems.
        let mut request_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&request_path)
            .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { no_longer_waiting() } else { ToolError::io(request_path.display(), e) })?;
        let started = Instant::now();
        loop {
            match request_file.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) if started.elapsed() < ANSWER_LOCK_WAIT => std::thread::sleep(LOCK_POLL),
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(ToolError::new(ErrorCode::TimedOut, "Another answer to this question is being saved.")
                        .with_remediation("Wait for that answer to finish, then retry if the question is still waiting."));
                }
                Err(std::fs::TryLockError::Error(e)) => return Err(ToolError::io(request_path.display(), e)),
            }
        }
        if !request_path.try_exists().map_err(|e| ToolError::io(request_path.display(), e))? {
            return Err(no_longer_waiting());
        }
        // Windows denies reads through a second handle to an exclusively locked byte range.
        // Read through the handle that owns this reservation, after rejecting an unlinked one.
        let mut request_bytes = Vec::new();
        request_file.read_to_end(&mut request_bytes).map_err(|e| ToolError::io(request_path.display(), e))?;
        if request_bytes.is_empty() {
            return Err(no_longer_waiting());
        }
        if self.delivery_state(id)? == DeliveryState::Delivered {
            return Err(no_longer_waiting());
        }
        let request: Request = serde_json::from_slice(&request_bytes).map_err(|_| {
            ToolError::new(ErrorCode::GarbledOutput, "This question could not be read.").with_remediation("Ask the builder to send a new request.")
        })?;
        if request.id != id {
            return Err(ToolError::invalid("The saved question does not match its inbox id."));
        }
        let guard = prepare()?;
        // Deletion can finish while preparation is waiting for the project lock. Reopening
        // the same path must not turn this old request handle into a reservation on a new one.
        let metadata = std::fs::symlink_metadata(&request_path)
            .map_err(|error| if error.kind() == std::io::ErrorKind::NotFound { no_longer_waiting() } else { ToolError::io(request_path.display(), error) })?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(no_longer_waiting());
        }
        let current = same_file::Handle::from_path(&request_path).map_err(|error| ToolError::io(request_path.display(), error))?;
        let reserved = request_file.try_clone().and_then(same_file::Handle::from_file).map_err(|error| ToolError::io(request_path.display(), error))?;
        if current != reserved {
            return Err(no_longer_waiting());
        }
        if recorded {
            delivery::stage(&self.dir, id)?;
        }
        persist(&guard, &request)?;
        before_publish();
        craft_anything_platform::fs::write_atomic_private(&answer_path, &bytes)
    }

    /// Compatibility wrapper for `try_wait`. None also includes a logged waiting or withdrawal
    /// error; callers that need to distinguish it from a withdrawn request use `try_wait`.
    pub fn wait(&self, id: &str, timeout: Duration, cancelled: &dyn Fn() -> bool) -> Option<Answer> {
        match self.try_wait(id, timeout, cancelled) {
            Ok(answer) => answer,
            Err(error) => {
                log::warn!("Couldn't wait for this question: {}", error.for_people());
                None
            }
        }
    }

    /// Wait for the answer to `id` (polling). Timeout and cancellation withdraw a request that
    /// has not yet been accepted. An accepted save/submission finishes first: an answer wins
    /// over withdrawal, while a failed submission leaves the waiter free to withdraw. A file
    /// lock is released when its holder closes or exits; this cannot interrupt blocked file I/O.
    /// If a request cannot be unlinked, a synced empty file records its withdrawal. A failure
    /// to remove or invalidate it is an error, never a claim that withdrawal succeeded.
    pub fn try_wait(&self, id: &str, timeout: Duration, cancelled: &dyn Fn() -> bool) -> Result<Option<Answer>, ToolError> {
        self.try_wait_with_directory_sync(id, timeout, cancelled, crate::feed::sync_directory)
    }

    fn try_wait_with_directory_sync(
        &self,
        id: &str,
        timeout: Duration,
        cancelled: &dyn Fn() -> bool,
        mut sync: impl FnMut(&Path) -> Result<(), ToolError>,
    ) -> Result<Option<Answer>, ToolError> {
        let (req, ans) = (self.path(id, "request")?, self.path(id, "answer")?);
        let start = Instant::now();
        loop {
            let request_file = match std::fs::OpenOptions::new().read(true).write(true).open(&req) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && !request_entry_exists(&req)? => return Ok(None),
                Err(error) => {
                    let mut error = ToolError::io(req.display(), error);
                    if error.code == ErrorCode::NotFound {
                        error.code = ErrorCode::FileError;
                    }
                    return Err(error);
                }
            };
            let delay = match request_file.try_lock() {
                Ok(()) => {
                    if !request_entry_exists(&req)? || request_file.metadata().map_err(|error| ToolError::io(req.display(), error))?.len() == 0 {
                        return Ok(None);
                    }
                    if let Some(answer) = read_answer(&ans)? {
                        // Never discard the last proof of delivery. Commit the receipt's
                        // directory entry before either unlink; otherwise retain both files
                        // while returning the actual answer. Pending views still recognize
                        // the retained answer as delivered.
                        match delivery::mark_delivered(&self.dir, id).and_then(|()| sync(&self.dir)) {
                            Ok(()) => {
                                let _ = std::fs::remove_file(&ans);
                                let _ = std::fs::remove_file(&req);
                            }
                            Err(error) => log::warn!("Couldn't retain this answer's delivery receipt: {}", error.message),
                        }
                        return Ok(Some(answer));
                    }
                    if cancelled() || start.elapsed() >= timeout {
                        withdraw_request(&req, &request_file)?;
                        return Ok(None);
                    }
                    Duration::from_millis(200)
                }
                Err(std::fs::TryLockError::WouldBlock) => LOCK_POLL,
                Err(std::fs::TryLockError::Error(error)) => return Err(ToolError::io(req.display(), error)),
            };
            drop(request_file);
            std::thread::sleep(delay);
        }
    }
}

/// The folder in the inbox where tools say what they are waiting on.
const ACTIVITY: &str = "activity";

/// A line a tool working on a project says while it waits (a Ghidra decompile, #1714), for the
/// run that shows the project's feed. Our MCP server runs inside the coding agent, so a tool's own
/// progress reaches only the agent; this file reaches the app. Removed when the wait ends.
pub struct Activity {
    file: PathBuf,
}

impl Activity {
    pub fn start(inbox: &Path) -> Activity {
        Activity { file: inbox.join(ACTIVITY).join(format!("{}-{}.txt", std::process::id(), next_counter())) }
    }

    /// Keep one notice per approval kind until the next build clears the activity folder.
    pub(crate) fn approval_refused(inbox: &Path, kind: crate::workspace::ActionKind) {
        let path = inbox.join(ACTIVITY).join(format!("approval-{}.txt", kind.name()));
        Self::write(
            &path,
            &format!(
                "The builder was refused permission for {}. To ask before trying again, use Approvals or type /approvals {} ask.",
                kind.what(),
                kind.name()
            ),
        );
    }

    /// Say `line`, in place of what this wait said before.
    pub fn say(&self, line: &str) {
        Self::write(&self.file, line);
    }

    fn write(file: &Path, line: &str) {
        let written = match file.parent() {
            Some(dir) => std::fs::create_dir_all(dir).map_err(|e| e.to_string()).and_then(|()| write_atomic(file, line.as_bytes()).map_err(|e| e.message)),
            None => Ok(()),
        };
        if let Err(e) = written {
            log::warn!("Couldn't tell the app {line:?}: {e}");
        }
    }
}

impl Drop for Activity {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.file);
    }
}

/// What the tools working on the project are saying now: (which wait, its line), oldest first.
pub fn activity(inbox: &Path) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(inbox.join(ACTIVITY)) else { return Vec::new() };
    let mut lines: Vec<(String, String)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_str()?.strip_suffix(".txt")?.to_string();
            let line = std::fs::read_to_string(e.path()).ok()?;
            Some((name, line.trim().to_string())).filter(|(_, l)| !l.is_empty())
        })
        .collect();
    lines.sort();
    lines
}

/// Forget what tools said in an earlier run: a tool that was stopped mid-wait left its line.
pub fn clear_activity(inbox: &Path) {
    let _ = std::fs::remove_dir_all(inbox.join(ACTIVITY));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_waits_for_the_conversations_directory_sync() {
        use crate::workspace::{TargetRef, Workspace};
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Colour", "Pick a colour", TargetRef::None).unwrap();
        let inbox = Inbox::new(ws.inbox_dir());
        let id = inbox.post(RequestKind::Question, "Which colour?", "", vec![], true, None).unwrap();
        let answer = Answer { text: Some("Blue".into()), ..Default::default() };
        let prepare = crate::feed::load(&ws).prepare_transaction(&ws.root);
        let error = inbox
            .answer_with_guard(&id, &answer, prepare, |guard, request| {
                let mut feed = crate::feed::load(&ws);
                crate::feed::record_pending_answer(&mut feed, request, "Blue");
                crate::testing::save_feed_with_directory_sync(guard, &mut feed, false, |_| {
                    Err(ToolError::new(ErrorCode::FileError, "injected directory sync failure"))
                })
            })
            .unwrap_err();

        assert!(error.message.contains("injected directory sync failure"));
        assert!(!inbox.dir().join(format!("{id}.answer.json")).exists());
        assert_eq!(inbox.pending().len(), 1);
        assert!(crate::feed::load(&ws).iter().any(|item| matches!(item, crate::feed::FeedItem::Ask { answer: Some(answer), .. } if answer == "Blue")));
        inbox.answer_recorded(&ws, &id, &answer, "Blue").unwrap();
        assert_eq!(inbox.wait(&id, Duration::ZERO, &|| false).unwrap().as_text(), "Blue");
        assert_eq!(crate::feed::load(&ws).iter().filter(|item| matches!(item, crate::feed::FeedItem::Ask { request, .. } if request.id == id)).count(), 1);
    }

    #[test]
    fn a_recorded_answer_holds_the_project_lock_through_publication() {
        use crate::workspace::{TargetRef, Workspace};
        use std::sync::mpsc;
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Colour", "Pick a colour", TargetRef::None).unwrap();
        let deleting = Workspace::open(&ws.root).unwrap();
        let inbox = Inbox::new(ws.inbox_dir());
        let id = inbox.post(RequestKind::Question, "Which colour?", "", vec![], true, None).unwrap();
        let answer = Answer { text: Some("Blue".into()), ..Default::default() };
        let (release, waiting) = mpsc::channel();
        let (started, entering) = mpsc::channel();
        let (finished, completed) = mpsc::channel();
        std::thread::scope(|scope| {
            let deletion = scope.spawn(move || {
                waiting.recv_timeout(Duration::from_secs(5)).unwrap();
                started.send(()).unwrap();
                let result = deleting.delete();
                finished.send(()).unwrap();
                result
            });
            let prepare = crate::feed::load(&ws).prepare_transaction(&ws.root);
            inbox
                .answer_inner_using(
                    &id,
                    &answer,
                    true,
                    prepare,
                    |guard, request| crate::feed::record_answer_locked(guard, &ws, request, "Blue"),
                    || {
                        let probe = std::fs::OpenOptions::new().read(true).write(true).open(inbox.dir().join("feed.lock")).unwrap();
                        assert!(
                            matches!(probe.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
                            "the project guard must remain held after persistence returns"
                        );
                        release.send(()).unwrap();
                        entering.recv_timeout(Duration::from_secs(5)).unwrap();
                        assert!(matches!(completed.recv_timeout(Duration::from_millis(100)), Err(mpsc::RecvTimeoutError::Timeout)));
                        assert!(ws.root.join(crate::workspace::MANIFEST).is_file());
                        assert!(!inbox.dir().join(format!("{id}.answer.json")).exists());
                    },
                )
                .unwrap();
            deletion.join().unwrap().unwrap();
        });
        assert!(!ws.root.join(crate::workspace::MANIFEST).exists());
        assert!(!inbox.dir().exists(), "publication cannot recreate inbox payloads after deletion completes");
    }

    #[test]
    fn an_answer_reserved_before_deletion_cannot_recreate_delivery_records() {
        use crate::workspace::{TargetRef, Workspace};
        for recreate in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let ws = Workspace::create(directory.path(), "Colour", "Pick a colour", TargetRef::None).unwrap();
            let inbox = Inbox::new(ws.inbox_dir());
            let id = inbox.post(RequestKind::Question, "Which colour?", "", vec![], true, None).unwrap();
            let prepare = crate::feed::load(&ws).prepare_transaction(&ws.root);
            let answer = Answer { text: Some("Blue".into()), ..Default::default() };
            let result = inbox.answer_with_guard(
                &id,
                &answer,
                || {
                    Workspace::open(&ws.root)?.delete()?;
                    if recreate {
                        Workspace::create_at(&ws.root, "Replacement", "Keep this new project", TargetRef::None, None)?;
                    }
                    prepare()
                },
                |_, _| panic!("a deleted or replaced project cannot persist the old reply"),
            );

            assert!(result.is_err());
            assert!(!inbox.dir().join(format!("{id}.delivery.json")).exists());
            assert!(!inbox.dir().join(format!("{id}.answer.json")).exists());
            assert!(!ws.root.join(crate::feed::FEED_FILE).exists());
            if recreate {
                assert_eq!(Workspace::open(&ws.root).unwrap().manifest.request, "Keep this new project");
            } else {
                assert!(!ws.root.exists());
            }
        }
    }

    #[test]
    fn a_replaced_request_is_rejected_before_staging() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(directory.path().join("inbox"));
        let id = inbox.post(RequestKind::Question, "Which colour?", "", vec![], true, None).unwrap();
        let request_path = inbox.dir().join(format!("{id}.request.json"));
        let original = std::fs::read(&request_path).unwrap();
        let result = inbox.answer_with_guard(
            &id,
            &Answer { text: Some("Blue".into()), ..Default::default() },
            || {
                std::fs::remove_file(&request_path).unwrap();
                std::fs::write(&request_path, &original).unwrap();
                Ok(())
            },
            |_, _| panic!("the reservation belongs to the old file, even when its bytes match"),
        );

        assert_eq!(result.unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(std::fs::read(&request_path).unwrap(), original);
        assert!(!inbox.dir().join(format!("{id}.delivery.json")).exists());
        assert!(!inbox.dir().join(format!("{id}.answer.json")).exists());
    }

    #[test]
    fn a_tool_says_what_it_waits_on_until_it_stops() {
        let d = tempfile::tempdir().unwrap();
        assert!(activity(d.path()).is_empty(), "no folder, nothing said");
        let first = Activity::start(d.path());
        first.say("Ghidra is decompiling this function (1 min so far; it gives up at 15 min)");
        first.say("Ghidra is decompiling this function (2 min so far; it gives up at 15 min)");
        let second = Activity::start(d.path());
        second.say("Waiting for Ghidra to finish decompiling another function (1 min so far)");
        let now = activity(d.path());
        assert_eq!(now.len(), 2, "{now:?}");
        assert!(now.iter().any(|(_, l)| l.starts_with("Ghidra is decompiling this function (2 min")), "the latest line replaces the last");
        drop(first);
        let now = activity(d.path());
        assert_eq!(now.len(), 1);
        assert!(now[0].1.starts_with("Waiting for Ghidra"));
        std::mem::forget(second);
        clear_activity(d.path());
        assert!(activity(d.path()).is_empty(), "a wait cut off mid-line is forgotten by the next run");
    }

    #[test]
    fn question_round_trip_across_handles() {
        let dir = tempfile::tempdir().unwrap();
        let asker = Inbox::new(dir.path().join("inbox"));
        let app = Inbox::new(dir.path().join("inbox"));
        let id = asker.post(RequestKind::Question, "Which colour?", "", vec!["Blue".into(), "Red".into()], true, None).unwrap();
        let pending = app.pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].choices, vec!["Blue", "Red"]);
        app.answer(&id, &Answer { choice: Some("Red".into()), text: Some(" darker ".into()), ..Default::default() }).unwrap();
        assert!(app.pending().is_empty(), "answered requests are no longer pending");
        let a = asker.wait(&id, Duration::from_secs(2), &|| false).unwrap();
        assert_eq!(a.as_text(), "Red — darker");
        assert_eq!(std::fs::read_dir(dir.path().join("inbox")).unwrap().count(), 0, "both files are cleaned up");
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_answer_links_stop_before_persistence() {
        use std::os::unix::fs::symlink;
        for kind in ["loop", "missing", "directory"] {
            let directory = tempfile::tempdir().unwrap();
            let inbox = Inbox::new(directory.path());
            let id = inbox.post(RequestKind::Approval, "Run the check?", "", vec![], false, None).unwrap();
            let request_path = inbox.path(&id, "request").unwrap();
            let request_bytes = std::fs::read(&request_path).unwrap();
            let answer_path = inbox.path(&id, "answer").unwrap();
            let target = if kind == "loop" { answer_path.clone() } else { directory.path().join(kind) };
            if kind == "directory" {
                std::fs::create_dir(&target).unwrap();
            }
            symlink(&target, &answer_path).unwrap();
            let mut persisted = false;
            let result = inbox.answer_with(&id, &Answer { allow: Some(true), ..Default::default() }, |_| {
                persisted = true;
                Ok(())
            });
            assert!(!persisted, "an unreadable {kind} answer must stop before saving another reply");
            let error = result.unwrap_err();
            assert_ne!(error.code, ErrorCode::NotFound, "an existing unreadable answer is not a withdrawn request");
            assert!(error.remediation.is_some());
            assert_eq!(std::fs::read_link(&answer_path).unwrap(), target, "the original entry is retained");
            assert_eq!(std::fs::read(&request_path).unwrap(), request_bytes);
            assert!(!inbox.dir().join(format!("{id}.delivery.json")).exists(), "no new submission was staged");
        }
    }

    #[test]
    fn missing_and_malformed_answers_can_still_be_retried() {
        for previous in [None, Some(b"not an answer".as_slice())] {
            let directory = tempfile::tempdir().unwrap();
            let inbox = Inbox::new(directory.path());
            let id = inbox.post(RequestKind::Question, "Which colour?", "", vec![], true, None).unwrap();
            if let Some(bytes) = previous {
                std::fs::write(inbox.path(&id, "answer").unwrap(), bytes).unwrap();
            }
            let answer = Answer { text: Some("Blue".into()), ..Default::default() };
            let mut persisted = 0;
            inbox
                .answer_with(&id, &answer, |_| {
                    persisted += 1;
                    Ok(())
                })
                .unwrap();
            assert_eq!(persisted, 1);
            assert_eq!(inbox.wait(&id, Duration::ZERO, &|| false), Some(answer));
        }
    }

    #[cfg(unix)]
    #[test]
    fn readable_answer_symlinks_keep_the_existing_decision() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(directory.path());
        let id = inbox.post(RequestKind::Approval, "Run the check?", "", vec![], false, None).unwrap();
        let target = directory.path().join("saved-decision.json");
        let answer = Answer { allow: Some(false), ..Default::default() };
        let bytes = serde_json::to_vec(&answer).unwrap();
        std::fs::write(&target, &bytes).unwrap();
        let path = inbox.path(&id, "answer").unwrap();
        symlink(&target, &path).unwrap();
        let mut persisted = false;
        let error = inbox
            .answer_with(&id, &Answer { allow: Some(true), ..Default::default() }, |_| {
                persisted = true;
                Ok(())
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);
        assert!(!persisted);
        assert_eq!(std::fs::read_link(&path).unwrap(), target);
        assert_eq!(inbox.wait(&id, Duration::ZERO, &|| false), Some(answer));
        assert_eq!(std::fs::read(&target).unwrap(), bytes, "consuming the link leaves the saved target intact");
    }

    #[test]
    fn timeout_withdraws_and_bad_ids_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path());
        let id = inbox.post(RequestKind::Approval, "Run npm install", "", vec![], false, Some("bash".into())).unwrap();
        assert!(inbox.wait(&id, Duration::from_millis(50), &|| false).is_none());
        assert!(inbox.pending().is_empty());
        assert!(inbox.answer("../escape", &Answer::default()).is_err());
        assert!(inbox.answer("A", &Answer::default()).is_err());
    }

    #[test]
    fn an_expired_request_cannot_be_answered() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path());
        let id = inbox.post(RequestKind::Question, "Which colour?", "", vec![], true, None).unwrap();
        assert!(inbox.wait(&id, Duration::ZERO, &|| false).is_none());
        let error = inbox.answer(&id, &Answer { text: Some("Blue".into()), ..Default::default() }).unwrap_err();
        assert_eq!(error.code, craft_anything_evidence::ErrorCode::NotFound);
        assert!(!dir.path().join(format!("{id}.answer.json")).exists());
    }

    /// Guarded child fixture: normal test discovery must not touch a real inbox.
    #[test]
    fn inbox_answer_child() {
        let Ok(directory) = std::env::var("CRAFT_TEST_INBOX_DIR") else { return };
        let id = std::env::var("CRAFT_TEST_INBOX_ID").unwrap();
        let label = std::env::var("CRAFT_TEST_INBOX_LABEL").unwrap();
        let directory = PathBuf::from(directory);
        std::fs::write(directory.join(format!("{label}.ready")), b"ready").unwrap();
        let started = Instant::now();
        while !directory.join("go").exists() && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(directory.join("go").exists(), "the parent releases the submitters");
        let result = Inbox::new(directory.join("inbox")).answer(&id, &Answer { text: Some(label.clone()), ..Default::default() });
        let outcome = match result {
            Ok(()) => "ok",
            Err(error) if error.code == craft_anything_evidence::ErrorCode::NotFound => "expired",
            Err(error) => panic!("{error:?}"),
        };
        std::fs::write(directory.join(format!("{label}.result")), outcome).unwrap();
    }

    #[test]
    fn only_one_process_can_submit_an_answer() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(directory.path().join("inbox"));
        let id = inbox.post(RequestKind::Question, "Which colour?", "", vec![], true, None).unwrap();
        let children: Vec<_> = ["blue", "red"]
            .into_iter()
            .map(|label| {
                let root = directory.path().to_path_buf();
                let id = id.clone();
                std::thread::spawn(move || {
                    craft_anything_platform::Process::new(std::env::current_exe().unwrap())
                        .args(["--exact", "inbox::tests::inbox_answer_child", "--test-threads=1"])
                        .env("CRAFT_TEST_INBOX_DIR", &root)
                        .env("CRAFT_TEST_INBOX_ID", &id)
                        .env("CRAFT_TEST_INBOX_LABEL", label)
                        .timeout(Duration::from_secs(10))
                        .stdout_cap(64 * 1024)
                        .stderr_cap(64 * 1024)
                        .run_checked()
                })
            })
            .collect();
        let started = Instant::now();
        let ready = || ["blue", "red"].into_iter().all(|label| directory.path().join(format!("{label}.ready")).exists());
        while !ready() && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        let both_ready = ready();
        std::fs::write(directory.path().join("go"), b"go").unwrap();
        for child in children {
            child.join().unwrap().unwrap();
        }
        assert!(both_ready, "both processes reached the same submission race");
        let outcomes: Vec<_> =
            ["blue", "red"].into_iter().map(|label| (label, std::fs::read_to_string(directory.path().join(format!("{label}.result"))).unwrap())).collect();
        assert_eq!(outcomes.iter().filter(|(_, result)| result == "ok").count(), 1, "{outcomes:?}");
        assert_eq!(outcomes.iter().filter(|(_, result)| result == "expired").count(), 1, "{outcomes:?}");
        let winner = outcomes.iter().find(|(_, result)| result == "ok").unwrap().0;
        assert_eq!(inbox.wait(&id, Duration::ZERO, &|| false).unwrap().as_text(), winner);
        assert_eq!(std::fs::read_dir(inbox.dir()).unwrap().count(), 0);
    }

    #[test]
    fn withdrawal_prevents_persistence_and_delivery() {
        for cancelled in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let inbox = Inbox::new(directory.path());
            let id = inbox.post(RequestKind::Question, "Which colour?", "", vec![], true, None).unwrap();
            assert!(inbox.wait(&id, Duration::ZERO, &|| cancelled).is_none());
            let mut saved = false;
            let error = inbox
                .answer_with(&id, &Answer::default(), |_| {
                    saved = true;
                    Ok(())
                })
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::NotFound);
            assert!(!saved, "an expired request must not stage an answer in the conversation");
            assert!(!directory.path().join(format!("{id}.answer.json")).exists());
        }
    }

    #[cfg(unix)]
    fn assert_no_late_submission_after_unlink_failure(cancelled: bool) {
        use std::os::unix::fs::PermissionsExt as _;
        struct RestorePermissions {
            path: PathBuf,
            permissions: std::fs::Permissions,
        }
        impl Drop for RestorePermissions {
            fn drop(&mut self) {
                let _ = std::fs::set_permissions(&self.path, self.permissions.clone());
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(directory.path().join("inbox"));
        let id = inbox.post(RequestKind::Approval, "Run the check?", "", vec![], false, None).unwrap();
        let request_path = inbox.path(&id, "request").unwrap();
        let probe = inbox.dir().join("unlink-probe");
        std::fs::write(&probe, b"check directory permissions").unwrap();
        let restore = RestorePermissions { path: inbox.dir().to_path_buf(), permissions: std::fs::metadata(inbox.dir()).unwrap().permissions() };
        std::fs::set_permissions(inbox.dir(), std::fs::Permissions::from_mode(0o500)).unwrap();
        match std::fs::remove_file(&probe) {
            Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied),
            Ok(()) => {
                eprintln!("permission test skipped: this process can unlink entries in a read-only directory");
                return;
            }
        }

        let timeout = if cancelled { Duration::from_secs(60) } else { Duration::ZERO };
        let answer = inbox.wait(&id, timeout, &|| cancelled);
        let request_remains = request_path.is_file();
        drop(restore);

        assert!(answer.is_none());
        assert!(request_remains, "the request could not be unlinked from the read-only directory");
        let reopened = Inbox::new(inbox.dir());
        let mut persisted = false;
        let result = reopened.answer_with(&id, &Answer { allow: Some(true), ..Default::default() }, |_| {
            persisted = true;
            Ok(())
        });
        assert!(!persisted, "restoring directory access must not resurrect a withdrawn approval");
        assert_eq!(result.unwrap_err().code, ErrorCode::NotFound);
        assert!(reopened.pending().is_empty());
        assert!(!inbox.path(&id, "answer").unwrap().exists());
        assert!(!inbox.dir().join(format!("{id}.delivery.json")).exists(), "withdrawal must not invent delivery evidence");
    }

    #[cfg(unix)]
    #[test]
    fn timeout_prevents_submission_after_unlink_failure() {
        assert_no_late_submission_after_unlink_failure(false);
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_prevents_submission_after_unlink_failure() {
        assert_no_late_submission_after_unlink_failure(true);
    }

    #[cfg(unix)]
    #[test]
    fn checked_wait_reports_existing_request_and_answer_read_errors() {
        use std::os::unix::fs::symlink;
        for (suffix, missing_target) in [("request", false), ("request", true), ("answer", false), ("answer", true)] {
            let directory = tempfile::tempdir().unwrap();
            let inbox = Inbox::new(directory.path());
            let id = inbox.post(RequestKind::Question, "Which colour?", "", vec![], true, None).unwrap();
            let path = inbox.path(&id, suffix).unwrap();
            if suffix == "request" {
                std::fs::remove_file(&path).unwrap();
            }
            let target = if missing_target { directory.path().join("missing-target") } else { path.clone() };
            symlink(&target, &path).unwrap();

            let result = inbox.try_wait(&id, Duration::ZERO, &|| true);

            assert!(result.is_err(), "an existing unreadable {suffix} is not successful withdrawal");
            assert_eq!(std::fs::read_link(&path).unwrap(), target);
            assert!(!inbox.dir().join(format!("{id}.delivery.json")).exists());
        }
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(Inbox::new(directory.path()).try_wait("absent", Duration::ZERO, &|| true).unwrap(), None);
    }

    #[cfg(unix)]
    #[test]
    fn failed_request_invalidation_is_reported_without_claiming_withdrawal() {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(directory.path());
        let id = inbox.post(RequestKind::Approval, "Run the check?", "", vec![], false, None).unwrap();
        let path = inbox.path(&id, "request").unwrap();
        let before = std::fs::read(&path).unwrap();
        let request = std::fs::File::open(&path).unwrap();
        request.try_lock().unwrap();
        let probe = directory.path().join("unlink-probe");
        std::fs::write(&probe, b"check directory permissions").unwrap();
        let permissions = std::fs::metadata(directory.path()).unwrap().permissions();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let denied = std::fs::remove_file(&probe);
        if denied.is_ok() {
            std::fs::set_permissions(directory.path(), permissions).unwrap();
            eprintln!("permission test skipped: this process can unlink entries in a read-only directory");
            return;
        }

        // A real read-only file handle makes invalidation fail too. The helper reports that
        // failure to try_wait instead of returning success or retrying indefinitely.
        let result = withdraw_request(&path, &request);
        std::fs::set_permissions(directory.path(), permissions).unwrap();

        assert_eq!(denied.unwrap_err().kind(), std::io::ErrorKind::PermissionDenied);
        let error = result.unwrap_err();
        assert!(error.message.contains("could not be withdrawn"));
        assert!(error.remediation.unwrap().contains("may still be waiting"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!inbox.path(&id, "answer").unwrap().exists());
        assert!(!inbox.dir().join(format!("{id}.delivery.json")).exists());
    }

    #[test]
    fn accepted_submission_finishes_before_timeout_or_cancellation() {
        for cancelled in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let inbox = Inbox::new(directory.path());
            let id = inbox.post(RequestKind::Question, "Which colour?", "", vec![], true, None).unwrap();
            let (accepted, accepting) = std::sync::mpsc::channel();
            let (release, finishing) = std::sync::mpsc::channel();
            std::thread::scope(|scope| {
                let (inbox, id) = (&inbox, &id);
                let submitting = scope.spawn(move || {
                    inbox.answer_with(id, &Answer { text: Some("Blue".into()), ..Default::default() }, |_| {
                        accepted.send(()).unwrap();
                        finishing.recv_timeout(Duration::from_secs(5)).unwrap();
                        Ok(())
                    })
                });
                accepting.recv_timeout(Duration::from_secs(5)).unwrap();
                let (finished, waiting) = std::sync::mpsc::channel();
                let waiter = scope.spawn(move || {
                    finished.send(inbox.wait(id, Duration::ZERO, &|| cancelled)).unwrap();
                });
                let premature = waiting.recv_timeout(Duration::from_millis(50));
                release.send(()).unwrap();
                submitting.join().unwrap().unwrap();
                waiter.join().unwrap();
                assert!(matches!(premature, Err(std::sync::mpsc::RecvTimeoutError::Timeout)), "an accepted save finishes before withdrawal");
                assert_eq!(waiting.recv_timeout(Duration::from_secs(5)).unwrap().unwrap().as_text(), "Blue");
            });
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1, "the delivered receipt survives request cleanup");
        }
    }

    #[test]
    fn a_failed_recorded_answer_can_be_retried() {
        use crate::workspace::{TargetRef, Workspace};
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Colour", "Pick a colour", TargetRef::None).unwrap();
        let inbox = Inbox::new(ws.inbox_dir());
        let id = inbox.post(RequestKind::Question, "Which colour?", "", vec![], true, None).unwrap();
        let path = ws.root.join(crate::feed::FEED_FILE);
        std::fs::create_dir(&path).unwrap();
        let answer = Answer { text: Some("Blue".into()), ..Default::default() };
        assert!(inbox.answer_recorded(&ws, &id, &answer, "Blue").is_err());
        assert_eq!(inbox.pending().len(), 1);
        assert!(!inbox.dir().join(format!("{id}.answer.json")).exists());
        std::fs::remove_dir(&path).unwrap();
        inbox.answer_recorded(&ws, &id, &answer, "Blue").unwrap();
        assert_eq!(inbox.wait(&id, Duration::ZERO, &|| false).unwrap().as_text(), "Blue");
        assert!(
            crate::feed::for_display(&ws)
                .iter()
                .any(|item| matches!(item, crate::feed::FeedItem::Ask { request, answer: Some(answer) } if request.id == id && answer == "Blue"))
        );
    }

    #[test]
    fn a_saved_reply_is_not_shown_as_delivered_after_its_request_expires() {
        use crate::feed::{self, FeedItem};
        use crate::workspace::{TargetRef, Workspace};
        for cancelled in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let ws = Workspace::create(directory.path(), "Colour", "Pick a colour", TargetRef::None).unwrap();
            let inbox = Inbox::new(ws.inbox_dir());
            let id = inbox.post(RequestKind::Question, "Which colour?", "Choose carefully", vec![], true, None).unwrap();
            let answer_path = inbox.path(&id, "answer").unwrap();
            std::fs::create_dir(&answer_path).unwrap();
            let answer = Answer { text: Some("Blue".into()), ..Default::default() };
            assert!(inbox.answer_recorded(&ws, &id, &answer, "Blue").is_err());
            assert!(feed::load(&ws).iter().any(|item| matches!(item, FeedItem::Ask { answer: Some(shown), .. } if shown == "Blue")));
            assert!(feed::for_display(&ws).iter().any(|item| matches!(item, FeedItem::Ask { request, answer: None } if request.id == id)));
            assert!(inbox.wait(&id, Duration::ZERO, &|| cancelled).is_none());
            let reopened = Workspace::open(&ws.root).unwrap();
            let display = feed::for_display(&reopened);
            assert!(
                !display.iter().any(|item| matches!(item, FeedItem::Ask { answer: Some(_), .. })),
                "an expired request cannot turn a staged reply into a delivered one"
            );
            assert!(display.iter().any(|item| matches!(item, FeedItem::Error { message, remediation: Some(help), .. } if message.contains("has not been delivered") && message.contains("Blue") && help.contains("new request"))), "keep the saved reply visible with an honest delivery state");
            assert!(
                feed::load(&reopened).iter().any(|item| matches!(item, FeedItem::Ask { answer: Some(shown), .. } if shown == "Blue")),
                "displaying the warning does not erase saved words"
            );
        }
    }

    #[test]
    fn a_retried_reply_stays_delivered_after_consumption_and_reopening() {
        use crate::feed::{self, FeedItem};
        use crate::workspace::{TargetRef, Workspace};
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Colour", "Pick a colour", TargetRef::None).unwrap();
        let inbox = Inbox::new(ws.inbox_dir());
        let id = inbox.post(RequestKind::Question, "Which colour?", "", vec![], true, None).unwrap();
        let request_path = inbox.path(&id, "request").unwrap();
        let request_bytes = std::fs::read(&request_path).unwrap();
        let answer_path = inbox.path(&id, "answer").unwrap();
        std::fs::create_dir(&answer_path).unwrap();
        let answer = Answer { text: Some("Blue".into()), ..Default::default() };
        assert!(inbox.answer_recorded(&ws, &id, &answer, "Blue").is_err());
        assert!(feed::for_display(&ws).iter().any(|item| matches!(item, FeedItem::Ask { request, answer: None } if request.detail.contains("Blue"))));
        std::fs::remove_dir(&answer_path).unwrap();
        inbox.answer_recorded(&ws, &id, &answer, "Blue").unwrap();
        assert_eq!(inbox.wait(&id, Duration::ZERO, &|| false).unwrap().as_text(), "Blue");
        assert!(!answer_path.exists());
        // A stale request left by failed cleanup must not offer an already delivered answer.
        std::fs::write(&request_path, request_bytes).unwrap();
        assert!(Inbox::new(ws.inbox_dir()).pending().is_empty());
        let reopened = Workspace::open(&ws.root).unwrap();
        assert!(
            feed::for_display(&reopened)
                .iter()
                .any(|item| matches!(item, FeedItem::Ask { request, answer: Some(shown) } if request.id == id && shown == "Blue"))
        );
        let mut persisted = false;
        assert_eq!(
            inbox
                .answer_with(&id, &answer, |_| {
                    persisted = true;
                    Ok(())
                })
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        assert!(!persisted);
    }

    #[test]
    fn a_failed_delivery_receipt_keeps_the_answer_as_durable_proof() {
        use crate::feed::{self, FeedItem};
        use crate::workspace::{TargetRef, Workspace};
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Colour", "Pick a colour", TargetRef::None).unwrap();
        let inbox = Inbox::new(ws.inbox_dir());
        let id = inbox.post(RequestKind::Question, "Which colour?", "", vec![], true, None).unwrap();
        let answer = Answer { text: Some("Blue".into()), ..Default::default() };
        inbox.answer_recorded(&ws, &id, &answer, "Blue").unwrap();
        let receipt = inbox.dir().join(format!("{id}.delivery.json"));
        std::fs::remove_file(&receipt).unwrap();
        std::fs::create_dir(&receipt).unwrap();
        assert_eq!(inbox.wait(&id, Duration::ZERO, &|| false).unwrap().as_text(), "Blue");
        assert!(inbox.path(&id, "answer").unwrap().is_file(), "the consumed answer is the remaining proof of delivery");
        assert!(inbox.pending().is_empty());
        assert!(
            feed::for_display(&Workspace::open(&ws.root).unwrap())
                .iter()
                .any(|item| matches!(item, FeedItem::Ask { answer: Some(shown), .. } if shown == "Blue"))
        );
    }

    #[test]
    fn delivery_sync_precedes_cleanup_and_failure_retains_both_files() {
        use crate::workspace::{TargetRef, Workspace};
        for fail_sync in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let ws = Workspace::create(directory.path(), "Colour", "Pick a colour", TargetRef::None).unwrap();
            let inbox = Inbox::new(ws.inbox_dir());
            let id = inbox.post(RequestKind::Question, "Which colour?", "", vec![], true, None).unwrap();
            let answer = Answer { text: Some("Blue".into()), ..Default::default() };
            inbox.answer_recorded(&ws, &id, &answer, "Blue").unwrap();
            let request_path = inbox.path(&id, "request").unwrap();
            let answer_path = inbox.path(&id, "answer").unwrap();
            let request_bytes = std::fs::read(&request_path).unwrap();
            let answer_bytes = std::fs::read(&answer_path).unwrap();
            let mut syncs = 0;

            let received = inbox
                .try_wait_with_directory_sync(&id, Duration::ZERO, &|| false, |dir| {
                    syncs += 1;
                    assert_eq!(dir, inbox.dir());
                    assert_eq!(delivery::state(dir, &id).unwrap(), DeliveryState::Delivered);
                    assert!(request_path.exists(), "sync must precede request cleanup");
                    assert_eq!(std::fs::read(&answer_path).unwrap(), answer_bytes, "sync must precede answer cleanup");
                    if fail_sync {
                        Err(ToolError::new(ErrorCode::FileError, "injected delivery directory sync failure"))
                    } else {
                        crate::feed::sync_directory(dir)
                    }
                })
                .unwrap();

            assert_eq!(received, Some(answer.clone()));
            assert_eq!(syncs, 1, "receipt durability is checked before either unlink");
            assert!(inbox.pending().is_empty(), "a retained accepted answer must not be offered for resubmission");
            if fail_sync {
                assert_eq!(std::fs::read(&request_path).unwrap(), request_bytes);
                assert_eq!(std::fs::read(&answer_path).unwrap(), answer_bytes);
                assert_eq!(inbox.answer_recorded(&ws, &id, &answer, "Blue").unwrap_err().code, ErrorCode::NotFound);
                assert_eq!(inbox.try_wait(&id, Duration::ZERO, &|| false).unwrap(), Some(answer));
            }
            assert!(!request_path.exists());
            assert!(!answer_path.exists());
            assert_eq!(delivery::state(inbox.dir(), &id).unwrap(), DeliveryState::Delivered);
        }
    }

    /// A request can carry the person's own words and an answer can carry a typed reply or an
    /// approval; another local account must not be able to read either from the project folder.
    #[cfg(unix)]
    #[test]
    fn requests_and_answers_are_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path());
        let id = inbox.post(RequestKind::Question, "Which folder?", "the one on the desktop", vec![], true, None).unwrap();
        let request = dir.path().join(format!("{id}.request.json"));
        assert_eq!(std::fs::metadata(&request).unwrap().permissions().mode() & 0o777, 0o600, "the request is readable only by its owner");
        inbox.answer(&id, &Answer { text: Some("the blue one".into()), ..Default::default() }).unwrap();
        let answer = dir.path().join(format!("{id}.answer.json"));
        assert_eq!(std::fs::metadata(&answer).unwrap().permissions().mode() & 0o777, 0o600, "the answer is readable only by its owner");
    }
}
