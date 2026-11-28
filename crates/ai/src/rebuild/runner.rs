//! A build in a process of its own, so it keeps going when the app closes (#1970).
//!
//! The app starts the command line's hidden `rebuild-run` command ([`spawn`]): a copy of
//! `craft-anything-cli` that runs one [`RebuildRun`] for one project and ends with it. It never reads
//! a terminal: questions and approvals wait in the project's inbox for whoever is attached, as they
//! do for a run in the app. Its files, all in the project's `inbox/`:
//!
//! ```text
//! run-options.json   what to run (RunnerOptions, versioned); written by spawn, read and deleted by the runner
//! run-error.json     why it couldn't start (RunError, versioned); a successful start removes it
//! run-started.json   the runner's process id once its run started (RunStarted, versioned); gone when it ends
//! run-output.log     what the runner printed, replaced at each start
//! build.lock         held by the run for its whole life (build_lock)
//! run-stop.json      a Pause asked before the run began, for its start's token (attach; the run takes it)
//! watchers.lock      held (shared) by every window following the run (watchers)
//! run-keep           the person chose "Keep building" on closing (unwatched; a window following removes it)
//! ```
//!
//! A runner's run pauses before its next builder turn once nobody has watched for a minute
//! without the person choosing to keep it building, and while nobody watches it says with a
//! system notification when it ends or needs the person ([`super::unwatched`]).
//!
//! The runner is started from a staged copy of the CLI ([`stage`]), never from the installed file:
//! Windows refuses to replace a program that's running, so an update or a rebuild of the CLI would
//! fail for as long as a build runs.

use super::runs::Starter;
use super::{KindRegistry, RebuildRun, RunEvent, RunOptions, Wake};
use crate::AiHub;
use crate::cli_agents::AgentAdapters;
use craft_anything_engine::workspace::{Workspace, build_lock};
use craft_anything_evidence::{ErrorCode, ToolError};
use craft_anything_platform::process::{Launch, Launched, native_path};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

/// The version of the files this module writes. A runner refuses options from a newer one.
pub const RUNNER_VERSION: u32 = 1;
/// The CLI's hidden command that runs a build ([`spawn`] starts it).
pub const SUBCOMMAND: &str = "rebuild-run";
pub const OPTIONS_FILE: &str = "run-options.json";
pub const ERROR_FILE: &str = "run-error.json";
pub const STARTED_FILE: &str = "run-started.json";
pub const OUTPUT_LOG: &str = "run-output.log";
/// In a staged copy's folder: held (shared) by every runner started from it and by [`spawn`] while
/// it starts one, so a cleanup never removes a copy in use.
pub const IN_USE: &str = "in-use.lock";
/// How long [`spawn`] waits for the runner's build to start.
pub const START_WAIT: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(100);

/// The part of [`RunOptions`] that crosses to the runner process: everything but the kinds, the
/// adapters and the wake, which are code (the runner uses its own built-in ones).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunnerOptions {
    pub version: u32,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub shown: Option<String>,
    #[serde(default)]
    pub shown_pictures: Vec<PathBuf>,
    #[serde(default)]
    pub continuing: bool,
    pub max_rounds: u32,
    pub good_enough: f64,
    pub beginner: bool,
    /// The run's mode: Deep (true) or Clean room. Always explicit: the runner's own session may
    /// read different settings by the time it starts.
    pub deep: bool,
    /// Names this start ([`RunOptions::start_token`]), so a Pause asked before the run began
    /// reaches this runner and no later one.
    #[serde(default)]
    pub start_token: Option<String>,
    /// Waiting notes already included in `message` ([`RunOptions::carried_notes`]), so the runner
    /// doesn't hand them to the builder a second time.
    #[serde(default)]
    pub carried_notes: Vec<super::notes::Carried>,
    /// When the person made the request ([`RunOptions::asked`]); `None` (an older app's options)
    /// is taken as when the runner starts.
    #[serde(default)]
    pub asked: Option<std::time::SystemTime>,
}

