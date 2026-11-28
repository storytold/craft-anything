//! Adapters for optional external engines (Ghidra, adb, Chromium, Frida, JADX…).
//!
//! Every engine is an entry in [`ENGINES`] (see [`table`]). Each is found, in order: the path the
//! user set in Settings → Engines, the managed install in the engine store, `PATH`, then the
//! engine's standard install folders. Nothing here is required for Clean room analysis.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod adb;
pub mod assets;
pub mod cdp;
pub mod cutter;
pub mod frida;
pub mod game;
pub mod gdb;
pub mod ghidra;
pub mod install;
pub mod jadx;
pub mod java;
#[cfg(test)]
mod known_roots_tests;
pub mod mcp_client;
pub mod rizin;
pub mod table;
pub mod version;
#[cfg(feature = "wasm-plugins")]
pub mod wasm_plugin;

pub use table::{ArchiveFormat, ENGINES, ExternalEngine, Install, PackagePath, Pin};
pub use version::version_order;

use std::path::{Path, PathBuf};

/// Cut `line` to at most `max` bytes without splitting a UTF-8 character. The engine parsers cap
/// each line before inspecting hostile output; a plain byte slice can land inside a character, and
/// keeping the whole line then would defeat the cap.
pub(crate) fn capped_line(line: &str, max: usize) -> &str {
    let mut end = max.min(line.len());
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    line.get(..end).unwrap_or_default()
}

/// User overrides, mirrored from the engine settings so this crate doesn't depend on the engine.
pub trait ToolPathLookup {
    fn override_for(&self, id: &str) -> Option<PathBuf>;
}

/// The table entry for an engine id.
pub fn engine(id: &str) -> Option<&'static ExternalEngine> {
    ENGINES.iter().find(|e| e.id == id)
}

/// The ids that have a path override in Settings → Engines, in table order.
pub fn path_setting_ids() -> impl Iterator<Item = &'static str> {
    ENGINES.iter().filter(|e| e.path_setting).map(|e| e.id)
}

pub fn display_name(id: &str) -> &'static str {
    engine(id).map(|e| e.name).unwrap_or("an external tool")
}

/// What to tell the person when an engine wasn't found, as (reason, remediation): `set` is the path
/// in Settings → Engines, which wasn't there or was a folder without the program, and `flatpak`
/// whether this app runs in that sandbox.
pub fn not_found_help(id: &str, set: Option<&Path>, flatpak: bool) -> (String, String) {
    let name = display_name(id);
    // Settings → Engines can only install what has pins; these the person installs themselves.
    let manual = engine(id).is_some_and(|e| matches!(e.install, Install::Manual));
    let program = engine(id).and_then(|e| e.host_exes().first().copied());
    let reason = match set {
        // A folder that's there: what's missing is the program in it.
        Some(p) if p.is_dir() => match program {
            Some(exe) => format!(
                "This needs {name}, and the folder set for it in Settings → Engines ({}) has no {name} program in it (`{exe}`, looked for in the folder, its `bin` folder and the folders inside it).",
                p.display()
            ),
            None => format!("This needs {name}, and the path set for it in Settings → Engines ({}) is a folder, not the program.", p.display()),
        },
        Some(p) => format!("This needs {name}, and the path set for it in Settings → Engines ({}) isn't there.", p.display()),
        None => format!("This needs {name} installed."),
    };
    const CORRECT: &str = "Correct the path in Settings → Engines, or clear it to look in the usual places.";
    let sandbox = (flatpak && manual).then(|| {
        format!(
            "This app is running as a Flatpak, and its sandbox can't see programs in your system's folders, so a path to one won't work. To use {name} from your computer, run the AppImage, .deb, .rpm or tarball version instead."
        )
    });
    let remediation = match (set, sandbox) {
        (Some(_), Some(sandbox)) => format!("{CORRECT} {sandbox}"),
        (Some(_), None) => CORRECT.to_string(),
        (None, Some(sandbox)) => sandbox,
        (None, None) if manual => format!("Install {name}, then set its path in Settings → Engines if it still isn't found."),
        (None, None) => format!("Install {name} from Settings → Engines."),
    };
    (reason, remediation)
}

