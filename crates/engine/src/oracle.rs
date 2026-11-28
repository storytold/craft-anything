//! Behaviour oracles: a scripted experiment run against the original and against the build, so
//! the two can be compared.
//!
//! The website check already does this — Study photographs the original, Check photographs the
//! build, and the two are compared. The other kinds have no such comparison, so their builds are
//! judged by their own reports. This module is the seam that generalises it: an oracle records a
//! [`Script`] against the original once, replays the same script against the build, and compares
//! what the two saw.
//!
//! Three rules govern every comparison, and they are the same for every oracle:
//!
//! - a probe that reads a different value on the two sides is a [`DifferenceKind::Value`],
//!   a failed claim;
//! - a probe one side could not read is a [`DifferenceKind::Missing`], also a failed claim;
//! - a probe neither side could read is a [`DifferenceKind::Unreadable`], an unknown, never a
//!   pass.
//!
//! The last rule is the important one. A missing reading is not a reading of nothing, so it
//! never counts as alike.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::evidence::ToolError;
use crate::tool::ToolOutput;
use crate::workspace::TargetRef;

/// A script's format version. A script from an older version is read by that version's reader
/// or refused, never guessed at.
pub const SCRIPT_VERSION: u32 = 1;

/// What every oracle runs in: the engine, the target, and the mode.
///
/// The engine is reached through one function, so an oracle drives the same tools the builder
/// does and inherits everything the engine guarantees: external programs run through the
/// platform layer, the session lock is held correctly, and a run is cancelled when the person
/// pauses. An oracle never starts a process of its own.
///
/// The call is a reference to a closure the caller owns, so it outlives the context.
///
/// What an oracle can do with the build depends on what it is: a program is started, a page is
/// driven, a report is read. Those are separate capabilities, each with its own trait, so an
/// oracle declares only the ones it has.
pub struct OracleCx<'a> {
    /// Runs one engine tool by id.
    pub call: &'a dyn Fn(&str, Value) -> Result<ToolOutput, ToolError>,
    /// What is being recreated: the original's address or path, or none.
    pub target: &'a TargetRef,
    /// Deep mode is on: the original's inside may be read.
    pub deep: bool,
    /// The build as a program, when it is one.
    pub program: Option<&'a dyn RunsAProgram>,
    /// The build as a program to start, when it is one.
    pub app: Option<&'a dyn DrivesAnApp>,
    /// The build as a page, when it is one.
    pub page: Option<&'a dyn DrivesAPage>,
    /// The build's own report, when it saved one.
    pub report: Option<&'a dyn ReadsAReport>,
    /// The values both sides speak, when there are any.
    pub sheet: Option<&'a dyn NamesValues>,
}

