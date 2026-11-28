//! A build nobody is watching (#1970). A build in a runner process ([`super::runner`]) goes on
//! when the app closes, and that's only what the person wants when they said so. Windows that
//! follow the build hold `inbox/watchers.lock` ([`watchers`]); the run's control thread looks at
//! it every quarter second ([`Watch::look`]). When nobody has watched for [`UNWATCHED_LIMIT`] and
//! the person didn't choose "Keep building" (the app's [`KEEP_FILE`]), the run finishes the turn
//! or check it's on and pauses before it would start another builder turn
//! ([`Watch::should_pause`]), so the app vanishing (a crash, a kill, an update) doesn't leave a
//! build spending the person's AI with nobody there.
//!
//! The keep marker is `inbox/run-keep`, beside `control/` rather than in it: a run clears
//! `control/` as it begins, and "Keep building" can be chosen while a runner is still starting.
//! The app writes it for each build it follows when the person chooses to keep building (in the
//! close dialog, or by the `build.on_close = "keep"` setting), and every window that starts to
//! follow a build removes it, so a later crash of that window pauses the build again. A window
//! that closes without a choice (it wasn't asked to close) writes nothing.
//!
//! While nobody watches, the run also shows a system notification when it ends (ready, stopped,
//! or paused for this reason) and when a question or approval starts waiting
//! ([`craft_anything_platform::notify`]). Its words are English: the runner has no translations.
//!
//! Only runs started with [`RunOptions::when_unwatched`] (the runner's `rebuild-run`) do any of
//! this; a run in the app or from the terminal's `rebuild` never pauses for it.

use super::*;
use craft_anything_engine::workspace::watchers;

/// How long nobody may watch before the build pauses at its next builder turn.
pub const UNWATCHED_LIMIT: Duration = Duration::from_secs(60);
/// What the run says when it pauses because nobody watched.
pub const UNWATCHED_PAUSE: &str = "Paused, because Craft Anything wasn't open to watch this build. Continue picks up where it stopped.";
/// The person chose to keep building with the app closed (in the project's inbox/).
pub const KEEP_FILE: &str = "run-keep";
/// The notifications' title.
pub const NOTIFY_TITLE: &str = "Craft Anything";

/// Where the app says to keep building with nobody watching.
pub fn keep_path(root: &Path) -> PathBuf {
    root.join("inbox").join(KEEP_FILE)
}

/// Says something to the person outside the app: `(title, body)`. The system's notification
/// unless a test swaps it.
#[derive(Clone)]
pub struct Notify(Arc<SendNotification>);

/// What [`Notify`] calls with `(title, body)`.
type SendNotification = dyn Fn(&str, &str) + Send + Sync;

impl Notify {
    pub fn new(f: impl Fn(&str, &str) + Send + Sync + 'static) -> Notify {
        Notify(Arc::new(f))
    }

    /// The system's notification ([`craft_anything_platform::notify`]).
    pub fn system() -> Notify {
        Notify::new(craft_anything_platform::notify)
    }

    fn send(&self, title: &str, body: &str) {
        (self.0)(title, body);
    }
}

impl std::fmt::Debug for Notify {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Notify")
    }
}

/// What a run does when nobody watches it ([`RunOptions::when_unwatched`]).
#[derive(Debug, Clone)]
pub struct WhenUnwatched {
    /// Pause at the next builder turn once nobody has watched for this long.
    pub limit: Duration,
    /// How the person is told it ended or needs them.
    pub notify: Notify,
}

impl WhenUnwatched {
    /// A runner's: [`UNWATCHED_LIMIT`] and the system's notifications.
    pub fn runner() -> WhenUnwatched {
        WhenUnwatched { limit: UNWATCHED_LIMIT, notify: Notify::system() }
    }
}

/// A run's view of who watches it: kept by its control thread, read by its worker.
pub(super) struct Watch {
    rule: WhenUnwatched,
    root: PathBuf,
    /// The project's name, for the notifications.
    name: String,
    /// Since when nobody has watched; `None` while someone does.
    since: Mutex<Option<Instant>>,
    /// The questions and approvals already notified about.
    told: Mutex<HashSet<String>>,
}

impl Watch {
    /// A run of `ws` starting under `rule`, with one look already taken.
    pub(super) fn new(ws: &Workspace, rule: WhenUnwatched) -> Arc<Watch> {
        let watch = Arc::new(Watch { rule, root: ws.root.clone(), name: ws.manifest.name.clone(), since: Mutex::new(None), told: Mutex::new(HashSet::new()) });
        watch.look();
        watch
    }

    /// Look whether anyone watches now (on the control thread: it reads the disk).
    pub(super) fn look(&self) {
        let watched = watchers::watched(&self.root);
        let mut since = self.since.lock().unwrap_or_else(PoisonError::into_inner);
        if watched {
            *since = None;
        } else if since.is_none() {
            *since = Some(Instant::now());
        }
    }

    /// Nobody watched at the last look.
    fn unwatched(&self) -> bool {
        self.since.lock().unwrap_or_else(PoisonError::into_inner).is_some()
    }

    /// Nobody has watched for the rule's limit, and the person didn't say to keep building.
    pub(super) fn should_pause(&self) -> bool {
        let long = self.since.lock().unwrap_or_else(PoisonError::into_inner).is_some_and(|s| s.elapsed() >= self.rule.limit);
        long && !keep_path(&self.root).exists()
    }

    /// A question or approval started waiting: told once, while nobody watches. Sent on a thread
    /// of its own, so the run's events don't wait for the notification's program.
    pub(super) fn asking(&self, request: &Request) {
        if !self.unwatched() || !self.told.lock().unwrap_or_else(PoisonError::into_inner).insert(request.id.clone()) {
            return;
        }
        let (notify, body) = (self.rule.notify.clone(), format!("{}: the build needs your answer.", self.name));
        let sent = std::thread::Builder::new().name("rebuild-notify".into()).spawn(move || notify.send(NOTIFY_TITLE, &body));
        if let Err(e) = sent {
            log::debug!("no notification for a question: {e}");
        }
    }

    /// The run ended, as `(ok, message, paused)` says; `for_nobody`: it paused because nobody
    /// watched. Told while nobody watches, except a pause someone asked for. Waits for the
    /// notification (a few seconds at most): the runner ends right after.
    pub(super) fn finished(&self, ok: bool, message: &str, paused: bool, for_nobody: bool) {
        if !self.unwatched() {
            return;
        }
        let body = match (ok, paused) {
            (true, _) => format!("{}: the build is ready.", self.name),
            (false, true) if for_nobody => format!("{}: the build paused.", self.name),
            (false, true) => return,
            (false, false) => format!("{}: the build stopped: {}", self.name, message.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("it failed.")),
        };
        self.rule.notify.send(NOTIFY_TITLE, &body);
    }
}
