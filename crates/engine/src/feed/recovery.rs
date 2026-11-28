//! Explicit recovery copies of a run's visible conversation. These files are never imported,
//! replaced or removed automatically: retaining one does not claim the main feed was saved.

use super::{FeedItem, LoadedFeed};
use craft_anything_evidence::{ErrorCode, ToolError, sha256_hex};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const RECOVERY_ARCHIVE_DIR: &str = "feed-recovery";
const FORMAT: &str = "craft-anything.feed-recovery";
const VERSION: u32 = 1;
const SUFFIX: &str = ".recovery.json";
const STAGING_DIR: &str = "pending";
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 256;
const MAX_NOTICES: usize = 16;
const NOTICE_MESSAGE: &str = "An earlier run kept a separate conversation recovery copy.";
const NOTICE_REMEDIATION: &str = "Open the copy to inspect its conversation. It has not been imported automatically, and may overlap the saved conversation.";

/// Version 1: the complete folded conversation, its loaded source's identity, and the
/// caller's recovery context. `metadata` contains `format`, `source` and `details`; source
/// records a decoded/damaged SHA-256 or an unreadable/missing state, never damaged bytes.
/// Details contain only information already kept in the conversation or manifest, not raw
/// tool events. Unknown versions and invalid records are left alone, not imported or removed.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverySnapshot {
    pub version: u32,
    pub items: Vec<FeedItem>,
    pub metadata: Value,
    #[serde(skip)]
    project: Option<Result<std::sync::Arc<super::transaction::ProjectIdentity>, ToolError>>,
}

impl RecoverySnapshot {
    pub fn from_loaded(feed: &LoadedFeed, details: Value) -> Self {
        let source = if let Some(digest) = &feed.decoded_source {
            json!({"state": "decoded", "sha256": digest})
        } else if let Some(bytes) = &feed.damaged {
            json!({"state": "damaged", "sha256": sha256_hex(bytes)})
        } else if let Some(error) = &feed.not_loaded {
            json!({"state": "unreadable", "error_code": format!("{:?}", error.code)})
        } else {
            json!({"state": "missing"})
        };
        Self {
            version: VERSION,
            items: feed.to_vec(),
            metadata: json!({"format": FORMAT, "source": source, "details": details}),
            project: Some(feed.project.clone()),
        }
    }

    fn recognized(&self) -> bool {
        let valid_source = self.metadata.get("source").is_some_and(|source| match source.get("state").and_then(Value::as_str) {
            Some("decoded" | "damaged") => source
                .get("sha256")
                .and_then(Value::as_str)
                .is_some_and(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))),
            Some("unreadable") => source.get("error_code").and_then(Value::as_str).is_some_and(|code| !code.is_empty()),
            Some("missing") => true,
            _ => false,
        });
        self.version == VERSION
            && self.metadata.get("format").and_then(Value::as_str) == Some(FORMAT)
            && self.metadata.get("details").is_some_and(Value::is_object)
            && valid_source
    }
}

fn directory(root: &Path, create: bool) -> Result<Option<PathBuf>, ToolError> {
    directory_using(root, create, sync_directory)
}

fn directory_using(root: &Path, create: bool, mut sync: impl FnMut(&Path) -> Result<(), ToolError>) -> Result<Option<PathBuf>, ToolError> {
    let mut path = root.to_path_buf();
    for name in ["inbox", RECOVERY_ARCHIVE_DIR] {
        let parent = path.clone();
        path.push(name);
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !create => return Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let builder = std::fs::DirBuilder::new();
                #[cfg(unix)]
                let mut builder = builder;
                #[cfg(unix)]
                std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
                match builder.create(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(ToolError::io(path.display(), error)),
                }
                std::fs::symlink_metadata(&path).map_err(|error| ToolError::io(path.display(), error))?
            }
            Err(error) => return Err(ToolError::io(path.display(), error)),
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(ToolError::invalid(format!("{} isn't a regular conversation-recovery folder.", path.display())));
        }
        if create {
            // A previous attempt may have created this entry and failed before its parent was synced.
            sync(&parent)?;
        }
    }
    Ok(Some(path))
}

#[cfg(unix)]
pub(crate) fn sync_directory(path: &Path) -> Result<(), ToolError> {
    std::fs::File::open(path).and_then(|file| file.sync_all()).map_err(|error| ToolError::io(path.display(), error))
}

#[cfg(not(unix))]
pub(crate) fn sync_directory(_path: &Path) -> Result<(), ToolError> {
    Ok(())
}

struct CappedWriter<'a> {
    file: &'a mut std::fs::File,
    remaining: u64,
}

