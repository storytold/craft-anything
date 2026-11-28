//! Several rebuilds at once. A [`RunSet`] keeps every project's run by folder: up to its limit
//! work at the same time, the rest wait in order and start as others finish. The app and the
//! rebuild tools both hold one; each run keeps its own project's feed, so nothing here depends on
//! which project is on screen.

use super::*;
use std::collections::{HashMap, VecDeque};

/// How a prepared run is started; the built-in worker unless a fork or a test swaps it.
/// `RunSet` has already restored its request and owns cleanup of the saved record.
pub type Starter = Box<dyn FnMut(&AiHub, Workspace, RunOptions) -> Result<RebuildRun, ToolError> + Send>;

/// What [`RunSet::start`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Started {
    Running,
    /// Waiting for a free place; 1 starts next.
    Queued {
        position: usize,
    },
}

/// What [`RunSet::stop`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    /// Asked to stop; its `Finished` event follows.
    Stopping,
    /// Paused before it started. Its request stays for the next manual start.
    Unqueued,
    Nothing,
}

/// An event and, at the end of its polled batch, the display that belongs to that same run.
#[derive(Debug)]
pub struct RunUpdate {
    pub root: PathBuf,
    pub event: RunEvent,
    /// The originating project, even when polling starts a later run at the same path.
    project: Option<feed::ProjectIdentity>,
    /// Only the last update in a run's batch carries its display. Failed starts have none.
    pub display_feed: Option<Vec<feed::FeedItem>>,
    /// The queued start failed before a run existed, so earlier errors cannot explain it.
    pub start_failed: bool,
}

impl RunUpdate {
    pub fn is_for_project(&self, root: &Path, project: &feed::ProjectIdentity) -> bool {
        self.root.as_path() == root && self.project.as_ref().is_some_and(|owner| owner.is_same_project(project))
    }

    /// Install the writer's display without folding events through its display-only notices.
    /// Callers still handle every event's workflow effects separately.
    pub fn update_feed(&mut self, items: &mut Vec<feed::FeedItem>) {
        if let Some(display) = self.display_feed.take() {
            *items = display;
        } else if self.start_failed {
            let mut explained = false;
            feed::apply_run_event(items, &self.event, &mut explained);
        }
    }
}

struct TrackedRun {
    run: RebuildRun,
    project: feed::ProjectIdentity,
    /// A temporary manifest read failure must not discard a finished run's workflow events.
    pending: Vec<RunEvent>,
}

impl TrackedRun {
    fn new(run: RebuildRun) -> Result<Self, ToolError> {
        let project = run.project_identity().ok_or_else(|| {
            ToolError::new(ErrorCode::FileChanged, "The project's identity couldn't be confirmed for this run.")
                .with_remediation("Open the current project and start again.")
        })?;
        Ok(Self { run, project, pending: Vec::new() })
    }
}

pub struct RunSet {
    limit: usize,
    /// Running and finished runs (a finished one keeps its preview and unsent notes).
    runs: HashMap<PathBuf, TrackedRun>,
    queue: VecDeque<(Workspace, RunOptions, feed::ProjectIdentity)>,
    starter: Starter,
    /// A [`keep_queue_moving`] thread is looking after the queue.
    watched: bool,
}

impl std::fmt::Debug for RunSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunSet").field("limit", &self.limit).field("running", &self.active()).field("queued", &self.queue.len()).finish()
    }
}

impl RunSet {
    /// At most `limit` runs work at once (at least one).
    pub fn new(limit: usize) -> RunSet {
        RunSet { limit: limit.max(1), runs: HashMap::new(), queue: VecDeque::new(), starter: Box::new(RebuildRun::start_prepared), watched: false }
    }

    /// Start runs another way (a fork's runner, or a test's).
    pub fn with_starter(mut self, starter: Starter) -> RunSet {
        self.starter = starter;
        self
    }

    /// Change how many may work at once; runs already working go on.
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit.max(1);
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Start a run on `ws`, or queue it when `limit` are already working. A project that is
    /// working or queued is refused.
    pub fn start(&mut self, hub: &AiHub, mut ws: Workspace, mut opts: RunOptions) -> Result<Started, ToolError> {
        if let Ok(project) = feed::ProjectIdentity::from_workspace(&ws) {
            self.detach_other_project(&ws.root, &project);
        }
        if self.is_running(&ws.root) || self.position(&ws.root).is_some() {
            return Err(
                ToolError::new(ErrorCode::BadInput, "This project is already building.").with_remediation("Add to it while it works, or stop it first.")
            );
        }
        // Queueing saves its waiting status too. Keep the last run's final session and builder
        // choices, rather than writing them back from the caller's older project snapshot.
        ws.reload()?;
        let saved = queued_request::restore(&ws, &mut opts)?;
        let pending = self.runs.get_mut(&ws.root).is_some_and(|tracked| {
            // The worker can finish between UI polls. Keep its unread completion before a
            // direct retry is allowed to replace the receiver that owns those events.
            tracked.pending.extend(tracked.run.poll());
            !tracked.pending.is_empty()
        });
        if self.active() < self.limit && self.queue.is_empty() && !pending {
            let root = ws.root.clone();
            let run = TrackedRun::new((self.starter)(hub, ws.clone(), opts)?)?;
            queued_request::finished(&ws, saved.as_ref());
            self.runs.insert(root, run);
            return Ok(Started::Running);
        }
        // Fail now, not when its turn comes, when no coding agent is set up.
        let cfg = hub.config();
        builder_backend(&cfg, opts.backend.as_deref().or_else(|| super::project_builder(&cfg, &ws)))?;
        let project = feed::ProjectIdentity::from_workspace(&ws)?;
        queued_request::save(&ws, &opts)?;
        ws.set_status(Status::Stopped, "Waiting for a build slot.")?;
        self.queue.push_back((ws, opts, project));
        Ok(Started::Queued { position: self.queue.len() })
    }