/// Find an engine: user override, then the managed store (its [`ExternalEngine::store_folder`],
/// `<store>/<id>` unless a guided setup puts it elsewhere), then PATH,
/// then its standard install folders, then a package manager's copy (Homebrew's Ghidra),
/// then the folders where an unpacked copy is usually kept.
pub fn locate(id: &str, overrides: &dyn ToolPathLookup, store: &Path) -> Option<PathBuf> {
    locate_configured(id, overrides, store).or_else(|| {
        let e = engine(id)?;
        find_on_path(e.host_exes())
            .or_else(|| e.known_install_paths().into_iter().find(|p| p.is_file()))
            .or_else(|| find_in_package_paths(e).map(|(p, _)| p))
            .or_else(|| find_in_known_roots(id, e.host_exes(), &e.known_root_folders()))
    })
}

/// Find an engine where the settings or the managed store say only: what [`locate`] finds before
/// it looks around the computer (PATH and standard install folders).
pub fn locate_configured(id: &str, overrides: &dyn ToolPathLookup, store: &Path) -> Option<PathBuf> {
    if let Some(p) = overrides.override_for(id).and_then(|p| in_path_setting(id, &p)) {
        return Some(p);
    }
    let e = engine(id)?;
    let exes = e.host_exes();
    let managed = store.join(e.store_folder());
    if let Some(p) = in_managed(&managed, exes) {
        return Some(p);
    }
    // A single-file install (a .jar) is found by the name its pin saved it under.
    for pin in e.pins() {
        if let ArchiveFormat::File { name } = pin.format
            && managed.join(name).is_file()
        {
            return Some(managed.join(name));
        }
    }
    None
}

/// The program a path set in Settings → Engines names: the file itself, or, for a folder (a
/// JDK's folder, an unzipped JADX), the engine's program inside it as an install folder holds it
/// (see [`in_managed`]), or its single-file install (`vineflower.jar`). `None` when there's no
/// such program, so a folder is never run as the program.
fn in_path_setting(id: &str, p: &Path) -> Option<PathBuf> {
    if p.is_file() {
        return Some(p.to_path_buf());
    }
    if !p.is_dir() {
        return None;
    }
    let e = engine(id)?;
    in_managed(p, e.host_exes()).or_else(|| {
        e.pins().iter().find_map(|pin| match pin.format {
            ArchiveFormat::File { name } => Some(p.join(name)).filter(|f| f.is_file()),
            _ => None,
        })
    })
}

/// An executable in a managed install folder: `<dir>/<exe>`, `<dir>/bin/<exe>` or
/// `<dir>/support/<exe>`, in `dir` itself or in one of its sub-folders, newest version first (an
/// archive unpacks into one top folder, often versioned: `ghidra_12.2_PUBLIC/support/…`; a
/// folder of JDKs holds `java-8-…` beside `java-25-…`).
pub(crate) fn in_managed(dir: &Path, exes: &[&str]) -> Option<PathBuf> {
    let here = |d: &Path| exes.iter().flat_map(|exe| [d.join(exe), d.join("bin").join(exe), d.join("support").join(exe)]).find(|p| p.is_file());
    // A JDK folder may keep its home one `Contents/Home` deeper (macOS, and the guided install
    // unpacks Temurin so), which `java::home_in` resolves.
    let found = |d: &Path| here(d).or_else(|| java::home_in(d).and_then(|home| here(&home)));
    if let Some(p) = found(dir) {
        return Some(p);
    }
    let mut subs: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    subs.sort_by(|a, b| version_order(&a.file_name().unwrap_or_default().to_string_lossy(), &b.file_name().unwrap_or_default().to_string_lossy()));
    subs.iter().rev().find_map(|d| found(d))
}

/// An unpacked install in one of `roots` (each with how many levels down to look): a folder whose
/// name starts with the engine id (`ghidra`, `Ghidra`, `ghidra_12.1.4_PUBLIC`) and holds one of
/// `exes` (see [`in_managed`]). Newer names first. Reads folder names only, never file contents.
pub fn find_in_known_roots(id: &str, exes: &[&str], roots: &[(PathBuf, usize)]) -> Option<PathBuf> {
    roots.iter().find_map(|(root, depth)| search_named(root, id, exes, *depth))
}

