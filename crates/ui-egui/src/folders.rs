//! Where a new project goes: the projects folder from Settings, or a folder the person picks,
//! which may already have their files. Then they're asked (unless they said not to) whether those
//! files are the app to edit in place or material to start from.

use crate::i18n::{tr, trf};
use crate::state::Modal;
use crate::theme::{self, Tokens};
use crate::{CraftApp, Pick, widgets};
use craft_anything_ai::rebuild::stacks;
use craft_anything_engine::workspace::{self, ExistingFiles, FolderState, Workspace};
use egui::{RichText, Ui, Vec2};
use std::path::PathBuf;

/// The Make screen's "Saved in" line: the folder the project will be in, and a way to pick another.
pub fn make_line(app: &mut CraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let d = &app.state.make;
    let (place, how) = match &d.folder {
        Some(f) => (
            f.display().to_string(),
            match (d.folder_files.is_some(), d.existing) {
                (false, _) => None,
                (true, Some(ExistingFiles::Edit)) => Some(tr("your files there are the app, edited in place")),
                (true, Some(ExistingFiles::StartFrom)) => Some(tr("the app is built beside your files, starting from them")),
                (true, None) => Some(tr("say how to use the files there")),
            },
        ),
        None => (trf("a new folder in {dir}", &[("dir", &app.settings.projects_dir().display().to_string())]), None),
    };
    let picked = d.folder.is_some();
    // Where it goes (and what the files there are), then what can be changed, each on its own line.
    ui.horizontal(|ui| {
        crate::icons::show(ui, "folder", theme::icon::SM, t.text_faint);
        ui.label(RichText::new(tr("Saved in")).size(theme::SMALL).color(t.text_dim));
        ui.add(egui::Label::new(RichText::new(&place).size(theme::SMALL).color(t.text)).truncate()).on_hover_text(&place);
    });
    ui.horizontal_wrapped(|ui| {
        ui.add_space(theme::space::XL);
        if let Some(how) = how {
            ui.label(RichText::new(how).size(theme::SMALL).color(t.text_dim));
            if widgets::link(ui, "Change").clicked() {
                app.state.modal = Some(Modal::ExistingFiles);
            }
            ui.label(RichText::new("·").size(theme::SMALL).color(t.text_faint));
        }
        if widgets::link(ui, if picked { "Pick another folder…" } else { "Choose a folder…" }).clicked() {
            app.pick(Pick::ProjectFolder);
        }
        if picked {
            ui.label(RichText::new("·").size(theme::SMALL).color(t.text_faint));
            if widgets::link(ui, "Use the projects folder").clicked() {
                clear(app);
            }
        }
    });
}

fn clear(app: &mut CraftApp) {
    let d = &mut app.state.make;
    d.folder = None;
    d.folder_files = None;
    d.folder_stack = None;
    d.existing = None;
}

/// A folder was picked for the new project: an existing project opens; one with files asks what
/// they are (or uses the answer from Settings).
pub fn picked(app: &mut CraftApp, folder: PathBuf) {
    match workspace::folder_state(&folder) {
        Ok(FolderState::Project) => open_project(app, &folder),
        Ok(FolderState::Empty) => {
            clear(app);
            app.state.make.folder = Some(folder);
        }
        Ok(FolderState::HasFiles { names, count }) => {
            clear(app);
            let d = &mut app.state.make;
            d.folder = Some(folder);
            d.folder_files = Some((names, count));
            match app.settings.existing_files {
                Some(choice) => choose(app, choice),
                None => app.state.modal = Some(Modal::ExistingFiles),
            }
        }
        Err(e) => app.toast(e.for_people(), true),
    }
}

/// "Work on my project" picked a folder: its files are the app, edited in place (nothing asked).
/// An existing project opens; an empty folder has nothing to work on.
pub fn picked_own(app: &mut CraftApp, folder: PathBuf) {
    // The dialog runs beside the window: a pick that ends after leaving the door is dropped, or
    // a version made on Make would edit that folder.
    if !app.state.make.mine {
        return;
    }
    match workspace::folder_state(&folder) {
        Ok(FolderState::Project) => open_project(app, &folder),
        Ok(FolderState::Empty) => app.toast(tr("That folder is empty. Pick the folder your project's files are in."), true),
        Ok(FolderState::HasFiles { names, count }) => {
            clear(app);
            let d = &mut app.state.make;
            d.folder = Some(folder);
            d.folder_files = Some((names, count));
            choose(app, ExistingFiles::Edit);
        }
        Err(e) => app.toast(e.for_people(), true),
    }
}

