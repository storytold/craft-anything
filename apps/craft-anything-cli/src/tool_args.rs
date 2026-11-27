//! Turn `craft-anything-cli <tool> [path] [--json '{..}' | @file.json] [--key value…]` into tool params,
//! using the tool's JSON schema to type each flag.

use craft_anything_engine::{Requires, ToolSpec};
use serde_json::{Map, Value, json};

/// A parsed tool invocation.
#[derive(Debug, Default, PartialEq)]
pub struct ToolInvocation {
    pub params: Value,
    /// Path to open first (for tools that need an open target).
    pub open_path: Option<String>,
    /// Extra `core_target_open` params for that path (`format`, `architecture`, `entry_point`, `language_id`).
    pub open_params: Map<String, Value>,
    /// The output format asked for after the tool name (`--output-format`, or `--format` when the
    /// tool has no `format` option of its own).
    pub output_format: Option<String>,
    pub help: bool,
    /// `--version` or `-V`.
    pub version: bool,
}

/// A tool command line that can't be turned into the tool's parameters: a usage error.
#[derive(Debug, thiserror::Error)]
pub enum ToolArgsError {
    #[error("--{0} needs a value")]
    NeedsValue(String),
    #[error("{command} has no option --{flag}. Options: {known}")]
    UnknownOption { command: String, flag: String, known: String },
    #[error("--{flag} expects a number, got {got:?}")]
    ExpectsNumber { flag: String, got: String },
    #[error("--{flag} expects a whole number, got {got:?}")]
    ExpectsWholeNumber { flag: String, got: String },
    #[error("--{flag} expects true or false, got {got:?}")]
    ExpectsBool { flag: String, got: String },
    #[error("--{flag} expects JSON: {source}")]
    ExpectsJson { flag: String, source: serde_json::Error },
    #[error("params must be a JSON object: {0}")]
    ParamsNotJson(#[source] serde_json::Error),
    #[error("params must be a JSON object")]
    ParamsNotObject,
    #[error("can't read {path}: {source}")]
    ReadFile { path: String, source: std::io::Error },
    #[error("--json - reads parameters from a pipe, but standard input is a terminal; pipe the JSON in, or pass it inline or as @file.json")]
    StdinIsTerminal,
    #[error("can't read parameters from standard input: {0}")]
    StdinRead(#[source] std::io::Error),
    #[error("parameters on standard input are over the limit of {limit} bytes; use @file.json")]
    StdinTooBig { limit: u64 },
    #[error("--{0} was given more than once")]
    Duplicate(String),
    #[error("--{0} is also set by --json or an @file.json; give each parameter once")]
    Conflict(String),
    #[error("--{flag} needs a value, not the option {value}; to pass a value that starts with -, write --{flag}={value}")]
    ValueLooksLikeFlag { flag: String, value: String },
    #[error("unknown option {0}; put an argument that starts with - after --")]
    UnknownFlag(String),
    #[error("{command} doesn't take another argument ({arg:?})")]
    ExtraArgument { command: String, arg: String },
}

/// `core_target_open` options a target tool accepts for its `<target-path>` (raw binaries, Deep pins).
const OPEN_OPTIONS: [&str; 3] = ["architecture", "entry_point", "language_id"];
/// `--format` values that pick how the target is opened rather than how output is printed.
const OPEN_FORMATS: [&str; 2] = ["raw-binary", "dos-com"];

fn schema_props(schema: &Value) -> Map<String, Value> {
    schema.get("properties").and_then(Value::as_object).cloned().unwrap_or_default()
}

fn required(schema: &Value) -> Vec<String> {
    schema.get("required").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// The JSON type of a property ("string", "integer", "number", "boolean", "array", "object"), skipping "null".
fn prop_type(schema: &Value, prop: &Value) -> String {
    let resolved = resolve_ref(schema, prop);
    match resolved.get("type") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).find(|t| *t != "null").unwrap_or("string").to_string(),
        _ => {
            // anyOf/oneOf of [X, null] is X; a real choice (a string or an object) is "any", so the
            // value keeps whatever JSON the person wrote.
            for key in ["anyOf", "oneOf"] {
                if let Some(arr) = resolved.get(key).and_then(Value::as_array) {
                    let mut types = arr.iter().map(|alt| prop_type(schema, alt)).filter(|t| t != "null");
                    match (types.next(), types.next()) {
                        (Some(t), None) => return t,
                        (Some(_), Some(_)) => return "any".into(),
                        (None, _) => {}
                    }
                }
            }
            if resolved.get("enum").is_some() { "string".into() } else { "any".into() }
        }
    }
}

fn resolve_ref<'a>(schema: &'a Value, prop: &'a Value) -> &'a Value {
    if let Some(r) = prop.get("$ref").and_then(Value::as_str) {
        let name = r.rsplit('/').next().unwrap_or_default();
        for defs in ["$defs", "definitions"] {
            if let Some(d) = schema.get(defs).and_then(|d| d.get(name)) {
                return d;
            }
        }
    }
    prop
}

fn convert(ty: &str, key: &str, raw: &str) -> Result<Value, ToolArgsError> {
    let flag = || key.to_string();
    let got = || raw.to_string();
    match ty {
        "integer" => {
            let t = raw.trim();
            if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                // `from_str_radix` accepts a leading sign, so check the digits first.
                let digits = !h.is_empty() && h.bytes().all(|b| b.is_ascii_hexdigit());
                u64::from_str_radix(h, 16).ok().filter(|_| digits).map(|n| json!(n)).ok_or_else(|| ToolArgsError::ExpectsNumber { flag: flag(), got: got() })
            } else {
                t.parse::<i64>().map(|n| json!(n)).map_err(|_| ToolArgsError::ExpectsWholeNumber { flag: flag(), got: got() })
            }
        }
        "number" => {
            raw.trim().parse::<f64>().ok().filter(|n| n.is_finite()).map(|n| json!(n)).ok_or_else(|| ToolArgsError::ExpectsNumber { flag: flag(), got: got() })
        }
        "boolean" => match raw.trim() {
            "true" | "yes" | "1" => Ok(json!(true)),
            "false" | "no" | "0" => Ok(json!(false)),
            _ => Err(ToolArgsError::ExpectsBool { flag: flag(), got: got() }),
        },
        "object" => serde_json::from_str::<Value>(raw).map_err(|source| ToolArgsError::ExpectsJson { flag: flag(), source }),
        "array" => Ok(serde_json::from_str::<Value>(raw).ok().filter(Value::is_array).unwrap_or_else(|| json!([raw]))),
        "string" => Ok(json!(raw)),
        _ => Ok(serde_json::from_str::<Value>(raw).unwrap_or_else(|_| json!(raw))),
    }
}

