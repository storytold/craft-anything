//! The web behaviour oracle, live: a page whose button depends on what was typed into a field.
//!
//! This is the case a probe-style enumeration misses. Acting on each control on its own shows
//! nothing about the dependency; only filling the field and then pressing the button shows it.
//!
//! It needs a browser and a served build, so it is `#[ignore]`d, like the project's other live
//! tests.

use craft_anything_engine::oracle::{BehaviorOracle, OracleCx, ThePage};
use craft_anything_engine::workspace::{TargetRef, Workspace};
use serde_json::Value;

/// A page whose button's effect depends on what was typed.
const PAGE: &str = "<!doctype html><html><body>\
<h1>Sign in</h1>\
<form onsubmit=\"event.preventDefault();document.getElementById('out').textContent='Welcome '+document.getElementById('u').value\">\
<label>User <input id=u type=text></label>\
<button type=submit>Sign in</button>\
</form>\
<div id=out>Not signed in</div>\
</body></html>";

/// A build that presses the button but never fills the field: the dependency is missing.
const CLICK_ONLY: &str = "<!doctype html><html><body>\
<h1>Sign in</h1>\
<form onsubmit=\"event.preventDefault();document.getElementById('out').textContent='Welcome '+document.getElementById('u').value\">\
<label>User <input id=u type=text></label>\
<button type=submit>Sign in</button>\
</form>\
<div id=out>Not signed in</div>\
</body></html>";

/// The context the oracle records and replays through.
///
/// The call and the page are owned by the caller, so both outlive the context.
fn context<'a>(
    call: &'a dyn Fn(&str, serde_json::Value) -> Result<craft_anything_engine::tool::ToolOutput, craft_anything_engine::evidence::ToolError>,
    target: &'a TargetRef,
    page: &'a ThePage,
) -> OracleCx<'a> {
    OracleCx::original(call, target, false).with_page(page)
}

/// The build's own page, served on loopback.
fn serve_build(ws: &Workspace, body: &str) -> String {
    std::fs::write(ws.app_dir().join("index.html"), body).expect("the build's page");
    let server = craft_anything_platform::serve::serve_dir(&ws.app_dir()).expect("serve the build");
    let url = server.url();
    // The server must outlive this function, so it is kept in the project's own folder.
    let keep = ws.root.join("preview-server.txt");
    std::fs::write(&keep, &url).expect("keep the address");
    url
}

#[ignore = "needs a browser and a served build"]
#[test]
fn a_chain_is_recorded_and_a_build_that_only_clicks_is_caught() {
    let d = tempfile::tempdir().unwrap();

    // The original: a page whose button depends on the field.
    let original = d.path().join("original");
    std::fs::create_dir_all(&original).unwrap();
    std::fs::write(original.join("index.html"), PAGE).unwrap();
    let original_server = craft_anything_platform::serve::serve_dir(&original).expect("serve the original");
    let original_url = original_server.url();

    // The build: the same page, which will not fill the field.
    let ws = Workspace::create(d.path(), "Site", "Rebuild this sign-in page", TargetRef::Website { url: original_url.clone() }).unwrap();
    std::fs::create_dir_all(ws.app_dir()).unwrap();
    let build_url = serve_build(&ws, CLICK_ONLY);

    let mut session = craft_anything_engine::testing::session();
    // The web path reads the page's own structure, so it is Deep mode's work.
    session.settings.deep_mode = true;
    let engine = craft_anything_engine::Engine::new(session);
    let call = |tool: &str, params: serde_json::Value| {
        engine.call(
            tool,
            params,
            craft_anything_engine::CallOptions::new(craft_anything_engine::Caller::Agent).with_consent(craft_anything_engine::Consent::Granted),
        )
    };

    // The original's session is recorded: the field is filled, then the button pressed.
    let target = TargetRef::Website { url: original_url.clone() };
    let original_page = ThePage(original_url.clone());
    let oracle = super::oracle_web::WebOracle;
    let script = oracle.record(&context(&call, &target, &original_page)).expect("record the original").expect("the original was recorded");

    // The chain is there: a fill before the click.
    let actions: Vec<String> =
        script.steps.iter().filter_map(|s| s.get("action")).filter_map(|a| a.get("kind")).filter_map(|k| k.as_str()).map(str::to_string).collect();
    assert!(actions.iter().any(|a| a == "fill"), "the field is filled: {actions:?}");
    assert!(actions.iter().any(|a| a == "click"), "the button is pressed: {actions:?}");
    let fill_at = actions.iter().position(|a| a == "fill").expect("a fill");
    let click_at = actions.iter().position(|a| a == "click").expect("a click");
    assert!(fill_at < click_at, "the field is filled before the button is pressed: {actions:?}");

    // The build, driven through the same chain, does not show the same thing.
    let build_target = TargetRef::Website { url: build_url.clone() };
    let build_page = ThePage(build_url.clone());
    let observation = oracle.replay(&context(&call, &build_target, &build_page), &script).expect("replay the build");
    let differences = oracle.compare(&script, &observation);
    let failures: Vec<_> = differences.iter().filter(|d| d.is_failure()).collect();
    assert!(!failures.is_empty(), "a build that only clicks must be caught: {differences:?}");
}

