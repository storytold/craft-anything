//! File safety: bounded regular-file reads and atomic writes (temp file, fsync, rename),
//! optionally private (0600 on Unix).

use craft_anything_evidence::{ErrorCode, ToolError};
use std::io::{Read as _, Write as _};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn check_child_name(name: &Path) -> std::io::Result<()> {
    let mut components = name.components();
    if !matches!(components.next(), Some(std::path::Component::Normal(component)) if component == name.as_os_str()) || components.next().is_some() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "expected a single child name"));
    }
    Ok(())
}

/// Open one existing child directory relative to its retained parent, without following links.
pub fn open_child_directory(parent: &std::fs::File, name: &Path) -> std::io::Result<std::fs::File> {
    check_child_name(name)?;
    #[cfg(any(unix, windows))]
    {
        #[cfg(unix)]
        let file = {
            use nix::fcntl::{OFlag, openat};
            use nix::sys::stat::Mode;
            let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC;
            std::fs::File::from(openat(parent, name, flags, Mode::empty()).map_err(std::io::Error::from)?)
        };
        #[cfg(windows)]
        let file = {
            use fs_at::os::windows::OpenOptionsExt as _;
            use windows::Win32::Storage::FileSystem::{FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_TRAVERSE};
            // Keep the no-follow handle in a File before rejecting links below: fs_at 0.2.1's
            // own link check can fail before taking ownership. open_path_at needs explicit access.
            fs_at::OpenOptions::default().desired_access((FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY | FILE_TRAVERSE).0).open_path_at(parent, name)?
        };
        let metadata = file.metadata()?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "the child is not a directory"));
        }
        Ok(file)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = parent;
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "directory-relative opening is unavailable"))
    }
}

/// Open one regular child for updating, optionally creating it exclusively.
/// New Unix files request owner read/write permissions (0600). Windows creation does not set an
/// explicit private security descriptor and uses the default access-control settings.
/// The parent handle is authoritative even if its former pathname now names another directory.
/// Existing special files cannot block a Unix open before their type is checked.
pub fn open_child_file_for_update(parent: &std::fs::File, name: &Path, create_new: bool) -> std::io::Result<std::fs::File> {
    check_child_name(name)?;
    #[cfg(any(unix, windows))]
    {
        #[cfg(unix)]
        let file = {
            use nix::fcntl::{OFlag, openat};
            use nix::sys::stat::Mode;
            let mut flags = OFlag::O_RDWR | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC;
            if create_new {
                flags |= OFlag::O_CREAT | OFlag::O_EXCL;
            }
            std::fs::File::from(openat(parent, name, flags, Mode::S_IRUSR | Mode::S_IWUSR).map_err(std::io::Error::from)?)
        };
        #[cfg(windows)]
        let file = {
            use fs_at::os::windows::OpenOptionsExt as _;
            use windows::Win32::Storage::FileSystem::{FILE_GENERIC_READ, FILE_GENERIC_WRITE};
            // As above, validate the owned no-follow File and retain read/write and exclusive creation.
            fs_at::OpenOptions::default().desired_access((FILE_GENERIC_READ | FILE_GENERIC_WRITE).0).create_new(create_new).open_path_at(parent, name)?
        };
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "the child is not a regular file"));
        }
        Ok(file)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (parent, create_new);
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "directory-relative opening is unavailable"))
    }
}