impl<'a> OracleCx<'a> {
    /// A context with no build: the original's side, recorded before there is anything to judge.
    pub fn original(call: &'a dyn Fn(&str, Value) -> Result<ToolOutput, ToolError>, target: &'a TargetRef, deep: bool) -> OracleCx<'a> {
        OracleCx { call, target, deep, program: None, app: None, page: None, report: None, sheet: None }
    }

    /// The same context, with the build as a program.
    pub fn with_program(mut self, program: &'a dyn RunsAProgram) -> OracleCx<'a> {
        self.program = Some(program);
        self
    }

    /// The same context, with the build as a program to start.
    pub fn with_app(mut self, app: &'a dyn DrivesAnApp) -> OracleCx<'a> {
        self.app = Some(app);
        self
    }

    /// The same context, with the build as a page.
    pub fn with_page(mut self, page: &'a dyn DrivesAPage) -> OracleCx<'a> {
        self.page = Some(page);
        self
    }

    /// The same context, with the values both sides speak.
    pub fn with_sheet(mut self, sheet: &'a dyn NamesValues) -> OracleCx<'a> {
        self.sheet = Some(sheet);
        self
    }

    /// The same context, with the build's own report.
    pub fn with_report(mut self, report: &'a dyn ReadsAReport) -> OracleCx<'a> {
        self.report = Some(report);
        self
    }
}

/// What an oracle needs from the build before it can judge it.
///
/// The set is open: a new kind of capability is a new [`Capability`], and nothing here has to
/// change to make room for it. An oracle that needs a page says so, and a caller with only a
/// program does not pretend otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Needs(Vec<Capability>);

/// One kind of thing the build can be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// The build is a program to start.
    Program,
    /// The build is a page to drive.
    Page,
    /// The build saved a report to read.
    Report,
    /// The build is a program to start and watch.
    App,
}

impl Needs {
    /// Nothing beyond the original and the engine.
    pub fn none() -> Needs {
        Needs(Vec::new())
    }

    /// What the build must be for this oracle to judge it.
    pub fn of(capabilities: impl IntoIterator<Item = Capability>) -> Needs {
        Needs(capabilities.into_iter().collect())
    }

    /// The build is a program to start.
    pub fn program() -> Needs {
        Needs::of([Capability::Program])
    }

    /// The build is a page to drive.
    pub fn page() -> Needs {
        Needs::of([Capability::Page])
    }

    /// The build saved a report to read.
    pub fn report() -> Needs {
        Needs::of([Capability::Report])
    }

    /// The build is a program to start and watch.
    pub fn app() -> Needs {
        Needs::of([Capability::App])
    }

    /// What is needed, by kind.
    pub fn capabilities(&self) -> &[Capability] {
        &self.0
    }

    /// Whether these needs are met by a context.
    pub fn met_by(&self, cx: &OracleCx) -> bool {
        self.0.iter().all(|c| cx.has(*c))
    }
}

impl OracleCx<'_> {
    /// Whether the context has this kind of build.
    pub fn has(&self, capability: Capability) -> bool {
        match capability {
            Capability::Program => self.program.is_some(),
            Capability::Page => self.page.is_some(),
            Capability::Report => self.report.is_some(),
            Capability::App => self.app.is_some(),
        }
    }
}

/// The build is a program on this computer, which an oracle can start.
///
/// The context holds what the build is; an oracle asks for what it needs. An oracle that starts
/// a program never has to think about a page.
pub trait RunsAProgram {
    /// The program to start.
    fn program(&self) -> &std::path::Path;
}

/// The build is a program on this computer that can be started and watched from outside.
///
/// This is the capability an app or a game needs: it has no page and no report, only a window
/// that can be read and driven.
pub trait DrivesAnApp {
    /// The program to start.
    fn app_program(&self) -> &std::path::Path;
    /// Arguments for it.
    fn app_args(&self) -> &[String];
}

/// The build is a page served somewhere, which an oracle can drive.
pub trait DrivesAPage {
    /// Where the build is served.
    fn page_url(&self) -> &str;
}

/// The build saved a report of its own, which an oracle can read.
pub trait ReadsAReport {
    /// The file the report was written to.
    fn report_path(&self) -> &std::path::Path;
}

/// The values both sides speak: the mechanics sheet, naming what each number is for.
///
/// This is the one vocabulary the original's code and the build's report have in common: the
/// original's code has numbers and function names, the build's report has numbers and step
/// names, and the sheet says which number is which.
pub trait NamesValues {
    /// The values, by the name the sheet gives them.
    fn named_values(&self) -> serde_json::Value;
}

/// The build, as a program to start.
///
/// A path is unsized, so it cannot be a trait object on its own; this holds one.
#[derive(Debug, Clone)]
pub struct TheProgram(pub std::path::PathBuf);

impl RunsAProgram for TheProgram {
    fn program(&self) -> &std::path::Path {
        self.0.as_path()
    }
}

/// The build, as a program to start and watch.
#[derive(Debug, Clone)]
pub struct TheApp {
    /// The program to start.
    pub program: std::path::PathBuf,
    /// Arguments for it.
    pub args: Vec<String>,
}

impl DrivesAnApp for TheApp {
    fn app_program(&self) -> &std::path::Path {
        self.program.as_path()
    }

    fn app_args(&self) -> &[String] {
        &self.args
    }
}

/// The build, as a page to drive.
#[derive(Debug, Clone)]
pub struct ThePage(pub String);

impl DrivesAPage for ThePage {
    fn page_url(&self) -> &str {
        &self.0
    }
}

/// The build's own report, to read.
#[derive(Debug, Clone)]
pub struct TheReport(pub std::path::PathBuf);

impl ReadsAReport for TheReport {
    fn report_path(&self) -> &std::path::Path {
        self.0.as_path()
    }
}

/// A way to run the same scripted experiment against the original and against the build, so the
/// two can be compared.
///
/// The recording and the replay are separate on purpose: the script must come from the original,
/// never from the build. A build that chooses its own test chooses its own oracle.
pub trait BehaviorOracle: Send + Sync {
    /// This oracle's id, stored with the script so a build is compared the same way.
    fn id(&self) -> &'static str;

    /// Whether this oracle can judge this target: a website for one, a program for another.
    fn supports(&self, target: &TargetRef) -> bool;

    /// What this oracle needs from the build to judge it.
    ///
    /// A caller that does not have these says so plainly, rather than the oracle finding out
    /// halfway through. The default is nothing: an oracle that only needs the original and the
    /// engine — a comparison of two recorded runs — asks for no more.
    fn needs(&self) -> Needs {
        Needs::none()
    }

    /// Record the experiment against the original. Called once, in Study. The script it returns
    /// is kept with the project and replayed, unchanged, against the build.
    ///
    /// `None` when this oracle has nothing to record for this target: a page with no control to
    /// act on, a program that will not run. Nothing is recorded, and nothing is judged.
    fn record(&self, cx: &OracleCx) -> Result<Option<Script>, ToolError>;

    /// Run the recorded script against the build and return what it saw, one observation per
    /// step.
    ///
    /// What the build is — a program to start, a page to drive, a report to read — comes from
    /// the context, which holds it. An oracle asks the context for what it needs, so an oracle
    /// that starts a program never has to think about a page.
    fn replay(&self, cx: &OracleCx, script: &Script) -> Result<Vec<Observation>, ToolError>;

    /// Compare the original's observation with the build's. The differences become claims.
    ///
    /// The script carries the original's observation and says how each reading is compared, so
    /// the build's observation is all that is needed beside it.
    fn compare(&self, script: &Script, build: &[Observation]) -> Vec<Difference> {
        compare(script, build)
    }
}

/// Compare two observations of the same script.
///
/// The probes of the script decide the comparison, so the same probe is compared the same way on
/// both sides. A probe in neither observation is unreadable: it is reported, never assumed.
pub fn compare_scripted(script: &Script, build: &[Observation]) -> Vec<Difference> {
    compare(script, build)
}

/// A scripted experiment: the steps, and how to read what came out.
///
/// Stored with the project so the same steps run against the original and against the build.
///
/// The observations are one per step, so a sequence is compared step by step: the value of a
/// sequence is in what each step changed, not only in where it ended up. A script with one
/// observation is a script of one step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Script {
    /// The format version ([`SCRIPT_VERSION`]); missing means 1.
    #[serde(default = "version")]
    pub version: u32,
    /// Which oracle recorded it, so a build is compared the same way ("console", "web", "native").
    pub oracle: String,
    /// The steps, in the oracle's own vocabulary: a terminal timeline, a list of UI actions, a
    /// list of browser actions.
    pub steps: Vec<Value>,
    /// What the oracle read back after each step, and how it is compared: the same fields, the
    /// same normalisation, the same tolerances on both sides.
    pub probes: Vec<Probe>,
    /// What the original was seen to do, one observation per step. Recorded once, in Study, and
    /// replayed against the build: this is the oracle.
    pub original: Vec<Observation>,
}

fn version() -> u32 {
    SCRIPT_VERSION
}

/// One thing the oracle reads back, and how two readings of it are compared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Probe {
    /// A stable name for what is read ("exit_code", "terminal", "a11y:Result").
    pub name: String,
    /// How two readings of this probe are compared.
    pub kind: ProbeKind,
    /// How much two readings may differ and still count as alike, when the kind allows a
    /// tolerance. A percentage (0..=100) for a likeness kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance: Option<f64>,
}

