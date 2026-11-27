//! Render the real Settings modal in both modes. Locale changes stay in one test (the catalog is global).
//! Optional evidence: CRAFT_SETTINGS_SHOTS=target/settings-after cargo test -p
//! craft-anything-ui-egui --test responsive_settings -- --nocapture

use craft_anything_engine::Mode;
use craft_anything_ui_egui::state::SettingsTab;
use craft_anything_ui_egui::{CraftApp, demo, i18n, settings_ui, theme};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const TABS: [(SettingsTab, &str, &str); 7] = [
    (SettingsTab::General, "General", "Mode"),
    (SettingsTab::Engines, "Engines", "Clean room / Deep"),
    (SettingsTab::Ai, "AI", "Claude"),
    (SettingsTab::Builds, "Builds", "Good enough"),
    (SettingsTab::Plugins, "Plugins", "No plugins yet."),
    (SettingsTab::Safety, "Safety", "Ask before running anything"),
    (SettingsTab::About, "About", "Craft Anything"),
];

fn harness(mode: Mode, size: egui::Vec2, zoom: f32, lang: &str) -> Harness<'static, CraftApp> {
    let mut app = demo::app(demo::engine(mode, true));
    app.settings.language = lang.into();
    app.settings.projects_dir = Some("/samples/projects with a deliberately long name for wrapping".into());
    app.state.settings_open = true;
    app.detected = Some(demo::detected());
    i18n::set_language(lang);
    let mut first = true;
    let mut h = Harness::builder().with_size(size).with_max_steps(64).build_ui_state(
        move |ui, app| {
            if first {
                CraftApp::setup_context(ui.ctx());
                ui.ctx().set_zoom_factor(zoom);
                first = false;
                // Font definitions bind at the start of the next pass.
                return;
            }
            theme::apply(ui.ctx(), app.theme_kind());
            settings_ui::show(app, ui.ctx());
        },
        app,
    );
    h.run_steps(4);
    h
}

fn modal(h: &Harness<'_, CraftApp>) -> egui::Rect {
    h.ctx.memory(|m| m.area_rect(egui::Id::new("settings-window"))).expect("Settings area")
}

fn screenshot(h: &mut Harness<'_, CraftApp>, name: &str) {
    if let Some(dir) = std::env::var_os("CRAFT_SETTINGS_SHOTS") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let image = h.render().expect("render Settings");
        image.save(dir.join(format!("{:?}-{name}.png", h.state().settings.mode))).unwrap();
    }
}

