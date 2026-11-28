//! The native behaviour oracle: the controls a program shows, read from outside, and compared.

use serde_json::{Value, json};

use super::oracle_native::controls_of;

/// A window with a heading, a field and two buttons.
fn a_window() -> serde_json::Value {
    json!({
        "nodes": [
            {"depth": 0, "role": "AXWindow", "title": "Demo"},
            {"depth": 1, "role": "AXHeading", "title": "Calculator"},
            {"depth": 1, "role": "AXTextField", "title": "Number", "value": "41"},
            {"depth": 1, "role": "AXButton", "title": "Add"},
            {"depth": 1, "role": "AXButton", "title": "Clear"},
            {"depth": 1, "role": "AXStaticText", "title": "Result: 42"},
            {"depth": 1, "role": "AXGroup", "title": ""}
        ]
    })
}

/// One observation of a window.
fn observation(tree: &serde_json::Value) -> craft_anything_engine::oracle::Observation {
    craft_anything_engine::oracle::Observation {
        oracle: super::oracle_native::ORACLE_ID.into(),
        version: 1,
        readings: super::oracle_native::readings(tree),
        gaps: vec![],
    }
}

#[test]
fn the_controls_are_read_by_role_and_title() {
    let controls = controls_of(&a_window());
    let obj = controls.as_object().expect("an object of controls");
    assert_eq!(obj.get("AXButton: Add").and_then(|v| v.as_str()), Some(""));
    assert_eq!(obj.get("AXTextField: Number").and_then(|v| v.as_str()), Some("41"));
    assert_eq!(obj.get("AXHeading: Calculator").and_then(|v| v.as_str()), Some(""));
}

#[test]
fn a_node_with_neither_role_nor_title_is_not_read() {
    let with_empty = json!({
        "nodes": [
            {"depth": 0, "role": "", "title": ""},
            {"depth": 1, "role": "AXButton", "title": "Add"}
        ]
    });
    let obj = controls_of(&with_empty).as_object().expect("an object of controls").clone();
    // A node with neither a role nor a title is left out; a role alone is enough.
    assert_eq!(obj.len(), 1, "{:?}", obj.keys().collect::<Vec<_>>());
    assert!(obj.contains_key("AXButton: Add"));
}

#[test]
fn a_window_with_nothing_shows_nothing() {
    assert_eq!(controls_of(&json!({"nodes": []})), json!({}));
    assert_eq!(controls_of(&json!({})), json!({}));
}

/// The same controls in another order are the same controls.
#[test]
fn the_order_of_the_controls_does_not_matter() {
    let mut nodes = a_window()["nodes"].as_array().cloned().unwrap_or_default();
    nodes.reverse();
    let reversed = controls_of(&json!({"nodes": nodes}));
    assert_eq!(reversed, controls_of(&a_window()));
}

/// A build that shows the same controls is not caught.
#[test]
fn a_build_showing_the_same_controls_is_not_caught() {
    let script = super::oracle_native::script(vec![], vec![observation(&a_window())]);
    let build = vec![craft_anything_engine::oracle::Observation {
        oracle: super::oracle_native::ORACLE_ID.into(),
        version: 1,
        readings: super::oracle_native::readings(&a_window()),
        gaps: vec![],
    }];
    let differences = craft_anything_engine::oracle::compare(&script, &build);
    assert!(differences.iter().all(|d| !d.is_failure()), "the same controls must not differ: {differences:?}");
}

/// A build that shows a different control is caught.
#[test]
fn a_build_showing_a_different_control_is_caught() {
    let script = super::oracle_native::script(vec![], vec![observation(&a_window())]);
    let other = json!({
        "nodes": [
            {"depth": 0, "role": "AXWindow", "title": "Demo"},
            {"depth": 1, "role": "AXHeading", "title": "Calculator"},
            {"depth": 1, "role": "AXTextField", "title": "Number", "value": "41"},
            {"depth": 1, "role": "AXButton", "title": "Add"},
            {"depth": 1, "role": "AXButton", "title": "Cancel"}
        ]
    });
    let build = vec![craft_anything_engine::oracle::Observation {
        oracle: super::oracle_native::ORACLE_ID.into(),
        version: 1,
        readings: super::oracle_native::readings(&other),
        gaps: vec![],
    }];
    let differences = craft_anything_engine::oracle::compare(&script, &build);
    let failures: Vec<_> = differences.iter().filter(|d| d.is_failure()).collect();
    assert!(!failures.is_empty(), "a different control must be caught: {differences:?}");
    assert!(failures[0].detail.contains("Clear"), "{}", failures[0].detail);
}

