//! The web behaviour oracle: drive the original's page through a scripted session, keep what it
//! did, then drive the build through the same session and compare.
//!
//! The website check already compares the build with the original's pictures. This adds the other
//! half: what the page *does*. The original's session is the oracle, recorded once in Study and
//! replayed against the build, so the build is judged against the original's behaviour rather
//! than against its own account of itself.
//!
//! The comparison is the engine's own `web_snapshots_compare`, which walks two scripted runs step
//! by step and names the field that changed. This module's job is to record the original, replay
//! the build, and turn that diff into claims.

use craft_anything_engine::evidence::ToolError;
use craft_anything_engine::oracle::{BehaviorOracle, Needs, Observation, OracleCx, Probe, ProbeKind, Reading, SCRIPT_VERSION, Script, ThePage};
use craft_anything_engine::workspace::{Claim, TargetRef};
use serde_json::{Value, json};

use super::CheckResult;
use super::kinds::{Check, CheckCx};

/// The oracle's id, stored with the script so a build is compared the same way.
pub const ORACLE_ID: &str = "web";

/// The probes a web run is read through.
///
/// The page's accessibility tree is compared as labelled values — what the page showed, by role
/// and name — rather than as bytes, so two pages that show the same thing in a different order
/// behave the same way.
fn probes() -> Vec<Probe> {
    vec![Probe { name: "page".into(), kind: ProbeKind::Fields, tolerance: None }]
}