#[test]
fn settings_fits_and_remains_usable_in_each_viewport_and_language() {
    let mut review_failures = Vec::new();
    for mode in [Mode::Beginner, Mode::Experienced] {
        focused_about_survives_a_round_trip(mode, &mut review_failures);
        path_draft_survives_row_layout_changes(mode, &mut review_failures);
        configured_model_choices_fit(mode, &mut review_failures);
    }
    for mode in [Mode::Beginner, Mode::Experienced] {
        let mut failures = Vec::new();
        widget_focus_survives_orientation_changes(mode, &mut failures);
        drafts_survive_resizing(mode, &mut failures);
        focused_tabs_survive_resizing(mode, &mut failures);
        server_model_choices_fit(mode, &mut failures);
        translated_key_rows_fit(mode, &mut failures);
        repeated_errors_return_into_view(mode, &mut failures);
        settings_errors_have_one_visible_copy(mode, &mut failures);
        // The harness receives window points at native scale 1; egui's real zoom then halves
        // the small viewport to 380×260 logical points. Rendered PNGs remain 760×520 pixels.
        for lang in ["en", "tr", "de"] {
            for (name, size, zoom) in
                [("small-2x", egui::vec2(760.0, 520.0), 2.0), ("normal", egui::vec2(900.0, 600.0), 1.0), ("desktop", egui::vec2(1440.0, 900.0), 1.0)]
            {
                let mut h = harness(mode, size, zoom, lang);
                let viewport = h.ctx.content_rect();
                assert_eq!(viewport.size(), size / zoom);
                for (tab, label, content) in TABS {
                    // Select every page even on the broken layout, collecting all failures before
                    // failing. The interaction checks below exercise actual clicks and keyboard input.
                    h.state_mut().state.settings_tab = tab;
                    h.run_steps(4);
                    h.get_by_label(&i18n::tr(content));
                    let dialog = modal(&h);
                    if !viewport.expand(0.5).contains_rect(dialog) {
                        failures.push(format!("{lang}/{name}/{label}: modal {dialog:?} exceeds {viewport:?}"));
                    }
                    let selected = h.get_by_label(&i18n::tr(label)).rect();
                    if !dialog.contains_rect(selected) {
                        failures.push(format!("{lang}/{name}/{label}: selected tab {selected:?} is outside {dialog:?}"));
                    }
                    if name == "desktop" && (dialog.width() > 900.0 || dialog.height() > 700.0) {
                        failures.push(format!("{lang}/{name}/{label}: desktop dialog lost its usual proportions: {dialog:?}"));
                    }
                    for key in ["Settings", "Close"] {
                        // Turkish also calls an AI's Off action "Kapat"; the header comes first.
                        let rect = h.get_all_by_label(&i18n::tr(key)).next().expect("header control").rect();
                        if !viewport.contains_rect(rect) {
                            failures.push(format!("{lang}/{name}/{label}: {key} {rect:?} is offscreen"));
                        }
                    }
                    for role in [
                        egui::accesskit::Role::Button,
                        egui::accesskit::Role::CheckBox,
                        egui::accesskit::Role::TextInput,
                        egui::accesskit::Role::ComboBox,
                        egui::accesskit::Role::SpinButton,
                    ] {
                        for node in h.query_all_by_role(role) {
                            let rect = node.rect();
                            if TABS.iter().any(|(_, title, _)| h.query_all_by_label(&i18n::tr(title)).any(|tab| tab.rect() == rect)) {
                                continue;
                            }
                            if rect.left() < dialog.left() - 0.5 || rect.right() > dialog.right() + 0.5 {
                                failures.push(format!("{lang}/{name}/{label}: {role:?} {rect:?} exceeds dialog horizontally"));
                            }
                        }
                    }
                    // Controls and prose must fit horizontally even when their page needs vertical
                    // scrolling. Inspect the real painted text, including wrapped translated blurbs.
                    for shape in &h.output().shapes {
                        if let egui::Shape::Text(text) = &shape.shape {
                            // Offscreen navigation labels are legitimate inside a scrolling rail.
                            if TABS.iter().any(|(_, title, _)| i18n::tr(title) == text.galley.text()) {
                                continue;
                            }
                            let rect = text.galley.rect.translate(text.pos.to_vec2());
                            if rect.right() > dialog.right() + 0.5 || rect.left() < dialog.left() - 0.5 {
                                failures.push(format!("{lang}/{name}/{label}: text {:?} exceeds dialog horizontally", text.galley.text()));
                            }
                        }
                    }
                    screenshot(&mut h, &format!("{lang}-{name}-{label}"));
                }
            }
        }
        i18n::set_language("en");
        assert!(failures.is_empty(), "{mode:?}: {} layout failures:\n{}", failures.len(), failures.join("\n"));
        navigation_and_editing(mode);
        longer_content_and_resize(mode);
        eprintln!("{mode:?}: 63 viewport/tab/locale cases plus navigation, forms, errors and resize regressions passed");
    }
    assert!(review_failures.is_empty(), "{} review failures:\n{}", review_failures.len(), review_failures.join("\n"));
}

fn widget_focus_survives_orientation_changes(mode: Mode, failures: &mut Vec<String>) {
    let wide = theme::breakpoint::SETTINGS_RAIL + 2.0 * theme::space::XXL + 1.0;
    let narrow = wide - 2.0;
    let mut h = harness(mode, egui::vec2(wide, 600.0), 1.0, "en");
    h.state_mut().state.settings_tab = SettingsTab::Ai;
    h.run_steps(4);
    h.get_by_label("Requests per minute").scroll_to_me();
    h.run_steps(4);
    h.get_by_label("Requests per minute").focus();
    h.run_steps(2);
    h.get_by_label("Requests per minute").type_text("137");
    h.run_steps(2);
    h.key_press(egui::Key::Home);
    h.run_steps(2);
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(2);
    for (width, typed, expected) in [(narrow, "4", "1437"), (wide, "2", "14237")] {
        h.set_size(egui::vec2(width, 600.0));
        h.run_steps(4);
        if !h.get_by_label("Requests per minute").is_focused() {
            failures.push(format!("review/input-focus/{width}: request-rate input lost focus"));
        }
        // type_text sends input only; it does not focus the widget.
        h.get_by_label("Requests per minute").type_text(typed);
        h.run_steps(3);
        if h.get_by_label("Requests per minute").value().as_deref() != Some(expected) {
            failures.push(format!("review/input-cursor/{width}: typing did not continue at the retained cursor"));
        }
    }
    screenshot(&mut h, "review-request-rate-cursor");
    for (tab, label, _) in TABS {
        let mut h = harness(mode, egui::vec2(wide, 600.0), 1.0, "en");
        let other = if tab == SettingsTab::General { SettingsTab::Safety } else { SettingsTab::General };
        h.state_mut().state.settings_tab = other;
        h.run_steps(3);
        h.get_by_label(label).focus();
        h.run_steps(3);
        assert!(h.get_by_label(label).is_focused());
        for (name, width) in [("narrow", narrow), ("wide", wide)] {
            h.state_mut().state.settings_tab = other;
            h.run_steps(3);
            h.set_size(egui::vec2(width, 600.0));
            h.run_steps(4);
            if !h.get_by_label(label).is_focused() || !modal(&h).contains_rect(h.get_by_label(label).rect()) {
                failures.push(format!("review/tab-orientation/{label}/{name}: focused tab was lost or hidden"));
            }
            h.key_press(egui::Key::Enter);
            h.run_steps(3);
            if h.state().state.settings_tab != tab {
                failures.push(format!("review/tab-activation/{label}/{name}: Enter did not activate the focused tab"));
            }
            if tab == SettingsTab::About {
                screenshot(&mut h, &format!("review-oriented-tab-About-{name}"));
            }
        }
    }
}