impl RunnerOptions {
    /// What `opts` asks for, in `deep` mode or Clean room.
    pub fn new(opts: &RunOptions, deep: bool) -> RunnerOptions {
        RunnerOptions {
            version: RUNNER_VERSION,
            backend: opts.backend.clone(),
            message: opts.message.clone(),
            shown: opts.shown.clone(),
            shown_pictures: opts.shown_pictures.clone(),
            continuing: opts.continuing,
            max_rounds: opts.max_rounds,
            good_enough: opts.good_enough,
            beginner: opts.beginner,
            deep,
            start_token: opts.start_token.clone(),
            carried_notes: opts.carried_notes.clone(),
            asked: Some(opts.asked),
        }
    }

    /// The options for [`RebuildRun::start`], with the code a run needs and its mode fixed. What
    /// it does with nobody watching ([`RunOptions::when_unwatched`]) is the runner's to set.
    pub fn into_run_options(self, kinds: KindRegistry, adapters: AgentAdapters, wake: Wake) -> RunOptions {
        RunOptions {
            backend: self.backend,
            message: self.message,
            shown: self.shown,
            shown_pictures: self.shown_pictures,
            continuing: self.continuing,
            max_rounds: self.max_rounds,
            good_enough: self.good_enough,
            beginner: self.beginner,
            kinds,
            adapters,
            wake,
            clean_room: Some(!self.deep),
            when_unwatched: None,
            start_token: self.start_token,
            carried_notes: self.carried_notes,
            asked: self.asked.unwrap_or_else(std::time::SystemTime::now),
        }
    }

    pub fn to_json(&self) -> Result<String, ToolError> {
        serde_json::to_string_pretty(self).map_err(|e| ToolError::internal(format!("Couldn't write the build's options: {e}")))
    }

    /// Read options, refusing ones written by a newer Craft Anything (their meaning may differ).
    pub fn from_json(text: &str) -> Result<RunnerOptions, ToolError> {
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| bad_options(&e))?;
        check_version(&value)?;
        serde_json::from_value(value).map_err(|e| bad_options(&e))
    }

    pub fn read(path: &Path) -> Result<RunnerOptions, ToolError> {
        let text = std::fs::read_to_string(path).map_err(|e| ToolError::io(path.display(), e))?;
        RunnerOptions::from_json(&text)
    }
}

fn bad_options(e: &serde_json::Error) -> ToolError {
    ToolError::invalid(format!("The build's options couldn't be read: {e}")).with_remediation("Start the build again from Craft Anything.")
}

fn check_version(value: &serde_json::Value) -> Result<(), ToolError> {
    let version = value.get("version").and_then(serde_json::Value::as_u64);
    match version {
        Some(v) if v > u64::from(RUNNER_VERSION) => Err(ToolError::new(
            ErrorCode::NotSupported,
            format!("This build was started by a newer Craft Anything (its options are version {v}; this one reads up to {RUNNER_VERSION})."),
        )
        .with_remediation("Update Craft Anything, or start the build from the copy that wrote it.")),
        Some(_) => Ok(()),
        None => Err(ToolError::invalid("The build's options have no version.").with_remediation("Start the build again from Craft Anything.")),
    }
}

/// Why a runner couldn't start its build (`run-error.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunError {
    pub version: u32,
    pub error: ToolError,
}

/// A runner's build started (`run-started.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStarted {
    pub version: u32,
    /// The runner's process id.
    pub pid: u32,
}

/// Record in `inbox` why the runner couldn't start (best effort: the runner is ending anyway).
pub fn write_error(inbox: &Path, error: &ToolError) {
    let record = RunError { version: RUNNER_VERSION, error: error.clone() };
    let written = std::fs::create_dir_all(inbox)
        .map_err(|e| ToolError::io(inbox.display(), e))
        .and_then(|()| serde_json::to_vec_pretty(&record).map_err(|e| ToolError::internal(e.to_string())))
        .and_then(|bytes| craft_anything_platform::fs::write_atomic(&inbox.join(ERROR_FILE), &bytes));
    if let Err(e) = written {
        log::warn!("couldn't record why the build didn't start ({}): {}", error.message, e.message);
    }
}