/// How two readings of one probe are compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProbeKind {
    /// The readings must be equal.
    Exact,
    /// The readings are compared as text: the share of words they have in common, in percent.
    Words,
    /// The readings are compared as pictures: their likeness, in percent.
    Picture,
    /// The readings are compared by what they say, not by their bytes: a set of labelled values.
    Fields,
}

/// What an oracle saw when it replayed a script.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Observation {
    /// Which oracle produced this, and the script version it replayed.
    pub oracle: String,
    #[serde(default = "version")]
    pub version: u32,
    /// One entry per probe, in the script's order.
    ///
    /// A probe that could not be read is `None`, not a default value: a missing reading is not a
    /// reading of nothing, and must never count as alike.
    pub readings: Vec<Option<Reading>>,
    /// Why some probes could not be read, when any could not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<String>,
}

/// One reading of one probe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Reading {
    /// The value that was read. Its shape depends on the probe: a number, a string, a list of
    /// labelled values.
    pub value: Value,
    /// How the reading was taken, when it matters ("builtin", "ghidra").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
}

/// The kind of a difference between the original's reading and the build's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DifferenceKind {
    /// The two sides read different values.
    Value,
    /// One side could read the probe and the other could not.
    Missing,
    /// Neither side could read the probe. An unknown, never a pass.
    Unreadable,
}

