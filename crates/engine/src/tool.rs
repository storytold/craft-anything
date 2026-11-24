//! Tool specs: the single contract the UI, CLI, MCP server and built-in agent all dispatch through.

use crate::session::Session;
use bitflags::bitflags;
use craft_anything_evidence::{Evidence, ToolError};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

bitflags! {
    /// Side effects of a tool. Every tool declares them; MCP hints and consent are derived from them.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct Effects: u16 {
        /// Records evidence or changes session state (almost every analysis tool).
        const MUTATES_SESSION = 1 << 0;
        /// Changes the target or its annotations (rename, comment).
        const MUTATES_TARGET = 1 << 1;
        const WRITES_FILESYSTEM = 1 << 2;
        const LAUNCHES_PROCESS = 1 << 3;
        const ACCESSES_NETWORK = 1 << 4;
        const CHANGES_UI_STATE = 1 << 5;
        const MAY_DISCARD_DATA = 1 << 6;
        /// Runs code from the target or a script (process capture, Frida, walkthroughs). Needs consent.
        const EXECUTES_FOREIGN_CODE = 1 << 7;
        /// Changes a connected device (install, push, start). Needs consent.
        const MUTATES_DEVICE = 1 << 8;
        const IDEMPOTENT = 1 << 9;
        /// Its result carries content taken from the target (decompiled text, strings, bytes,
        /// rows, plist values): clients are told to treat it as untrusted data.
        const CARRIES_TARGET_CONTENT = 1 << 10;
    }
}

impl Effects {
    /// Plain evidence-producing analysis.
    pub const EVIDENCE: Effects = Effects::MUTATES_SESSION.union(Effects::IDEMPOTENT);
    /// Evidence that needs an engine process.
    pub const NATIVE_EVIDENCE: Effects = Effects::EVIDENCE.union(Effects::LAUNCHES_PROCESS);
    /// Evidence that talks to a browser or device over the network.
    pub const NETWORK_EVIDENCE: Effects = Effects::EVIDENCE.union(Effects::ACCESSES_NETWORK);
    /// Pure reads with no effect at all.
    pub const READ_ONLY: Effects = Effects::IDEMPOTENT;

    pub fn read_only_hint(self) -> bool {
        (self - Effects::IDEMPOTENT).is_empty()
    }

    pub fn destructive_hint(self) -> bool {
        self.intersects(Effects::MAY_DISCARD_DATA | Effects::EXECUTES_FOREIGN_CODE | Effects::MUTATES_DEVICE)
    }

    pub fn open_world_hint(self) -> bool {
        self.intersects(Effects::LAUNCHES_PROCESS | Effects::ACCESSES_NETWORK)
    }

    pub fn needs_consent(self) -> bool {
        self.intersects(Effects::EXECUTES_FOREIGN_CODE | Effects::MUTATES_DEVICE)
    }
}

/// Groups of tools. Agents see `Core` plus whatever toolsets they enable, which keeps their
/// context small. Every tool is always reachable from the CLI and UI. The built-in groups are
/// listed here; a plugin or a fork adds its own with [`Toolset::Custom`] (see
/// [`crate::registry::RuntimeTool`]), and [`crate::Registry::toolsets`] lists every group in use.
/// Serialized as its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Toolset {
    /// Triage, open/close, overview, search, decompile, function dossier, views, evidence, unknowns.
    Core,
    /// Deep native: listings, navigation, annotations, xrefs, bytes, call graphs, value traces.
    Native,
    /// Mach-O, code signing, plists, Swift/ObjC, Interface Builder, asset catalogs, dyld.
    Apple,
    /// Target-free formats and diagnostics: Go, WASM, EVM, PE resources, ELF layout, crashes, firmware.
    Formats,
    /// .NET assemblies.
    Managed,
    /// APK, DEX, resources, Flutter, JEB.
    Android,
    /// Java `.class` files and JAR/WAR archives: constant pool, bytecode, pseudo-Java.
    Java,
    /// A phone or emulator over adb.
    Adb,
    /// Web pages through the browser debugging protocol.
    Browser,
    /// Electron apps, JavaScript bundles and the V8 inspector.
    Javascript,
    /// Running programs: process capture, native calls, native UI.
    Runtime,
    /// Frida instrumentation.
    Frida,
    /// Application graphs, comparisons and rebuild (reconstruction) ledgers.
    Application,
    /// Databases on disk: SQLite file inspection.
    Database,
    /// What the person is making: projects, their conversation and questions.
    Projects,
    /// Setting up this computer: engines and settings.
    Setup,
    /// A builder's own hands: reading, writing and building the project it works in.
    Build,
    /// A group added at run time (its id, lower-case letters, digits and `_`).
    Custom(&'static str),
}