    /// Every run's new events, by project folder, then start queued runs in the places that came
    /// free. A queued run that can't start gives a `Finished { ok: false }` with the reason.
    pub fn poll(&mut self, hub: &AiHub) -> Vec<(PathBuf, RunEvent)> {
        self.poll_using(hub, |root, event, _, _, _| (root, event))
    }

    /// Attach one display per polled run before a queued run can replace its owner. A failed
    /// start has no snapshot, even when the map still retains an older run for the same project.
    pub fn poll_with_snapshots(&mut self, hub: &AiHub) -> Vec<RunUpdate> {
        let queued: HashMap<_, _> = self
            .queue
            .iter()
            .filter_map(|(ws, opts, project)| {
                opts.shown
                    .as_ref()
                    .map(|shown| (ws.root.clone(), (project.clone(), feed::FeedItem::User(feed::UserItem::new(shown.clone(), opts.shown_pictures.clone())))))
            })
            .collect();
        self.poll_using(hub, |root, event, run, project, last| {
            let mut display_feed = if last { run.map(RebuildRun::display_feed) } else { None };
            if let Some(display) = &mut display_feed
                && let Some(owner) = project
                && let Some((waiting_owner, message)) = queued.get(&root)
                && owner.is_same_project(waiting_owner)
            {
                // This request has not reached a worker's log yet. Keep its visible row while
                // the completed run's snapshot is installed, even when another project starts.
                display.push(message.clone());
            }
            RunUpdate { root, event, project: project.cloned(), display_feed, start_failed: run.is_none() }
        })
    }

    fn poll_using<T>(&mut self, hub: &AiHub, collect: impl FnMut(PathBuf, RunEvent, Option<&RebuildRun>, Option<&feed::ProjectIdentity>, bool) -> T) -> Vec<T> {
        self.poll_using_open(hub, Workspace::open, collect)
    }

    fn poll_using_open<T>(
        &mut self,
        hub: &AiHub,
        mut open: impl FnMut(&Path) -> Result<Workspace, ToolError>,
        mut collect: impl FnMut(PathBuf, RunEvent, Option<&RebuildRun>, Option<&feed::ProjectIdentity>, bool) -> T,
    ) -> Vec<T> {
        let mut out = Vec::new();
        self.runs.retain(|root, tracked| {
            tracked.pending.extend(tracked.run.poll());
            if tracked.pending.is_empty() {
                return true;
            }
            if let Err(error) = tracked.project.check_current(root) {
                // A replacement is certain only for FileChanged. A temporarily unreadable
                // manifest leaves the original events pending for the next successful poll.
                return error.code != ErrorCode::FileChanged;
            }
            let events = std::mem::take(&mut tracked.pending);
            let last = events.len().saturating_sub(1);
            for (index, event) in events.into_iter().enumerate() {
                out.push(collect(root.clone(), event, Some(&tracked.run), Some(&tracked.project), index == last));
            }
            true
        });
        while self.active() < self.limit {
            // Keep retries behind their own pending events without blocking other projects.
            // Removing only the first eligible request preserves the remaining queue's order.
            let next = self.queue.iter().position(|(ws, _, _)| self.runs.get(&ws.root).is_none_or(|tracked| tracked.pending.is_empty()));
            let Some((ws, opts, project)) = next.and_then(|index| self.queue.remove(index)) else { break };
            // The project as it is now (its Builder may have been changed while it waited), the copy
            // `RebuildRun::start` reads too.
            let current = project.check_current(&ws.root);
            let ws = open(&ws.root).unwrap_or(ws);
            let root = ws.root.clone();
            let mut keep = ws.clone();
            let saved = queued_request::QueuedRequest::from_options(&opts);
            let started = current
                .and_then(|()| {
                    let opened = feed::ProjectIdentity::from_workspace(&ws)?;
                    if !project.is_same_project(&opened) {
                        return Err(ToolError::new(ErrorCode::FileChanged, "The project changed while its request was waiting.")
                            .with_remediation("Open the original project to continue its saved request."));
                    }
                    queued_request::validate(&ws)
                })
                .and_then(|()| (self.starter)(hub, ws, opts))
                .and_then(TrackedRun::new);
            match started {
                Ok(run) => {
                    queued_request::finished(&keep, saved.as_ref());
                    self.runs.insert(root, run);
                }
                Err(e) => {
                    let message = e.for_people();
                    if project.check_current(&root).is_ok() {
                        let _ = keep.set_status(Status::NeedsYou, message.clone());
                    }
                    out.push(collect(root, RunEvent::Finished { ok: false, message, paused: false }, None, Some(&project), true));
                }
            }
        }
        out
    }

    /// The project's run, working or finished.
    pub fn get(&self, root: &Path) -> Option<&RebuildRun> {
        self.runs.get(root).map(|tracked| &tracked.run)
    }

    pub fn get_mut(&mut self, root: &Path) -> Option<&mut RebuildRun> {
        self.runs.get_mut(root).map(|tracked| &mut tracked.run)
    }

    /// Completed runs can still own events waiting for a readable project identity. Collect
    /// unread events too, so the UI keeps scheduling polls even after the worker stops.
    pub fn has_pending_events(&mut self) -> bool {
        self.runs.values_mut().any(|tracked| {
            tracked.pending.extend(tracked.run.poll());
            !tracked.pending.is_empty()
        })
    }

    /// Detach a run or queued request belonging to an earlier project at this path. Reuse
    /// the caller's confirmed identity without another read of a potentially changing path.
    pub fn detach_other_project(&mut self, root: &Path, project: &feed::ProjectIdentity) -> bool {
        let changed = self.runs.get(root).is_some_and(|tracked| !tracked.project.is_same_project(project));
        if changed {
            self.runs.remove(root);
        }
        let queued = self.queue.len();
        self.queue.retain(|(waiting, _, owner)| waiting.root.as_path() != root || owner.is_same_project(project));
        changed || self.queue.len() != queued
    }