/// One difference between the original's observation and the build's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Difference {
    pub kind: DifferenceKind,
    /// Which probe differed.
    pub probe: String,
    /// What the original read, when it read something.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original: Option<Value>,
    /// What the build read, when it read something.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<Value>,
    /// The difference in plain words.
    pub detail: String,
}

impl Difference {
    /// Whether this difference keeps the build from being good enough. An unreadable probe does
    /// not: it is an unknown, and an unknown is reported as one.
    pub fn is_failure(&self) -> bool {
        matches!(self.kind, DifferenceKind::Value | DifferenceKind::Missing)
    }

    /// A one-line title for the claim this difference becomes.
    pub fn title(&self) -> String {
        match self.kind {
            DifferenceKind::Value => format!("The build behaves like the original at {}", self.probe),
            DifferenceKind::Missing => format!("The build shows what the original shows at {}", self.probe),
            DifferenceKind::Unreadable => format!("{} could not be compared", self.probe),
        }
    }
}

/// Compare two observations of the same script.
///
/// The probes of the script decide the comparison, so the same probe is compared the same way on
/// both sides. A probe in neither observation is unreadable: it is reported, never assumed.
pub fn compare(script: &Script, build: &[Observation]) -> Vec<Difference> {
    let mut out = Vec::new();
    for (step, (original, built)) in script.original.iter().zip(build.iter()).enumerate() {
        out.extend(compare_step(script, step, original, built));
    }
    // A step the build did not reach at all: the original's step is a difference of its own.
    for step in build.len()..script.original.len() {
        out.push(Difference {
            kind: DifferenceKind::Missing,
            probe: step_name(script, step),
            original: None,
            build: None,
            detail: format!("the original did {} but the build did not", step_name(script, step)),
        });
    }
    out
}

/// The name of a step, for a difference that names it.
fn step_name(script: &Script, step: usize) -> String {
    let name = script.steps.get(step).and_then(|s| s.get("step_id")).and_then(Value::as_str).unwrap_or("");
    if name.is_empty() { format!("step {step}") } else { format!("step {step} ({name})") }
}

