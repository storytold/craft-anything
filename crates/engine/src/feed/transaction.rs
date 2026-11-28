//! One project's feed writers and recovery publication share a bounded, cross-process lock.

use craft_anything_evidence::{ErrorCode, ToolError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(20);
pub(crate) const PROJECT_LOCK_FILE: &str = "feed.lock";

/// A non-reentrant project feed transaction. Dropping the owning file releases the OS lock.
/// Acquire request locks before this guard; never acquire a request lock while holding it.
/// The empty lock file is retained until project deletion removes its manifest first.
#[derive(Debug)]
pub struct ProjectFeedGuard {
    pub(super) root: PathBuf,
    project: ProjectIdentity,
    _file: same_file::Handle,
}

impl ProjectFeedGuard {
    pub(super) fn check_project(&self) -> Result<(), ToolError> {
        self.project.check(&self.root)
    }

    /// A lock acquired by pathname also has to match the workspace the caller loaded.
    pub(crate) fn check_workspace(&self, ws: &crate::workspace::Workspace) -> Result<(), ToolError> {
        self.check_project()?;
        if self.project.same(&ProjectIdentity::from_workspace(ws)?) { Ok(()) } else { Err(project_changed()) }
    }

    /// Preserve the held coordination inode even when the filesystem spells its name differently.
    pub(crate) fn is_lock_entry(&self, path: &Path) -> Result<bool, ToolError> {
        check_entry(&self.root.join("inbox").join(PROJECT_LOCK_FILE), &self._file)?;
        let metadata = std::fs::symlink_metadata(path).map_err(|error| ToolError::io(path.display(), error))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != 0 {
            return Ok(false);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            // Unlinking a payload does not require permission to read its contents.
            let held = self._file.as_file().metadata().map_err(|error| ToolError::io(path.display(), error))?;
            Ok(metadata.dev() == held.dev() && metadata.ino() == held.ino())
        }
        #[cfg(not(unix))]
        {
            let (file, _) = craft_anything_platform::fs::open_regular_file(path)?;
            let file = same_file::Handle::from_file(file).map_err(|error| ToolError::io(path.display(), error))?;
            Ok(file == self._file)
        }
    }

    pub(crate) fn finish_deletion(
        &self,
        inbox: &same_file::Handle,
        cleanup: &crate::workspace::CreationCleanup,
        after_claim: impl FnOnce(),
    ) -> Result<(), ToolError> {
        crate::workspace::finish_deleted_inbox(&self.root, &self.project.directory, inbox, &self._file, cleanup, after_claim)
    }
}

/// A read-only copy holds a shared lock, but cannot supply a writer's transaction guard.
#[derive(Debug)]
pub(crate) struct ProjectCopyGuard {
    guard: ProjectFeedGuard,
    inbox: same_file::Handle,
}

impl ProjectCopyGuard {
    pub(crate) fn check_workspace(&self, ws: &crate::workspace::Workspace) -> Result<(), ToolError> {
        let directory = self.guard.root.join("inbox");
        check_directory(&directory, &self.inbox)?;
        check_entry(&directory.join(PROJECT_LOCK_FILE), &self.guard._file)?;
        self.guard.check_workspace(ws)
    }
}

#[derive(Debug)]
pub(super) struct ProjectIdentity {
    directory: same_file::Handle,
    created_ms: u64,
    generation: Option<String>,
}

impl ProjectIdentity {
    fn read(root: &Path) -> Result<Self, ToolError> {
        Self::from_workspace(&crate::workspace::Workspace::open(root)?)
    }

    pub(super) fn from_workspace(ws: &crate::workspace::Workspace) -> Result<Self, ToolError> {
        let root = &ws.root;
        if !root.is_dir() {
            return Err(ToolError::new(ErrorCode::FileError, "The project's folder isn't available, so its conversation couldn't be locked.")
                .with_remediation("Restore the project folder, then reopen the project before retrying."));
        }
        let directory = same_file::Handle::from_path(root).map_err(|error| ToolError::io(root.display(), error))?;
        let created_ms = ws.manifest.created_ms;
        let generation = ws.manifest.generation.clone();
        Ok(Self { directory, created_ms, generation })
    }

    pub(super) fn same(&self, other: &Self) -> bool {
        self.directory == other.directory && self.created_ms == other.created_ms && self.generation == other.generation
    }

    pub(super) fn check(&self, root: &Path) -> Result<(), ToolError> {
        let current = Self::read(root)?;
        if !self.same(&current) {
            return Err(project_changed());
        }
        Ok(())
    }
}

fn project_changed() -> ToolError {
    ToolError::new(ErrorCode::FileChanged, "The project changed while its conversation operation was waiting.")
        .with_remediation("Open the current project before retrying. This operation has not changed its conversation.")
}

/// Coordinate feed comparison, replacement and backup publication across application processes.
/// Do not call another locking feed operation while this guard is held.
pub fn lock_project(root: &Path) -> Result<ProjectFeedGuard, ToolError> {
    lock_for(root, WAIT)
}

/// Read an existing coordination file without creating or writing source entries.
pub(crate) fn lock_project_for_copy(root: &Path) -> Result<ProjectCopyGuard, ToolError> {
    copy_lock_for(root, WAIT)
}

fn copy_lock_for(root: &Path, wait: Duration) -> Result<ProjectCopyGuard, ToolError> {
    let project = ProjectIdentity::read(root)?;
    let directory = root.join("inbox");
    let inbox = open_directory(&directory)?;
    let path = directory.join(PROJECT_LOCK_FILE);
    let metadata = std::fs::symlink_metadata(&path).map_err(|error| ToolError::io(path.display(), error))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != 0 {
        return Err(invalid(&path));
    }
    let (file, _) = craft_anything_platform::fs::open_regular_file(&path)?;
    let file = same_file::Handle::from_file(file).map_err(|error| ToolError::io(path.display(), error))?;
    check_entry(&path, &file)?;
    wait_for_lock(file.as_file(), &path, Instant::now(), wait, true)?;
    let guard = ProjectCopyGuard { guard: ProjectFeedGuard { root: root.to_path_buf(), project, _file: file }, inbox };
    guard.check_workspace(&crate::workspace::Workspace::open(root)?)?;
    Ok(guard)
}

fn lock_for(root: &Path, wait: Duration) -> Result<ProjectFeedGuard, ToolError> {
    lock_for_using(root, wait, || {})
}

fn lock_for_using(root: &Path, wait: Duration, after_identity: impl FnOnce()) -> Result<ProjectFeedGuard, ToolError> {
    lock_for_using_after_mkdir(root, wait, after_identity, || {})
}

fn lock_for_using_after_mkdir(root: &Path, wait: Duration, after_identity: impl FnOnce(), after_mkdir: impl FnOnce()) -> Result<ProjectFeedGuard, ToolError> {
    lock_for_using_before_open(root, wait, after_identity, after_mkdir, || {})
}

fn lock_for_using_before_open(
    root: &Path,
    wait: Duration,
    after_identity: impl FnOnce(),
    after_mkdir: impl FnOnce(),
    before_open: impl FnOnce(),
) -> Result<ProjectFeedGuard, ToolError> {
    // Earlier projects can lack an inbox even when they have a generation. Initialization
    // uses the same exclusive lock as initial creation. A newly created inbox must first
    // be confirmed against the original project before later failure can authorize cleanup.
    let project = ProjectIdentity::read(root)?;
    after_identity();
    let directory = root.join("inbox");
    let created_inbox = match std::fs::symlink_metadata(&directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            #[cfg(unix)]
            let builder = {
                use std::os::unix::fs::DirBuilderExt as _;
                let mut builder = std::fs::DirBuilder::new();
                builder.mode(0o700);
                builder
            };
            #[cfg(not(unix))]
            let builder = std::fs::DirBuilder::new();
            match builder.create(&directory) {
                Ok(()) => true,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
                Err(error) => return Err(ToolError::io(directory.display(), error)),
            }
        }
        Ok(_) => false,
        Err(error) => return Err(ToolError::io(directory.display(), error)),
    };
    if created_inbox {
        after_mkdir();
    }
    let inbox = craft_anything_platform::fs::open_child_directory(project.directory.as_file(), Path::new("inbox"))
        .and_then(same_file::Handle::from_file)
        .map_err(|error| ToolError::io(directory.display(), error))?;
    if created_inbox {
        // mkdir does not return a handle: deletion and another creator can reuse the name
        // before this first capture. An unconfirmed capture grants neither opening nor cleanup.
        project.check(root).map_err(|error| {
            let guidance = format!(
                "The coordination directory could not be confirmed against the original project. Inspect {} before retrying; its current contents were kept and this attempt did not create or acquire its lock.",
                directory.display()
            );
            let guidance = match &error.remediation {
                Some(previous) => format!("{previous} {guidance}"),
                None => guidance,
            };
            error.with_remediation(guidance)
        })?;
    }
    let mut opened = None;
    let mut created_lock = false;
    let mut held_lock = false;
    let acquired = (|| {
        if !created_inbox {
            // Reject a known stale caller without populating an existing reservation.
            // A newly created inbox already passed the first-capture confirmation above.
            project.check(root)?;
        }
        check_directory(&directory, &inbox)?;
        let path = directory.join(PROJECT_LOCK_FILE);
        let started = Instant::now();
        before_open();
        let (file, absent) = open_coordination_file(&directory, &inbox, started, wait)?;
        let file = opened.insert(file);
        created_lock = absent;
        check_entry(&path, file)?;
        wait_for_lock(file.as_file(), &path, started, wait, false)?;
        held_lock = true;
        check_entry(&path, file)?;
        check_directory(&directory, &inbox)?;
        project.check(root)
    })();
    if let Err(error) = acquired {
        return Err(if created_inbox {
            match crate::workspace::release_created_inbox(
                root,
                project.directory,
                inbox,
                // Another initializer can create this inbox's coordination file first.
                // Once held and verified, it belongs to the same cleanup protocol.
                opened.as_ref().filter(|_| created_lock || held_lock),
                opened.as_ref().filter(|_| held_lock),
            ) {
                Ok(()) => error,
                Err(cleanup) => {
                    let guidance = format!(
                        "The initializer's coordination entry could not be fully released: {} Inspect {} and any retained path before retrying.",
                        cleanup.for_people(),
                        directory.display()
                    );
                    let guidance = match &error.remediation {
                        Some(previous) => format!("{previous} {guidance}"),
                        None => guidance,
                    };
                    error.with_remediation(guidance)
                }
            }
        } else {
            error
        });
    }
    let file = opened.ok_or_else(|| ToolError::internal("The conversation lock was acquired without retaining its file."))?;
    Ok(ProjectFeedGuard { root: root.to_path_buf(), project, _file: file })
}

