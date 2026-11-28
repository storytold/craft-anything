//! Finding Ghidra and the Java it runs on, and getting both for the person once they agree:
//! pinned releases, each checked against its publisher's SHA-256 before anything is unpacked.

use craft_anything_evidence::{ErrorCode, ToolError};
use craft_anything_platform::process::Cancel;
use std::path::{Path, PathBuf};

use super::{Config, GhidraError};
use crate::java::{self, JavaVerdict};
use crate::version::{Version, parse as version_parse, version_order};

/// Ghidra's engine id, as the engine table, Settings and every listing name it.
pub const ID: &str = "ghidra";
/// The engine store folder the Java for Ghidra is unpacked into; the Java runtime engine is found
/// there too (the engine table's `Install::Guided::folder`).
pub const JAVA_FOLDER: &str = "jdk";
/// The Ghidra release Craft Anything installs (it needs Java 25).
pub const VERSION: &str = "12.2";
/// What the person downloads, for the question we ask first (Ghidra 543 MB + Java 120–140 MB,
/// rounded up to the next 10 MB).
pub const DOWNLOAD_MB: u32 = 700;
/// Ghidra 12.2 runs on Java 25; a Java below that is refused before Ghidra starts, not hours
/// into an analysis.
pub const JAVA_MAJOR: u32 = 25;
/// What a kept legacy install (12.1.4) needs at least, while it may still be used.
pub const LEGACY_JAVA_MAJOR: u32 = 21;
/// Whether a Ghidra older than [`VERSION`] may still be used: `AllowWithHint` keeps it usable
/// and says a newer one exists; `Refuse` stops it. One constant, one policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyPolicy {
    AllowWithHint,
    Refuse,
}
pub const LEGACY_POLICY: LegacyPolicy = LegacyPolicy::AllowWithHint;
/// Where to get Ghidra by hand when we can't install it here.
pub const PAGE: &str = "https://github.com/NationalSecurityAgency/ghidra/releases";

/// The version below which an install counts as legacy: [`VERSION`].
pub fn min_current() -> Version {
    // An unparsable VERSION would leave nothing below it; `pins_come_from_the_engine_table` guards the parse.
    Version(version_parse(VERSION).map(|v| v.0).unwrap_or_default())
}

/// Whether a version hint names a Ghidra older than [`VERSION`]. A name without a readable
/// version is never assumed old: it gets the current Java floor and the current verdict.
fn is_below_current(hint: Option<&Version>) -> bool {
    hint.is_some_and(|v| *v < min_current())
}

/// Whether a folder name says a release this new or newer. A name without a readable version
/// can't be told from an older one, so it never counts as this one — only for deciding whether
/// the guided setup may keep what it finds, never for refusing to start Ghidra.
fn is_at_least_current(hint: Option<&Version>) -> bool {
    hint.is_some_and(|v| *v >= min_current())
}

/// The Java floor a Ghidra of this version needs: 25 for this release and anything of unknown
/// version, 21 for a kept legacy install.
fn java_floor(hint: Option<&Version>) -> u32 {
    if is_below_current(hint) { LEGACY_JAVA_MAJOR } else { JAVA_MAJOR }
}

/// The host's pin for a guided engine (`ghidra`, `java`) from the engine table.
fn pin(id: &str) -> Option<&'static crate::Pin> {
    crate::engine(id).and_then(crate::ExternalEngine::host_pin)
}

/// The executables to look for on this computer.
fn host_exes() -> &'static [&'static str] {
    crate::engine(ID).map(crate::ExternalEngine::host_exes).unwrap_or(&[])
}

/// Whether [`install`] works on this computer: Ghidra and its Java both have a pin here (Java is
/// pinned for Windows x64 and for Linux and macOS on x64 and arm64).
pub fn can_install() -> bool {
    pin(ID).is_some() && pin("java").is_some()
}

/// Where a found Ghidra comes from, shown in listings and the doctor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhidraSource {
    /// The folder set in Settings → Engines.
    UserFolder,
    /// The managed install in the engine store.
    EngineStore,
    /// An `analyzeHeadless` on PATH.
    Path,
    /// A system package's folder (`/opt/ghidra` on Arch, `/usr/share/ghidra` on Debian).
    StandardLocation,
    /// A package manager's copy, from the engine table's package paths (`Homebrew`).
    PackageManager(&'static str),
    /// An unpacked copy in a usual place (Downloads, the home folder…).
    UnpackedFolder,
}

impl GhidraSource {
    /// Where this source says the install came from, for a listing's detail line.
    pub fn label(&self) -> String {
        match self {
            GhidraSource::UserFolder => "the folder set in Settings → Engines".into(),
            GhidraSource::EngineStore => "the engine store".into(),
            GhidraSource::Path => "PATH".into(),
            GhidraSource::StandardLocation => "a standard install folder".into(),
            GhidraSource::PackageManager(who) => (*who).to_string(),
            GhidraSource::UnpackedFolder => "an unpacked copy".into(),
        }
    }
}

/// A Ghidra found on this computer. `version_hint` is read from folder names alone: it decides
/// which Java floor applies and what the status shows; the version recorded as evidence is the
/// one Ghidra itself reports once it answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhidraInstall {
    /// The headless launcher, ready to run.
    pub launcher: PathBuf,
    pub source: GhidraSource,
    pub version_hint: Option<Version>,
}

/// Whether Ghidra is usable, and when it isn't, why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhidraState {
    /// Found, with Java good enough for its version.
    Ready,
    /// A legacy install (below `12.2`) kept in use, with a hint that a newer one exists.
    Legacy,
    /// A legacy install refused (only under `LegacyPolicy::Refuse`).
    UnsupportedVersion,
    /// A folder for Ghidra is there but holds no headless launcher.
    Incomplete,
    /// Found, but the Java chosen for it (a `JAVA_HOME_OVERRIDE` beside it, or the one set for
    /// it) is known to be too old: Ghidra would be refused before it starts.
    JavaTooOld,
    /// Nothing found.
    NotFound,
}

impl GhidraState {
    /// Whether Deep mode may run Ghidra in this state.
    pub fn usable(&self) -> bool {
        matches!(self, GhidraState::Ready | GhidraState::Legacy)
    }
}

/// The one verdict on Ghidra that every listing shows: Settings → Engines, `engines_list`, the
/// doctor and `Requires::Ghidra` all take theirs from here, so none of them can disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhidraStatus {
    pub state: GhidraState,
    pub install: Option<GhidraInstall>,
    pub java: GhidraJava,
}

/// The version a launcher's folder name carries: the launcher sits in `support/`, so the
/// install's top folder is above it — its own name for a release (`ghidra_12.2_PUBLIC`), or one
/// level higher only for a package manager's `libexec/support/…` layout
/// (`Cellar/ghidra/12.2/libexec/support/analyzeHeadless`, which is where Homebrew's `opt/ghidra`
/// link leads). Links are resolved first, so a link's own name never stands in for the version,
/// and any other parent's name (`user1`, say) is never read as one.
fn version_hint_of(launcher: &Path) -> Option<Version> {
    let launcher = std::fs::canonicalize(launcher).unwrap_or_else(|_| launcher.to_path_buf());
    let root = launcher.parent()?.parent()?;
    if let Some(v) = version_parse(&root.file_name()?.to_string_lossy()) {
        return Some(v);
    }
    if root.file_name()? != "libexec" {
        return None;
    }
    version_parse(&root.parent()?.file_name()?.to_string_lossy())
}

fn install_of(launcher: PathBuf, source: GhidraSource) -> GhidraInstall {
    let version_hint = version_hint_of(&launcher);
    GhidraInstall { launcher, source, version_hint }
}

/// The one place Ghidra is looked for, in a fixed order — the folder the person picked, the
/// engine store (newest version first), PATH, standard install folders, a package manager's
/// copy, then the usual unpacked places — stopping at the first hit. Within one source the
/// newest version wins, by the numbers (`12.10.0` beats `12.2`, which text order would get
/// backwards). Read-only: nothing is run, nothing is written.
pub fn locate_install(install_dir: Option<&Path>, store: &Path) -> Option<GhidraInstall> {
    locate_install_limited(install_dir, store, true)
}

/// [`locate_install`], but `look_around = false` keeps to what the settings and the managed
/// store say: a test session doesn't look around the computer.
pub fn locate_install_limited(install_dir: Option<&Path>, store: &Path, look_around: bool) -> Option<GhidraInstall> {
    if let Some(dir) = install_dir {
        let found = if dir.is_file() { launcher_from_file(dir) } else { in_root_or_child(dir, launcher) };
        if let Some(launcher) = found {
            return Some(install_of(launcher, GhidraSource::UserFolder));
        }
    }
    if let Some(launcher) = crate::in_managed(&store.join(ID), host_exes()) {
        return Some(install_of(launcher, GhidraSource::EngineStore));
    }
    if !look_around {
        return None;
    }
    if let Some(launcher) = crate::find_on_path(host_exes()) {
        return Some(install_of(launcher, GhidraSource::Path));
    }
    if let Some(launcher) = crate::well_known(ID).into_iter().find(|p| p.is_file()) {
        return Some(install_of(launcher, GhidraSource::StandardLocation));
    }
    if let Some((launcher, who)) = crate::engine(ID).and_then(crate::find_in_package_paths) {
        return Some(install_of(launcher, GhidraSource::PackageManager(who)));
    }
    let e = crate::engine(ID)?;
    crate::find_in_known_roots(ID, e.host_exes(), &e.known_root_folders()).map(|launcher| install_of(launcher, GhidraSource::UnpackedFolder))
}