/// Compare one step's readings, through the script's probes.
fn compare_step(script: &Script, step: usize, original: &Observation, built: &Observation) -> Vec<Difference> {
    let mut out = Vec::new();
    for (i, probe) in script.probes.iter().enumerate() {
        let original_reading = original.readings.get(i).and_then(|r| r.as_ref());
        let built_reading = built.readings.get(i).and_then(|r| r.as_ref());
        let difference = match (original_reading, built_reading) {
            (None, None) => Some(Difference {
                kind: DifferenceKind::Unreadable,
                probe: format!("{} at {}", probe.name, step_name(script, step)),
                original: None,
                build: None,
                detail: format!("{} could not be read on either side", probe.name),
            }),
            (Some(o), None) => Some(Difference {
                kind: DifferenceKind::Missing,
                probe: format!("{} at {}", probe.name, step_name(script, step)),
                original: Some(o.value.clone()),
                build: None,
                detail: format!("at {} the original read {} but the build could not", step_name(script, step), probe.name),
            }),
            (None, Some(b)) => Some(Difference {
                kind: DifferenceKind::Missing,
                probe: format!("{} at {}", probe.name, step_name(script, step)),
                original: None,
                build: Some(b.value.clone()),
                detail: format!("at {} the build read {} but the original could not", step_name(script, step), probe.name),
            }),
            (Some(o), Some(b)) => match probe.kind {
                ProbeKind::Exact => (o.value != b.value).then(|| Difference {
                    kind: DifferenceKind::Value,
                    probe: format!("{} at {}", probe.name, step_name(script, step)),
                    original: Some(o.value.clone()),
                    build: Some(b.value.clone()),
                    detail: format!("at {} the original read {} but the build read {}", step_name(script, step), show(&o.value), show(&b.value)),
                }),
                ProbeKind::Words => {
                    let likeness = word_likeness(&o.value, &b.value);
                    let bar = probe.tolerance.unwrap_or(100.0);
                    (likeness + f64::EPSILON < bar).then(|| Difference {
                        kind: DifferenceKind::Value,
                        probe: probe.name.clone(),
                        original: Some(o.value.clone()),
                        build: Some(b.value.clone()),
                        detail: format!("at {} the two read {likeness:.0}% alike, which is under the {bar:.0}% this probe needs", step_name(script, step)),
                    })
                }
                ProbeKind::Picture => {
                    let likeness = likeness(&o.value, &b.value);
                    let bar = probe.tolerance.unwrap_or(100.0);
                    (likeness + f64::EPSILON < bar).then(|| Difference {
                        kind: DifferenceKind::Value,
                        probe: probe.name.clone(),
                        original: Some(o.value.clone()),
                        build: Some(b.value.clone()),
                        detail: format!(
                            "at {} the two pictures are {likeness:.0}% alike, which is under the {bar:.0}% this probe needs",
                            step_name(script, step)
                        ),
                    })
                }
                ProbeKind::Fields => {
                    let missing = missing_fields(&o.value, &b.value);
                    (!missing.is_empty()).then(|| Difference {
                        kind: DifferenceKind::Value,
                        probe: probe.name.clone(),
                        original: Some(o.value.clone()),
                        build: Some(b.value.clone()),
                        detail: format!("at {} the build's reading lacks {}", step_name(script, step), missing.join(", ")),
                    })
                }
            },
        };
        out.extend(difference);
    }
    out
}

/// A number as the text both sides write it as.
///
/// A whole number is written without a fraction (`4`), so it matches what a self-test says
/// ("walked 4 m") and what a decompiled constant shows (`0x4`); a fraction keeps its decimals.
pub fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        let mut text = format!("{n}");
        if text.ends_with(".0") {
            text.truncate(text.len() - 2);
        }
        text
    }
}

/// A short form of a value, for a difference's detail.
fn show(v: &Value) -> String {
    let s = match v {
        Value::String(t) => t.clone(),
        other => other.to_string(),
    };
    let s = s.trim();
    if s.is_empty() { "nothing".to_string() } else { s.chars().take(80).collect() }
}

/// The share of words two readings have in common, in percent.
///
/// Words are compared as sets, so the order they come in does not count. A single character is
/// not a word: a reading of `4.5` and a reading of `9.0` share no words, and would otherwise
/// compare as two empty readings and look alike.
fn word_likeness(a: &Value, b: &Value) -> f64 {
    let words = |v: &Value| -> std::collections::BTreeSet<String> {
        let text = match v {
            Value::String(t) => t.clone(),
            other => other.to_string(),
        };
        text.split_whitespace()
            // A sign is part of the word: `-42` and `42` are different values, and a comparison
            // that cannot tell them apart would call a wrong answer right. Only punctuation
            // around a word goes; a leading `-` or `+` stays.
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '+').to_lowercase())
            .filter(|w| w.len() > 1)
            .collect()
    };
    let wa = words(a);
    let wb = words(b);
    if wa.is_empty() && wb.is_empty() {
        return 100.0;
    }
    if wa.is_empty() || wb.is_empty() {
        return 0.0;
    }
    let shared = wa.intersection(&wb).count() as f64;
    200.0 * shared / (wa.len() + wb.len()) as f64
}

/// The likeness of two readings that carry a likeness themselves (a picture's percent).
fn likeness(a: &Value, b: &Value) -> f64 {
    let n = |v: &Value| v.as_f64().unwrap_or(0.0);
    n(a).min(n(b))
}