/// Open a regular file and return metadata from the opened handle. Symlinks to regular files
/// are allowed. On Unix, a path replaced by a FIFO cannot block the open before its type is checked.
pub fn open_regular_file(path: &Path) -> Result<(std::fs::File, std::fs::Metadata), ToolError> {
    let invalid = || ToolError::invalid(format!("{} is not a regular file", path.display()));
    let before = std::fs::metadata(path).map_err(|e| ToolError::io(path.display(), e))?;
    if !before.is_file() {
        return Err(invalid());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(nix::libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|e| ToolError::io(path.display(), e))?;
    let metadata = file.metadata().map_err(|e| ToolError::io(path.display(), e))?;
    if !metadata.is_file() {
        return Err(invalid());
    }
    Ok((file, metadata))
}

/// Read a whole regular file, enforcing `max` on the bytes read as well as the size metadata.
/// The one-byte overflow probe does not add to `max`, so even `u64::MAX` cannot overflow.
pub fn read_file_capped(path: &Path, max: u64) -> Result<Vec<u8>, ToolError> {
    let (mut file, metadata) = open_regular_file(path)?;
    if metadata.len() > max {
        return Err(ToolError::new(ErrorCode::OverLimit, format!("{} is {} bytes; the limit is {max}", path.display(), metadata.len()))
            .with_remediation("Raise the size limit parameter if you really want to read it."));
    }
    let mut bytes = Vec::new();
    (&mut file).take(max).read_to_end(&mut bytes).map_err(|e| ToolError::io(path.display(), e))?;
    let mut extra = [0u8; 1];
    match file.read_exact(&mut extra) {
        Ok(()) => Err(ToolError::new(ErrorCode::OverLimit, format!("{} contains more than {max} bytes; the read limit was reached", path.display()))
            .with_remediation("Raise the size limit parameter if you really want to read it.")),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(bytes),
        Err(error) => Err(ToolError::io(path.display(), error)),
    }
}

/// Write `bytes` to `path` atomically, keeping the existing file's permissions when it exists.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ToolError> {
    write_impl(path, bytes, false)
}

/// Write `bytes` to `path` atomically with owner-only permissions (0600 on Unix). Used for
/// configuration files that may hold paths or tokens, and for the key fallback file.
pub fn write_atomic_private(path: &Path, bytes: &[u8]) -> Result<(), ToolError> {
    write_impl(path, bytes, true)
}

/// Restrict an existing, caller-selected private-data directory to its owner (0700 on Unix).
/// This changes the supplied directory, including an existing custom config directory, not its
/// ancestors. The caller must select a dedicated directory, not an unrelated shared folder.
/// Returns an error if permissions cannot be set; does not create the directory.
pub fn set_private_dir(dir: &Path) -> Result<(), ToolError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| ToolError::io(dir.display(), e))?;
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

fn write_impl(path: &Path, bytes: &[u8], private: bool) -> Result<(), ToolError> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        Some(_) => std::path::PathBuf::from("."),
        None => return Err(ToolError::invalid(format!("{} has no parent folder", path.display()))),
    };
    std::fs::create_dir_all(&dir).map_err(|e| ToolError::io(dir.display(), e))?;
    let existing_perms = std::fs::metadata(path).ok().map(|m| m.permissions());
    let (tmp, mut f) = create_temp(&dir, path, private, &COUNTER)?;
    let result = (|| {
        if private {
            set_private(&tmp)?;
        } else if let Some(p) = existing_perms {
            std::fs::set_permissions(&tmp, p).map_err(|e| ToolError::io(tmp.display(), e))?;
        }
        f.write_all(bytes).map_err(|e| ToolError::io(tmp.display(), e))?;
        f.sync_all().map_err(|e| ToolError::io(tmp.display(), e))?;
        drop(f);
        rename_retrying(&tmp, path).map_err(|e| ToolError::io(path.display(), e))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Reserve a short, new temporary file before entering the cleanup scope. A collision belongs to
/// another write, so it must never be truncated or removed by this call.
fn create_temp(dir: &Path, destination: &Path, private: bool, counter: &AtomicU64) -> Result<(std::path::PathBuf, std::fs::File), ToolError> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        if private {
            options.mode(0o600);
        }
    }
    #[cfg(not(unix))]
    let _ = private;
    let destination_name = destination.file_name().and_then(|name| name.to_str());
    for _ in 0..64 {
        let n = counter.fetch_add(1, Ordering::Relaxed);
        let name = format!(".craft-anything.tmp-{}-{n}", std::process::id());
        // Conservatively exclude case-insensitive and Windows trailing-dot/space aliases.
        if destination_name.is_some_and(|destination| destination.trim_end_matches(['.', ' ']).eq_ignore_ascii_case(&name)) {
            continue;
        }
        let tmp = dir.join(name);
        match options.open(&tmp) {
            Ok(file) => return Ok((tmp, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(ToolError::io(tmp.display(), error)),
        }
    }
    let error = std::io::Error::new(std::io::ErrorKind::AlreadyExists, "could not reserve an unused temporary file");
    Err(ToolError::io(dir.display(), error))
}

/// On Windows a file another program has open for a moment (an antivirus scan, the search
/// indexer) can't be replaced: try again briefly before giving up.
fn rename_retrying(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut tries = 0;
    loop {
        match std::fs::rename(from, to) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && tries < 20 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            r => return r,
        }
    }
}

#[cfg(unix)]
fn set_private(path: &Path) -> Result<(), ToolError> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| ToolError::io(path.display(), e))
}