impl std::io::Write for CappedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let length = u64::try_from(bytes.len()).map_err(|_| std::io::Error::other("The recovery copy is too large."))?;
        if length > self.remaining {
            return Err(std::io::Error::other("The conversation recovery copy exceeds 64 MiB."));
        }
        let written = self.file.write(bytes)?;
        self.remaining = self.remaining.saturating_sub(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

/// Keep a distinct owner-private copy, bounded to 64 MiB. Success means the complete file was
/// synced, and on Unix its directory and ancestor directory entries were synced too. Even a
/// partial failed write is left alone in private staging, outside discovery's file/byte budgets.
/// Never touches the main feed.
pub fn write_recovery_snapshot(root: &Path, snapshot: &RecoverySnapshot) -> Result<PathBuf, ToolError> {
    write_recovery_snapshot_report(root, snapshot).map(archive_path)
}

/// Publication succeeded even when removing its extra staging link did not. The path always
/// names the complete synced archive; cleanup warnings must not hide that saved copy.
#[derive(Debug)]
pub struct RecoveryWrite {
    pub path: PathBuf,
    pub cleanup_warning: Option<ToolError>,
}

pub fn write_recovery_snapshot_report(root: &Path, snapshot: &RecoverySnapshot) -> Result<RecoveryWrite, ToolError> {
    write_recovery_snapshot_report_using(root, snapshot, sync_directory, MAX_BYTES, sync_file, remove_staging)
}

fn archive_path(report: RecoveryWrite) -> PathBuf {
    if let Some(warning) = report.cleanup_warning {
        log::warn!("Conversation recovery copy kept at {}: {}", report.path.display(), warning.for_people());
    }
    report.path
}

#[cfg(all(test, unix))]
fn write_recovery_snapshot_using(root: &Path, snapshot: &RecoverySnapshot, sync: impl FnMut(&Path) -> Result<(), ToolError>) -> Result<PathBuf, ToolError> {
    write_recovery_snapshot_with_limit(root, snapshot, sync, MAX_BYTES)
}

#[cfg(test)]
fn write_recovery_snapshot_with_limit(
    root: &Path,
    snapshot: &RecoverySnapshot,
    sync: impl FnMut(&Path) -> Result<(), ToolError>,
    limit: u64,
) -> Result<PathBuf, ToolError> {
    write_recovery_snapshot_report_using(root, snapshot, sync, limit, sync_file, remove_staging).map(archive_path)
}

fn sync_file(file: &std::fs::File, path: &Path) -> Result<(), ToolError> {
    file.sync_all().map_err(|error| ToolError::io(path.display(), error))
}

fn remove_staging(path: &Path) -> Result<(), ToolError> {
    std::fs::remove_file(path).map_err(|error| ToolError::io(path.display(), error))
}

fn write_recovery_snapshot_report_using(
    root: &Path,
    snapshot: &RecoverySnapshot,
    mut sync: impl FnMut(&Path) -> Result<(), ToolError>,
    limit: u64,
    mut sync_file: impl FnMut(&std::fs::File, &Path) -> Result<(), ToolError>,
    mut remove_staging: impl FnMut(&Path) -> Result<(), ToolError>,
) -> Result<RecoveryWrite, ToolError> {
    if !snapshot.recognized() {
        return Err(ToolError::new(ErrorCode::NotSupported, "This conversation recovery-copy format isn't supported."));
    }
    let _guard = super::lock_project(root)?;
    if let Some(project) = &snapshot.project {
        project.as_ref().map_err(Clone::clone)?.check(root)?;
    }
    let directory = directory_using(root, true, &mut sync)?.ok_or_else(|| ToolError::internal("The conversation-recovery folder wasn't created."))?;
    let staging = staging_directory(&directory, &mut sync)?;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    for _ in 0..64 {
        let name = format!("conversation-{}-{}-{}{SUFFIX}", crate::inbox::now_ms(), std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        let path = directory.join(&name);
        let pending = staging.join(format!("{name}.partial"));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file = match options.open(&pending) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(ToolError::io(pending.display(), error)),
        };
        let identity = same_file::Handle::from_file(file).map_err(|error| ToolError::io(pending.display(), error))?;
        let mut published = false;
        let publication = (|| {
            let mut file = identity.as_file().try_clone().map_err(|error| ToolError::io(pending.display(), error))?;
            serde_json::to_writer(CappedWriter { file: &mut file, remaining: limit.min(MAX_BYTES) }, snapshot)
                .map_err(|error| ToolError::io(pending.display(), error.into()))?;
            sync_file(&file, &pending)?;
            if !super::backups::same_file_at(&pending, &identity)? {
                return Err(ToolError::new(ErrorCode::FileChanged, "The staged conversation copy changed before publication."));
            }
            // Expose only a complete synced file, without replacing an existing archive.
            // A collision after creating this partial is reported, not retried into more files.
            std::fs::hard_link(&pending, &path).map_err(|error| ToolError::io(path.display(), error))?;
            published = true;
            if !super::backups::same_file_at(&path, &identity)? {
                return Err(ToolError::new(ErrorCode::FileChanged, "The conversation recovery copy changed during publication."));
            }
            sync(&directory)
        })();
        if let Err(error) = publication {
            return Err(retained_write_error(error, &pending, published.then_some(path.as_path()), &identity, false));
        }
        let cleanup = (|| {
            if !super::backups::same_file_at(&pending, &identity)? {
                return Err(ToolError::new(ErrorCode::FileChanged, "The staging entry changed after the recovery archive was saved."));
            }
            remove_staging(&pending)?;
            sync(&staging)
        })();
        let cleanup_warning = cleanup.err().map(|error| retained_write_error(error, &pending, Some(&path), &identity, true));
        return Ok(RecoveryWrite { path, cleanup_warning });
    }
    Err(ToolError::new(ErrorCode::FileError, "Couldn't reserve a new conversation recovery copy.")
        .with_remediation("Keep the existing recovery copies and retry."))
}

fn retained_write_error(error: ToolError, pending: &Path, published: Option<&Path>, identity: &same_file::Handle, complete: bool) -> ToolError {
    let mut retained = Vec::new();
    let mut unverified = Vec::new();
    let mut staging_verified = false;
    for path in std::iter::once(pending).chain(published) {
        if super::backups::same_file_at(path, identity).unwrap_or(false) {
            staging_verified |= path == pending;
            retained.push(path.display().to_string());
        } else {
            unverified.push(path.display().to_string());
        }
    }
    let outcome = if let Some(path) = published.filter(|_| complete) {
        format!("The recovery archive was saved at {}; only staging cleanup at {} is unconfirmed.", path.display(), pending.display())
    } else {
        "This recovery write did not complete.".into()
    };
    let location = if retained.is_empty() {
        String::new()
    } else {
        format!(
            "This attempt's verified remaining file paths are: {}. Inspect them and save any useful conversation fragments elsewhere first.",
            retained.join(", ")
        )
    };
    let staging = if staging_verified {
        "To reclaim staging space, move only the inspected staging file out of pending; keep a completed archive. Recheck the file before moving it if anything else has changed it.".into()
    } else {
        format!(
            "The original staging entry at {} could not be verified and may now name another file. Do not remove it based on its name, or move it to reclaim staging space.",
            pending.display()
        )
    };
    let retry = if complete {
        "Do not repeat the archive merely because its staging cleanup failed."
    } else {
        "Retrying the same oversized conversation still exceeds the limit and can retain another partial file."
    };
    let previous = error.remediation.clone().unwrap_or_default();
    let cause = error.details.clone();
    error
        .with_remediation(format!("{previous} {outcome} {location} {staging} {retry}").trim().to_owned())
        .with_details(json!({"recovery_complete": complete, "retained_paths": retained, "unverified_paths": unverified, "expected_staging_path": pending.display().to_string(), "cause": cause}))
}

fn staging_directory(directory: &Path, sync: &mut impl FnMut(&Path) -> Result<(), ToolError>) -> Result<PathBuf, ToolError> {
    let staging = directory.join(STAGING_DIR);
    let builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    let mut builder = builder;
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    match builder.create(&staging) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(ToolError::io(staging.display(), error)),
    }
    let metadata = std::fs::symlink_metadata(&staging).map_err(|error| ToolError::io(staging.display(), error))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(ToolError::invalid(format!("{} isn't a regular conversation-recovery staging folder.", staging.display())));
    }
    sync(directory)?;
    Ok(staging)
}

