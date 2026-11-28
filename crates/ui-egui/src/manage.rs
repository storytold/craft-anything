//! Rename, duplicate and delete a project, from its row in the sidebar (right-click) or its page.
//! Each goes through its tool (project_rename, project_duplicate, project_delete), as the CLI and
//! MCP clients do; deleting asks first and says what stays.

use crate::i18n::{tr, trf};
use crate::state::{Modal, Screen};
use crate::theme::{self, Tokens};
use crate::{CraftApp, icons, widgets};
use craft_anything_engine::evidence::ToolError;
use craft_anything_engine::workspace::Workspace;
use egui::{RichText, Ui, Vec2};
use serde_json::{Value, json};
use std::path::Path;

impl CraftApp {
    pub fn rename_project(&mut self, root: &Path, name: &str) {
        self.act("project_rename", json!({"project": root, "name": name}), tr("Rename the project"));
    }

    pub fn duplicate_project(&mut self, root: &Path) {
        self.act("project_duplicate", json!({"project": root}), tr("Duplicate the project"));
    }

    /// Delete a project (after the person confirmed). Never one that's building.
    pub fn delete_project(&mut self, root: &Path) {
        if self.runs.is_busy(root) {
            return self.toast(tr("Pause the build first, then delete the project."), true);
        }
        self.act("project_delete", json!({"project": root, "confirm": true}), tr("Delete the project"));
    }

    /// A rename, duplicate or delete finished.
    pub(crate) fn managed(&mut self, tool: &str, result: Result<Value, ToolError>) {
        let v = match result {
            Ok(v) => v,
            Err(e) => return self.toast(e.for_people(), true),
        };
        let root = v.get("project").or_else(|| v.get("deleted")).and_then(Value::as_str).map(std::path::PathBuf::from);
        match tool {
            "project_rename" => {
                if let Some(ws) = self.workspace.as_mut().filter(|w| Some(&w.root) == root.as_ref()) {
                    let _ = ws.reload();
                }
            }
            "project_duplicate" => {
                let name = v.get("name").and_then(Value::as_str).unwrap_or_default();
                let made = trf("Made a copy: {name}", &[("name", name)]);
                match v.get("creation_warning").and_then(Value::as_str) {
                    Some(warning) => self.toast(format!("{made} {warning}"), true),
                    None => self.toast(made, false),
                }
            }
            _ => {
                if self.workspace.as_ref().is_some_and(|w| Some(&w.root) == root.as_ref()) {
                    self.workspace = None;
                    self.workspace_identity = None;
                    self.state.feed.clear();
                    self.state.screen = Screen::Home;
                }
                if let Some(r) = &root {
                    self.usage.remove(r);
                    self.state.unsent.remove(r);
                }
                let kept = v.get("your_files_kept").and_then(Value::as_bool).unwrap_or(false);
                self.toast(tr(if kept { "Deleted. Your own files are still in the folder." } else { "Deleted." }), false);
            }
        }
        self.refresh_projects();
    }
}

/// The right-click menu on a project in the sidebar.
pub fn menu(app: &mut CraftApp, ui: &mut Ui, ws: &Workspace) {
    if widgets::menu_item(ui, false, tr("Rename…")).clicked() {
        ask_rename(app, ws);
        ui.close();
    }
    if widgets::menu_item(ui, false, tr("Duplicate")).clicked() {
        app.duplicate_project(&ws.root);
        ui.close();
    }
    ui.separator();
    if ui
        .add_enabled_ui(!app.runs.is_busy(&ws.root), |ui| widgets::menu_item(ui, false, tr("Delete…")))
        .inner
        .on_disabled_hover_text(tr("Pause the build first."))
        .clicked()
    {
        app.state.modal = Some(Modal::Delete(ws.root.clone()));
        ui.close();
    }
}

/// The same, as quiet links on the project's page.
pub fn rows(app: &mut CraftApp, ui: &mut Ui, ws: &Workspace) {
    if widgets::action_row(ui, "file", "Rename", false).clicked() {
        ask_rename(app, ws);
    }
    if widgets::action_row(ui, "copy", "Duplicate", false).on_hover_text(tr("A copy to try something else from here.")).clicked() {
        app.duplicate_project(&ws.root);
    }
}

/// The row that deletes the project: last in its list, and not there while it builds.
pub fn delete_row(app: &mut CraftApp, ui: &mut Ui, ws: &Workspace) {
    if !app.runs.is_busy(&ws.root) && widgets::action_row(ui, "trash-2", "Delete the project", true).clicked() {
        app.state.modal = Some(Modal::Delete(ws.root.clone()));
    }
}

fn ask_rename(app: &mut CraftApp, ws: &Workspace) {
    app.state.rename_input = ws.manifest.name.clone();
    app.state.modal = Some(Modal::Rename(ws.root.clone()));
}