#[cfg(not(unix))]
fn set_private(_path: &Path) -> Result<(), ToolError> {
    // Windows: files under the user's profile are already private to the user by default ACLs.
    Ok(())
}

/// Copy `from` to `to` only if `to` does not exist yet (the first backup is never overwritten).
/// Returns true when a copy was made, false when a backup already existed.
pub fn backup_once(from: &Path, to: &Path) -> Result<bool, ToolError> {
    if to.exists() {
        return Ok(false);
    }
    let bytes = std::fs::read(from).map_err(|e| ToolError::io(from.display(), e))?;
    let mut f = match std::fs::OpenOptions::new().write(true).create_new(true).open(to) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(ToolError::io(to.display(), e)),
    };
    set_private(to)?;
    f.write_all(&bytes).map_err(|e| ToolError::io(to.display(), e))?;
    f.sync_all().map_err(|e| ToolError::io(to.display(), e))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(unix, windows))]
    fn open_test_directory(path: &Path) -> std::fs::File {
        let mut options = std::fs::File::options();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            // FILE_FLAG_BACKUP_SEMANTICS permits an ordinary handle to a directory.
            options.custom_flags(0x0200_0000);
        }
        options.open(path).unwrap()
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn retained_child_handles_do_not_follow_reused_directory_names() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("project");
        std::fs::create_dir_all(root.join("inbox")).unwrap();
        let directory = open_test_directory(&root);
        let original = temporary.path().join("original");
        std::fs::rename(&root, &original).unwrap();
        std::fs::create_dir_all(root.join("inbox")).unwrap();

        let inbox = open_child_directory(&directory, Path::new("inbox")).unwrap();
        std::fs::write(original.join("inbox/feed.lock"), b"existing").unwrap();
        let retained = original.join("retained");
        std::fs::rename(original.join("inbox"), &retained).unwrap();
        std::fs::create_dir(original.join("inbox")).unwrap();
        let mut existing = open_child_file_for_update(&inbox, Path::new("feed.lock"), false).unwrap();
        let mut bytes = Vec::new();
        existing.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"existing");
        assert_eq!(open_child_file_for_update(&inbox, Path::new("feed.lock"), true).unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
        drop(existing);
        std::fs::remove_file(retained.join("feed.lock")).unwrap();
        let mut created = open_child_file_for_update(&inbox, Path::new("feed.lock"), true).unwrap();
        created.write_all(b"created").unwrap();
        drop(created);

        assert_eq!(std::fs::read(retained.join("feed.lock")).unwrap(), b"created");
        assert_eq!(std::fs::read_dir(root.join("inbox")).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(original.join("inbox")).unwrap().count(), 0);
        for name in ["", ".", "..", "../outside", "inbox/feed.lock", "inbox/.", "inbox/"] {
            assert_eq!(open_child_directory(&directory, Path::new(name)).unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
            assert_eq!(open_child_file_for_update(&inbox, Path::new(name), true).unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
        }
    }

    #[cfg(unix)]
    #[test]
    fn relative_children_reject_links_and_special_files_without_blocking() {
        use nix::sys::stat::Mode;
        use std::os::unix::fs::FileTypeExt as _;
        let temporary = tempfile::tempdir().unwrap();
        let directory = open_test_directory(temporary.path());
        let personal = temporary.path().join("personal");
        std::fs::create_dir(&personal).unwrap();
        std::os::unix::fs::symlink(&personal, temporary.path().join("inbox")).unwrap();
        for name in ["inbox", "inbox/.", "inbox/"] {
            assert!(open_child_directory(&directory, Path::new(name)).is_err());
        }
        std::fs::write(personal.join("keep"), b"Keep these words").unwrap();
        std::os::unix::fs::symlink(personal.join("keep"), temporary.path().join("feed.lock")).unwrap();
        assert!(open_child_file_for_update(&directory, Path::new("feed.lock"), false).is_err());
        assert_eq!(std::fs::read(personal.join("keep")).unwrap(), b"Keep these words");
        std::fs::remove_file(temporary.path().join("feed.lock")).unwrap();
        nix::unistd::mkfifo(&temporary.path().join("feed.lock"), Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
        assert_eq!(open_child_file_for_update(&directory, Path::new("feed.lock"), false).unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
        assert!(std::fs::symlink_metadata(temporary.path().join("feed.lock")).unwrap().file_type().is_fifo());
    }

    #[test]
    fn atomic_write_replaces_and_private_is_0600() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub").join("a.txt");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        let q = dir.path().join("secret");
        write_atomic_private(&q, b"k").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(&q).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // no temp files left behind
        assert_eq!(std::fs::read_dir(dir.path().join("sub")).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn temporary_file_creation_permissions_respect_private_flag() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let counter = AtomicU64::new(0);
        let destination = dir.path().join("settings.json");
        let (_, private) = create_temp(dir.path(), &destination, true, &counter).unwrap();
        // Observe the new file directly, before write_impl can call set_private.
        assert_eq!(private.metadata().unwrap().permissions().mode() & 0o077, 0);

        let regular = std::fs::File::create(dir.path().join("regular")).unwrap();
        let (_, non_private) = create_temp(dir.path(), &destination, false, &counter).unwrap();
        assert_eq!(non_private.metadata().unwrap().permissions().mode() & 0o777, regular.metadata().unwrap().permissions().mode() & 0o777);
    }

    #[test]
    fn temporary_candidates_exclude_destination_aliases() {
        let name = format!(".craft-anything.tmp-{}-0", std::process::id());
        for (component, next_counter) in [
            (name.clone(), 2),
            (name.to_ascii_uppercase(), 2),
            (format!("{name}. "), 2),
            (format!("{}. ", name.to_ascii_uppercase()), 2),
            ("settings.json".into(), 1),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let counter = AtomicU64::new(0);
            let destination = dir.path().join(".").join(component);
            let (temporary, file) = create_temp(dir.path(), &destination, false, &counter).unwrap();
            assert!(!destination.exists());
            assert_eq!(counter.load(Ordering::Relaxed), next_counter);
            assert_eq!(temporary, dir.path().join(format!(".craft-anything.tmp-{}-{}", std::process::id(), next_counter - 1)));
            assert_eq!(file.metadata().unwrap().len(), 0);
        }
    }

    #[test]
    fn backup_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("a.bak");
        std::fs::write(&a, "first").unwrap();
        assert!(backup_once(&a, &b).unwrap());
        std::fs::write(&a, "second").unwrap();
        assert!(!backup_once(&a, &b).unwrap());
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "first");
    }
}