impl Serialize for Toolset {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.id())
    }
}

impl Toolset {
    pub const ALL: [Toolset; 17] = [
        Toolset::Core,
        Toolset::Native,
        Toolset::Apple,
        Toolset::Formats,
        Toolset::Managed,
        Toolset::Android,
        Toolset::Java,
        Toolset::Adb,
        Toolset::Browser,
        Toolset::Javascript,
        Toolset::Runtime,
        Toolset::Frida,
        Toolset::Application,
        Toolset::Database,
        Toolset::Projects,
        Toolset::Setup,
        Toolset::Build,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Toolset::Core => "core",
            Toolset::Native => "native",
            Toolset::Apple => "apple",
            Toolset::Formats => "formats",
            Toolset::Managed => "managed",
            Toolset::Android => "android",
            Toolset::Java => "java",
            Toolset::Adb => "adb",
            Toolset::Browser => "browser",
            Toolset::Javascript => "javascript",
            Toolset::Runtime => "runtime",
            Toolset::Frida => "frida",
            Toolset::Application => "application",
            Toolset::Database => "database",
            Toolset::Projects => "projects",
            Toolset::Setup => "setup",
            Toolset::Build => "build",
            Toolset::Custom(id) => id,
        }
    }

    /// A built-in group by id. Groups added at run time are found with [`crate::Registry::toolset`].
    pub fn from_id(s: &str) -> Option<Toolset> {
        Toolset::ALL.into_iter().find(|t| t.id() == s)
    }

    pub fn title(self) -> &'static str {
        match self {
            Toolset::Core => "Essentials",
            Toolset::Native => "Programs (native code)",
            Toolset::Apple => "Apple apps",
            Toolset::Formats => "File formats",
            Toolset::Managed => ".NET apps",
            Toolset::Android => "Android apps",
            Toolset::Java => "Java programs",
            Toolset::Adb => "Connected phone",
            Toolset::Browser => "Websites",
            Toolset::Javascript => "Electron and JavaScript",
            Toolset::Runtime => "Running programs",
            Toolset::Frida => "Frida instrumentation",
            Toolset::Application => "Comparing and rebuilding",
            Toolset::Database => "Databases",
            Toolset::Projects => "Projects",
            Toolset::Setup => "Engines and settings",
            Toolset::Build => "Building",
            Toolset::Custom(id) => id,
        }
    }
}

/// What a tool needs before it can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Requires {
    #[default]
    Nothing,
    /// An open target.
    Target,
    /// The Deep (Ghidra) engine to be enabled and installed.
    Ghidra,
    /// An external tool, by engine id (`adb`, `jadx`, `chromium`, `frida`, …).
    Engine(&'static str),
    /// One of these hosts, by `std::env::consts::OS` name (`macos`, `windows`, `linux`).
    Os(&'static [&'static str]),
}

/// Who invoked a tool. Programmatic callers never get dialogs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Caller {
    Ui,
    Cli,
    Mcp,
    Agent,
}

/// Set in the environment of every coding agent Craft Anything starts, and inherited by what it
/// runs, so a `craft-anything-cli` a builder starts calls tools as [`Caller::Agent`], not as the
/// person (it can't loosen a project's approvals that way).
pub const AGENT_ENV: &str = "CRAFT_ANYTHING_AGENT";

/// Cooperative cancellation, shared with long-running work (it lives in `platform`, so the
/// transports below the engine can take it too).
pub use craft_anything_platform::cancel::CancelToken;