/// A page whose choice shows a part of the form: choosing a card shows a number and a CVC.
const CONDITIONAL: &str = "<!doctype html><html><body>\
<h1>Checkout</h1>\
<label>Method <select id=m onchange=\"document.getElementById('card').style.display=this.value==='card'?'block':'none'\">\
<option value=card>Card</option><option value=cash>Cash</option></select></label>\
<div id=card style=\"display:none\">\
<label>Number <input id=cn type=text></label>\
<label>CVC <input id=cc type=text></label>\
</div>\
<button id=go>Pay</button>\
</body></html>";

#[ignore = "needs a browser and a served page"]
#[test]
fn a_choice_that_shows_fields_brings_them_into_the_session() {
    let d = tempfile::tempdir().unwrap();
    let original = d.path().join("original");
    std::fs::create_dir_all(&original).unwrap();
    std::fs::write(original.join("index.html"), CONDITIONAL).unwrap();
    let server = craft_anything_platform::serve::serve_dir(&original).expect("serve the original");
    let url = server.url();

    let mut session = craft_anything_engine::testing::session();
    session.settings.deep_mode = true;
    let engine = craft_anything_engine::Engine::new(session);
    let call = |tool: &str, params: serde_json::Value| {
        engine.call(
            tool,
            params,
            craft_anything_engine::CallOptions::new(craft_anything_engine::Caller::Agent).with_consent(craft_anything_engine::Consent::Granted),
        )
    };

    // The controls the page shows to begin with.
    let opened = call(
        "web_walkthrough_record",
        serde_json::json!({
            "actions": [{"kind": "goto", "url": url}],
            "capture": {"at_end": ["accessibility", "dom"]},
            "headless": true,
            "timeout_secs": 60,
        }),
    )
    .expect("open the page");
    let first = opened.to_json().get("result").cloned().unwrap_or(serde_json::Value::Null);
    let dom = first.pointer("/final_artifacts/dom").cloned().unwrap_or(serde_json::Value::Null);
    let controls = super::oracle_web::controls_of_page(&first, &dom);

    // The session is grouped: the choice is selected on its own, and the fields it shows are
    // filled after it.
    let groups = super::oracle_web::groups_of(&controls, &call, &url).expect("group the session");
    let kinds = |group: &Vec<Value>| -> Vec<String> { group.iter().filter_map(|a| a.get("kind")).filter_map(|k| k.as_str()).map(str::to_string).collect() };
    let all: Vec<String> = groups.iter().flat_map(kinds).collect();
    assert!(all.iter().any(|a| a == "select"), "the choice is selected: {all:?}");
    // The group that chose it is the group that fills the fields it showed.
    let group = groups.iter().find(|g| kinds(g).iter().any(|k| k == "select")).expect("a group with the choice");
    let group_kinds = kinds(group);
    assert!(group_kinds.iter().any(|k| k == "fill"), "the fields it showed are filled in its group: {group_kinds:?}");
    // And the choice comes before them: the fields were not on the page until it was chosen.
    let select_at = group_kinds.iter().position(|k| k == "select").expect("a select");
    let fill_at = group_kinds.iter().position(|k| k == "fill").expect("a fill");
    assert!(select_at < fill_at, "the choice is chosen before the fields it showed are filled: {group_kinds:?}");
}