fn open_project(app: &mut CraftApp, folder: &std::path::Path) {
    match Workspace::open(folder) {
        Ok(ws) => {
            app.toast(tr("That folder is already a project, so it's open now."), false);
            app.open_workspace(ws);
            app.remember_project(folder);
        }
        Err(e) => app.toast(e.for_people(), true),
    }
}

/// Use the picked folder's files this way, unless something there is named like Craft Anything's
/// own files: then the folder isn't used (nothing of theirs is ever replaced).
fn choose(app: &mut CraftApp, choice: ExistingFiles) {
    let Some(folder) = app.state.make.folder.clone() else { return };
    let clash = workspace::clashes(&folder, choice);
    if clash.is_empty() {
        let d = &mut app.state.make;
        d.existing = Some(choice);
        // Read here, once: the screens that use it are drawn every frame.
        d.folder_stack = (choice == ExistingFiles::Edit).then(|| stacks::found_in(&folder)).flatten().map(|s| s.name.clone());
    } else {
        clear(app);
        let names = clash.join(", ");
        app.toast(
            trf(
                "That folder already has {names}, where Craft Anything keeps its own files. Pick another folder, or move those out first.",
                &[("names", &names)],
            ),
            true,
        );
    }
}

/// "This folder already has files": the app itself, or material to start from.
pub fn existing_modal(app: &mut CraftApp, ui: &mut Ui, close: &mut bool) {
    let t = Tokens::get(ui.ctx());
    ui.set_width(560.0);
    let Some(folder) = app.state.make.folder.clone() else {
        *close = true;
        return;
    };
    let (names, count) = app.state.make.folder_files.clone().unwrap_or_default();
    ui.label(theme::heading(tr("This folder already has files"), theme::SECTION));
    ui.add_space(theme::space::XS);
    let some = names.iter().take(4).cloned().collect::<Vec<_>>().join(", ");
    let more = if count > 4 { trf(" and {n} more", &[("n", &(count - 4).to_string())]) } else { String::new() };
    ui.add(egui::Label::new(RichText::new(format!("{}: {some}{more}.", folder.display())).color(t.text_dim)).wrap());
    ui.add_space(theme::space::XS);
    ui.label(RichText::new(tr("How should the builder use them?")).color(t.text));
    ui.add_space(theme::space::MD);
    let current = app.state.make.existing;
    let size = Vec2::new(268.0, 168.0);
    let mut chosen = None;
    ui.horizontal(|ui| {
        let edit = widgets::choice_tile(
            ui,
            size,
            "code",
            "They're my app: edit them in place",
            "The builder works on these files directly and keeps what works. Craft Anything adds a few records of its own beside them.",
            current == Some(ExistingFiles::Edit),
        );
        if edit.clicked() {
            chosen = Some(ExistingFiles::Edit);
        }
        let start = widgets::choice_tile(
            ui,
            size,
            "layers",
            "Start from them",
            "The builder reads them (code, notes, pictures) and builds the app in a new app folder beside them. Your files stay as they are.",
            current == Some(ExistingFiles::StartFrom),
        );
        if start.clicked() {
            chosen = Some(ExistingFiles::StartFrom);
        }
    });
    ui.add_space(theme::space::MD);
    let mut remember = app.state.make.remember_existing;
    if widgets::toggle(ui, &mut remember, "Don't ask again (you can change this in Settings)").changed() {
        app.state.make.remember_existing = remember;
    }
    ui.add_space(theme::space::MD);
    if widgets::secondary(ui, None, "Cancel: don't use this folder").clicked() {
        clear(app);
        *close = true;
    }
    if let Some(choice) = chosen {
        if app.state.make.remember_existing {
            app.settings.existing_files = Some(choice);
            app.settings_changed();
        }
        choose(app, choice);
        *close = true;
    }
}

/// Settings: the answer used when a picked folder has files.
pub const EXISTING_CHOICES: [(&str, Option<ExistingFiles>); 3] =
    [("Ask each time", None), ("Edit them in place", Some(ExistingFiles::Edit)), ("Start from them", Some(ExistingFiles::StartFrom))];

impl CraftApp {
    /// Keep a project that lives outside the project folders in the list.
    pub fn remember_project(&mut self, root: &std::path::Path) {
        if self.settings.remember_project(root) {
            self.settings_changed();
        }
        self.refresh_projects();
    }

    /// Settings: list projects from another folder too.
    pub fn add_project_folder(&mut self, folder: PathBuf) {
        if !self.settings.project_locations().contains(&folder) {
            self.settings.project_folders.push(folder);
            self.settings_changed();
            self.refresh_projects();
        }
    }