/// The fields one reading has that the other lacks.
fn missing_fields(a: &Value, b: &Value) -> Vec<String> {
    let fields = |v: &Value| -> std::collections::BTreeSet<String> { v.as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default() };
    let fa = fields(a);
    let fb = fields(b);
    fa.difference(&fb).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn script(probes: Vec<Probe>, original: Vec<Option<Value>>) -> Script {
        Script {
            version: SCRIPT_VERSION,
            oracle: "test".into(),
            steps: vec![],
            probes,
            // One observation per step: each probe's reading is its own step.
            original: original.into_iter().map(|v| observation(vec![v])).collect(),
        }
    }

    fn observation(values: Vec<Option<Value>>) -> Observation {
        Observation {
            oracle: "test".into(),
            version: SCRIPT_VERSION,
            readings: values.into_iter().map(|v| v.map(|value| Reading { value, engine: None })).collect(),
            gaps: vec![],
        }
    }

    fn probe(name: &str, kind: ProbeKind) -> Probe {
        Probe { name: name.into(), kind, tolerance: None }
    }

    #[test]
    fn the_same_reading_is_no_difference() {
        let s = script(vec![probe("exit_code", ProbeKind::Exact)], vec![Some(json!(0))]);
        assert!(compare(&s, &[observation(vec![Some(json!(0))])]).is_empty());
    }

    #[test]
    fn a_different_value_is_a_failed_claim() {
        let s = script(vec![probe("exit_code", ProbeKind::Exact)], vec![Some(json!(0))]);
        let d = compare(&s, &[observation(vec![Some(json!(1))])]);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].kind, DifferenceKind::Value);
        assert!(d[0].is_failure());
        assert!(d[0].detail.contains("0") && d[0].detail.contains("1"), "{}", d[0].detail);
    }

    #[test]
    fn a_reading_the_build_could_not_take_is_a_failed_claim() {
        let s = script(vec![probe("exit_code", ProbeKind::Exact)], vec![Some(json!(0))]);
        let d = compare(&s, &[observation(vec![None])]);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].kind, DifferenceKind::Missing);
        assert!(d[0].is_failure());
    }

    /// The rule that keeps an unreadable probe from counting as alike.
    #[test]
    fn a_probe_neither_side_could_read_is_an_unknown_not_a_pass() {
        let s = script(vec![probe("screen", ProbeKind::Exact)], vec![None]);
        let d = compare(&s, &[observation(vec![None])]);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].kind, DifferenceKind::Unreadable);
        assert!(!d[0].is_failure(), "an unknown is not a failed claim");
    }

    /// A probe neither side could read is an unknown, never a pass.
    #[test]
    fn a_probe_missing_from_both_observations_is_an_unknown() {
        // One step, whose only probe has no reading on either side.
        let s = script(vec![probe("screen", ProbeKind::Exact)], vec![None]);
        let d = compare(&s, &[observation(vec![None])]);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].kind, DifferenceKind::Unreadable);
        assert!(!d[0].is_failure(), "an unknown is not a failed claim");
    }

    #[test]
    fn words_are_compared_as_a_set_and_a_single_character_is_not_a_word() {
        let s = script(vec![probe("out", ProbeKind::Words)], vec![Some(json!("4.5"))]);
        let d = compare(&s, &[observation(vec![Some(json!("9.0"))])]);
        assert_eq!(d.len(), 1, "two different numbers are not alike: {d:?}");
        assert_eq!(d[0].kind, DifferenceKind::Value);
    }

    #[test]
    fn the_same_words_in_another_order_are_alike() {
        let s = script(vec![probe("out", ProbeKind::Words)], vec![Some(json!("hello world"))]);
        assert!(compare(&s, &[observation(vec![Some(json!("world hello"))])]).is_empty());
    }

    #[test]
    fn fields_are_compared_by_what_is_missing() {
        let s = script(vec![probe("a11y", ProbeKind::Fields)], vec![Some(json!({"Result": "42", "Score": "1"}))]);
        let d = compare(&s, &[observation(vec![Some(json!({"Result": "42"}))])]);
        assert_eq!(d.len(), 1);
        assert!(d[0].detail.contains("Score"), "{}", d[0].detail);
        assert!(compare(&s, &[observation(vec![Some(json!({"Result": "42", "Score": "1"}))])]).is_empty());
    }

    #[test]
    fn a_picture_probe_needs_its_likeness_to_reach_the_bar() {
        let mut p = probe("shot", ProbeKind::Picture);
        p.tolerance = Some(90.0);
        let s = script(vec![p], vec![Some(json!(95.0))]);
        assert!(compare(&s, &[observation(vec![Some(json!(80.0))])]).len() == 1, "80% is under the 90% bar");
        assert!(compare(&s, &[observation(vec![Some(json!(95.0))])]).is_empty());
    }

    /// A sign is part of a value: `-42` and `42` are not the same answer.
    #[test]
    fn a_signed_number_is_not_the_same_as_an_unsigned_one() {
        assert!(word_likeness(&json!("Result 42 done"), &json!("Result -42 done")) < 100.0);
        assert!(word_likeness(&json!("-42"), &json!("42")) < 100.0, "a sign must count");
    }

    #[test]
    fn a_script_without_a_version_reads_as_the_current_one() {
        let raw = r#"{"oracle":"x","steps":[],"probes":[],"original":[{"oracle":"x","readings":[]}]}"#;
        let s: Script = serde_json::from_str(raw).unwrap();
        assert_eq!(s.version, SCRIPT_VERSION);
    }
}