/// Whether Ghidra is usable on this computer, and when it isn't, why: the one verdict that
/// `engines_list`, the Settings page, the doctor and `Requires::Ghidra` all show. `configured`
/// is the Java folder set for Ghidra in Settings (else the managed JDK is used, as
/// `Settings::ghidra_java` does). Read-only, deterministic, and it never starts a process — the
/// Java version comes from the JDK's own `release` file.
pub fn status(install_dir: Option<&Path>, configured: Option<&Path>, store: &Path) -> GhidraStatus {
    status_limited(install_dir, configured, store, true)
}

/// [`status`], but `look_around = false` keeps to what the settings and the managed store say: a
/// test session doesn't look around the computer.
pub fn status_limited(install_dir: Option<&Path>, configured: Option<&Path>, store: &Path, look_around: bool) -> GhidraStatus {
    let inherited = std::env::var_os("JAVA_HOME").map(PathBuf::from);
    status_with(install_dir, configured, inherited.as_deref(), store, look_around)
}

/// [`status_limited`] with the `JAVA_HOME` this program has handed in, so a test can say where the
/// inherited Java is instead of reading the environment.
fn status_with(install_dir: Option<&Path>, configured: Option<&Path>, inherited: Option<&Path>, store: &Path, look_around: bool) -> GhidraStatus {
    let Some(install) = locate_install_limited(install_dir, store, look_around) else {
        // A folder that is there but holds no launcher is an incomplete install, not a missing
        // one: the person sees which folder was checked, not just "nothing".
        let incomplete = install_dir.is_some_and(Path::is_dir)
            || store.join(ID).is_dir()
            || look_around
                && crate::engine(ID).is_some_and(|e| e.package_paths.iter().any(|p| Path::new(p.prefix).join("opt").join(p.package).join("libexec").is_dir()));
        let state = if incomplete { GhidraState::Incomplete } else { GhidraState::NotFound };
        return GhidraStatus { state, install: None, java: GhidraJava::Missing };
    };
    // The Java a launch would actually use: the one set for Ghidra, else the managed JDK a guided
    // install brought for that Ghidra's version — the same fallback `Settings::ghidra_java()`
    // hands to Ghidra.
    let effective = java_home(Some(&install.launcher), configured, store);
    let java = java_in_use(&install.launcher, effective.as_deref(), inherited);
    // Only a Java chosen for Ghidra — a `JAVA_HOME_OVERRIDE` beside it, or the one set for it —
    // refuses it before it starts: the `JAVA_HOME` this program inherits and a `java` on PATH may
    // be replaced by a wrapper launcher, so Ghidra gets the last word on those. The same line
    // `check_java` draws, so the verdict never refuses what a launch would allow.
    let chosen = explicit_java(&install.launcher, effective.as_deref()).is_some();
    let below = is_below_current(install.version_hint.as_ref());
    if below && LEGACY_POLICY == LegacyPolicy::Refuse {
        return GhidraStatus { state: GhidraState::UnsupportedVersion, install: Some(install), java };
    }
    let state = match &java {
        GhidraJava::TooOld { .. } | GhidraJava::NotJava(_) if chosen => GhidraState::JavaTooOld,
        _ if below => GhidraState::Legacy,
        _ => GhidraState::Ready,
    };
    GhidraStatus { state, install: Some(install), java }
}

/// Ghidra's headless launcher: whatever [`locate_install`] finds first, in the folder the person
/// picked, the managed install, PATH, a standard install folder, a package manager's copy or the
/// folders where an unpacked copy is usually kept (#740).
pub fn headless(install_dir: Option<&Path>, store: &Path) -> Option<PathBuf> {
    locate_install(install_dir, store).map(|i| i.launcher)
}

/// Ghidra's headless launcher in the folder the person picked or the managed install only: what
/// [`headless`] finds before it looks around the computer.
pub fn headless_configured(install_dir: Option<&Path>, store: &Path) -> Option<PathBuf> {
    if let Some(dir) = install_dir {
        let found = if dir.is_file() { launcher_from_file(dir) } else { in_root_or_child(dir, launcher) };
        if found.is_some() {
            return found;
        }
    }
    crate::in_managed(&store.join(ID), host_exes())
}

/// Longest start script read when following one to its Ghidra folder.
const SCRIPT_CAP: u64 = 16 * 1024;
/// Most paths a start script names that are looked at on disk.
const SCRIPT_PATHS: usize = 32;

/// Ghidra's headless launcher from a file the person picked as Ghidra's folder. Whatever is returned
/// is run as the launcher, so it is only ever a file named like Ghidra's own `support/analyzeHeadless`
/// or a short text file that names one by its absolute path (whether it runs it isn't checked):
/// - the file itself when it is that launcher (by name, as given, even through a link), or a script
///   naming that launcher (a wrapper that sets up Java first, say);
/// - else the launcher of the Ghidra folder the file is in, links followed (`ghidraRun` on PATH as
///   a link);
/// - else, for a start script that names Ghidra's folder (`exec /opt/ghidra/ghidraRun "$@"`), the
///   launcher of the Ghidra folder it names. (Arch's package links `/usr/bin/ghidra` to `ghidraRun`
///   instead, which the case above covers.)
///
/// Anything else is `None`, so the setting isn't listed as Ghidra.
fn launcher_from_file(file: &Path) -> Option<PathBuf> {
    let exes = host_exes();
    let is_launcher = |p: &Path| p.file_name().is_some_and(|n| exes.iter().any(|exe| n == *exe));
    let real = std::fs::canonicalize(file).ok()?;
    if is_launcher(file) || is_launcher(&real) {
        return Some(file.to_path_buf());
    }
    // A script that runs the launcher is kept before anything else, even in Ghidra's own folder.
    let named = named_in_script(&real).unwrap_or_default();
    if named.iter().any(|p| is_launcher(p) && p.is_file()) {
        return Some(file.to_path_buf());
    }
    let folder_of = |p: &Path| p.parent().and_then(|dir| launcher(dir).or_else(|| dir.parent().and_then(launcher)));
    // The path as given first, then where its links lead: on Windows `canonicalize` adds `\\?\`,
    // which batch files handle badly.
    let followed = |p: &Path| folder_of(p).or_else(|| folder_of(&std::fs::canonicalize(p).ok()?));
    if let Some(found) = folder_of(file).or_else(|| folder_of(&real)) {
        return Some(found);
    }
    named.iter().find_map(|p| if p.is_dir() { launcher(p) } else { followed(p) })
}

/// The absolute paths a short text file names that are on disk, as a start script names Ghidra's
/// folder (`None` when it is too long or isn't text). A quoted path is taken whole, spaces and all.
/// One level only: a script it names isn't read in turn.
fn named_in_script(file: &Path) -> Option<Vec<PathBuf>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(file).ok()?.take(SCRIPT_CAP + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > SCRIPT_CAP || bytes.contains(&0) {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    // Between quotes first (odd pieces of a split on that quote), then word by word.
    let quoted = ['"', '\''].into_iter().flat_map(|q| text.split(q).skip(1).step_by(2));
    // `$GHIDRA_HOME/ghidraRun` after `GHIDRA_HOME=/opt/ghidra`: the assigned folder is named too.
    let words = text.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '=' | ';' | '`' | '(' | ')'));
    let mut paths: Vec<PathBuf> = Vec::new();
    for p in quoted.chain(words).map(PathBuf::from).filter(|p| p.is_absolute()) {
        if !paths.contains(&p) {
            paths.push(p);
        }
        if paths.len() == SCRIPT_PATHS {
            break;
        }
    }
    paths.retain(|p| p.exists());
    Some(paths)
}

/// The Java runtime to run the Ghidra at `launcher` with: the one the person set (its fitness for
/// that Ghidra's version is judged before Ghidra starts, in [`check_java`]), else the managed one
/// with the highest major version that is new enough for it — 25 for this release, 21 for a kept
/// legacy install, so a 12.1.4 beside a 12.2 keeps the managed Java it needs. `None` lets Ghidra
/// look for its own.
pub fn java_home(launcher: Option<&Path>, java_home: Option<&Path>, store: &Path) -> Option<PathBuf> {
    let floor = java_floor(launcher.and_then(version_hint_of).as_ref());
    java_home.filter(|p| p.is_dir()).map(Path::to_path_buf).or_else(|| managed_java(store, floor))
}

/// The managed Java with the highest major version that is at least `min`: the guided install
/// unpacks one, and an older one beside it (left by an earlier Ghidra) must never be picked
/// over it — 12.2 on Java 21 fails hours after it starts, or before it does.
fn managed_java(store: &Path, min: u32) -> Option<PathBuf> {
    let jdk = store.join(JAVA_FOLDER);
    let mut homes: Vec<(u32, PathBuf)> =
        std::fs::read_dir(&jdk).ok()?.flatten().map(|e| e.path()).filter_map(|d| java::home_in(&d)).filter_map(|h| java::major(&h).map(|m| (m, h))).collect();
    homes.sort_by(|a, b| {
        b.0.cmp(&a.0).then_with(|| {
            version_parse(&a.1.file_name().unwrap_or_default().to_string_lossy()).cmp(&version_parse(&b.1.file_name().unwrap_or_default().to_string_lossy()))
        })
    });
    homes.into_iter().find(|(m, _)| *m >= min).map(|(_, h)| h)
}