fn drafts_survive_resizing(mode: Mode, failures: &mut Vec<String>) {
    let wide = theme::breakpoint::SETTINGS_RAIL + 2.0 * theme::space::XXL + 1.0;
    let narrow = wide - 2.0;
    let mut h = harness(mode, egui::vec2(wide, 600.0), 1.0, "en");
    h.state_mut().state.settings_tab = SettingsTab::Ai;
    h.run_steps(4);
    h.get_by_label("Requests per minute").scroll_to_me();
    h.run_steps(4);
    h.get_by_label("Requests per minute").focus();
    h.get_by_label("Requests per minute").type_text("137");
    h.run_steps(3);
    assert_eq!(h.get_by_label("Requests per minute").value().as_deref(), Some("137"));
    for width in [narrow, wide] {
        h.set_size(egui::vec2(width, 600.0));
        h.run_steps(4);
        if h.get_by_label("Requests per minute").value().as_deref() != Some("137") {
            failures.push(format!("review/resize-draft/{width}: request-rate draft disappeared"));
        }
    }
    screenshot(&mut h, "review-request-rate-draft");
    h.state_mut().toast("A save failed while editing a request rate.", true);
    h.run_steps(4);
    assert_eq!(h.get_by_label("Requests per minute").value().as_deref(), Some("137"), "new errors preserve drafts too");
    h.get_by_label("Apply").scroll_to_me();
    h.run_steps(4);
    h.get_by_label("Apply").click();
    h.run_steps(4);
    assert!(h.get_by_label("Requests per minute").accesskit_node().is_disabled(), "the pending save disables its row");
    // This focused harness does not poll jobs: their pending UI state must survive the resize.
    for width in [narrow, wide] {
        h.set_size(egui::vec2(width, 600.0));
        h.run_steps(4);
        if !h.get_by_label("Requests per minute").accesskit_node().is_disabled() {
            failures.push(format!("review/resize-pending/{width}: a pending save became editable"));
        }
    }
}

fn focused_tabs_survive_resizing(mode: Mode, failures: &mut Vec<String>) {
    for (selected, focused) in
        [(SettingsTab::General, "About"), (SettingsTab::Safety, "About"), (SettingsTab::About, "General"), (SettingsTab::About, "Safety")]
    {
        let mut h = harness(mode, egui::vec2(1440.0, 900.0), 1.0, "en");
        h.state_mut().state.settings_tab = selected;
        h.run_steps(4);
        h.get_by_label(focused).focus();
        h.run_steps(4);
        assert!(h.get_by_label(focused).is_focused());
        h.set_size(egui::vec2(900.0, 260.0));
        h.run_steps(4);
        assert!(h.get_by_label(focused).is_focused(), "resizing preserves keyboard focus");
        assert_eq!(h.state().state.settings_tab, selected, "focus does not activate a tab");
        if !modal(&h).contains_rect(h.get_by_label(focused).rect()) {
            failures.push(format!("review/resize-focus/{selected:?}: focused {focused} tab is outside Settings"));
        }
        // Adjacent selected/focused tabs can both fit in the shortened rail.
        if selected == SettingsTab::Safety || focused == "Safety" {
            let label = if selected == SettingsTab::Safety { "Safety" } else { "About" };
            if !modal(&h).contains_rect(h.get_by_label(label).rect()) {
                failures.push(format!("review/resize-focus: selected {label} tab is outside Settings"));
            }
        }
        screenshot(&mut h, &format!("review-focused-tab-{selected:?}-{focused}"));
        h.get_by_label("General").scroll_to_me();
        h.run_steps(4);
        let scrolled = h.get_by_label("General").rect();
        assert!(modal(&h).contains_rect(scrolled), "manual scrolling remains available with a focused tab");
        h.run_steps(4);
        assert_eq!(h.get_by_label("General").rect(), scrolled, "ordinary frames do not pull the rail back");
    }
}

