//! The session: open targets, the evidence ledger, unknowns, annotations and caches.

use crate::project::Project;
use crate::settings::Settings;
use crate::target::{Target, TargetId};
use craft_anything_evidence::{ErrorCode, Evidence, ToolError, Unknown};
use std::any::{Any, TypeId};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// Per-target annotations made by people or agents (names, comments, bookmarks).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Annotations {
    pub names: BTreeMap<u64, String>,
    pub comments: BTreeMap<u64, String>,
    pub inline_comments: BTreeMap<u64, String>,
    pub bookmarks: BTreeMap<u64, String>,
}

/// Where the user (or agent) is looking in a target.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Cursor {
    pub address: Option<u64>,
}

/// Type-keyed cache slots that analysis layers attach to a target (disassembly, module graphs…).
#[derive(Default)]
pub struct Extensions(HashMap<TypeId, Arc<dyn Any + Send + Sync>>);

impl Extensions {
    pub fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.0.get(&TypeId::of::<T>()).and_then(|a| a.clone().downcast::<T>().ok())
    }

    pub fn insert<T: Any + Send + Sync>(&mut self, value: Arc<T>) {
        self.0.insert(TypeId::of::<T>(), value);
    }

    pub fn remove<T: Any + Send + Sync>(&mut self) -> Option<Arc<T>> {
        self.0.remove(&TypeId::of::<T>()).and_then(|a| a.downcast::<T>().ok())
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

impl std::fmt::Debug for Extensions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Extensions({} slots)", self.0.len())
    }
}

/// One open target with its annotations and caches.
#[derive(Debug)]
pub struct OpenTarget {
    pub target: Target,
    pub annotations: Annotations,
    pub cursor: Cursor,
    pub ext: Extensions,
}

/// How many project evidence records a session keeps in memory (newest first). Older ones stay on
/// disk and still resolve by id.
pub const MAX_LOADED_EVIDENCE: usize = 5_000;
/// Total bytes of project evidence a session loads into memory before leaving the rest on disk.
const MAX_LOADED_EVIDENCE_BYTES: u64 = 64 * 1024 * 1024;

/// All state a tool can touch.
#[derive(Debug, Default)]
pub struct Session {
    pub settings: Settings,
    pub project: Option<Project>,
    targets: BTreeMap<TargetId, OpenTarget>,
    active: Option<TargetId>,
    ledger: BTreeMap<String, Evidence>,
    /// Insertion order of the ledger, for "latest first" views.
    order: Vec<String>,
    unknowns: BTreeMap<String, Unknown>,
    /// Project records left on disk because the project holds more than [`MAX_LOADED_EVIDENCE`].
    /// They still resolve by id through [`Session::resolve_evidence`].
    unloaded_evidence: usize,
    /// Raw-file digests for the last refresh's verified, budgeted records. They refer to the
    /// corresponding ledger values, so replacing one locally invalidates its digest.
    project_evidence_digests: HashMap<String, String>,
    project_evidence_root: Option<std::path::PathBuf>,
    /// Session-wide caches not tied to a target (device lists, browser connections…).
    pub ext: Extensions,
    /// Where this session keeps config.toml and plugin caches, when not the usual folder
    /// ([`crate::settings::config_dir`]); see [`Session::isolated`].
    config_dir: Option<std::path::PathBuf>,
    /// Deep mode (`true`) or Clean room (`false`) for this process only, over
    /// `settings.deep_mode` and never saved: the CLI's `--deep` and `--clean-room`.
    pub deep_for_run: Option<bool>,
    /// Where Cargo builds, as the environment says, for finding a Rust build by its files; `None`
    /// reads this process's environment. Tests give one ([`crate::testing::session`]) so they
    /// don't depend on this computer's Cargo home or `CARGO_TARGET_DIR`.
    pub cargo_env: Option<crate::outputs::CargoEnv>,
}

impl Session {
    pub fn new(settings: Settings) -> Self {
        Session { settings, ..Default::default() }
    }