/// A build that shows nothing cannot be said to show the same.
#[test]
fn a_build_showing_nothing_is_not_alike() {
    let script = super::oracle_native::script(vec![], vec![observation(&a_window())]);
    let build = vec![craft_anything_engine::oracle::Observation {
        oracle: super::oracle_native::ORACLE_ID.into(),
        version: 1,
        readings: super::oracle_native::readings(&json!({"nodes": []})),
        gaps: vec![],
    }];
    let differences = craft_anything_engine::oracle::compare(&script, &build);
    assert!(differences.iter().any(|d| d.is_failure()), "showing nothing is not showing the same: {differences:?}");
}

/// The native oracle needs a program to start.
#[test]
fn the_native_oracle_needs_a_program() {
    use craft_anything_engine::oracle::Capability;
    let oracles = super::oracles::Oracles::built_in();
    let needs = oracles.by_id(super::oracle_native::ORACLE_ID).map(|o| o.needs().capabilities().to_vec());
    assert_eq!(needs, Some(vec![Capability::App]));
}

/// The native oracle is registered.
#[test]
fn the_native_oracle_is_registered() {
    let oracles = super::oracles::Oracles::built_in();
    assert!(oracles.by_id(super::oracle_native::ORACLE_ID).is_some());
    assert!(oracles.ids().contains(&"native"));
}

/// The end-to-end test: a real program, watched and compared.
///
/// It needs a window on this computer, so it is `#[ignore]`d, like the project's other live
/// tests.
#[ignore = "needs a window on this computer"]
#[test]
fn a_program_is_started_watched_and_closed() {
    use craft_anything_engine::oracle::{BehaviorOracle, OracleCx, TheApp};
    use craft_anything_engine::tool::ToolOutput;
    use craft_anything_engine::workspace::TargetRef;

    let d = tempfile::tempdir().unwrap();
    let script = d.path().join("app.py");
    std::fs::write(&script, "import tkinter as tk\nr = tk.Tk()\nr.title('Demo')\ntk.Button(r, text='Add').pack()\nr.mainloop()\n").unwrap();
    // The program is the interpreter; the script is its argument.
    let python = craft_anything_platform::which("python3").expect("install Python 3 to run this ignored test");
    let app = python.clone();

    let mut session = craft_anything_engine::testing::session();
    session.settings.deep_mode = false;
    let engine = craft_anything_engine::Engine::new(session);
    let call = |tool: &str, params: serde_json::Value| -> Result<ToolOutput, craft_anything_engine::evidence::ToolError> {
        engine.call(
            tool,
            params,
            craft_anything_engine::CallOptions::new(craft_anything_engine::Caller::Agent).with_consent(craft_anything_engine::Consent::Granted),
        )
    };
    let target = TargetRef::Local { path: app.clone() };
    let context = OracleCx::original(&call, &target, false);
    let held = TheApp { program: app.clone(), args: vec![script.display().to_string()] };
    let context = context.with_app(&held);
    let oracle = super::oracle_native::NativeOracle;
    let script = oracle.record(&context).expect("the original is watched");
    assert!(script.is_some(), "the program showed a window");
    let script = script.unwrap();
    assert!(!script.steps.is_empty(), "the controls it showed became steps: {:?}", script.steps);
}

/// A field takes text; a button does not.
#[test]
fn a_field_takes_text_and_a_button_does_not() {
    assert!(super::oracle_native::takes_text("AXTextField"));
    assert!(super::oracle_native::takes_text("AXSecureTextField"));
    assert!(super::oracle_native::takes_text("AXComboBox"));
    assert!(!super::oracle_native::takes_text("AXButton"));
    assert!(!super::oracle_native::takes_text("AXStaticText"));
}

/// The native probe text is the same as the web one, so both sides are filled the same way.
#[test]
fn the_native_probe_text_is_the_same_as_the_web_one() {
    assert_eq!(super::oracle_native::PROBE_TEXT, "craft-anything");
    assert_eq!(super::oracle_native::PROBE_TEXT, super::oracle_web::PROBE_TEXT);
}

/// The native session fills the fields before it presses the buttons.
///
/// The order is the point: a control that depends on what was typed only shows its effect when
/// the field is filled first.
#[test]
fn the_native_session_fills_the_fields_before_it_presses_the_buttons() {
    // The controls the program showed, as the tree gives them.
    let controls = json!({
        "AXButton: Save": "",
        "AXTextField: Name": "",
        "AXStaticText: Hello": ""
    });
    let mut fills = Vec::new();
    let mut presses = Vec::new();
    for key in controls.as_object().expect("an object of controls").keys() {
        let (role, title) = key.split_once(": ").expect("a role and a title");
        if super::oracle_native::takes_text(role) {
            fills.push(title.to_string());
        } else if role != "AXStaticText" {
            presses.push(title.to_string());
        }
    }
    assert_eq!(fills, ["Name"], "the field is filled");
    assert_eq!(presses, ["Save"], "the button is pressed");
}

