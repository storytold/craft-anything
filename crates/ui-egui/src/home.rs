//! Simple-mode Home: "What do you want to make?" and your projects, with one quiet link to look
//! inside something instead. Plus the friendly modals.

use crate::i18n::tr;
use crate::state::{Modal, Screen};
use crate::theme::{self, Tokens};
use crate::{CraftApp, Pick, Route, icons, widgets};
use craft_anything_engine::Consent;
use egui::{RichText, Ui, Vec2};
use serde_json::json;

/// Tools that open a website, best first.
pub const WEBSITE_TOOLS: &[&str] = &["web_site_open"];
/// Tools that list connected phones, best first.
pub const PHONE_TOOLS: &[&str] = &["adb_devices_find"];

pub static TUTORIALS: &[(&str, &str, &str, &[&str])] = &[
    (
        "calculator",
        "How does the % key work?",
        "Open a calculator app and find the code behind one button.",
        &[
            "Drop a calculator app (or any small program) on the window.",
            "Ask: \"How does the % key work?\"",
            "Open \"Show me the proof\" to see the exact code we found, and how sure we are.",
        ],
    ),
    (
        "network",
        "What does it send over the internet?",
        "See which servers an app talks to, and what it says.",
        &[
            "Open the app you're curious about.",
            "Ask: \"What does it send over the internet?\"",
            "We list the web addresses and messages we can see inside it. Watching it live needs your OK first.",
        ],
    ),
    (
        "safe",
        "Is this file safe to open?",
        "A first look at a file you're unsure about, without running it.",
        &[
            "Drop the file on the window. We only read it; nothing runs.",
            "Ask: \"Is it safe to run?\"",
            "Each answer says how sure we are: Seen it, Worked it out or Best guess.",
        ],
    ),
];

pub fn show(app: &mut CraftApp, ui: &mut Ui) {
    // Keep the content width stable while the scrollbar animates in and out.
    let full_w = (ui.available_width() - ui.spacing().scroll.allocated_width()).max(0.0);
    egui::ScrollArea::vertical().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded).auto_shrink([false, false]).show(ui, |ui| {
        let w = full_w.min(1000.0) - 48.0;
        let pad = ((full_w - w) / 2.0).max(24.0);
        // Tight enough that Start is in view at 900 px with every choice showing (tested).
        ui.add_space(theme::space::XL);
        ui.horizontal(|ui| {
            ui.add_space(pad);
            ui.vertical(|ui| {
                ui.set_width(w);
                crate::make::hero(app, ui, w);
                ui.add_space(theme::space::XXL);
            });
        });
    });
}

