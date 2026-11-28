//! Following a build that another process runs (#1970): a second window, the command line, or the
//! app opened again while its last build went on. The run that owns the project (it holds
//! [`BuildLock`]) is the only writer of its feed, activity log and manifest; everything here
//! goes through two folders in the project's `inbox/`:
//!
//! - `run-events.jsonl`, written by the run: emptied when a run starts, then one JSON line per
//!   change. Line 1 is `{"header": {version, run, clean_room, started_ms}}` (`run` names this
//!   run, so a follower notices a new run replacing the file), line 2 is `{"base": {items}}`, the
//!   feed as it was when the run started, then `{"event": {at_ms, event}}` for every
//!   [`RunEvent`] the run sends (`at_ms` from the run's start) and `{"answered": {id, shown}}`
//!   when a card is marked answered in its feed. A follower takes the base and applies every line
//!   in order with the run's own code ([`feed::FeedLog::record_at`], [`feed::apply`]), so it ends
//!   with the run's feed. The base is a copy, not a count of the saved feed's items, because
//!   applying an event can change items from before the run (a checklist updated in place or
//!   replaced, a card answered, a streaming reply ended), and the saved file already has those
//!   changes.
//! - `control/`, read by the run every [`CONTROL_EVERY`]: `stop` pauses it as its own Pause does,
//!   and `answers/<name>.json` (`{id, shown}`) records how an answer given elsewhere reads in the
//!   feed, so the run stays the feed's one writer. Notes need nothing new: a follower writes them
//!   to the notes folder with ids from [`notes::foreign_id`], and the run takes each one it
//!   doesn't know as one of its waiting notes (and gives it up if its file goes before the builder
//!   reads it).
//! - `run-stop.json` (`{version, token}`), beside `control/` because a run clears that as it
//!   begins: a Pause asked while a runner is still starting ([`starting`]), written at once for the
//!   token that runner was started with ([`RunOptions::start_token`]). The run pauses as it begins
//!   when it finds its own token there (or later, at its next look), and removes it; one with
//!   another token is an earlier start's and goes. So the Pause holds even if the window that
//!   asked closes before the runner has begun.
//! - `watchers.lock` ([`craft_anything_engine::workspace::watchers`]), held (shared) by every
//!   follower for its life, so the run can tell whether anyone watches ([`super::unwatched`]); a
//!   follower also removes the app's "keep building" marker as it begins, so it holds only until
//!   a window watches again.
//!
//! Tool inputs and program output are scrubbed of what looks like a secret before they're logged,
//! as the activity log does, and on Unix the log is readable only by its owner.

use super::activity::Activity;
use super::*;
use craft_anything_engine::journal;
use craft_anything_engine::workspace::watchers::Watching;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};

/// The run's event log, in the project's inbox/.
pub const EVENTS_FILE: &str = "run-events.jsonl";
/// The event log's format. A follower refuses a log with a newer one.
pub const EVENTS_VERSION: u32 = 1;
/// What a follower says when the process running the build ended without saying it finished.
pub const ENDED_ELSEWHERE: &str = "The program running this build closed before the build finished, so it stopped part-way.";
/// What [`RebuildRun::attach`] says when nothing is building the project.
pub const NOT_BUILDING: &str = "This project isn't building anywhere now.";

/// How often a run looks for a stop, answers and notes left by its followers.
const CONTROL_EVERY: Duration = Duration::from_millis(250);
/// How often a follower reads the log.
const FOLLOW_EVERY: Duration = Duration::from_millis(200);
/// How long a follower waits for a run that has just taken the lock to write its first lines.
const HEADER_WAIT: Duration = Duration::from_secs(5);
/// A run releases its lock just before it says it finished, so a follower waits this long after
/// the lock goes before taking the run for lost.
const GONE_GRACE: Duration = Duration::from_secs(1);
/// The most a follower reads in one go, and the longest line it keeps.
const MOST_READ: u64 = 16 << 20;
const LONGEST_LINE: usize = 64 << 20;

pub fn events_path(root: &Path) -> PathBuf {
    root.join("inbox").join(EVENTS_FILE)
}

/// Where followers leave word for the run.
pub fn control_dir(root: &Path) -> PathBuf {
    root.join("inbox").join("control")
}

/// Where a follower asks the run to pause.
pub fn stop_path(root: &Path) -> PathBuf {
    control_dir(root).join("stop")
}

fn answers_dir(root: &Path) -> PathBuf {
    control_dir(root).join("answers")
}