/// Progress reports from a running tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    /// 0..=1 when known.
    pub fraction: Option<f32>,
    pub message: String,
}

pub type ProgressFn = dyn Fn(Progress) + Send + Sync;

/// Everything a tool run can touch.
pub struct Ctx<'a> {
    /// The id of the tool being run (one function can serve several tools added at run time).
    pub tool: &'static str,
    pub session: &'a mut Session,
    pub cancel: CancelToken,
    pub progress: &'a ProgressFn,
    pub caller: Caller,
}

impl Ctx<'_> {
    pub fn report(&self, fraction: Option<f32>, message: impl Into<String>) {
        (self.progress)(Progress { fraction, message: message.into() });
    }
}

/// What a tool returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ToolOutput {
    /// An evidence record; the engine stores it in the session ledger.
    Evidence(Box<Evidence>),
    /// A lifecycle or status result that is not itself evidence.
    Value(Value),
}

impl ToolOutput {
    pub fn evidence(e: Evidence) -> Self {
        ToolOutput::Evidence(Box::new(e))
    }

    /// The JSON payload agents see.
    pub fn to_json(&self) -> Value {
        match self {
            ToolOutput::Evidence(e) => serde_json::to_value(e).unwrap_or(Value::Null),
            ToolOutput::Value(v) => serde_json::json!({ "result": v }),
        }
    }
}

pub type ToolFn = fn(&mut Ctx<'_>, Value) -> Result<ToolOutput, ToolError>;

/// A registered tool.
#[derive(Clone, Copy)]
pub struct ToolSpec {
    /// snake_case id; also the MCP tool name. The CLI uses the kebab-case form.
    pub id: &'static str,
    pub title: &'static str,
    /// One or two sentences for agents and experienced users.
    pub summary: &'static str,
    /// Plain-English description for Beginner mode.
    pub beginner: &'static str,
    pub toolset: Toolset,
    pub effects: Effects,
    pub requires: Requires,
    pub input_schema: fn() -> Value,
    pub run: ToolFn,
}

impl std::fmt::Debug for ToolSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolSpec").field("id", &self.id).field("toolset", &self.toolset).finish()
    }
}

impl ToolSpec {
    pub fn cli_name(&self) -> String {
        self.id.replace('_', "-")
    }
}

/// JSON Schema for a params type.
pub fn schema_of<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Value::Null)
}

/// The schema of a tool that takes no params.
pub fn empty_schema() -> Value {
    serde_json::json!({"type": "object", "properties": {}, "additionalProperties": false})
}

/// Parse tool params; `null` is treated as `{}` so programmatic callers can omit them.
pub fn parse_params<T: DeserializeOwned>(v: Value) -> Result<T, ToolError> {
    let v = if v.is_null() { Value::Object(Default::default()) } else { v };
    serde_path_to_error::deserialize(v).map_err(|error| {
        let pointer = parameter_pointer(error.path());
        let location = pointer.as_deref().filter(|p| !p.is_empty()).map(|p| format!(" at {p:?}")).unwrap_or_default();
        ToolError::invalid(format!("invalid params{location}: {}", error.inner()))
            .with_remediation("Correct the parameter to match the tool's input schema, then call the tool again.")
            .with_details(serde_json::json!({"parameter_path": pointer}))
    })
}

/// Tool params with a number or a boolean the caller wrote as text (`"limit": "200"`, a model's
/// usual slip) read as what the tool's schema says they are. Only where the schema types a value
/// as an integer, a number or a boolean is its text rewritten: a string parameter keeps its text,
/// even when that text is `"200"`. Nested objects follow `properties`, arrays follow `items`, and
/// nothing the schema doesn't describe is touched (#1825). A nested type is usually a `$ref` into
/// the schema's own `$defs` or `definitions`, which is read through by the reference's own path.
pub fn params_as_schema(schema: &Value, params: Value) -> Value {
    let mut walk = Walk::new();
    typed_as_schema(schema, schema, &params, MAX_PARAM_DEPTH, &mut walk)
}