    /// Settings: stop listing a folder's projects (nothing is deleted).
    ///
    /// If the currently open project is no longer listed under `projects_dir` or any remaining
    /// `project_folders` entry, close it and return to Home so closing/restarting the app does not
    /// restore it through `last_project` and silently add it back to `project_folders` (#2340).
    pub fn remove_project_folder(&mut self, folder: &std::path::Path) {
        let target = workspace::normalized(folder);
        self.settings.project_folders.retain(|f| workspace::normalized(f) != target);
        if let Some(ws) = self.workspace.as_ref().filter(|ws| !self.settings.lists_project(&ws.root)) {
            let root = ws.root.clone();
            let typed = (std::mem::take(&mut self.state.project_input), std::mem::take(&mut self.state.project_attachments));
            if !typed.0.trim().is_empty() || !typed.1.is_empty() {
                self.state.unsent.insert(root, typed);
            }
            self.workspace = None;
            self.preview = None;
            self.state.feed.clear();
            if self.state.screen == crate::state::Screen::Project {
                self.state.screen = crate::state::Screen::Home;
            }
        }
        self.settings_changed();
        self.refresh_projects();
    }
}

#[cfg(test)]
mod tests {
    use crate::demo;
    use crate::state::Screen;
    use craft_anything_engine::workspace::{TargetRef, Workspace};