fn server_model_choices_fit(mode: Mode, failures: &mut Vec<String>) {
    for lang in ["en", "tr", "de"] {
        for (name, size, zoom) in [("small", egui::vec2(760.0, 520.0), 2.0), ("desktop", egui::vec2(1440.0, 900.0), 1.0)] {
            let mut h = harness(mode, size, zoom, lang);
            h.state_mut().state.settings_tab = SettingsTab::Ai;
            h.run_steps(4);
            let original = modal(&h);
            let long = format!("local/{}", "a-very-long-model-name-".repeat(10));
            let form = &mut h.state_mut().server_form;
            form.address = "http://127.0.0.1:11434/v1".into();
            form.choices = Some((form.address.clone(), vec![long.clone(), "small-model".into()]));
            form.model = long.clone();
            form.picked = Some((form.address.clone(), long.clone()));
            h.run_steps(4);
            let label = i18n::tr("Choose a model");
            h.get_by_label(&label).scroll_to_me();
            h.run_steps(4);
            let rect = h.get_by_label(&label).rect();
            if !h.ctx.content_rect().contains_rect(modal(&h)) || !modal(&h).contains_rect(rect) {
                failures.push(format!("review/model/{lang}: populated dropdown {rect:?}, modal {:?}, viewport {:?}", modal(&h), h.ctx.content_rect()));
            }
            assert_eq!(modal(&h), original, "populated model choices must not resize Settings");
            screenshot(&mut h, &format!("review-{lang}-{name}-server-model"));
            if h.ctx.content_rect().contains_rect(rect) {
                h.get_by_label(&label).click();
                h.run_steps(4);
                for model in [long.as_str(), "small-model"] {
                    let choice = h.get_by_label(model).rect();
                    if !h.ctx.content_rect().contains_rect(choice) {
                        failures.push(format!("review/model-popup/{lang}/{name}: choice {choice:?} exceeds {:?}", h.ctx.content_rect()));
                    }
                }
                screenshot(&mut h, &format!("review-{lang}-{name}-server-model-menu"));
                h.get_by_label("small-model").click();
                h.run_steps(3);
                assert_eq!(h.state().server_form.model, "small-model");
                h.get_by_label(&label).click();
                h.run_steps(4);
                h.get_by_label(&long).click();
                h.run_steps(3);
                assert_eq!(h.state().server_form.model, long, "ellipsis must not change the saved model identifier");
                assert_eq!(h.state().server_form.picked.as_ref().unwrap().1, long);
            }
        }
    }
}

fn translated_key_rows_fit(mode: Mode, failures: &mut Vec<String>) {
    for lang in ["en", "tr", "de"] {
        for width in [360.0, 399.0, 400.0, 410.0, 430.0, 500.0] {
            let app = demo::app(demo::engine(mode, true));
            i18n::set_language(lang);
            let mut first = true;
            let mut h = Harness::builder().with_size(egui::vec2(width + 16.0, 500.0)).build_ui_state(
                move |ui, app| {
                    if first {
                        CraftApp::setup_context(ui.ctx());
                        first = false;
                        return;
                    }
                    theme::apply(ui.ctx(), app.theme_kind());
                    ui.set_width(width);
                    let bounds = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(width, 480.0));
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("key-row-bounds"), bounds));
                    craft_anything_ui_egui::engine_ui::assets_settings(app, ui);
                },
                app,
            );
            h.run_steps(4);
            let bounds = h.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new("key-row-bounds"))).unwrap();
            let save = h.get_by_label(&i18n::tr("Save")).rect();
            if !bounds.contains_rect(save) {
                failures.push(format!("review/key/{lang}/{width}: Save {save:?} exceeds {bounds:?}"));
            }
            let key = h.get_by_label(&i18n::trf("{name} key", &[("name", "fal.ai")])).rect();
            let required = key.width() + h.ctx.style_of(egui::Theme::Dark).spacing.item_spacing.x + save.width();
            let horizontal = (save.center().y - key.center().y).abs() < 1.0;
            if horizontal != (required <= width) {
                failures.push(format!("review/key-fit/{lang}/{width}: horizontal={horizontal}, rendered row needs {required}"));
            }
            if width == 400.0 {
                screenshot(&mut h, &format!("review-{lang}-key-400"));
            }
        }
    }
}