/// Read inline JSON or `@file.json`.
pub fn read_json_arg(raw: &str) -> Result<Value, ToolArgsError> {
    let text = if raw == "-" {
        read_stdin_json(std::io::stdin())?
    } else {
        match raw.strip_prefix('@') {
            Some(path) => std::fs::read_to_string(path).map_err(|source| ToolArgsError::ReadFile { path: path.to_string(), source })?,
            None => raw.to_string(),
        }
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    serde_json::from_str(text).map_err(ToolArgsError::ParamsNotJson)
}

/// Most bytes of parameters taken from standard input.
const MAX_STDIN_JSON: u64 = 64 << 20;

/// Parameters piped in (`--json -`). Never waits at a terminal: nobody is going to type JSON there.
fn read_stdin_json<R: std::io::Read + std::io::IsTerminal>(input: R) -> Result<String, ToolArgsError> {
    if input.is_terminal() {
        return Err(ToolArgsError::StdinIsTerminal);
    }
    read_bounded_json(input)
}

fn read_bounded_json<R: std::io::Read>(input: R) -> Result<String, ToolArgsError> {
    use std::io::Read as _;
    let mut text = String::new();
    let read = input.take(MAX_STDIN_JSON + 1).read_to_string(&mut text).map_err(ToolArgsError::StdinRead)?;
    if read as u64 > MAX_STDIN_JSON {
        return Err(ToolArgsError::StdinTooBig { limit: MAX_STDIN_JSON });
    }
    Ok(text)
}

/// Whether `v` is meant as a flag rather than a value: it starts with `-`, and is neither `-` (standard
/// input) nor a negative number.
fn looks_like_flag(v: &str) -> bool {
    v.starts_with('-') && v != "-" && v.parse::<f64>().is_err()
}

/// A value the flag `key` takes: the part after `=`, or the next argument, which must not itself be
/// a flag (a value that starts with `-` is written `--key=-value`).
fn take_value(args: &[String], i: &mut usize, key: &str, inline: Option<String>) -> Result<String, ToolArgsError> {
    if let Some(v) = inline {
        return Ok(v);
    }
    let v = args.get(*i).ok_or_else(|| ToolArgsError::NeedsValue(key.to_string()))?;
    if looks_like_flag(v) {
        return Err(ToolArgsError::ValueLooksLikeFlag { flag: key.to_string(), value: v.clone() });
    }
    *i += 1;
    Ok(v.clone())
}

/// Records that an option was given, and refuses it the second time.
pub fn once(seen: &mut Vec<String>, key: &str) -> Result<(), ToolArgsError> {
    if seen.iter().any(|s| s == key) {
        return Err(ToolArgsError::Duplicate(key.to_string()));
    }
    seen.push(key.to_string());
    Ok(())
}

/// Turns the words after a tool's name into its parameters, checking them against the tool's schema
/// before anything runs: an unknown, repeated or conflicting option, a value that is another flag, a
/// wrong type and malformed JSON are all errors (RFC-0018 rule 1.4). `--` ends the options.
pub fn parse(spec: &ToolSpec, schema: &Value, args: &[String]) -> Result<ToolInvocation, ToolArgsError> {
    let props = schema_props(schema);
    let mut params = Map::new();
    // Keys given by `--json`/`@file`, so an option can't silently change one of them.
    let mut from_json: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut positionals = Vec::new();
    let mut inv = ToolInvocation::default();
    let mut i = 0usize;
    let mut options_ended = false;
    while let Some(a) = args.get(i) {
        i += 1;
        if options_ended {
            positionals.push(a.clone());
            continue;
        }
        if a == "--" {
            options_ended = true;
            continue;
        }
        if a == "--help" || a == "-h" {
            inv.help = true;
            continue;
        }
        if a == "--version" || a == "-V" {
            inv.version = true;
            continue;
        }
        let long = a.strip_prefix("--").map(|rest| match rest.split_once('=') {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (rest.to_string(), None),
        });
        // The tool's parameters as JSON: `--json '{..}'`, `--json=@file.json`, `--json -`; `--params` is the old name.
        if let Some((key, inline)) = long.as_ref().filter(|(k, _)| (k == "json" || k == "params") && !props.contains_key(k)) {
            once(&mut seen, "json")?;
            let raw = take_json_value(args, &mut i, key, inline.clone())?;
            merge(&mut params, &mut from_json, read_json_arg(&raw)?)?;
            continue;
        }
        if let Some((key, inline)) = long {
            let snake = key.replace('-', "_");
            let is_target = spec.requires == Requires::Target;
            // Output format and target-open options, unless the tool has an option of the same name.
            if !props.contains_key(&snake) && (snake == "output_format" || snake == "format" || (is_target && OPEN_OPTIONS.contains(&snake.as_str()))) {
                once(&mut seen, if snake == "format" { "output_format" } else { &snake })?;
                let raw = take_value(args, &mut i, &key, inline)?;
                if snake == "output_format" || (snake == "format" && !(is_target && OPEN_FORMATS.contains(&raw.as_str()))) {
                    inv.output_format = Some(raw);
                } else {
                    inv.open_params.insert(snake, json!(raw));
                }
                continue;
            }
            let prop = props.get(&snake).ok_or_else(|| {
                let known: Vec<String> = props.keys().map(|k| format!("--{}", k.replace('_', "-"))).collect();
                ToolArgsError::UnknownOption {
                    command: spec.cli_name().to_string(),
                    flag: key.clone(),
                    known: if known.is_empty() { "(none)".into() } else { known.join(", ") },
                }
            })?;
            let ty = prop_type(schema, prop);
            // A list option is repeatable; every other option is given once.
            if ty != "array" {
                once(&mut seen, &snake)?;
            }
            if from_json.contains(&snake) {
                return Err(ToolArgsError::Conflict(key));
            }
            let raw = match inline {
                Some(v) => v,
                // A yes/no option alone means yes; it only takes the next word when that is true or false.
                None if ty == "boolean" && args.get(i).is_none_or(|n| !is_bool_word(n)) => "true".to_string(),
                None => take_value(args, &mut i, &key, None)?,
            };
            let value = convert(&ty, &key, &raw)?;
            if ty == "array"
                && let Some(Value::Array(existing)) = params.get_mut(&snake)
                && let Value::Array(more) = value
            {
                existing.extend(more);
                continue;
            }
            params.insert(snake, value);
            continue;
        }
        if looks_like_flag(a) && !a.starts_with('@') {
            return Err(ToolArgsError::UnknownFlag(a.clone()));
        }
        if let Some(path) = a.strip_prefix('@')
            && path.ends_with(".json")
        {
            merge(&mut params, &mut from_json, read_json_arg(a)?)?;
            continue;
        }
        positionals.push(a.clone());
    }
    // An architecture or entry point only means something for a raw binary.
    if (inv.open_params.contains_key("architecture") || inv.open_params.contains_key("entry_point")) && !inv.open_params.contains_key("format") {
        inv.open_params.insert("format".into(), json!("raw-binary"));
    }
    let mut positionals = positionals.into_iter();
    if spec.requires == Requires::Target
        && let Some(p) = positionals.next()
    {
        inv.open_path = Some(p);
    }
    // Remaining positionals fill required string params in order, then an optional path (what a bare
    // argument almost always is, whatever other options sort before it), then other string params.
    let mut slots: Vec<String> = required(schema).into_iter().filter(|k| !params.contains_key(k)).collect();
    if !slots.iter().any(|k| k == "path") && !params.contains_key("path") && props.get("path").is_some_and(|v| prop_type(schema, v) == "string") {
        slots.push("path".into());
    }
    for k in props.keys() {
        if !slots.contains(k) && !params.contains_key(k) && prop_type(schema, props.get(k).unwrap_or(&Value::Null)) == "string" {
            slots.push(k.clone());
        }
    }
    let mut slots = slots.into_iter();
    for p in positionals {
        let slot = slots.next().ok_or_else(|| ToolArgsError::ExtraArgument { command: spec.cli_name().to_string(), arg: p.clone() })?;
        let ty = prop_type(schema, props.get(&slot).unwrap_or(&Value::Null));
        params.insert(slot.clone(), convert(&ty, &slot.replace('_', "-"), &p)?);
    }
    inv.params = Value::Object(params);
    Ok(inv)
}

/// Whether `--json`'s next word is the tool's parameters (inline JSON, `@file` or `-`) rather than another argument.
pub fn is_json_params(next: &str) -> bool {
    next.starts_with('{') || next.starts_with('@') || next == "-"
}

fn is_bool_word(word: &str) -> bool {
    matches!(word.trim(), "true" | "yes" | "1" | "false" | "no" | "0")
}

/// The value of `--json`: after `=`, or the next argument (`-` is standard input).
fn take_json_value(args: &[String], i: &mut usize, key: &str, inline: Option<String>) -> Result<String, ToolArgsError> {
    if let Some(v) = inline {
        return Ok(v);
    }
    let v = args.get(*i).ok_or_else(|| ToolArgsError::NeedsValue(key.to_string()))?;
    *i += 1;
    Ok(v.clone())
}

/// Adds a JSON object's keys to the parameters. Two `--json` objects can't give the same key.
fn merge(into: &mut Map<String, Value>, keys: &mut Vec<String>, v: Value) -> Result<(), ToolArgsError> {
    match v {
        Value::Object(m) => {
            for (k, v) in m {
                if into.contains_key(&k) {
                    return Err(ToolArgsError::Conflict(k.replace('_', "-")));
                }
                keys.push(k.clone());
                into.insert(k, v);
            }
            Ok(())
        }
        _ => Err(ToolArgsError::ParamsNotObject),
    }
}

/// Help text for one tool, generated from its schema.
/// The fields inside an object option (or an array of objects), listed under it as
/// `option.field`, two levels deep and at most 30 a level.
fn nested_fields(schema: &Value, prop: &Value, path: &str, depth: usize, out: &mut String) {
    if depth > 2 {
        return;
    }
    let mut obj = resolve_ref(schema, prop);
    // Option<T> arrives as anyOf/oneOf [T, null].
    if let Some(alt) = ["anyOf", "oneOf"]
        .iter()
        .find_map(|k| obj.get(*k).and_then(Value::as_array))
        .and_then(|a| a.iter().find(|x| x.get("type").and_then(Value::as_str) != Some("null")))
    {
        obj = resolve_ref(schema, alt);
    }
    if obj.get("type").and_then(Value::as_str) == Some("array")
        && let Some(items) = obj.get("items")
    {
        obj = resolve_ref(schema, items);
    }
    let Some(fields) = obj.get("properties").and_then(Value::as_object) else { return };
    let req = required(obj);
    for (name, f) in fields.iter().take(30) {
        let ty = prop_type(schema, f);
        let desc = f.get("description").and_then(Value::as_str).unwrap_or_default();
        let star = if req.contains(name) { " (required)" } else { "" };
        let field = format!("{path}.{name}");
        out.push_str(&format!("      {field:<26} {ty}{star}  {desc}\n"));
        nested_fields(schema, f, &field, depth + 1, out);
    }
}

pub fn help(spec: &ToolSpec, schema: &Value) -> String {
    let props = schema_props(schema);
    let req = required(schema);
    let mut s = format!("{} — {}\n\n{}\n\nUsage: craft-anything-cli {}", spec.cli_name(), spec.title, spec.summary, spec.cli_name());
    if spec.requires == Requires::Target {
        s.push_str(" <target-path>");
    }
    s.push_str(" [options] [--json '{...}' | --json=@params.json | --json -] [--] [arguments]\n");
    s.push_str("\nEach option may be given once (list options repeat). A value that starts with - is written --option=-value; -- ends the options.\n");
    s.push_str("--json with parameters after it sets the tool's parameters; a bare --json is --format json.\n");
    if !props.is_empty() {
        s.push_str("\nOptions:\n");
        for (k, p) in &props {
            let ty = prop_type(schema, p);
            let desc = p.get("description").and_then(Value::as_str).unwrap_or_default();
            let star = if req.contains(k) { " (required)" } else { "" };
            s.push_str(&format!("  --{:<28} {ty}{star}  {desc}\n", k.replace('_', "-")));
            nested_fields(schema, p, k, 1, &mut s);
        }
    }
    if spec.requires == Requires::Target {
        s.push_str("\nTarget options (how <target-path> is opened):\n");
        s.push_str("  --architecture                 x86, x86_64, arm or arm64 for a raw binary (implies --format raw-binary)\n");
        s.push_str("  --entry-point                  Load/entry address (0x-hex) for a raw binary\n");
        s.push_str("  --format                       raw-binary or dos-com to force how the file is read\n");
        s.push_str("  --language-id                  Deep mode only: pin a Ghidra processor language\n");
    }
    let own_format = props.contains_key("format");
    s.push_str(&format!(
        "\nOutput:\n  --output-format                json, jsonl, md, text, csv or tsv{}\n",
        if own_format { " (--format after the tool name is this tool's own option)" } else { " (--format works too)" }
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_anything_engine::Registry;

    // The tests use built-in tools, whose schema is their own type's.
    fn parse(spec: &ToolSpec, args: &[String]) -> Result<ToolInvocation, ToolArgsError> {
        super::parse(spec, &(spec.input_schema)(), args)
    }
    fn help(spec: &ToolSpec) -> String {
        super::help(spec, &(spec.input_schema)())
    }

    #[test]
    fn integer_hex_rejects_sign() {
        for bad in ["0x+10", "0x-10", "0x", "0x 10"] {
            let e = super::convert("integer", "addr", bad).unwrap_err();
            assert!(e.to_string().contains("expects a number"), "{bad}: {e}");
        }
        assert_eq!(super::convert("integer", "addr", "0x10").unwrap(), json!(16));
    }

    #[test]
    fn number_rejects_non_finite() {
        for raw in ["NaN", "inf", "-inf", "infinity"] {
            let e = convert("number", "x", raw).expect_err(raw);
            assert!(e.to_string().contains("expects a number"), "{e}");
        }
        assert_eq!(convert("number", "x", "1.5").unwrap(), json!(1.5));
    }

    #[test]
    fn help_lists_csv_and_tsv_output_formats() {
        let r = Registry::with_all_tools();
        let h = help(r.get("core_journal_read").unwrap());
        assert!(h.lines().any(|line| line.contains("--output-format") && line.contains("csv") && line.contains("tsv")), "{h}");
    }

    #[test]
    fn help_lists_fields_inside_object_options() {
        let r = Registry::with_all_tools();
        let h = help(r.get("app_rebuild_claim_check").unwrap());
        assert!(h.contains("specification.goal"), "{h}");
        assert!(h.contains("specification.expectations.evidence_ids"), "array items are listed too: {h}");
        let plan = help(r.get("core_plan_update").unwrap());
        assert!(plan.contains("      steps."), "{plan}");
        let flat = help(r.get("core_target_open").unwrap());
        assert!(!flat.contains("      path."), "plain options have no nested lines");
    }

    #[test]
    fn a_union_of_two_types_takes_any_json() {
        let schema = json!({"type": "object", "properties": {"left": {"anyOf": [{"type": "string"}, {"type": "object"}]}}});
        assert_eq!(prop_type(&schema, &schema["properties"]["left"]), "any");
        // Option<T> is anyOf [T, null]: still T.
        assert_eq!(prop_type(&schema, &json!({"anyOf": [{"type": "string"}, {"type": "null"}]})), "string");
        // A lone null branch says nothing: fall back to "any".
        assert_eq!(prop_type(&schema, &json!({"anyOf": [{"type": "null"}]})), "any");
    }

    #[test]
    fn an_evidence_input_takes_a_bare_id_or_an_object() {
        let r = Registry::with_all_tools();
        let spec = r.get("runtime_captures_compare").unwrap();
        let inv = parse(spec, &["--left".into(), r#"{"kind":"saved","evidence_id":"ev_1"}"#.into(), "--right".into(), "ev_2".into()]).unwrap();
        assert!(inv.params["left"].is_object(), "{}", inv.params);
        assert_eq!(inv.params["right"], json!("ev_2"));
        assert!(help(spec).contains("any (required)"), "{}", help(spec));
    }

    #[test]
    fn flags_positionals_and_json() {
        let r = Registry::with_all_tools();
        let open = r.get("core_target_open").unwrap();
        let inv = parse(open, &["/bin/ls".into()]).unwrap();
        assert_eq!(inv.params["path"], "/bin/ls");
        assert!(inv.open_path.is_none());

        let upd = r.get("core_plan_update").unwrap();
        let inv = parse(upd, &["--json".into(), r#"{"steps":[]}"#.into(), "--based-on-revision".into(), "3".into()]).unwrap();
        assert_eq!(inv.params, json!({"steps": [], "based_on_revision": 3}));
        assert!(parse(upd, &["--nope".into(), "1".into()]).unwrap_err().to_string().contains("--steps"));
        assert!(parse(upd, &["--based-on-revision".into(), "x".into()]).is_err());

        let close = r.get("core_target_close").unwrap();
        let inv = parse(close, &["/tmp/a.bin".into()]).unwrap();
        assert_eq!(inv.open_path.as_deref(), Some("/tmp/a.bin"));

        let rj = r.get("core_journal_read").unwrap();
        let inv = parse(rj, &["--limit=5".into(), "--kind".into(), "tool_call".into()]).unwrap();
        assert_eq!(inv.params, json!({"limit": 5, "kind": "tool_call"}));
        assert!(parse(rj, &["--help".into()]).unwrap().help);
        assert!(help(rj).contains("--limit"));
    }

    #[test]
    fn format_flag_goes_to_the_tool_its_target_or_the_output() {
        let r = Registry::with_all_tools();
        // A tool with its own `format` option keeps it; the output format has its own flag.
        let open = r.get("core_target_open").unwrap();
        let inv = parse(open, &["--format".into(), "raw-binary".into(), "--output-format".into(), "text".into()]).unwrap();
        assert_eq!(inv.params["format"], "raw-binary");
        assert_eq!(inv.output_format.as_deref(), Some("text"));
        assert!(help(open).contains("this tool's own option"));
        // Without one, --format is the output format...
        let rj = r.get("core_journal_read").unwrap();
        let inv = parse(rj, &["--format=md".into()]).unwrap();
        assert_eq!(inv.output_format.as_deref(), Some("md"));
        assert_eq!(inv.params, json!({}));
        // ...and on a target tool, raw-binary/dos-com and the raw options describe how to open the path.
        let pc = r.get("native_function_pseudocode").unwrap();
        let args: Vec<String> = ["/tmp/raw.bin", "--architecture", "x86_64", "--entry-point", "0x1000", "--function", "0x1000", "--format", "text"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let inv = parse(pc, &args).unwrap();
        assert_eq!(inv.open_path.as_deref(), Some("/tmp/raw.bin"));
        assert_eq!(Value::Object(inv.open_params.clone()), json!({"architecture": "x86_64", "entry_point": "0x1000", "format": "raw-binary"}));
        assert_eq!(inv.output_format.as_deref(), Some("text"));
        let inv = parse(pc, &["/tmp/a.com".into(), "--format".into(), "dos-com".into()]).unwrap();
        assert_eq!(inv.open_params["format"], "dos-com");
        assert!(inv.output_format.is_none());
        assert!(help(pc).contains("--entry-point"));
    }

    #[test]
    fn at_file_input() {
        let dir = std::env::temp_dir().join(format!("cany-cli-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("p.json");
        let r = Registry::with_all_tools();
        for contents in [r#"{"question":"q?"}"#, "\u{feff}{\"question\":\"q?\"}"] {
            std::fs::write(&f, contents).unwrap();
            let inv = parse(r.get("core_brief_write").unwrap(), &[format!("@{}", f.display())]).unwrap();
            assert_eq!(inv.params["question"], "q?");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parameters_from_a_pipe_are_bounded() {
        assert_eq!(read_bounded_json(&b"{\"a\":1}"[..]).unwrap(), "{\"a\":1}");
        let big = std::io::repeat(b' ').take(MAX_STDIN_JSON + 10);
        use std::io::Read as _;
        assert!(read_bounded_json(big).unwrap_err().to_string().contains("over the limit"));
        assert!(read_bounded_json(&[0xffu8, 0xfe][..]).is_err(), "bad text is an error");
    }

    /// A tool whose schema has one option of each kind, to pin the strict rules (RFC-0018 rule 1.4).
    fn strict(args: &[&str]) -> Result<ToolInvocation, ToolArgsError> {
        let r = Registry::with_all_tools();
        let spec = r.get("core_journal_read").unwrap();
        let schema = json!({"type": "object", "properties": {
            "name": {"type": "string"},
            "limit": {"type": "integer"},
            "tag": {"type": "array", "items": {"type": "string"}},
            "verbose": {"type": "boolean"},
            "path": {"type": "string"},
        }});
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        super::parse(spec, &schema, &args)
    }

    #[test]
    fn an_option_given_twice_is_refused_unless_it_is_a_list() {
        for args in [
            &["--name", "a", "--name", "b"][..],
            &["--limit=1", "--limit", "2"],
            &["--verbose", "--verbose"],
            &["--json", "{}", "--json", "{}"],
            &["--output-format", "md", "--format", "text"],
        ] {
            let e = strict(args).unwrap_err();
            assert!(matches!(e, ToolArgsError::Duplicate(_)), "{args:?}: {e}");
        }
        // A list option is repeatable and collects every value.
        assert_eq!(strict(&["--tag", "a", "--tag=b"]).unwrap().params, json!({"tag": ["a", "b"]}));
    }

    #[test]
    fn a_value_is_never_the_next_flag() {
        let e = strict(&["--name", "--verbose"]).unwrap_err();
        assert!(matches!(e, ToolArgsError::ValueLooksLikeFlag { .. }), "{e}");
        assert!(e.to_string().contains("--name=--verbose"), "{e}");
        // `=` passes a value that starts with `-`; a negative number and `-` are values.
        assert_eq!(strict(&["--name=-x"]).unwrap().params, json!({"name": "-x"}));
        assert_eq!(strict(&["--limit", "-5"]).unwrap().params, json!({"limit": -5}));
        assert_eq!(strict(&["--name", "-"]).unwrap().params, json!({"name": "-"}));
        assert!(matches!(strict(&["--name"]).unwrap_err(), ToolArgsError::NeedsValue(_)));
    }

    #[test]
    fn a_short_flag_the_tool_lacks_is_refused_and_double_dash_ends_the_options() {
        assert!(matches!(strict(&["-x"]).unwrap_err(), ToolArgsError::UnknownFlag(_)));
        assert!(matches!(strict(&["--nope"]).unwrap_err(), ToolArgsError::UnknownOption { .. }));
        // After `--` an argument that starts with `-` is a positional (here the `name` slot).
        let inv = strict(&["--", "--odd"]).unwrap();
        assert_eq!(inv.params, json!({"path": "--odd"}));
        assert_eq!(strict(&["--", "-x", "-y"]).unwrap().params, json!({"path": "-x", "name": "-y"}));
        assert!(matches!(strict(&["--", "a", "b", "c"]).unwrap_err(), ToolArgsError::ExtraArgument { .. }));
    }

    #[test]
    fn an_option_cannot_silently_change_a_parameter_given_as_json() {
        let e = strict(&["--json", r#"{"limit":3}"#, "--limit", "4"]).unwrap_err();
        assert!(matches!(e, ToolArgsError::Conflict(_)), "{e}");
        // Whichever comes first.
        let e = strict(&["--limit", "4", "--json", r#"{"limit":3}"#]).unwrap_err();
        assert!(matches!(e, ToolArgsError::Conflict(_)), "{e}");
        // Different keys merge.
        assert_eq!(strict(&["--json", r#"{"limit":3}"#, "--name", "n"]).unwrap().params, json!({"limit": 3, "name": "n"}));
        // `--json=` takes the parameters inline, from a file or from standard input like `--json`.
        assert_eq!(strict(&["--json={\"limit\":3}"]).unwrap().params, json!({"limit": 3}));
        assert!(matches!(strict(&["--json", "[1]"]).unwrap_err(), ToolArgsError::ParamsNotObject));
        assert!(matches!(strict(&["--json", "{oops"]).unwrap_err(), ToolArgsError::ParamsNotJson(_)));
    }

    #[test]
    fn a_yes_no_option_alone_means_yes_and_leaves_the_next_word_alone() {
        assert_eq!(strict(&["--verbose", "somewhere"]).unwrap().params, json!({"verbose": true, "path": "somewhere"}));
        assert_eq!(strict(&["--verbose", "false"]).unwrap().params, json!({"verbose": false}));
        assert_eq!(strict(&["--verbose=no"]).unwrap().params, json!({"verbose": false}));
        assert!(matches!(strict(&["--verbose=maybe"]).unwrap_err(), ToolArgsError::ExpectsBool { .. }));
    }

    #[test]
    fn version_is_asked_for_like_help() {
        assert!(strict(&["--version"]).unwrap().version);
        assert!(strict(&["-V"]).unwrap().version);
    }
}