/// A Pause asked before a runner's run began, for that start's token (in the project's inbox/).
pub const RUN_STOP_FILE: &str = "run-stop.json";
/// `run-stop.json`'s format.
pub const RUN_STOP_VERSION: u32 = 1;

pub fn run_stop_path(root: &Path) -> PathBuf {
    root.join("inbox").join(RUN_STOP_FILE)
}

#[derive(Debug, Serialize, Deserialize)]
struct RunStop {
    version: u32,
    token: String,
}

/// The token `run-stop.json` names, if it can be read. A file from a newer version is still a
/// Pause for its token, as long as it says one.
fn run_stop_token(root: &Path) -> Option<String> {
    let bytes = std::fs::read(run_stop_path(root)).ok()?;
    serde_json::from_slice::<Value>(&bytes).ok()?.get("token")?.as_str().map(str::to_string)
}

/// Ask the runner started with `token` to pause before its run has begun.
fn write_run_stop(root: &Path, token: &str) {
    let written = serde_json::to_vec(&RunStop { version: RUN_STOP_VERSION, token: token.to_string() })
        .map_err(|e| ToolError::internal(e.to_string()))
        .and_then(|bytes| craft_anything_platform::fs::write_atomic(&run_stop_path(root), &bytes));
    if let Err(e) = written {
        log::warn!("Couldn't ask the build to pause: {}", e.message);
    }
}

/// A run begins: was a Pause asked for its start (`token`) before it did? The file goes either
/// way: one for another token is from an earlier start.
pub(super) fn stopped_before_start(root: &Path, token: Option<&str>) -> bool {
    let path = run_stop_path(root);
    if !path.exists() {
        return false;
    }
    let asked = token.is_some_and(|t| run_stop_token(root).as_deref() == Some(t));
    let _ = std::fs::remove_file(&path);
    asked
}

/// A follower begins: it watches the build, and a "keep building" said before goes, so a later
/// end of this follower without the choice pauses the build again ([`super::unwatched`]).
fn begin_watching(root: &Path) -> Option<Watching> {
    let _ = std::fs::remove_file(super::unwatched::keep_path(root));
    match Watching::begin(root) {
        Ok(w) => w,
        Err(e) => {
            log::warn!("Couldn't mark the build of {} as watched, so it may pause while shown: {e}", root.display());
            None
        }
    }
}

/// The person chose to keep the build going with nobody watching ([`super::unwatched`]).
pub(super) fn keep_building(root: &Path) {
    if let Err(e) = craft_anything_platform::fs::write_atomic(&super::unwatched::keep_path(root), b"") {
        log::warn!("Couldn't tell the build to keep going while the app is closed: {}", e.message);
    }
}

fn unix_ms() -> u64 {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
}

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Header {
    version: u32,
    /// Names this run, so a follower can tell a new run's log from its own.
    run: String,
    /// The run's mode, for the Just-ask answers a note from a follower may carry.
    clean_room: bool,
    /// When the run began (Unix ms), so a follower's times are the run's.
    started_ms: u64,
}