fn repeated_errors_return_into_view(mode: Mode, failures: &mut Vec<String>) {
    let mut h = harness(mode, egui::vec2(760.0, 520.0), 2.0, "en");
    let error = "Couldn't save your settings. A deliberately long save error. ".repeat(10);
    h.state_mut().toast(error.clone(), true);
    h.run_steps(4);
    h.get_by_label("Reset").scroll_to_me();
    h.run_steps(4);
    let scrolled = h.get_by_label(&error).rect().top();
    h.run_steps(4);
    assert_eq!(h.get_by_label(&error).rect().top(), scrolled, "ordinary frames preserve scrolling");
    h.state_mut().toast("A confirmation", false);
    h.run_steps(4);
    assert_eq!(h.get_by_label(&error).rect().top(), scrolled, "confirmations do not reset the error page");
    h.state_mut().toast(error.clone(), true);
    h.run_steps(4);
    if h.get_by_label(&error).rect().top() < modal(&h).top() {
        failures.push("review/repeated-error: an identical new save failure stayed above the visible page".into());
    }
    assert_eq!(h.state().state.toasts.iter().filter(|(_, error)| *error).count(), 1, "repeated errors remain deduplicated");
    screenshot(&mut h, "review-repeated-error");
    h.get_by_label("Reset").scroll_to_me();
    h.run_steps(4);
    let scrolled = h.get_by_label(&error).rect().top();
    h.run_steps(4);
    assert_eq!(h.get_by_label(&error).rect().top(), scrolled, "scrolling after a repeated failure stays usable");
}

fn settings_errors_have_one_visible_copy(mode: Mode, failures: &mut Vec<String>) {
    let error = "Couldn't save your settings. This error belongs inside Settings.".repeat(5);
    let mut h = Harness::builder().with_size(egui::vec2(760.0, 520.0)).with_max_steps(64).build_eframe(move |cc| {
        CraftApp::setup_context(&cc.egui_ctx);
        let mut app = demo::app(demo::engine(mode, true));
        app.settings.projects_dir = Some("/samples/projects".into());
        app.state.settings_open = true;
        app
    });
    h.ctx.set_zoom_factor(2.0);
    h.run_steps(4);
    assert_eq!(h.ctx.content_rect().size(), egui::vec2(380.0, 260.0));
    assert!(h.state().state.toasts.is_empty());
    // Exercise delivery after Settings is already on screen, through the same entry point
    // used by failed background jobs. The full app must paint only the bounded page copy.
    let short = "Couldn't save your settings. Destination is read-only.";
    h.state_mut().toast(short, true);
    h.run_steps(4);
    assert_eq!(h.query_all_by_label(short).count(), 1);
    assert!(modal(&h).contains_rect(h.get_by_label(short).rect()));
    h.get_by_label("Dismiss").scroll_to_me();
    h.run_steps(4);
    assert!(modal(&h).contains_rect(h.get_by_label("Dismiss").rect()));
    h.get_by_label("Dismiss").click();
    h.run_steps(4);
    assert!(h.state().state.toasts.is_empty(), "in-page dismissal removes the original error");
    assert!(h.query_by_label(short).is_none());

    h.state_mut().toast(error.clone(), true);
    h.run_steps(4);
    assert_eq!(h.state().state.toasts, vec![(error.clone(), true)], "delivery preserves the complete original message");
    let dialog = modal(&h);
    let close = h.query_all_by_label("Close").map(|node| node.rect()).find(|rect| dialog.contains_rect(*rect)).expect("Settings Close");
    let header_bottom = close.bottom();
    screenshot(&mut h, "review-full-app-error");
    let mut painted = 0;
    for shape in &h.output().shapes {
        if let egui::Shape::Text(text) = &shape.shape
            && text.galley.text() == error
        {
            painted += 1;
            // A vertical ScrollArea clips vertically; its horizontal clip is inherited.
            // Check the actual visible ink, rather than requiring egui to narrow both axes.
            assert!(shape.clip_rect.bottom() <= dialog.bottom(), "the page clips long errors above the modal edge");
            assert!(shape.clip_rect.top() > header_bottom, "the error never paints over the heading or Close");
            let visible = text.galley.rect.translate(text.pos.to_vec2()).intersect(shape.clip_rect);
            assert!(visible.is_positive(), "an error generated while Settings is open remains visible");
            assert!(dialog.contains_rect(visible), "all visible error text stays inside Settings");
        }
    }
    assert_eq!(painted, 1, "no duplicate overlay paints over Settings controls");
    assert!(h.ctx.content_rect().contains_rect(close));
    let copies = h.query_all_by_label(&error).count();
    if copies != 1 {
        failures.push(format!("review/duplicate-error: the full app renders {copies} copies while Settings is open"));
    }
    h.state_mut().toast("A confirmation", false);
    h.run_steps(4);
    assert_eq!(h.query_all_by_label("A confirmation").count(), 1, "confirmation overlays still render");
    assert_eq!(h.query_all_by_label(&error).count(), 1);
    h.state_mut().state.toasts.retain(|(_, error)| *error);
    h.state_mut().state.settings_open = false;
    h.run_steps(4);
    assert_eq!(h.query_all_by_label(&error).count(), 1, "the stored error is shown normally outside Settings");
    assert_eq!(h.state().state.toasts.len(), 1);
    h.get_by_label("Dismiss").click();
    h.run_steps(4);
    assert!(h.state().state.toasts.is_empty(), "normal overlay dismissal still works");
    let outside = "An operation failed outside Settings.";
    h.state_mut().toast(outside, true);
    h.run_steps(4);
    assert_eq!(h.query_all_by_label(outside).count(), 1);
    h.get_by_label("Dismiss").click();
    h.run_steps(4);
    assert!(h.state().state.toasts.is_empty());
}