fn recovery_filename(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with("conversation-") && name.ends_with(SUFFIX))
}

/// Earlier writers saved these display rows as errors. Match their complete generated shape,
/// including paths from before a project move, without removing ordinary errors with a log.
pub(super) fn is_recovery_notice(item: &FeedItem) -> bool {
    let FeedItem::Error { message, remediation: Some(remediation), log: Some(path) } = item else { return false };
    message == NOTICE_MESSAGE
        && remediation == NOTICE_REMEDIATION
        && recovery_filename(path)
        && path.parent().is_some_and(|directory| {
            directory.file_name().is_some_and(|name| name == RECOVERY_ARCHIVE_DIR)
                && directory.parent().and_then(Path::file_name).is_some_and(|name| name == "inbox")
        })
}

/// Read historical recovery notices for a display snapshot. Call when loading a view, not
/// while polling it, and keep the returned rows separate from the feed passed to `save`.
pub fn recovery_notices(root: &Path) -> Vec<FeedItem> {
    let mut items = Vec::new();
    append_recovery_notices(root, &mut items);
    items
}

fn read_snapshot(path: &Path, remaining: &mut u64) -> Option<RecoverySnapshot> {
    let before = std::fs::symlink_metadata(path).ok()?;
    if !before.is_file() || before.file_type().is_symlink() || before.len() > *remaining {
        return None;
    }
    let (mut file, opened) = craft_anything_platform::fs::open_regular_file(path).ok()?;
    // Reject observed path replacements, including a link installed after the directory walk.
    let current = std::fs::symlink_metadata(path).ok()?;
    if !current.is_file() || current.file_type().is_symlink() || current.len() != opened.len() || current.modified().ok() != opened.modified().ok() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if current.dev() != opened.dev() || current.ino() != opened.ino() {
            return None;
        }
    }
    let limit = (*remaining).min(MAX_BYTES);
    let mut bytes = Vec::new();
    let read = (&mut file).take(limit).read_to_end(&mut bytes);
    *remaining = remaining.saturating_sub(bytes.len() as u64);
    read.ok()?;
    let mut extra = [0u8; 1];
    if file.read(&mut extra).ok()? != 0 {
        return None;
    }
    let snapshot: RecoverySnapshot = serde_json::from_slice(&bytes).ok()?;
    snapshot.recognized().then_some(snapshot)
}