/// One line as written (borrowed) and as read.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum LineOut<'a> {
    Header(&'a Header),
    Base { items: &'a [FeedItem] },
    Event { at_ms: u64, event: &'a RunEvent },
    Answered { id: &'a str, shown: &'a str },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum LineIn {
    Header(Header),
    Base { items: Vec<FeedItem> },
    Event { at_ms: u64, event: RunEvent },
    Answered { id: String, shown: String },
}

use feed::FeedItem;

/// The run's side of `run-events.jsonl`. Each line is flushed as it's written, so a follower sees
/// it at once; a write that fails is said once and the log stops there, and the run goes on.
pub(super) struct EventLog {
    path: PathBuf,
    file: Option<BufWriter<std::fs::File>>,
    told: bool,
}

impl EventLog {
    /// A run begins (it holds the lock): its log starts empty, and anything an earlier run's
    /// followers left in control/ (a stop it never saw) goes.
    pub(super) fn begin(root: &Path) -> EventLog {
        let _ = std::fs::remove_dir_all(control_dir(root));
        let mut log = EventLog { path: events_path(root), file: None, told: false };
        match open_new(&log.path) {
            Ok(f) => log.file = Some(BufWriter::new(f)),
            Err(e) => log.failed(&e),
        }
        log
    }

    /// The header and the feed as it is when the run starts, `elapsed` into it.
    pub(super) fn start(&mut self, clean_room: bool, elapsed: Duration, base: &[FeedItem]) {
        let run = format!("{}-{}-{:x}", std::process::id(), unix_ms(), {
            use std::hash::{BuildHasher, Hasher};
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u64(unix_ms());
            h.finish()
        });
        let header = Header { version: EVENTS_VERSION, run, clean_room, started_ms: unix_ms().saturating_sub(ms(elapsed)) };
        self.write(&LineOut::Header(&header));
        self.write(&LineOut::Base { items: base });
    }

    pub(super) fn event(&mut self, at: Duration, e: &RunEvent) {
        let e = scrubbed(e);
        self.write(&LineOut::Event { at_ms: ms(at), event: &e });
    }

    pub(super) fn answered(&mut self, id: &str, shown: &str) {
        self.write(&LineOut::Answered { id, shown });
    }

    fn write(&mut self, line: &LineOut) {
        let Some(file) = self.file.as_mut() else { return };
        let written = serde_json::to_writer(&mut *file, line).map_err(std::io::Error::other).and_then(|()| file.write_all(b"\n")).and_then(|()| file.flush());
        if let Err(e) = written {
            // A line may be cut off: nothing more goes after it.
            self.file = None;
            self.failed(&e);
        }
    }

    fn failed(&mut self, e: &std::io::Error) {
        if !std::mem::replace(&mut self.told, true) {
            log::warn!("couldn't write {}, so other windows can't follow this build: {e}", self.path.display());
        }
    }
}

fn open_new(path: &Path) -> std::io::Result<std::fs::File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    // It names the commands the builder ran, as the activity log does.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

/// `e` as it's logged: what a tool was given and what a program printed, with what looks like a
/// secret hidden (field names first, then each string).
fn scrubbed(e: &RunEvent) -> Cow<'_, RunEvent> {
    fn strings(v: &Value) -> Value {
        match v {
            Value::String(s) => Value::String(journal::scrubbed_line(s)),
            Value::Array(a) => Value::Array(a.iter().map(strings).collect()),
            Value::Object(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), strings(v))).collect()),
            other => other.clone(),
        }
    }
    match e {
        RunEvent::Agent(AgentEvent::ToolStarted { call_id, tool, input }) => {
            Cow::Owned(RunEvent::Agent(AgentEvent::ToolStarted { call_id: call_id.clone(), tool: tool.clone(), input: strings(&journal::redacted(input)) }))
        }
        RunEvent::Agent(AgentEvent::Output { text }) => Cow::Owned(RunEvent::Agent(AgentEvent::Output { text: journal::scrubbed_line(text) })),
        _ => Cow::Borrowed(e),
    }
}

/// Look after what followers leave for the run, every [`CONTROL_EVERY`] until `running` goes
/// false (the run finished).
pub(super) fn watch(
    root: PathBuf,
    stop: CancelToken,
    notes: Arc<Mutex<Vec<Note>>>,
    feed: Arc<Mutex<feed::FeedLog>>,
    running: Arc<AtomicBool>,
    control: Control,
) {
    let spawned = std::thread::Builder::new().name("rebuild-control".into()).spawn(move || {
        let mut control = control;
        while running.load(Ordering::SeqCst) {
            std::thread::sleep(CONTROL_EVERY);
            control.once(&root, &stop, &notes, &feed);
        }
    });
    if let Err(e) = spawned {
        log::warn!("Other windows can't pause this build or add to it: {e}");
    }
}

/// The run's side of control/ and of followers' notes.
#[derive(Default)]
pub(super) struct Control {
    /// Followers' notes already taken, so one isn't taken twice once the builder has it.
    adopted: HashSet<u64>,
    /// The runner start this run is ([`RunOptions::start_token`]): a Pause written for it in
    /// `run-stop.json` after the run began is still taken.
    token: Option<String>,
    /// Who watches, for a runner's run ([`super::unwatched`]).
    watch: Option<Arc<super::unwatched::Watch>>,
}

#[derive(Deserialize)]
struct AnswerFile {
    id: String,
    shown: String,
}

impl Control {
    pub(super) fn new(token: Option<String>, watch: Option<Arc<super::unwatched::Watch>>) -> Control {
        Control { adopted: HashSet::new(), token, watch }
    }