/// Initial creation participates in coordination before copying or publishing a manifest.
/// The raw handle grants no project identity and cannot authorize feed writes.
pub(crate) fn lock_creation(root: &Path, expected_inbox: &same_file::Handle) -> Result<same_file::Handle, Box<CreationLockFailure>> {
    lock_creation_using(root, expected_inbox, |file, path, started| wait_for_lock(file, path, started, WAIT, false))
}

/// A failed acquisition retains only a newly created or already exclusively held inode.
/// An opened but unlocked pre-existing file does not authorize creation rollback.
#[derive(Debug)]
pub(crate) struct CreationLockFailure {
    pub(crate) error: ToolError,
    file: Option<same_file::Handle>,
    held: bool,
}

impl CreationLockFailure {
    /// The caller checks the original root and absent manifest before and after recovery.
    /// An unheld handle must acquire its own exclusive lock before becoming coordination.
    pub(crate) fn recover(self, root: &Path, expected_inbox: &same_file::Handle) -> Result<Option<same_file::Handle>, ToolError> {
        let Some(file) = self.file else { return Ok(None) };
        let directory = root.join("inbox");
        let path = directory.join(PROJECT_LOCK_FILE);
        check_directory(&directory, expected_inbox)?;
        check_entry(&path, &file)?;
        if !self.held {
            wait_for_lock(file.as_file(), &path, Instant::now(), Duration::ZERO, false)?;
        }
        check_directory(&directory, expected_inbox)?;
        check_entry(&path, &file)?;
        Ok(Some(file))
    }
}