/// Params are trees the schema bounds, but a self-referencing schema would not end: past this
/// depth a value is taken as written.
const MAX_PARAM_DEPTH: usize = 64;
/// A schema can repeat a branch through references without repeating it in the file. This bounds
/// the whole walk, so nested alternatives cannot expand exponentially under the session lock.
const MAX_SCHEMA_STEPS: usize = 10_000;

/// The work one call may spend walking a schema. When it runs out, the walk stops converting.
struct Walk {
    left: usize,
}

impl Walk {
    fn new() -> Self {
        Walk { left: MAX_SCHEMA_STEPS }
    }

    fn step(&mut self) -> bool {
        if self.left == 0 {
            return false;
        }
        self.left -= 1;
        true
    }
}

fn typed_as_schema(schema: &Value, root: &Value, params: &Value, depth: usize, walk: &mut Walk) -> Value {
    if depth == 0 || !walk.step() {
        return params.clone();
    }
    if let Some(next) = resolve_ref(schema, root) {
        return typed_as_schema(next, root, params, depth - 1, walk);
    }
    typed_from_schemas(&[schema], root, params, depth, walk)
}

/// A value described by every matching schema: the object's properties, the array's items, or all
/// the kinds a leaf allows. Collecting every alternative before converting each leaf keeps the
/// original text when any alternative allows it, whatever order the alternatives are in.
fn typed_from_schemas(schemas: &[&Value], root: &Value, params: &Value, depth: usize, walk: &mut Walk) -> Value {
    if depth == 0 || schemas.is_empty() || !walk.step() {
        return params.clone();
    }
    match params {
        Value::String(text) => {
            if schemas.iter().any(|schema| allows_string(schema, root, 0, walk)) {
                return params.clone();
            }
            written_as_schema(schemas, root, text, params, walk)
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let mut child = Vec::new();
                    for schema in schemas {
                        property_schemas(root, schema, key, &mut child, 0, walk);
                    }
                    (key.clone(), typed_from_schemas(&child, root, value, depth - 1, walk))
                })
                .collect(),
        ),
        Value::Array(items) => {
            let mut child = Vec::new();
            for schema in schemas {
                item_schemas(root, schema, &mut child, 0, walk);
            }
            Value::Array(items.iter().map(|item| typed_from_schemas(&child, root, item, depth - 1, walk)).collect())
        }
        _ => params.clone(),
    }
}

/// The schemas that describe `key` in `schema`: its own `properties`, then object alternatives in
/// `anyOf`/`oneOf`, following local references. This is how an `Option<Struct>` field's schema
/// (`anyOf: [$ref, null]`) keeps the numbers inside the struct.
fn property_schemas<'a>(root: &'a Value, schema: &'a Value, key: &str, out: &mut Vec<&'a Value>, depth: usize, walk: &mut Walk) {
    if depth >= MAX_PARAM_DEPTH || !walk.step() {
        return;
    }
    let schema = resolve_ref(schema, root).unwrap_or(schema);
    if let Some(sub) = schema.get("properties").and_then(Value::as_object).and_then(|p| p.get(key)) {
        out.push(sub);
    }
    for alternatives in ["anyOf", "oneOf"] {
        if let Some(branches) = schema.get(alternatives).and_then(Value::as_array) {
            for branch in branches {
                property_schemas(root, branch, key, out, depth + 1, walk);
            }
        }
    }
}

/// The item schemas of `schema`: its own `items`, then array alternatives in `anyOf`/`oneOf`,
/// following local references.
fn item_schemas<'a>(root: &'a Value, schema: &'a Value, out: &mut Vec<&'a Value>, depth: usize, walk: &mut Walk) {
    if depth >= MAX_PARAM_DEPTH || !walk.step() {
        return;
    }
    let schema = resolve_ref(schema, root).unwrap_or(schema);
    if let Some(items) = schema.get("items") {
        out.push(items);
    }
    for alternatives in ["anyOf", "oneOf"] {
        if let Some(branches) = schema.get(alternatives).and_then(Value::as_array) {
            for branch in branches {
                item_schemas(root, branch, out, depth + 1, walk);
            }
        }
    }
}