fn longer_content_and_resize(mode: Mode) {
    let mut h = harness(mode, egui::vec2(760.0, 520.0), 2.0, "en");
    h.state_mut().settings.plugins = serde_json::from_value(serde_json::json!({
        "mcp": [{"id": "docs", "name": "Documentation tools with a deliberately long display name", "command": "/samples/tools/documentation-helper-with-a-long-name"}],
        "wasm": [{"id": "sample", "name": "Sample component with another long display name", "path": "/samples/plugins/a-long-component-name.wasm"}]
    })).unwrap();
    h.state_mut().state.settings_tab = SettingsTab::Plugins;
    h.run_steps(4);
    let initial = modal(&h);
    assert!(h.ctx.content_rect().contains_rect(initial));
    for label in ["Read its tools", "Plugin id", "Plugin command or address", "Plugin arguments"] {
        h.get_by_label(label).scroll_to_me();
        h.run_steps(4);
        assert!(modal(&h).contains_rect(h.get_by_label(label).rect()), "{label} must be fully reachable");
        assert_eq!(modal(&h), initial, "scrolling a populated plugin page must not resize Settings");
    }
    screenshot(&mut h, "en-small-2x-plugin-form");
    let error = "A settings save failed. Please check the destination and try again. ".repeat(12);
    h.state_mut().toast(error.clone(), true);
    h.run_steps(4);
    assert!(h.ctx.content_rect().contains_rect(modal(&h)));
    assert!(modal(&h).contains_rect(h.get_by_label("Close").rect()), "a long error leaves Close in view");
    assert!(h.get_by_label(&error).rect().top() < modal(&h).bottom(), "a new save error starts in view");
    h.get_by_label("Dismiss").scroll_to_me();
    h.run_steps(4);
    assert!(modal(&h).contains_rect(h.get_by_label("Dismiss").rect()));
    h.get_by_label("Dismiss").click();
    h.run_steps(3);
    assert!(h.state().state.toasts.is_empty());
    // Resize the same open modal and exercise the vertical rail in a wide, short viewport.
    h.ctx.set_zoom_factor(1.0);
    h.run_steps(2);
    // Let egui apply zoom before supplying the new logical screen rect: applying both in
    // one frame makes egui's zoom adjustment replace the harness's explicit resize.
    h.set_size(egui::vec2(900.0, 260.0));
    h.run_steps(4);
    assert_eq!(h.ctx.content_rect().size(), egui::vec2(900.0, 260.0));
    h.get_by_label("About").scroll_to_me();
    h.run_steps(4);
    assert!(modal(&h).contains_rect(h.get_by_label("About").rect()));
    h.get_by_label("About").click();
    h.run_steps(3);
    assert_eq!(h.state().state.settings_tab, SettingsTab::About);
    assert!(h.ctx.content_rect().contains_rect(modal(&h)));
    screenshot(&mut h, "en-wide-short-last-tab");
    h.ctx.set_zoom_factor(2.0);
    h.run_steps(2);
    h.set_size(egui::vec2(380.0, 260.0));
    h.run_steps(4);
    assert_eq!(h.ctx.content_rect().size(), egui::vec2(380.0, 260.0));
    assert!(h.ctx.content_rect().contains_rect(modal(&h)));
    assert!(modal(&h).contains_rect(h.get_by_label("About").rect()), "selected tab stays in view after resizing");
}