    /// The project's run is working.
    pub fn is_running(&self, root: &Path) -> bool {
        self.get(root).is_some_and(RebuildRun::is_running)
    }

    /// Its place in the queue (1 starts next).
    pub fn position(&self, root: &Path) -> Option<usize> {
        self.queue.iter().position(|(ws, _, _)| ws.root == root).map(|i| i + 1)
    }

    /// How many are waiting.
    pub fn waiting(&self) -> usize {
        self.queue.len()
    }

    /// Working or waiting.
    pub fn is_busy(&self, root: &Path) -> bool {
        self.is_running(root) || self.position(root).is_some()
    }

    /// How many are working.
    pub fn active(&self) -> usize {
        self.runs.values().filter(|tracked| tracked.run.is_running()).count()
    }

    /// Any project is working or waiting.
    pub fn any_busy(&self) -> bool {
        self.active() > 0 || !self.queue.is_empty()
    }

    /// The projects working or waiting.
    pub fn busy(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.runs.iter().filter(|(_, tracked)| tracked.run.is_running()).map(|(p, _)| p.clone()).collect();
        v.extend(self.queue.iter().map(|(ws, _, _)| ws.root.clone()));
        v
    }

    /// Stop the project's run, or take it out of the queue.
    pub fn stop(&mut self, root: &Path) -> Stopped {
        if let Some(i) = self.queue.iter().position(|(ws, _, _)| ws.root == root) {
            self.queue.remove(i);
            return Stopped::Unqueued;
        }
        match self.get(root).filter(|r| r.is_running()) {
            Some(r) => {
                r.stop();
                Stopped::Stopping
            }
            None => Stopped::Nothing,
        }
    }

    /// Empty the queue and stop every run (closing the app).
    pub fn stop_all(&mut self) {
        self.queue.clear();
        for tracked in self.runs.values() {
            tracked.run.stop();
        }
    }
}