/// The values the page showed, read from its accessibility tree.
///
/// Each value is a labelled pair — the role and the name — so a difference is a difference in
/// what the page showed, not in the order the tree happened to list it.
pub fn page_values(result: &Value) -> Value {
    // The tree is read wherever it is: a whole run keeps it under `final_artifacts`, one step
    // under `artifacts`, and the tree itself is the nodes.
    let nodes = result
        .pointer("/final_artifacts/accessibility/nodes")
        .or_else(|| result.pointer("/artifacts/accessibility/nodes"))
        .or_else(|| result.pointer("/nodes"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut fields = serde_json::Map::new();
    for node in nodes {
        let Some(name) = node.get("name").and_then(Value::as_str) else { continue };
        if name.is_empty() {
            continue;
        }
        let role = node.get("role").and_then(Value::as_str).unwrap_or("");
        // The same name under two roles is two things; the role keeps them apart.
        let key = if role.is_empty() { name.to_string() } else { format!("{role}: {name}") };
        // A name that appears twice is one thing shown twice, not two things.
        fields.entry(key).or_insert(Value::String(name.to_string()));
    }
    Value::Object(fields)
}

/// The readings of one run, in the order of [`probes`].
///
/// The page's values are read, and the run they came from is kept beside them, so the two runs
/// can be compared step by step.
pub fn readings(result: &Value) -> Vec<Option<Reading>> {
    let mut values = page_values(result);
    if let Some(obj) = values.as_object_mut() {
        obj.insert("run".into(), result.clone());
    }
    vec![Some(Reading { value: values, engine: None })]
}

/// One observation per step of a run: what the page showed after each action.
///
/// The last step's reading is the whole page; the earlier ones are what it showed then, so a
/// difference in the middle of a session is visible and not only the end.
pub fn step_observations(result: &Value) -> Vec<Observation> {
    let steps = result.get("steps").and_then(Value::as_array).cloned().unwrap_or_default();
    if steps.is_empty() {
        return vec![Observation { oracle: ORACLE_ID.into(), version: SCRIPT_VERSION, readings: readings(result), gaps: Vec::new() }];
    }
    steps
        .iter()
        .map(|step| {
            // What this step showed: the page as it was after the step, from that step's own
            // accessibility tree. A step without one falls back to the whole run.
            let tree = step.get("artifacts").and_then(|a| a.get("accessibility")).cloned().unwrap_or(Value::Null);
            let mut values = if tree.is_null() { page_values(result) } else { page_values(&tree) };
            // The run is kept beside the values, so the two runs can be compared as a whole too.
            if let Some(obj) = values.as_object_mut() {
                obj.insert("run".into(), result.clone());
            }
            Observation { oracle: ORACLE_ID.into(), version: SCRIPT_VERSION, readings: vec![Some(Reading { value: values, engine: None })], gaps: Vec::new() }
        })
        .collect()
}

/// The script for a page that was driven through these actions.
///
/// The original's own run is kept with the script: it is the oracle the build is compared with.
pub fn script(steps: Vec<Value>, original: Vec<Observation>) -> Script {
    Script { version: SCRIPT_VERSION, oracle: ORACLE_ID.into(), steps, probes: probes(), original }
}

/// The actions that drive the session: the original's own, replayed against the build's address.
fn actions(script: &Script, url: &str) -> Vec<Value> {
    script
        .steps
        .iter()
        .map(|step| {
            let mut action = step.get("action").cloned().unwrap_or(Value::Null);
            // The original's address becomes the build's: the same page, somewhere else.
            if let Some(obj) = action.as_object_mut()
                && obj.get("kind").and_then(Value::as_str) == Some("goto")
            {
                obj.insert("url".into(), json!(url));
            }
            action
        })
        .collect()
}

/// The web behaviour check: the build is driven through the original's session and the two runs
/// are compared.
pub struct WebBehaviourCheck;

impl Check for WebBehaviourCheck {
    fn id(&self) -> &'static str {
        "web-behaviour"
    }

    fn applies(&self, cx: &CheckCx) -> bool {
        // Only a site has a page to drive, and only a recorded session can be replayed.
        matches!(cx.ws.manifest.target, TargetRef::Website { .. }) && cx.ws.manifest.script.is_some()
    }

    fn run(&self, cx: &mut CheckCx) -> Result<CheckResult, ToolError> {
        let Some(script) = cx.ws.manifest.script.as_ref() else {
            return Ok(CheckResult::new(self.id(), cx.round, false, "There's no recorded behaviour to compare with."));
        };
        let Some(site) = super::output::site_dir(&cx.ws.app_dir()) else {
            return Ok(CheckResult {
                claims: vec![Claim::new("The build behaves like the original", false, "There's no built site to drive.")],
                ..CheckResult::new(self.id(), cx.round, true, "There's no built site to drive.")
            });
        };
        let url = cx.serve_preview(&site)?;
        // The oracle that recorded the original is the one that judges the build.
        let oracles = crate::rebuild::oracles::Oracles::built_in();
        let Some(oracle) = oracles.by_id(ORACLE_ID) else {
            return Ok(CheckResult::new(self.id(), cx.round, false, "There's no web oracle to compare with."));
        };
        let call = |tool: &str, params: Value| cx.call(tool, params);
        let held = ThePage(url.clone());
        let context = OracleCx::original(&call, &cx.ws.manifest.target, cx.deep()).with_page(&held);
        let observation = match oracle.replay(&context, script) {
            Ok(o) => o,
            Err(e) => {
                let detail = format!("The build's page couldn't be driven: {}", e.message);
                return Ok(CheckResult {
                    claims: vec![Claim::new("The build behaves like the original", false, detail.clone())],
                    ..CheckResult::new(self.id(), cx.round, true, detail)
                });
            }
        };

        // What the page showed, step by step, through the same comparison every oracle uses.
        let mut claims = Vec::new();
        let mut unknowns = Vec::new();
        for difference in oracle.compare(script, &observation) {
            if difference.is_failure() {
                claims.push(Claim::new(difference.title(), false, difference.detail.clone()));
            } else {
                unknowns.push(difference.detail.clone());
            }
        }

        let note = if claims.is_empty() && unknowns.is_empty() {
            "It behaves like the original through the recorded session.".to_string()
        } else {
            let mut parts = Vec::new();
            if !claims.is_empty() {
                parts.push(format!("{} of the original's behaviours don't hold yet", claims.len()));
            }
            if !unknowns.is_empty() {
                parts.push(format!("{} could not be compared", unknowns.len()));
            }
            format!("{}.", parts.join(", "))
        };
        Ok(CheckResult { claims, unknowns, ..CheckResult::new(self.id(), cx.round, true, note) })
    }
}

/// The text a field is filled with when the original's own content is not known.
///
/// It is a plain, visible value, so filling it changes the page in a way both sides can be
/// compared on: the same text on the original and on the build.
pub const PROBE_TEXT: &str = "craft-anything";

/// A control the page shows, that a session can act on.
#[derive(Debug, Clone)]
pub struct Control {
    pub role: String,
    pub name: String,
    pub step_id: String,
    /// How to find the control again: a CSS selector when it has one, or its role and name.
    ///
    /// A control with no accessible name cannot be found by role and name, so it is found by
    /// its id in the page's own markup.
    pub selector: Option<String>,
    /// The value a choice is given: one of the options it offers, so the choice really takes.
    pub option: Option<String>,
    /// Every option the control offers, in order.
    ///
    /// A choice whose options change when another control is pressed *depends* on it. That is
    /// how a country decides which regions are offered, and it is invisible to a method that
    /// only looks for controls that appeared.
    pub offers: Vec<String>,
    /// The option the control already has.
    ///
    /// Choosing it again is not a change: the page is already in that state.
    pub selected: Option<String>,
    /// What the control holds.
    ///
    /// A choice that rewrites a field's value shows here and nowhere else: the markup keeps
    /// the attribute a field was written with, not the one it holds now.
    pub holds: Option<String>,
}

impl Control {
    /// A control with a role and a name, found by its role and name.
    pub fn of(role: &str, name: &str, step_id: &str) -> Control {
        Control { role: role.into(), name: name.into(), step_id: step_id.into(), selector: None, option: None, offers: Vec::new(), selected: None, holds: None }
    }

    /// Whether the control takes text: a field, a box or a combo.
    pub fn takes_text(&self) -> bool {
        matches!(self.role.as_str(), "textbox" | "searchbox" | "combobox" | "textarea" | "spinbutton")
    }

    /// Whether the control is pressed: a button, a link, a checkbox, a tab.
    pub fn is_button(&self) -> bool {
        matches!(self.role.as_str(), "button" | "link" | "checkbox" | "radio" | "switch" | "tab" | "menuitem" | "option")
    }

    /// Whether the control may show other controls: a choice, a menu, a tab.
    ///
    /// These are the ones that can bring a part of the form onto the page, so they are acted on
    /// on their own to see what they show.
    pub fn reveals(&self) -> bool {
        matches!(self.role.as_str(), "combobox" | "listbox" | "menu" | "menuitem" | "tab" | "treeitem")
    }

    /// The action that shows what this control shows.
    ///
    /// A choice is *selected*, not clicked: clicking a combo box opens it but chooses nothing, so
    /// nothing appears.
    pub fn reveal_action(&self) -> Value {
        let mut action = match self.role.as_str() {
            "combobox" | "listbox" => json!({"kind": "select", "value": self.value(), "step_id": self.step_id}),
            _ => json!({"kind": "click", "step_id": self.step_id}),
        };
        self.locate(&mut action);
        action
    }

    /// The action that chooses this control by one of the options it offers *now*.
    ///
    /// A choice whose options changed is chosen by one of the new ones: the old value is not
    /// offered any more, and choosing it would do nothing.
    pub fn choose_offered_action(&self) -> Option<Value> {
        let value = self.offers.first()?.clone();
        let mut action = json!({"kind": "select", "value": value, "step_id": self.step_id});
        self.locate(&mut action);
        Some(action)
    }

    /// The action that reads this control's value, leaving it as it is.
    ///
    /// A field another control rewrites is read, not filled: the choice decides what it holds,
    /// so typing into it would not be the same behaviour.
    pub fn read_action(&self) -> Value {
        let mut action = json!({"kind": "read", "step_id": self.step_id});
        self.locate(&mut action);
        action
    }

    /// The action that fills this control with the probe text.
    pub fn fill_action(&self) -> Value {
        let mut action = json!({"kind": "fill", "value": PROBE_TEXT, "step_id": self.step_id});
        self.locate(&mut action);
        action
    }

    /// The action that presses this control.
    pub fn click_action(&self) -> Value {
        let mut action = json!({"kind": "click", "step_id": self.step_id});
        self.locate(&mut action);
        action
    }

    /// Say where the control is: by its selector when it has one, by its role and name otherwise.
    fn locate(&self, action: &mut Value) {
        match &self.selector {
            Some(selector) => {
                action["selector"] = json!(selector);
            }
            None => {
                action["role"] = json!(self.role);
                action["name"] = json!(self.name);
            }
        }
    }

    /// The value a choice is given: one of its options, so the choice really takes.
    pub fn value(&self) -> String {
        self.option.clone().unwrap_or_else(|| PROBE_TEXT.to_string())
    }

    /// Every option this control offers, in order.
    ///
    /// All of them are tried, including the one the control already has: a page may hide what
    /// it shows until the choice is made, so the default can be the option that matters. A
    /// control with no options in the markup is pressed once, as it is.
    pub fn choices(&self) -> Vec<String> {
        if self.offers.is_empty() {
            return vec![self.value()];
        }
        self.offers.clone()
    }

    /// The action that chooses this control by a given option.
    pub fn choose_action(&self, value: &str) -> Value {
        let mut action = json!({"kind": "select", "value": value, "step_id": self.step_id});
        self.locate(&mut action);
        action
    }
}

/// The controls a page's accessibility tree shows, in the order they appear.
///
/// Only the roles a script can act on are listed: a link, a button, a checkbox, a radio button,
/// a switch, a tab, a menu item or a textbox. A name that appears twice is one control, acted
/// on once.
pub fn controls_of(result: &Value) -> Vec<Control> {
    controls_of_page(result, &Value::Null)
}

/// The controls a page shows, with the page's own markup beside them.
///
/// The markup gives a control with no accessible name a way to be found again — its id — and
/// gives a choice the option it offers, so choosing it really takes.
pub fn controls_of_page(result: &Value, dom: &Value) -> Vec<Control> {
    const ACTABLE: &[&str] = &["link", "button", "checkbox", "radio", "switch", "tab", "menuitem", "textbox", "combobox", "option"];
    let nodes = result.pointer("/final_artifacts/accessibility/nodes").and_then(Value::as_array).cloned().unwrap_or_default();
    let markup = markup_says(dom);
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    // How many controls of each role have been taken from the markup already.
    let mut taken: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for node in nodes.iter() {
        let role = node.get("role").and_then(Value::as_str).unwrap_or("");
        if !ACTABLE.contains(&role) {
            continue;
        }
        let name = node.get("name").and_then(Value::as_str).unwrap_or("");
        // A control with a role but no name is still a control: a choice with no label.
        let key = if name.is_empty() {
            let value = node.get("value").and_then(Value::as_str).unwrap_or("");
            format!("{role}:<unnamed:{value}>")
        } else {
            format!("{role}:{name}")
        };
        if !seen.insert(key) {
            continue;
        }
        // The markup says how to find the control; the tree says what it is called. The
        // selector is the surer way to find it, so it is used when there is one.
        //
        // The tree and the markup list the controls in the same order, so the *n*-th control of
        // a role in the tree is the *n*-th of that role in the markup. A page with two choices
        // is told apart this way: pairing them by role alone would give both the first one's
        // selector.
        let index = taken.entry(role.to_string()).or_insert(0);
        let element = markup.elements.get(role).and_then(|list| list.get(*index)).cloned();
        *index += 1;
        let selector = element.as_ref().filter(|e| !e.selector.is_empty()).map(|e| e.selector.clone());
        let step_id = format!("probe-{}", out.len() + 1);
        let option = element.as_ref().and_then(|e| e.option.clone());
        let offers = element.as_ref().map(|e| e.offers.clone()).unwrap_or_default();
        let selected = element.and_then(|e| e.selected);
        // What the control holds, from the accessibility tree.
        let holds = node.get("value").and_then(Value::as_str).map(str::to_string);
        out.push(Control { role: role.to_string(), name: name.to_string(), step_id, selector, option, offers, selected, holds });
    }
    // A choice with no markup is given one of the options the tree names. Only a choice that
    // has no value of its own is given one: a choice the markup was read for already knows its
    // value, and giving it another would choose the wrong option.
    let options: Vec<String> = nodes
        .iter()
        .filter(|n| n.get("role").and_then(Value::as_str) == Some("option"))
        .filter_map(|n| n.get("name").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    if let Some(choice) = out.iter_mut().find(|c| (c.role == "combobox" || c.role == "listbox") && c.option.is_none()) {
        choice.option = options.first().cloned();
    }
    out
}

/// What a page's markup says about its controls: the selector that finds each, and the value a
/// choice takes.
///
/// The accessibility tree says what a control is *called*; the markup says what it *is*. Both are
/// needed: a choice is chosen by the value of one of its options ("card"), not by its label
/// ("Card"), and a control with no label is found by its id.
fn markup_says(dom: &Value) -> Markup {
    // The markup is kept under `text`, beside its size and hash.
    let html = dom.get("text").and_then(Value::as_str).or_else(|| dom.as_str()).unwrap_or("");
    let mut out = Markup::default();
    if html.is_empty() {
        return out;
    }
    // The tags that hold a control, and the role each shows.
    const TAGS: &[(&str, &str)] = &[("select", "combobox"), ("input", "textbox"), ("textarea", "textbox"), ("button", "button")];
    for (tag, role) in TAGS {
        let mut rest = html;
        while let Some(at) = rest.find(&format!("<{tag}")) {
            let tail = &rest[at..];
            rest = &tail[1..];
            let Some(end) = tail.find('>') else { continue };
            let element = &tail[..end];
            let Some(id) = attribute(element, "id") else { continue };
            if id.is_empty() {
                continue;
            }
            let selector = format!("#{id}");
            // A page may hold several controls of the same kind — two choices, say — so each
            // one is kept, in the order the page lists them. The accessibility tree lists them
            // in the same order, so the two can be paired.
            let mut element = MarkupElement { selector: selector.clone(), option: None, offers: Vec::new(), selected: None };
            // A choice takes the value of one of its options, not the label.
            if *role == "combobox" {
                let after = &tail[end + 1..];
                let Some(close) = after.find("</select>") else { continue };
                let options = &after[..close];
                element.option = first_option_value(options);
                element.offers = all_option_values(options);
                // The option the choice already has: the one marked `selected`, or the first,
                // which is what a browser picks when none is marked.
                element.selected = selected_option(options).or_else(|| element.offers.first().cloned());
            }
            out.elements.entry((*role).to_string()).or_default().push(element);
        }
    }
    out
}

/// What a page's markup says about its controls.
#[derive(Debug, Clone, Default)]
struct Markup {
    /// The controls the markup holds, by the role they show, in the order the page lists them.
    elements: std::collections::BTreeMap<String, Vec<MarkupElement>>,
}

/// One control in a page's markup.
#[derive(Debug, Clone, Default)]
struct MarkupElement {
    /// How to find it again.
    selector: String,
    /// The value a choice takes.
    option: Option<String>,
    /// Every option a choice offers, in order.
    ///
    /// The options of a select are what another control can change, so they are kept whole.
    offers: Vec<String>,
    /// The option the choice already has.
    selected: Option<String>,
}

/// An attribute's value, from a tag's opening text.
fn attribute(element: &str, name: &str) -> Option<String> {
    let at = element.find(&format!("{name}=\""))?;
    let tail = &element[at + name.len() + 2..];
    let end = tail.find('"')?;
    Some(tail[..end].to_string())
}

/// The value of the first option in a select's markup.
fn first_option_value(options: &str) -> Option<String> {
    let at = options.find("<option")?;
    let tail = &options[at..];
    let end = tail.find('>')?;
    attribute(&tail[..end], "value")
}

/// The value of the option a select has marked as its own.
fn selected_option(options: &str) -> Option<String> {
    let mut rest = options;
    while let Some(at) = rest.find("<option") {
        let tail = &rest[at..];
        rest = &tail[1..];
        let Some(end) = tail.find('>') else { break };
        let element = &tail[..end];
        if element.contains("selected") {
            return attribute(element, "value");
        }
    }
    None
}

/// Every option's value in a select's markup, in the order they are offered.
fn all_option_values(options: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = options;
    while let Some(at) = rest.find("<option") {
        let tail = &rest[at..];
        rest = &tail[1..];
        let Some(end) = tail.find('>') else { break };
        if let Some(value) = attribute(&tail[..end], "value") {
            out.push(value);
        }
    }
    out
}

/// Record the original's session: drive its page through the controls it shows, and keep what
/// it showed.
///
/// The steps are the page's own controls, one action each, so a control that stops working is
/// visible without needing a path through the whole site.
pub fn record_session(call: &dyn Fn(&str, Value) -> Result<craft_anything_engine::tool::ToolOutput, ToolError>, url: &str) -> Result<Script, ToolError> {
    let mut actions = vec![json!({"kind": "goto", "url": url, "step_id": "open"})];
    let opened = call(
        "web_walkthrough_record",
        json!({
            "actions": actions.clone(),
            "capture": {"at_end": ["accessibility", "dom"]},
            "headless": true,
            "timeout_secs": 60,
        }),
    )?;
    let first = opened.to_json().get("result").cloned().unwrap_or(Value::Null);
    // The page's own markup, so a control with no accessible name can be found again.
    let dom = first.pointer("/final_artifacts/dom").cloned().unwrap_or(Value::Null);
    let controls = controls_of_page(&first, &dom);
    // The session is a chain, not a list: every field is filled first, then every button is
    // clicked. A page whose button depends on what was typed into a field only shows the effect
    // when the field is filled first — which is the whole point of a chain.
    //
    // The chain is grouped. A control that *reveals* other controls — a choice that shows a
    // part of the form — is pressed first, and the fields it revealed are filled after it, in
    // that group. A flat chain would fill fields that were not even on the page yet.
    let groups = within_budget(groups_of(&controls, call, url)?);
    for group in groups {
        for action in group {
            actions.push(action);
        }
    }
    if actions.len() == 1 {
        // Nothing to drive: the page shows no control to act on.
        return Ok(script(
            vec![json!({"action": json!({"kind": "goto", "url": url}), "step_id": "open", "url": url, "result": first})],
            vec![Observation { oracle: ORACLE_ID.into(), version: SCRIPT_VERSION, readings: readings(&first), gaps: Vec::new() }],
        ));
    }
    let run = call(
        "web_walkthrough_record",
        json!({
            "actions": actions.clone(),
            "capture": {"after_each_step": ["accessibility"], "at_end": ["accessibility"]},
            "headless": true,
            "timeout_secs": 120,
        }),
    )?;
    let result = run.to_json().get("result").cloned().unwrap_or(Value::Null);
    // The steps: the actions that were sent, in order, each with the original's own run kept
    // beside it. The run's result names only what each step did ("goto"), not the action itself,
    // so the actions are kept from what was sent.
    let sent = result
        .get("steps")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, step)| {
            json!({
                "action": actions.get(i).cloned().unwrap_or(Value::Null),
                "step_id": step.get("step_id").cloned().unwrap_or(Value::Null),
                "url": url,
                "result": result,
            })
        })
        .collect();
    Ok(script(sent, step_observations(&result)))
}