    pub(super) fn once(&mut self, root: &Path, stop: &CancelToken, notes: &Mutex<Vec<Note>>, feed: &Mutex<feed::FeedLog>) {
        let stop_file = stop_path(root);
        if stop_file.exists() {
            stop.cancel();
            let _ = std::fs::remove_file(&stop_file);
        }
        // Written by a window that asked before the run began and saw it begin only later.
        if let Some(token) = &self.token
            && run_stop_token(root).as_deref() == Some(token.as_str())
        {
            stop.cancel();
            let _ = std::fs::remove_file(run_stop_path(root));
        }
        if let Some(w) = &self.watch {
            w.look();
        }
        if let Ok(entries) = std::fs::read_dir(answers_dir(root)) {
            let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
            files.sort();
            for path in files {
                if let Some(a) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<AnswerFile>(&b).ok()) {
                    feed.lock().unwrap_or_else(PoisonError::into_inner).answered(&a.id, &a.shown);
                }
                let _ = std::fs::remove_file(&path);
            }
        }
        let dir = notes::notes_dir(&root.join("inbox"));
        // Read the folder without holding the notes, which the screen reads while drawing.
        let theirs: Vec<u64> = notes.lock().unwrap_or_else(PoisonError::into_inner).iter().map(|n| n.id).filter(|id| notes::is_foreign(*id)).collect();
        // A follower's note whose file went before the builder read it was taken back. (The run
        // takes a note out of this list before it removes the note's files.)
        let gone: HashSet<u64> = theirs.into_iter().filter(|id| !notes::is_waiting(&dir, *id) && !notes::was_read(&dir, *id)).collect();
        let new: Vec<notes::Waiting> = notes::waiting(&dir).into_iter().filter(|w| notes::is_foreign(w.id) && !self.adopted.contains(&w.id)).collect();
        if gone.is_empty() && new.is_empty() {
            return;
        }
        let mut notes = notes.lock().unwrap_or_else(PoisonError::into_inner);
        notes.retain(|n| !gone.contains(&n.id));
        for w in new {
            self.adopted.insert(w.id);
            notes.push(waiting_note(w));
        }
    }
}

/// What a follower keeps beside the parts every run has.
#[derive(Debug, Default)]
pub(super) struct Remote {
    /// Where the run serves its preview (its own address, from [`RunEvent::Preview`]).
    preview: Mutex<Option<String>>,
    /// The run has begun (it took the lock and cleared control/), so a stop written now reaches
    /// it. False only while a runner is being started ([`starting`]).
    begun: AtomicBool,
    /// Pause was asked for: written again once the run has begun, if it hadn't yet.
    stop_asked: AtomicBool,
    /// The runner's start ([`RunOptions::start_token`]): a Pause asked before its run began is
    /// written for it at once, so it holds even if this process ends first.
    token: Option<String>,
}

impl Remote {
    fn following() -> Remote {
        Remote { begun: AtomicBool::new(true), ..Remote::default() }
    }

    fn starting(token: Option<String>) -> Remote {
        Remote { token, ..Remote::default() }
    }

    pub(super) fn preview_url(&self) -> Option<String> {
        self.preview.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Ask the run to pause. Before it has begun the request can't go in control/ (a run clears
    /// it as it begins, and until then the lock may still be another build's): it's remembered,
    /// and written for this start's token in `run-stop.json`, which only that runner takes.
    pub(super) fn ask_to_stop(&self, root: &Path) {
        self.stop_asked.store(true, Ordering::SeqCst);
        if self.begun.load(Ordering::SeqCst) {
            write_stop(root);
        } else if let Some(token) = &self.token {
            write_run_stop(root, token);
        }
    }

    /// The run has begun: a pause asked for before now is written for it. (Whichever of this and
    /// [`ask_to_stop`](Self::ask_to_stop) comes second sees the other's flag, so it's never lost.)
    fn begun(&self, root: &Path) {
        self.begun.store(true, Ordering::SeqCst);
        if self.stop_asked.load(Ordering::SeqCst) {
            write_stop(root);
        }
    }
}

/// Reads the log from where it last stopped, a line at a time.
struct Reader {
    file: std::fs::File,
    offset: u64,
    partial: Vec<u8>,
    /// A line too long to keep is being skipped to its end.
    skipping: bool,
}

/// The log isn't the one this reader was reading.
struct Replaced;

impl Reader {
    fn open(root: &Path) -> std::io::Result<Reader> {
        Ok(Reader { file: std::fs::File::open(events_path(root))?, offset: 0, partial: Vec::new(), skipping: false })
    }