/// A `$ref` into the schema itself (`#/$defs/Name`, `#/definitions/Name`), read at the path the
/// reference names, so a schema that keeps both dictionaries has each reference obey its own.
fn resolve_ref<'a>(schema: &'a Value, root: &'a Value) -> Option<&'a Value> {
    let reference = schema.get("$ref")?.as_str()?;
    let pointer = reference.strip_prefix('#')?;
    if !pointer.starts_with('/') {
        return None;
    }
    root.pointer(pointer)
}

/// A string the caller wrote. Where the schema allows text, text stays text: a number field that
/// also allows a string keeps `"200"` as written. Where it doesn't, the schema's own kinds decide:
/// an integer, number or boolean the text spells exactly is read as one. Text that reads as a
/// float for an integer field, or is too large for the type, stays as written, so the normal
/// refusal still describes what the caller sent.
fn written_as_schema(schemas: &[&Value], root: &Value, text: &str, params: &Value, walk: &mut Walk) -> Value {
    let mut kinds = Vec::new();
    for schema in schemas {
        schema_kinds(schema, root, 0, &mut kinds, walk);
    }
    if kinds.contains(&"string") {
        return params.clone();
    }
    let numeric = kinds.iter().any(|kind| matches!(*kind, "integer" | "number"));
    if numeric
        && text.trim() == text
        && !text.is_empty()
        && let Ok(Value::Number(number)) = serde_json::from_str::<Value>(text)
    {
        // A number field takes a float; an integer field takes only an integer that fits the type.
        let fits = kinds.contains(&"number") || number.is_i64() || number.is_u64();
        if fits {
            return Value::Number(number);
        }
    }
    if kinds.contains(&"boolean") {
        match text {
            "true" => return Value::Bool(true),
            "false" => return Value::Bool(false),
            _ => {}
        }
    }
    params.clone()
}

/// Every type a schema allows, whether written as one word or a list, through local references
/// and `anyOf`/`oneOf` alternatives.
fn schema_kinds<'a>(schema: &'a Value, root: &'a Value, depth: usize, out: &mut Vec<&'a str>, walk: &mut Walk) {
    if depth >= MAX_PARAM_DEPTH || !walk.step() {
        return;
    }
    let schema = resolve_ref(schema, root).unwrap_or(schema);
    match schema.get("type") {
        Some(Value::String(kind)) => out.push(kind.as_str()),
        Some(Value::Array(kinds)) => out.extend(kinds.iter().filter_map(Value::as_str)),
        _ => {}
    }
    for alternatives in ["anyOf", "oneOf"] {
        if let Some(branches) = schema.get(alternatives).and_then(Value::as_array) {
            for branch in branches {
                schema_kinds(branch, root, depth + 1, out, walk);
            }
        }
    }
}

/// Whether a schema allows text anywhere, through local references and alternatives. Text a
/// schema allows is never rewritten, even when it also allows a number. A schema with no `type`
/// and no alternatives accepts any value, text included.
fn allows_string(schema: &Value, root: &Value, depth: usize, walk: &mut Walk) -> bool {
    if depth >= MAX_PARAM_DEPTH || !walk.step() {
        return true;
    }
    let schema = resolve_ref(schema, root).unwrap_or(schema);
    let typed = schema.get("type");
    let own = match typed {
        Some(Value::String(kind)) => kind == "string",
        Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind.as_str() == Some("string")),
        _ => false,
    };
    if own {
        return true;
    }
    let alternatives = ["anyOf", "oneOf"];
    if alternatives.iter().any(|key| {
        schema.get(*key).and_then(Value::as_array).is_some_and(|branches| branches.iter().any(|branch| allows_string(branch, root, depth + 1, walk)))
    }) {
        return true;
    }
    // No `type`: the schema permits any JSON value, including text.
    typed.is_none()
}

