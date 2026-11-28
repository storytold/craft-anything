//! Keep locale-changing UI checks in one integration-test executable: i18n is process-wide.

use craft_anything_engine::{Mode, Session, Settings};
use craft_anything_ui_egui::{CraftApp, demo, i18n};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::path::Path;

fn harness(mut settings: Settings, dir: &Path) -> Harness<'static, CraftApp> {
    let dir = dir.to_path_buf();
    // Keep every engine path under the test directory, including after a saved-settings reload.
    settings.engines.store_dir = Some(dir.join("engines"));
    settings.projects_dir = Some(dir.join("projects"));
    Harness::builder().with_size(egui::vec2(1280.0, 1120.0)).with_max_steps(32).build_eframe(move |cc| {
        CraftApp::setup_context(&cc.egui_ctx);
        let engine = demo::engine(Mode::Beginner, true);
        {
            // The home/settings scenario needs no sample target. Isolate the session and set
            // its language BEFORE CraftApp::new (called by demo::app) reads its settings.
            let mut session = engine.session();
            *session = Session::isolated(&dir);
            session.settings = settings;
        }
        let mut app = demo::app(engine);
        // demo::app keeps persistence disabled and uses an isolated, file-only AI key store.
        // Never enable the real settings worker in this test: Settings::save uses the global
        // config path. The roundtrip below uses save_to with an explicit temporary path.
        assert!(!app.persist);
        assert!(app.headless);
        app.state.settings_open = true;
        app
    })
}