/// Download, check and unpack Java (unless one new enough is already managed) and Ghidra (unless
/// one this new or newer is already managed or outside the store), then point Ghidra at that
/// Java. `explicit_java` is the Java folder set for Ghidra in Settings: an outside copy that would
/// run on a too-old Java isn't reused, because only a store install can be pointed at the Java the
/// setup brings. `progress` gets (what, bytes so far, total); a step with no bytes (0, `None`) is
/// setting up. Returns the headless launcher. Only ever writes inside `store`.
pub fn install(store: &Path, explicit_java: Option<&Path>, cancel: Option<Cancel>, progress: &dyn Fn(&str, u64, Option<u64>)) -> Result<PathBuf, ToolError> {
    // Ghidra this new or newer is already on this computer, outside our store (PATH, a system
    // package, Homebrew, an unzipped copy in Downloads…): use it rather than download the same
    // release again (#740). An older copy is not reused (the person asked for this release), and
    // neither is one whose Java is known to be too old or missing: writing a Java override into
    // another program's folder isn't ours to do, so the store copy is set up instead.
    let inherited = std::env::var_os("JAVA_HOME").map(PathBuf::from);
    if let Some(found) = locate_install(None, store).filter(|i| reusable_outside(i, store, explicit_java, inherited.as_deref())) {
        return Ok(found.launcher);
    }
    let (Some(_), Some(_)) = (pin(ID), pin("java")) else {
        return Err(ToolError::new(ErrorCode::NotSupported, "Ghidra can't be installed for you on this computer.")
            .with_remediation(format!("Install Ghidra {VERSION} from {PAGE}, then choose its folder in Settings → Engines.")));
    };
    let java = ensure_java(store, cancel.clone(), progress)?;
    let exe = ensure_ghidra(store, cancel, progress)?;
    if let Some(support) = exe.parent() {
        point_at_java(&support.join("launch.properties"), &java)?;
    }
    Ok(exe)
}

/// Whether an install outside our store can be used as it is: a folder that names this release or
/// newer, and a Java its launcher would really run on that is good enough. A too-old or missing
/// Java isn't ours to change in another program's folder (`brew`'s, say), so the store copy is set
/// up instead — which is also the recovery "Get Ghidra" offers for a Java too old for an outside
/// Ghidra. A folder that names no version can't be told from an older one, so it isn't reused: the
/// person asked for this release. The Java is the one *this candidate* would get — the folder set
/// in Settings, else the managed JDK its own version needs — not one resolved for another Ghidra.
fn reusable_outside(install: &GhidraInstall, store: &Path, explicit: Option<&Path>, inherited: Option<&Path>) -> bool {
    let configured = java_home(Some(&install.launcher), explicit, store);
    !install.launcher.starts_with(store)
        && is_at_least_current(install.version_hint.as_ref())
        && can_start_on(&java_in_use(&install.launcher, configured.as_deref(), inherited))
}

/// Whether a Ghidra can start on the Java it would run on: anything but one known to be too old,
/// a folder set as its Java with none in it, or no Java at all — a store install is the one we can
/// point at the Java the setup brings, so an outside copy that can't start isn't reused.
fn can_start_on(java: &GhidraJava) -> bool {
    !matches!(java, GhidraJava::TooOld { .. } | GhidraJava::NotJava(_) | GhidraJava::Missing)
}

/// A managed Java new enough for Ghidra: the one already there, or a freshly downloaded Temurin
/// 25. A managed Java below the floor (left by an earlier Ghidra) is replaced, not reused.
fn ensure_java(store: &Path, cancel: Option<Cancel>, progress: &dyn Fn(&str, u64, Option<u64>)) -> Result<PathBuf, ToolError> {
    if let Some(home) = managed_java(store, JAVA_MAJOR) {
        return Ok(home);
    }
    let Some(java) = pin("java") else {
        return Err(ToolError::new(ErrorCode::NotSupported, format!("Ghidra {VERSION} needs Java {JAVA_MAJOR}, and none can be installed for you here."))
            .with_remediation(format!("Install a JDK {JAVA_MAJOR} yourself, then choose its folder in Settings → Engines.")));
    };
    fetch(java, store, JAVA_FOLDER, &format!("Java {JAVA_MAJOR} runtime"), cancel, progress)?;
    managed_java(store, JAVA_MAJOR).ok_or_else(|| ToolError::new(ErrorCode::GarbledOutput, "Java was unpacked, but there was no usable Java home in it."))
}

/// The managed Ghidra: the one already unpacked with a folder naming this version or newer, or a
/// freshly downloaded one. An older managed install is kept as it is and left untouched; this
/// release lands beside it, and the Java that runs it is the one [`ensure_java`] made sure of. A
/// folder that names no version can't be told from an older one, so it isn't kept either.
fn ensure_ghidra(store: &Path, cancel: Option<Cancel>, progress: &dyn Fn(&str, u64, Option<u64>)) -> Result<PathBuf, ToolError> {
    // The version check goes to the candidates, not to whichever launcher a plain lookup returns
    // first: a copy kept directly in the folder names no version, so it is neither kept as this
    // release nor allowed to hide a versioned one beside it.
    let kept = || in_child(&store.join(ID), |d| crate::in_managed(d, host_exes())).filter(|l| is_at_least_current(version_hint_of(l).as_ref()));
    if let Some(found) = kept() {
        return Ok(found);
    }
    let Some(ghidra) = pin(ID) else {
        return Err(ToolError::new(ErrorCode::NotSupported, "Ghidra can't be installed for you on this computer.")
            .with_remediation(format!("Install Ghidra {VERSION} from {PAGE}, then choose its folder in Settings → Engines.")));
    };
    fetch(ghidra, store, ID, "Ghidra", cancel, progress)?;
    kept().ok_or_else(|| ToolError::new(ErrorCode::GarbledOutput, "Ghidra was unpacked, but its launcher wasn't where we expected."))
}

/// One pinned archive: download beside the store (hashed as it streams; a mismatch deletes it),
/// unpack into a temporary folder and only then swap it in as `<store>/<folder>`.
fn fetch(pin: &crate::Pin, store: &Path, folder: &str, what: &str, cancel: Option<Cancel>, progress: &dyn Fn(&str, u64, Option<u64>)) -> Result<(), ToolError> {
    std::fs::create_dir_all(store).map_err(|e| ToolError::io(store.display(), e))?;
    let archive = store.join(format!(".{folder}.archive"));
    crate::install::download_verified(pin.url, &archive, pin.sha256, 1 << 30, cancel.as_ref(), &|got, total| progress(what, got, total))?;
    progress(&format!("Setting up {what}"), 0, None);
    let data = std::fs::read(&archive).map_err(|e| ToolError::io(archive.display(), e));
    let _ = std::fs::remove_file(&archive);
    let data = data?;
    let incoming = store.join(format!(".{folder}.incoming"));
    let _ = std::fs::remove_dir_all(&incoming);
    let dest = store.join(folder);
    let swapped = crate::install::unpack(&data, pin.format, &incoming, pin.strip_prefix, cancel.as_ref(), &|_| {}).and_then(|_| swap_in(&incoming, &dest));
    if swapped.is_err() {
        let _ = std::fs::remove_dir_all(&incoming);
    }
    swapped
}

/// Replace only the entries an unpacked release brings; a kept install beside them (an older
/// Ghidra, still usable under the legacy policy) stays untouched.
fn swap_in(incoming: &Path, dest: &Path) -> Result<(), ToolError> {
    std::fs::create_dir_all(dest).map_err(|e| ToolError::io(dest.display(), e))?;
    for from in std::fs::read_dir(incoming).map_err(|e| ToolError::io(incoming.display(), e))?.flatten().map(|e| e.path()).collect::<Vec<_>>() {
        let to = dest.join(from.file_name().unwrap_or_default());
        let _ = std::fs::remove_dir_all(&to);
        let _ = std::fs::remove_file(&to);
        std::fs::rename(&from, &to).map_err(|e| ToolError::io(to.display(), e))?;
    }
    let _ = std::fs::remove_dir_all(incoming);
    Ok(())
}

/// Ghidra's supported way to choose its Java: `JAVA_HOME_OVERRIDE` in support/launch.properties.
fn point_at_java(props: &Path, java: &Path) -> Result<(), ToolError> {
    // Forward slashes: Ghidra reads this file without escapes, and Java takes them on Windows too.
    let line = format!("JAVA_HOME_OVERRIDE={}", java.display().to_string().replace('\\', "/"));
    let old = std::fs::read_to_string(props).unwrap_or_default();
    let mut lines: Vec<String> = old.lines().filter(|l| !l.trim_start().starts_with("JAVA_HOME_OVERRIDE")).map(str::to_string).collect();
    lines.push(line);
    std::fs::write(props, lines.join("\n") + "\n").map_err(|e| ToolError::io(props.display(), e))
}