/// Why the runner for `inbox`'s project couldn't start, if it said. A record it can't read (cut
/// short, or from a newer version) still says it failed.
pub fn read_error(inbox: &Path) -> Option<ToolError> {
    let text = std::fs::read_to_string(inbox.join(ERROR_FILE)).ok()?;
    let parsed = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| check_version(&v).ok().map(|()| v))
        .and_then(|v| serde_json::from_value::<RunError>(v).ok());
    Some(match parsed {
        Some(r) => r.error,
        None => ToolError::internal("The build's runner couldn't start, and its reason couldn't be read.")
            .with_remediation(format!("Its output is in {}.", inbox.join(OUTPUT_LOG).display())),
    })
}

/// Record that this process's build started, and remove an earlier failure's record.
pub fn mark_started(inbox: &Path) {
    let _ = std::fs::remove_file(inbox.join(ERROR_FILE));
    let record = RunStarted { version: RUNNER_VERSION, pid: std::process::id() };
    let written = serde_json::to_vec(&record)
        .map_err(|e| ToolError::internal(e.to_string()))
        .and_then(|bytes| craft_anything_platform::fs::write_atomic(&inbox.join(STARTED_FILE), &bytes));
    if let Err(e) = written {
        log::warn!("couldn't record that the build started: {}", e.message);
    }
}

/// Remove this process's started record, as its run ends.
pub fn mark_ended(inbox: &Path) {
    if read_started(inbox).is_some_and(|s| s.pid == std::process::id()) {
        let _ = std::fs::remove_file(inbox.join(STARTED_FILE));
    }
}