pub(crate) fn lock_creation_using(
    root: &Path,
    expected_inbox: &same_file::Handle,
    acquire: impl FnOnce(&std::fs::File, &Path, Instant) -> Result<(), ToolError>,
) -> Result<same_file::Handle, Box<CreationLockFailure>> {
    let directory = root.join("inbox");
    let started = Instant::now();
    let opened = check_directory(&directory, expected_inbox).and_then(|()| open_coordination_file(&directory, expected_inbox, started, WAIT));
    let (file, created) = opened.map_err(|error| Box::new(CreationLockFailure { error, file: None, held: false }))?;
    let path = directory.join(PROJECT_LOCK_FILE);
    let mut held = false;
    let acquired = (|| {
        check_entry(&path, &file)?;
        acquire(file.as_file(), &path, started)?;
        held = true;
        check_directory(&directory, expected_inbox)?;
        check_entry(&path, &file)
    })();
    match acquired {
        Ok(()) => Ok(file),
        Err(error) => Err(Box::new(CreationLockFailure { error, file: (created || held).then_some(file), held })),
    }
}

fn open_coordination_file(directory: &Path, inbox: &same_file::Handle, started: Instant, wait: Duration) -> Result<(same_file::Handle, bool), ToolError> {
    let path = directory.join(PROJECT_LOCK_FILE);
    loop {
        // Both opening and creation belong to this retained directory. A different creator
        // may already own its former pathname, including an empty reservation with no lock.
        match craft_anything_platform::fs::open_child_file_for_update(inbox.as_file(), Path::new(PROJECT_LOCK_FILE), false) {
            Ok(file) => return Ok((same_file::Handle::from_file(file).map_err(|error| ToolError::io(path.display(), error))?, false)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(ToolError::io(path.display(), error)),
        }
        // Exclusive locks may be emulated as write locks on network filesystems.
        match craft_anything_platform::fs::open_child_file_for_update(inbox.as_file(), Path::new(PROJECT_LOCK_FILE), true) {
            Ok(file) => return Ok((same_file::Handle::from_file(file).map_err(|error| ToolError::io(path.display(), error))?, true)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && started.elapsed() < wait => {}
            Err(error) => return Err(ToolError::io(path.display(), error)),
        }
    }
}

fn open_directory(path: &Path) -> Result<same_file::Handle, ToolError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| ToolError::io(path.display(), error))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(invalid(path));
    }
    let directory = same_file::Handle::from_path(path).map_err(|error| ToolError::io(path.display(), error))?;
    check_directory(path, &directory)?;
    Ok(directory)
}

fn check_directory(path: &Path, expected: &same_file::Handle) -> Result<(), ToolError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| ToolError::io(path.display(), error))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(invalid(path));
    }
    let current = same_file::Handle::from_path(path).map_err(|error| ToolError::io(path.display(), error))?;
    if &current == expected { Ok(()) } else { Err(project_changed()) }
}

fn wait_for_lock(file: &std::fs::File, path: &Path, started: Instant, wait: Duration, shared: bool) -> Result<(), ToolError> {
    loop {
        let locked = if shared { file.try_lock_shared() } else { file.try_lock() };
        match locked {
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::WouldBlock) if started.elapsed() < wait => std::thread::sleep(POLL.min(wait.saturating_sub(started.elapsed()))),
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(ToolError::new(ErrorCode::TimedOut, "Another operation is updating or copying this project's conversation.")
                    .with_remediation("Let that operation finish, then retry. This operation has not changed the conversation."));
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(ToolError::io(path.display(), error)),
        }
    }
}