    /// [`Session::cargo_env`], or this process's environment when none was given.
    pub fn cargo(&self) -> crate::outputs::CargoEnv {
        self.cargo_env.clone().unwrap_or_else(crate::outputs::CargoEnv::of_process)
    }

    /// A session that keeps everything under `dir` (config.toml, plugin caches, the engine store,
    /// new projects) instead of this computer's own folders: for tests, and anything that must not
    /// touch the person's setup.
    pub fn isolated(dir: &std::path::Path) -> Self {
        let mut settings = Settings::default();
        settings.engines.store_dir = Some(dir.join("engines"));
        settings.projects_dir = Some(dir.join("projects"));
        Session { settings, config_dir: Some(dir.to_path_buf()), ..Default::default() }
    }

    /// A session for one piece of work beside this one ([`crate::Engine::for_a_run`]): the same
    /// settings, plugins and asset providers, nothing open, and this session's mode, kept for the
    /// whole run.
    pub fn for_a_run(&self) -> Session {
        let mut run = Session { config_dir: self.config_dir.clone(), deep_for_run: Some(!self.clean_room()), ..Default::default() };
        run.follow(self);
        run
    }

    /// A session for one piece of work from the saved settings, for when the session it works
    /// beside is busy ([`crate::Engine::for_a_run`]); `config_dir` is where they were read from.
    pub fn for_a_run_from(settings: Settings, config_dir: Option<std::path::PathBuf>, deep_for_run: Option<bool>) -> Session {
        Session { settings, config_dir, deep_for_run, ..Default::default() }
    }

    /// Take on what the person changes in the app while this session works: settings and what
    /// they've allowed for this project. Asset keys are shared by the engines themselves
    /// ([`crate::Engine::load_asset_keys`]). Not the mode: a Clean room run that took on Deep mode
    /// would read inside the original for a build recorded as Clean room.
    pub fn follow(&mut self, app: &Session) {
        use craft_anything_engines::assets::AssetProviders;
        self.settings.clone_from(&app.settings);
        if let Some(plugins) = app.ext.get::<crate::plugins::Plugins>() {
            self.ext.insert(plugins);
        }
        if let Some(providers) = app.ext.get::<AssetProviders>() {
            self.ext.insert(providers);
        }
        if let (Some(mine), Some(theirs)) = (self.project.as_mut(), app.project.as_ref())
            && mine.root == theirs.root
        {
            mine.meta.allowed_tools.clone_from(&theirs.meta.allowed_tools);
        }
    }

    /// Keep a project started in a folder the person picked in the project list
    /// ([`Settings::remember_project`]), saving settings when that changed them.
    pub fn remember_project(&mut self, root: &std::path::Path) -> Result<(), craft_anything_evidence::ToolError> {
        let mut next = self.settings.clone();
        if next.remember_project(root) {
            next.save_to(&self.config_dir().join("config.toml"))?;
            self.settings = next;
        }
        Ok(())
    }

    /// Where this session's config.toml and plugin caches live.
    pub fn config_dir(&self) -> std::path::PathBuf {
        self.config_dir.clone().unwrap_or_else(crate::settings::config_dir)
    }

    /// True in Clean room: nothing inside a target is read ([`crate::clean_room`]).
    pub fn clean_room(&self) -> bool {
        !self.deep_for_run.unwrap_or(self.settings.deep_mode)
    }

    // ---- targets ----

    /// Metadata for a local MCP child, with the active target last so restoring the list also
    /// restores selection. No file bytes or analysis caches cross the process boundary.
    pub fn shared_targets(&self) -> Vec<crate::target::TargetContext> {
        let active = self.active().ok().map(|target| &target.target.path);
        let mut targets: Vec<_> = self.targets().map(|target| crate::target::TargetContext::from_target(&target.target)).collect();
        targets.sort_by_key(|target| active == Some(&target.path));
        targets
    }