    /// The complete lines written since the last read.
    fn lines(&mut self) -> Result<Vec<Vec<u8>>, Replaced> {
        let Ok(len) = self.file.metadata().map(|m| m.len()) else { return Ok(Vec::new()) };
        if len < self.offset {
            return Err(Replaced);
        }
        let mut out = Vec::new();
        let mut want = len.saturating_sub(self.offset);
        while want > 0 {
            let mut bytes = Vec::new();
            let read = self.file.seek(SeekFrom::Start(self.offset)).and_then(|_| (&mut self.file).take(want.min(MOST_READ)).read_to_end(&mut bytes));
            match read {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    self.offset = self.offset.saturating_add(n as u64);
                    want = want.saturating_sub(n as u64);
                }
            }
            for part in bytes.split_inclusive(|b| *b == b'\n') {
                let complete = part.last() == Some(&b'\n');
                if self.skipping {
                    self.skipping = !complete;
                    continue;
                }
                self.partial.extend_from_slice(part);
                if complete {
                    out.push(std::mem::take(&mut self.partial));
                } else if self.partial.len() > LONGEST_LINE {
                    log::warn!("A line of a build's event log is too long to follow; it's skipped.");
                    self.partial.clear();
                    self.skipping = true;
                }
            }
        }
        Ok(out)
    }
}

/// The run that wrote the log at `root` now, from its first line.
fn log_run(root: &Path) -> Option<String> {
    let mut first = Vec::new();
    std::fs::File::open(events_path(root)).ok()?.take(4096).read_to_end(&mut first).ok()?;
    let line = first.split(|b| *b == b'\n').next()?;
    match serde_json::from_slice::<LineIn>(line).ok()? {
        LineIn::Header(h) => Some(h.run),
        _ => None,
    }
}

/// The start of a run's log: its header, the feed it began with, and the lines after them.
type LogStart = (Reader, Header, Vec<FeedItem>, Vec<Vec<u8>>);

/// The log's first two lines, once both are there. A log from a newer version is refused.
fn read_start(root: &Path) -> Result<Option<LogStart>, ToolError> {
    let Ok(mut reader) = Reader::open(root) else { return Ok(None) };
    let Ok(mut lines) = reader.lines() else { return Ok(None) };
    if lines.len() < 2 {
        return Ok(None);
    }
    let rest = lines.split_off(2);
    let (Some(first), Some(second)) = (lines.first(), lines.get(1)) else { return Ok(None) };
    let version = serde_json::from_slice::<Value>(first).ok().and_then(|v| v.get("header")?.get("version")?.as_u64());
    if version.is_some_and(|v| v > u64::from(EVENTS_VERSION)) {
        return Err(ToolError::new(ErrorCode::NotSupported, "This build was started by a newer Craft Anything, so this one can't follow it.")
            .with_remediation("Update Craft Anything to follow it here, or watch it where it runs."));
    }
    let (Ok(LineIn::Header(header)), Ok(LineIn::Base { items })) = (serde_json::from_slice::<LineIn>(first), serde_json::from_slice::<LineIn>(second)) else {
        return Ok(None);
    };
    Ok(Some((reader, header, items, rest)))
}

fn not_building() -> ToolError {
    ToolError::new(ErrorCode::NotFound, NOT_BUILDING).with_remediation("Start a build, or open the project to see how its last one ended.")
}