/// A page whose country choice decides which regions are offered.
///
/// The region choice is on the page from the start, so nothing *appears* when the country is
/// chosen — only what the region *offers* changes. A method that looks for controls that
/// appeared finds nothing here.
const DEPENDENT: &str = r#"<!doctype html><html><head><script>
function regions() {
  var list = document.getElementById('regions');
  list.innerHTML = '';
  var opts = document.getElementById('country').value === 'de'
    ? [['de-b', 'Berlin'], ['de-h', 'Hamburg']]
    : [['us-n', 'New York'], ['us-c', 'Chicago']];
  opts.forEach(function (o) {
    var opt = document.createElement('option');
    opt.value = o[0];
    opt.textContent = o[1];
    list.appendChild(opt);
  });
}
</script></head><body>
<h1>Shipping</h1>
<label for="country">Country</label>
<select id="country" onchange="regions()">
  <option value="us">United States</option>
  <option value="de">Germany</option>
</select>
<label for="regions">Region</label>
<select id="regions"></select>
<button onclick="document.getElementById('out').textContent='sent'">Send</button>
<p id="out"></p>
</body></html>"#;

/// A choice that changes what another choice offers brings that other choice into the session.
///
/// The dependency is invisible to a method that looks for controls that appeared: the region
/// choice was on the page all along. It shows only in what the region *offers*, so that is what
/// is compared.
#[ignore = "needs a browser and a served page"]
#[test]
fn a_choice_that_changes_what_another_offers_brings_it_into_the_session() {
    let d = tempfile::tempdir().unwrap();
    let original = d.path().join("original");
    std::fs::create_dir_all(&original).unwrap();
    std::fs::write(original.join("index.html"), DEPENDENT).unwrap();
    let server = craft_anything_platform::serve::serve_dir(&original).expect("serve the original");
    let url = server.url();

    let mut session = craft_anything_engine::testing::session();
    session.settings.deep_mode = true;
    let engine = craft_anything_engine::Engine::new(session);
    let call = |tool: &str, params: serde_json::Value| {
        engine.call(
            tool,
            params,
            craft_anything_engine::CallOptions::new(craft_anything_engine::Caller::Agent).with_consent(craft_anything_engine::Consent::Granted),
        )
    };

    let opened = call(
        "web_walkthrough_record",
        serde_json::json!({
            "actions": [{"kind": "goto", "url": url}],
            "capture": {"at_end": ["accessibility", "dom"]},
            "headless": true,
            "timeout_secs": 60,
        }),
    )
    .expect("open the page");
    let first = opened.to_json().get("result").cloned().unwrap_or(serde_json::Value::Null);
    let dom = first.pointer("/final_artifacts/dom").cloned().unwrap_or(serde_json::Value::Null);
    let controls = super::oracle_web::controls_of_page(&first, &dom);

    // The region choice is on the page to begin with, offering nothing.
    let region = controls.iter().find(|c| c.name == "Region").expect("the region choice");
    assert!(region.offers.is_empty(), "the region choice offers nothing yet: {:?}", region.offers);

    let groups = super::oracle_web::groups_of(&controls, &call, &url).expect("group the session");
    let kinds = |group: &Vec<Value>| -> Vec<String> { group.iter().filter_map(|a| a.get("kind")).filter_map(|k| k.as_str()).map(str::to_string).collect() };
    let all: Vec<String> = groups.iter().flat_map(kinds).collect();

    // Two choices are made: the country, and the region the country decided.
    let selects: Vec<&Value> = groups.iter().flatten().filter(|a| a.get("kind").and_then(|k| k.as_str()) == Some("select")).collect();
    assert_eq!(selects.len(), 2, "the country is chosen and the region it decided: {all:?}");
    // The region is chosen by its value, and its value is one the country made possible.
    let region_value = selects[1]["value"].as_str().unwrap_or_default();
    assert!(region_value.starts_with("de-"), "the region is one Germany offers: {region_value}");
}