/// The web oracle: a page driven through the same session, compared by what it showed and by how
/// the two runs went step by step.
pub struct WebOracle;

impl BehaviorOracle for WebOracle {
    fn id(&self) -> &'static str {
        ORACLE_ID
    }

    fn supports(&self, target: &TargetRef) -> bool {
        matches!(target, TargetRef::Website { .. })
    }

    /// The build is a page to drive.
    fn needs(&self) -> Needs {
        Needs::page()
    }

    /// Record the original's session: drive its page through the controls it shows.
    fn record(&self, cx: &OracleCx) -> Result<Option<Script>, ToolError> {
        let TargetRef::Website { url } = cx.target else { return Ok(None) };
        Ok(Some(record_session(cx.call, url)?))
    }

    /// Drive the build's page through the recorded session and read what it showed.
    fn replay(&self, cx: &OracleCx, script: &Script) -> Result<Vec<Observation>, ToolError> {
        let url = cx.page.ok_or_else(|| ToolError::invalid("There's no built page to drive."))?.page_url();
        let params = json!({
            "actions": actions(script, url),
            "capture": {"after_each_step": ["accessibility"], "at_end": ["accessibility"]},
            "headless": true,
            "timeout_secs": 60,
        });
        let out = (cx.call)("web_walkthrough_record", params)?;
        let result = out.to_json().get("result").cloned().unwrap_or(Value::Null);
        Ok(step_observations(&result))
    }
}