/// Discover at most 256 directory entries, 16 valid records and 64 MiB in total. This is a
/// historical notice, not a statement that today's conversation failed to save. No feed lock
/// is taken and no contents are replayed; callers keep the loaded source digest unchanged.
pub(super) fn append_recovery_notices(root: &Path, items: &mut Vec<FeedItem>) {
    let Ok(Some(directory)) = directory(root, false) else { return };
    let Ok(entries) = std::fs::read_dir(directory) else { return };
    let mut remaining = MAX_BYTES;
    let mut shown = 0;
    for entry in entries.take(MAX_ENTRIES).flatten() {
        if shown >= MAX_NOTICES || remaining == 0 {
            break;
        }
        let path = entry.path();
        if !recovery_filename(&path) {
            continue;
        }
        if read_snapshot(&path, &mut remaining).is_none() {
            continue;
        }
        shown += 1;
        if !items.iter().any(|item| matches!(item, FeedItem::Error { log: Some(saved), .. } if saved == &path)) {
            items.push(FeedItem::Error { message: NOTICE_MESSAGE.into(), remediation: Some(NOTICE_REMEDIATION.into()), log: Some(path) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::{FEED_FILE, FEED_VERSION, Saved, UserItem, for_display, load, record_answer, save};
    use crate::inbox::{Inbox, RequestKind};
    use crate::workspace::{TargetRef, Workspace};

    #[test]
    fn recovery_copies_are_private_distinct_and_preserve_the_loaded_source() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "A game", TargetRef::None).unwrap();
        let original = br#"{"version":5,"items":[{"User":"The saved conversation"}]}"#;
        std::fs::write(ws.root.join(FEED_FILE), original).unwrap();
        let mut feed = load(&ws);
        feed.push(FeedItem::User(UserItem::Text("The unsaved words".into())));
        let snapshot = RecoverySnapshot::from_loaded(&feed, json!({"pending_event_count": 1}));
        let first = write_recovery_snapshot(&ws.root, &snapshot).unwrap();
        let second = write_recovery_snapshot(&ws.root, &snapshot).unwrap();
        assert_ne!(first, second);
        let bytes = std::fs::read(&first).unwrap();
        let kept: RecoverySnapshot = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(kept.metadata["source"]["sha256"], sha256_hex(original));
        assert_eq!(kept.items, feed.to_vec());
        assert_eq!(std::fs::read(&second).unwrap(), bytes);
        assert_eq!(std::fs::read(ws.root.join(FEED_FILE)).unwrap(), original);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(&first).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let mut shown = Vec::new();
        append_recovery_notices(&ws.root, &mut shown);
        append_recovery_notices(&ws.root, &mut shown);
        assert_eq!(shown.len(), 2, "each exact archive path gets one historical notice");
        assert!(shown.iter().all(|item| matches!(item, FeedItem::Error { message, .. } if message.contains("earlier run"))));
    }

    #[test]
    fn recovery_discovery_leaves_unknown_damaged_oversized_and_linked_entries_alone() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "A game", TargetRef::None).unwrap();
        let mut snapshot = RecoverySnapshot::from_loaded(&load(&ws), json!({}));
        let valid = write_recovery_snapshot(&ws.root, &snapshot).unwrap();
        let directory = valid.parent().unwrap();
        let damaged = directory.join("conversation-damaged.recovery.json");
        std::fs::write(&damaged, b"Unrelated personal contents").unwrap();
        let unknown = directory.join("conversation-newer.recovery.json");
        snapshot.version = 2;
        let unknown_bytes = serde_json::to_vec(&snapshot).unwrap();
        std::fs::write(&unknown, &unknown_bytes).unwrap();
        assert_eq!(write_recovery_snapshot(&ws.root, &snapshot).unwrap_err().code, ErrorCode::NotSupported);
        let huge = directory.join("conversation-oversized.recovery.json");
        std::fs::File::create(&huge).unwrap().set_len(MAX_BYTES + 1).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&valid, directory.join("conversation-linked.recovery.json")).unwrap();
        let mut shown = Vec::new();
        append_recovery_notices(&ws.root, &mut shown);
        assert_eq!(shown.len(), 1);
        assert!(matches!(&shown[0], FeedItem::Error { log: Some(path), .. } if path == &valid));
        assert_eq!(std::fs::read(&damaged).unwrap(), b"Unrelated personal contents");
        assert_eq!(std::fs::read(&unknown).unwrap(), unknown_bytes);
        assert_eq!(std::fs::metadata(&huge).unwrap().len(), MAX_BYTES + 1);
        // Enforce the same serialization cap while writing, before reporting a kept copy.
        let capped = directory.join("partial");
        let mut file = std::fs::File::create(&capped).unwrap();
        assert!(std::io::Write::write_all(&mut CappedWriter { file: &mut file, remaining: 4 }, b"12345").is_err());
        assert_eq!(std::fs::metadata(capped).unwrap().len(), 0);
    }

    #[test]
    fn failed_archive_serialization_cannot_consume_discovery_budgets() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Project", "Test", TargetRef::None).unwrap();
        let mut feed = load(&ws);
        feed.push(FeedItem::User(UserItem::Text("x".repeat(4096))));
        let snapshot = RecoverySnapshot::from_loaded(&feed, json!({}));
        let error = write_recovery_snapshot_with_limit(&ws.root, &snapshot, sync_directory, 1024).unwrap_err();
        assert!(error.message.contains("exceeds 64 MiB"));
        let directory = ws.inbox_dir().join(RECOVERY_ARCHIVE_DIR);
        let pending = directory.join(STAGING_DIR);
        assert!(std::fs::read_dir(&directory).unwrap().all(|entry| !recovery_filename(&entry.unwrap().path())));
        let partial = std::fs::read_dir(&pending).unwrap().next().unwrap().unwrap().path();
        assert!(std::fs::metadata(&partial).unwrap().len() > 0);
        assert!(!recovery_filename(&partial));
        // A sparse failed payload fills the byte budget without a large test allocation.
        std::fs::OpenOptions::new().write(true).open(&partial).unwrap().set_len(MAX_BYTES).unwrap();
        for index in 0..=MAX_ENTRIES {
            std::fs::write(pending.join(format!("earlier-{index}.partial")), b"partial").unwrap();
        }
        let valid = write_recovery_snapshot(&ws.root, &RecoverySnapshot::from_loaded(&load(&ws), json!({}))).unwrap();
        let shown = recovery_notices(&ws.root);
        assert_eq!(shown.len(), 1, "private partial files use neither the archive entry budget nor its byte budget");
        assert!(matches!(&shown[0], FeedItem::Error { log: Some(path), .. } if path == &valid));
        assert_eq!(std::fs::metadata(partial).unwrap().len(), MAX_BYTES, "failed payloads remain available for inspection");
    }

    #[test]
    fn failed_writes_report_the_verified_partial_and_manual_reclamation() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Project", "Keep my conversation", TargetRef::None).unwrap();
        let mut feed = load(&ws);
        feed.push(FeedItem::User(UserItem::Text("Recover these words. ".repeat(256))));
        let snapshot = RecoverySnapshot::from_loaded(&feed, json!({}));
        let error = write_recovery_snapshot_with_limit(&ws.root, &snapshot, sync_directory, 1024).unwrap_err();
        let pending = ws.inbox_dir().join(RECOVERY_ARCHIVE_DIR).join(STAGING_DIR);
        let partial = std::fs::read_dir(&pending).unwrap().next().unwrap().unwrap().path();
        let bytes = std::fs::read(&partial).unwrap();
        assert!(!bytes.is_empty());
        assert_eq!(error.details.as_ref().unwrap()["retained_paths"], json!([partial.display().to_string()]));
        assert_eq!(error.details.as_ref().unwrap()["unverified_paths"], json!([]));
        assert_eq!(error.details.as_ref().unwrap()["recovery_complete"], false);
        let guidance = error.remediation.unwrap();
        assert!(guidance.contains(&partial.display().to_string()));
        assert!(guidance.contains("Inspect") && guidance.contains("save any useful") && guidance.contains("move only"));
        assert!(guidance.contains("Retrying the same oversized conversation"));

        // Model the explicitly described manual action, not automatic name-based cleanup.
        let inspected = d.path().join("inspected-conversation-fragment.json");
        std::fs::rename(&partial, &inspected).unwrap();
        assert_eq!(std::fs::read(inspected).unwrap(), bytes);
        assert_eq!(std::fs::read_dir(&pending).unwrap().count(), 0);
        assert!(write_recovery_snapshot(&ws.root, &RecoverySnapshot::from_loaded(&load(&ws), json!({}))).is_ok());
    }

    #[test]
    fn a_failed_write_never_authorizes_reclaiming_a_replaced_staging_path() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Project", "Keep my conversation", TargetRef::None).unwrap();
        let snapshot = RecoverySnapshot::from_loaded(&load(&ws), json!({}));
        let moved = d.path().join("retained-original-partial");
        let mut replaced = None;
        let error = write_recovery_snapshot_report_using(
            &ws.root,
            &snapshot,
            sync_directory,
            MAX_BYTES,
            |_, pending| {
                std::fs::rename(pending, &moved).unwrap();
                std::fs::write(pending, b"Keep this personal replacement").unwrap();
                replaced = Some(pending.to_path_buf());
                Err(ToolError::internal("injected original sync failure"))
            },
            remove_staging,
        )
        .unwrap_err();
        let replaced = replaced.unwrap();
        assert!(error.message.contains("injected original sync failure"));
        assert_eq!(error.details.as_ref().unwrap()["retained_paths"], json!([]));
        assert_eq!(error.details.as_ref().unwrap()["unverified_paths"], json!([replaced.display().to_string()]));
        assert!(error.remediation.unwrap().contains("Do not remove it based on its name"));
        assert_eq!(std::fs::read(replaced).unwrap(), b"Keep this personal replacement");
        assert!(moved.is_file());
        assert!(recovery_notices(&ws.root).is_empty());
    }

    #[test]
    fn a_completed_archive_does_not_authorize_moving_replaced_staging() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Project", "Keep my conversation", TargetRef::None).unwrap();
        let snapshot = RecoverySnapshot::from_loaded(&load(&ws), json!({}));
        let directory = ws.inbox_dir().join(RECOVERY_ARCHIVE_DIR);
        let moved = d.path().join("original-staging");
        let mut replaced = None;
        let report = write_recovery_snapshot_report_using(
            &ws.root,
            &snapshot,
            |path| {
                sync_directory(path)?;
                if path == directory && std::fs::read_dir(path).unwrap().any(|entry| recovery_filename(&entry.unwrap().path())) {
                    let pending = std::fs::read_dir(directory.join(STAGING_DIR)).unwrap().next().unwrap().unwrap().path();
                    std::fs::rename(&pending, &moved).unwrap();
                    std::fs::write(&pending, b"Keep this personal replacement").unwrap();
                    replaced = Some(pending);
                }
                Ok(())
            },
            MAX_BYTES,
            sync_file,
            |_| panic!("a replaced staging entry must not be removed"),
        )
        .unwrap();
        let replaced = replaced.unwrap();
        let warning = report.cleanup_warning.unwrap();
        assert_eq!(warning.details.as_ref().unwrap()["retained_paths"], json!([report.path.display().to_string()]));
        assert_eq!(warning.details.as_ref().unwrap()["unverified_paths"], json!([replaced.display().to_string()]));
        let remediation = warning.remediation.unwrap();
        assert!(remediation.contains(&report.path.display().to_string()));
        assert!(remediation.contains("Do not remove it based on its name"), "the text-only notice must protect unverified staging: {remediation}");
        assert!(!remediation.contains("move only the inspected staging file"));
        assert_eq!(std::fs::read(replaced).unwrap(), b"Keep this personal replacement");
        assert_eq!(std::fs::read(&report.path).unwrap(), std::fs::read(moved).unwrap());
    }

    #[test]
    fn a_published_archive_survives_staging_cleanup_errors() {
        for fail_unlink in [true, false] {
            let d = tempfile::tempdir().unwrap();
            let ws = Workspace::create(d.path(), "Project", "Keep my conversation", TargetRef::None).unwrap();
            let snapshot = RecoverySnapshot::from_loaded(&load(&ws), json!({}));
            let report = write_recovery_snapshot_report_using(
                &ws.root,
                &snapshot,
                |path| {
                    if !fail_unlink && path.file_name().is_some_and(|name| name == STAGING_DIR) {
                        Err(ToolError::internal("injected staging directory sync failure"))
                    } else {
                        sync_directory(path)
                    }
                },
                MAX_BYTES,
                sync_file,
                |path| {
                    if fail_unlink { Err(ToolError::internal("injected staging unlink failure")) } else { remove_staging(path) }
                },
            )
            .unwrap();
            let saved: RecoverySnapshot = serde_json::from_slice(&std::fs::read(&report.path).unwrap()).unwrap();
            assert_eq!(saved.items, snapshot.items);
            let warning = report.cleanup_warning.unwrap();
            assert_eq!(warning.details.as_ref().unwrap()["recovery_complete"], true);
            assert!(warning.remediation.unwrap().contains(&report.path.display().to_string()));
            assert!(recovery_notices(&ws.root).iter().any(|item| matches!(item, FeedItem::Error { log: Some(path), .. } if path == &report.path)));
            assert_eq!(std::fs::read_dir(ws.inbox_dir().join(RECOVERY_ARCHIVE_DIR).join(STAGING_DIR)).unwrap().count(), usize::from(fail_unlink));
        }
    }

    #[test]
    fn archive_file_and_publication_sync_failures_remain_errors_with_retained_paths() {
        for fail_file in [true, false] {
            let d = tempfile::tempdir().unwrap();
            let ws = Workspace::create(d.path(), "Project", "Keep my conversation", TargetRef::None).unwrap();
            let snapshot = RecoverySnapshot::from_loaded(&load(&ws), json!({}));
            let error = write_recovery_snapshot_report_using(
                &ws.root,
                &snapshot,
                |path| {
                    if !fail_file
                        && path.file_name().is_some_and(|name| name == RECOVERY_ARCHIVE_DIR)
                        && std::fs::read_dir(path).unwrap().any(|entry| recovery_filename(&entry.unwrap().path()))
                    {
                        Err(ToolError::internal("injected archive directory sync failure"))
                    } else {
                        sync_directory(path)
                    }
                },
                MAX_BYTES,
                |file, path| {
                    if fail_file { Err(ToolError::internal("injected archive file sync failure")) } else { sync_file(file, path) }
                },
                |_| panic!("cleanup cannot run before archive durability succeeds"),
            )
            .unwrap_err();
            assert!(error.message.contains("injected archive"));
            let details = error.details.unwrap();
            assert_eq!(details["recovery_complete"], false);
            assert_eq!(details["retained_paths"].as_array().unwrap().len(), if fail_file { 1 } else { 2 });
            assert_eq!(details["unverified_paths"], json!([]));
        }
    }

    #[cfg(unix)]
    #[test]
    fn archive_staging_does_not_follow_an_existing_link() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Project", "Test", TargetRef::None).unwrap();
        let archive = directory(&ws.root, true).unwrap().unwrap();
        let elsewhere = d.path().join("personal");
        std::fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, archive.join(STAGING_DIR)).unwrap();
        let snapshot = RecoverySnapshot::from_loaded(&load(&ws), json!({}));
        assert!(write_recovery_snapshot(&ws.root, &snapshot).is_err());
        assert!(std::fs::read_dir(elsewhere).unwrap().next().is_none());
        assert!(recovery_notices(&ws.root).is_empty());
    }

    #[test]
    fn recovery_copy_does_not_replace_an_existing_folder_entry_or_follow_a_link() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "A game", TargetRef::None).unwrap();
        let snapshot = RecoverySnapshot::from_loaded(&load(&ws), json!({}));
        std::fs::create_dir_all(ws.inbox_dir()).unwrap();
        let entry = ws.inbox_dir().join(RECOVERY_ARCHIVE_DIR);
        std::fs::write(&entry, b"Keep this existing file").unwrap();
        assert!(write_recovery_snapshot(&ws.root, &snapshot).is_err());
        assert_eq!(std::fs::read(&entry).unwrap(), b"Keep this existing file");
        #[cfg(unix)]
        {
            let external = d.path().join("elsewhere");
            std::fs::create_dir(&external).unwrap();
            std::fs::remove_file(&entry).unwrap();
            std::os::unix::fs::symlink(&external, &entry).unwrap();
            assert!(write_recovery_snapshot(&ws.root, &snapshot).is_err());
            assert_eq!(std::fs::read_dir(external).unwrap().count(), 0);
        }
    }

    #[test]
    fn answering_a_question_does_not_persist_recovery_notices() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "A game", TargetRef::None).unwrap();
        let snapshot = RecoverySnapshot::from_loaded(&load(&ws), json!({}));
        let archive = write_recovery_snapshot(&ws.root, &snapshot).unwrap();
        assert_eq!(for_display(&ws).iter().filter(|item| is_recovery_notice(item)).count(), 1);

        let inbox = Inbox::new(ws.inbox_dir());
        let id = inbox.post(RequestKind::Question, "Which colour?", "", vec![], true, None).unwrap();
        let request = inbox.pending().into_iter().find(|request| request.id == id).unwrap();
        record_answer(&ws, &request, "Blue").unwrap();
        let saved: Saved = serde_json::from_slice(&std::fs::read(ws.root.join(FEED_FILE)).unwrap()).unwrap();
        assert!(saved.items.iter().all(|item| !is_recovery_notice(item)));
        assert!(load(&ws).iter().all(|item| !is_recovery_notice(item)));
        assert!(saved.items.iter().any(|item| matches!(item, FeedItem::Ask { answer: Some(answer), .. } if answer == "Blue")));

        let retained = d.path().join("retained-conversation.json");
        std::fs::rename(&archive, &retained).unwrap();
        assert!(for_display(&ws).iter().all(|item| !is_recovery_notice(item)));
        assert_eq!(serde_json::from_slice::<RecoverySnapshot>(&std::fs::read(retained).unwrap()).unwrap().items, snapshot.items);
    }

    #[test]
    fn legacy_saved_notices_are_removed_without_losing_ordinary_errors() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "x", "A game", TargetRef::None).unwrap();
        let archive = ws.inbox_dir().join(RECOVERY_ARCHIVE_DIR).join("conversation-1-2-3.recovery.json");
        let notice = FeedItem::Error {
            message: NOTICE_MESSAGE.into(),
            remediation: Some(NOTICE_REMEDIATION.into()),
            log: Some(d.path().join("previous-project").join("inbox").join(RECOVERY_ARCHIVE_DIR).join("conversation-1-2-3.recovery.json")),
        };
        let expected = vec![
            FeedItem::User(UserItem::Text(NOTICE_MESSAGE.into())),
            FeedItem::Agent { text: NOTICE_MESSAGE.into(), streaming: false },
            FeedItem::Error { message: "The builder stopped.".into(), remediation: Some(NOTICE_REMEDIATION.into()), log: Some(archive.clone()) },
            FeedItem::Error { message: NOTICE_MESSAGE.into(), remediation: Some("Inspect the builder's failure.".into()), log: Some(archive.clone()) },
            FeedItem::Error { message: NOTICE_MESSAGE.into(), remediation: Some(NOTICE_REMEDIATION.into()), log: Some(ws.root.join("build.log")) },
            FeedItem::Error {
                message: NOTICE_MESSAGE.into(),
                remediation: Some(NOTICE_REMEDIATION.into()),
                log: Some(ws.inbox_dir().join(RECOVERY_ARCHIVE_DIR).join("personal.json")),
            },
        ];
        let mut old_items = vec![notice];
        old_items.extend(expected.clone());
        let old = Saved { version: FEED_VERSION, items: old_items, recovery: None };
        let path = ws.root.join(FEED_FILE);
        std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();

        let mut loaded = load(&ws);
        assert_eq!(loaded.to_vec(), expected);
        assert_eq!(for_display(&ws), expected);
        save(&ws.root, &mut loaded).unwrap();
        let saved: Saved = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(saved.items, expected);
    }

    #[cfg(unix)]
    #[test]
    fn recovery_creation_syncs_both_parent_entries_and_the_archive_directory() {
        let d = tempfile::tempdir().unwrap();
        let root = Workspace::create(d.path(), "Project", "Test", TargetRef::None).unwrap().root;
        let snapshot = RecoverySnapshot {
            version: VERSION,
            items: vec![],
            metadata: json!({"format": FORMAT, "source": {"state": "missing"}, "details": {}}),
            project: None,
        };
        let mut synced = Vec::new();
        let archive = write_recovery_snapshot_using(&root, &snapshot, |path| {
            synced.push(path.to_path_buf());
            sync_directory(path)
        })
        .unwrap();
        let directory = root.join("inbox").join(RECOVERY_ARCHIVE_DIR);
        assert_eq!(synced, vec![root.clone(), root.join("inbox"), directory.clone(), directory.clone(), directory.join(STAGING_DIR)]);
        assert!(archive.is_file());
    }

    #[cfg(unix)]
    #[test]
    fn parent_sync_failures_stop_archiving_and_are_retried_for_existing_directories() {
        for failed_component in [None, Some("inbox")] {
            let d = tempfile::tempdir().unwrap();
            let root = Workspace::create(d.path(), "Project", "Test", TargetRef::None).unwrap().root;
            let snapshot = RecoverySnapshot {
                version: VERSION,
                items: vec![],
                metadata: json!({"format": FORMAT, "source": {"state": "missing"}, "details": {}}),
                project: None,
            };
            let failed = failed_component.map_or_else(|| root.clone(), |name| root.join(name));
            for _ in 0..2 {
                let error = write_recovery_snapshot_using(&root, &snapshot, |path| {
                    if path == failed.as_path() {
                        Err(ToolError::io(path.display(), std::io::Error::other("injected parent synchronization failure")))
                    } else {
                        sync_directory(path)
                    }
                })
                .unwrap_err();
                assert!(error.message.contains("injected parent synchronization failure"));
                let archive = root.join("inbox").join(RECOVERY_ARCHIVE_DIR);
                assert!(!archive.exists() || std::fs::read_dir(archive).unwrap().next().is_none());
            }
            let mut synced = Vec::new();
            let archive = write_recovery_snapshot_using(&root, &snapshot, |path| {
                synced.push(path.to_path_buf());
                sync_directory(path)
            })
            .unwrap();
            let directory = root.join("inbox").join(RECOVERY_ARCHIVE_DIR);
            assert_eq!(synced, vec![root.clone(), root.join("inbox"), directory.clone(), directory.clone(), directory.join(STAGING_DIR)]);
            assert!(archive.is_file());
        }
    }

    #[test]
    fn recovery_does_not_recreate_a_deleted_projects_inbox() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Project", "Test", TargetRef::None).unwrap();
        let root = ws.root.clone();
        let personal = root.join("personal.txt");
        std::fs::write(&personal, b"Keep these words").unwrap();
        let snapshot = RecoverySnapshot::from_loaded(&load(&ws), json!({}));
        ws.delete().unwrap();
        assert!(write_recovery_snapshot(&root, &snapshot).is_err());
        assert!(!root.join("inbox").exists());
        assert_eq!(std::fs::read(personal).unwrap(), b"Keep these words");
    }

    #[test]
    fn loaded_writers_and_archives_reject_a_recreated_project_at_the_same_path() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Project", "Old project", TargetRef::None).unwrap();
        let root = ws.root.clone();
        let created_ms = ws.manifest.created_ms;
        let personal = root.join("personal.txt");
        std::fs::write(&personal, b"Keep these words").unwrap();
        let mut old = load(&ws);
        let snapshot = RecoverySnapshot::from_loaded(&old, json!({}));
        ws.delete().unwrap();
        let mut new = Workspace::create_at(&root, "New project", "New words", TargetRef::None, Some(crate::workspace::ExistingFiles::Edit)).unwrap();
        new.manifest.created_ms = created_ms.saturating_add(1);
        std::fs::write(root.join(crate::workspace::MANIFEST), serde_json::to_vec(&new.manifest).unwrap()).unwrap();
        let new = Workspace::open(&root).unwrap();
        old.push(FeedItem::User(UserItem::Text("Stale words".into())));
        assert_eq!(save(&root, &mut old).unwrap_err().code, ErrorCode::FileChanged);
        assert_eq!(write_recovery_snapshot(&root, &snapshot).unwrap_err().code, ErrorCode::FileChanged);
        assert!(!root.join(FEED_FILE).exists());
        assert!(!root.join("inbox").join(RECOVERY_ARCHIVE_DIR).exists());
        assert_eq!(load(&new).to_vec(), vec![FeedItem::User(UserItem::Text("New words".into()))]);
        assert_eq!(std::fs::read(personal).unwrap(), b"Keep these words");
    }
}