#[cfg(test)]
mod sequences {
    use super::{DifferenceKind, Observation, Probe, ProbeKind, Reading, Script, compare};
    use serde_json::Value;
    use serde_json::json;

    fn probe(name: &str) -> Probe {
        Probe { name: name.into(), kind: ProbeKind::Exact, tolerance: None }
    }

    fn observation(value: Value) -> Observation {
        Observation { oracle: "test".into(), version: 1, readings: vec![Some(Reading { value, engine: None })], gaps: Vec::new() }
    }

    fn script(original: Vec<Observation>) -> Script {
        Script { version: 1, oracle: "test".into(), steps: vec![json!({"step_id": "one"}), json!({"step_id": "two"})], probes: vec![probe("value")], original }
    }

    /// A difference in the middle of a sequence is found, and says which step it was.
    #[test]
    fn a_difference_in_the_middle_is_found_and_named() {
        let s = script(vec![observation(json!("a")), observation(json!("b")), observation(json!("c"))]);
        let build = vec![observation(json!("a")), observation(json!("different")), observation(json!("c"))];
        let differences = compare(&s, &build);
        assert_eq!(differences.len(), 1, "{differences:?}");
        assert!(differences[0].is_failure());
        assert!(differences[0].probe.contains("step 1"), "{}", differences[0].probe);
        assert!(differences[0].detail.contains("one") || differences[0].detail.contains("step 1"), "{}", differences[0].detail);
    }

    /// A sequence that ends the same but went differently is caught: the end state alone would
    /// have missed it.
    #[test]
    fn a_sequence_that_ends_the_same_but_went_differently_is_caught() {
        let s = script(vec![observation(json!("start")), observation(json!("middle")), observation(json!("end"))]);
        let build = vec![observation(json!("start")), observation(json!("other")), observation(json!("end"))];
        let differences = compare(&s, &build);
        assert!(!differences.is_empty(), "the middle differs even though the end does not: {differences:?}");
    }

    /// A build that stopped early is missing the steps it did not reach.
    #[test]
    fn a_build_that_stopped_early_is_missing_its_steps() {
        let s = script(vec![observation(json!("a")), observation(json!("b")), observation(json!("c"))]);
        let build = vec![observation(json!("a"))];
        let differences = compare(&s, &build);
        assert!(!differences.is_empty(), "the build did fewer steps: {differences:?}");
        assert!(differences.iter().any(|d| d.is_failure()), "{differences:?}");
    }

    /// A step neither side could read is an unknown, never a pass.
    #[test]
    fn an_unreadable_step_is_an_unknown_not_a_pass() {
        let unreadable = Observation { oracle: "test".into(), version: 1, readings: vec![None], gaps: Vec::new() };
        let s = script(vec![observation(json!("a")), unreadable.clone()]);
        let build = vec![observation(json!("a")), unreadable.clone()];
        let differences = compare(&s, &build);
        assert!(differences.iter().all(|d| !d.is_failure()), "an unreadable step is not a failure: {differences:?}");
        assert!(differences.iter().any(|d| d.kind == DifferenceKind::Unreadable), "{differences:?}");
    }

    /// A sequence of one step is compared like any other.
    #[test]
    fn a_sequence_of_one_step_is_compared() {
        let mut s = script(vec![observation(json!("a"))]);
        s.steps.truncate(1);
        let differences = compare(&s, &[observation(json!("b"))]);
        assert_eq!(differences.len(), 1);
        assert!(differences[0].is_failure());
    }
}