pub fn read_started(inbox: &Path) -> Option<RunStarted> {
    let text = std::fs::read_to_string(inbox.join(STARTED_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Drive a started run until it ends, sending its events nowhere: whoever is attached reads the
/// project's files. `stop` is looked at between polls (Ctrl+C, a termination signal). Returns
/// true when the run finished well or was paused.
pub fn drive(run: &mut RebuildRun, stop: impl Fn() -> bool) -> bool {
    let mut outcome = false;
    let mut stopping = false;
    loop {
        if !stopping && stop() {
            run.stop();
            stopping = true;
        }
        let running = run.is_running();
        for e in run.poll() {
            if let RunEvent::Finished { ok, message, paused } = e {
                log::info!("build ended (ok: {ok}, paused: {paused}): {message}");
                outcome = ok || paused;
            }
        }
        // Read before this round's poll, so the Finished event sent as it stopped was drained.
        if !running {
            return outcome;
        }
        std::thread::sleep(POLL);
    }
}

/// Where the installed `craft-anything-cli` is, beside this program (the app ships next to it):
/// the same lookup that starts the builder's tool server.
pub fn cli_beside_app() -> Option<PathBuf> {
    craft_anything_platform::which::self_cli_path()
}

/// A starter for [`super::RunSet::with_starter`] that runs each build in a runner process of its
/// own ([`start_in_runner`]), started from a copy of `cli` staged under `staging`.
pub fn starter(cli: PathBuf, staging: PathBuf) -> Starter {
    Box::new(move |hub, ws, opts| start_in_runner(hub, ws, opts, &cli, &staging))
}

/// A [`super::RunSet`] of at most `limit` builds at once, each in a runner process of its own
/// ([`starter`]).
pub fn run_set(limit: usize, cli: PathBuf, staging: PathBuf) -> super::RunSet {
    super::RunSet::new(limit).with_starter(starter(cli, staging))
}

/// Start a build of `ws` in a runner process and follow it, returning at once (the app calls it
/// on its UI thread): the runner is started on a thread ([`spawn`]), and the follower shows its
/// run as [`RebuildRun::attach`] would once it has begun.
///
/// Fails at once for what [`RebuildRun::start`] refuses before doing any work and that can be
/// told without touching the project: another build holds it, or no coding agent is set up. The
/// run's mode is fixed here as `start` would choose it. What only the runner can find out (its own
/// settings, a key) ends the follower with the runner's own message
/// ([`super::attach::starting`]). The runner uses the hub's config folder.
pub fn start_in_runner(hub: &AiHub, ws: Workspace, mut opts: RunOptions, cli: &Path, staging: &Path) -> Result<RebuildRun, ToolError> {
    // Names this start, so a Pause asked before its run begins reaches it alone.
    opts.start_token = Some(new_token());
    // The project as it is now, as `RebuildRun::start` reads it.
    let ws = Workspace::open(&ws.root).unwrap_or(ws);
    if build_lock::held(&ws.root) {
        return Err(super::building_elsewhere());
    }
    let cfg = hub.config();
    super::builder_backend(&cfg, opts.backend.as_deref().or_else(|| super::project_builder(&cfg, &ws)))?;
    let deep = super::run_deep_mode(hub, &opts);
    let options = RunnerOptions::new(&opts, deep);
    let config_dir = hub.settings_path().parent().map_or_else(craft_anything_engine::settings::config_dir, Path::to_path_buf);
    let (cli, staging, runner_ws) = (cli.to_path_buf(), staging.to_path_buf(), ws.clone());
    super::attach::starting(&ws, &opts, !deep, move || spawn_in(&cli, &staging, &runner_ws, &options, &config_dir))
}

/// A token no other start has: this process, the time, and a random number.
fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(now);
    h.write_u64(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    format!("{}-{now:x}-{:016x}", std::process::id(), h.finish())
}

/// A staged copy of the CLI, and the hold that keeps a cleanup from removing it until dropped.
#[derive(Debug)]
pub struct Staged {
    pub exe: PathBuf,
    _in_use: InUse,
}

/// A shared hold on a staged copy's [`IN_USE`] file, until dropped.
#[derive(Debug)]
pub struct InUse(File);

impl Drop for InUse {
    /// Let go now, not when the file's last copy closes: a program another thread is starting has a
    /// copy of every open file until it runs (Unix, between fork and exec), and the hold would stay
    /// with it, so a cleanup would leave the copy behind.
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Copy `cli` into its own folder under `staging`, keeping its file name (the CLI finds itself by
/// name to start the builder's tools). The folder is named for the file's size and modified time,
/// so a rebuilt or updated CLI gets a new one and a copy that's running is never written over;
/// a copy already there is used again. Folders of older copies that nothing holds are removed.
///
/// What ships beside the CLI needs no copy: it's one self-contained program (the packages put
/// only `portable.txt` and licences next to it, and portable mode is handed to the runner as its
/// config folder by [`spawn`]).
pub fn stage(cli: &Path, staging: &Path) -> Result<Staged, ToolError> {
    let meta = std::fs::metadata(cli).map_err(|e| ToolError::io(cli.display(), e))?;
    if !meta.is_file() {
        return Err(ToolError::invalid(format!("{} isn't a program.", cli.display())));
    }
    let Some(name) = cli.file_name() else { return Err(ToolError::invalid(format!("{} has no file name.", cli.display()))) };
    let key = staging_key(&meta);
    let dir = staging.join(&key);
    std::fs::create_dir_all(&dir).map_err(|e| ToolError::io(dir.display(), e))?;
    let in_use = hold(&dir.join(IN_USE))?;
    let exe = dir.join(name);
    let ready = std::fs::metadata(&exe).is_ok_and(|m| m.is_file() && m.len() == meta.len());
    if !ready {
        // Copied beside it and renamed, so a copy cut short is never taken for a finished one.
        let partial = dir.join(format!("{}.partial-{}", name.to_string_lossy(), std::process::id()));
        std::fs::copy(cli, &partial).map_err(|e| ToolError::io(partial.display(), e))?;
        let _ = std::fs::remove_file(&exe);
        if let Err(e) = std::fs::rename(&partial, &exe) {
            let _ = std::fs::remove_file(&partial);
            return Err(ToolError::io(exe.display(), e));
        }
    }
    remove_old_copies(staging, &key);
    Ok(Staged { exe, _in_use: in_use })
}

/// A staged copy's folder name: the source's size and modified time.
fn staging_key(meta: &std::fs::Metadata) -> String {
    let modified = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
    format!("{:x}-{modified:x}", meta.len())
}

/// Take a shared hold on a staged copy's lock file. Only a cleanup takes it whole, for a moment.
fn hold(path: &Path) -> Result<InUse, ToolError> {
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path).map_err(|e| ToolError::io(path.display(), e))?;
    for _ in 0..50 {
        match file.try_lock_shared() {
            Ok(()) => return Ok(InUse(file)),
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(20)),
            Err(TryLockError::Error(e)) => return Err(ToolError::io(path.display(), e)),
        }
    }
    Err(ToolError::new(ErrorCode::TimedOut, format!("{} stayed busy.", path.display())).with_remediation("Try again in a moment."))
}

/// Remove the staged folders other than `keep` that no runner holds. Best effort: a folder that
/// can't be removed (on Windows, a copy that's running) stays for the next time.
fn remove_old_copies(staging: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(staging) else { return };
    for entry in entries.flatten() {
        let dir = entry.path();
        if entry.file_name() == keep || !dir.is_dir() {
            continue;
        }
        let lock = dir.join(IN_USE);
        if lock.exists() {
            let Ok(file) = OpenOptions::new().read(true).write(true).open(&lock) else { continue };
            // Held by a runner (or a start under way): leave it. Unix would let us delete a
            // running program, and that runner could then no longer start the builder's tools.
            if file.try_lock().is_err() {
                continue;
            }
        }
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            log::debug!("left the older staged CLI in {}: {e}", dir.display());
        }
    }
}

/// For the runner: hold the staged copy it runs from (its folder's [`IN_USE`] file), so no
/// cleanup removes it while the build runs. `None` when it isn't run from a staged copy.
pub fn hold_staged_copy() -> Option<InUse> {
    let exe = std::env::current_exe().ok()?;
    let lock = exe.parent()?.join(IN_USE);
    if !lock.is_file() {
        return None;
    }
    hold(&lock).ok()
}

/// Start a build of `ws` in a runner process that outlives this one, from a copy of `cli` staged
/// under `staging`, and wait (up to [`START_WAIT`]) until its build has started. The runner uses
/// this process's config folder (and so its settings, and portable mode).
///
/// Errors: another build holds the project; the runner couldn't start its build (its own message
/// and remediation, from `run-error.json`); it ended or didn't start in time (with its log).
pub fn spawn(cli: &Path, staging: &Path, ws: &Workspace, options: &RunnerOptions) -> Result<Launched, ToolError> {
    spawn_in(cli, staging, ws, options, &craft_anything_engine::settings::config_dir())
}

/// [`spawn`] with the runner's config folder given (tests use a temp folder).
pub fn spawn_in(cli: &Path, staging: &Path, ws: &Workspace, options: &RunnerOptions, config_dir: &Path) -> Result<Launched, ToolError> {
    if build_lock::held(&ws.root) {
        return Err(super::building_elsewhere());
    }
    let staged = stage(cli, staging)?;
    let inbox = ws.inbox_dir();
    std::fs::create_dir_all(&inbox).map_err(|e| ToolError::io(inbox.display(), e))?;
    // An earlier runner's records would answer for this one.
    let _ = std::fs::remove_file(inbox.join(ERROR_FILE));
    let _ = std::fs::remove_file(inbox.join(STARTED_FILE));
    let options_path = inbox.join(OPTIONS_FILE);
    craft_anything_platform::fs::write_atomic(&options_path, options.to_json()?.as_bytes())?;
    let log = inbox.join(OUTPUT_LOG);
    let launched = Launch::new(&staged.exe)
        .arg(SUBCOMMAND)
        .arg("--project")
        .arg(native_path(&ws.root))
        .arg("--options")
        .arg(native_path(&options_path))
        .cwd(&ws.root)
        .env("CRAFT_ANYTHING_CONFIG_DIR", config_dir)
        .log_to(&log)
        .detached()
        .label("the build's runner")
        .remediation("Reinstall Craft Anything: its command-line program is missing.")
        .start_watched();
    let launched = match launched {
        Ok(l) => l,
        Err(e) => {
            let _ = std::fs::remove_file(&options_path);
            return Err(e);
        }
    };
    if !launched.outside_our_job() {
        log::info!("the build's runner ({}) is in this process's job: it ends if that job is stopped", launched.pid());
    }
    let result = wait_started(&launched, &inbox, &log, START_WAIT);
    if result.is_err() && launched.is_running() {
        launched.stop();
    }
    drop(staged);
    result.map(|()| launched)
}

/// Wait until `launched`'s build has started, it says why it couldn't, it ends, or `limit` passes.
fn wait_started(launched: &Launched, inbox: &Path, log: &Path, limit: Duration) -> Result<(), ToolError> {
    let deadline = Instant::now() + limit;
    loop {
        let ended = launched.ended();
        if let Some(e) = read_error(inbox) {
            return Err(e);
        }
        if read_started(inbox).is_some_and(|s| s.pid == launched.pid()) {
            return Ok(());
        }
        if let Some(ended) = ended {
            // It ran and finished well before we looked (it removes its record as it ends).
            if ended.code == Some(0) {
                return Ok(());
            }
            let code = ended.code.map_or_else(|| "it was stopped".to_string(), |c| format!("exit code {c}"));
            return Err(ToolError::internal(format!("The build's runner ended before the build started ({code})."))
                .with_remediation(format!("What it printed is in {}.", log.display())));
        }
        if Instant::now() >= deadline {
            return Err(ToolError::new(ErrorCode::TimedOut, format!("The build's runner didn't start the build within {} s.", limit.as_secs()))
                .with_remediation(format!("What it printed is in {}. Try again; if it happens again, report it with that file.", log.display())));
        }
        std::thread::sleep(POLL);
    }
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod starter_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> RunnerOptions {
        let opts = RunOptions {
            backend: Some("Claude Code".into()),
            message: Some("make it blue".into()),
            shown: Some("make it blue".into()),
            shown_pictures: vec![PathBuf::from("pictures/1.png")],
            continuing: true,
            max_rounds: 3,
            good_enough: 85.5,
            beginner: false,
            start_token: Some("this-start".into()),
            carried_notes: vec![super::super::notes::Carried {
                id: 7,
                note: super::super::notes::Left { text: "and green".into(), shown: "and green".into(), pictures: Vec::new(), added: Some(UNIX_EPOCH) },
            }],
            asked: UNIX_EPOCH + Duration::from_secs(5),
            ..RunOptions::default()
        };
        RunnerOptions::new(&opts, true)
    }

    #[test]
    fn options_cross_to_the_runner_unchanged() {
        let o = options();
        let back = RunnerOptions::from_json(&o.to_json().unwrap()).unwrap();
        assert_eq!(back, o);
        let run = back.into_run_options(KindRegistry::built_in(), AgentAdapters::default(), Wake::default());
        assert_eq!(run.backend.as_deref(), Some("Claude Code"));
        assert_eq!(run.message.as_deref(), Some("make it blue"));
        assert_eq!(run.shown_pictures, vec![PathBuf::from("pictures/1.png")]);
        assert!(run.continuing && !run.beginner);
        assert_eq!((run.max_rounds, run.good_enough), (3, 85.5));
        assert_eq!(run.clean_room, Some(false), "Deep mode is carried as the run's mode");
        assert_eq!(run.start_token.as_deref(), Some("this-start"), "and the start it was asked for");
        assert_eq!(run.carried_notes.iter().map(|c| c.id).collect::<Vec<_>>(), [7], "the notes its message already carries");
        assert_eq!(run.asked, UNIX_EPOCH + Duration::from_secs(5), "and when the person asked");
        assert!(run.when_unwatched.is_none(), "the runner sets what it does with nobody watching");
        let clean = RunnerOptions::new(&RunOptions::default(), false).into_run_options(KindRegistry::built_in(), AgentAdapters::default(), Wake::default());
        assert_eq!(clean.clean_room, Some(true));
    }

    #[test]
    fn options_from_a_newer_version_are_refused() {
        let mut v = serde_json::to_value(options()).unwrap();
        v["version"] = serde_json::json!(RUNNER_VERSION + 1);
        let err = RunnerOptions::from_json(&v.to_string()).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotSupported);
        assert!(err.message.contains("newer Craft Anything") && err.remediation.is_some(), "{err:?}");
        v["version"] = serde_json::json!(RUNNER_VERSION);
        v["a_field_from_later"] = serde_json::json!(true);
        assert!(RunnerOptions::from_json(&v.to_string()).is_ok(), "same version, an extra field is ignored");
        let err = RunnerOptions::from_json(r#"{"max_rounds":1}"#).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadInput, "no version: {err:?}");
        assert_eq!(RunnerOptions::from_json("not json").unwrap_err().code, ErrorCode::BadInput);
    }

    #[test]
    fn errors_and_starts_are_recorded_for_the_app() {
        let d = tempfile::tempdir().unwrap();
        let inbox = d.path().join("inbox");
        assert!(read_error(&inbox).is_none());
        write_error(&inbox, &ToolError::invalid("No coding agent").with_remediation("Add one"));
        let e = read_error(&inbox).unwrap();
        assert_eq!((e.message.as_str(), e.remediation.as_deref()), ("No coding agent", Some("Add one")));
        mark_started(&inbox);
        assert!(read_error(&inbox).is_none(), "a start removes an earlier failure");
        assert_eq!(read_started(&inbox), Some(RunStarted { version: RUNNER_VERSION, pid: std::process::id() }));
        mark_ended(&inbox);
        assert!(read_started(&inbox).is_none());
        std::fs::write(inbox.join(ERROR_FILE), "{cut sh").unwrap();
        assert!(read_error(&inbox).unwrap().message.contains("couldn't be read"), "a broken record still says it failed");
    }

    #[test]
    fn a_staged_copy_keeps_its_name_and_is_used_again_until_the_cli_changes() {
        let d = tempfile::tempdir().unwrap();
        let cli = d.path().join(craft_anything_platform::which::cli_file_name());
        std::fs::write(&cli, b"first build").unwrap();
        let staging = d.path().join("staging");
        let first = stage(&cli, &staging).unwrap();
        assert_eq!(first.exe.file_name(), cli.file_name(), "the CLI keeps its own name");
        assert_eq!(std::fs::read(&first.exe).unwrap(), b"first build");
        let again = stage(&cli, &staging).unwrap();
        assert_eq!(again.exe, first.exe, "the same CLI is staged once");
        let first_dir = first.exe.parent().unwrap().to_path_buf();
        drop((first, again));

        // A rebuilt CLI: another size (and time), another folder.
        std::fs::write(&cli, b"second, longer build").unwrap();
        let second = stage(&cli, &staging).unwrap();
        assert_ne!(second.exe.parent(), Some(first_dir.as_path()));
        assert_eq!(std::fs::read(&second.exe).unwrap(), b"second, longer build");
        assert!(!first_dir.exists(), "the older copy nothing holds is removed");

        // One a runner holds stays.
        let held_dir = second.exe.parent().unwrap().to_path_buf();
        let runner_hold = hold(&held_dir.join(IN_USE)).unwrap();
        drop(second);
        std::fs::write(&cli, b"third build, longer still").unwrap();
        let third = stage(&cli, &staging).unwrap();
        assert!(held_dir.join(craft_anything_platform::which::cli_file_name()).is_file(), "a copy in use is kept");
        drop(runner_hold);
        drop(third);
        let fourth = stage(&cli, &staging).unwrap();
        assert!(!held_dir.exists(), "and removed once nothing holds it");
        assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 1);
        drop(fourth);
    }

    /// A program another thread is starting holds a copy of every open file until it runs (Unix:
    /// between fork and exec); a copy of the hold left there mustn't keep an old copy from cleanup.
    #[test]
    fn a_hold_ends_when_dropped_even_with_a_copy_of_its_file_open() {
        let d = tempfile::tempdir().unwrap();
        let cli = d.path().join(craft_anything_platform::which::cli_file_name());
        std::fs::write(&cli, b"first build").unwrap();
        let staging = d.path().join("staging");
        let first = stage(&cli, &staging).unwrap();
        let first_dir = first.exe.parent().unwrap().to_path_buf();
        let starting = first._in_use.0.try_clone().unwrap();
        drop(first);
        std::fs::write(&cli, b"second, longer build").unwrap();
        let second = stage(&cli, &staging).unwrap();
        assert!(!first_dir.exists(), "the older copy nothing holds is removed");
        drop((second, starting));
    }

    #[test]
    fn a_missing_cli_is_an_error_not_a_panic() {
        let d = tempfile::tempdir().unwrap();
        assert!(stage(&d.path().join("nothing"), &d.path().join("staging")).is_err());
        assert!(stage(d.path(), &d.path().join("staging")).is_err(), "a folder isn't a program");
    }
}