/// The Java Ghidra will run on, found the way its launcher looks: a working `JAVA_HOME_OVERRIDE`
/// in its `support/launch.properties` first, then the `JAVA_HOME` it's started with, then a `java`
/// on PATH — each judged against the floor the launcher's own version needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhidraJava {
    /// `JAVA_HOME_OVERRIDE` in Ghidra's launch.properties (a guided install writes it).
    Override(PathBuf),
    /// The `JAVA_HOME` Ghidra is started with: the one set for it (`configured`), or the one this
    /// program was started with.
    Home { dir: PathBuf, configured: bool },
    /// A `java` on PATH.
    OnPath(PathBuf),
    /// The folder set as Ghidra's Java has no `bin/java`.
    NotJava(PathBuf),
    /// A Java whose version is known and below what this Ghidra needs: Ghidra is stopped before
    /// it starts, with the versions spelled out.
    TooOld { dir: PathBuf, found: u32, needed: u32 },
    /// A Java with a `bin/java` whose version can't be read: Ghidra gets to judge it.
    Unknown(PathBuf),
    /// No Java at all: Ghidra looks on its own paths (next to a Homebrew install, say).
    Missing,
}

/// Which Java the Ghidra at `launcher` will run on, and whether it is new enough: the floor comes
/// from the launcher's own version (25 for this release, 21 for a kept legacy install).
/// `configured` is the Java set for it (`Settings::ghidra_java`), `inherited` the `JAVA_HOME`
/// this program has.
pub fn java_in_use(launcher: &Path, configured: Option<&Path>, inherited: Option<&Path>) -> GhidraJava {
    let floor = java_floor(version_hint_of(launcher).as_ref());
    // Ghidra's launcher follows a link to itself (one on PATH) and reads the properties beside
    // the real one.
    let launcher = std::fs::canonicalize(launcher).unwrap_or_else(|_| launcher.to_path_buf());
    let props = launcher.parent().map(|d| d.join("launch.properties"));
    let over = props.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|text| java_override(&text, &|k| std::env::var(k).ok()));
    if let Some(dir) = over.filter(|d| has_java(d)) {
        return judged_java(GhidraJava::Override(dir), floor);
    }
    match configured {
        Some(dir) if has_java(dir) => return judged_java(GhidraJava::Home { dir: dir.to_path_buf(), configured: true }, floor),
        Some(dir) => return GhidraJava::NotJava(dir.to_path_buf()),
        None => {}
    }
    if let Some(dir) = inherited.filter(|d| has_java(d)) {
        return judged_java(GhidraJava::Home { dir: dir.to_path_buf(), configured: false }, floor);
    }
    match crate::find_on_path(&[if cfg!(windows) { "java.exe" } else { "java" }]) {
        Some(p) => judged_java(GhidraJava::OnPath(p), floor),
        None => GhidraJava::Missing,
    }
}

/// A Java in one of the `has a bin/java` branches, judged against the floor it must meet: a
/// known version at or above the floor keeps its branch, a known lower one becomes `TooOld`,
/// an unreadable one `Unknown`.
fn judged_java(java: GhidraJava, floor: u32) -> GhidraJava {
    let home = match &java {
        GhidraJava::Override(d) | GhidraJava::Home { dir: d, .. } => Some(d.clone()),
        // `java` on PATH is usually a link (`/usr/bin/java` → the JDK's `bin/java`), so follow
        // it; the folder its link target sits in is the home whose `release` file has the version.
        GhidraJava::OnPath(p) => {
            let real = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
            real.parent().and_then(Path::parent).map(Path::to_path_buf)
        }
        _ => None,
    };
    let Some(home) = home else { return java };
    match java::verdict(&home, floor) {
        JavaVerdict::Ok(_) => java,
        JavaVerdict::TooOld(found) => GhidraJava::TooOld { dir: home, found, needed: floor },
        JavaVerdict::Unknown => GhidraJava::Unknown(home),
    }
}

/// Stops before Ghidra starts when the Java explicitly chosen for it is known to be too old or
/// has no `bin/java`, or the chosen Ghidra is no longer supported: failing here, with the
/// versions spelled out, beats failing hours into an analysis. "Explicitly chosen" is a working
/// `JAVA_HOME_OVERRIDE` beside the launcher (what a guided install writes) or the Java set in
/// Settings — not a `JAVA_HOME` this program inherited nor a `java` on PATH, because a wrapper
/// used as the launcher may pick its own and must get the last word. A Java whose version can't
/// be read is likewise left for Ghidra. Every way Ghidra is started goes through
/// [`super::analyse`], so UI, CLI, MCP and the build agent all get the same stop.
pub fn check_java(cfg: &Config) -> Result<(), GhidraError> {
    let below = is_below_current(version_hint_of(&cfg.analyze_headless).as_ref());
    if below && LEGACY_POLICY == LegacyPolicy::Refuse {
        return Err(GhidraError::Preflight(format!(
            "Ghidra {VERSION} is the supported version, and the one chosen ({}) isn't supported any more. Install Ghidra {VERSION} from Settings → Engines.",
            cfg.analyze_headless.display()
        )));
    }
    let Some(home) = explicit_java(&cfg.analyze_headless, cfg.java_home.as_deref()) else {
        // Nothing was chosen for it: a wrapper beside a Ghidra folder may pick its own Java, so
        // what it runs on is left for Ghidra (and the wrapper) to judge.
        return Ok(());
    };
    if !has_java(&home) {
        return Err(GhidraError::Preflight(format!(
            "The folder set as Ghidra's Java ({}) has no bin/java, so Ghidra can't start. Choose a JDK folder in Settings → Engines, or clear the setting to use the one the guided install brings.",
            home.display()
        )));
    }
    let floor = java_floor(version_hint_of(&cfg.analyze_headless).as_ref());
    match java::verdict(&home, floor) {
        JavaVerdict::Ok(_) => Ok(()),
        JavaVerdict::TooOld(found) => Err(GhidraError::Preflight(format!(
            "Ghidra needs Java {floor}, but the Java it would run on is Java {found} ({}). Choose a newer Java in Settings → Engines, or clear the setting to use the one the guided install brings.",
            home.display()
        ))),
        // Can't tell: don't guess, let Ghidra start and say what it finds.
        JavaVerdict::Unknown => Ok(()),
    }
}

/// The Java explicitly chosen for Ghidra: a working `JAVA_HOME_OVERRIDE` beside its launcher
/// (read through a link the way the launcher is), else the one set in Settings. `None` when
/// nothing was chosen and the wrapper (or Ghidra) picks for itself.
fn explicit_java(launcher: &Path, configured: Option<&Path>) -> Option<PathBuf> {
    let real = std::fs::canonicalize(launcher).unwrap_or_else(|_| launcher.to_path_buf());
    let props = real.parent().map(|d| d.join("launch.properties"));
    let over = props.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|text| java_override(&text, &|k| std::env::var(k).ok()));
    over.filter(|d| has_java(d)).or_else(|| configured.map(Path::to_path_buf))
}

/// The `JAVA_HOME_OVERRIDE` in Ghidra's launch.properties text, read the way Ghidra does: `KEY=value`
/// with or without spaces around the `=`, the first one with a value, and `${NAME}` replaced by
/// that environment variable (`env`; an unset one becomes nothing).
fn java_override(text: &str, env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let value =
        text.lines().filter_map(|l| l.split_once('=')).filter(|(k, _)| k.trim() == "JAVA_HOME_OVERRIDE").map(|(_, v)| v.trim()).find(|v| !v.is_empty())?;
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        out.push_str(rest.get(..start).unwrap_or_default());
        let after = rest.get(start + 2..).unwrap_or_default();
        let Some(end) = after.find('}') else {
            // No closing brace: the rest is taken as it is.
            out.push_str(rest.get(start..).unwrap_or_default());
            rest = "";
            break;
        };
        out.push_str(&env(after.get(..end).unwrap_or_default()).unwrap_or_default());
        rest = after.get(end + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    Some(PathBuf::from(out)).filter(|p| !p.as_os_str().is_empty())
}

fn launcher(root: &Path) -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["analyzeHeadless.bat"] } else { &["analyzeHeadless"] };
    names.iter().map(|n| root.join("support").join(n)).find(|p| p.is_file())
}

fn has_java(dir: &Path) -> bool {
    dir.join("bin").join(if cfg!(windows) { "java.exe" } else { "java" }).is_file()
}

/// `find` on `dir` itself, else on its sub-folders (a zip unpacks into one top folder).
fn in_root_or_child(dir: &Path, find: impl Fn(&Path) -> Option<PathBuf>) -> Option<PathBuf> {
    find(dir).or_else(|| in_child(dir, find))
}