#[test]
fn chinese_startup_language_menu_and_saved_selection() {
    let temp = tempfile::tempdir().unwrap();
    for (initial, native, other, other_native, translated_language, translated_open) in [
        ("zh-hans", "简体中文", "zh-hant", "繁體中文", "语言", "已打开 sample.exe"),
        ("zh-hant", "繁體中文", "zh-hans", "简体中文", "語言", "已開啟 sample.exe"),
    ] {
        let dir = temp.path().join(initial);
        let settings = Settings { mode: Mode::Beginner, first_run_done: true, language: initial.into(), ..Default::default() };
        let mut h = harness(settings, &dir);
        h.run_steps(3);

        // Startup must apply the saved locale itself, rather than relying on a manual call to
        // set_language after app construction. Check both lookup and a rendered settings row.
        assert_eq!(i18n::language(), initial);
        h.get_by_label(translated_language);
        assert_eq!(i18n::trf("Opened {name}", &[("name", "sample.exe")]), translated_open);
        assert_eq!(i18n::tr("A deliberately uncatalogued message"), "A deliberately uncatalogued message");
        assert_eq!(i18n::trf("Uncatalogued {name}", &[("name", "sample.exe")]), "Uncatalogued sample.exe");
        assert_eq!(craft_anything_ui_egui::usage::asset_cost_words(0.0031), if initial == "zh-hans" { "低于 $0.01" } else { "低於 $0.01" });
        let kinds = craft_anything_ai::rebuild::KindRegistry::built_in();
        for (suggestion, expected) in [("A website for my bakery", "website"), ("A simple snake game", "game")] {
            let translated = i18n::tr(suggestion);
            assert_ne!(translated, suggestion);
            let (kind, _) = kinds.choose(&translated, &craft_anything_engine::workspace::TargetRef::None, false).unwrap();
            assert_eq!(kind.id(), expected, "{initial}: {translated}");
        }
        let title = h.state().engine.registry().get("native_functions_list").unwrap().title;
        let translated_title = i18n::tr(title);
        assert_ne!(translated_title, title);
        h.state_mut().state.settings_open = false;
        h.state_mut().state.palette.open = true;
        h.state_mut().state.palette.query = translated_title.clone();
        h.run_steps(3);
        h.get_by_label(&translated_title);
        h.state_mut().state.palette.open = false;
        h.state_mut().state.settings_open = true;
        h.run_steps(3);

        // Exercise the real ComboBox and selectable row, including the language-change callback.
        h.get_by_value(native).click();
        h.run();
        h.get_by_label(other_native).scroll_to_me();
        h.run();
        let choice_rect = h.get_by_label(other_native).rect();
        h.get_by_label(other_native).click();
        h.run_steps(3);
        assert_eq!(i18n::language(), other, "language choice at {choice_rect:?}");
        assert_eq!(h.state().settings.language, other);
        let selected = h.state().engine.session().settings.clone();
        assert_eq!(selected.language, other, "the UI must propagate the choice to the engine session");

        // Verify the persisted settings representation and the next startup using only a
        // temporary config.toml. This does not claim to test the UI's debounce worker path.
        let config = dir.join("config.toml");
        selected.save_to(&config).unwrap();
        let loaded = Settings::load_from(&config);
        assert_eq!(loaded.language, other);
        drop(h);
        let mut restored = harness(loaded, &dir);
        restored.run_steps(3);
        assert_eq!(i18n::language(), other);
        restored.get_by_value(other_native);
        restored.get_by_label(if other == "zh-hans" { "语言" } else { "語言" });

        // English remains available from either Chinese catalog and takes effect immediately.
        restored.get_by_value(other_native).click();
        restored.run();
        restored.get_by_label("English").scroll_to_me();
        restored.run();
        restored.get_by_label("English").click();
        restored.run_steps(3);
        assert_eq!(i18n::language(), "en");
        assert_eq!(restored.state().settings.language, "en");
        assert_eq!(restored.state().engine.session().settings.language, "en");
        restored.get_by_label("Language");
        assert!(restored.query_by_label("语言").is_none());
        assert!(restored.query_by_label("語言").is_none());

        restored.get_by_value("English").click();
        restored.run();
        // Theme has a System choice too, on Studio's line: take the language menu's.
        let off_theme_line = |y: f32, line: f32| (y - line).abs() >= 4.0;
        let line = restored.get_by_label("Studio").rect().center().y;
        restored.query_all_by_label("System").find(|n| off_theme_line(n.rect().center().y, line)).expect("the language menu's System").scroll_to_me();
        restored.run();
        let line = restored.get_by_label("Studio").rect().center().y;
        restored.query_all_by_label("System").find(|n| off_theme_line(n.rect().center().y, line)).expect("the language menu's System").click();
        restored.run_steps(3);
        assert!(restored.state().settings.language.is_empty());
        let expected = sys_locale::get_locales().find_map(|tag| i18n::language_from_tag(&tag)).unwrap_or("en");
        assert_eq!(i18n::language(), expected);
        assert!(restored.state().engine.session().settings.language.is_empty());
    }
    for (code, native, label) in [
        ("de", "Deutsch", "Sprache"),
        ("fr", "Français", "Langue"),
        ("ja", "日本語", "言語"),
        ("ka", "ქართული", "ენა"),
        ("ko", "한국어", "언어"),
        ("vi", "Tiếng Việt", "Ngôn ngữ"),
    ] {
        let settings = Settings { mode: Mode::Beginner, first_run_done: true, language: code.into(), ..Default::default() };
        let mut existing = harness(settings, &temp.path().join(code));
        existing.run_steps(3);
        assert_eq!(i18n::language(), code);
        existing.get_by_label(label);
        existing.get_by_value(native);
    }
    i18n::set_language("unsupported-locale");
    assert_eq!(i18n::language(), "en");
    assert_eq!(i18n::tr("Settings"), "Settings");
    builder_replacement_notices(temp.path());
    rtl_main_screens_at_smallest_window(temp.path());
    ltr_screens_and_wrapped_label_unchanged(temp.path());
}

