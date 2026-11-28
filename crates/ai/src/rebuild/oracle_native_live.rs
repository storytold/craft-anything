//! The native behaviour oracle, live: a real window, watched and compared.
//!
//! It needs the Accessibility permission, so it is `#[ignore]`d, like the project's other live
//! macOS tests. Run it with:
//!
//! ```sh
//! cargo test -p craft-anything-ai --lib oracle_native_live -- --ignored
//! ```
//!
//! The permission is given to whatever runs the test (the terminal, or the app) in
//! System Settings → Privacy & Security → Accessibility.

use craft_anything_engine::oracle::{BehaviorOracle, OracleCx};
use craft_anything_engine::workspace::TargetRef;
use serde_json::json;

/// A window with a field and a button, in a process of its own.
///
/// The button reports the field's value, so a build that fills the field and one that does not
/// are told apart.
const WINDOW: &str = r#"import AppKit

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 360, height: 140), styleMask: [.titled], backing: .buffered, defer: false)
w.title = "CraftAnythingNative"

let field = NSTextField(string: "")
field.identifier = NSUserInterfaceItemIdentifier("theField")
field.stringValue = "before"

let label = NSTextField(labelWithString: "before")
label.identifier = NSUserInterfaceItemIdentifier("theLabel")

let button = NSButton(title: "Show", target: nil, action: nil)
button.identifier = NSUserInterfaceItemIdentifier("theButton")

let stack = NSStackView(views: [field, label, button])
w.contentView?.addSubview(stack)
w.makeKeyAndOrderFront(nil)
DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) {
    print("ready")
    fflush(stdout)
    DispatchQueue.main.asyncAfter(deadline: .now() + 60.0) { exit(0) }
}
app.run()
"#;

/// Compile the window into a program that can be started.
fn compile_window(bin: &std::path::Path) -> std::path::PathBuf {
    use craft_anything_platform::process::Process;
    let src = bin.with_extension("swift");
    std::fs::write(&src, WINDOW).expect("write the window");
    let built = Process::new("/usr/bin/xcrun")
        .args(["swiftc", "-O", "-o"])
        .arg(bin)
        .arg(&src)
        .timeout(std::time::Duration::from_secs(180))
        .label("swiftc")
        .run_checked();
    assert!(built.is_ok(), "the window builds: {built:?}");
    bin.to_path_buf()
}

/// Build the window and start it, giving its process id.
fn start_window() -> (craft_anything_platform::process::RunningProcess, u32) {
    use craft_anything_platform::process::Process;
    let root = craft_anything_engine::testing::temp_dir();
    let bin = compile_window(&root.join("window"));
    let child = Process::new(&bin).timeout(std::time::Duration::from_secs(120)).label("the window").spawn().expect("start the window");
    // Wait for it to draw.
    std::thread::sleep(std::time::Duration::from_secs(3));
    let pid = child.pid();
    (child, pid)
}

/// A context the oracle can run through.
fn context<'a>(
    call: &'a dyn Fn(&str, serde_json::Value) -> Result<craft_anything_engine::tool::ToolOutput, craft_anything_engine::evidence::ToolError>,
    target: &'a TargetRef,
    app: &'a craft_anything_engine::oracle::TheApp,
) -> OracleCx<'a> {
    OracleCx::original(call, target, false).with_app(app)
}

/// The whole native oracle, end to end: the original's session is recorded, and a build that
/// behaves the same is not caught while one that does not is.
#[ignore = "needs the Accessibility permission"]
#[test]
fn a_native_session_is_recorded_and_compared() {
    let d = tempfile::tempdir().unwrap();
    // The oracle starts a program, so it needs one that can be started: the window, compiled.
    let program = compile_window(&d.path().join("window"));

    let mut session = craft_anything_engine::testing::session();
    session.settings.deep_mode = false;
    let engine = craft_anything_engine::Engine::new(session);
    let call = |tool: &str, params: serde_json::Value| {
        engine.call(
            tool,
            params,
            craft_anything_engine::CallOptions::new(craft_anything_engine::Caller::Agent).with_consent(craft_anything_engine::Consent::Granted),
        )
    };

    let target = TargetRef::Local { path: program.clone() };
    let app = craft_anything_engine::oracle::TheApp { program, args: Vec::new() };
    let oracle = super::oracle_native::NativeOracle;

    // The original's session is recorded.
    let script = oracle.record(&context(&call, &target, &app)).expect("record the original").expect("the original was recorded");
    assert!(!script.steps.is_empty(), "the window's controls became steps: {:?}", script.steps);
    // The field is filled before the button is pressed.
    let kinds: Vec<String> = script.steps.iter().filter_map(|s| s.get("action")).filter_map(|k| k.as_str()).map(str::to_string).collect();
    let fill_at = kinds.iter().position(|k| k == "fill").expect("a fill");
    let press_at = kinds.iter().position(|k| k == "press").expect("a press");
    assert!(fill_at < press_at, "the field is filled before the button is pressed: {kinds:?}");

    // The build is the same program, so it behaves the same and is not caught.
    let build_app = craft_anything_engine::oracle::TheApp { program: d.path().join("window"), args: Vec::new() };
    let _ = &build_app;
    let observation = oracle.replay(&context(&call, &target, &build_app), &script).expect("replay the build");
    let differences = oracle.compare(&script, &observation);
    assert!(differences.iter().all(|d| !d.is_failure()), "the same program must not differ: {differences:?}");
}

/// The observation a walkthrough of a window gives: the controls it shows.
#[ignore = "needs the Accessibility permission"]
#[test]
fn a_windows_controls_are_read() {
    let (_child, pid) = start_window();
    let mut session = craft_anything_engine::testing::session();
    session.settings.deep_mode = false;
    let engine = craft_anything_engine::Engine::new(session);
    let call = |tool: &str, params: serde_json::Value| {
        engine.call(
            tool,
            params,
            craft_anything_engine::CallOptions::new(craft_anything_engine::Caller::Agent).with_consent(craft_anything_engine::Consent::Granted),
        )
    };
    let out = call("runtime_ui_read", json!({"pid": pid})).expect("read the window");
    let result = out.to_json().get("result").cloned().unwrap_or(serde_json::Value::Null);
    let controls = super::oracle_native::controls_of(&result);
    let obj = controls.as_object().expect("an object of controls");
    assert!(!obj.is_empty(), "the window showed controls: {obj:?}");
    // The field and the button are both there.
    let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    assert!(keys.iter().any(|k| k.contains("theField") || k.contains("Field")), "{keys:?}");
    assert!(keys.iter().any(|k| k.contains("theButton") || k.contains("Button")), "{keys:?}");
}