fn search_named(dir: &Path, id: &str, exes: &[&str], depth: usize) -> Option<PathBuf> {
    if depth == 0 {
        return None;
    }
    let mut subs: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    subs.sort_by(|a, b| version_order(&a.file_name().unwrap_or_default().to_string_lossy(), &b.file_name().unwrap_or_default().to_string_lossy()));
    subs.iter().rev().find_map(|d| {
        let named = d.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.to_ascii_lowercase().starts_with(id));
        if named && let Some(p) = in_managed(d, exes) {
            return Some(p);
        }
        search_named(d, id, exes, depth - 1)
    })
}

/// Standard install locations for programs that don't put themselves on PATH (browsers on
/// Windows and macOS, Ghidra from Linux packages), most common first.
pub fn well_known(id: &str) -> Vec<PathBuf> {
    engine(id).map(ExternalEngine::known_install_paths).unwrap_or_default()
}

/// A package manager's copy (Homebrew's Ghidra formula): the stable `opt` link first, then the
/// version folders under `Cellar`, newest first. Read-only, and `brew` is never run: the prefixes
/// come from the engine table, and a broken or unreadable link is just skipped. `PATH` plays no
/// part — a formula links only the window launcher (`ghidraRun`), never the headless one. Returns
/// the launcher path and the package manager's name ("Homebrew").
pub fn find_in_package_paths(e: &ExternalEngine) -> Option<(PathBuf, &'static str)> {
    e.host_package_paths().find_map(|p| package_path_at(Path::new(p.prefix), p.package, p.rel).map(|found| (found, p.label)))
}

/// One package location: `<prefix>/opt/<package>/<rel>` when the link is there, else the newest
/// `<prefix>/Cellar/<package>/<version>/<rel>` that is. Failures (missing, broken link, unreadable)
/// leave the candidate out, they never stop the search.
fn package_path_at(prefix: &Path, package: &str, rel: &str) -> Option<PathBuf> {
    let stable = prefix.join("opt").join(package).join(rel);
    if stable.is_file() {
        return Some(stable);
    }
    let cellar = prefix.join("Cellar").join(package);
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&cellar).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    versions.sort_by(|a, b| version_order(&a.file_name().unwrap_or_default().to_string_lossy(), &b.file_name().unwrap_or_default().to_string_lossy()));
    versions.iter().rev().map(|v| v.join(rel)).find(|p| p.is_file())
}