/// A JSON Pointer to the rejected value, without guessing when Serde cannot locate it.
fn parameter_pointer(path: &serde_path_to_error::Path) -> Option<String> {
    const LIMIT: usize = 1024;
    let mut pointer = String::new();
    for segment in path {
        pointer.push('/');
        match segment {
            serde_path_to_error::Segment::Map { key } if key.len() <= LIMIT => {
                pointer.push_str(&key.replace('~', "~0").replace('/', "~1"));
            }
            serde_path_to_error::Segment::Seq { index } => pointer.push_str(&index.to_string()),
            // Enum variants need not be object keys (for example, internally tagged enums).
            _ => return None,
        }
        if pointer.len() > LIMIT {
            return None;
        }
    }
    Some(pointer)
}

#[cfg(test)]
mod parameter_tests {
    use super::*;
    use craft_anything_evidence::ErrorCode;
    use serde_json::json;

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct Question {
        question: String,
        #[serde(default)]
        probes: Vec<String>,
    }

    #[test]
    fn an_array_for_a_string_identifies_the_parameter_to_correct() {
        for input in [json!({"question": ["Where?"]}), json!({"question": ["Where?", "Why?"]})] {
            let error = parse_params::<Question>(input).unwrap_err();
            assert_eq!(error.code, ErrorCode::BadInput);
            assert!(error.message.contains("question") && error.message.contains("expected a string"), "{error:?}");
            assert_eq!(error.details.as_ref().unwrap()["parameter_path"], "/question");
            assert!(error.remediation.as_deref().unwrap().contains("call the tool again"));
            assert!(!error.message.contains("Where?"), "the parameter value is not needed to locate the error");
        }
        let valid = parse_params::<Question>(json!({"question": "Where?", "probes": ["inspect"]})).unwrap();
        assert_eq!(valid, Question { question: "Where?".into(), probes: vec!["inspect".into()] });
    }

    /// #1825: params follow the schema: a number or a boolean written as text is read as one,
    /// a string stays text even when it reads as a number, and undescribed values are untouched.
    #[test]
    fn a_number_the_schema_types_is_read_as_one_and_text_stays_text() {
        let schema = json!({
            "type": "object",
            "properties": {
                "offset": {"type": "integer"},
                "ratio": {"type": ["number", "null"]},
                "deep": {"type": "boolean"},
                "note": {"type": "string"},
                "pages": {"type": "array", "items": {"type": "integer"}}
            }
        });
        let typed = params_as_schema(&schema, json!({"offset": "592", "ratio": "1.5", "deep": "true", "note": "200", "pages": ["1", 2], "other": "3"}));
        assert_eq!(typed, json!({"offset": 592, "ratio": 1.5, "deep": true, "note": "200", "pages": [1, 2], "other": "3"}));
        // Text that isn't the value the schema wants stays as written, so the error still says
        // what is wrong where.
        let typed = params_as_schema(&schema, json!({"deep": "yes", "note": "1"}));
        assert_eq!(typed, json!({"deep": "yes", "note": "1"}));
        assert!(parse_params::<Question>(params_as_schema(&json!({"type": "object"}), json!({"question": "Where?"}))).is_ok());

        // A nested type is a `$ref` into the schema's own `$defs`.
        let nested = json!({
            "type": "object",
            "properties": {"range": {"$ref": "#/$defs/ByteRange"}},
            "$defs": {"ByteRange": {"type": "object", "properties": {"offset": {"type": "integer"}, "length": {"type": "integer"}}}}
        });
        let typed = params_as_schema(&nested, json!({"range": {"offset": "592", "length": "1024"}}));
        assert_eq!(typed, json!({"range": {"offset": 592, "length": 1024}}));

        // A schema that keeps both dictionaries is read by each reference's own path: the one into
        // `definitions` wants text, the one into `$defs` a number.
        let both = json!({
            "type": "object",
            "properties": {"a": {"$ref": "#/definitions/Value"}, "b": {"$ref": "#/$defs/Value"}},
            "definitions": {"Value": {"type": "string"}},
            "$defs": {"Value": {"type": "integer"}}
        });
        let typed = params_as_schema(&both, json!({"a": "200", "b": "200"}));
        assert_eq!(typed, json!({"a": "200", "b": 200}));

        // A list of types is read as a set: the order never decides, and a value the schema also
        // allows as text keeps its text.
        let either = json!({"type": "object", "properties": {"maybe": {"type": ["null", "integer"]}, "or_text": {"type": ["integer", "string"]}}});
        let typed = params_as_schema(&either, json!({"maybe": "200", "or_text": "200"}));
        assert_eq!(typed, json!({"maybe": 200, "or_text": "200"}));
    }