/// Poll `set` every `every` on a background thread while runs wait, so queued runs start as
/// soon as a place comes free: for holders that poll only when called (the rebuild tools, which
/// an MCP client may not call again for a while). `settings_path` is the session's config file,
/// read when the session is busy. One thread per set at a time; it ends when the queue is empty
/// or the engine is gone, and never waits on a busy session.
pub fn keep_queue_moving(set: &Arc<Mutex<RunSet>>, engine: &Arc<Engine>, settings_path: PathBuf, every: Duration) {
    {
        let mut s = set.lock().unwrap_or_else(PoisonError::into_inner);
        if s.watched || s.queue.is_empty() {
            return;
        }
        s.watched = true;
    }
    let (set_ref, engine_ref) = (Arc::downgrade(set), Arc::downgrade(engine));
    let spawned = std::thread::Builder::new().name("rebuild-queue".into()).spawn(move || {
        loop {
            std::thread::sleep(every);
            let Some(set) = set_ref.upgrade() else { return };
            let mut s = set.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(engine) = engine_ref.upgrade() else {
                s.watched = false;
                return;
            };
            if let Some(session) = engine.try_session() {
                s.set_limit(session.settings.build.parallel);
            }
            // Each run keeps its own project's feed, so the events aren't needed here.
            let hub = AiHub::with_stores(engine, settings_path.clone(), crate::KeyStore::system());
            let _ = s.poll(&hub);
            if s.queue.is_empty() {
                s.watched = false;
                return;
            }
        }
    });
    if spawned.is_err() {
        set.lock().unwrap_or_else(PoisonError::into_inner).watched = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rebuild::StandIns;

    /// A hub whose Builder is an agent app, so queued runs pass the up-front check.
    fn hub() -> AiHub {
        let mut s = craft_anything_engine::testing::session();
        s.settings.ai = toml::from_str("[[backends]]\nname = \"Agent\"\nkind = \"agent_cli\"\nmodel = \"claude-code\"\n").unwrap();
        assert!(builder_backend(&crate::AiConfig::from_settings(&s.settings), None).is_ok());
        AiHub::new(Arc::new(Engine::new(s)))
    }

    type SeenRequest = (Option<String>, Option<String>, Vec<PathBuf>);

    fn remembering_requests(seen: Arc<Mutex<Vec<SeenRequest>>>) -> Starter {
        let mut start = StandIns::new().starter();
        Box::new(move |hub, ws, opts| {
            seen.lock().unwrap().push((opts.message.clone(), opts.shown.clone(), opts.shown_pictures.clone()));
            start(hub, ws, opts)
        })
    }

    #[test]
    fn replaced_projects_retire_the_old_run_before_publishing_events() {
        for replace_directory in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let ws = Workspace::create(directory.path(), "Project", "A game", TargetRef::None).unwrap();
            let next = Workspace::create(directory.path(), "Next", "Another game", TargetRef::None).unwrap();
            let hub = hub();
            let fake = StandIns::new();
            let mut set = RunSet::new(1).with_starter(fake.starter());
            set.start(&hub, ws.clone(), RunOptions::default()).unwrap();
            set.start(&hub, next.clone(), RunOptions::default()).unwrap();
            fake.send(&ws.root, RunEvent::Agent(AgentEvent::TextDelta { text: "The old project's conversation".into() }));
            let manifest = ws.root.join("craft.json");
            let bytes = std::fs::read(&manifest).unwrap();
            if replace_directory {
                move_project_with_open_run(&ws, &directory.path().join("retired"));
                let replacement = Workspace::create(directory.path(), "Project", "A replacement", TargetRef::None).unwrap();
                assert_eq!(replacement.root, ws.root);
                // Even a restored copy with the same generation and timestamp is a new directory.
                std::fs::write(&manifest, &bytes).unwrap();
            } else {
                let mut replacement: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                replacement["generation"] = serde_json::json!("ffffffffffffffffffffffffffffffff");
                std::fs::write(&manifest, serde_json::to_vec(&replacement).unwrap()).unwrap();
            }

            let updates = set.poll_with_snapshots(&hub);
            assert!(updates.iter().all(|update| update.root != ws.root), "old events must not reach the replacement project");
            assert!(set.get(&ws.root).is_none(), "the old run is detached from its reused path");
            assert!(set.is_running(&next.root), "retirement releases the old run's slot");
        }
    }

    fn move_project_with_open_run(ws: &Workspace, destination: &Path) {
        // Windows can move these opened files, but not their ancestor directory while they
        // remain inside it. Keep the live writer and its identity while relocating the files.
        for (name, retained) in [(feed::FEED_FILE, "retired-feed.json"), (craft_anything_engine::workspace::ACTIVITY_LOG, "retired-activity.log")] {
            let source = ws.root.join(name);
            if source.exists() {
                std::fs::rename(source, destination.parent().unwrap().join(retained)).unwrap();
            }
        }
        std::fs::rename(&ws.root, destination).unwrap();
    }

    #[test]
    fn an_unreadable_manifest_keeps_finished_events_until_identity_can_be_checked() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Project", "A game", TargetRef::None).unwrap();
        let hub = hub();
        let fake = StandIns::new();
        let mut set = RunSet::new(1).with_starter(fake.starter());
        set.start(&hub, ws.clone(), RunOptions::default()).unwrap();
        fake.finish(&ws.root);
        let manifest = ws.root.join("craft.json");
        let bytes = std::fs::read(&manifest).unwrap();
        std::fs::remove_file(&manifest).unwrap();

        assert!(!set.any_busy());
        assert!(set.has_pending_events(), "an unread completion still needs another scheduled poll");
        assert!(set.poll_with_snapshots(&hub).is_empty());
        assert!(set.has_pending_events(), "a temporarily unreadable manifest must not stop polling");
        assert!(set.get(&ws.root).is_some(), "an unavailable manifest is not proof of replacement");
        std::fs::write(&manifest, bytes).unwrap();
        let updates = set.poll_with_snapshots(&hub);
        assert_eq!(updates.iter().filter(|update| matches!(&update.event, RunEvent::Finished { .. })).count(), 1);
        assert!(set.poll_with_snapshots(&hub).is_empty(), "the deferred Finished event is delivered once");
        assert!(!set.has_pending_events(), "delivered completions need no further repaint");
    }

    #[test]
    fn a_direct_retry_preserves_an_unread_completion_but_does_not_wait_for_a_delivered_one() {
        for delivered in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let ws = Workspace::create(directory.path(), "Project", "A game", TargetRef::None).unwrap();
            let hub = hub();
            let fake = StandIns::new();
            let mut set = RunSet::new(1).with_starter(fake.starter());
            set.start(&hub, ws.clone(), RunOptions::default()).unwrap();
            fake.finish(&ws.root);
            if delivered {
                assert_eq!(set.poll(&hub).iter().filter(|(_, event)| matches!(event, RunEvent::Finished { .. })).count(), 1);
            }

            let started = set.start(&hub, ws.clone(), RunOptions::default()).unwrap();

            if delivered {
                assert_eq!(started, Started::Running);
                assert!(set.poll(&hub).is_empty());
            } else {
                assert_eq!(started, Started::Queued { position: 1 });
                assert_eq!(fake.started().as_slice(), std::slice::from_ref(&ws.root));
                let updates = set.poll_with_snapshots(&hub);
                assert_eq!(updates.iter().filter(|update| matches!(&update.event, RunEvent::Finished { .. })).count(), 1);
                assert!(updates.iter().all(|update| update.display_feed.is_some()));
                assert!(set.poll(&hub).is_empty(), "the original completion is delivered exactly once");
            }
            assert!(set.is_running(&ws.root));
            assert_eq!(fake.started(), [ws.root.clone(), ws.root]);
        }
    }

    #[test]
    fn a_same_path_followup_waits_until_deferred_finished_events_are_delivered() {
        for queued_before_error in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let ws = Workspace::create(directory.path(), "Project", "A game", TargetRef::None).unwrap();
            let other = Workspace::create(directory.path(), "Other", "Another game", TargetRef::None).unwrap();
            let hub = hub();
            let fake = StandIns::new();
            let mut set = RunSet::new(1).with_starter(fake.starter());
            set.start(&hub, ws.clone(), RunOptions::default()).unwrap();
            if queued_before_error {
                set.start(&hub, other.clone(), RunOptions::default()).unwrap();
            }
            fake.finish(&ws.root);
            let request = RunOptions { message: Some("Follow up".into()), shown: Some("Follow up".into()), ..Default::default() };
            if queued_before_error {
                set.start(&hub, ws.clone(), request.clone()).unwrap();
            }
            let manifest = ws.root.join("craft.json");
            let bytes = std::fs::read(&manifest).unwrap();
            std::fs::remove_file(&manifest).unwrap();
            assert!(set.poll_with_snapshots(&hub).is_empty());
            if queued_before_error {
                fake.finish(&other.root);
                set.poll_with_snapshots(&hub);
                assert_eq!(fake.started(), [ws.root.clone(), other.root.clone()]);
                assert!(set.position(&ws.root).is_some(), "the waiting run cannot overwrite deferred Finished events");
            }
            std::fs::write(&manifest, bytes).unwrap();
            if !queued_before_error {
                assert!(matches!(set.start(&hub, ws.clone(), request).unwrap(), Started::Queued { .. }));
                assert_eq!(fake.started().as_slice(), std::slice::from_ref(&ws.root), "a direct retry must also wait for the old batch");
            }

            let updates = set.poll_with_snapshots(&hub);

            assert_eq!(updates.iter().filter(|update| update.root == ws.root && matches!(&update.event, RunEvent::Finished { .. })).count(), 1);
            assert!(set.is_running(&ws.root));
            assert!(set.position(&ws.root).is_none());
            assert!(set.poll_with_snapshots(&hub).is_empty(), "the old completion is delivered only once");
        }
    }

    #[test]
    fn a_replaced_queued_project_is_not_started_or_changed_by_its_old_request() {
        let directory = tempfile::tempdir().unwrap();
        let blocker = Workspace::create(directory.path(), "Blocker", "A game", TargetRef::None).unwrap();
        let waiting = Workspace::create(directory.path(), "Waiting", "A game", TargetRef::None).unwrap();
        let hub = hub();
        let fake = StandIns::new();
        let mut set = RunSet::new(1).with_starter(fake.starter());
        set.start(&hub, blocker.clone(), RunOptions::default()).unwrap();
        set.start(&hub, waiting.clone(), RunOptions { message: Some("The old request".into()), ..Default::default() }).unwrap();
        std::fs::rename(&waiting.root, directory.path().join("retired")).unwrap();
        let replacement = Workspace::create(directory.path(), "Waiting", "A replacement", TargetRef::None).unwrap();
        let manifest = std::fs::read(replacement.root.join("craft.json")).unwrap();

        fake.finish(&blocker.root);
        set.poll_with_snapshots(&hub);

        assert_eq!(fake.started(), [blocker.root]);
        assert!(set.get(&replacement.root).is_none());
        assert_eq!(std::fs::read(replacement.root.join("craft.json")).unwrap(), manifest);
    }

    #[test]
    fn an_unreadable_completion_does_not_block_other_queued_projects() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Project", "A game", TargetRef::None).unwrap();
        let first = Workspace::create(directory.path(), "First", "Another game", TargetRef::None).unwrap();
        let next = Workspace::create(directory.path(), "Next", "A third game", TargetRef::None).unwrap();
        let hub = hub();
        let fake = StandIns::new();
        let mut set = RunSet::new(1).with_starter(fake.starter());
        set.start(&hub, ws.clone(), RunOptions::default()).unwrap();
        set.start(&hub, first.clone(), RunOptions::default()).unwrap();
        fake.finish(&ws.root);
        set.start(&hub, ws.clone(), RunOptions { message: Some("Follow up".into()), ..Default::default() }).unwrap();
        set.start(&hub, next.clone(), RunOptions::default()).unwrap();
        let manifest = ws.root.join("craft.json");
        let bytes = std::fs::read(&manifest).unwrap();
        std::fs::remove_file(&manifest).unwrap();
        assert!(set.poll_with_snapshots(&hub).is_empty());
        assert!(set.is_running(&first.root));

        fake.finish(&first.root);
        let updates = set.poll_with_snapshots(&hub);

        assert!(updates.iter().all(|update| update.root != ws.root));
        assert!(set.is_running(&next.root), "an unreadable completion cannot hold another project's free slot");
        assert_eq!(set.position(&ws.root), Some(1), "the blocked request keeps its place");
        assert_eq!(fake.started(), [ws.root.clone(), first.root, next.root.clone()]);
        std::fs::write(&manifest, bytes).unwrap();
        fake.finish(&next.root);
        let updates = set.poll_with_snapshots(&hub);
        assert_eq!(updates.iter().filter(|update| update.root == ws.root && matches!(&update.event, RunEvent::Finished { .. })).count(), 1);
        assert!(set.is_running(&ws.root), "the original request starts after its pending completion is delivered");
    }

    #[test]
    fn a_project_replaced_while_reopening_a_queued_request_is_not_started() {
        let directory = tempfile::tempdir().unwrap();
        let blocker = Workspace::create(directory.path(), "Blocker", "A game", TargetRef::None).unwrap();
        let waiting = Workspace::create(directory.path(), "Waiting", "Original", TargetRef::None).unwrap();
        let retired = directory.path().join("retired");
        let hub = hub();
        let fake = StandIns::new();
        let mut set = RunSet::new(1).with_starter(fake.starter());
        set.start(&hub, blocker.clone(), RunOptions::default()).unwrap();
        set.start(&hub, waiting.clone(), RunOptions { message: Some("Keep the original request".into()), ..Default::default() }).unwrap();
        let request = std::fs::read(waiting.inbox_dir().join("queued-request.json")).unwrap();
        fake.finish(&blocker.root);
        let mut replacement_manifest = Vec::new();

        let events = set.poll_using_open(
            &hub,
            |root| {
                assert_eq!(root, waiting.root);
                std::fs::rename(root, &retired).unwrap();
                let replacement = Workspace::create(directory.path(), "Waiting", "Replacement", TargetRef::None).unwrap();
                replacement_manifest = std::fs::read(replacement.root.join("craft.json")).unwrap();
                Workspace::open(root)
            },
            |root, event, _, _, _| (root, event),
        );

        assert_eq!(fake.started(), [blocker.root]);
        assert!(events.iter().any(|(root, event)| root == &waiting.root && matches!(event, RunEvent::Finished { ok: false, .. })));
        assert_eq!(std::fs::read(waiting.root.join("craft.json")).unwrap(), replacement_manifest);
        assert_eq!(std::fs::read(retired.join("inbox/queued-request.json")).unwrap(), request);
        assert!(!waiting.inbox_dir().join("queued-request.json").exists());
    }

    #[test]
    fn a_queued_request_survives_shutdown_and_an_abruptly_lost_queue() {
        for stopped in [false, true] {
            let d = tempfile::tempdir().unwrap();
            let a = Workspace::create(d.path(), "A", "A game", TargetRef::None).unwrap();
            let b = Workspace::create(d.path(), "B", "A game", TargetRef::None).unwrap();
            let picture = b.root.join("pictures/reference.png");
            let opts = RunOptions {
                message: Some("Keep the exact request\nwith its attachment path".into()),
                shown: Some("Keep the exact request".into()),
                shown_pictures: vec![picture.clone()],
                ..RunOptions::default()
            };
            let hub = hub();
            let mut set = RunSet::new(1).with_starter(StandIns::new().starter());
            set.start(&hub, a, RunOptions::default()).unwrap();
            assert_eq!(set.start(&hub, b.clone(), opts).unwrap(), Started::Queued { position: 1 });
            assert_eq!(Workspace::open(&b.root).unwrap().manifest.status, Status::Stopped, "a reopened project offers Continue");
            if stopped {
                set.stop_all();
            }
            drop(set);

            let seen = Arc::new(Mutex::new(Vec::new()));
            let mut reopened = RunSet::new(1).with_starter(remembering_requests(seen.clone()));
            assert_eq!(reopened.start(&hub, Workspace::open(&b.root).unwrap(), RunOptions::default()).unwrap(), Started::Running);
            assert_eq!(
                *seen.lock().unwrap(),
                [(Some("Keep the exact request\nwith its attachment path".into()), Some("Keep the exact request".into()), vec![picture])],
                "the next run receives the request, its display text and its pictures after the queue is gone"
            );
        }
    }

    #[test]
    fn a_queued_request_is_not_accepted_when_it_cannot_be_saved() {
        let d = tempfile::tempdir().unwrap();
        let a = Workspace::create(d.path(), "A", "A game", TargetRef::None).unwrap();
        let b = Workspace::create(d.path(), "B", "A game", TargetRef::None).unwrap();
        // Preserve the coordination directory created with the workspace, then
        // install the same non-directory storage blocker this case exercises.
        std::fs::rename(b.inbox_dir(), b.root.join("saved-inbox")).unwrap();
        std::fs::write(b.inbox_dir(), b"not a folder").unwrap();
        let hub = hub();
        let mut set = RunSet::new(1).with_starter(StandIns::new().starter());
        set.start(&hub, a, RunOptions::default()).unwrap();
        let result = set.start(&hub, b.clone(), RunOptions { message: Some("Keep this request".into()), ..RunOptions::default() });
        assert!(result.is_err(), "an accepted request must outlive the in-memory queue");
        assert_eq!(set.position(&b.root), None, "a failed save must not enqueue the request");
    }

    #[test]
    fn a_queued_request_survives_a_failure_to_start() {
        let d = tempfile::tempdir().unwrap();
        let a = Workspace::create(d.path(), "A", "A game", TargetRef::None).unwrap();
        let b = Workspace::create(d.path(), "B", "A game", TargetRef::None).unwrap();
        let hub = hub();
        let fake = StandIns::new().failing("B");
        let mut set = RunSet::new(1).with_starter(fake.starter());
        set.start(&hub, a.clone(), RunOptions::default()).unwrap();
        set.start(&hub, b.clone(), RunOptions { message: Some("Retry this request".into()), ..RunOptions::default() }).unwrap();
        fake.finish(&a.root);
        assert!(set.poll(&hub).iter().any(|(root, e)| root == &b.root && matches!(e, RunEvent::Finished { ok: false, .. })));
        drop(set);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut reopened = RunSet::new(1).with_starter(remembering_requests(seen.clone()));
        reopened.start(&hub, b, RunOptions::default()).unwrap();
        assert_eq!(seen.lock().unwrap()[0].0.as_deref(), Some("Retry this request"));
    }

    #[test]
    fn a_changed_unreadable_request_stops_its_queued_start_and_leaves_the_next_project_free() {
        for (case, contents) in
            [("malformed", Some(b"{broken".as_slice())), ("newer", Some(br#"{"version":4,"message":"A later request"}"#.as_slice())), ("unreadable", None)]
        {
            let d = tempfile::tempdir().unwrap();
            let blocker = Workspace::create(d.path(), "Blocker", "A game", TargetRef::None).unwrap();
            let queued = Workspace::create(d.path(), "Queued", "A game", TargetRef::None).unwrap();
            let next = Workspace::create(d.path(), "Next", "A game", TargetRef::None).unwrap();
            let hub = hub();
            let fake = StandIns::new();
            let mut set = RunSet::new(1).with_starter(fake.starter());
            set.start(&hub, blocker.clone(), RunOptions::default()).unwrap();
            let opts = RunOptions { message: Some("The queued request".into()), ..Default::default() };
            assert_eq!(set.start(&hub, queued.clone(), opts).unwrap(), Started::Queued { position: 1 });
            assert_eq!(set.start(&hub, next.clone(), RunOptions::default()).unwrap(), Started::Queued { position: 2 });
            let record = queued.inbox_dir().join("queued-request.json");
            if let Some(contents) = contents {
                std::fs::write(&record, contents).unwrap();
            } else {
                std::fs::remove_file(&record).unwrap();
                std::fs::create_dir(&record).unwrap();
                std::fs::write(record.join("keep"), b"leave unchanged").unwrap();
            }
            fake.finish(&blocker.root);
            let events = set.poll(&hub);
            assert!(
                events.iter().any(|(root, event)| {
                    root == &queued.root && matches!(event, RunEvent::Finished { ok: false, message, paused: false } if message.contains("queued request"))
                }),
                "{case}: the existing failed-start path must report the record error"
            );
            assert_eq!(fake.started(), [blocker.root.clone(), next.root.clone()], "{case}: the unreadable request must not reach the starter");
            assert!(set.get(&queued.root).is_none() && set.position(&queued.root).is_none());
            assert_eq!(Workspace::open(&queued.root).unwrap().manifest.status, Status::NeedsYou);
            if let Some(contents) = contents {
                assert_eq!(std::fs::read(&record).unwrap(), contents, "{case}: the saved bytes remain unchanged");
            } else {
                assert_eq!(std::fs::read(record.join("keep")).unwrap(), b"leave unchanged");
            }
        }
    }

    #[test]
    fn a_successfully_started_request_is_not_replayed() {
        let d = tempfile::tempdir().unwrap();
        let a = Workspace::create(d.path(), "A", "A game", TargetRef::None).unwrap();
        let b = Workspace::create(d.path(), "B", "A game", TargetRef::None).unwrap();
        let hub = hub();
        let fake = StandIns::new();
        let mut start = fake.starter();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let requests = seen.clone();
        let mut set = RunSet::new(1).with_starter(Box::new(move |hub, ws, opts| {
            requests.lock().unwrap().push(opts.message.clone());
            start(hub, ws, opts)
        }));
        set.start(&hub, a.clone(), RunOptions::default()).unwrap();
        set.start(&hub, b.clone(), RunOptions { message: Some("Send once".into()), ..RunOptions::default() }).unwrap();
        fake.finish(&a.root);
        set.poll(&hub);
        assert_eq!(*seen.lock().unwrap(), [None, Some("Send once".into())]);
        drop(set);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut reopened = RunSet::new(1).with_starter(remembering_requests(seen.clone()));
        reopened.start(&hub, b, RunOptions::default()).unwrap();
        assert_eq!(seen.lock().unwrap()[0].0, None);
    }

    #[test]
    fn the_default_starter_keeps_a_request_saved_after_queueing() {
        let d = tempfile::tempdir().unwrap();
        let blocker = Workspace::create(d.path(), "Blocker", "A game", TargetRef::None).unwrap();
        let ws = Workspace::create(d.path(), "Queued", "A game", TargetRef::None).unwrap();
        let hub = AiHub::with_stores(hub().engine.clone(), d.path().join("settings.toml"), crate::KeyStore::file_only(d.path().join("keys.toml")));
        let fake = StandIns::new();
        let mut set = RunSet::new(1);
        // Only the occupied slot is a stand-in. Poll must use the real default starter for A.
        let blocker_run = fake.starter()(&hub, blocker.clone(), RunOptions::default()).unwrap();
        set.runs.insert(blocker.root.clone(), TrackedRun::new(blocker_run).unwrap());
        let opts = RunOptions { message: Some("Queued request A".into()), shown: Some("Queued request A".into()), ..Default::default() };
        assert_eq!(set.start(&hub, ws.clone(), opts).unwrap(), Started::Queued { position: 1 });
        queued_request::save(&ws, &RunOptions { message: Some("Later request B".into()), ..Default::default() }).unwrap();
        fake.finish(&blocker.root);
        set.poll(&hub);
        let run = set.get_mut(&ws.root).expect("the queued run started");
        assert!(run.feed_now().iter().any(|item| matches!(item, feed::FeedItem::User(user) if user.text() == "Queued request A")));
        run.stop();
        let deadline = Instant::now() + Duration::from_secs(30);
        while run.is_running() {
            assert!(Instant::now() < deadline, "the run did not stop");
            let _ = run.rx.recv_timeout(Duration::from_millis(50));
        }
        let mut next = RunOptions::default();
        queued_request::restore(&ws, &mut next).unwrap();
        assert_eq!(next.message.as_deref(), Some("Later request B"), "starting A must not retire B's backup");
    }

    #[test]
    fn queueing_keeps_the_last_runs_saved_session() {
        let d = tempfile::tempdir().unwrap();
        let a = Workspace::create(d.path(), "A", "A game", TargetRef::None).unwrap();
        let b = Workspace::create(d.path(), "B", "A game", TargetRef::None).unwrap();
        let mut current = Workspace::open(&b.root).unwrap();
        current.manifest.agent_session = Some("the-last-runs-session".into());
        current.save().unwrap();
        current.set_person(|m| m.builder = Some("Agent".into())).unwrap();
        let hub = hub();
        let mut set = RunSet::new(1).with_starter(StandIns::new().starter());
        set.start(&hub, a, RunOptions::default()).unwrap();
        assert_eq!(
            set.start(&hub, b.clone(), RunOptions { message: Some("Keep going".into()), ..Default::default() }).unwrap(),
            Started::Queued { position: 1 }
        );
        let saved = Workspace::open(&b.root).unwrap();
        assert_eq!(saved.manifest.agent_session.as_deref(), Some("the-last-runs-session"));
        assert_eq!(saved.manifest.builder.as_deref(), Some("Agent"));
        assert_eq!(saved.manifest.status, Status::Stopped);
    }

    #[test]
    fn pausing_a_queued_request_keeps_it_for_manual_continue() {
        let d = tempfile::tempdir().unwrap();
        let a = Workspace::create(d.path(), "A", "A game", TargetRef::None).unwrap();
        let b = Workspace::create(d.path(), "B", "A game", TargetRef::None).unwrap();
        let hub = hub();
        let mut set = RunSet::new(1).with_starter(StandIns::new().starter());
        set.start(&hub, a, RunOptions::default()).unwrap();
        set.start(&hub, b.clone(), RunOptions { message: Some("Resume my change".into()), ..Default::default() }).unwrap();
        assert_eq!(set.stop(&b.root), Stopped::Unqueued);
        assert_eq!(set.position(&b.root), None);
        // Continuing while the other build still works must keep the restored words when it is
        // queued a second time, and survive another restart before a slot comes free.
        assert_eq!(set.start(&hub, b.clone(), RunOptions::default()).unwrap(), Started::Queued { position: 1 });
        drop(set);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut reopened = RunSet::new(1).with_starter(remembering_requests(seen.clone()));
        reopened.start(&hub, b, RunOptions::default()).unwrap();
        assert_eq!(seen.lock().unwrap()[0].0.as_deref(), Some("Resume my change"));
    }

    #[test]
    fn runs_past_the_limit_wait_and_start_in_order() {
        let d = tempfile::tempdir().unwrap();
        let ws = |n: &str| Workspace::create(d.path(), n, "A game", TargetRef::None).unwrap();
        let (a, b, c) = (ws("A"), ws("B"), ws("C"));
        let fake = StandIns::new();
        let hub = hub();
        let mut set = RunSet::new(2).with_starter(fake.starter());
        assert_eq!(set.start(&hub, a.clone(), RunOptions::default()).unwrap(), Started::Running);
        assert_eq!(set.start(&hub, b.clone(), RunOptions::default()).unwrap(), Started::Running);
        assert_eq!(set.start(&hub, c.clone(), RunOptions::default()).unwrap(), Started::Queued { position: 1 });
        assert!(set.start(&hub, c.clone(), RunOptions::default()).is_err(), "a queued project isn't queued twice");
        assert!(set.start(&hub, a.clone(), RunOptions::default()).is_err(), "nor a working one started twice");
        assert_eq!((set.active(), set.position(&c.root), set.busy().len()), (2, Some(1), 3));

        // A finishes: its event is reported under its folder, and C takes its place.
        fake.finish(&a.root);
        let events = set.poll(&hub);
        assert_eq!(events, [(a.root.clone(), RunEvent::Finished { ok: true, message: "Done.".into(), paused: false })]);
        assert_eq!(fake.started(), [a.root.clone(), b.root.clone(), c.root.clone()]);
        assert!(set.is_running(&c.root) && !set.is_running(&a.root) && set.get(&a.root).is_some(), "a finished run is kept");
        // A can build again, and waits behind the two working.
        assert_eq!(set.start(&hub, a.clone(), RunOptions::default()).unwrap(), Started::Queued { position: 1 });
        assert_eq!(set.stop(&a.root), Stopped::Unqueued);
        assert_eq!(set.stop(&b.root), Stopped::Stopping);
        assert_eq!(set.stop(&a.root), Stopped::Nothing);
        set.stop_all();
        assert!(set.position(&a.root).is_none());
    }

    /// #943 review: a place that came free before the queue moved goes to the run waiting first.
    #[test]
    fn a_new_run_waits_behind_one_already_queued() {
        let d = tempfile::tempdir().unwrap();
        let ws = |n: &str| Workspace::create(d.path(), n, "A game", TargetRef::None).unwrap();
        let (a, c, e) = (ws("A"), ws("C"), ws("E"));
        let fake = StandIns::new();
        let hub = hub();
        let mut set = RunSet::new(1).with_starter(fake.starter());
        set.start(&hub, a.clone(), RunOptions::default()).unwrap();
        assert_eq!(set.start(&hub, c.clone(), RunOptions::default()).unwrap(), Started::Queued { position: 1 });
        fake.finish(&a.root);
        assert_eq!(set.active(), 0, "A's place is free before anyone polls");
        assert_eq!(set.start(&hub, e.clone(), RunOptions::default()).unwrap(), Started::Queued { position: 2 });
        set.poll(&hub);
        assert_eq!(fake.started(), [a.root.clone(), c.root.clone()], "C goes first");
        set.stop_all();
    }

    #[test]
    fn a_queued_run_that_cannot_start_says_why() {
        let d = tempfile::tempdir().unwrap();
        let a = Workspace::create(d.path(), "A", "A game", TargetRef::None).unwrap();
        let b = Workspace::create(d.path(), "B", "A game", TargetRef::None).unwrap();
        let fake = StandIns::new().failing("B");
        let hub = hub();
        let mut set = RunSet::new(1).with_starter(fake.starter());
        set.start(&hub, a.clone(), RunOptions::default()).unwrap();
        assert_eq!(set.start(&hub, b.clone(), RunOptions::default()).unwrap(), Started::Queued { position: 1 });
        fake.finish(&a.root);
        let events = set.poll(&hub);
        assert!(events.contains(&(b.root.clone(), RunEvent::Finished { ok: false, message: "No coding agent.".into(), paused: false })), "{events:?}");
        assert!(!set.any_busy());
        assert_eq!(Workspace::open(&b.root).unwrap().manifest.status, Status::NeedsYou);
    }

    #[test]
    fn a_queued_run_starts_without_anyone_polling() {
        let d = tempfile::tempdir().unwrap();
        let a = Workspace::create(d.path(), "A", "A game", TargetRef::None).unwrap();
        let b = Workspace::create(d.path(), "B", "A game", TargetRef::None).unwrap();
        let fake = StandIns::new();
        let hub = hub();
        let settings_path = hub.engine.session().config_dir().join("config.toml");
        let set = Arc::new(Mutex::new(RunSet::new(1).with_starter(fake.starter())));
        set.lock().unwrap().start(&hub, a.clone(), RunOptions::default()).unwrap();
        // Nothing waits yet, so no thread is started.
        keep_queue_moving(&set, &hub.engine, settings_path.clone(), Duration::from_millis(5));
        assert!(!set.lock().unwrap().watched);
        set.lock().unwrap().start(&hub, b.clone(), RunOptions::default()).unwrap();
        keep_queue_moving(&set, &hub.engine, settings_path.clone(), Duration::from_millis(5));
        keep_queue_moving(&set, &hub.engine, settings_path, Duration::from_millis(5));
        assert!(set.lock().unwrap().watched);
        fake.finish(&a.root);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while set.lock().unwrap().watched && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(fake.started(), [a.root.clone(), b.root.clone()], "one thread started B once");
        let s = set.lock().unwrap();
        assert!(s.is_running(&b.root) && s.waiting() == 0 && !s.watched);
    }

    #[test]
    fn nothing_is_queued_without_a_coding_agent() {
        let d = tempfile::tempdir().unwrap();
        let a = Workspace::create(d.path(), "A", "A game", TargetRef::None).unwrap();
        let b = Workspace::create(d.path(), "B", "A game", TargetRef::None).unwrap();
        let hub = AiHub::new(Arc::new(Engine::new(craft_anything_engine::testing::session())));
        let mut set = RunSet::new(1).with_starter(StandIns::new().starter());
        set.start(&hub, a, RunOptions::default()).unwrap();
        let root = b.root.clone();
        assert!(set.start(&hub, b, RunOptions::default()).is_err());
        assert_eq!(set.position(&root), None);
    }
}