fn invalid(path: &Path) -> ToolError {
    ToolError::new(ErrorCode::FileError, format!("{} is not a regular conversation coordination entry.", path.display()))
        .with_remediation("Keep the existing entry unchanged and restore access to the project's inbox before retrying.")
}

/// Cleanup must not move an inbox out from under a deleter that still owns its lock.
/// `held` is supplied only by an initializer that has already acquired that exclusive handle.
pub(crate) fn lock_inbox_for_cleanup(root: &Path, held: Option<&same_file::Handle>) -> Result<Option<same_file::Handle>, ToolError> {
    let path = root.join("inbox").join(PROJECT_LOCK_FILE);
    if let Some(file) = held {
        check_entry(&path, file)?;
        return Ok(None);
    }
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ToolError::io(path.display(), error)),
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() == 0 => {}
        Ok(_) => return Err(invalid(&path)),
    }
    let file = std::fs::File::options().read(true).write(true).open(&path).map_err(|error| ToolError::io(path.display(), error))?;
    let file = same_file::Handle::from_file(file).map_err(|error| ToolError::io(path.display(), error))?;
    check_entry(&path, &file)?;
    wait_for_lock(file.as_file(), &path, Instant::now(), Duration::ZERO, false)?;
    check_entry(&path, &file)?;
    Ok(Some(file))
}