    struct MemStorage(std::collections::HashMap<String, String>);
    impl eframe::Storage for MemStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }

    fn store_prefs(prefs: &crate::Prefs) -> MemStorage {
        let mut s = MemStorage(std::collections::HashMap::new());
        s.0.insert(crate::PREFS_KEY.into(), serde_json::to_string(prefs).unwrap());
        s
    }

    fn make_app(default_projects: &std::path::Path, folders: Vec<std::path::PathBuf>) -> crate::CraftApp {
        let engine = demo::engine(craft_anything_engine::Mode::Beginner, true);
        {
            let mut s = engine.session();
            s.settings.projects_dir = Some(default_projects.to_path_buf());
            s.settings.project_folders = folders;
        }
        let mut app = demo::app(engine);
        app.settings.projects_dir = Some(default_projects.to_path_buf());
        app
    }

    /// #2340: clicking "Stop listing" on the active external project (or the listed parent folder
    /// containing it) closes the now-unlisted project so a restart does not reopen and re-add it,
    /// while stopping an unrelated folder leaves a project inside `projects_dir` open.
    #[test]
    fn stopping_a_listed_project_folder_closes_unlisted_active_project_and_stays_unlisted_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let default_projects = dir.path().join("default-projects");
        let external_parent = dir.path().join("external");
        let ws = Workspace::create(&external_parent, "Picked", "Build a clock", TargetRef::None).unwrap();

        // Direct project folder listed and open: "Stop listing" (even via a non-normalized path)
        // closes it, clears preview/feed, stashes unsent input without leaking it to a new project,
        // and clears `last_project`.
        let mut app = make_app(&default_projects, Vec::new());
        app.open_workspace(ws.clone());
        assert_eq!(app.state.screen, Screen::Project);
        assert!(app.settings.project_folders.contains(&ws.root), "opening an external project lists it");
        app.preview = Some("http://127.0.0.1:4321".into());
        app.state.feed.push(crate::state::FeedItem::User(crate::state::UserItem::Text("Hello P".into())));
        app.state.project_input = "draft for P".into();

        app.remove_project_folder(&ws.root.join("."));
        assert!(!app.settings.project_folders.contains(&ws.root), "removed from project_folders via normalized match");
        assert!(app.workspace.is_none(), "unlisted open project is closed");
        assert!(app.preview.is_none(), "preview cleared when open project closes");
        assert!(app.state.feed.is_empty(), "feed cleared when open project closes");
        assert!(app.state.project_input.is_empty(), "project_input cleared so it cannot leak into another project");
        assert_eq!(app.state.unsent.get(&ws.root).map(|(t, _)| t.as_str()), Some("draft for P"), "unsent draft preserved for P");
        assert_eq!(app.state.screen, Screen::Home, "closing the open project returns to Home");
        let saved_prefs = app.prefs();
        assert_eq!(saved_prefs.last_project, None, "prefs no longer point at the unlisted project");

        let saved_store = store_prefs(&saved_prefs);
        let mut restarted = make_app(&default_projects, app.settings.project_folders.clone());
        restarted.load_prefs(Some(&saved_store));
        assert!(!restarted.settings.project_folders.contains(&ws.root), "restart must not re-add P");
        assert!(restarted.workspace.is_none());

        // Listed parent folder removed while a child project is open on another screen (e.g. Ask):
        // closes the child workspace without forcibly overriding the active non-Project screen.
        let mut parent_app = make_app(&default_projects, vec![external_parent.clone()]);
        parent_app.open_workspace(ws.clone());
        parent_app.state.screen = Screen::Ask;
        assert_eq!(parent_app.settings.project_folders, vec![external_parent.clone()]);
        parent_app.remove_project_folder(&external_parent);
        assert!(parent_app.workspace.is_none(), "removing listed parent closes active child project");
        assert_eq!(parent_app.state.screen, Screen::Ask, "non-Project screen is preserved");

        // Removing an unrelated external folder keeps an active project inside `projects_dir` open.
        let default_ws = Workspace::create(&default_projects, "Default", "Build a timer", TargetRef::None).unwrap();
        let mut keep_app = make_app(&default_projects, vec![external_parent.clone()]);
        keep_app.open_workspace(default_ws.clone());
        keep_app.remove_project_folder(&external_parent);
        assert_eq!(keep_app.workspace.as_ref().map(|w| &w.root), Some(&default_ws.root));
        assert_eq!(keep_app.state.screen, Screen::Project);
    }

    /// #2340: if `project_folders` was edited out of `config.toml` while UI prefs still hold
    /// `last_project`, startup restore must not silently re-add the unlisted project to
    /// `project_folders` or the sidebar project list (even after `refresh_projects` finishes),
    /// whereas a still-listed project is shown in the sidebar and explicit `open_workspace` still
    /// adds an external project back.
    #[test]
    fn restoring_last_project_does_not_relist_unlisted_project_folders() {
        let dir = tempfile::tempdir().unwrap();
        let default_projects = dir.path().join("default-projects");
        let external_parent = dir.path().join("external");
        let ws = Workspace::create(&external_parent, "Picked", "Build a clock", TargetRef::None).unwrap();
        let listed_ws = Workspace::create(&default_projects, "Listed", "Build a timer", TargetRef::None).unwrap();

        let mut app = make_app(&default_projects, Vec::new());
        let mut stale_prefs = app.prefs();
        stale_prefs.last_project = Some(ws.root.clone());
        let stale_store = store_prefs(&stale_prefs);

        app.load_prefs(Some(&stale_store));
        assert!(!app.settings.project_folders.contains(&ws.root), "stale last_project must not re-add an unlisted folder");
        assert!(!app.projects.iter().any(|p| p.root == ws.root), "unlisted project is not inserted back into sidebar list");

        // Running `refresh_projects()` (`Done::Projects`) must also keep the unlisted workspace out
        // of `app.projects`.
        app.refresh_projects();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut drained = false;
        while std::time::Instant::now() < deadline {
            let finished = app.jobs.poll();
            if !finished.is_empty() {
                for f in finished {
                    app.handle_done(f);
                }
                drained = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(drained, "refresh_projects job completed");
        assert!(!app.projects.iter().any(|p| p.root == ws.root), "Done::Projects must not re-insert unlisted workspace into sidebar");

        // Restoring a project that IS still listed puts it into `app.projects` immediately.
        let mut listed_app = make_app(&default_projects, Vec::new());
        let mut listed_prefs = listed_app.prefs();
        listed_prefs.last_project = Some(listed_ws.root.clone());
        let listed_store = store_prefs(&listed_prefs);
        listed_app.load_prefs(Some(&listed_store));
        assert!(listed_app.projects.iter().any(|p| p.root == listed_ws.root), "listed project restored on startup appears in sidebar");

        // Explicitly opening the external project again still remembers it in `project_folders`.
        app.open_workspace(ws.clone());
        assert!(app.settings.project_folders.contains(&ws.root));
        assert_eq!(app.workspace.as_ref().map(|w| &w.root), Some(&ws.root));

        // Stale `Done::Projects` snapshot arriving after "Stop listing" must not put the unlisted
        // folder's project back into `app.projects`, while still keeping listed projects visible.
        app.remove_project_folder(&ws.root);
        app.handle_done(crate::jobs::Finished { job: 99, result: Ok(crate::Done::Projects(vec![ws.clone(), listed_ws.clone()])) });
        assert!(!app.projects.iter().any(|p| p.root == ws.root), "stale Done::Projects snapshot must not re-add unlisted project");
        assert!(app.projects.iter().any(|p| p.root == listed_ws.root), "still-listed project remains in sidebar");
    }
}
