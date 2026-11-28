//! Stand-in runs, for tests of whatever holds a [`RunSet`] (the app's screens, a fork's own front
//! end): they start at once, run no coding agent, and report only what the test tells them to.
//! Each keeps its project's feed like a real run, so opening a project shows what it reported.

use super::runs::Starter;
use super::*;

struct Handle {
    root: PathBuf,
    /// Started to carry on where the last run stopped (`RunOptions::continuing`).
    continuing: bool,
    running: Arc<AtomicBool>,
    tx: Sender<RunEvent>,
    feed: Arc<Mutex<feed::FeedLog>>,
}

/// Hands out stand-in runs ([`StandIns::starter`]) and drives them.
#[derive(Default, Clone)]
pub struct StandIns {
    started: Arc<Mutex<Vec<Handle>>>,
    /// Projects (by name) whose runs fail to start.
    failing: Vec<String>,
}

impl StandIns {
    pub fn new() -> StandIns {
        StandIns::default()
    }

    /// Runs on the project named `name` fail to start, as with no coding agent.
    pub fn failing(mut self, name: &str) -> StandIns {
        self.failing.push(name.to_string());
        self
    }

    /// A starter for [`RunSet::with_starter`].
    pub fn starter(&self) -> Starter {
        let (all, failing) = (self.started.clone(), self.failing.clone());
        Box::new(move |_hub, ws, opts| {
            if failing.contains(&ws.manifest.name) {
                return Err(ToolError::invalid("No coding agent."));
            }
            let (tx, rx) = std::sync::mpsc::channel();
            let running = Arc::new(AtomicBool::new(true));
            let clean_room = opts.clean_room.unwrap_or(false);
            let feed = Arc::new(Mutex::new(feed::FeedLog::open(&ws)));
            if let Some(shown) = opts.shown {
                feed.lock().unwrap_or_else(PoisonError::into_inner).push(feed::FeedItem::User(feed::UserItem::new(shown, opts.shown_pictures)));
            }
            all.lock().unwrap_or_else(PoisonError::into_inner).push(Handle {
                root: ws.root.clone(),
                continuing: opts.continuing,
                running: running.clone(),
                tx,
                feed: feed.clone(),
            });
            let activity = feed.lock().unwrap_or_else(PoisonError::into_inner).activity();
            Ok(RebuildRun {
                activity,
                rx,
                stop: CancelToken::default(),
                inbox: Inbox::new(ws.inbox_dir()),
                running,
                preview: Arc::default(),
                notes: Arc::default(),
                clean_room,
                feed,
                root: ws.root.clone(),
                started: std::time::Instant::now(),
                remote: None,
                _watching: None,
            })
        })
    }

    /// Every run started so far, by project folder, in order.
    pub fn started(&self) -> Vec<PathBuf> {
        self.started.lock().unwrap_or_else(PoisonError::into_inner).iter().map(|h| h.root.clone()).collect()
    }

    /// The runs started to carry on where the last one stopped, by project folder, in order.
    pub fn continued(&self) -> Vec<PathBuf> {
        self.started.lock().unwrap_or_else(PoisonError::into_inner).iter().filter(|h| h.continuing).map(|h| h.root.clone()).collect()
    }

    /// The project's latest run reports `e` (kept in its feed too). A `Finished` ends it.
    /// False when no run was started for it.
    pub fn send(&self, root: &Path, e: RunEvent) -> bool {
        let all = self.started.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(h) = all.iter().rev().find(|h| h.root == root) else { return false };
        h.feed.lock().unwrap_or_else(PoisonError::into_inner).record(&e);
        let done = matches!(e, RunEvent::Finished { .. });
        let _ = h.tx.send(e);
        if done {
            h.running.store(false, Ordering::SeqCst);
        }
        true
    }

    /// The project's latest run finishes well ("Done.").
    pub fn finish(&self, root: &Path) -> bool {
        self.send(root, RunEvent::Finished { ok: true, message: "Done.".into(), paused: false })
    }
}

/// For tests: a build as the process that owns it sets it up (its lock, event log, feed and
/// control folder, as [`RebuildRun::start`] does), without a worker, so a test can follow it from
/// "another process" ([`RebuildRun::attach`]) and say what it does. The lock is per open file, so
/// a follower in the same test process sees it as a build elsewhere. It does nothing by itself:
/// [`Elsewhere::look`] takes in what followers left, as the run's control thread would.
pub struct Elsewhere {
    pub(super) lock: Option<BuildLock>,
    pub(super) root: PathBuf,
    pub(super) feed: Arc<Mutex<feed::FeedLog>>,
    pub(super) tx: Sender<RunEvent>,
    pub(super) rx: Receiver<RunEvent>,
    pub(super) stop: CancelToken,
    pub(super) notes: Arc<Mutex<Vec<Note>>>,
    pub(super) running: Arc<AtomicBool>,
    pub(super) control: attach::Control,
}

impl Elsewhere {
    /// Begin a build of `ws` that shows `shown` as the person's message. Fails when another
    /// build holds the project.
    pub fn start(ws: &Workspace, shown: &str) -> Result<Elsewhere, ToolError> {
        let lock = take_build_lock(ws)?;
        let events = attach::EventLog::begin(&ws.root);
        let feed = Arc::new(Mutex::new(feed::FeedLog::open(ws)));
        feed.lock().unwrap_or_else(PoisonError::into_inner).push(feed::FeedItem::User(feed::UserItem::new(shown.into(), Vec::new())));
        feed.lock().unwrap_or_else(PoisonError::into_inner).log_events(events, false);
        let (tx, rx) = std::sync::mpsc::channel();
        Ok(Elsewhere {
            lock: Some(lock),
            root: ws.root.clone(),
            feed,
            tx,
            rx,
            stop: CancelToken::default(),
            notes: Arc::default(),
            running: Arc::new(AtomicBool::new(true)),
            control: attach::Control::default(),
        })
    }

    /// The build reports `e`, to its feed and its event log.
    pub fn emit(&self, e: RunEvent) {
        emit_event(&self.feed, &self.tx, &Wake::default(), e);
    }

    /// One look at what followers left (a pause, answers, notes), as the run's control thread does.
    pub fn look(&mut self) {
        self.control.once(&self.root, &self.stop, &self.notes, &self.feed);
    }

    /// A follower asked it to pause (seen by [`Elsewhere::look`]).
    pub fn pause_asked(&self) -> bool {
        self.stop.is_cancelled()
    }

    /// The notes followers left that it has taken, waiting for its builder.
    pub fn waiting(&self) -> Vec<Note> {
        self.notes.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Ends as the run does: the lock first, then the word.
    pub fn finish(&mut self, ok: bool, message: &str, paused: bool) {
        self.lock = None;
        self.emit(RunEvent::Finished { ok, message: message.into(), paused });
        self.running.store(false, Ordering::SeqCst);
    }

    /// What it has reported since the last call.
    pub fn events(&self) -> Vec<RunEvent> {
        self.rx.try_iter().collect()
    }

    /// Its feed as it is now.
    pub fn items(&self) -> Vec<feed::FeedItem> {
        self.feed.lock().unwrap_or_else(PoisonError::into_inner).items().to_vec()
    }
}
