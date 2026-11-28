//! File safety: bounded regular-file reads and atomic writes (temp file, fsync, rename),
//! optionally private (0600 on Unix).

use craft_anything_evidence::{ErrorCode, ToolError};
use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

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

/// Chunk size for bounded file reads so growth stays predictable without a giant stack buffer.
const READ_CHUNK_BYTES: usize = 64 * 1024;

/// Build the `OverLimit` error returned when a capped read cannot reserve its buffer.
fn reserve_over_limit(path: &Path, requested: u64, max: u64) -> ToolError {
    ToolError::new(ErrorCode::OverLimit, format!("{} needs a {requested} byte buffer that could not be reserved (limit is {max})", path.display()))
        .with_remediation("Free memory or lower the size limit parameter.")
}

/// Build the `OverLimit` error returned when a stream yields more than `max` bytes.
fn read_over_limit(path: &Path, max: u64) -> ToolError {
    ToolError::new(ErrorCode::OverLimit, format!("{} contains more than {max} bytes; the read limit was reached", path.display()))
        .with_remediation("Raise the size limit parameter if you really want to read it.")
}

/// Read up to `max` bytes from `reader`, pre-reserving up to `size_hint.min(max)` and checking
/// buffer reservations before every growth step so allocation failure returns `OverLimit` instead
/// of aborting. Rejects streams that contain more than `max` bytes without overflowing `max`.
fn read_capped_with_hint<R: std::io::Read>(reader: &mut R, path: &Path, size_hint: u64, max: u64) -> Result<Vec<u8>, ToolError> {
    let initial_hint = size_hint.min(max);
    let initial = usize::try_from(initial_hint).map_err(|_| reserve_over_limit(path, initial_hint, max))?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(initial).map_err(|_| reserve_over_limit(path, initial_hint, max))?;

    let mut chunk = [0u8; READ_CHUNK_BYTES];
    let mut total: u64 = 0;
    loop {
        let remaining = max.saturating_sub(total);
        let to_read = if remaining == 0 { 1 } else { usize::try_from(remaining).unwrap_or(chunk.len()).min(chunk.len()) };
        let Some(buf) = chunk.get_mut(..to_read) else {
            return Err(ToolError::io(path.display(), std::io::Error::other("read chunk slice out of bounds")));
        };
        match reader.read(buf) {
            Ok(0) => return Ok(bytes),
            Ok(n) => {
                if remaining == 0 {
                    return Err(read_over_limit(path, max));
                }
                let next_total = total.checked_add(n as u64).ok_or_else(|| read_over_limit(path, max))?;
                if next_total > max {
                    return Err(read_over_limit(path, max));
                }
                let Some(slice) = chunk.get(..n) else {
                    return Err(ToolError::io(path.display(), std::io::Error::other("reader returned more bytes than requested")));
                };
                if bytes.capacity().saturating_sub(bytes.len()) < n {
                    let max_more = usize::try_from(remaining).unwrap_or(usize::MAX);
                    let grow = bytes.len().max(n).min(max_more);
                    bytes.try_reserve_exact(grow).or_else(|_| bytes.try_reserve_exact(n)).map_err(|_| reserve_over_limit(path, next_total, max))?;
                }
                bytes.extend_from_slice(slice);
                total = next_total;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(ToolError::io(path.display(), error)),
        }
    }
}

/// Read a whole regular file, enforcing `max` on the bytes read as well as the size metadata.
/// Reserves the buffer fallibly before reading and before growing, so an unsatisfiable buffer
/// returns `OverLimit` instead of aborting the process.
pub fn read_file_capped(path: &Path, max: u64) -> Result<Vec<u8>, ToolError> {
    let (mut file, metadata) = open_regular_file(path)?;
    if metadata.len() > max {
        return Err(ToolError::new(ErrorCode::OverLimit, format!("{} is {} bytes; the limit is {max}", path.display(), metadata.len()))
            .with_remediation("Raise the size limit parameter if you really want to read it."));
    }
    read_capped_with_hint(&mut file, path, metadata.len(), max)
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

    /// Guards that an unsatisfiable buffer reservation returns `OverLimit` naming the file
    /// rather than aborting on allocation failure, and that `size_hint` is clamped to `max`.
    #[test]
    fn capped_read_returns_over_limit_naming_the_file_when_buffer_reservation_fails() {
        let path = Path::new("huge-target.bin");
        for (size_hint, max) in [(isize::MAX as u64, isize::MAX as u64), (usize::MAX as u64, usize::MAX as u64), (u64::MAX, u64::MAX)] {
            let mut reader = std::io::Cursor::new(b"small".as_slice());
            let error = read_capped_with_hint(&mut reader, path, size_hint, max).unwrap_err();
            assert_eq!(error.code, ErrorCode::OverLimit);
            assert!(error.message.contains("huge-target.bin"), "expected file name in error: {}", error.message);
        }
        // A huge hint above `max` must clamp to `max` instead of failing reservation.
        let mut clamped_reader = std::io::Cursor::new(b"small".as_slice());
        assert_eq!(read_capped_with_hint(&mut clamped_reader, path, u64::MAX, 5).unwrap(), b"small");
    }

    /// Guards empty files, normal files, exact-limit files, over-limit files, and non-regular paths.
    #[test]
    fn capped_read_handles_empty_exact_limit_over_limit_and_non_regular_paths() {
        let dir = tempfile::tempdir().unwrap();

        let empty = dir.path().join("empty.bin");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(read_file_capped(&empty, 0).unwrap(), b"");
        assert_eq!(read_file_capped(&empty, 16).unwrap(), b"");

        let exact = dir.path().join("exact.bin");
        let payload = b"hello world!";
        std::fs::write(&exact, payload).unwrap();
        assert_eq!(read_file_capped(&exact, payload.len() as u64).unwrap(), payload);
        assert_eq!(read_file_capped(&exact, payload.len() as u64 + 8).unwrap(), payload);

        let too_big = read_file_capped(&exact, payload.len() as u64 - 1).unwrap_err();
        assert_eq!(too_big.code, ErrorCode::OverLimit);
        assert!(too_big.message.contains("exact.bin"), "expected file name in error: {}", too_big.message);

        let dir_err = read_file_capped(dir.path(), 1024).unwrap_err();
        assert_eq!(dir_err.code, ErrorCode::BadInput);
    }

    /// Guards streaming growth beyond a zero metadata hint, exact cap acceptance, and +1 byte rejection.
    #[test]
    fn capped_read_streams_past_zero_size_hint_and_rejects_one_byte_past_cap() {
        let payload: Vec<u8> = (0..(READ_CHUNK_BYTES * 2 + 17)).map(|i| (i % 251) as u8).collect();
        let path = Path::new("growing.bin");

        let mut exact_reader = std::io::Cursor::new(payload.as_slice());
        let got = read_capped_with_hint(&mut exact_reader, path, 0, payload.len() as u64).unwrap();
        assert_eq!(got, payload);

        let mut over_reader = std::io::Cursor::new(payload.as_slice());
        let err = read_capped_with_hint(&mut over_reader, path, 0, (payload.len() - 1) as u64).unwrap_err();
        assert_eq!(err.code, ErrorCode::OverLimit);
        assert!(err.message.contains("growing.bin"), "expected file name in error: {}", err.message);

        let mut zero_cap_reader = std::io::Cursor::new(b"x".as_slice());
        let zero_err = read_capped_with_hint(&mut zero_cap_reader, path, 0, 0).unwrap_err();
        assert_eq!(zero_err.code, ErrorCode::OverLimit);
        assert!(zero_err.message.contains("growing.bin"), "expected file name in error: {}", zero_err.message);
    }

    /// Reader wrapper that yields one `Interrupted` error before delegating to the inner reader.
    struct InterruptedOnce<R> {
        inner: R,
        interrupted: bool,
    }

    impl<R: std::io::Read> std::io::Read for InterruptedOnce<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "simulated interrupt"));
            }
            self.inner.read(buf)
        }
    }

    /// Reader that always returns a non-`Interrupted` I/O error.
    struct FailingReader;

    impl std::io::Read for FailingReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("simulated disk read failure"))
        }
    }

    /// Guards that transient `Interrupted` reads are retried while other I/O errors fail cleanly.
    #[test]
    fn capped_read_retries_after_interrupted_read() {
        let mut reader = InterruptedOnce { inner: std::io::Cursor::new(b"resumed".as_slice()), interrupted: false };
        let got = read_capped_with_hint(&mut reader, Path::new("retry.bin"), 0, 64).unwrap();
        assert_eq!(got, b"resumed");

        let mut failing = FailingReader;
        let err = read_capped_with_hint(&mut failing, Path::new("broken.bin"), 0, 64).unwrap_err();
        assert_eq!(err.code, ErrorCode::FileError);
        assert!(err.message.contains("broken.bin"), "expected file name in error: {}", err.message);
    }
}