fn in_child(dir: &Path, find: impl Fn(&Path) -> Option<PathBuf>) -> Option<PathBuf> {
    let mut subs: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    subs.sort_by(|a, b| version_order(&a.file_name().unwrap_or_default().to_string_lossy(), &b.file_name().unwrap_or_default().to_string_lossy()));
    subs.iter().rev().find_map(|d| find(d))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::JDK_PINNED;

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }

    /// A JDK folder a test can point at: `bin/java` and the `release` file a version is read from.
    fn jdk(at: &Path, version: &str) -> PathBuf {
        let home = at.to_path_buf();
        touch(&home.join("bin").join(if cfg!(windows) { "java.exe" } else { "java" }));
        std::fs::write(home.join("release"), format!("JAVA_VERSION=\"{version}\"\n")).unwrap();
        home
    }

    #[test]
    fn a_macos_jdk_unpacks_to_a_java_home_we_find() {
        use craft_anything_testkit::firmware as fx;
        let pin = crate::engine("java").and_then(|e| e.pin_for("macos", "aarch64")).unwrap();
        let java = if cfg!(windows) { "java.exe" } else { "java" };
        let top = format!("jdk-{JDK_PINNED}/");
        let tar = fx::tar_entries(&[
            (&format!("{top}Contents/Info.plist"), b'0', b"<plist/>", ""),
            (&format!("{top}Contents/Home/bin/{java}"), b'0', b"java", ""),
            (&format!("{top}Contents/Home/lib/modules"), b'0', b"m", ""),
            (&format!("{top}Contents/Home/release"), b'0', b"JAVA_VERSION=\"25.0.1\"\n", ""),
        ]);
        let store = tempfile::tempdir().unwrap();
        let jdk_home = store.path().join("jdk");
        crate::install::unpack(&fx::gzip(&tar, None), pin.format, &jdk_home, pin.strip_prefix, None, &|_| {}).unwrap();
        // The JDK keeps its own version folder (so a JDK the person already had beside it stays),
        // and is found through the JDK's `Contents/Home`.
        let installed = jdk_home.join(format!("jdk-{JDK_PINNED}/Contents/Home"));
        assert_eq!(java_home(None, None, store.path()), Some(installed.clone()));
        // A managed Java from an earlier setup (`jdk/Home`, as 12.1.4's pin unpacked) is left on
        // disk and not picked for 12.2, which needs Java 25.
        let old = jdk_home.join("Home");
        touch(&old.join("bin").join(java));
        std::fs::write(old.join("release"), "JAVA_VERSION=\"21.0.5\"\n").unwrap();
        assert_eq!(java_home(None, None, store.path()), Some(installed), "the old Java stays, 12.2 uses Java 25");
        assert!(old.join("bin").join(java).is_file(), "the old Java is untouched");
        let linux = crate::engine("java").and_then(|e| e.pin_for("linux", "x86_64")).unwrap();
        assert_eq!((linux.format, linux.strip_prefix), (crate::ArchiveFormat::TarGz, None), "Linux keeps jdk-…/bin/java, one folder down");
    }

    /// Installing Java 25 over a store that already holds a managed `jdk/Home` (what the 12.1.4
    /// setup unpacked) leaves that runtime in place: a kept install keeps the Java it runs on.
    #[test]
    fn a_newer_java_lands_beside_the_runtime_it_does_not_replace() {
        let store = tempfile::tempdir().unwrap();
        let dest = store.path().join(JAVA_FOLDER);
        let old = jdk(&dest.join("Home"), "21.0.5");
        let incoming = store.path().join(".jdk.incoming");
        jdk(&incoming.join(format!("jdk-{JDK_PINNED}/Contents/Home")), "25.0.1");
        swap_in(&incoming, &dest).unwrap();
        let java = if cfg!(windows) { "java.exe" } else { "java" };
        let new = dest.join(format!("jdk-{JDK_PINNED}/Contents/Home"));
        assert!(old.join("bin").join(java).is_file(), "the Java a kept 12.1.4 runs on is untouched");
        assert_eq!(java_home(None, None, store.path()), Some(new), "12.2 gets the Java 25 that came with it");
        assert!(!incoming.exists());
    }

    /// Ghidra's folder holds both scripts; the `.bat` can't start off Windows (#1084).
    #[test]
    fn the_launcher_looked_for_on_path_is_this_systems_own() {
        let exes = crate::engine(ID).unwrap().host_exes();
        assert_eq!(exes, [if cfg!(windows) { "analyzeHeadless.bat" } else { "analyzeHeadless" }]);
    }

    #[test]
    fn finds_ghidra_and_java_where_they_really_live() {
        let d = tempfile::tempdir().unwrap();
        let store = d.path().join("store");
        let exe = if cfg!(windows) { "analyzeHeadless.bat" } else { "analyzeHeadless" };
        // None, unless this computer has Ghidra on PATH, from a system package, from Homebrew, or
        // in a usual unpacked folder.
        let e = crate::engine(ID).unwrap();
        let installed = crate::find_on_path(&[exe]).is_some()
            || crate::well_known(ID).iter().any(|p| p.is_file())
            || crate::find_in_package_paths(e).is_some()
            || crate::find_in_known_roots(ID, e.host_exes(), &e.known_root_folders()).is_some();
        assert!(headless(None, &store).is_none() || installed);
        // The managed install: the zip's own top folder under <store>/ghidra.
        let managed = store.join("ghidra/ghidra_12.2_PUBLIC/support").join(exe);
        touch(&managed);
        assert_eq!(headless(None, &store), Some(managed.clone()));
        // A folder the person picked wins, whether it's Ghidra's folder or the one holding it.
        let own = d.path().join("tools/ghidra_11.4/support").join(exe);
        touch(&own);
        assert_eq!(headless(Some(&d.path().join("tools/ghidra_11.4")), &store), Some(own.clone()));
        assert_eq!(headless(Some(&d.path().join("tools")), &store), Some(own));

        assert!(java_home(Some(&managed), None, &store).is_none());
        let jdk_dir = jdk(&store.join("jdk").join("jdk-25.0.5.1+2"), "25.0.1");
        assert_eq!(java_home(Some(&managed), None, &store), Some(jdk_dir));
        assert_eq!(java_home(Some(&managed), Some(d.path()), &store).as_deref(), Some(d.path()), "a Java the person set wins");
    }

    const EXE: &str = if cfg!(windows) { "analyzeHeadless.bat" } else { "analyzeHeadless" };

    /// A Ghidra folder as its zip unpacks: `ghidraRun` at the top, the headless launcher in `support/`.
    fn ghidra_folder(at: &Path) -> PathBuf {
        touch(&at.join("ghidraRun"));
        let exe = at.join("support").join(EXE);
        touch(&exe);
        exe
    }

    fn same(a: Option<PathBuf>, b: &Path) -> bool {
        a.and_then(|a| std::fs::canonicalize(a).ok()) == std::fs::canonicalize(b).ok()
    }

    /// A Ghidra setting that names a file is run as Ghidra's headless launcher, so it has to be one.
    #[test]
    fn a_setting_naming_the_launcher_itself_is_that_launcher() {
        let d = tempfile::tempdir().unwrap();
        let exe = ghidra_folder(&d.path().join("ghidra_12.2_PUBLIC"));
        assert_eq!(headless_configured(Some(&exe), &d.path().join("store")), Some(exe));
    }

    /// Arch's package puts a start script on PATH that runs `/opt/ghidra/ghidraRun`, Ghidra's window,
    /// not its headless launcher: picked as the setting, it was listed as found and Deep mode
    /// quietly used the built-in decompiler.
    #[test]
    fn a_start_script_is_followed_to_the_ghidra_folder_it_names() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("opt").join("My Ghidra");
        let exe = ghidra_folder(&root);
        let script = d.path().join("bin").join("ghidra");
        let store = d.path().join("store");
        let mut bodies = vec![
            format!("#!/bin/sh\nexec \"{}\" \"$@\"\n", root.join("ghidraRun").display()),
            format!("#!/bin/sh\nGHIDRA_HOME='{}'\nexec \"$GHIDRA_HOME/ghidraRun\" \"$@\"\n", root.display()),
        ];
        let plain = d.path().join("opt").join("ghidra");
        if !plain.display().to_string().contains(' ') {
            ghidra_folder(&plain);
            bodies.push(format!("#!/bin/sh\nexec {}/ghidraRun \"$@\"\n", plain.display()));
        }
        for body in bodies {
            touch(&script);
            std::fs::write(&script, &body).unwrap();
            let found = headless_configured(Some(&script), &store);
            assert!(same(found.clone(), &exe) || same(found, &plain.join("support").join(EXE)), "{body}");
            assert!(!headless_configured(Some(&script), &store).unwrap().display().to_string().starts_with(r"\\?\"));
        }
        // Too long to be a start script: not read.
        std::fs::write(&script, format!("{}\nexec \"{}\"\n", "#".repeat(SCRIPT_CAP as usize), root.join("ghidraRun").display())).unwrap();
        assert_eq!(headless_configured(Some(&script), &store), None);
    }

    /// A script that runs the headless launcher itself (setting up Java first, say) is run as it is.
    #[test]
    fn a_wrapper_around_the_launcher_is_kept() {
        let d = tempfile::tempdir().unwrap();
        let exe = ghidra_folder(&d.path().join("ghidra"));
        let wrapper = d.path().join("bin").join("ghidra-headless");
        touch(&wrapper);
        std::fs::write(&wrapper, format!("#!/bin/sh\nJAVA_HOME=/jdk exec \"{}\" \"$@\"\n", exe.display())).unwrap();
        assert_eq!(headless_configured(Some(&wrapper), &d.path().join("store")), Some(wrapper));
        // Also when it sits in Ghidra's own folder, beside `ghidraRun`.
        let beside = d.path().join("ghidra").join("headless-with-java");
        std::fs::write(&beside, format!("#!/bin/sh\nJAVA_HOME=/jdk exec \"{}\" \"$@\"\n", exe.display())).unwrap();
        assert_eq!(headless_configured(Some(&beside), &d.path().join("store")), Some(beside));
    }

    /// A start script that runs a link onto PATH (`exec /usr/bin/ghidraRun`) leads to where it points.
    #[cfg(unix)]
    #[test]
    fn a_start_script_naming_a_link_is_followed_through_it() {
        let d = tempfile::tempdir().unwrap();
        let exe = ghidra_folder(&d.path().join("opt").join("ghidra"));
        let bin = d.path().join("usr").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(d.path().join("opt").join("ghidra").join("ghidraRun"), bin.join("ghidraRun")).unwrap();
        let script = d.path().join("ghidra");
        std::fs::write(&script, format!("#!/bin/sh\nexec \"{}\" \"$@\"\n", bin.join("ghidraRun").display())).unwrap();
        assert!(same(headless_configured(Some(&script), &d.path().join("store")), &exe));
    }

    #[test]
    fn a_file_that_leads_to_no_ghidra_is_not_ghidra() {
        let d = tempfile::tempdir().unwrap();
        let store = d.path().join("store");
        let notes = d.path().join("notes.txt");
        std::fs::write(&notes, "nothing to see").unwrap();
        assert_eq!(headless_configured(Some(&notes), &store), None);
        let script = d.path().join("ghidra");
        std::fs::write(&script, format!("#!/bin/sh\nexec {}/ghidraRun \"$@\"\n", d.path().join("gone").display())).unwrap();
        assert_eq!(headless_configured(Some(&script), &store), None);
        std::fs::write(&script, [0xff, 0xfe, 0x00, 0x2f]).unwrap();
        assert_eq!(headless_configured(Some(&script), &store), None, "not text");
        // With nothing usable set, the managed install is still found.
        let managed = ghidra_folder(&store.join(ID).join("ghidra_12.2_PUBLIC"));
        assert_eq!(headless_configured(Some(&notes), &store), Some(managed));
    }

    /// Homebrew and some packages link `ghidraRun` onto PATH instead.
    #[cfg(unix)]
    #[test]
    fn a_link_to_ghidras_start_script_is_followed_to_its_folder() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("Caskroom").join("ghidra_12.2_PUBLIC");
        let exe = ghidra_folder(&root);
        let bin = d.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(root.join("ghidraRun"), bin.join("ghidraRun")).unwrap();
        assert!(same(headless_configured(Some(&bin.join("ghidraRun")), &d.path().join("store")), &exe));
        std::os::unix::fs::symlink(&exe, bin.join(EXE)).unwrap();
        assert_eq!(headless_configured(Some(&bin.join(EXE)), &d.path().join("store")), Some(bin.join(EXE)), "a linked launcher is kept as given");
    }

    #[test]
    fn the_java_override_is_read_as_ghidra_reads_it() {
        let env = |k: &str| (k == "JDK_HOME").then(|| "/opt/jdk-25".to_string());
        let read = |t: &str| java_override(t, &env);
        assert_eq!(read("JAVA_HOME_OVERRIDE=/a\nJAVA_HOME_OVERRIDE=/b\n"), Some(PathBuf::from("/a")), "the first one");
        assert_eq!(read("JAVA_HOME_OVERRIDE=\nJAVA_HOME_OVERRIDE = /b\n"), Some(PathBuf::from("/b")), "the first with a value");
        assert_eq!(read("JAVA_HOME_OVERRIDE=${JDK_HOME}\n"), Some(PathBuf::from("/opt/jdk-25")));
        assert_eq!(read("JAVA_HOME_OVERRIDE=${JDK_HOME}/x/${NOPE}y\n"), Some(PathBuf::from("/opt/jdk-25/x/y")));
        assert_eq!(read("JAVA_HOME_OVERRIDE=${NOPE}\n"), None, "an unset variable leaves no path");
        assert_eq!(read("JAVA_HOME_OVERRIDE=/a${JDK\n"), Some(PathBuf::from("/a${JDK")));
        assert_eq!(read("VMARGS=-Xshare:off\n"), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_launcher_reached_through_a_link_reads_the_real_installs_properties() {
        let d = tempfile::tempdir().unwrap();
        let support = d.path().join("ghidra").join("support");
        let jdk_home = jdk(&d.path().join("jdk"), "25.0.1");
        std::fs::create_dir_all(&support).unwrap();
        std::fs::write(support.join("analyzeHeadless"), "").unwrap();
        std::fs::write(support.join("launch.properties"), format!("JAVA_HOME_OVERRIDE={}\n", jdk_home.display())).unwrap();
        let bin = d.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(support.join("analyzeHeadless"), bin.join("analyzeHeadless")).unwrap();
        let canon = std::fs::canonicalize(&jdk_home).unwrap();
        assert!(matches!(java_in_use(&bin.join("analyzeHeadless"), None, None), GhidraJava::Override(p) if p == jdk_home || p == canon));
    }

    /// A `java` on PATH is normally a link (`/usr/bin/java` → the JDK's `bin/java`): judging it
    /// must read the real JDK's `release`, or a known-too-old Java passes as unknown and the early
    /// stop never fires.
    #[cfg(unix)]
    #[test]
    fn a_java_on_path_is_judged_through_the_link_to_its_jdk() {
        let d = tempfile::tempdir().unwrap();
        let home = jdk(&d.path().join("jdk-21"), "21.0.5");
        let bin = d.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(home.join("bin").join("java"), bin.join("java")).unwrap();
        let judged = judged_java(GhidraJava::OnPath(bin.join("java")), JAVA_MAJOR);
        assert!(matches!(judged, GhidraJava::TooOld { found: 21, needed: 25, .. }), "{judged:?}");
    }

    #[test]
    fn ghidra_is_pointed_at_our_java() {
        let d = tempfile::tempdir().unwrap();
        let props = d.path().join("launch.properties");
        std::fs::write(&props, "VMARGS=-Xshare:off\nJAVA_HOME_OVERRIDE=\n").unwrap();
        point_at_java(&props, Path::new(r"C:\store\jdk\jdk-25")).unwrap();
        let text = std::fs::read_to_string(&props).unwrap();
        assert_eq!(text, "VMARGS=-Xshare:off\nJAVA_HOME_OVERRIDE=C:/store/jdk/jdk-25\n");
    }

    #[test]
    fn pins_come_from_the_engine_table() {
        // Checksum handling is tested once, in `install` (download_verified).
        let ghidra = crate::engine("ghidra").unwrap().pins();
        assert!(ghidra.iter().all(|p| p.url.contains(VERSION)));
        assert!(version_parse(VERSION).is_some(), "VERSION names a version: min_current depends on it");
        let java_pinned = cfg!(any(
            all(windows, target_arch = "x86_64"),
            all(any(target_os = "linux", target_os = "macos"), any(target_arch = "x86_64", target_arch = "aarch64"))
        ));
        assert_eq!(can_install(), java_pinned);
    }

    /// Versions are read out of folder names, a few levels up from the launcher: a release's own
    /// top folder, Homebrew's `Cellar` version folder; a folder without a version names none.
    #[test]
    fn the_version_hint_comes_from_the_folder_name() {
        let exe = if cfg!(windows) { "analyzeHeadless.bat" } else { "analyzeHeadless" };
        let store = tempfile::tempdir().unwrap();
        let launcher = store.path().join("ghidra/ghidra_12.2_PUBLIC/support").join(exe);
        touch(&launcher);
        let install = locate_install(None, store.path()).unwrap();
        assert_eq!(install.version_hint, Some(Version(vec![12, 2])));
        assert_eq!(install.source, GhidraSource::EngineStore);
        // Homebrew's layout carries the version two folders higher up.
        let d = tempfile::tempdir().unwrap();
        let brew = d.path().join("homebrew");
        let rel = "libexec/support/analyzeHeadless";
        touch(&brew.join("Cellar/ghidra/12.2").join(rel));
        let hint = version_hint_of(&brew.join("Cellar/ghidra/12.2").join(rel));
        assert_eq!(hint, Some(Version(vec![12, 2])));
        // A name without a version stays without one.
        let plain = d.path().join("opt/ghidra/support").join(exe);
        touch(&plain);
        assert_eq!(version_hint_of(&plain), None);
        // A number in an unrelated parent's name is not a version (`/home/user1/ghidra/…`).
        let user = d.path().join("home/user1/ghidra/support").join(exe);
        touch(&user);
        assert_eq!(version_hint_of(&user), None);
    }

    /// Homebrew links `opt/ghidra` to `Cellar/ghidra/<version>`: the version is in the real path,
    /// so the linked path gets it too.
    #[cfg(unix)]
    #[test]
    fn a_linked_launcher_takes_its_version_from_where_it_leads() {
        let d = tempfile::tempdir().unwrap();
        let rel = "libexec/support/analyzeHeadless";
        let cellar = d.path().join("Cellar/ghidra/12.2").join(rel);
        touch(&cellar);
        std::fs::create_dir_all(d.path().join("opt")).unwrap();
        // `opt/ghidra` stands in for the linked package folder; the launcher is reached through it.
        std::os::unix::fs::symlink(d.path().join("Cellar/ghidra/12.2"), d.path().join("opt/ghidra")).unwrap();
        let linked = d.path().join("opt/ghidra").join(rel);
        assert_eq!(version_hint_of(&linked), Some(Version(vec![12, 2])));
        // The launch uses the path as given; only the version is read through the link.
        let install = install_of(linked.clone(), GhidraSource::PackageManager("Homebrew"));
        assert_eq!(install.launcher, linked);
        assert_eq!(install.version_hint, Some(Version(vec![12, 2])));
    }

    /// Two managed installs side by side: the newest wins, by the numbers. `12.10.0` beats
    /// `12.2`, which plain text order would get backwards, and a name without a version never
    /// stops the search.
    #[test]
    fn when_versions_coexist_the_newest_is_chosen() {
        let exe = if cfg!(windows) { "analyzeHeadless.bat" } else { "analyzeHeadless" };
        let store = tempfile::tempdir().unwrap();
        let old = store.path().join("ghidra/ghidra_12.1.4_PUBLIC/support").join(exe);
        let new = store.path().join("ghidra/ghidra_12.2_PUBLIC/support").join(exe);
        touch(&old);
        touch(&new);
        assert_eq!(headless(None, store.path()), Some(new.clone()));
        // A Java 25 home set for Ghidra, so the verdict doesn't depend on the host's own Java.
        let java = jdk(&store.path().join("jdk-25"), "25.0");
        let status = status(None, Some(&java), store.path());
        assert_eq!(status.state, GhidraState::Ready, "{:?}", status.java);
        assert_eq!(status.install.as_ref().unwrap().version_hint, Some(Version(vec![12, 2])));
        // A patch release outranks the release it patches: `12.2.1` over `12.2`, though `_PUBLIC`
        // sorts after `.` as text.
        let patched = store.path().join("ghidra/ghidra_12.2.1_PUBLIC/support").join(exe);
        touch(&patched);
        let found = locate_install(None, store.path()).unwrap();
        assert_eq!(found.launcher, patched, "the patch release is the one selected");
        assert_eq!(found.version_hint, Some(Version(vec![12, 2, 1])));
        // `12.10.0` is newer than `12.2`, not the other way round.
        let newest = store.path().join("ghidra/ghidra_12.10.0_PUBLIC/support").join(exe);
        touch(&newest);
        assert_eq!(headless(None, store.path()), Some(newest.clone()));
        // A folder whose name carries no version beside them: it can't be told from an older
        // release, so it never wins over one that says its version, whatever it sorts as.
        for name in ["ghidra-custom", "ghidra_custom"] {
            touch(&store.path().join("ghidra").join(name).join("support").join(exe));
        }
        assert_eq!(headless(None, store.path()), Some(newest));
    }

    /// A folder the person picked that holds two releases: the newest wins by the numbers there
    /// too (`12.10.0` beats `12.2`, which text order would get backwards).
    #[test]
    fn a_picked_folder_with_two_releases_picks_the_newest_by_the_numbers() {
        let d = tempfile::tempdir().unwrap();
        let newest = ghidra_folder(&d.path().join("ghidra_12.10.0_PUBLIC"));
        let _ = ghidra_folder(&d.path().join("ghidra_12.2_PUBLIC"));
        assert_eq!(headless(Some(d.path()), &d.path().join("store")), Some(newest));
    }

    /// Swapping a release in replaces only what it brings: a kept legacy install beside it stays.
    #[test]
    fn swapping_a_release_in_keeps_a_legacy_install_beside_it() {
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("ghidra");
        let legacy = dest.join("ghidra_12.1.4_PUBLIC/support");
        touch(&legacy.join(EXE));
        touch(&dest.join("ghidra_12.2_PUBLIC/support").join(EXE));
        let incoming = d.path().join("incoming");
        touch(&incoming.join("ghidra_12.2_PUBLIC/support").join(EXE));
        swap_in(&incoming, &dest).unwrap();
        assert!(legacy.join(EXE).is_file(), "the kept legacy install is untouched");
        assert!(dest.join("ghidra_12.2_PUBLIC/support").join(EXE).is_file());
        assert!(!incoming.exists());
    }

    /// The Java floor follows the Ghidra version: this release needs 25, a kept legacy install
    /// may run on 21, and an unknown version gets the current floor rather than a guess.
    #[test]
    fn the_java_floor_follows_the_ghidra_version() {
        assert_eq!(java_floor(Some(&Version(vec![12, 2]))), JAVA_MAJOR);
        assert_eq!(java_floor(Some(&Version(vec![12, 10, 0]))), JAVA_MAJOR);
        assert_eq!(java_floor(None), JAVA_MAJOR);
        assert_eq!(java_floor(Some(&Version(vec![12, 1, 4]))), LEGACY_JAVA_MAJOR);
    }

    /// A managed Java below the floor is not picked, whatever its name sorts as; the newest
    /// major version that is new enough wins.
    #[test]
    fn a_managed_java_below_the_floor_is_not_picked() {
        let store = tempfile::tempdir().unwrap();
        jdk(&store.path().join("jdk").join("jdk-21.0.12.1+1"), "21.0.5");
        assert_eq!(managed_java(store.path(), JAVA_MAJOR), None, "the old managed Java is refused");
        let twenty_five = jdk(&store.path().join("jdk").join("jdk-25.0.5.1+2"), "25.0.1");
        assert_eq!(managed_java(store.path(), JAVA_MAJOR), Some(twenty_five.clone()));
        // A newer major version beside it wins over the pinned one.
        let twenty_six = jdk(&store.path().join("jdk").join("jdk-26.0.1"), "26.0.1");
        assert_eq!(managed_java(store.path(), JAVA_MAJOR), Some(twenty_six));
        // The old Java is still there, untouched (other Java tools keep using it).
        let exe = if cfg!(windows) { "java.exe" } else { "java" };
        assert!(store.path().join("jdk").join("jdk-21.0.12.1+1").join("bin").join(exe).is_file());
        let _ = twenty_five;
    }

    /// A kept 12.1.4 falls back to the managed Java it needs, not the one 12.2 wants: with only a
    /// managed Java 21 in the store, that is the Java handed to it, and the status says so.
    #[test]
    fn a_kept_legacy_install_gets_the_managed_java_it_needs() {
        let store = tempfile::tempdir().unwrap();
        let legacy = store.path().join("ghidra/ghidra_12.1.4_PUBLIC/support").join(EXE);
        touch(&legacy);
        let twenty_one = jdk(&store.path().join(JAVA_FOLDER).join("jdk-21.0.12.1+1"), "21.0.5");
        assert_eq!(java_home(Some(&legacy), None, store.path()), Some(twenty_one.clone()));
        let found = status_with(None, None, None, store.path(), false);
        assert_eq!(found.state, GhidraState::Legacy, "{found:?}");
        assert!(matches!(&found.java, GhidraJava::Home { dir, .. } if same(Some(dir.clone()), &twenty_one)), "{:?}", found.java);
        // The 12.2 install beside it still asks for Java 25, which isn't there.
        let current = ghidra_folder(&store.path().join("ghidra/ghidra_12.2_PUBLIC"));
        assert_eq!(java_home(Some(&current), None, store.path()), None, "Java 21 doesn't run 12.2");
    }

    /// The status tells the states apart, in a limited (test-session) lookup so nothing installed
    /// outside the store can change the verdict: nothing anywhere is `NotFound`; a folder with no
    /// launcher is `Incomplete`; a legacy install is kept (`Legacy`) and reports its real version;
    /// a Java too old that was chosen for Ghidra stops it before it starts (`JavaTooOld`).
    #[test]
    fn the_status_tells_the_states_apart() {
        let d = tempfile::tempdir().unwrap();
        let store = d.path().join("store");
        let exe = if cfg!(windows) { "analyzeHeadless.bat" } else { "analyzeHeadless" };
        let limited = |store: &Path| status_limited(None, None, store, false);
        // Nothing set, nothing found: a limited status never looks around the computer.
        assert_eq!(limited(&store).state, GhidraState::NotFound);
        // A folder that is there but has no launcher is incomplete, not missing.
        std::fs::create_dir_all(store.join("ghidra/ghidra_12.2_PUBLIC")).unwrap();
        assert_eq!(limited(&store).state, GhidraState::Incomplete);
        // Ready: a launcher and a Java good enough for it.
        ghidra_folder(&store.join("ghidra/ghidra_12.2_PUBLIC"));
        let good = jdk(&d.path().join("jdk-25"), "25.0.1");
        assert_eq!(status_limited(None, Some(&good), &store, false).state, GhidraState::Ready);
        // A Java too old for this release, chosen for it, stops it before it starts.
        let old = jdk(&d.path().join("jdk-21"), "21.0.5");
        let too_old = status_limited(None, Some(&old), &store, false);
        assert_eq!(too_old.state, GhidraState::JavaTooOld, "{too_old:?}");
        assert!(matches!(too_old.java, GhidraJava::TooOld { found: 21, needed: 25, .. }), "{:?}", too_old.java);
        // A kept legacy install stays usable and says its real version.
        let legacy_store = d.path().join("legacy-store");
        touch(&legacy_store.join("ghidra/ghidra_12.1.4_PUBLIC/support").join(exe));
        let legacy = status_limited(None, Some(&old), &legacy_store, false);
        assert_eq!(legacy.state, GhidraState::Legacy, "{legacy:?}");
        assert_eq!(legacy.install.as_ref().unwrap().version_hint, Some(Version(vec![12, 1, 4])));
    }

    /// A Java this program merely inherits never refuses Ghidra — a wrapper launcher may set its
    /// own up, so Ghidra judges it — while one chosen for Ghidra does, as `check_java` refuses it:
    /// the verdict and the launch check draw the same line.
    #[test]
    fn only_a_chosen_java_refuses_ghidra() {
        let d = tempfile::tempdir().unwrap();
        let store = d.path().join("store");
        ghidra_folder(&store.join("ghidra/ghidra_12.2_PUBLIC"));
        let old = jdk(&d.path().join("jdk-21"), "21.0.5");
        // Inherited, nothing chosen: still usable, though the Java it would run on is too old.
        let inherited = status_with(None, None, Some(&old), &store, false);
        assert_eq!(inherited.state, GhidraState::Ready, "{inherited:?}");
        assert!(matches!(inherited.java, GhidraJava::TooOld { found: 21, .. }), "{:?}", inherited.java);
        // Chosen for Ghidra: refused before it starts, as `check_java` refuses it.
        let chosen = status_with(None, Some(&old), None, &store, false);
        assert_eq!(chosen.state, GhidraState::JavaTooOld, "{chosen:?}");
        let cfg = Config {
            analyze_headless: store.join("ghidra/ghidra_12.2_PUBLIC/support").join(EXE),
            java_home: Some(old),
            startup_timeout: std::time::Duration::from_secs(60),
            store_dir: store.clone(),
            language_id: None,
            compiler_spec: None,
            memory_gb: Some(8),
        };
        assert!(check_java(&cfg).is_err(), "the launch check refuses what the status refuses");
    }

    /// `check_java` stops a known-too-old Java before Ghidra starts, and lets an unreadable one
    /// or no override through for Ghidra to judge.
    #[test]
    fn check_java_stops_a_known_too_old_java() {
        let d = tempfile::tempdir().unwrap();
        let exe = if cfg!(windows) { "analyzeHeadless.bat" } else { "analyzeHeadless" };
        let launcher = d.path().join("ghidra_12.2_PUBLIC/support").join(exe);
        touch(&launcher);
        let old = jdk(&d.path().join("jdk-21"), "21.0.5");
        let good = jdk(&d.path().join("jdk-25"), "25.0.1");
        let cfg = |java: Option<&Path>| Config {
            analyze_headless: launcher.clone(),
            java_home: java.map(Path::to_path_buf),
            startup_timeout: std::time::Duration::from_secs(60),
            store_dir: d.path().join("store"),
            language_id: None,
            compiler_spec: None,
            memory_gb: Some(8),
        };
        assert!(check_java(&cfg(Some(&old))).is_err(), "Java 21 must not run Ghidra 12.2");
        let err = check_java(&cfg(Some(&old))).unwrap_err().to_string();
        assert!(err.contains("25") && err.contains("21"), "{err}");
        // A preflight refusal, not a start failure: Deep mode must not remember it for the target
        // (an import failure is a `Launch` and stays remembered).
        assert!(matches!(check_java(&cfg(Some(&old))), Err(GhidraError::Preflight(_))), "the Java refusal is the preflight's");
        assert!(check_java(&cfg(Some(&good))).is_ok());
        // Nothing chosen: the inherited `JAVA_HOME` or a `java` on PATH isn't ours to refuse,
        // because a wrapper launcher may pick its own Java.
        assert!(check_java(&cfg(None)).is_ok());
        let unreadable = d.path().join("jdk-broken");
        touch(&unreadable.join("bin").join(if cfg!(windows) { "java.exe" } else { "java" }));
        assert!(check_java(&cfg(Some(&unreadable))).is_ok(), "a version that can't be read is Ghidra's to judge");
        // A folder set as the Java with no `bin/java` in it stops before Ghidra starts.
        let not_a_jdk = d.path().join("not-a-jdk");
        std::fs::create_dir_all(&not_a_jdk).unwrap();
        let not_java = check_java(&cfg(Some(&not_a_jdk))).unwrap_err().to_string();
        assert!(not_java.contains("no bin/java"), "{not_java}");
        // A wrapper launcher that picks its own Java isn't refused over the inherited one.
        let wrapper = d.path().join("bin/ghidra-headless");
        touch(&wrapper);
        let wrapper_cfg = Config { analyze_headless: wrapper, ..cfg(None) };
        assert!(check_java(&wrapper_cfg).is_ok());
        // A legacy install runs on the older floor.
        let legacy = d.path().join("ghidra_12.1.4_PUBLIC/support").join(exe);
        touch(&legacy);
        assert!(check_java(&cfg(Some(&old))).is_err(), "the launcher above is still 12.2");
        let legacy_cfg = Config { analyze_headless: legacy, ..cfg(Some(&old)) };
        assert!(check_java(&legacy_cfg).is_ok(), "12.1.4 kept under the soft policy runs on Java 21");
    }

    /// A store install's `JAVA_HOME_OVERRIDE` is what Ghidra really runs on: it wins over a Java
    /// set in Settings that would be too old, so the launch check and the status agree on the
    /// Java the setup pointed Ghidra at.
    #[test]
    fn a_java_override_wins_over_a_too_old_setting() {
        let d = tempfile::tempdir().unwrap();
        let exe = if cfg!(windows) { "analyzeHeadless.bat" } else { "analyzeHeadless" };
        let support = d.path().join("ghidra_12.2_PUBLIC/support");
        let launcher = support.join(exe);
        touch(&launcher);
        let old = jdk(&d.path().join("jdk-21"), "21.0.5");
        let good = jdk(&d.path().join("jdk-25"), "25.0.1");
        std::fs::write(support.join("launch.properties"), format!("JAVA_HOME_OVERRIDE={}\n", good.display())).unwrap();
        let cfg = Config {
            analyze_headless: launcher.clone(),
            java_home: Some(old.clone()),
            startup_timeout: std::time::Duration::from_secs(60),
            store_dir: d.path().join("store"),
            language_id: None,
            compiler_spec: None,
            memory_gb: Some(8),
        };
        assert!(check_java(&cfg).is_ok(), "the override is the Java Ghidra starts with");
        assert!(matches!(java_in_use(&launcher, Some(&old), None), GhidraJava::Override(dir) if dir == good || dir == std::fs::canonicalize(&good).unwrap()));
    }

    /// An outside Ghidra is reused only when it can really run: this release or newer, and on a
    /// Java its launcher would run on. One on a too-old Java is left alone, so "Get Ghidra" sets
    /// up the store copy with the Java the setup brings instead.
    #[test]
    fn only_an_outside_ghidra_that_can_run_is_reused() {
        let d = tempfile::tempdir().unwrap();
        let store = d.path().join("store");
        let good = jdk(&d.path().join("jdk-25"), "25.0");
        let old = jdk(&d.path().join("jdk-21"), "21.0");
        let outside = install_of(ghidra_folder(&d.path().join("elsewhere/ghidra_12.2_PUBLIC")), GhidraSource::UnpackedFolder);
        assert!(reusable_outside(&outside, &store, Some(&good), None));
        assert!(!reusable_outside(&outside, &store, Some(&old), None), "a too-old Java is not ours to change outside the store");
        // A store install is the managed path, never reused as an outside one.
        let inside = install_of(ghidra_folder(&store.join("ghidra/ghidra_12.2_PUBLIC")), GhidraSource::EngineStore);
        assert!(!reusable_outside(&inside, &store, Some(&good), None));
        // Older than this release: not reused.
        let legacy = install_of(ghidra_folder(&d.path().join("elsewhere/ghidra_12.1.4_PUBLIC")), GhidraSource::UnpackedFolder);
        assert!(!reusable_outside(&legacy, &store, Some(&good), None));
        // A folder that names no version can't be told from an old release: the pinned release is
        // set up in the store instead, so "Get Ghidra" brings this one and not something older.
        let unnamed = install_of(ghidra_folder(&d.path().join("elsewhere/ghidra")), GhidraSource::UnpackedFolder);
        assert_eq!(unnamed.version_hint, None);
        assert!(!reusable_outside(&unnamed, &store, Some(&good), None));
        // A managed JDK new enough for that copy is the Java it would run on, with nothing set for
        // it: resolved for this candidate, not for whichever Ghidra Settings selects.
        jdk(&store.join(JAVA_FOLDER).join("jdk-25.0.5.1+2"), "25.0.1");
        assert!(reusable_outside(&outside, &store, None, None), "the managed Java it needs is enough");
    }

    /// A copy with no Java at all can't be reused: nothing could start it, so "Get Ghidra" sets up
    /// the store copy with the Java it brings instead of reporting that the outside one is ready.
    #[test]
    fn a_ghidra_with_no_java_is_not_something_to_reuse() {
        assert!(!can_start_on(&GhidraJava::Missing));
        assert!(!can_start_on(&GhidraJava::TooOld { dir: PathBuf::from("/jdk-21"), found: 21, needed: 25 }));
        assert!(!can_start_on(&GhidraJava::NotJava(PathBuf::from("/not-a-jdk"))));
        // The rest may start: a good one, one Ghidra was pointed at, and one whose version can't be
        // read (Ghidra gets to judge that, as at launch).
        assert!(can_start_on(&GhidraJava::Override(PathBuf::from("/jdk-25"))));
        assert!(can_start_on(&GhidraJava::OnPath(PathBuf::from("/usr/bin/java"))));
        assert!(can_start_on(&GhidraJava::Unknown(PathBuf::from("/jdk"))));
    }
}
