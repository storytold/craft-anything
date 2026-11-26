//! Several rebuilds at once. A [`RunSet`] keeps every project's run by folder: up to its limit
//! work at the same time, the rest wait in order and start as others finish. The app and the
//! rebuild tools both hold one; each run keeps its own project's feed, so nothing here depends on
//! which project is on screen.

use super::*;
use std::collections::{HashMap, VecDeque};

/// How a run is started; [`RebuildRun::start`] unless a fork or a test swaps it.
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
    /// Taken out of the queue before it started.
    Unqueued,
    Nothing,
}

pub struct RunSet {
    limit: usize,
    /// Running and finished runs (a finished one keeps its preview and unsent notes).
    runs: HashMap<PathBuf, RebuildRun>,
    queue: VecDeque<(Workspace, RunOptions)>,
    starter: Starter,
    /// A [`keep_queue_moving`] thread is looking after the queue.
    watched: bool,
}

/// The builder this launch will use. One call per attempt: the run that starts receives it.
fn chosen_builder(hub: &AiHub, ws: &Workspace, opts: &RunOptions) -> Result<crate::BackendEntry, ToolError> {
    let cfg = hub.config();
    builder_backend(&cfg, opts.backend.as_deref().or_else(|| super::project_builder(&cfg, ws)))
}

/// `opts` with the builder already chosen, so a later lookup does not pick again.
fn with_builder(opts: RunOptions, name: &str) -> RunOptions {
    RunOptions { backend: Some(name.to_string()), ..opts }
}

impl std::fmt::Debug for RunSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunSet").field("limit", &self.limit).field("running", &self.active()).field("queued", &self.queue.len()).finish()
    }
}

