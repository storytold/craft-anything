use super::*;
use crate::rebuild::tests::check_worker;

fn workspace() -> (tempfile::TempDir, Workspace) {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::create(dir.path(), "Recorded site", "Recreate this website", TargetRef::Website { url: "https://example.test/".into() }).unwrap();
    (dir, ws)
}

fn brief(ws: &Workspace) -> String {
    WebsiteKind.brief_sections(&BriefCx { ws, deep: false, ghidra: false, editor: None, assets: false, image_model: None, sees_pictures: true }).join("\n")
}

fn record(ws: &Workspace, name: &str, bytes: &[u8]) -> PathBuf {
    let path = ws.capture_dir().join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    path
}

fn rejected(ws: &Workspace, name: &str) {
    let note = brief(ws);
    assert!(note.contains("can't read the website study") && note.contains(name), "the builder must see the unreadable inventory: {note}");
    assert!(note.contains("Keep the recorded files"), "the builder must know how to recover without discarding the study: {note}");
    let error = check_worker(ws, false).run_checks(ws, &WebsiteKind, 1).unwrap_err();
    assert!(error.message.contains(name), "the actual check must reject the same record: {error:?}");
}

#[test]
fn malformed_routes_are_reported_without_rewriting_them() {
    let (_dir, ws) = workspace();
    let bytes = b"[{\"path\":\"/pricing\",\"slug\":\"pricing\",\"url\":";
    let path = record(&ws, ROUTES_FILE, bytes);
    rejected(&ws, ROUTES_FILE);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn malformed_state_inventories_are_reported() {
    for bytes in [b"{not JSON".as_slice(), b"{}", b"{\"states\":null}", b"{\"states\":[false]}"] {
        let (_dir, ws) = workspace();
        let path = record(&ws, "states-390/states.json", bytes);
        rejected(&ws, "states.json");
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn an_existing_state_directory_requires_its_inventory() {
    let (_dir, ws) = workspace();
    std::fs::create_dir_all(ws.capture_dir().join(STATES_DIR)).unwrap();
    rejected(&ws, "states.json");
}

#[test]
fn non_file_inventories_are_not_empty_studies() {
    for name in [ROUTES_FILE, "states/states.json"] {
        let (_dir, ws) = workspace();
        std::fs::create_dir_all(ws.capture_dir().join(name)).unwrap();
        rejected(&ws, name);
    }
}

#[test]
fn an_oversized_inventory_is_reported_without_rewriting_it() {
    for (name, json) in [(ROUTES_FILE, "[]"), ("states/states.json", r#"{"states":[]}"#)] {
        let (_dir, ws) = workspace();
        let mut bytes = json.as_bytes().to_vec();
        bytes.resize((1 << 20) + 1, b' ');
        let path = record(&ws, name, &bytes);
        rejected(&ws, name);
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn the_check_reports_invalid_inventory_before_replacing_earlier_results() {
    let (_dir, ws) = workspace();
    record(&ws, ROUTES_FILE, b"{broken");
    std::fs::create_dir_all(ws.app_dir()).unwrap();
    std::fs::write(ws.app_dir().join("index.html"), "<!doctype html><p>Existing build</p>").unwrap();
    let saved = ws.checks_dir().join("round-1/previous.txt");
    std::fs::create_dir_all(saved.parent().unwrap()).unwrap();
    std::fs::write(&saved, b"previous comparison").unwrap();
    let error = check_worker(&ws, false).run_checks(&ws, &WebsiteKind, 1).unwrap_err();
    assert!(error.message.contains(ROUTES_FILE), "{error:?}");
    assert_eq!(std::fs::read(saved).unwrap(), b"previous comparison");
}

#[test]
fn the_check_retry_keeps_inventory_recovery_guidance_and_recorded_files() {
    for name in [ROUTES_FILE, "states/states.json"] {
        let (_dir, ws) = workspace();
        let bytes = b"{broken";
        let path = record(&ws, name, bytes);
        std::fs::create_dir_all(ws.app_dir()).unwrap();
        std::fs::write(ws.app_dir().join("index.html"), "<!doctype html><p>Existing build</p>").unwrap();
        let saved = ws.checks_dir().join("round-1/previous.txt");
        std::fs::create_dir_all(saved.parent().unwrap()).unwrap();
        std::fs::write(&saved, b"previous comparison").unwrap();
        let mut worker = check_worker(&ws, false);
        let (tx, rx) = std::sync::mpsc::channel();
        worker.tx = tx;

        let error = worker.checks_with_a_retry_now(&ws, &WebsiteKind, 1).unwrap_err();

        assert_eq!(error.code, craft_anything_evidence::ErrorCode::BadInput);
        assert!(
            error.message.starts_with("The build is finished, but it couldn't be checked: ") && error.message.contains(&path.display().to_string()),
            "{error:?}"
        );
        assert_eq!(
            error.remediation.as_deref(),
            Some("Keep the recorded files. Restore the named record from a backup, or study the website again before comparing the build.")
        );
        let retry_notices = rx.try_iter().filter(|event| matches!(event, RunEvent::Agent(crate::AgentEvent::Status { .. }))).count();
        assert_eq!(retry_notices, 1, "the real check was tried once more before the error was returned");
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert_eq!(std::fs::read(saved).unwrap(), b"previous comparison");
        assert!(worker.preview.lock().unwrap().is_none(), "an invalid inventory must not start a preview");
    }
}

/// Drive the real retry wrapper through the existing check trait, without a browser or tool.
#[derive(Clone)]
struct RetryErrors {
    errors: Arc<Mutex<std::collections::VecDeque<ToolError>>>,
}

impl Check for RetryErrors {
    fn id(&self) -> &'static str {
        "retry-error"
    }

    fn run(&self, _cx: &mut CheckCx) -> Result<CheckResult, ToolError> {
        Err(self.errors.lock().unwrap().pop_front().expect("a check must not run more often than the retry allows"))
    }
}

impl TargetKind for RetryErrors {
    fn id(&self) -> &'static str {
        "retry-error"
    }

    fn words(&self) -> KindWords {
        WebsiteKind.words()
    }

    fn detect(&self, cx: &DetectCx) -> Option<u32> {
        WebsiteKind.detect(cx)
    }

    fn default_stack(&self, cx: &DetectCx) -> &'static str {
        WebsiteKind.default_stack(cx)
    }

    fn goal(&self) -> &'static str {
        WebsiteKind.goal()
    }

    fn checks(&self, _ws: &Workspace) -> Vec<Box<dyn Check>> {
        vec![Box::new(self.clone())]
    }
}

#[test]
fn the_check_retry_uses_generic_guidance_only_when_none_was_supplied() {
    for remediation in [None, Some("Restore the recorded inputs."), Some("")] {
        let (_dir, ws) = workspace();
        let first = ToolError::invalid("The first check failed.").with_remediation("The first attempt's guidance.");
        let mut second = ToolError::invalid("The second check failed.").with_details(serde_json::json!({"attempt": "second"}));
        second.remediation = remediation.map(str::to_owned);
        let kind = RetryErrors { errors: Arc::new(Mutex::new([first, second].into())) };

        let error = check_worker(&ws, false).checks_with_a_retry_now(&ws, &kind, 1).unwrap_err();

        assert_eq!(error.message, "The build is finished, but it couldn't be checked: The second check failed.");
        assert_eq!(error.code, craft_anything_evidence::ErrorCode::BadInput);
        assert_eq!(error.remediation.as_deref(), Some(remediation.unwrap_or("Press Try again to check it again. Nothing needs building again.")));
        assert_eq!(error.details, Some(serde_json::json!({"attempt": "second"})));
        assert!(kind.errors.lock().unwrap().is_empty(), "exactly two checks must have run");
    }
}

#[test]
fn the_check_retry_returns_cancellation_unchanged_on_either_attempt() {
    for fail_first in [false, true] {
        let (_dir, ws) = workspace();
        let mut cancelled = ToolError::cancelled().with_remediation("Resume when the recorded inputs are ready.");
        cancelled.details = Some(serde_json::json!({"check": "inventory"}));
        let mut errors = std::collections::VecDeque::new();
        if fail_first {
            errors.push_back(ToolError::invalid("The first check failed."));
        }
        errors.push_back(cancelled.clone());
        errors.push_back(ToolError::invalid("This check must not run after cancellation."));
        let kind = RetryErrors { errors: Arc::new(Mutex::new(errors)) };

        let error = check_worker(&ws, false).checks_with_a_retry_now(&ws, &kind, 1).unwrap_err();

        assert_eq!(error, cancelled);
        assert_eq!(kind.errors.lock().unwrap().len(), 1, "cancellation must stop any remaining retry");
    }
}

#[cfg(unix)]
#[test]
fn a_dangling_route_link_is_not_an_absent_optional_inventory() {
    let (_dir, ws) = workspace();
    std::fs::create_dir_all(ws.capture_dir()).unwrap();
    let path = ws.capture_dir().join(ROUTES_FILE);
    std::os::unix::fs::symlink("missing-routes.json", &path).unwrap();
    rejected(&ws, ROUTES_FILE);
    assert!(std::fs::symlink_metadata(path).unwrap().file_type().is_symlink());
}

#[test]
fn absent_optional_records_and_empty_recorded_states_still_work() {
    let (_dir, ws) = workspace();
    assert_eq!(brief(&ws), HOW_TO_RUN);
    record(&ws, "states/states.json", br#"{"width":1440,"states":[]}"#);
    assert_eq!(brief(&ws), HOW_TO_RUN);
    let checks = check_worker(&ws, false).run_checks(&ws, &WebsiteKind, 1).unwrap();
    assert_eq!(checks.len(), 1);
    assert!(!checks[0].passed && checks[0].note.contains("No built site"));
}

#[test]
fn a_route_inventory_at_the_byte_limit_is_accepted() {
    let (_dir, ws) = workspace();
    let mut bytes = b"[]".to_vec();
    bytes.resize(1 << 20, b' ');
    record(&ws, ROUTES_FILE, &bytes);
    assert_eq!(brief(&ws), HOW_TO_RUN);
}

#[test]
fn recorded_pages_and_pictured_controls_remain_in_the_brief() {
    let (_dir, ws) = workspace();
    record(&ws, ROUTES_FILE, br#"[{"path":"/pricing","slug":"pricing","url":"https://example.test/pricing"}]"#);
    record(&ws, "states/states.json", br#"{"width":1440,"states":[{"kind":"menu","label":"Products","file":"state-1.png"},{"kind":"dialog","label":"Sign in","changed":["address changed"]}]}"#);
    let note = brief(&ws);
    assert!(note.contains("/pricing (capture/routes/pricing/)"), "{note}");
    assert!(note.contains("states/state-1.png (Products, menu, 1440 px)"), "{note}");
    assert!(!note.contains("can't read") && !note.contains("Sign in"), "unpictured controls stay out of the comparison: {note}");
}