/// Friendly modals (coming soon, website, tutorials).
pub fn modals(app: &mut CraftApp, ctx: &egui::Context) {
    let Some(modal) = app.state.modal.clone() else {
        // Closed, however it was: the website field is focused again when it next opens.
        ctx.data_mut(|d| {
            d.remove::<bool>(egui::Id::new("website-dialog-focused"));
            d.insert_temp(egui::Id::new("craft-modal-open"), false);
        });
        return;
    };
    // A fresh area per opening: the shared area's size from the last dialog would otherwise cap
    // this one's scroll viewport and clip a row that fits.
    let was_open = ctx.data(|d| d.get_temp::<bool>(egui::Id::new("craft-modal-open")).unwrap_or(false));
    let generation = ctx.data_mut(|d| {
        let mut generation = d.get_temp::<u64>(egui::Id::new("craft-modal-generation")).unwrap_or(0);
        if !was_open {
            generation = generation.wrapping_add(1);
            d.insert_temp(egui::Id::new("craft-modal-generation"), generation);
        }
        d.insert_temp(egui::Id::new("craft-modal-open"), true);
        generation
    });
    let modal_id = egui::Id::new(("craft-modal", generation));
    let t = Tokens::get(ctx);
    let width = modal_width(&modal);
    let max_h = (ctx.content_rect().height() - 2.0 * theme::space::XL - 2.0 * theme::space::LG).max(120.0);
    // The sizing pass starts at the largest the dialog may be; the content then shrinks it. Without
    // this the scroll area would be measured against the previous frame's shorter dialog and clip
    // a bottom row that should fit.
    let area = egui::Modal::default_area(modal_id).default_size(Vec2::new(width + 2.0 * theme::space::XL, max_h + 2.0 * theme::space::XL));
    let mut close = false;
    let resp = egui::Modal::new(modal_id)
        .area(area)
        .frame(egui::Frame::new().fill(t.card).corner_radius(t.r_lg()).inner_margin(theme::pad::ROOMY).stroke(egui::Stroke::new(1.0, t.card_border)))
        .show(ctx, |ui| {
            ui.set_width(width);
            // The area's stored size is the previous frame's content. Re-apply the current
            // allowance so a dialog that grows (an engine inventory arriving) can use it.
            ui.set_max_height(max_h);
            let scroll = egui::ScrollArea::vertical()
                .id_salt(("craft-modal-scroll", generation))
                .min_scrolled_height(0.0)
                .max_height(max_h)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    match &modal {
                        Modal::ComingSoon { title, body, icon } => {
                            let (rect, _) = ui.allocate_exact_size(Vec2::splat(52.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, t.radius, t.accent_soft);
                            icons::paint(ui, rect, icon, theme::icon::XL, t.accent_ink);
                            ui.add_space(theme::space::MD);
                            ui.label(theme::heading(tr(title), theme::SECTION));
                            ui.add_space(theme::space::XS);
                            ui.label(RichText::new(tr(body)).color(t.text_dim));
                            ui.add_space(theme::space::LG);
                            if widgets::primary(ui, "Got it").clicked() {
                                close = true;
                            }
                        }
                        Modal::Website => {
                            ui.label(theme::heading(tr("Look at a website"), theme::SECTION));
                            ui.add_space(theme::space::SM);
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut app.state.url_input)
                                    .hint_text("https://example.com")
                                    .desired_width(f32::INFINITY)
                                    .frame(widgets::text_field_frame(&t)),
                            );
                            widgets::named(r.clone(), "Website address");
                            // Ready to type into as the dialog opens.
                            let opened = egui::Id::new("website-dialog-focused");
                            if !ui.data(|d| d.get_temp::<bool>(opened).unwrap_or(false)) {
                                r.request_focus();
                                ui.data_mut(|d| d.insert_temp(opened, true));
                            }
                            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            ui.add_space(theme::space::MD);
                            ui.horizontal(|ui| {
                                let ok = !app.state.url_input.trim().is_empty();
                                if (widgets::primary_icon(ui, Some("globe"), "Open it", ok).clicked() || enter)
                                    && ok
                                    && let Some(tool) = app.first_tool(WEBSITE_TOOLS)
                                {
                                    let url = app.state.url_input.trim().to_string();
                                    let params = app.params_for(tool, &[("url", json!(url))]);
                                    app.state.screen = Screen::Ask;
                                    app.run_tool(
                                        tool,
                                        params,
                                        Route::Chat { intro: tr("Here's what I found on that page:"), cta: app.state.backend.is_none() },
                                        Consent::NotAsked,
                                    );
                                    close = true;
                                }
                                if widgets::secondary(ui, None, "Cancel").clicked() {
                                    close = true;
                                }
                            });
                        }
                        Modal::Tutorial(i) => {
                            if let Some((_, title, blurb, steps)) = TUTORIALS.get(*i) {
                                ui.label(theme::heading(tr(title), theme::SECTION));
                                ui.label(RichText::new(tr(blurb)).color(t.text_dim));
                                ui.add_space(theme::space::MD);
                                for (n, s) in steps.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(24.0), egui::Sense::hover());
                                        ui.painter().circle_filled(rect.center(), 12.0, t.accent_soft);
                                        ui.painter().text(
                                            rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            (n + 1).to_string(),
                                            theme::semibold(theme::CAPTION),
                                            t.accent_ink,
                                        );
                                        ui.add(egui::Label::new(tr(s)).wrap());
                                    });
                                    ui.add_space(theme::space::XS);
                                }
                                ui.add_space(theme::space::MD);
                                ui.horizontal(|ui| {
                                    if widgets::primary_icon(ui, Some("folder-open"), "Open something", true).clicked() {
                                        app.pick(Pick::OpenFile);
                                        close = true;
                                    }
                                    if widgets::secondary(ui, None, "Close").clicked() {
                                        close = true;
                                    }
                                });
                            }
                        }
                        Modal::Engine(b) => crate::engine_ui::modal(app, ui, *b, &mut close),
                        Modal::Assets => crate::engine_ui::assets_modal(app, ui, &mut close),
                        Modal::Ghidra => crate::engine_ui::ghidra_modal(app, ui, &mut close),
                        Modal::GamePermission => crate::game_files::permission_modal(app, ui, &mut close),
                        Modal::DeepMode => crate::engine_ui::deep_modal(app, ui, &mut close),
                        Modal::ExistingFiles => crate::folders::existing_modal(app, ui, &mut close),
                        Modal::NeedBuilder { start } => crate::builders::need_builder_modal(app, ui, *start, &mut close),
                        Modal::Rename(root) => crate::manage::rename_modal(app, ui, root, &mut close),
                        Modal::Delete(root) => crate::manage::delete_modal(app, ui, root, &mut close),
                        Modal::Pin => crate::pins::pin_modal(app, ui, &mut close),
                        Modal::PinRename(root, id) => crate::pins::rename_modal(app, ui, root, *id, &mut close),
                        Modal::PinDelete(root, id) => crate::pins::delete_modal(app, ui, root, *id, &mut close),
                        Modal::About => {
                            ui.label(theme::heading("Craft Anything", theme::SECTION));
                            ui.label(RichText::new(format!("{} {}", tr("Version"), env!("CARGO_PKG_VERSION"))).color(t.text_dim));
                            ui.add_space(theme::space::MD);
                            if widgets::primary(ui, "Close").clicked() {
                                close = true;
                            }
                        }
                    }
                });
            // The layout regression checks the dialog's own viewport, not only the window.
            ui.data_mut(|d| d.insert_temp(egui::Id::new("craft-modal-clip"), scroll.inner_rect));
        });
    if resp.should_close() {
        dismiss(app);
    } else if close {
        app.state.modal = None;
    }
}

/// The width a modal body asks for; the scroll area around it must reserve the same width.
fn modal_width(modal: &Modal) -> f32 {
    match modal {
        Modal::ExistingFiles | Modal::Pin => 560.0,
        Modal::Engine(_) | Modal::Assets | Modal::Ghidra | Modal::NeedBuilder { .. } => 520.0,
        _ => 440.0,
    }
}

/// Close the dialog without choosing (Esc, a click outside). With nothing downloading, the
/// project waiting on it doesn't start later by itself.
pub fn dismiss(app: &mut CraftApp) {
    if app.ghidra.job.is_none() {
        app.ghidra.start_after = None;
    }
    if app.engine_flow.job.is_none() {
        app.engine_flow.start_after = false;
    }
    app.state.modal = None;
}