fn navigation_and_editing(mode: Mode) {
    let mut h = harness(mode, egui::vec2(760.0, 520.0), 2.0, "en");
    let close = h.get_by_label("Close").rect();
    // Scroll the rail to its last tab, then back through every tab by real pointer clicks.
    h.get_by_label("About").scroll_to_me();
    h.run_steps(4);
    assert!(modal(&h).contains_rect(h.get_by_label("About").rect()));
    h.get_by_label("About").click();
    h.run_steps(3);
    assert_eq!(h.state().state.settings_tab, SettingsTab::About);
    for (tab, label, content) in TABS {
        h.get_by_label(label).scroll_to_me();
        h.run_steps(4);
        h.get_by_label(label).click();
        h.run_steps(3);
        assert_eq!(h.state().state.settings_tab, tab);
        h.get_by_label(content);
        assert_eq!(h.get_by_label("Close").rect(), close, "header stays pinned on {label}");
    }
    // Tab reaches the next tab and brings it into view; Enter activates it.
    h.get_by_label("General").focus();
    h.run_steps(3);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert!(h.get_by_label("Engines").is_focused());
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().state.settings_tab, SettingsTab::Engines);
    h.get_by_label("General").scroll_to_me();
    h.run_steps(3);
    h.get_by_label("General").click();
    h.run_steps(3);
    h.get_by_label("Reset").scroll_to_me();
    h.run_steps(4);
    assert!(modal(&h).contains_rect(h.get_by_label("Reset").rect()), "the last setting scrolls into view");
    h.get_by_label("Studio Light").scroll_to_me();
    h.run_steps(4);
    assert!(modal(&h).contains_rect(h.get_by_label("Studio Light").rect()));
    h.get_by_label("Studio Light").click();
    h.run_steps(3);
    assert_eq!(h.state().settings.theme, "studio-light");
    // This focused modal harness doesn't run the app's debounce worker. Verify the changed
    // value's persisted representation under a temporary path; exit_settings covers the worker.
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    h.state().settings.save_to(&config).unwrap();
    assert_eq!(craft_anything_engine::Settings::load_from(&config).theme, "studio-light");
    assert_eq!(h.get_by_label("Close").rect(), close, "content scrolling leaves Close in place");
    screenshot(&mut h, "en-small-2x-scrolled-theme");
    h.get_by_label("General").focus();
    h.run_steps(3);
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
    h.run_steps(3);
    assert!(h.get_by_label("Close").is_focused(), "Shift+Tab from the first tab reaches Close");
    h.key_press(egui::Key::Space);
    h.run_steps(3);
    assert!(!h.state().state.settings_open);
    h.state_mut().state.settings_open = true;
    h.run_steps(3);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!h.state().state.settings_open);
    h.state_mut().state.settings_open = true;
    h.run_steps(3);
    h.get_by_label("Close").click();
    h.run_steps(3);
    assert!(!h.state().state.settings_open);
}

fn focused_about_survives_a_round_trip(mode: Mode, failures: &mut Vec<String>) {
    let mut h = harness(mode, egui::vec2(900.0, 260.0), 1.0, "en");
    assert_eq!(h.state().state.settings_tab, SettingsTab::General);
    h.set_size(egui::vec2(380.0, 260.0));
    h.run_steps(4);
    h.get_by_label("About").focus();
    h.run_steps(4);
    assert!(h.get_by_label("About").is_focused());
    h.set_size(egui::vec2(900.0, 260.0));
    h.run_steps(4);
    assert_eq!(h.ctx.content_rect().size(), egui::vec2(900.0, 260.0));
    assert_eq!(h.state().state.settings_tab, SettingsTab::General, "focus must not select About");
    if !h.get_by_label("About").is_focused() || !modal(&h).contains_rect(h.get_by_label("About").rect()) {
        failures.push(format!("{mode:?}/tab-round-trip: focused About is lost or outside the restored vertical rail"));
    }
    screenshot(&mut h, "followup-about-round-trip");
}