/// Run alongside the other locale checks in this test, never concurrently with them.
fn builder_replacement_notices(dir: &Path) {
    use craft_anything_ai::{AiHub, BackendConfig, BackendKind, Detected, KeyStore};
    use std::sync::Arc;

    for code in i18n::LANGUAGES.iter().map(|l| l.code).chain(["tr-TR", "unsupported-locale"]) {
        let dir = dir.join(format!("builder-{code}"));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = Settings { mode: Mode::Beginner, first_run_done: true, language: code.into(), ..Default::default() };
        let mut h = harness(settings, &dir);
        let turkish = i18n::language() == "tr";
        let notice = |saved: &str, used: &str| {
            if turkish {
                format!(
                    "Bu proje için seçilen yapay zekâ ({saved}) devre dışı bırakılmış veya artık yapılandırılmamış. Projeyi bunun yerine {used} oluşturacak."
                )
            } else {
                craft_anything_ai::rebuild::replaced_builder_notice(saved, used)
            }
        };
        assert_eq!(
            craft_anything_ai::rebuild::replaced_builder_notice("Off", "Claude"),
            "The AI chosen for this project (Off) is turned off or no longer set up. Claude builds instead.",
            "{code}: changing the UI language must not change the headless notice",
        );
        h.state_mut().detected = Some(Vec::new());
        h.state_mut().backends.push(BackendConfig {
            name: "Off".into(),
            kind: BackendKind::Anthropic,
            model: "test-model".into(),
            base_url: None,
            ssh_host: None,
            enabled: false,
        });

        // Disabled and removed choices retain both names, including Unicode and punctuation.
        for saved in ["Off", "Gone", "Önceki AI (özel)", "{used}", "{saved}"] {
            assert_eq!(h.state_mut().builder_backend(Some(saved)).as_deref(), Some("Claude"));
            assert_eq!(h.state().state.toasts.last(), Some(&(notice(saved, "Claude"), false)), "{code}");
        }

        // The Make picker uses the same localised notice and still clears its stale choice.
        h.state_mut().state.settings_open = false;
        h.state_mut().state.make.ai = Some("Off".into());
        h.run_steps(3);
        assert!(h.state().state.make.ai.is_none());
        h.get_by_label(&notice("Off", "Claude"));

        h.state_mut().state.toasts.clear();
        for wanted in [Some("Claude"), None, Some("")] {
            assert_eq!(h.state_mut().builder_backend(wanted).as_deref(), Some("Claude"));
            assert!(h.state().state.toasts.is_empty(), "{code}: no replacement");
        }
        h.state_mut().backends.clear();
        assert!(h.state_mut().builder_backend(Some("Off")).is_none());
        assert!(h.state().state.toasts.is_empty(), "{code}: nothing can build");

        // A discovered agent is announced only after its setup succeeds.
        h.state_mut().detected = Some(vec![Detected {
            suggestion: BackendConfig {
                name: "Yeni AI {saved} {used}".into(),
                kind: BackendKind::AgentCli,
                model: "claude-code".into(),
                base_url: None,
                ssh_host: None,
                enabled: true,
            },
            reason: "Fixture".into(),
            needs_key: false,
        }]);
        let hub = h.state().hub.clone();
        let blocked = dir.join("blocked");
        std::fs::write(&blocked, "a file, not a settings folder").unwrap();
        h.state_mut().hub = Arc::new(AiHub::with_stores(h.state().engine.clone(), blocked.join("config.toml"), KeyStore::file_only(dir.join("keys.toml"))));
        assert!(h.state_mut().builder_backend(Some("Off")).is_none());
        assert!(!h.state().backends.iter().any(|b| b.name == "Yeni AI {saved} {used}"));
        assert!(!h.state().state.toasts.is_empty());
        assert!(h.state().state.toasts.iter().all(|(_, error)| *error), "{code}: failed setup must not claim an agent builds");

        h.state_mut().hub = hub;
        assert_eq!(h.state_mut().builder_backend(Some("Off")).as_deref(), Some("Yeni AI {saved} {used}"));
        assert_eq!(h.state().state.toasts.last(), Some(&(notice("Off", "Yeni AI {saved} {used}"), false)), "{code}");

        // Deliver the same worker response as Just ask, with deliberately unrelated English
        // wording: the UI must use the structured names, never parse the headless sentence.
        for (saved, used) in [("Off", "Claude"), ("Önceki {used}", "Yeni {saved} {used}")] {
            h.state_mut().state.toasts.clear();
            let result = serde_json::json!({
                "project": dir,
                "question": "where?",
                "answered": {
                    "answer": "In app.", "from": "the_files", "backend": used,
                    "builder_replacement": {"saved": saved, "used": used},
                    "builder_notice": "Headless English wording is independent."
                }
            });
            h.state_mut().jobs.spawn("Question fixture", false, move |_| {
                Ok(craft_anything_ui_egui::Done::Acted { tool: "rebuild_question_ask", params: serde_json::Value::Null, result: Ok(result) })
            });
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while h.state().state.toasts.is_empty() && std::time::Instant::now() < deadline {
                h.step();
                std::thread::yield_now();
            }
            assert_eq!(h.state().state.toasts.last(), Some(&(notice(saved, used), false)), "{code}: Just ask");
        }
    }
    i18n::set_language("en");
}