fn project<'a>(app: &'a CraftApp, root: &Path) -> Option<&'a Workspace> {
    app.workspace.iter().chain(&app.projects).find(|w| w.root == root)
}

pub fn rename_modal(app: &mut CraftApp, ui: &mut Ui, root: &Path, close: &mut bool) {
    let t = Tokens::get(ui.ctx());
    ui.label(theme::heading(tr("Rename the project"), theme::SECTION));
    ui.add_space(theme::space::SM);
    let r = ui.add(egui::TextEdit::singleline(&mut app.state.rename_input).desired_width(f32::INFINITY).frame(widgets::text_field_frame(&t)));
    widgets::named(r.clone(), "Project name");
    if !r.has_focus() && !r.lost_focus() {
        r.request_focus();
    }
    let ok = !app.state.rename_input.trim().is_empty();
    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    ui.add(
        egui::Label::new(
            RichText::new(tr("Only its name changes; its folder stays where it is."))
                .color(t.text_faint)
                .size(theme::SMALL)
                .line_height(Some(theme::leading(theme::SMALL))),
        )
        .wrap(),
    );
    ui.add_space(theme::space::MD);
    ui.horizontal(|ui| {
        if (widgets::primary_icon(ui, None, "Rename", ok).clicked() || enter) && ok {
            let name = app.state.rename_input.trim().to_string();
            app.rename_project(root, &name);
            *close = true;
        }
        if widgets::secondary(ui, None, "Cancel").clicked() {
            *close = true;
        }
    });
}

pub fn delete_modal(app: &mut CraftApp, ui: &mut Ui, root: &Path, close: &mut bool) {
    let t = Tokens::get(ui.ctx());
    let Some(ws) = project(app, root).cloned() else {
        *close = true;
        return;
    };
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, t.radius, t.raised);
        icons::paint(ui, rect, "trash-2", theme::icon::LG, t.danger);
        ui.label(theme::heading(trf("Delete {name}?", &[("name", &ws.manifest.name)]), theme::SECTION));
    });
    ui.add_space(theme::space::SM);
    // What goes and what stays, in so many words: it can't be undone.
    let m = &ws.manifest;
    let what = if m.app_in_root || !m.start_files.is_empty() {
        tr("Craft Anything's records, checks and anything it built in app/ are deleted. Your own files in the folder stay as they are.")
    } else {
        tr("Its folder is deleted with everything in it: what was built, the checks and the conversation.")
    };
    ui.add(egui::Label::new(RichText::new(what).color(t.text_dim)).wrap());
    ui.add(egui::Label::new(RichText::new(crate::short_path(&ws.root)).font(theme::mono(theme::CODE)).color(t.text_faint)).truncate());
    ui.add_space(theme::space::XS);
    ui.label(RichText::new(tr("This can't be undone.")).color(t.danger).size(theme::SMALL));
    ui.add_space(theme::space::MD);
    ui.horizontal(|ui| {
        if widgets::danger(ui, Some("trash-2"), "Delete it").clicked() {
            app.delete_project(root);
            *close = true;
        }
        if widgets::primary(ui, "Keep it").clicked() {
            *close = true;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_anything_engine::workspace::TargetRef;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn a_published_copy_with_cleanup_guidance_keeps_the_successful_result() {
        for warning in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let original = Workspace::create(directory.path(), "Original", "A notes app", TargetRef::None).unwrap();
            let copy = original.duplicate("Copy").unwrap();
            let guidance = format!("Inspect {} before retrying cleanup.", copy.root.join("retained-cleanup").display());
            let mut result = json!({"project": copy.root, "name": copy.manifest.name, "copied_from": original.root});
            if warning {
                result["creation_warning"] = json!(guidance);
            }
            let mut app = crate::demo::app(crate::demo::engine(craft_anything_engine::Mode::Beginner, true));
            app.open_workspace(original.clone());

            app.managed("project_duplicate", Ok(result));

            assert_eq!(app.workspace.as_ref().map(|workspace| &workspace.root), Some(&original.root), "copying keeps the existing open project");
            assert_eq!(Workspace::open(&copy.root).unwrap().manifest.name, "Copy");
            assert!(app.state.toasts.iter().any(|(text, persistent)| text.contains("Copy") && *persistent == warning));
            let mut harness = Harness::builder().with_size(egui::vec2(1280.0, 820.0)).with_max_steps(32).build_eframe(move |cc| {
                CraftApp::setup_context(&cc.egui_ctx);
                app
            });
            harness.run_steps(3);
            if warning {
                harness.get_by_label_contains(&guidance);
            } else {
                harness.get_by_label_contains("Made a copy:");
                assert!(harness.state().state.toasts.iter().all(|(text, _)| !text.contains(&guidance)));
            }
        }
    }
}