fn path_draft_survives_row_layout_changes(mode: Mode, failures: &mut Vec<String>) {
    let mut h = harness(mode, egui::vec2(1440.0, 900.0), 1.0, "en");
    h.state_mut().state.settings_tab = SettingsTab::Engines;
    h.run_steps(4);
    h.get_all_by_label("Ghidra folder").last().unwrap().scroll_to_me();
    h.run_steps(4);
    h.get_all_by_label("Ghidra folder").last().unwrap().focus();
    h.run_steps(2);
    h.get_all_by_label("Ghidra folder").last().unwrap().type_text("/tmp/Ghidra ");
    h.run_steps(2);
    h.key_press(egui::Key::Home);
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(2);
    let focused = h.ctx.memory(|m| m.focused()).unwrap();
    let mut cursor = egui::TextEdit::load_state(&h.ctx, focused).unwrap().cursor.char_range();
    let mut expected = "/tmp/Ghidra ".to_string();
    for (name, size, typed) in [("stacked", egui::vec2(380.0, 260.0), "a"), ("horizontal", egui::vec2(1440.0, 900.0), "b")] {
        h.set_size(size);
        h.run_steps(4);
        let input = h.get_all_by_label("Ghidra folder").last().unwrap();
        if input.value().as_deref() != Some(expected.as_str()) || !input.is_focused() {
            failures.push(format!("{mode:?}/path-{name}: lost the untrimmed draft or keyboard focus"));
        }
        let retained = h.ctx.memory(|m| m.focused()).and_then(|id| egui::TextEdit::load_state(&h.ctx, id)).and_then(|s| s.cursor.char_range());
        if retained != cursor {
            failures.push(format!("{mode:?}/path-{name}: cursor moved during resize"));
        }
        // type_text sends text without requesting focus. Insert at the retained interior cursor.
        input.type_text(typed);
        h.run_steps(3);
        let at = cursor.unwrap().primary.index;
        expected.insert_str(at.into(), typed);
        if h.get_all_by_label("Ghidra folder").last().unwrap().value().as_deref() != Some(expected.as_str()) {
            failures.push(format!("{mode:?}/path-{name}: typing did not continue at the interior cursor"));
        }
        h.get_all_by_label("Ghidra folder").last().unwrap().scroll_to_me();
        h.run_steps(4);
        assert!(modal(&h).contains_rect(h.get_all_by_label("Ghidra folder").last().unwrap().rect()));
        screenshot(&mut h, &format!("followup-ghidra-{name}"));
        // Remember the updated cursor for the next orientation transition.
        cursor = egui::TextEdit::load_state(&h.ctx, focused).unwrap().cursor.char_range();
    }
}

fn configured_model_choices_fit(mode: Mode, failures: &mut Vec<String>) {
    let long = format!("local/{}", "configured-model-name-".repeat(10));
    let other = format!("local/{}", "another-complete-model-name-".repeat(9));
    let mut h = harness(mode, egui::vec2(760.0, 520.0), 2.0, "en");
    h.state()
        .hub
        .add_backend(
            craft_anything_ai::BackendConfig {
                name: "Box".into(),
                kind: craft_anything_ai::BackendKind::OpenaiCompatible,
                model: long.clone(),
                base_url: Some("http://127.0.0.1:11434/v1".into()),
                ssh_host: None,
                enabled: true,
            },
            None,
        )
        .unwrap();
    h.state_mut().backends = h.state().hub.backends();
    h.state_mut().state.settings_tab = SettingsTab::Ai;
    h.run_steps(4);
    let initial = modal(&h);
    h.state_mut().model_picker = Some(("Box".into(), Some(vec![long.clone(), other.clone(), "small-model".into()])));
    h.run_steps(4);
    assert_eq!(h.ctx.content_rect().size(), egui::vec2(380.0, 260.0));
    h.get_by_value(&long).scroll_to_me();
    h.run_steps(4);
    if modal(&h) != initial || !h.ctx.content_rect().contains_rect(modal(&h)) || !modal(&h).contains_rect(h.get_by_value(&long).rect()) {
        failures.push(format!("{mode:?}/configured-model: selected value widens or leaves Settings"));
    }
    screenshot(&mut h, "followup-configured-model");
    // Open by keyboard even on the broken layout, where the node's center is offscreen.
    h.get_by_value(&long).focus();
    h.run_steps(2);
    h.key_press(egui::Key::Space);
    h.run_steps(4);
    for model in [&long, &other] {
        if let Some(choice) = h.query_by_label(model) {
            if !h.ctx.content_rect().contains_rect(choice.rect()) {
                failures.push(format!("{mode:?}/configured-menu: full accessible model choice exceeds the viewport"));
            }
        } else {
            failures.push(format!("{mode:?}/configured-menu: missing complete accessible identifier"));
        }
    }
    screenshot(&mut h, "followup-configured-menu");
    if h.query_by_label(&other).is_some_and(|choice| h.ctx.content_rect().contains_rect(choice.rect())) {
        h.get_by_label(&other).click();
        h.run_steps(3);
        // The choice saves on a worker to the demo's isolated config, with no server request.
        // Wait for completion rather than assuming a fixed number of frames finishes file I/O.
        let start = std::time::Instant::now();
        while !h.state().hub.backends().iter().any(|b| b.name == "Box" && b.model == other) && start.elapsed() < std::time::Duration::from_secs(10) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(h.state().hub.backends().iter().any(|b| b.name == "Box" && b.model == other), "selection must save the complete identifier");
        assert!(h.state().model_picker.is_none());
    }
}