const MINI_HE_CATALOG: &str = "\
Change it\tשינוי
Clean room\tחדר נקי
Continue\tהמשך
Continue where it paused\tהמשך מהנקודה שבה נעצר
Export a report (Markdown)\tייצוא דוח (Markdown)
General\tכללי
Just ask\tפשוט לשאול
Just look inside\tפשוט להציץ פנימה
Language\tשפה
Make my own version\tליצור גרסה משלי
New project\tפרויקט חדש
Plugins\tתוספים
Plugins add tools Craft Anything offers as its own: MCP servers (a command or an address; read their tools after adding) or WebAssembly components (a .wasm file, which can't touch your files or the network).\tתוספים מוסיפים כלים ש-Craft Anything מציעה כשלה: שרתי MCP (פקודה או כתובת; ניתן לקרוא את הכלים שלהם לאחר ההוספה) או רכיבי WebAssembly (קובץ .wasm, שאינו יכול לגעת בקבצים או ברשת).
Project details\tפרטי פרויקט
Settings\tהגדרות
Start\tהתחלה
";

fn smallest_harness(dir: &Path, lang: Option<(&str, &str)>, prepare: impl FnOnce(&mut CraftApp) + 'static) -> Harness<'static, CraftApp> {
    let dir = dir.to_path_buf();
    let custom = lang.map(|(code, cat)| (code.to_string(), cat.to_string()));
    Harness::builder().with_size(egui::vec2(760.0, 520.0)).with_max_steps(32).build_eframe(move |cc| {
        CraftApp::setup_context(&cc.egui_ctx);
        let engine = demo::engine(Mode::Beginner, true);
        {
            let mut session = engine.session();
            *session = Session::isolated(&dir);
            session.settings = Settings { mode: Mode::Beginner, first_run_done: true, language: "en".into(), ..Default::default() };
            session.settings.engines.store_dir = Some(dir.join("engines"));
            session.settings.projects_dir = Some(dir.join("projects"));
        }
        let mut app = demo::app(engine);
        if let Some((code, cat)) = custom {
            craft_anything_ui_egui::testing::set_custom_catalog(&code, &cat);
        } else {
            i18n::set_language("en");
        }
        prepare(&mut app);
        app
    })
}

fn assert_control_inside_range(h: &Harness<'static, CraftApp>, label: &str, min_left: f32, max_right: f32) {
    let nodes: Vec<_> = h.get_all_by_label(label).collect();
    assert!(!nodes.is_empty(), "expected control labeled {label:?}");
    for node in nodes {
        let rect = node.rect();
        assert!(rect.width() > 0.0 && rect.height() > 0.0, "empty rect for {label:?}: {rect:?}");
        assert!(rect.left() >= min_left && rect.right() <= max_right, "{label:?} overflows horizontally at {rect:?} (allowed [{min_left}, {max_right}])");
    }
}

fn assert_control_inside_window(h: &Harness<'static, CraftApp>, label: &str, width: f32) {
    assert_control_inside_range(h, label, -2.0, width + 2.0);
}

fn assert_hebrew_shapes_inside_range(h: &Harness<'static, CraftApp>, min_left: f32, max_right: f32, screen: &str) {
    let is_hebrew = |c: char| ('\u{0590}'..='\u{05FF}').contains(&c);
    let mut saw_hebrew = false;
    for clipped in &h.output().shapes {
        if let egui::Shape::Text(text) = &clipped.shape
            && text.galley.text().chars().any(is_hebrew)
        {
            saw_hebrew = true;
            let bounds = text.galley.rect.translate(text.pos.to_vec2());
            let visible_right = bounds.right().min(clipped.clip_rect.right());
            assert!(
                bounds.left() >= min_left && visible_right <= max_right,
                "{screen}: Hebrew text {:?} overflows at bounds={bounds:?}, clip={:?} (allowed [{min_left}, {max_right}])",
                text.galley.text(),
                clipped.clip_rect
            );
        }
    }
    assert!(saw_hebrew, "{screen}: expected rendered Hebrew glyphs at 760x520");
}

fn assert_hebrew_shapes_inside_window(h: &Harness<'static, CraftApp>, width: f32, screen: &str) {
    assert_hebrew_shapes_inside_range(h, -2.0, width + 2.0, screen);
}

/// Render Home, Make, Project, and Settings with an RTL language at the 760×520 minimum window
/// and verify Hebrew strings appear without horizontal overflow or clipping regressions.
fn rtl_main_screens_at_smallest_window(dir: &Path) {
    let width = 760.0;

    // 1. Home
    let mut home = smallest_harness(&dir.join("rtl-home"), Some(("he", MINI_HE_CATALOG)), |_| {});
    home.run_steps(3);
    assert!(i18n::is_rtl());
    for key in ["Make my own version", "Just look inside", "Settings", "Clean room"] {
        assert_control_inside_window(&home, &i18n::tr(key), width);
    }
    assert_hebrew_shapes_inside_window(&home, width, "rtl-home");
    drop(home);

    // 2. Make (draft populated, Start visible via scroll)
    let mut make = smallest_harness(&dir.join("rtl-make"), Some(("he", MINI_HE_CATALOG)), |app| {
        demo::fill_projects(app);
    });
    make.run_steps(3);
    make.get_all_by_label(&i18n::tr("Start")).next().unwrap().scroll_to_me();
    make.run_steps(3);
    for key in ["Make my own version", "Start"] {
        assert_control_inside_window(&make, &i18n::tr(key), width);
    }
    assert_hebrew_shapes_inside_window(&make, width, "rtl-make");
    drop(make);

    // 3. Project
    let mut project = smallest_harness(&dir.join("rtl-project"), Some(("he", MINI_HE_CATALOG)), |app| {
        demo::fill_project(app);
    });
    project.run_steps(3);
    for key in ["New project", "Project details", "Change it", "Just ask", "Settings"] {
        assert_control_inside_window(&project, &i18n::tr(key), width);
    }
    assert_hebrew_shapes_inside_window(&project, width, "rtl-project");
    drop(project);

    // 4. Settings (General + wrapped Plugins note)
    // At 760x520 the upstream General tab's unwrapped zoom hint already widens the centered Settings
    // modal slightly past x = 0 in English LTR; verify RTL does not widen or clip beyond that baseline.
    let mut ltr_settings = smallest_harness(&dir.join("ltr-settings-ref"), None, |app| {
        demo::fill_projects(app);
        app.state.settings_open = true;
    });
    ltr_settings.run_steps(3);
    let ltr_settings_left = ltr_settings.get_all_by_label("Settings").map(|n| n.rect().left()).fold(f32::INFINITY, f32::min);
    let settings_min_left = ltr_settings_left.min(0.0) - 2.0;
    drop(ltr_settings);

    let mut settings = smallest_harness(&dir.join("rtl-settings"), Some(("he", MINI_HE_CATALOG)), |app| {
        demo::fill_projects(app);
        app.state.settings_open = true;
    });
    settings.run_steps(3);
    let rtl_settings_left = settings.get_all_by_label(&i18n::tr("Settings")).map(|n| n.rect().left()).fold(f32::INFINITY, f32::min);
    assert!(rtl_settings_left >= ltr_settings_left - 2.0, "RTL widened Settings modal: rtl_left={rtl_settings_left} < ltr_left={ltr_settings_left}");
    for key in ["Settings", "General", "Plugins", "Language"] {
        assert_control_inside_range(&settings, &i18n::tr(key), settings_min_left, width + 2.0);
    }
    settings.get_all_by_label(&i18n::tr("Plugins")).next().unwrap().click();
    settings.run_steps(3);
    assert_hebrew_shapes_inside_range(&settings, settings_min_left, width + 2.0, "rtl-settings");
    // AccessKit keeps logical reading order while painted shapes render in visual RTL order
    // (opening words first, and mixed LTR runs + mirrored brackets intact).
    let plugins_logical = i18n::tr(
        "Plugins add tools Craft Anything offers as its own: MCP servers (a command or an address; read their tools after adding) or WebAssembly components (a .wasm file, which can't touch your files or the network).",
    );
    settings.get_by_label(&plugins_logical);
    let painted_texts: Vec<String> = settings
        .output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
            _ => None,
        })
        .collect();
    assert!(
        painted_texts.iter().any(|t| t.contains("MCP") && t.contains("כלים")),
        "expected visual RTL wrapped line containing Hebrew + MCP, got {painted_texts:?}"
    );
    drop(settings);

    // Command palette keeps logical title for AccessKit/search while painting visual RTL + mirrored brackets.
    let mut palette = smallest_harness(&dir.join("rtl-palette"), Some(("he", MINI_HE_CATALOG)), |app| {
        demo::fill_project(app);
        app.state.palette.open = true;
        app.state.palette.query = "ייצוא דוח".into();
    });
    palette.run_steps(3);
    palette.get_by_label("ייצוא דוח (Markdown)");
    let has_mixed_visual = palette.output().shapes.iter().any(|s| match &s.shape {
        egui::Shape::Text(t) => t.galley.text() == "(Markdown) דוח ייצוא",
        _ => false,
    });
    assert!(has_mixed_visual, "expected painted visual RTL '(Markdown) דוח ייצוא' in palette shapes");
    drop(palette);

    i18n::set_language("en");
}

