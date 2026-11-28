//! Who is watching a project's build (#1970). Every window that follows a build (it shows the
//! build's feed and can pause it) holds a shared OS lock on `inbox/watchers.lock` for as long as it
//! follows, and the system releases it however that window's process ends. The build asks "is
//! anyone watching?" by trying the lock whole for a moment ([`watched`]): it can only take it when
//! no follower holds it. A build in a process of its own uses that to pause once nobody has been
//! watching for a while, rather than spend the person's AI with nobody there to see it.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The lock's file name, in the project's inbox/ (one of Craft Anything's own records, which the
/// builder leaves alone).
pub const WATCHERS_LOCK: &str = "watchers.lock";

/// How often, and how long apart, [`Watching::begin`] tries a lock that's busy: [`watched`] takes
/// it whole for a moment to look, and a window starting to follow then must not be refused for it.
const TRIES: u32 = 10;
const PAUSE: Duration = Duration::from_millis(20);

/// A follower's hold on the project's watchers lock, released when dropped.
#[derive(Debug)]
pub struct Watching {
    file: File,
}

impl Drop for Watching {
    /// Let go now, not when the file's last copy closes (see `BuildLock`'s drop): on Unix a program
    /// another thread is starting holds a copy for a moment, and the build would still look watched.
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

impl Watching {
    /// Start watching the project at `root`. Many followers can watch at once (the hold is shared).
    /// `Ok(None)`: the lock stayed taken whole for every try, which only a look holding it far
    /// longer than a look takes would do. The file is never read or removed; it only carries the
    /// lock.
    pub fn begin(root: &Path) -> std::io::Result<Option<Watching>> {
        let path = path(root);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path)?;
        for i in 0..TRIES {
            match file.try_lock_shared() {
                Ok(()) => return Ok(Some(Watching { file })),
                Err(TryLockError::WouldBlock) if i + 1 < TRIES => std::thread::sleep(PAUSE),
                Err(TryLockError::WouldBlock) => return Ok(None),
                Err(TryLockError::Error(e)) => return Err(e),
            }
        }
        Ok(None)
    }
}

/// Where the project at `root` keeps its watchers lock.
pub fn path(root: &Path) -> PathBuf {
    root.join("inbox").join(WATCHERS_LOCK)
}

/// Someone follows the project's build now, in this process or another. It reads the disk, so
/// call it on a worker, never while drawing. A project with no lock file has never been watched,
/// so nobody watches it; a lock that can't be tried for another reason counts as watched, so an
/// odd file system never pauses a build.
pub fn watched(root: &Path) -> bool {
    let Ok(file) = OpenOptions::new().read(true).open(path(root)) else { return false };
    match file.try_lock() {
        // Let go at once, as a follower does (see `Watching`'s drop).
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(TryLockError::WouldBlock) => true,
        Err(TryLockError::Error(e)) => {
            log::debug!("couldn't tell whether anyone watches the build in {}: {e}", root.display());
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_is_watched_while_a_follower_holds_on() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        assert!(!watched(root), "nobody yet, and no file");
        assert!(!path(root).exists(), "looking doesn't create one");
        let first = Watching::begin(root).unwrap().expect("free");
        assert!(watched(root));
        // Two windows at once: each holds its own share.
        let second = Watching::begin(root).unwrap().expect("shared");
        drop(first);
        assert!(watched(root), "the other window still watches");
        drop(second);
        assert!(!watched(root), "released when the last one goes");
        assert!(!watched(root), "and a look doesn't count as watching");
    }

    #[test]
    fn a_look_never_refuses_a_follower() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().to_path_buf();
        drop(Watching::begin(&root).unwrap());
        // The build looks, holding the lock whole for a moment: a window following then still gets it.
        let look = OpenOptions::new().read(true).open(path(&root)).unwrap();
        look.try_lock().unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            drop(look);
        });
        assert!(Watching::begin(&root).unwrap().is_some(), "a look isn't in the way for long");
        release.join().unwrap();
    }
}