/// The session's actions, grouped by what reveals what.
///
/// A page's controls are not all on the page at once: a choice shows a part of the form, and
/// that part has fields of its own. So the session is a chain of groups:
///
/// 1. the controls that are on the page to begin with are filled and pressed;
/// 2. pressing one of them may show more controls, and those are filled and pressed next.
///
/// The groups are found by observation, not by guessing: a control is pressed, and the page is
/// read again to see what it showed. What appeared is that control's group.
///
/// Every group is tried on its own, so one control that reveals nothing does not hide the ones
/// that do. The number of groups is bounded, so a page that reveals controls forever does not
/// run forever.
pub fn groups_of(
    controls: &[Control],
    call: &dyn Fn(&str, Value) -> Result<craft_anything_engine::tool::ToolOutput, ToolError>,
    url: &str,
) -> Result<Vec<Vec<Value>>, ToolError> {
    // The first group: the fields that are here now, then the buttons that are here now.
    let mut groups = vec![first_group(controls)];
    // A control that shows more: a choice, a menu, a tab.
    //
    // Each of these costs a browser session of its own, so the number of them is bounded: a
    // page with a hundred choices must not make the study take a hundred sessions.
    for control in controls.iter().filter(|c| c.reveals()).take(MAX_REVEALING_PROBES) {
        // Press it on its own and see what the page shows.
        //
        // A choice offers several options and only some of them change the page, so each option
        // is tried in turn and the one that changed something is kept. Choosing the first
        // option every time would pick the page's default every time, and a dependency would
        // never show: a country's first option is the one that is already chosen.
        let mut best: Option<(Value, Vec<Control>, Vec<Control>, Vec<Control>)> = None;
        for value in control.choices() {
            let action = control.choose_action(&value);
            let Ok(shown) = read_after(call, url, &action) else { continue };
            let dom = shown.pointer("/final_artifacts/dom").cloned().unwrap_or(Value::Null);
            let revealed = controls_of_page(&shown, &dom);
            // What changed by pressing it: controls that appeared, and controls whose *options*
            // changed. A choice that decides which options another choice offers is a
            // dependency that no list of appeared controls would show.
            let appeared: Vec<Control> = revealed.iter().filter(|c| !controls.iter().any(|k| k.role == c.role && k.name == c.name)).cloned().collect();
            let changed: Vec<Control> = revealed
                .iter()
                .filter(|c| !c.offers.is_empty())
                .filter(|c| controls.iter().find(|k| k.role == c.role && k.name == c.name).is_some_and(|before| before.offers != c.offers))
                .cloned()
                .collect();
            // A control whose *value* changed: a choice that rewrites a field. The control that
            // was acted on is left out — it changed because it was acted on, not because it
            // depends on anything.
            let rewritten: Vec<Control> = revealed
                .iter()
                .filter(|c| c.role == "textbox")
                .filter(|c| c.name != control.name)
                .filter(|c| controls.iter().find(|k| k.role == c.role && k.name == c.name).is_some_and(|before| before.holds != c.holds))
                .cloned()
                .collect();
            if appeared.is_empty() && changed.is_empty() && rewritten.is_empty() {
                continue;
            }
            // The option that changed the most is kept. A page's default may hide what it
            // shows until the choice is made, and another option may show less, so the one
            // that revealed the most is the one the page is understood by.
            let more = appeared.len() + changed.len() + rewritten.len();
            let best_more = best.as_ref().map(|(_, ap, ch, rw)| ap.len() + ch.len() + rw.len()).unwrap_or(0);
            if more >= best_more {
                best = Some((action, appeared, changed, rewritten));
            }
        }
        let Some((action, appeared, changed, rewritten)) = best else { continue };
        // The group is the control that revealed them, then what it revealed.
        let mut group = vec![action];
        for field in appeared.iter().filter(|c| c.takes_text()) {
            group.push(field.fill_action());
        }
        // A choice whose options changed is chosen too: which option it offers is part of how
        // the page behaves, and the build has to make the same offer. It is chosen by one of the
        // options it offers *now*, because the old value is not offered any more.
        for choice in changed.iter() {
            if let Some(action) = choice.choose_offered_action() {
                group.push(action);
            }
        }
        // A field the choice rewrote is read, not filled: the choice decides what it holds, so
        // typing into it would not be the same behaviour.
        for field in rewritten.iter() {
            group.push(field.read_action());
        }
        groups.push(group);
        if groups.len() >= MAX_GROUPS {
            break;
        }
    }
    Ok(groups)
}