    #[test]
    fn integer_fields_take_only_text_that_is_an_integer_that_fits() {
        let schema = json!({"type": "object", "properties": {
            "offset": {"type": "integer"},
            "ratio": {"type": "number"},
            "either": {"type": ["integer", "number"]}
        }});
        let typed = params_as_schema(
            &schema,
            json!({
                "offset": "-12",
                "ratio": "1.5",
                "either": "1e3",
            }),
        );
        assert_eq!(typed, json!({"offset": -12, "ratio": 1.5, "either": 1000.0}));
        // A float, exponent form or a value too large for the field stays as written, so the
        // refusal names the text the caller actually sent.
        let typed = params_as_schema(
            &schema,
            json!({
                "offset": "18446744073709551616",
                "ratio": "1e3",
                "either": "1e3",
            }),
        );
        assert_eq!(typed, json!({"offset": "18446744073709551616", "ratio": 1000.0, "either": 1000.0}));
    }

    #[test]
    fn optional_object_and_nested_alternatives_keep_their_numbers() {
        // The real `Option<ByteRange>` schema from `firmware_regions_list`.
        let registry = crate::Registry::with_all_tools();
        let spec = registry.get("firmware_regions_list").unwrap();
        let schema = registry.input_schema(spec);
        let typed = params_as_schema(&schema, json!({"path": "image.bin", "range": {"offset": "0", "length": "4096"}, "limit": "5"}));
        assert_eq!(typed, json!({"path": "image.bin", "range": {"offset": 0, "length": 4096}, "limit": 5}));

        // An object inside `oneOf`, an array inside `anyOf`, and a string field in an alternative
        // all keep their own behaviour.
        let schema = json!({"type": "object", "properties": {
            "range": {"oneOf": [{"type": "object", "properties": {"offset": {"type": "integer"}}}, {"type": "null"}]},
            "windows": {"anyOf": [{"type": "array", "items": {"type": "object", "properties": {"start": {"type": "integer"}}}}, {"type": "null"}]},
            "label": {"anyOf": [{"type": "object", "properties": {"text": {"type": "string"}}}, {"type": "null"}]}
        }});
        let typed = params_as_schema(&schema, json!({"range": {"offset": "0"}, "windows": [{"start": "8"}], "label": {"text": "200"}}));
        assert_eq!(typed, json!({"range": {"offset": 0}, "windows": [{"start": 8}], "label": {"text": "200"}}));
    }

    #[test]
    fn alternatives_are_collected_before_each_leaf_is_converted() {
        // The same field in two object alternatives: a string in either one keeps the text,
        // whatever order the alternatives are in.
        for branches in [
            json!([
                {"type": "object", "properties": {"payload": {"type": "object", "properties": {"value": {"type": "integer"}}}}},
                {"type": "object", "properties": {"payload": {"type": "object", "properties": {"value": {"type": "string"}}}}}
            ]),
            json!([
                {"type": "object", "properties": {"payload": {"type": "object", "properties": {"value": {"type": "string"}}}}},
                {"type": "object", "properties": {"payload": {"type": "object", "properties": {"value": {"type": "integer"}}}}}
            ]),
        ] {
            let schema = json!({"anyOf": branches});
            assert_eq!(params_as_schema(&schema, json!({"payload": {"value": "200"}})), json!({"payload": {"value": "200"}}));
        }

        // A string alternative does not stop an object value from having its own fields read.
        let schema = json!({"anyOf": [
            {"type": "string"},
            {"type": "object", "properties": {"count": {"type": "integer"}}}
        ]});
        assert_eq!(params_as_schema(&schema, json!({"count": "5"})), json!({"count": 5}));

        // An alternative without a type accepts any value, text included.
        let schema = json!({"anyOf": [{"type": "integer"}, {}]});
        assert_eq!(params_as_schema(&schema, json!("200")), json!("200"));
    }

