//! Read-only facts about a JDK folder: which of the JDKs on this computer can run a program that
//! needs a given Java version. Nothing here runs `java` or writes anything: the version comes
//! from the JDK's own `release` file, so an engine listing never starts a process.

use std::path::{Path, PathBuf};

/// What a JDK folder is worth for a program needing at least `min`: a known version that is
/// good enough ([`JavaVerdict::Ok`]), a known one that is too old ([`JavaVerdict::TooOld`]), or
/// one we can't read ([`JavaVerdict::Unknown`]) — the caller decides whether that is a hard stop
/// or a "let the program find out" case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JavaVerdict {
    Ok(u32),
    TooOld(u32),
    Unknown,
}

/// The Java executable's name on this computer.
fn java_exe() -> &'static str {
    if cfg!(windows) { "java.exe" } else { "java" }
}

/// Whether `dir` holds a usable Java home: a `bin/java` in it.
fn has_java(dir: &Path) -> bool {
    dir.join("bin").join(java_exe()).is_file()
}

/// The Java home in `dir`: the folder itself when it holds `bin/java`, else its
/// `Contents/Home` (a JDK folder as macOS keeps it, and as the guided install unpacks it).
pub fn home_in(dir: &Path) -> Option<PathBuf> {
    if has_java(dir) {
        return Some(dir.to_path_buf());
    }
    let mac = dir.join("Contents").join("Home");
    has_java(&mac).then_some(mac)
}

/// The Java major version a home runs, read from its `release` file (`JAVA_VERSION="21.0.5"`).
/// The old scheme (`1.8.0_402`) reads as `8`. `None` when there is no readable version to find.
pub fn major(home: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(home.join("release")).ok()?;
    let line = text.lines().find(|l| l.starts_with("JAVA_VERSION="))?;
    let value = line.trim_start_matches("JAVA_VERSION=").trim().trim_matches('"');
    let mut parts = value.split('.');
    let first = parts.next()?.parse().ok()?;
    // `1.8.0_402` was Java 8, not Java 1.
    if first == 1 { parts.next()?.parse().ok() } else { Some(first) }
}

/// Whether `home` can run a program that needs at least `min`.
pub fn verdict(home: &Path, min: u32) -> JavaVerdict {
    match major(home) {
        Some(v) if v >= min => JavaVerdict::Ok(v),
        Some(v) => JavaVerdict::TooOld(v),
        None => JavaVerdict::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jdk(at: &Path, version: &str) -> PathBuf {
        let home = at.join("jdk");
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::write(home.join("bin").join(java_exe()), b"").unwrap();
        std::fs::write(home.join("release"), format!("JAVA_VERSION=\"{version}\"\n")).unwrap();
        home
    }

    #[test]
    fn a_jdk_folder_is_its_own_home() {
        let d = tempfile::tempdir().unwrap();
        let home = jdk(d.path(), "21.0.5");
        assert_eq!(home_in(&home), Some(home.clone()));
        assert_eq!(major(&home), Some(21));
        assert_eq!(verdict(&home, 21), JavaVerdict::Ok(21));
        assert_eq!(verdict(&home, 25), JavaVerdict::TooOld(21));
    }

    /// macOS keeps the JDK one `Contents/Home` deeper, and the guided install unpacks it so.
    #[test]
    fn a_macos_jdk_is_found_through_contents_home() {
        let d = tempfile::tempdir().unwrap();
        let home = jdk(&d.path().join("wrap"), "25.0.1");
        let wrapped = d.path().join("wrap").join("Contents").join("Home");
        std::fs::create_dir_all(wrapped.parent().unwrap()).unwrap();
        std::fs::rename(&home, &wrapped).unwrap();
        assert_eq!(home_in(&d.path().join("wrap")), Some(wrapped.clone()));
        assert_eq!(major(&wrapped), Some(25));
        assert_eq!(verdict(&wrapped, 25), JavaVerdict::Ok(25));
    }

    #[test]
    fn the_old_naming_scheme_reads_as_the_java_it_named() {
        let d = tempfile::tempdir().unwrap();
        let home = jdk(d.path(), "1.8.0_402");
        assert_eq!(major(&home), Some(8));
        assert_eq!(verdict(&home, 8), JavaVerdict::Ok(8));
        assert_eq!(verdict(&home, 9), JavaVerdict::TooOld(8));
    }

    #[test]
    fn an_unreadable_version_is_unknown_not_a_guess() {
        let d = tempfile::tempdir().unwrap();
        let home = jdk(d.path(), "21.0.5");
        std::fs::remove_file(home.join("release")).unwrap();
        assert_eq!(major(&home), None);
        assert_eq!(verdict(&home, 21), JavaVerdict::Unknown);
        // No `bin/java` at all: no home, no verdict.
        assert_eq!(home_in(&d.path().join("nothing")), None);
    }
}