/// The session's actions, within the budget.
///
/// The groups are kept in order; a group that would take the session past the budget is left
/// out, and the groups before it are kept.
pub fn within_budget(groups: Vec<Vec<Value>>) -> Vec<Vec<Value>> {
    let mut kept = Vec::new();
    let mut count = 0;
    for group in groups {
        if count + group.len() > MAX_SESSION_ACTIONS {
            break;
        }
        count += group.len();
        kept.push(group);
    }
    kept
}

/// Most groups a session is split into: enough for a form with a few choices, bounded so a page
/// that reveals controls forever does not run forever.
pub const MAX_GROUPS: usize = 8;

/// Most controls a session tries on their own, to see what they show.
///
/// Each one costs a browser session of its own, so this is what keeps a study from taking
/// forever on a page with many choices.
pub const MAX_REVEALING_PROBES: usize = 12;

/// Most actions one session may have.
///
/// A session is replayed against the build, so every action is run twice. This keeps a page with
/// many controls from making each round slow.
pub const MAX_SESSION_ACTIONS: usize = 40;

/// The first group: the fields that are here now, then the buttons that are here now.
fn first_group(controls: &[Control]) -> Vec<Value> {
    let mut group = Vec::new();
    for field in controls.iter().filter(|c| c.takes_text()) {
        group.push(field.fill_action());
    }
    for control in controls.iter().filter(|c| c.is_button() && !c.reveals()) {
        group.push(control.click_action());
    }
    group
}

/// Read the page after one action, on a page of its own.
fn read_after(call: &dyn Fn(&str, Value) -> Result<craft_anything_engine::tool::ToolOutput, ToolError>, url: &str, action: &Value) -> Result<Value, ToolError> {
    let actions = vec![json!({"kind": "goto", "url": url}), action.clone()];
    let out = call(
        "web_walkthrough_record",
        json!({
            "actions": actions,
            // The markup as well as the tree: what a choice *offers* is in the markup, and a
            // choice whose options changed is a dependency.
            "capture": {"at_end": ["accessibility", "dom"]},
            "headless": true,
            "timeout_secs": 60,
        }),
    )?;
    let result = out.to_json().get("result").cloned().unwrap_or(Value::Null);
    // The page as it was after the action: the run's final tree, which is the page as the last
    // action left it. It is handed back in the shape the controls are read from.
    Ok(json!({"final_artifacts": {
        "accessibility": result.pointer("/final_artifacts/accessibility").cloned().unwrap_or(Value::Null),
        "dom": result.pointer("/final_artifacts/dom").cloned().unwrap_or(Value::Null),
    }}))
}