/// A page whose plan choice rewrites the name field's value.
///
/// Nothing appears and no choice offers anything new — only a field's *value* changes. A
/// method that looks for controls that appeared, or for options that changed, finds nothing
/// here.
const REWRITES: &str = r#"<!doctype html><html><head><script>
function sync() {
  document.getElementById('name').value = 'user-' + document.getElementById('plan').value;
}
</script></head><body>
<h1>Sign up</h1>
<label for="plan">Plan</label>
<select id="plan" onchange="sync()">
  <option value="free">Free</option>
  <option value="pro">Pro</option>
</select>
<label for="name">Name</label>
<input id="name" type="text" value="">
<label for="email">Email</label>
<input id="email" type="text" value="">
</body></html>"#;

/// A choice that rewrites a field's value brings that field into the session.
///
/// The dependency is invisible to a method that looks for controls that appeared, and to one
/// that looks for options that changed: the field was there all along, offering the same
/// options. It shows only in what the field holds.
#[ignore = "needs a browser and a served page"]
#[test]
fn a_choice_that_rewrites_a_fields_value_brings_it_into_the_session() {
    let d = tempfile::tempdir().unwrap();
    let original = d.path().join("original");
    std::fs::create_dir_all(&original).unwrap();
    std::fs::write(original.join("index.html"), REWRITES).unwrap();
    let server = craft_anything_platform::serve::serve_dir(&original).expect("serve the original");
    let url = server.url();

    let mut session = craft_anything_engine::testing::session();
    session.settings.deep_mode = true;
    let engine = craft_anything_engine::Engine::new(session);
    let call = |tool: &str, params: serde_json::Value| {
        engine.call(
            tool,
            params,
            craft_anything_engine::CallOptions::new(craft_anything_engine::Caller::Agent).with_consent(craft_anything_engine::Consent::Granted),
        )
    };

    let opened = call(
        "web_walkthrough_record",
        serde_json::json!({
            "actions": [{"kind": "goto", "url": url}],
            "capture": {"at_end": ["accessibility", "dom"]},
            "headless": true,
            "timeout_secs": 60,
        }),
    )
    .expect("open the page");
    let first = opened.to_json().get("result").cloned().unwrap_or(serde_json::Value::Null);
    let dom = first.pointer("/final_artifacts/dom").cloned().unwrap_or(serde_json::Value::Null);
    let controls = super::oracle_web::controls_of_page(&first, &dom);

    let groups = super::oracle_web::groups_of(&controls, &call, &url).expect("group the session");
    let all: Vec<String> = groups.iter().flatten().filter_map(|a| a.get("kind")).filter_map(|k| k.as_str()).map(str::to_string).collect();

    // The choice is made and the field it rewrote is read.
    assert!(all.iter().any(|a| a == "select"), "the choice is made: {all:?}");
    assert!(all.iter().any(|a| a == "read"), "the field it rewrote is read: {all:?}");
    // And the read comes after the choice: before it, the field holds nothing.
    let select_at = all.iter().position(|a| a == "select").expect("a select");
    let read_at = all.iter().position(|a| a == "read").expect("a read");
    assert!(select_at < read_at, "the choice is made before the field is read: {all:?}");
}