    #[test]
    fn repeated_schema_branches_stop_at_the_work_limit() {
        let mut defs = serde_json::Map::new();
        defs.insert("N0".into(), json!({"type": "integer"}));
        for level in 1..=30 {
            let previous = format!("#/$defs/N{}", level - 1);
            defs.insert(format!("N{level}"), json!({"anyOf": [{"$ref": previous.clone()}, {"$ref": previous}]}));
        }
        let schema = json!({"type": "object", "properties": {"n": {"$ref": "#/$defs/N30"}}, "$defs": defs});
        // Without the walk bound this takes about 2^30 steps. With it, the text is left as sent.
        assert_eq!(params_as_schema(&schema, json!({"n": "200"})), json!({"n": "200"}));
    }

    #[test]
    fn a_nested_parameter_error_keeps_the_array_index() {
        let error = parse_params::<Question>(json!({"question": "Where?", "probes": ["inspect", ["trace"]]})).unwrap_err();
        assert_eq!(error.details.as_ref().unwrap()["parameter_path"], "/probes/1");
        assert!(error.message.contains("expected a string"), "{error:?}");
    }

    #[test]
    fn a_parameter_path_escapes_json_pointer_keys() {
        #[derive(Debug, Deserialize)]
        struct Envelope {
            #[serde(rename = "a~/b")]
            _questions: Vec<Question>,
        }
        let error = parse_params::<Envelope>(json!({"a~/b": [{"question": ["Where?"]}]})).unwrap_err();
        assert_eq!(error.details.as_ref().unwrap()["parameter_path"], "/a~0~1b/0/question");
    }

    #[test]
    fn oversized_parameter_paths_are_not_reported_as_complete_pointers() {
        let input = json!({"x".repeat(2048): {"question": ["Where?"]}});
        let error = parse_params::<std::collections::BTreeMap<String, Question>>(input).unwrap_err();
        assert!(error.details.as_ref().unwrap()["parameter_path"].is_null());
        assert!(error.message.len() < 200);
        assert!(error.message.contains("expected a string"));
    }

    #[test]
    fn an_enum_path_does_not_guess_a_json_object_location() {
        #[derive(Debug, Deserialize)]
        enum Choice {
            Question(Question),
        }
        let error = parse_params::<Choice>(json!({"Question": {"question": ["Where?"]}})).unwrap_err();
        assert!(error.details.as_ref().unwrap()["parameter_path"].is_null());
        assert!(error.message.contains("expected a string"));
        let Choice::Question(valid) = parse_params::<Choice>(json!({"Question": {"question": "Where?"}})).unwrap();
        assert_eq!(valid.question, "Where?");
    }

    #[test]
    fn parameter_defaults_and_strictness_are_preserved() {
        #[derive(Debug, Default, Deserialize, PartialEq)]
        struct Defaults {
            #[serde(default)]
            count: u32,
        }
        assert_eq!(parse_params::<Defaults>(Value::Null).unwrap(), Defaults::default());
        assert_eq!(parse_params::<Defaults>(json!({"count": 4})).unwrap().count, 4);
        for input in [json!({"count": -1}), json!({"count": "4"}), json!({"count": u64::MAX})] {
            let error = parse_params::<Defaults>(input).unwrap_err();
            assert_eq!(error.details.as_ref().unwrap()["parameter_path"], "/count");
        }
        let missing = parse_params::<Question>(json!({"probes": []})).unwrap_err();
        assert!(missing.message.contains("missing field `question`"));
        assert_eq!(missing.details.as_ref().unwrap()["parameter_path"], "", "the known location is the parent object, not an invented missing value");
        let extra = parse_params::<Question>(json!({"question": "Where?", "surprise": true})).unwrap_err();
        assert!(extra.message.contains("unknown field `surprise`"));
    }
}