impl RunSet {
    /// At most `limit` runs work at once (at least one).
    pub fn new(limit: usize) -> RunSet {
        RunSet { limit: limit.max(1), runs: HashMap::new(), queue: VecDeque::new(), starter: Box::new(RebuildRun::start), watched: false }
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
    pub fn start(&mut self, hub: &AiHub, ws: Workspace, opts: RunOptions) -> Result<Started, ToolError> {
        if self.is_running(&ws.root) || self.position(&ws.root).is_some() {
            return Err(
                ToolError::new(ErrorCode::BadInput, "This project is already building.").with_remediation("Add to it while it works, or stop it first.")
            );
        }
        // The copy the caller holds may predate a builder saved since. `poll` reloads before it
        // resolves; do the same here. An explicit `opts.backend` still wins over the saved one.
        let ws = Workspace::open(&ws.root).unwrap_or(ws);
        // Resolved once. A later reachability change cannot make the check and startup disagree.
        // No builder at all still reaches the starter when a place is free: that is where a real
        // run reports the error, and a stand-in run does not need one.
        let resolved = chosen_builder(hub, &ws, &opts);
        if self.active() < self.limit && self.queue.is_empty() {
            let root = ws.root.clone();
            let run_opts = match &resolved {
                Ok(entry) => with_builder(opts, &entry.config.name),
                Err(_) => opts,
            };
            let run = (self.starter)(hub, ws, run_opts)?;
            self.runs.insert(root, run);
            return Ok(Started::Running);
        }
        // Fail now, not when its turn comes, when no coding agent is set up. The queued run keeps
        // the builder chosen here, so its turn in `poll` (the window calls it every frame) names
        // it and never checks which local server answers.
        let entry = resolved?;
        self.queue.push_back((ws, with_builder(opts, &entry.config.name)));
        Ok(Started::Queued { position: self.queue.len() })
    }

    /// Every run's new events, by project folder, then start queued runs in the places that came
    /// free. A queued run that can't start gives a `Finished { ok: false }` with the reason.
    pub fn poll(&mut self, hub: &AiHub) -> Vec<(PathBuf, RunEvent)> {
        let mut out: Vec<(PathBuf, RunEvent)> = Vec::new();
        for (root, run) in &mut self.runs {
            out.extend(run.poll().into_iter().map(|e| (root.clone(), e)));
        }
        while self.active() < self.limit {
            let Some((ws, opts)) = self.queue.pop_front() else { break };
            // The project as it is now (its Builder may have been changed while it waited), the copy
            // `RebuildRun::start` reads too.
            let ws = Workspace::open(&ws.root).unwrap_or(ws);
            // A queued run already names its builder (`start` chose it), so this does not probe.
            let entry = match chosen_builder(hub, &ws, &opts) {
                Ok(entry) => entry,
                Err(e) => {
                    let root = ws.root.clone();
                    let message = e.for_people();
                    let mut keep = ws;
                    let _ = keep.set_status(Status::NeedsYou, message.clone());
                    out.push((root, RunEvent::Finished { ok: false, message, paused: false }));
                    continue;
                }
            };
            let pinned = with_builder(opts, &entry.config.name);
            let root = ws.root.clone();
            let mut keep = ws.clone();
            match (self.starter)(hub, ws, pinned) {
                Ok(run) => {
                    self.runs.insert(root, run);
                }
                Err(e) => {
                    let message = e.for_people();
                    let _ = keep.set_status(Status::NeedsYou, message.clone());
                    out.push((root, RunEvent::Finished { ok: false, message, paused: false }));
                }
            }
        }
        out
    }

    /// The project's run, working or finished.
    pub fn get(&self, root: &Path) -> Option<&RebuildRun> {
        self.runs.get(root)
    }

    pub fn get_mut(&mut self, root: &Path) -> Option<&mut RebuildRun> {
        self.runs.get_mut(root)
    }

    /// The project's run is working.
    pub fn is_running(&self, root: &Path) -> bool {
        self.runs.get(root).is_some_and(RebuildRun::is_running)
    }

    /// Its place in the queue (1 starts next).
    pub fn position(&self, root: &Path) -> Option<usize> {
        self.queue.iter().position(|(ws, _)| ws.root == root).map(|i| i + 1)
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
        self.runs.values().filter(|r| r.is_running()).count()
    }

    /// Any project is working or waiting.
    pub fn any_busy(&self) -> bool {
        self.active() > 0 || !self.queue.is_empty()
    }

    /// The projects working or waiting.
    pub fn busy(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.runs.iter().filter(|(_, r)| r.is_running()).map(|(p, _)| p.clone()).collect();
        v.extend(self.queue.iter().map(|(ws, _)| ws.root.clone()));
        v
    }

    /// Stop the project's run, or take it out of the queue.
    pub fn stop(&mut self, root: &Path) -> Stopped {
        if let Some(i) = self.queue.iter().position(|(ws, _)| ws.root == root) {
            self.queue.remove(i);
            return Stopped::Unqueued;
        }
        match self.runs.get(root).filter(|r| r.is_running()) {
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
        for r in self.runs.values() {
            r.stop();
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

    /// An agent app and an API backend, so a saved builder name can be told apart from another.
    fn two_backends() -> AiHub {
        let mut s = craft_anything_engine::testing::session();
        s.settings.ai = toml::from_str(
            "[[backends]]\nname = \"Agent\"\nkind = \"agent_cli\"\nmodel = \"claude-code\"\n[[backends]]\nname = \"Key\"\nkind = \"openrouter\"\nmodel = \"m\"\n",
        )
        .unwrap();
        AiHub::new(Arc::new(Engine::new(s)))
    }

    /// A builder saved after the caller loaded the project is the one an immediate start uses.
    /// An explicit `opts.backend` is not replaced by that save.
    #[test]
    fn an_immediate_start_reads_the_builder_saved_since_the_workspace_was_loaded() {
        let d = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(d.path(), "A", "A game", TargetRef::None).unwrap();
        ws.set_person(|m| m.builder = Some("Agent".into())).unwrap();
        let loaded = Workspace::open(&ws.root).unwrap();
        Workspace::open(&ws.root).unwrap().set_person(|m| m.builder = Some("Key".into())).unwrap();
        assert_eq!(loaded.manifest.builder.as_deref(), Some("Agent"));

        let hub = two_backends();
        let fake = StandIns::new();
        let seen = Arc::new(Mutex::new(None));
        let seen_in = seen.clone();
        let mut inner = fake.starter();
        let starter: Starter = Box::new(move |hub, ws, opts| {
            *seen_in.lock().unwrap() = opts.backend.clone();
            inner(hub, ws, opts)
        });
        let mut set = RunSet::new(1).with_starter(starter);
        assert_eq!(set.start(&hub, loaded, RunOptions::default()).unwrap(), Started::Running);
        assert_eq!(seen.lock().unwrap().as_deref(), Some("Key"));

        let mut other = Workspace::create(d.path(), "B", "A game", TargetRef::None).unwrap();
        other.set_person(|m| m.builder = Some("Key".into())).unwrap();
        let loaded_other = Workspace::open(&other.root).unwrap();
        let mut set = RunSet::new(1).with_starter({
            let seen_in = seen.clone();
            let mut inner = StandIns::new().starter();
            Box::new(move |hub, ws, opts| {
                *seen_in.lock().unwrap() = opts.backend.clone();
                inner(hub, ws, opts)
            })
        });
        let explicit = RunOptions { backend: Some("Agent".into()), ..RunOptions::default() };
        assert_eq!(set.start(&hub, loaded_other, explicit).unwrap(), Started::Running);
        assert_eq!(seen.lock().unwrap().as_deref(), Some("Agent"));
    }

    /// An unnamed local_first choice is made once: the run that starts is given that name, and a
    /// queued run is given the name resolved on its own turn, not looked up again inside startup.
    #[test]
    fn an_automatic_builder_is_resolved_once_for_admission_and_startup() {
        let d = tempfile::tempdir().unwrap();
        let ws = |n: &str| Workspace::create(d.path(), n, "A game", TargetRef::None).unwrap();
        let (a, b) = (ws("A"), ws("B"));
        let mut session = craft_anything_engine::testing::session();
        session.settings.ai = toml::from_str(
            "local_first = true\n[[backends]]\nname = \"Lemonade\"\nkind = \"openai_compatible\"\nmodel = \"qwen\"\nbase_url = \"http://127.0.0.1:9/v1\"\n",
        )
        .unwrap();
        let hub = AiHub::new(Arc::new(Engine::new(session)));
        let fake = StandIns::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_in = seen.clone();
        let mut inner = fake.starter();
        let starter: Starter = Box::new(move |hub, ws, opts| {
            seen_in.lock().unwrap().push(opts.backend.clone());
            inner(hub, ws, opts)
        });
        let mut set = RunSet::new(1).with_starter(starter);
        assert_eq!(set.start(&hub, a.clone(), RunOptions::default()).unwrap(), Started::Running);
        assert_eq!(set.start(&hub, b.clone(), RunOptions::default()).unwrap(), Started::Queued { position: 1 });
        fake.finish(&a.root);
        set.poll(&hub);
        assert!(set.is_running(&b.root));
        assert_eq!(seen.lock().unwrap().as_slice(), [Some("Lemonade".to_string()), Some("Lemonade".to_string())]);
    }

    /// A queued run keeps the builder chosen when it was queued: its turn in `poll` uses that name
    /// and does not choose again (choosing can open connections, and `poll` runs on the window).
    #[test]
    fn a_queued_run_keeps_the_builder_chosen_when_it_was_queued() {
        let d = tempfile::tempdir().unwrap();
        let ws = |n: &str| Workspace::create(d.path(), n, "A game", TargetRef::None).unwrap();
        let (a, b) = (ws("A"), ws("B"));
        let mut session = craft_anything_engine::testing::session();
        session.settings.ai = toml::from_str(
            "local_first = true\n[[backends]]\nname = \"Lemonade\"\nkind = \"openai_compatible\"\nmodel = \"qwen\"\nbase_url = \"http://127.0.0.1:9/v1\"\n",
        )
        .unwrap();
        let hub = AiHub::new(Arc::new(Engine::new(session)));
        let fake = StandIns::new();
        let mut set = RunSet::new(1).with_starter(fake.starter());
        assert_eq!(set.start(&hub, a.clone(), RunOptions::default()).unwrap(), Started::Running);
        assert_eq!(set.start(&hub, b.clone(), RunOptions::default()).unwrap(), Started::Queued { position: 1 });
        let mut cfg = hub.config();
        for entry in &mut cfg.backends {
            entry.config.enabled = false;
        }
        hub.save_config(&cfg).unwrap();
        fake.finish(&a.root);
        let events = set.poll(&hub);
        let message = events
            .iter()
            .find_map(|(root, e)| match e {
                RunEvent::Finished { ok: false, message, .. } if *root == b.root => Some(message.clone()),
                _ => None,
            })
            .expect("the queued run reports why it could not start");
        assert!(message.contains("is turned off"), "the queued run kept its builder: {message}");
    }
}