pub(crate) fn check_entry(path: &Path, file: &same_file::Handle) -> Result<(), ToolError> {
    let opened = file.as_file().metadata().map_err(|error| ToolError::io(path.display(), error))?;
    let current = std::fs::symlink_metadata(path).map_err(|error| ToolError::io(path.display(), error))?;
    if !opened.is_file()
        || opened.len() != 0
        || !current.is_file()
        || current.file_type().is_symlink()
        || current.len() != 0
        || opened.created().ok() != current.created().ok()
        || opened.modified().ok() != current.modified().ok()
    {
        return Err(invalid(path));
    }
    let (current, _) = craft_anything_platform::fs::open_regular_file(path)?;
    let current = same_file::Handle::from_file(current).map_err(|error| ToolError::io(path.display(), error))?;
    if &current != file {
        return Err(ToolError::new(ErrorCode::FileChanged, "The conversation's coordination file changed while it was being opened.")
            .with_remediation("Reopen the project before retrying. Do not remove its coordination file while an operation is active."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{ExistingFiles, TargetRef, Workspace};

    #[test]
    fn feed_lock_child() {
        let Ok(root) = std::env::var("CRAFT_TEST_FEED_LOCK_ROOT") else { return };
        let result = lock_for(Path::new(&root), Duration::from_millis(50));
        assert_eq!(result.unwrap_err().code, ErrorCode::TimedOut);
    }

    #[test]
    fn the_feed_lock_coordinates_processes_and_releases_on_drop() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Project", "Test", TargetRef::None).unwrap();
        let guard = lock_project(&ws.root).unwrap();
        craft_anything_platform::Process::new(std::env::current_exe().unwrap())
            .args(["--exact", "feed::transaction::tests::feed_lock_child", "--test-threads=1"])
            .env("CRAFT_TEST_FEED_LOCK_ROOT", &ws.root)
            .timeout(Duration::from_secs(5))
            .stdout_cap(64 * 1024)
            .stderr_cap(64 * 1024)
            .run_checked()
            .unwrap();
        assert_eq!(lock_for(&ws.root, Duration::ZERO).unwrap_err().code, ErrorCode::TimedOut);
        let path = ws.root.join("inbox/feed.lock");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        drop(guard);
        assert!(path.is_file(), "releasing the lock does not replace the coordination file");
        drop(lock_project(&ws.root).unwrap());
    }

    #[test]
    fn a_read_only_copy_lock_excludes_writers_and_never_creates_entries() {
        use std::io::Write as _;
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Project", "Test", TargetRef::None).unwrap();
        let path = ws.inbox_dir().join(PROJECT_LOCK_FILE);
        drop(lock_project(&ws.root).unwrap());
        std::fs::remove_file(&path).unwrap();
        assert!(copy_lock_for(&ws.root, Duration::ZERO).is_err());
        assert!(!path.exists(), "a read-only lock does not initialize coordination");
        drop(lock_project(&ws.root).unwrap());
        let identity = same_file::Handle::from_path(&path).unwrap();

        let first = copy_lock_for(&ws.root, Duration::ZERO).unwrap();
        let second = copy_lock_for(&ws.root, Duration::ZERO).unwrap();
        first.check_workspace(&ws).unwrap();
        assert!(first.guard._file.as_file().write_all(b"not writable").is_err());
        craft_anything_platform::Process::new(std::env::current_exe().unwrap())
            .args(["--exact", "feed::transaction::tests::feed_lock_child", "--test-threads=1"])
            .env("CRAFT_TEST_FEED_LOCK_ROOT", &ws.root)
            .timeout(Duration::from_secs(5))
            .stdout_cap(64 * 1024)
            .stderr_cap(64 * 1024)
            .run_checked()
            .unwrap();
        assert_eq!(lock_for(&ws.root, Duration::ZERO).unwrap_err().code, ErrorCode::TimedOut);
        drop(first);
        assert_eq!(lock_for(&ws.root, Duration::ZERO).unwrap_err().code, ErrorCode::TimedOut);
        drop(second);
        let writer = lock_for(&ws.root, Duration::ZERO).unwrap();
        assert_eq!(copy_lock_for(&ws.root, Duration::ZERO).unwrap_err().code, ErrorCode::TimedOut);
        drop(writer);

        assert_eq!(identity, same_file::Handle::from_path(&path).unwrap());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
    }

    #[test]
    fn read_only_copy_guards_recheck_the_inbox_and_coordination_inode() {
        for replace_inbox in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let ws = Workspace::create(directory.path(), "Project", "Test", TargetRef::None).unwrap();
            drop(lock_project(&ws.root).unwrap());
            let guard = copy_lock_for(&ws.root, Duration::ZERO).unwrap();
            let manifest = std::fs::read(ws.root.join(crate::workspace::MANIFEST)).unwrap();
            let inbox = ws.inbox_dir();
            let lock = inbox.join(PROJECT_LOCK_FILE);

            if replace_inbox {
                // Move the held child file first so Windows can relocate its parent.
                // A hard link retains that same locked inode in the replacement inbox.
                let retained_lock = ws.root.join("retained.lock");
                std::fs::rename(&lock, &retained_lock).unwrap();
                let retained = ws.root.join("retained-inbox");
                std::fs::rename(&inbox, &retained).unwrap();
                std::fs::create_dir(&inbox).unwrap();
                std::fs::hard_link(&retained_lock, &lock).unwrap();
                assert_eq!(same_file::Handle::from_path(&lock).unwrap(), guard.guard._file);
                assert_eq!(lock_for(&ws.root, Duration::ZERO).unwrap_err().code, ErrorCode::TimedOut);
            } else {
                // Observable external replacement can give a writer a different lock inode.
                std::fs::rename(&lock, ws.root.join("retained.lock")).unwrap();
                std::fs::File::create_new(&lock).unwrap();
                let writer = lock_for(&ws.root, Duration::ZERO).unwrap();
                assert!(guard.check_workspace(&ws).is_err());
                drop(writer);
            }

            assert!(guard.check_workspace(&ws).is_err());
            assert_eq!(std::fs::read(ws.root.join(crate::workspace::MANIFEST)).unwrap(), manifest);
            assert_eq!(std::fs::metadata(&lock).unwrap().len(), 0);
        }
    }

    #[test]
    fn coordination_preserves_unrecognized_entries_and_missing_projects() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing");
        assert!(lock_project(&missing).is_err());
        assert!(!missing.exists());
        assert!(lock_project(directory.path()).is_err());
        assert!(!directory.path().join("inbox").exists());
        let ws = Workspace::create(directory.path(), "Project", "Test", TargetRef::None).unwrap();
        let inbox = ws.root.join("inbox");
        assert!(inbox.is_dir(), "initial creation retains its coordination reservation");
        let path = inbox.join("feed.lock");
        std::fs::write(&path, b"Keep these words").unwrap();
        assert!(lock_project(&ws.root).is_err());
        assert!(copy_lock_for(&ws.root, Duration::ZERO).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"Keep these words");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(lock_project(&ws.root).is_err());
        assert!(copy_lock_for(&ws.root, Duration::ZERO).is_err());
        assert!(path.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn inbox_classification_and_deletion_do_not_require_payload_read_permission() {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Project", "Test", TargetRef::None).unwrap();
        let root = ws.root.clone();
        std::fs::write(root.join("personal.txt"), b"Keep my app").unwrap();
        let inbox = ws.inbox_dir();
        let payload = inbox.join("empty-payload");
        std::fs::File::create_new(&payload).unwrap();
        std::fs::set_permissions(&payload, std::fs::Permissions::from_mode(0o000)).unwrap();
        let alias = inbox.join("coordination-alias");
        std::fs::hard_link(inbox.join(PROJECT_LOCK_FILE), &alias).unwrap();

        let guard = lock_project(&root).unwrap();
        assert!(!guard.is_lock_entry(&payload).unwrap());
        assert!(guard.is_lock_entry(&alias).unwrap(), "an arbitrary same-inode name still belongs to coordination");
        assert_eq!(std::fs::symlink_metadata(&payload).unwrap().permissions().mode() & 0o777, 0);
        drop(guard);
        ws.delete().unwrap();

        assert!(!root.join(crate::workspace::MANIFEST).exists());
        assert!(!inbox.exists());
        assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep my app");
    }

    #[cfg(unix)]
    #[test]
    fn coordination_does_not_follow_existing_links() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Project", "Test", TargetRef::None).unwrap();
        let inbox = ws.root.join("inbox");
        assert!(inbox.is_dir(), "initial creation retains its coordination reservation");
        let path = inbox.join("feed.lock");
        let target = directory.path().join("personal");
        std::fs::write(&target, b"").unwrap();
        drop(lock_project(&ws.root).unwrap());
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(lock_project(&ws.root).is_err());
        assert!(copy_lock_for(&ws.root, Duration::ZERO).is_err());
        assert_eq!(std::fs::read_link(path).unwrap(), target);
        assert_eq!(std::fs::read(target).unwrap(), b"");
    }

    #[test]
    fn guards_recheck_the_project_after_locking() {
        let directory = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(directory.path(), "Project", "Test", TargetRef::None).unwrap();
        let guard = lock_project(&ws.root).unwrap();
        let manifest = ws.root.join(crate::workspace::MANIFEST);
        std::fs::remove_file(&manifest).unwrap();
        assert!(guard.check_project().is_err());
        ws.manifest.created_ms = ws.manifest.created_ms.saturating_add(1);
        // Construct an external replacement; ordinary workspace updates keep the identity
        // and acquire this same non-reentrant guard.
        std::fs::write(&manifest, serde_json::to_vec_pretty(&ws.manifest).unwrap()).unwrap();
        assert_eq!(guard.check_project().unwrap_err().code, ErrorCode::FileChanged);
        let replacement = directory.path().join("replacement");
        // Keep the guard alive while moving its held child outside the relocated root.
        std::fs::rename(ws.inbox_dir().join(PROJECT_LOCK_FILE), directory.path().join("retained.lock")).unwrap();
        std::fs::rename(&ws.root, &replacement).unwrap();
        std::fs::create_dir(&ws.root).unwrap();
        ws.manifest.created_ms = ws.manifest.created_ms.saturating_sub(1);
        std::fs::write(&manifest, serde_json::to_vec_pretty(&ws.manifest).unwrap()).unwrap();
        assert_eq!(guard.check_project().unwrap_err().code, ErrorCode::FileChanged);
    }

    #[test]
    fn earlier_generation_bearing_projects_initialize_coordination_without_changing_the_manifest() {
        for version in [6u32, 7] {
            let directory = tempfile::tempdir().unwrap();
            let ws = Workspace::create(directory.path(), "Earlier project", "Test", TargetRef::None).unwrap();
            assert!(ws.manifest.generation.is_some());
            let mut document = serde_json::to_value(&ws.manifest).unwrap();
            document["version"] = serde_json::Value::from(version);
            let original = serde_json::to_vec_pretty(&document).unwrap();
            let manifest = ws.root.join(crate::workspace::MANIFEST);
            std::fs::write(&manifest, &original).unwrap();
            drop(lock_project(&ws.root).unwrap());
            let path = ws.inbox_dir().join(PROJECT_LOCK_FILE);
            std::fs::remove_file(&path).unwrap();
            std::fs::remove_dir(ws.inbox_dir()).unwrap();

            let guard = lock_for(&ws.root, Duration::ZERO).unwrap();
            let opened = Workspace::open(&ws.root).unwrap();
            guard.check_workspace(&opened).unwrap();
            assert_eq!(opened.manifest.generation, ws.manifest.generation);
            assert_eq!(std::fs::read(&manifest).unwrap(), original, "coordination does not rewrite an older manifest");
            let identity = same_file::Handle::from_path(&path).unwrap();
            assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
            drop(guard);
            drop(lock_for(&ws.root, Duration::ZERO).unwrap());
            assert_eq!(same_file::Handle::from_path(&path).unwrap(), identity);
            assert_eq!(std::fs::read(&manifest).unwrap(), original);
        }
    }

    #[test]
    fn deletion_during_lock_creation_still_refuses_payload_writes() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Project", "Test", TargetRef::None).unwrap();
        let root = ws.root.clone();
        std::fs::write(root.join("personal.txt"), b"Keep my app").unwrap();
        let error = lock_for_using(&root, WAIT, || ws.delete().unwrap()).unwrap_err();
        assert!(!root.join(crate::workspace::MANIFEST).exists());
        assert!(!root.join(super::super::FEED_FILE).exists());
        assert!(root.join("inbox").is_dir(), "a failed first-capture confirmation cannot authorize directory cleanup");
        assert_eq!(std::fs::read_dir(root.join("inbox")).unwrap().count(), 0);
        assert!(error.remediation.as_deref().unwrap().contains(&root.join("inbox").display().to_string()));
        assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep my app");
        assert!(Workspace::create_at(&root, "Replacement", "Retry", TargetRef::None, Some(ExistingFiles::Edit)).is_err());
        // The fixture can inspect and release its known empty orphan; the initializer cannot.
        std::fs::remove_dir(root.join("inbox")).unwrap();
        Workspace::create_at(&root, "Replacement", "Retry", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
    }

    #[test]
    fn deleted_legacy_initializers_preserve_unconfirmed_coordination_entries() {
        for another_initializer_created_lock in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut ws = Workspace::create(directory.path(), "Project", "Test", TargetRef::None).unwrap();
            let root = ws.root.clone();
            std::fs::write(root.join("personal.txt"), b"Keep my app").unwrap();
            drop(lock_project(&root).unwrap());
            ws.manifest.generation = None;
            std::fs::write(root.join(crate::workspace::MANIFEST), serde_json::to_vec_pretty(&ws.manifest).unwrap()).unwrap();
            std::fs::remove_file(ws.inbox_dir().join(PROJECT_LOCK_FILE)).unwrap();
            std::fs::remove_dir(ws.inbox_dir()).unwrap();
            let mut retained = None;

            let result = lock_for_using_after_mkdir(
                &root,
                Duration::ZERO,
                || ws.delete().unwrap(),
                || {
                    if another_initializer_created_lock {
                        std::fs::File::create_new(root.join("inbox").join(PROJECT_LOCK_FILE)).unwrap();
                    }
                    retained = Some(same_file::Handle::from_path(root.join("inbox")).unwrap());
                },
            );

            let error = result.unwrap_err();
            assert!(!root.join(crate::workspace::MANIFEST).exists());
            assert!(!root.join(super::super::FEED_FILE).exists());
            assert_eq!(retained.unwrap(), same_file::Handle::from_path(root.join("inbox")).unwrap());
            assert_eq!(std::fs::read_dir(root.join("inbox")).unwrap().count(), usize::from(another_initializer_created_lock));
            if another_initializer_created_lock {
                assert_eq!(std::fs::metadata(root.join("inbox").join(PROJECT_LOCK_FILE)).unwrap().len(), 0);
            }
            let mut remaining = std::fs::read_dir(&root).unwrap().map(|entry| entry.unwrap().file_name()).collect::<Vec<_>>();
            remaining.sort();
            assert_eq!(
                remaining,
                [std::ffi::OsString::from("inbox"), std::ffi::OsString::from("personal.txt")],
                "uncertain contents stay at the public inspection path"
            );
            assert!(error.remediation.as_deref().unwrap().contains(&root.join("inbox").display().to_string()));
            assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep my app");
            assert!(Workspace::create_at(&root, "Replacement", "Retry", TargetRef::None, Some(ExistingFiles::Edit)).is_err());
            // Only the fixture knows these retained entries are orphaned; inspection precedes retry.
            if another_initializer_created_lock {
                std::fs::remove_file(root.join("inbox").join(PROJECT_LOCK_FILE)).unwrap();
            }
            std::fs::remove_dir(root.join("inbox")).unwrap();
            Workspace::create_at(&root, "Replacement", "Retry", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        }
    }

    #[test]
    fn a_stale_initializer_preserves_an_active_creators_existing_inbox() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Project", "Test", TargetRef::None).unwrap();
        let root = ws.root.clone();
        std::fs::write(root.join("personal.txt"), b"Keep my app").unwrap();
        let mut active = None;

        let result = lock_for_using(&root, WAIT, || {
            ws.delete().unwrap();
            std::fs::create_dir(root.join("inbox")).unwrap();
            active = Some(same_file::Handle::from_path(root.join("inbox")).unwrap());
        });

        assert!(result.is_err());
        assert_eq!(active.unwrap(), same_file::Handle::from_path(root.join("inbox")).unwrap());
        assert_eq!(
            std::fs::read_dir(root.join("inbox")).unwrap().count(),
            0,
            "the stale initializer neither claims nor populates another creator's reservation"
        );
        assert!(Workspace::create_at(&root, "Other", "Retry", TargetRef::None, Some(ExistingFiles::Edit)).is_err());
    }

    #[test]
    fn an_initializer_delayed_before_open_cannot_strand_a_new_creators_lock() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Project", "Test", TargetRef::None).unwrap();
        let root = ws.root.clone();
        std::fs::write(root.join("personal.txt"), b"Keep my app").unwrap();
        let mut creation = None;

        let result = lock_for_using_before_open(
            &root,
            Duration::ZERO,
            || {},
            || {},
            || {
                // The stale caller has already checked the old project and existing inbox.
                ws.delete().unwrap();
                std::fs::create_dir(root.join("inbox")).unwrap();
                let directory = same_file::Handle::from_path(&root).unwrap();
                let inbox = same_file::Handle::from_path(root.join("inbox")).unwrap();
                let lock = lock_creation(&root, &inbox).unwrap();
                creation = Some((directory, inbox, lock));
            },
        );

        assert_ne!(result.unwrap_err().code, ErrorCode::TimedOut, "the stale initializer must not wait on the new creator's lock");
        let (directory, inbox, lock) = creation.unwrap();
        assert_eq!(same_file::Handle::from_path(root.join("inbox")).unwrap(), inbox);
        assert_eq!(same_file::Handle::from_path(root.join("inbox").join(PROJECT_LOCK_FILE)).unwrap(), lock);
        assert_eq!(std::fs::metadata(root.join("inbox").join(PROJECT_LOCK_FILE)).unwrap().len(), 0);
        // A creator that does not publish releases that same held coordination inode.
        crate::workspace::release_created_inbox(&root, directory, inbox, Some(&lock), Some(&lock)).unwrap();
        drop(lock);
        assert!(!root.join("inbox").exists());
        let remaining = std::fs::read_dir(&root).unwrap().map(|entry| entry.unwrap().file_name()).collect::<Vec<_>>();
        assert_eq!(remaining, [std::ffi::OsString::from("personal.txt")]);
        Workspace::create_at(&root, "Replacement", "Retry", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep my app");
    }

    #[test]
    fn a_deleted_inbox_cannot_redirect_an_initializer_into_a_new_creators_empty_reservation() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Project", "Test", TargetRef::None).unwrap();
        let root = ws.root.clone();
        std::fs::write(root.join("personal.txt"), b"Keep my app").unwrap();
        let mut active = None;

        let result = lock_for_using_before_open(
            &root,
            Duration::ZERO,
            || {},
            || {},
            || {
                ws.delete().unwrap();
                std::fs::create_dir(root.join("inbox")).unwrap();
                active = Some(same_file::Handle::from_path(root.join("inbox")).unwrap());
            },
        );

        assert!(result.is_err());
        assert_eq!(active.unwrap(), same_file::Handle::from_path(root.join("inbox")).unwrap());
        assert_eq!(
            std::fs::read_dir(root.join("inbox")).unwrap().count(),
            0,
            "even before its lock is acquired, the replacement creator's reservation must remain empty"
        );
        assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep my app");
        std::fs::remove_dir(root.join("inbox")).unwrap();
        Workspace::create_at(&root, "Replacement", "Retry", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
    }

    #[test]
    fn a_stale_initializer_preserves_a_published_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Project", "Test", TargetRef::None).unwrap();
        let root = ws.root.clone();
        std::fs::write(root.join("personal.txt"), b"Keep my app").unwrap();
        let mut published = Vec::new();
        let mut published_lock = None;

        let result = lock_for_using(&root, WAIT, || {
            ws.delete().unwrap();
            Workspace::create_at(&root, "Replacement", "New request", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
            published = std::fs::read(root.join(crate::workspace::MANIFEST)).unwrap();
            published_lock = Some(same_file::Handle::from_path(root.join("inbox").join(PROJECT_LOCK_FILE)).unwrap());
        });

        assert_eq!(result.unwrap_err().code, ErrorCode::FileChanged);
        assert_eq!(std::fs::read(root.join(crate::workspace::MANIFEST)).unwrap(), published);
        assert_eq!(same_file::Handle::from_path(root.join("inbox").join(PROJECT_LOCK_FILE)).unwrap(), published_lock.unwrap());
        assert_eq!(std::fs::read_dir(root.join("inbox")).unwrap().count(), 1);
        assert_eq!(std::fs::metadata(root.join("inbox").join(PROJECT_LOCK_FILE)).unwrap().len(), 0);
        assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep my app");
    }

    #[test]
    fn a_legacy_initializers_late_first_capture_preserves_a_new_creators_reservation() {
        for creator_holds_lock in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut ws = Workspace::create(directory.path(), "Project", "Test", TargetRef::None).unwrap();
            let root = ws.root.clone();
            std::fs::write(root.join("personal.txt"), b"Keep my app").unwrap();
            drop(lock_project(&root).unwrap());
            ws.manifest.generation = None;
            std::fs::write(root.join(crate::workspace::MANIFEST), serde_json::to_vec_pretty(&ws.manifest).unwrap()).unwrap();
            std::fs::remove_file(ws.inbox_dir().join(PROJECT_LOCK_FILE)).unwrap();
            std::fs::remove_dir(ws.inbox_dir()).unwrap();
            let mut active = None;
            let mut creation_lock = None;

            let result = lock_for_using_after_mkdir(
                &root,
                Duration::ZERO,
                || {},
                || {
                    ws.delete().unwrap();
                    std::fs::create_dir(root.join("inbox")).unwrap();
                    let inbox = same_file::Handle::from_path(root.join("inbox")).unwrap();
                    if creator_holds_lock {
                        creation_lock = Some(lock_creation(&root, &inbox).unwrap());
                    }
                    active = Some(inbox);
                },
            );

            let error = result.unwrap_err();
            assert!(root.join("inbox").is_dir(), "an unconfirmed first capture must preserve the replacement reservation");
            let inbox = active.unwrap();
            assert_eq!(inbox, same_file::Handle::from_path(root.join("inbox")).unwrap());
            assert_eq!(
                std::fs::read_dir(root.join("inbox")).unwrap().count(),
                usize::from(creator_holds_lock),
                "the stale initializer neither adds nor removes coordination entries"
            );
            if let Some(lock) = &creation_lock {
                assert_eq!(&same_file::Handle::from_path(root.join("inbox").join(PROJECT_LOCK_FILE)).unwrap(), lock);
            } else {
                assert!(!root.join("inbox").join(PROJECT_LOCK_FILE).exists());
            }
            assert!(error.remediation.as_deref().unwrap().contains(&root.join("inbox").display().to_string()));
            assert!(Workspace::create_at(&root, "Other", "Retry", TargetRef::None, Some(ExistingFiles::Edit)).is_err());
            assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep my app");
            if creation_lock.is_none() {
                creation_lock = Some(lock_creation(&root, &inbox).unwrap());
            }
            assert_eq!(std::fs::metadata(root.join("inbox").join(PROJECT_LOCK_FILE)).unwrap().len(), 0);
            drop(creation_lock);
            drop(inbox);
            // The fixture owns this creator's unchanged reservation and explicitly releases it.
            std::fs::remove_file(root.join("inbox").join(PROJECT_LOCK_FILE)).unwrap();
            std::fs::remove_dir(root.join("inbox")).unwrap();
            Workspace::create_at(&root, "Replacement", "Retry", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
            assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep my app");
        }
    }
    #[cfg(unix)]
    #[test]
    fn creation_lock_rejects_directory_links_before_creating_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("project");
        let inbox = root.join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let held = same_file::Handle::from_path(&inbox).unwrap();
        let displaced = directory.path().join("displaced-inbox");
        std::fs::rename(&inbox, &displaced).unwrap();
        std::os::unix::fs::symlink(&displaced, &inbox).unwrap();

        assert!(lock_creation(&root, &held).is_err());

        assert_eq!(std::fs::read_link(&inbox).unwrap(), displaced);
        assert!(!displaced.join("feed.lock").exists(), "the refused link target receives no coordination file");
    }
}