/// The controls a window shows, grouped by the container they are in.
///
/// The accessibility tree says how deep each control sits, and the ones that share a container
/// belong together: a panel's fields and its button are one group.
#[test]
fn the_controls_are_grouped_by_the_container_they_are_in() {
    let tree = json!({
        "nodes": [
            {"depth": 0, "role": "AXWindow", "title": "Demo"},
            {"depth": 1, "role": "AXGroup", "title": "Details"},
            {"depth": 2, "role": "AXTextField", "title": "Name"},
            {"depth": 2, "role": "AXButton", "title": "Save"},
            {"depth": 1, "role": "AXButton", "title": "Cancel"}
        ]
    });
    let groups = super::oracle_native::groups_of(&tree);
    assert_eq!(groups.len(), 2, "the panel is one group and the outer button another: {groups:?}");
    let panel: Vec<String> = groups[0].iter().filter_map(|a| a.get("title")).filter_map(|t| t.as_str()).map(str::to_string).collect();
    assert_eq!(panel, ["Name", "Save"], "the panel's field and its button are one group");
    let outer: Vec<String> = groups[1].iter().filter_map(|a| a.get("title")).filter_map(|t| t.as_str()).map(str::to_string).collect();
    assert_eq!(outer, ["Cancel"], "the outer button is its own group");
}

/// Within a group, the fields are filled before the buttons are pressed.
#[test]
fn a_group_fills_its_fields_before_it_presses_its_buttons() {
    let tree = json!({
        "nodes": [
            {"depth": 1, "role": "AXButton", "title": "Save"},
            {"depth": 1, "role": "AXTextField", "title": "Name"}
        ]
    });
    let groups = super::oracle_native::groups_of(&tree);
    let kinds: Vec<String> = groups[0].iter().filter_map(|a| a.get("action")).filter_map(|k| k.as_str()).map(str::to_string).collect();
    assert_eq!(kinds, ["fill", "press"], "the field is filled before the button is pressed");
}

/// The recording and the replay must run the same steps in the same order, or the build is
/// compared with the original on different terms.
#[test]
fn the_steps_carry_what_to_do_and_where() {
    let tree = json!({
        "nodes": [
            {"depth": 1, "role": "AXTextField", "title": "Name"},
            {"depth": 1, "role": "AXButton", "title": "Save"}
        ]
    });
    let steps: Vec<Value> = super::oracle_native::groups_of(&tree).into_iter().flatten().collect();
    assert_eq!(steps.len(), 2);
    // The field is filled first, and the fill carries the text to put in.
    assert_eq!(steps[0]["action"], "fill");
    assert_eq!(steps[0]["title"], "Name");
    assert_eq!(steps[0]["value"], super::oracle_native::PROBE_TEXT);
    // Then the button is pressed.
    assert_eq!(steps[1]["action"], "press");
    assert_eq!(steps[1]["title"], "Save");
}

/// A control that cannot be acted on is recorded, so the build must fail it the same way.
#[test]
fn a_control_that_could_not_be_acted_on_is_a_gap() {
    // The gap is what the recording keeps when an action did not work.
    let gap = "the original's Name could not be acted on";
    assert!(gap.contains("could not be acted on"), "{gap}");
}

/// A native session is bounded, so a window with many panels cannot make the study endless.
#[test]
fn a_native_session_is_bounded() {
    // Many panels, each with its own field and button.
    let mut nodes = Vec::new();
    for i in 0..40 {
        nodes.push(json!({"depth": 1, "role": "AXGroup", "title": format!("Panel{i}")}));
        nodes.push(json!({"depth": 2, "role": "AXTextField", "title": format!("Name{i}")}));
        nodes.push(json!({"depth": 2, "role": "AXButton", "title": format!("Save{i}")}));
    }
    let groups = super::oracle_native::groups_of(&json!({"nodes": nodes}));
    assert!(groups.len() <= super::oracle_native::MAX_GROUPS, "the groups are bounded: {}", groups.len());
    let actions: usize = groups.iter().map(|g| g.len()).sum();
    assert!(actions <= super::oracle_native::MAX_SESSION_ACTIONS, "the session is bounded: {actions}");
}