    /// Restore a bounded target handoff before serving MCP. Validate every target before
    /// changing the session; the last entry is the desktop's active target.
    pub fn restore_targets(&mut self, targets: &[crate::target::TargetContext]) -> Result<(), ToolError> {
        if targets.len() > 128 {
            return Err(ToolError::invalid("At most 128 targets can be shared with an agent."));
        }
        let opened: Result<Vec<_>, _> = targets.iter().map(|target| target.open(self.clean_room())).collect();
        for target in opened? {
            self.add_target(target);
        }
        Ok(())
    }

    /// Give an external agent a durable store shared with this session. Existing projects stay
    /// open; unsaved evidence, unknowns and investigation memory are copied before attaching a
    /// new project. Call on a worker: creating and refreshing projects does disk I/O.
    pub fn ensure_shared_project(&mut self) -> Result<std::path::PathBuf, ToolError> {
        if let Some(project) = &self.project {
            return Ok(project.root.clone());
        }
        let investigation = crate::tools::investigation_tools::load(self);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|error| ToolError::internal(error.to_string()))?;
        let root = self.config_dir().join("investigations").join(format!("session-{}.cany", stamp.as_nanos()));
        let project = Project::create(&root, "Investigation", crate::project::Purpose::default())?;
        for evidence in self.ledger() {
            project.store_evidence(evidence)?;
        }
        for unknown in self.unknowns() {
            project.store_unknown(unknown)?;
        }
        self.project = Some(project);
        if let Err(error) = crate::tools::investigation_tools::save_new_project(self, investigation) {
            self.project = None;
            return Err(error);
        }
        Ok(root)
    }

    pub fn add_target(&mut self, target: Target) -> TargetId {
        let id = target.id.clone();
        let annotations = self.project.as_ref().and_then(|p| p.load_annotations(&target.sha256).ok()).unwrap_or_default();
        self.targets.insert(id.clone(), OpenTarget { target, annotations, cursor: Cursor::default(), ext: Extensions::default() });
        self.active = Some(id.clone());
        id
    }

    pub fn close_target(&mut self, id: &TargetId) -> Option<OpenTarget> {
        let removed = self.targets.remove(id);
        if self.active.as_ref() == Some(id) {
            self.active = self.targets.keys().next_back().cloned();
        }
        removed
    }

    pub fn targets(&self) -> impl Iterator<Item = &OpenTarget> {
        self.targets.values()
    }

    pub fn target(&self, id: &TargetId) -> Option<&OpenTarget> {
        self.targets.get(id)
    }

    pub fn target_mut(&mut self, id: &TargetId) -> Option<&mut OpenTarget> {
        self.targets.get_mut(id)
    }

    pub fn active_id(&self) -> Option<&TargetId> {
        self.active.as_ref()
    }

    pub fn set_active(&mut self, id: &TargetId) -> Result<(), ToolError> {
        if self.targets.contains_key(id) {
            self.active = Some(id.clone());
            Ok(())
        } else {
            Err(ToolError::new(ErrorCode::NotFound, format!("No open target {id}")))
        }
    }

    /// The active target, or a `not_found` error that tells the caller what to do.
    pub fn active(&self) -> Result<&OpenTarget, ToolError> {
        self.active.as_ref().and_then(|id| self.targets.get(id)).ok_or_else(ToolError::target_required)
    }

    pub fn active_mut(&mut self) -> Result<&mut OpenTarget, ToolError> {
        match self.active.clone() {
            Some(id) => self.targets.get_mut(&id).ok_or_else(ToolError::target_required),
            None => Err(ToolError::target_required()),
        }
    }

    /// Persist the active target's annotations to the project, if one is open.
    pub fn save_annotations(&self) -> Result<(), ToolError> {
        if let (Some(project), Ok(t)) = (&self.project, self.active()) {
            project.save_annotations(&t.target.sha256, &t.annotations)?;
        }
        Ok(())
    }

    // ---- evidence ----

    /// Record evidence in the ledger (and the project store, when a project is open).
    pub fn record(&mut self, ev: Evidence) -> Result<(), ToolError> {
        if let Some(project) = &self.project {
            project.store_evidence(&ev)?;
        }
        // An existing content-addressed file is not overwritten by store_evidence. The new
        // in-memory record may have different display fields and is not that disk snapshot.
        self.project_evidence_digests.remove(&ev.id);
        if !self.ledger.contains_key(&ev.id) {
            self.order.push(ev.id.clone());
        }
        self.ledger.insert(ev.id.clone(), ev);
        Ok(())
    }

    pub fn evidence(&self, id: &str) -> Option<&Evidence> {
        self.ledger.get(id).or(None)
    }

    /// Look up evidence in memory, then in the project store.
    pub fn resolve_evidence(&self, id: &str) -> Result<Evidence, ToolError> {
        if let Some(e) = self.ledger.get(id) {
            return Ok(e.clone());
        }
        if let Some(project) = &self.project
            && let Ok(e) = project.load_evidence(id)
        {
            return Ok(e);
        }
        Err(ToolError::invalid(format!("Unknown evidence id {id}")).with_details(serde_json::json!({"reason": "missing", "evidence_id": id})))
    }

    /// Evidence, newest first.
    pub fn ledger(&self) -> impl Iterator<Item = &Evidence> {
        self.order.iter().rev().filter_map(|id| self.ledger.get(id))
    }

    pub fn ledger_len(&self) -> usize {
        self.ledger.len()
    }

    /// Project records not loaded into the ledger because the project is larger than the in-memory cap.
    pub fn unloaded_evidence(&self) -> usize {
        self.unloaded_evidence
    }

    pub fn clear_ledger(&mut self) {
        self.ledger.clear();
        self.order.clear();
        self.unknowns.clear();
        self.unloaded_evidence = 0;
        self.project_evidence_digests.clear();
        self.project_evidence_root = None;
    }

    // ---- unknowns ----

    pub fn unknowns(&self) -> impl Iterator<Item = &Unknown> {
        self.unknowns.values()
    }

    pub fn unknown(&self, id: &str) -> Option<&Unknown> {
        self.unknowns.get(id)
    }

    pub fn put_unknown(&mut self, u: Unknown) -> Result<(), ToolError> {
        if let Some(project) = &self.project {
            project.store_unknown(&u)?;
        }
        self.unknowns.insert(u.id.clone(), u);
        Ok(())
    }

    pub fn unknown_mut(&mut self, id: &str) -> Option<&mut Unknown> {
        self.unknowns.get_mut(id)
    }

    /// Open a project: its evidence and unknowns become available.
    ///
    /// Opening a different project while one is already open replaces the session's ledger and
    /// open questions with the new project's and closes the previous project's open targets, so a
    /// report can't name them and an annotation can't be saved into the new project's store.
    /// Opening the same project again refreshes its stored records and keeps its targets in
    /// place. The newest stored records (by write time) are loaded into the ledger, up
    /// to [`MAX_LOADED_EVIDENCE`] records and a byte budget; the rest stay on disk and resolve by
    /// id. New or changed records pass their integrity check; tampered or unreadable ones are
    /// skipped. Unchanged records reuse the verified ledger value after checking a digest of the
    /// entire file, including display fields. Refresh still scans and reads the stored files.
    pub fn open_project(&mut self, project: Project) -> Result<(), ToolError> {
        // Read the new project's questions first: a failed read leaves the current project as it was.
        let unknowns = project.load_unknowns()?;
        let switched = self.project.as_ref().is_some_and(|open| crate::workspace::normalized(&open.root) != crate::workspace::normalized(&project.root));
        if switched {
            self.clear_ledger();
            // The old project's targets aren't this one's: a report would name them, and
            // save_annotations would write their annotations into this project's store.
            self.targets.clear();
            self.active = None;
        }
        let root = crate::workspace::normalized(&project.root);
        if self.project_evidence_root.as_ref() != Some(&root) {
            self.project_evidence_digests.clear();
        }
        // Recomputed below; an old count from a larger open must not survive this one.
        self.unloaded_evidence = 0;
        for u in unknowns {
            self.unknowns.insert(u.id.clone(), u);
        }
        let mut stored = project.evidence_files();
        // Newest first, so the budget keeps the most recent work.
        stored.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.id.cmp(&b.id)));
        let mut loaded = Vec::new();
        let mut loaded_count = 0usize;
        let mut digests = HashMap::new();
        let mut bytes = 0u64;
        for (i, f) in stored.iter().enumerate() {
            if loaded_count >= MAX_LOADED_EVIDENCE || bytes.saturating_add(f.bytes) > MAX_LOADED_EVIDENCE_BYTES {
                self.unloaded_evidence = stored.len().saturating_sub(i);
                break;
            }
            let previous = self.project_evidence_digests.get(&f.id).filter(|_| self.ledger.contains_key(&f.id)).map(String::as_str);
            match project.refresh_evidence(&f.id, previous) {
                Ok((evidence, digest)) => {
                    // Reusing an allocation does not make the record free: it consumes the same
                    // count and byte budgets as a fresh load, and only selected digests survive.
                    loaded_count += 1;
                    bytes = bytes.saturating_add(f.bytes);
                    digests.insert(f.id.clone(), digest);
                    if let Some(ev) = evidence {
                        loaded.push(ev);
                    }
                }
                Err(e) => log::warn!("skipping project evidence {}: {}", f.id, e.message),
            }
        }
        // Oldest first into the ledger so `ledger()` (newest first) matches write order.
        for ev in loaded.into_iter().rev() {
            if !self.ledger.contains_key(&ev.id) {
                self.order.push(ev.id.clone());
            }
            self.ledger.insert(ev.id.clone(), ev);
        }
        self.project_evidence_digests = digests;
        self.project_evidence_root = Some(root);
        self.project = Some(project);
        crate::tools::investigation_tools::refresh(self);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Project, Purpose};
    use craft_anything_evidence::{Environment, Evidence, EvidenceBuilder, Severity, Subject, Unknown};

    /// A record that differs by `tag`, and so gets its own id.
    fn evidence(tag: &str) -> Evidence {
        EvidenceBuilder::new("test", Subject::default(), Environment::builtin(true)).result(serde_json::json!({"tag": tag})).build().unwrap()
    }

    fn ledger_ids(session: &Session) -> Vec<String> {
        session.ledger().map(|e| e.id.clone()).collect()
    }

    /// A session that opens a second project must not carry the first project's ledger or open
    /// questions into it; reopening the same project still refreshes it without clearing it.
    #[test]
    fn opening_another_project_starts_from_its_own_evidence() {
        let dir = crate::testing::temp_dir();
        let a = Project::create(&dir.join("a.cany"), "A", Purpose::Education).unwrap();
        let b = Project::create(&dir.join("b.cany"), "B", Purpose::Education).unwrap();
        let ev_a = evidence("a");
        let ev_b = evidence("b");
        let q_a = Unknown::new("A's question", "native", Severity::Medium);
        a.store_evidence(&ev_a).unwrap();
        b.store_evidence(&ev_b).unwrap();
        a.store_unknown(&q_a).unwrap();

        let mut session = crate::testing::session();
        session.open_project(a.clone()).unwrap();
        assert_eq!(ledger_ids(&session), vec![ev_a.id.clone()]);
        assert!(session.unknown(&q_a.id).is_some());
        session.unloaded_evidence = 9;

        session.open_project(b.clone()).unwrap();
        assert_eq!(ledger_ids(&session), vec![ev_b.id.clone()], "B starts from its own store");
        assert_eq!(session.unknowns().count(), 0, "A's open question is gone");
        assert_eq!(session.unloaded_evidence(), 0, "A's unloaded count is gone");

        // Reopening the same project keeps its ledger and doesn't add the record twice.
        session.open_project(b).unwrap();
        assert_eq!(ledger_ids(&session), vec![ev_b.id.clone()]);

        // A's records are still in its own store and come back when it is opened again.
        session.open_project(a).unwrap();
        assert_eq!(ledger_ids(&session), vec![ev_a.id.clone()]);
        assert!(session.unknown(&q_a.id).is_some());

        // A new, empty project starts empty rather than inheriting A's records.
        let empty = Project::create(&dir.join("empty.cany"), "Empty", Purpose::Education).unwrap();
        session.open_project(empty).unwrap();
        assert!(ledger_ids(&session).is_empty(), "a new project starts without evidence");
        assert_eq!(session.unknowns().count(), 0, "and without open questions");
    }

    /// A project switch must close the first project's open targets: a report for the new project
    /// would name them, and an annotation saved afterwards would go to the new project's store
    /// under the old target. Reopening the same project keeps its targets.
    #[test]
    fn opening_another_project_closes_its_targets_and_keeps_annotations_apart() {
        let dir = crate::testing::temp_dir();
        let a = Project::create(&dir.join("a.cany"), "A", Purpose::Education).unwrap();
        let b = Project::create(&dir.join("b.cany"), "B", Purpose::Education).unwrap();
        let target = Target::from_bytes(dir.join("tool.bin"), "tool.bin".into(), b"\x7fELF\x02\x01".to_vec());
        let mut session = crate::testing::session();

        session.open_project(a.clone()).unwrap();
        let id = session.add_target(target.clone());
        session.active_mut().unwrap().annotations.comments.insert(0x1000, "from A".into());
        session.save_annotations().unwrap();
        assert_eq!(a.load_annotations(&target.sha256).unwrap().comments.get(&0x1000).map(String::as_str), Some("from A"), "A keeps its own annotation");

        // Another project starts without the first project's target, and a save writes nothing to it.
        session.open_project(b.clone()).unwrap();
        assert_eq!(session.targets().count(), 0, "A's target is closed");
        assert!(session.active().is_err(), "and nothing is active");
        session.save_annotations().unwrap();
        assert!(b.load_annotations(&target.sha256).is_err(), "no annotation reached B's store");

        // Reopening the same project keeps a target opened for it, and its selection.
        session.open_project(b.clone()).unwrap();
        session.add_target(target.clone());
        session.open_project(b).unwrap();
        assert_eq!(session.targets().count(), 1, "reopening the same project keeps its target");
        assert_eq!(session.active().map(|t| t.target.id.clone()).ok(), Some(id.clone()), "and its selection");

        // Coming back to A doesn't reopen a target that was closed, and A's annotation is intact.
        session.open_project(a.clone()).unwrap();
        assert_eq!(session.targets().count(), 0, "returning to A doesn't reopen what was closed");
        assert_eq!(a.load_annotations(&target.sha256).unwrap().comments.get(&0x1000).map(String::as_str), Some("from A"));
    }

    /// A project that can't be read doesn't open, so the current project and its target stay as
    /// they were. A project whose unknowns folder is simply gone still opens, with none.
    #[test]
    fn an_unreadable_project_leaves_the_current_project_and_its_targets_alone() {
        let dir = crate::testing::temp_dir();
        let a = Project::create(&dir.join("a.cany"), "A", Purpose::Education).unwrap();
        let b = Project::create(&dir.join("b.cany"), "B", Purpose::Education).unwrap();
        let target = Target::from_bytes(dir.join("tool.bin"), "tool.bin".into(), b"\x7fELF\x02\x01".to_vec());
        let mut session = crate::testing::session();
        session.open_project(a.clone()).unwrap();
        let id = session.add_target(target);

        let Some(closed) = crate::tools::artifact_tools::walk_testing::close(&b.root.join("unknowns")) else { return };
        let refused = session.open_project(b);
        drop(closed);
        assert!(refused.is_err(), "an unreadable unknowns folder fails the open");
        assert_eq!(session.project.as_ref().map(|p| p.root.clone()), Some(a.root.clone()), "A is still open");
        assert_eq!(session.targets().count(), 1, "A's target is still open");
        assert_eq!(session.active_id(), Some(&id), "and still selected");

        // A project whose unknowns folder is missing still opens, with no questions.
        let c = Project::create(&dir.join("c.cany"), "C", Purpose::Education).unwrap();
        std::fs::remove_dir_all(c.root.join("unknowns")).unwrap();
        session.open_project(c).unwrap();
        assert_eq!(session.unknowns().count(), 0);
    }
}