fn text_shapes(h: &Harness<'static, CraftApp>) -> Vec<(String, egui::Rect)> {
    h.output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(t) => Some((t.galley.text().to_owned(), t.galley.rect.translate(t.pos.to_vec2()))),
            _ => None,
        })
        .collect()
}

/// Verify an LTR language renders identically at 760×520 (comparing shapes within this build
/// before and after visiting RTL, rather than against a separate base-commit build) and
/// `wrapped_label_for(..., false, ...)` matches plain `egui::Label::wrap()` geometry.
fn ltr_screens_and_wrapped_label_unchanged(dir: &Path) {
    let width = 760.0;

    let mut home = smallest_harness(&dir.join("ltr-home"), None, |_| {});
    home.run_steps(3);
    assert!(!i18n::is_rtl());
    for key in ["Make my own version", "Just look inside", "Settings", "Clean room"] {
        assert_control_inside_window(&home, key, width);
    }
    let home_before = text_shapes(&home);
    drop(home);

    let mut project = smallest_harness(&dir.join("ltr-project"), None, |app| {
        demo::fill_project(app);
    });
    project.run_steps(3);
    for key in ["New project", "Project details", "Change it", "Just ask", "Settings"] {
        assert_control_inside_window(&project, key, width);
    }
    drop(project);

    let mut settings = smallest_harness(&dir.join("ltr-settings"), None, |app| {
        demo::fill_projects(app);
        app.state.settings_open = true;
    });
    settings.run_steps(3);
    let ltr_min_left = settings.get_all_by_label("Settings").map(|n| n.rect().left()).fold(f32::INFINITY, f32::min).min(0.0) - 2.0;
    for key in ["Settings", "General", "Plugins", "Language"] {
        assert_control_inside_range(&settings, key, ltr_min_left, width + 2.0);
    }
    let settings_before = text_shapes(&settings);
    drop(settings);

    // Round-trip through RTL, then verify LTR shapes are identical.
    craft_anything_ui_egui::testing::set_custom_catalog("he", MINI_HE_CATALOG);
    assert!(i18n::is_rtl());
    let mut home_after_rtl = smallest_harness(&dir.join("ltr-home"), None, |_| {});
    home_after_rtl.run_steps(3);
    assert_eq!(text_shapes(&home_after_rtl), home_before, "LTR Home shapes changed after RTL");
    drop(home_after_rtl);

    let mut settings_after_rtl = smallest_harness(&dir.join("ltr-settings"), None, |app| {
        demo::fill_projects(app);
        app.state.settings_open = true;
    });
    settings_after_rtl.run_steps(3);
    assert_eq!(text_shapes(&settings_after_rtl), settings_before, "LTR Settings shapes changed after RTL");
    drop(settings_after_rtl);

    // Direct LTR geometry equivalence check against plain `egui::Label::new(...).wrap()`.
    let ctx = egui::Context::default();
    craft_anything_ui_egui::theme::apply(&ctx, craft_anything_ui_egui::theme::ThemeKind::Studio);
    let sample = "Plugins add tools Craft Anything offers as its own: MCP servers or WebAssembly components.";
    let font = egui::FontId::proportional(craft_anything_ui_egui::theme::CAPTION);
    let mut wrapped_rect = egui::Rect::NOTHING;
    let mut plain_rect = egui::Rect::NOTHING;
    ctx.run_ui(egui::RawInput::default(), |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.allocate_ui(egui::vec2(260.0, 200.0), |ui| {
                wrapped_rect = i18n::wrapped_label_for(ui, false, sample, font.clone(), |r| r).rect;
            });
            ui.allocate_ui(egui::vec2(260.0, 200.0), |ui| {
                plain_rect = ui.add(egui::Label::new(egui::RichText::new(sample).font(font.clone())).wrap()).rect;
            });
        });
    })
    .textures_delta
    .clear();
    assert!(
        (wrapped_rect.width() - plain_rect.width()).abs() < 0.01 && (wrapped_rect.height() - plain_rect.height()).abs() < 0.01,
        "LTR wrapped_label_for geometry drifted: wrapped={wrapped_rect:?} plain={plain_rect:?}"
    );
}