pub(crate) fn find_on_path(exes: &[&str]) -> Option<PathBuf> {
    let path = craft_anything_platform::which::search_path()?;
    for dir in std::env::split_paths(&path) {
        for exe in exes {
            let p = dir.join(exe);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoOverrides;
    impl ToolPathLookup for NoOverrides {
        fn override_for(&self, _: &str) -> Option<PathBuf> {
            None
        }
    }

    #[test]
    fn unknown_engine_is_none() {
        assert!(locate("definitely-not-an-engine", &NoOverrides, Path::new("/nonexistent")).is_none());
        assert_eq!(display_name("adb"), "Android platform tools (adb)");
    }

    /// Asked to install Java, we say Ghidra's setup brings it; that setup unpacks it under
    /// `<store>/jdk`, which the Java runtime engine has to find (it was looked for in `<store>/java`).
    #[test]
    fn the_java_ghidras_setup_brings_is_the_java_runtime() {
        let java = if cfg!(windows) { "java.exe" } else { "java" };
        // The JDK keeps its own top folder and may keep its home one `Contents/Home` deeper (as
        // macOS has it and as the guided install unpacks it); a `jdk/Home` an earlier setup
        // unpacked is still found, so a kept install keeps the Java it had.
        for (top, major) in [("jdk-25.0.5.1+2", "25.0.1"), ("Home", "25.0.1"), ("jdk-25.0.5.1+2/Contents/Home", "25.0.1")] {
            let store = tempfile::tempdir().unwrap();
            let home = store.path().join("jdk").join(top);
            let exe = home.join("bin").join(java);
            std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
            std::fs::write(&exe, "").unwrap();
            std::fs::write(home.join("release"), format!("JAVA_VERSION=\"{major}\"\n")).unwrap();
            assert_eq!(locate_configured("java", &NoOverrides, store.path()), Some(exe.clone()), "{top}");
            // The same Java Ghidra is pointed at.
            assert_eq!(ghidra::get::java_home(None, None, store.path()).map(|h| h.join("bin").join(java)), Some(exe), "{top}");
        }
    }

    /// Ghidra's zip unpacks into `<store>/ghidra/ghidra_<version>_PUBLIC/`, with the launcher in
    /// `support/`; `engines_list` said "not found" for an install that Deep mode was using.
    #[test]
    fn a_managed_install_in_a_versioned_folder_is_found() {
        let store = tempfile::tempdir().unwrap();
        let exe = if cfg!(windows) { "analyzeHeadless.bat" } else { "analyzeHeadless" };
        let launcher = store.path().join("ghidra").join("ghidra_12.1.4_PUBLIC").join("support").join(exe);
        std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        std::fs::write(&launcher, b"x").unwrap();
        assert_eq!(locate("ghidra", &NoOverrides, store.path()), Some(launcher));
        // An older unpack beside it loses to the newest name.
        let newer = store.path().join("ghidra").join("ghidra_12.2.0_PUBLIC").join("support").join(exe);
        std::fs::create_dir_all(newer.parent().unwrap()).unwrap();
        std::fs::write(&newer, b"x").unwrap();
        assert_eq!(locate("ghidra", &NoOverrides, store.path()), Some(newer));
        // Nothing but folders: still none, not a panic.
        let empty = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(empty.path().join("ghidra").join("ghidra_12.1.4_PUBLIC")).unwrap();
        // (Unless this computer has Ghidra on PATH, from a system package, or from Homebrew.)
        let ghidra = engine("ghidra").unwrap();
        let installed = find_on_path(&[exe]).is_some() || well_known("ghidra").iter().any(|p| p.is_file()) || find_in_package_paths(ghidra).is_some();
        assert!(locate("ghidra", &NoOverrides, empty.path()).is_none() || installed);
    }

    struct SetPath(PathBuf);
    impl ToolPathLookup for SetPath {
        fn override_for(&self, _: &str) -> Option<PathBuf> {
            Some(self.0.clone())
        }
    }

    /// A path set to a folder (a JDK's folder for the Java runtime) counted as found, and running
    /// the folder then failed with "permission denied". The program in it is found instead, as an
    /// install folder holds it; a folder without it isn't found, and says so.
    #[test]
    fn a_folder_set_as_an_engines_path_is_looked_in() {
        let java = if cfg!(windows) { "java.exe" } else { "java" };
        let store = tempfile::tempdir().unwrap();
        let d = tempfile::tempdir().unwrap();
        let jdk = d.path().join("jdk-21");
        let exe = jdk.join("bin").join(java);
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "").unwrap();
        // The JDK's folder, or the folder that holds JDKs (`/usr/lib/jvm`).
        assert_eq!(locate_configured("java", &SetPath(jdk.clone()), store.path()), Some(exe.clone()));
        assert_eq!(locate_configured("java", &SetPath(d.path().to_path_buf()), store.path()), Some(exe.clone()));
        // The program itself is taken as it is, as before.
        assert_eq!(locate_configured("java", &SetPath(exe.clone()), store.path()), Some(exe));
        // A single-file install's folder: its jar.
        let jar = d.path().join("vf").join("vineflower.jar");
        std::fs::create_dir_all(jar.parent().unwrap()).unwrap();
        std::fs::write(&jar, "").unwrap();
        assert_eq!(locate_configured("vineflower", &SetPath(jar.parent().unwrap().to_path_buf()), store.path()), Some(jar.clone()));
        assert_eq!(locate_configured("vineflower", &SetPath(jar.clone()), store.path()), Some(jar));
        // A folder without the program isn't found (the empty store holds nothing either)...
        let empty = d.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(locate_configured("java", &SetPath(empty.clone()), store.path()), None);
        // ...and the message says what's missing, not that the folder isn't there.
        let (reason, fix) = not_found_help("java", Some(&empty), false);
        assert!(reason.contains("has no Java") && reason.contains(&format!("`{java}`")) && !reason.contains("isn't there"), "{reason}");
        assert!(fix.starts_with("Correct the path"), "{fix}");
    }

    /// A folder of JDKs (`/usr/lib/jvm`) gives its newest one, not the last by plain text order
    /// (`java-8` sorts after `java-21` as text).
    #[test]
    fn a_folder_of_versions_gives_the_newest() {
        let java = if cfg!(windows) { "java.exe" } else { "java" };
        let store = tempfile::tempdir().unwrap();
        let d = tempfile::tempdir().unwrap();
        for v in ["java-8-openjdk", "java-17-openjdk", "java-21-openjdk"] {
            std::fs::create_dir_all(d.path().join(v).join("bin")).unwrap();
            std::fs::write(d.path().join(v).join("bin").join(java), "").unwrap();
        }
        let want = d.path().join("java-21-openjdk").join("bin").join(java);
        assert_eq!(locate_configured("java", &SetPath(d.path().to_path_buf()), store.path()), Some(want));
        use std::cmp::Ordering::*;
        assert_eq!(version_order("ghidra_12.1.4_PUBLIC", "ghidra_12.10.0_PUBLIC"), Less);
        assert_eq!(version_order("ghidra_12.2.0_PUBLIC", "ghidra_12.1.4_PUBLIC"), Greater);
        assert_eq!(version_order("jdk-021", "jdk-21"), Less);
        assert_eq!(version_order("a", "a"), Equal);
    }

    /// #841: a path set for a browser was ignored without a word, and the message offered an
    /// install that Settings can't do.
    #[test]
    fn a_missing_engine_is_explained_by_why_it_was_not_found() {
        let set = Path::new("/usr/bin/brave");
        let (reason, fix) = not_found_help("chromium", Some(set), true);
        assert!(reason.contains(&set.display().to_string()), "{reason}");
        // The path may only be mistyped, and one in the home folder can work, so correcting it comes first.
        assert!(fix.starts_with("Correct the path") && fix.contains("Flatpak") && fix.contains("AppImage"), "{fix}");
        // A Flatpak can't reach a browser with no path set either, and there's no path to correct.
        let (_, fix) = not_found_help("chromium", None, true);
        assert!(fix.contains("Flatpak") && !fix.contains("Correct the path"), "{fix}");

        let (reason, fix) = not_found_help("chromium", Some(set), false);
        assert!(reason.contains("isn't there") && fix.contains("Correct the path") && !fix.contains("Flatpak"), "{reason} {fix}");

        let (_, fix) = not_found_help("chromium", None, false);
        assert!(fix.starts_with("Install a Chromium-based browser,") && !fix.contains("from Settings"), "{fix}");

        // What Settings can install still says so, and a sandbox doesn't stop it.
        for flatpak in [false, true] {
            assert_eq!(not_found_help("adb", None, flatpak).1, "Install Android platform tools (adb) from Settings → Engines.");
        }
    }

    #[test]
    fn browsers_are_looked_for_under_their_linux_package_names() {
        let exes = engine("chromium").unwrap().exes;
        assert!(["google-chrome-stable", "brave-browser", "microsoft-edge"].iter().all(|x| exes.contains(x)), "{exes:?}");
    }

    #[test]
    fn browsers_have_well_known_locations_off_path() {
        assert!(well_known("adb").is_empty());
        let paths = well_known("chromium");
        if cfg!(any(windows, target_os = "macos")) {
            assert!(paths.iter().any(|p| p.to_string_lossy().contains("Chrome")));
            assert!(paths.iter().any(|p| p.to_string_lossy().contains("Edge")));
        }
        // #85: Linux packages put Ghidra in its own folder with only a start script on PATH.
        let ghidra = well_known("ghidra");
        if cfg!(target_os = "linux") {
            assert_eq!(ghidra, [PathBuf::from("/opt/ghidra/support/analyzeHeadless"), PathBuf::from("/usr/share/ghidra/support/analyzeHeadless")]);
        } else {
            assert!(ghidra.is_empty());
        }
        // Homebrew's Ghidra is a package-manager location, not a standard one: it is found
        // through `find_in_package_paths`, never here.
        if cfg!(target_os = "macos") {
            assert!(ghidra.iter().all(|p| !p.to_string_lossy().contains("homebrew")), "{ghidra:?}");
        }
    }

    /// Homebrew's Ghidra formula: the stable `opt/ghidra/libexec` link first; when that link is
    /// gone, the newest version folder under `Cellar/ghidra/`; a broken link or an unreadable
    /// folder is skipped, never a crash. (Rows are for macOS, so the prefixes are only exercised
    /// through this test on a Mac; the lookup itself takes any prefix.)
    #[test]
    fn a_package_managers_copy_is_found_opt_first_then_cellar() {
        let rel = "libexec/support/analyzeHeadless";
        let d = tempfile::tempdir().unwrap();
        let prefix = d.path().join("homebrew");
        // The `opt` link is hit first, even with an older version in `Cellar`.
        let stable = prefix.join("opt/ghidra").join(rel);
        std::fs::create_dir_all(stable.parent().unwrap()).unwrap();
        std::fs::write(&stable, b"x").unwrap();
        let cellar_older = prefix.join("Cellar/ghidra/12.1.4").join(rel);
        std::fs::create_dir_all(cellar_older.parent().unwrap()).unwrap();
        std::fs::write(&cellar_older, b"x").unwrap();
        assert_eq!(package_path_at(&prefix, "ghidra", rel), Some(stable.clone()));
        // With the `opt` link gone, the newest `Cellar` version wins: 12.10.0 over 12.2 as
        // numbers, which plain text order would get backwards.
        std::fs::remove_file(&stable).unwrap();
        let newer = prefix.join("Cellar/ghidra/12.2").join(rel);
        let newest = prefix.join("Cellar/ghidra/12.10.0").join(rel);
        for at in [&newer, &newest] {
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(at, b"x").unwrap();
        }
        assert_eq!(package_path_at(&prefix, "ghidra", rel), Some(newest));
        // Nothing valid at all: none, not an error.
        std::fs::remove_dir_all(prefix.join("Cellar")).unwrap();
        assert_eq!(package_path_at(&prefix, "ghidra", rel), None);
    }

    /// An `opt` link left pointing at a deleted version is skipped; the `Cellar` copy is used.
    #[cfg(unix)]
    #[test]
    fn a_broken_opt_link_is_skipped_for_the_cellar_copy() {
        let rel = "libexec/support/analyzeHeadless";
        let d = tempfile::tempdir().unwrap();
        let prefix = d.path().join("homebrew");
        let newest = prefix.join("Cellar/ghidra/12.2").join(rel);
        std::fs::create_dir_all(newest.parent().unwrap()).unwrap();
        std::fs::write(&newest, b"x").unwrap();
        std::fs::create_dir_all(prefix.join("opt")).unwrap();
        std::os::unix::fs::symlink(prefix.join("Cellar/ghidra/gone"), prefix.join("opt/ghidra")).unwrap();
        assert_eq!(package_path_at(&prefix, "ghidra", rel), Some(newest), "the broken link is skipped, the Cellar copy is used");
    }

    /// Homebrew links only `ghidraRun` onto PATH — never the headless launcher. A folder holding
    /// just that window launcher must not count as Ghidra, and `PATH` without `analyzeHeadless`
    /// must not stop the package location from being found.
    #[test]
    fn a_homebrew_folder_with_only_ghidrarun_is_not_ghidra() {
        let d = tempfile::tempdir().unwrap();
        let prefix = d.path().join("homebrew");
        let libexec = prefix.join("opt/ghidra/libexec");
        std::fs::create_dir_all(&libexec).unwrap();
        std::fs::write(libexec.join("ghidraRun"), b"#!/bin/sh\n").unwrap();
        std::fs::write(libexec.join("pyghidraRun"), b"#!/bin/sh\n").unwrap();
        assert_eq!(package_path_at(&prefix, "ghidra", "libexec/support/analyzeHeadless"), None);
        // The headless launcher beside them: found, whatever `PATH` says.
        let rel = "libexec/support/analyzeHeadless";
        std::fs::create_dir_all(libexec.join("support")).unwrap();
        std::fs::write(libexec.join("support").join("analyzeHeadless"), b"x").unwrap();
        assert_eq!(package_path_at(&prefix, "ghidra", rel), Some(libexec.join("support").join("analyzeHeadless")));
    }

    /// A Mac with Homebrew's Ghidra installed finds it without any setting; elsewhere this
    /// asserts nothing, so it never depends on the machine running the tests.
    #[test]
    fn ghidra_is_found_from_homebrew_when_installed() {
        let Some(e) = engine("ghidra") else { return };
        let Some((found, label)) = find_in_package_paths(e) else {
            assert!(crate::well_known("ghidra").iter().all(|p| !p.is_file()) || cfg!(not(target_os = "macos")));
            return;
        };
        assert!(found.is_file(), "{found:?}");
        // Either Homebrew prefix: `/opt/homebrew` on Apple Silicon, `/usr/local` on Intel. The
        // found path may lie in `Cellar`, still under the same prefix.
        assert!(e.package_paths.iter().any(|p| found.starts_with(p.prefix)), "{found:?} is not under a Homebrew prefix");
        assert_eq!(label, "Homebrew");
    }
}