/// The log's start, waiting (up to [`HEADER_WAIT`]) for a run that has just taken the lock to
/// write it. Fails when nothing holds the project and there's no log, or the log can't be read.
fn wait_for_log(root: &Path) -> Result<LogStart, ToolError> {
    let deadline = Instant::now() + HEADER_WAIT;
    loop {
        if let Some(start) = read_start(root)? {
            return Ok(start);
        }
        if !build_lock::held(root) {
            return Err(not_building());
        }
        if Instant::now() >= deadline {
            return Err(ToolError::new(ErrorCode::NotSupported, "This project is building, but its build can't be followed from here.")
                .with_remediation("Watch it where it runs; a build started by an older Craft Anything can't be followed."));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// What a follower ([`RebuildRun`]) shares with the thread that reads its run's log.
#[derive(Clone)]
struct Parts {
    root: PathBuf,
    feed: Arc<Mutex<feed::FeedLog>>,
    activity: Arc<Activity>,
    running: Arc<AtomicBool>,
    remote: Arc<Remote>,
    notes: Arc<Mutex<Vec<Note>>>,
    wake: Wake,
}

impl Parts {
    /// A follower's parts, its feed `items` until the run's log says otherwise.
    fn new(root: &Path, items: Vec<FeedItem>, activity: Arc<Activity>, remote: Remote, wake: Wake) -> Parts {
        Parts {
            root: root.to_path_buf(),
            feed: Arc::new(Mutex::new(feed::FeedLog::mirror(root, items, activity.clone()))),
            activity,
            running: Arc::new(AtomicBool::new(true)),
            remote: Arc::new(remote),
            notes: Arc::default(),
            wake,
        }
    }

    /// The follower itself. `stop` ends its reading thread (it's cancelled when the follower is
    /// dropped), never the build.
    fn run(&self, rx: Receiver<RunEvent>, stop: CancelToken, clean_room: bool, started: Instant) -> RebuildRun {
        RebuildRun {
            rx,
            stop,
            inbox: Inbox::new(self.root.join("inbox")),
            running: self.running.clone(),
            preview: Arc::default(),
            notes: self.notes.clone(),
            clean_room,
            feed: self.feed.clone(),
            root: self.root.clone(),
            started,
            activity: self.activity.clone(),
            remote: Some(self.remote.clone()),
            _watching: begin_watching(&self.root),
        }
    }

    /// Follow the run whose log began with `start`: everything it has said so far is taken in
    /// before this returns, so the feed is whole from the start.
    fn tail(&self, tx: Sender<RunEvent>, start: LogStart) -> Tail {
        let (reader, header, _, rest) = start;
        let mut tail = Tail { parts: self.clone(), tx, run: header.run, reader, finished: false, gone_since: None, told_bad_line: false };
        tail.take(rest);
        tail.refresh_notes();
        tail
    }

    /// Recorded and sent under one lock, as the run does, so `feed_now` sees each event once.
    fn send(&self, tx: &Sender<RunEvent>, e: RunEvent, at: Duration) {
        let mut feed = self.feed.lock().unwrap_or_else(PoisonError::into_inner);
        feed.record_at(&e, at);
        let _ = tx.send(e);
    }

    /// The run couldn't be started or followed: say why, once, and stop. `never_ran`: no runner's
    /// build began, so nothing else will record how the project stands; it's saved here unless
    /// another build holds the project by now (that build writes its manifest, no one else).
    fn failed(&self, tx: &Sender<RunEvent>, e: &ToolError, never_ran: bool) {
        let message = e.for_people();
        if never_ran
            && !build_lock::held(&self.root)
            && let Ok(mut ws) = Workspace::open(&self.root)
        {
            let _ = ws.set_status(Status::NeedsYou, message.clone());
        }
        self.send(tx, RunEvent::Finished { ok: false, message, paused: false }, self.activity.elapsed());
        self.running.store(false, Ordering::SeqCst);
        self.wake.call();
    }
}

/// Read `tail`'s log every [`FOLLOW_EVERY`] until its run ends or `quit` is cancelled.
fn follow(mut tail: Tail, quit: &CancelToken) {
    while !tail.finished && !quit.is_cancelled() {
        std::thread::sleep(FOLLOW_EVERY);
        if quit.is_cancelled() || !tail.step() {
            return;
        }
    }
}

/// A follower's copy of a run: it reads the log and keeps the parts every run has up to date.
struct Tail {
    parts: Parts,
    tx: Sender<RunEvent>,
    run: String,
    reader: Reader,
    finished: bool,
    /// When the lock was first seen free without the run saying it finished.
    gone_since: Option<Instant>,
    told_bad_line: bool,
}

impl Tail {
    /// Take in complete lines; true when any changed something.
    fn take(&mut self, lines: Vec<Vec<u8>>) -> bool {
        let mut changed = false;
        for line in lines {
            if self.finished {
                break;
            }
            match serde_json::from_slice::<LineIn>(&line) {
                Ok(LineIn::Event { at_ms, event }) => {
                    if let RunEvent::Preview(url) = &event {
                        *self.parts.remote.preview.lock().unwrap_or_else(PoisonError::into_inner) = Some(url.clone());
                    }
                    let done = matches!(event, RunEvent::Finished { .. });
                    self.send(event, Duration::from_millis(at_ms));
                    if done {
                        self.end();
                    }
                    changed = true;
                }
                Ok(LineIn::Answered { id, shown }) => {
                    self.parts.feed.lock().unwrap_or_else(PoisonError::into_inner).answered(&id, &shown);
                    changed = true;
                }
                Ok(LineIn::Header(_) | LineIn::Base { .. }) => {}
                Err(e) => {
                    if !std::mem::replace(&mut self.told_bad_line, true) {
                        log::warn!("Skipped a line of {} that couldn't be read: {e}", events_path(&self.parts.root).display());
                    }
                }
            }
        }
        changed
    }

    fn send(&self, e: RunEvent, at: Duration) {
        self.parts.send(&self.tx, e, at);
    }

    fn end(&mut self) {
        self.finished = true;
        self.parts.running.store(false, Ordering::SeqCst);
    }

    /// The run is gone without saying it finished: say so, once.
    fn lost(&mut self) {
        if !self.finished {
            let at = self.parts.activity.elapsed();
            self.send(RunEvent::Finished { ok: false, message: ENDED_ELSEWHERE.into(), paused: false }, at);
            self.end();
        }
    }

    fn ours(&self) -> bool {
        log_run(&self.parts.root).as_deref() == Some(self.run.as_str())
    }

    /// One look at the log, the lock and the notes. False once it's done following.
    fn step(&mut self) -> bool {
        let mut changed = false;
        if !self.ours() {
            self.lost();
            self.parts.wake.call();
            return false;
        }
        match self.reader.lines() {
            // A new run's lines aren't this run's: read again only while the log is still ours.
            Ok(lines) if self.ours() => changed |= self.take(lines),
            Ok(_) | Err(Replaced) => self.lost(),
        }
        if !self.finished {
            if build_lock::held(&self.parts.root) {
                self.gone_since = None;
            } else if self.gone_since.get_or_insert_with(Instant::now).elapsed() >= GONE_GRACE {
                // Its last words may have come in the meantime.
                if let Ok(lines) = self.reader.lines()
                    && self.ours()
                {
                    self.take(lines);
                }
                self.lost();
            }
            changed |= self.refresh_notes();
        }
        if changed || self.finished {
            self.parts.wake.call();
        }
        !self.finished
    }

    /// The notes waiting in the project's folder, whoever added them.
    fn refresh_notes(&self) -> bool {
        let waiting: Vec<Note> = notes::waiting(&notes::notes_dir(&self.parts.root.join("inbox"))).into_iter().map(waiting_note).collect();
        let mut notes = self.parts.notes.lock().unwrap_or_else(PoisonError::into_inner);
        if *notes == waiting {
            return false;
        }
        *notes = waiting;
        true
    }
}

impl RebuildRun {
    /// Follow the build another process runs on `ws` (another window, the command line), with the
    /// same calls as a run started here: its events arrive by [`poll`](Self::poll), and
    /// [`stop`](Self::stop), notes and answers reach it through the project's files. Dropping the
    /// follower leaves the build running. It reads the disk (and waits a moment for a build that
    /// has just started), so call it on a worker.
    ///
    /// Fails when nothing builds the project, or when the build was started by a newer version.
    pub fn attach(ws: &Workspace, wake: Wake) -> Result<RebuildRun, ToolError> {
        let root = ws.root.clone();
        if !build_lock::held(&root) {
            return Err(not_building());
        }
        // A run takes the lock first and writes its first lines a moment later.
        let start = wait_for_log(&root)?;
        let header = start.1.clone();
        let began = Instant::now().checked_sub(Duration::from_millis(unix_ms().saturating_sub(header.started_ms))).unwrap_or_else(Instant::now);
        let activity = Arc::new(Activity::began_at(vec![ws.app_dir(), root.clone()], began));
        let parts = Parts::new(&root, start.2.clone(), activity, Remote::following(), wake);
        let (tx, rx) = std::sync::mpsc::channel();
        let tail = parts.tail(tx, start);
        // Cancelled when the follower is dropped: it ends the reading thread, not the build.
        let stop = CancelToken::default();
        if !tail.finished {
            let quit = stop.clone();
            std::thread::Builder::new().name("rebuild-follow".into()).spawn(move || follow(tail, &quit)).map_err(|e| ToolError::internal(e.to_string()))?;
        }
        Ok(parts.run(rx, stop, header.clean_room, began))
    }
}

/// A follower for a build that `start` is starting in another process (a runner, [`super::runner`]),
/// returned at once: `start` runs on a thread of its own and returns once that process's run has
/// begun (it holds the lock and has written its log's start), or why it couldn't. Until the log
/// is read the feed is the project's saved one with the person's message (`opts.shown`), so the
/// screen isn't empty meanwhile; from then on it's the run's.
///
/// A failed start ends the follower with one `Finished { ok: false }` carrying the error's message
/// and remediation, and saves the project as needing the person unless a build holds it. A pause
/// asked for before the run began reaches it as soon as it has. Dropping the follower leaves the
/// build running (a start under way still finishes, and a pause asked for is still delivered).
/// `handle` (what `start` returns, e.g. the runner's process) is kept while the build is followed.
///
/// `opts.start_token` names this start: a pause asked for before the run began is also written
/// at once for it (`run-stop.json`), so it holds even if this process ends before the run begins.
pub(super) fn starting<H: Send + 'static>(
    ws: &Workspace,
    opts: &RunOptions,
    clean_room: bool,
    start: impl FnOnce() -> Result<H, ToolError> + Send + 'static,
) -> Result<RebuildRun, ToolError> {
    let root = ws.root.clone();
    let activity = Arc::new(Activity::new(vec![ws.app_dir(), root.clone()]));
    let mut items = feed::load(ws);
    if let Some(shown) = &opts.shown {
        items.push(FeedItem::User(feed::UserItem::new(shown.clone(), opts.shown_pictures.clone())));
    }
    let parts = Parts::new(&root, items, activity, Remote::starting(opts.start_token.clone()), opts.wake.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    let stop = CancelToken::default();
    let (p, quit) = (parts.clone(), stop.clone());
    std::thread::Builder::new()
        .name("rebuild-runner".into())
        .spawn(move || {
            let handle = match start() {
                Ok(h) => h,
                Err(e) => return p.failed(&tx, &e, true),
            };
            // Its run has cleared control/: a pause asked for meanwhile can be written now.
            p.remote.begun(&p.root);
            match wait_for_log(&p.root) {
                Err(e) => p.failed(&tx, &e, false),
                Ok(start) => {
                    *p.feed.lock().unwrap_or_else(PoisonError::into_inner) = feed::FeedLog::mirror(&p.root, start.2.clone(), p.activity.clone());
                    let tail = p.tail(tx, start);
                    p.wake.call();
                    follow(tail, &quit);
                }
            }
            drop(handle);
        })
        .map_err(|e| ToolError::internal(e.to_string()))?;
    Ok(parts.run(rx, stop, clean_room, Instant::now()))
}

/// A note waiting in the project's folder, as the run's own list holds it. A note from a version
/// that kept no time counts as added long ago, the same every time it's read, so the list doesn't
/// look changed at each look, and a stop of Ghidra since holds.
fn waiting_note(w: notes::Waiting) -> Note {
    Note {
        id: w.id,
        text: w.left.text,
        shown: w.left.shown,
        pictures: w.left.pictures,
        questions: w.questions,
        added: w.left.added.unwrap_or(std::time::UNIX_EPOCH),
    }
}

/// A follower asks the run to pause ([`Remote::ask_to_stop`]).
fn write_stop(root: &Path) {
    if let Err(e) = craft_anything_platform::fs::write_atomic(&stop_path(root), b"") {
        log::warn!("Couldn't ask the build to pause: {}", e.message);
    }
}

/// A follower answers a card: the answer goes to the inbox (the builder is waiting on it), and how
/// it reads goes to the run, which records it in the feed. A run that is gone by then can't, so
/// it goes straight into the saved feed, as with no run.
pub(super) fn answer(run: &RebuildRun, id: &str, answer: &Answer, shown: &str) -> Result<(), ToolError> {
    let request = run
        .feed
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .items()
        .iter()
        .find_map(|i| match i {
            FeedItem::Ask { request, .. } if request.id == id => Some(request.clone()),
            _ => None,
        })
        .or_else(|| run.inbox.pending().into_iter().find(|r| r.id == id));
    run.inbox.answer(id, answer)?;
    let told = build_lock::held(&run.root) && {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = format!("{}-{}-{}.json", unix_ms(), std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        let bytes = serde_json::to_vec(&json!({"id": id, "shown": shown})).map_err(|e| ToolError::internal(e.to_string()))?;
        craft_anything_platform::fs::write_atomic(&answers_dir(&run.root).join(name), &bytes).is_ok()
    };
    if !told && let Some(request) = request {
        feed::record_answer(&Workspace::open(&run.root)?, &request, shown)?;
    }
    run.feed.lock().unwrap_or_else(PoisonError::into_inner).answered(id, shown);
    Ok(())
}

/// A follower takes the notes still waiting (the run ended first): each one whose file it can
/// take back, so no other run gets it too.
pub(super) fn take_notes(run: &RebuildRun) -> Vec<Note> {
    let dir = notes::notes_dir(run.inbox.dir());
    let waiting = std::mem::take(&mut *run.notes.lock().unwrap_or_else(PoisonError::into_inner));
    waiting.into_iter().filter(|n| notes::take_back(&dir, n.id)).collect()
}

#[cfg(test)]
#[path = "attach_tests.rs"]
mod tests;
