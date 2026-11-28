//! pi (`pi`) as a coding agent: one message in on stdin, its JSONL event stream out.
//!
//! `pi -p --mode json` runs without the terminal UI and writes one JSON object per line: a session
//! header, `message_update` deltas carrying the text, `tool_execution_start`/`tool_execution_end`
//! for each call, `message_end` with the turn's usage, and `agent_settled` at the end. The session
//! header's `id` continues the same conversation with `--session-id`, creating it the first time
//! and resuming the project's own copy afterwards (`--session` is pi's global lookup by path or id,
//! which can reach another folder's conversation, so it isn't used here). Checked against pi 1.6.0
//! (2026-11-25); 1.5.0 read the same records.
//!
//! pi has no flag for a tool server the way other agents do: servers come from a project's
//! `mcp.json` (which needs the project trusted) and from extensions. Our tool server therefore
//! arrives as a one-function extension this run loads with `-e`, registering it with
//! `exposure: "direct"` so its tools are declared to the model instead of being kept behind
//! `codemode`. The file is written into a folder of the run's own, owner-only on Unix, and removed
//! when the turn ends, so nothing another user could prepare in a shared temporary folder is ever
//! loaded through and nothing is left behind.
//!
//! `--no-extensions` keeps a person's and a project's own extensions from loading beside ours (one
//! could otherwise shield pi's changing tools from the bridge). It also turns off pi's built-in
//! extensions, MCP support among them since 0.99, so that one is loaded back by name
//! (`-e builtin:mcp`) or `registerMcpServer` fails. `--no-context-files` keeps AGENTS.md and
//! CLAUDE.md out, and `--no-approve` keeps trust-gated project files out even where a trust
//! decision was saved.
//!
//! `--tools` with an `mcp__` entry filters MCP tools as well as built-in ones (pi's own documented
//! behaviour), so a builder keeps our tools plus reading only: it has no `edit`, `write` or `bash`
//! of its own, and every file change and command goes through the tools whose effects, evidence
//! and consent Craft Anything records. `codemode` and `tool_search` are left out on purpose:
//! `codemode` calls a server's tools by name whatever the active tool set says, so leaving it in
//! would reach the servers this run isn't meant to use.
//!
//! pi has no `--permission-prompt-tool`, so the same extension holds the bridge: its `tool_call`
//! handler asks Craft Anything's own `core_action_approve` about every tool that changes something
//! (pi's `bash`, `powershell`, `write`, `edit`) and blocks the call when the policy refuses or the
//! person says no. The action's input goes in on stdin (a written file's content can be far past
//! Windows' command-line limit), and the call has a deadline longer than the approval's own wait,
//! after which the action is blocked. Reads and our own tools pass straight through: ours carry
//! their own consent. With that in place a Deep build may use pi's own editor and shell, as a
//! Claude Code build does.
//!
//! A run over SSH is refused: the extension file is written on this computer, so a pi on the host
//! would be handed a path it can't see. `sh`-style quoting is not involved anywhere: the extension
//! is JSON, and the message never enters argv (Windows refuses long command lines, and its limit
//! is far below a build's brief).

use super::*;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// The name our extension registers the server under. Not `craft-anything`, which a person's own
/// `mcp.json` entry may use: pi refuses two clients with one name, and a `mcp.json` entry would
/// win over the extension's.
const SERVER: &str = "craft-anything-run";

/// The name our tools reach the model under: pi turns every character other than letters, digits
/// and `_` into `_`, so `craft-anything-run` becomes `craft_anything_run`.
const TOOL_PREFIX: &str = "mcp__craft_anything_run__";

/// The built-in tools a builder keeps beside ours: reading, and in Deep mode the editor and shell
/// the extension's approval bridge gates. pi has no `TodoWrite`.
const READ_TOOLS: &[&str] = &["read", "grep", "find", "ls"];
const CHANGING_TOOLS: &[&str] = &["edit", "write", "bash", "powershell"];

/// The extension's file name inside a run's folder.
const EXTENSION_FILE: &str = "craft-anything-tools.js";

/// Saved extra arguments that decide where a run's mode, tools, extensions or session come from.
/// A saved `--tools` or `--extension` could re-open what this adapter holds shut (a Clean room
/// build's file tools, another extension beside ours), so every run drops them and passes its own.
/// #2118 proposed dropping such saved arguments; this is #1447's own list, applied to every run.
const RESTRICTED: &[&str] = &[
    "--mode",
    "--session",
    "--session-id",
    "--fork",
    "--session-dir",
    "--no-session",
    "--continue",
    "--resume",
    "--tools",
    "--exclude-tools",
    "--no-tools",
    "--no-builtin-tools",
    "--extension",
    "--no-extensions",
    "--no-mcp",
    "--approve",
    "--no-approve",
    "--no-context-files",
];

/// The short forms of [`RESTRICTED`]. `-p`, `-c` and `-r` would start another kind of run.
const RESTRICTED_SHORT: &[&str] = &["-p", "-c", "-r", "-t", "-xt", "-e", "-ne", "-nt", "-nbt", "-na", "-a"];

/// [`RESTRICTED`] entries that take the next argument as their value.
const RESTRICTED_VALUES: &[&str] =
    &["--mode", "--session", "--session-id", "--fork", "--session-dir", "--tools", "--exclude-tools", "--extension", "-t", "-xt", "-e"];

/// `extra` without the flags (and their values) that would override what this adapter decides.
/// Everything after a bare `--` goes too: pi would read it as the message, not as options.
fn strip_restricted_args(extra: &[String]) -> Vec<String> {
    let mut kept = Vec::new();
    let mut rest = extra.iter();
    while let Some(arg) = rest.next() {
        if arg == "--" {
            break;
        }
        let name = arg.split('=').next().unwrap_or(arg);
        if RESTRICTED.contains(&name) || RESTRICTED_SHORT.contains(&name) {
            // A value given with `=` goes with its flag; one given after it is the next argument.
            if !arg.contains('=') && RESTRICTED_VALUES.contains(&name) {
                let _ = rest.next();
            }
            continue;
        }
        kept.push(arg.clone());
    }
    kept
}

/// The file a preflight writes and removes to prove the folder takes an extension.
const PROBE_FILE: &str = "probe.js";

/// pi run as a coding agent.
pub struct PiCli;

impl AgentAdapter for PiCli {
    fn ids(&self) -> &'static [&'static str] {
        &["pi"]
    }

    /// The message goes in on stdin: `pi -p` reads it there when no prompt is given, which keeps a
    /// build's brief out of argv and away from Windows' command-line limit.
    fn prompt_on_stdin(&self) -> bool {
        true
    }

    fn streams_json(&self) -> bool {
        true
    }

    fn answers_questions(&self) -> bool {
        true
    }

    fn resumes(&self) -> bool {
        true
    }

    /// A question starts a fresh session on purpose (no `--session-id`), so it is never answered
    /// from a fork of the builder's conversation. #2118 proposed the hook; #1447's adapter answers
    /// it, so a question's answer isn't labelled as the builder's.
    fn questions_fork_session(&self) -> bool {
        false
    }

    /// `--help` lists `--mode`, `--session-id` and the switches that keep other extensions and
    /// project files out of a run; without them the plain form would boot the terminal UI in the
    /// middle of a build, so a failed check keeps this adapter.
    fn required_flags(&self) -> (&'static [&'static str], &'static [&'static str]) {
        (&["--help"], &["--mode", "--session-id", "--no-extensions", "--no-context-files", "--no-approve"])
    }

    /// The flags are older than every pi a build has been run with; one without them would load
    /// the person's and the project's own extensions beside ours, so the run stops instead.
    fn missing_flags_error(&self) -> Option<(&'static str, &'static str)> {
        Some((
            "This pi version doesn't have the switches a build needs to keep other extensions and project files out of it.",
            "Update pi (`pi update`), then start the build again.",
        ))
    }

    fn plain_fallback(&self) -> bool {
        false
    }

    /// A remote build would be handed a path on this computer: the extension file is written here,
    /// and pi on the host would never see it. Refused plainly until the host gets its own server.
    fn check_remote_options(&self, _options: &BackendOptions, _host: &str, _stop: &Stop) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::NotSupported,
            "Pi can't build on another computer over SSH yet: its tool server and approval bridge are files this computer writes for the run.",
        )
        .with_remediation("Build on this computer, or choose an agent whose tool server is written on the host (OpenCode), and try again."))
    }

    fn check_tool_server(&self, stop: &CancelToken) -> Result<(), ToolError> {
        cli_can_start(craft_anything_platform::self_cli_path().as_deref(), stop)?;
        // The folder and the write a run uses, once, before the build starts: a temporary folder
        // that can't take the extension stops the build here with a fix, not part-way through one.
        let dir = extension_dir()?;
        let written = craft_anything_platform::write_atomic_private(&dir.join(PROBE_FILE), b"// probe\n");
        let _ = std::fs::remove_dir_all(&dir);
        written
    }

    fn model_in_use(&self, options: Option<&BackendOptions>) -> Option<String> {
        Some(options.and_then(|o| o.cli_model.clone()).filter(|m| !m.trim().is_empty()).unwrap_or_else(|| "pi's own default".into()))
    }

    /// pi lists its own models, with the thinking depths each one actually exposes.
    fn has_model_catalog(&self) -> bool {
        true
    }

    /// The models a run can use, asked of pi itself: `--mode rpc` answers `get_available_models`
    /// with each model's thinking levels, and `--offline --list-models` is the fallback for a pi
    /// whose RPC answer didn't come. Both are asked with `--no-extensions`, because every run
    /// turns extensions off: a model an extension supplied could not be used by a build.
    fn model_catalog(
        &self,
        config: &BackendConfig,
        options: &BackendOptions,
        directory: Option<&Path>,
        stop: &Stop,
    ) -> Result<Vec<models::AgentModel>, ToolError> {
        stop.check()?;
        if config.ssh_host.as_deref().is_some_and(|h| !h.trim().is_empty()) {
            return Err(ToolError::new(
                ErrorCode::NotSupported,
                "Pi's models can't be listed on another computer over SSH yet: a run there is refused for the same reason.",
            )
            .with_remediation("Choose Pi on this computer, or remove the SSH host from this AI and try again."));
        }
        let program = program_of(config, options)?;
        let extra = strip_restricted_args(&options.args);
        let mut rpc = vec!["--mode".to_string(), "rpc".into(), "--no-session".into(), "--offline".into(), "--no-extensions".into()];
        rpc.extend(extra.iter().cloned());
        if let Ok(out) = query(&program, &rpc, Some(MODELS_REQUEST), directory, stop)
            && out.success
            && let Some(list) = parse_rpc_models(&out.stdout)?
        {
            return Ok(list);
        }
        let mut list = vec!["--offline".to_string(), "--list-models".into(), "--no-extensions".into()];
        list.extend(extra);
        let out = query(&program, &list, None, directory, stop)?;
        if !out.success {
            let name = Path::new(&program).file_name().map_or_else(|| program.clone(), |n| n.to_string_lossy().into_owned());
            return Err(craft_anything_platform::process::nonzero_error_with(&name, &out, None));
        }
        parse_list_models(&out.stdout)
    }

    /// The chosen model and thinking depth must be ones pi offers this run: a saved choice that
    /// pi no longer has (or a depth it doesn't expose) stops with a plain fix instead of being
    /// passed on and refused mid-build.
    fn validate_model(&self, config: &BackendConfig, options: &BackendOptions, directory: Option<&Path>, stop: &Stop) -> Result<(), ToolError> {
        stop.check()?;
        self.check_native_options(options, directory)?;
        if options.cli_model.is_none() && options.cli_thinking.is_none() {
            return Ok(());
        }
        models::validate(options, &self.model_catalog(config, options, directory, stop)?)
    }

    /// Sign-in belongs to the provider pi was set up with, and checking it needs a provider or a
    /// model name the adapter isn't given. The run itself reports what the provider refused.
    fn sign_in_fix(&self) -> Option<String> {
        Some("Sign in the way pi does: run `pi` once in a terminal, sign in to the provider its default model uses, then start the build again.".into())
    }

    /// pi keeps each session as `sessions/--<folder>--/<timestamp>_<id>.jsonl` under its own
    /// folder, so a project can keep a copy that travels with it.
    fn session_file(&self, id: &str) -> Option<std::path::PathBuf> {
        agent_dir().and_then(|dir| session_file_in(&dir, id))
    }

    fn handoff(&self, saved: &str, max: usize) -> Option<String> {
        pi_handoff(saved, max)
    }

    fn restore_session(&self, id: &str, saved: &[u8], cwd: &std::path::Path) -> Result<bool, ToolError> {
        let dir = agent_dir().ok_or_else(|| ToolError::new(ErrorCode::BadSettings, "Couldn't find pi's folder on this computer."))?;
        restore_session_in(&dir, id, saved, cwd)
    }

    /// The runner owns this turn's private extension folder and removes it when the turn ends.
    fn run_files(&self) -> bool {
        true
    }

    fn args(&self, cx: &ArgsCx) -> Vec<String> {
        let mut a = strip_restricted_args(&cx.options.args);
        a.extend(["-p".into(), "--mode".into(), "json".into(), "--append-system-prompt".into(), cx.preamble.to_string()]);
        if let Some(model) = cx.options.cli_model.clone().filter(|m| !m.trim().is_empty()) {
            a.extend(["--model".into(), model]);
        }
        if let Some(effort) = cx.options.cli_thinking {
            a.extend(["--thinking".into(), thinking_level(effort).into()]);
        }
        // Nothing of the person's and nothing of the project's loads beside this run: no
        // discovered extensions (a user's, or one a builder wrote into the project), no AGENTS.md
        // or CLAUDE.md, and no trust-gated project files.
        a.push("--no-extensions".into());
        a.extend(["--no-context-files".into(), "--no-approve".into()]);
        if cx.question {
            // A question only reads, and a fresh, ephemeral session leaves the builder's own
            // conversation and pi's session list untouched: no `--session-id`, no session file,
            // and no server of anyone's connected.
            a.extend(["--no-session".into(), "--no-mcp".into()]);
            if cx.reads_checked {
                a.push("--no-tools".into());
            } else {
                a.extend(["--tools".into(), READ_TOOLS.join(",")]);
            }
            return a;
        }
        // `--no-extensions` turns off pi's built-in extensions too, its MCP support among them,
        // so that one is loaded back by name: without it `registerMcpServer` fails and a build has
        // no Craft Anything tools at all. Our server arrives only through the extension below.
        a.extend(["-e".into(), "builtin:mcp".into(), "-e".into(), extension_file(cx.run_dir, &builder_server(cx.mcp))]);
        let mut tools: Vec<String> = Vec::new();
        // In Clean room the original may only be read through our tools, so no file tools of the
        // agent's own: the same rule its other adapters hold it to. In Deep mode a builder edits
        // and runs with pi's own tools, and the extension's `tool_call` bridge asks Craft Anything
        // about each of those calls.
        if !cx.reads_checked {
            tools.extend(READ_TOOLS.iter().map(|t| (*t).to_string()));
            tools.extend(CHANGING_TOOLS.iter().map(|t| (*t).to_string()));
        }
        tools.push(format!("{TOOL_PREFIX}*"));
        a.extend(["--tools".into(), tools.join(",")]);
        if let Some(resume) = cx.resume {
            a.extend(["--session-id".into(), resume.to_string()]);
        }
        a
    }

    /// A resumed conversation whose file pi can't find is started again under the same id, with
    /// only a warning on stderr. Unlike an agent that stops, pi answers, and the id it was given
    /// is now the project's conversation: it must be kept (clearing it would start yet another
    /// session on the next turn), and the person is told the earlier one wasn't here.
    fn parse_stderr(&self, _p: &mut StreamParser, line: &str) -> Vec<AgentEvent> {
        let lower = line.to_ascii_lowercase();
        let lost = lower.contains("no project session found with id") || lower.contains("no session found matching");
        if lost {
            return vec![AgentEvent::Status {
                text: "The builder's earlier conversation isn't on this computer, so pi started a new one under the project's session id.".into(),
            }];
        }
        Vec::new()
    }

    /// Each JSON line: the session header's id, text deltas, tool calls and their results, the
    /// turn's usage, and a retry that ended in failure as an error in plain words.
    fn parse(&self, p: &mut StreamParser, v: &Value) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default();
        match s("type") {
            "session" => {
                if !s("id").is_empty() {
                    p.session_id = Some(s("id").to_string());
                    p.run_session = p.session_id.clone();
                }
            }
            "message_update" => {
                if let Some(update) = v.get("assistantMessageEvent")
                    && update.get("type").and_then(Value::as_str) == Some("text_delta")
                    && let Some(delta) = update.get("delta").and_then(Value::as_str)
                {
                    out.push(AgentEvent::TextDelta { text: delta.to_string() });
                }
            }
            "message_end" => {
                let Some(message) = v.get("message") else { return out };
                if message.get("role").and_then(Value::as_str) != Some("assistant") {
                    return out;
                }
                p.saw_result = true;
                if let Some(usage) = message.get("usage") {
                    account(p, usage);
                }
                if let Some(why) = message.get("errorMessage").and_then(Value::as_str) {
                    out.push(failed(p, why));
                }
            }
            "tool_execution_start" => {
                let id = s("toolCallId").to_string();
                // The run's own server is named so it can't clash with a person's entry, which
                // makes its tools longer than the ids the rest of Craft Anything matches on.
                let name = s("toolName");
                let name = name.strip_prefix(TOOL_PREFIX).unwrap_or(name);
                out.push(p.started(&id, name, v.get("args").cloned().unwrap_or(Value::Null)));
            }
            "tool_execution_end" => {
                let id = s("toolCallId").to_string();
                let ok = !v.get("isError").and_then(Value::as_bool).unwrap_or(false);
                let text = result_text(v.get("result"));
                out.push(p.finished(&id, ok, &text));
            }
            "auto_retry_end" if v.get("success").and_then(Value::as_bool) == Some(false) => {
                let why = s("finalError");
                let why = if why.is_empty() { "the provider stopped answering".to_string() } else { why.to_string() };
                out.push(failed(p, &why));
            }
            _ => {}
        }
        out
    }
}

/// pi's own name for a thinking level: its lowest is `off`, where Craft Anything's is `none`.
/// `--thinking none` is refused and falls back to pi's default instead of choosing the lowest.
fn thinking_level(effort: crate::Effort) -> &'static str {
    match effort {
        crate::Effort::None => "off",
        other => other.as_str(),
    }
}

/// Add one assistant message's usage to the turn's totals. pi counts cached reads separately, and
/// its `cost` is what an API at list prices would have charged, not what a plan was billed.
fn account(p: &mut StreamParser, usage: &Value) {
    let n = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
    p.input_tokens = p.input_tokens.saturating_add(n("input")).saturating_add(n("cacheRead"));
    p.output_tokens = p.output_tokens.saturating_add(n("output"));
    if let Some(total) = usage.get("cost").and_then(|c| c.get("total")).and_then(Value::as_f64)
        && total > 0.0
    {
        p.cost_usd = Some(p.cost_usd.unwrap_or(0.0) + total);
    }
}

/// A run that ended in failure, as a typed failure and a message with the fix.
fn failed(p: &mut StreamParser, why: &str) -> AgentEvent {
    let kind = classify(why);
    p.failure.get_or_insert(TurnFailure::new(kind));
    AgentEvent::Error { message: format!("pi stopped before it answered: {}", why.trim()), remediation: Some(remediation(kind).to_string()), log: None }
}

/// The plain-words failure behind a provider's message, for the stop rules and the retry policy.
fn classify(why: &str) -> FailureKind {
    let why = why.to_ascii_lowercase();
    if has_code(&why, "401") || why.contains("unauthor") || why.contains("api key") || why.contains("invalid key") || why.contains("not signed in") {
        FailureKind::Authentication
    } else if has_code(&why, "402") || why.contains("quota") || why.contains("credit") || why.contains("billing") || why.contains("insufficient") {
        FailureKind::Billing
    } else if has_code(&why, "429") || why.contains("rate limit") || why.contains("too many requests") {
        FailureKind::RateLimited
    } else if why.contains("usage limit") || why.contains("plan limit") || why.contains("5-hour limit") || why.contains("weekly limit") {
        FailureKind::UsageLimit
    } else if has_code(&why, "529") || has_code(&why, "503") || why.contains("timeout") || why.contains("timed out") || why.contains("overload") {
        FailureKind::Transient
    } else {
        FailureKind::Unknown
    }
}

/// Whether `text` carries `code` as a status code, not as part of a longer number: a provider's
/// error code `14018` contains `401`, and reading it as one would call a used-up plan a bad key.
fn has_code(text: &str, code: &str) -> bool {
    let bytes = text.as_bytes();
    text.match_indices(code).any(|(i, _)| {
        let before = i.checked_sub(1).is_none_or(|j| !bytes[j].is_ascii_digit());
        let after = bytes.get(i + code.len()).is_none_or(|b| !b.is_ascii_digit());
        before && after
    })
}

/// What to do about a failure, in the words a person can act on.
fn remediation(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::Authentication => "Sign in to the provider pi uses: run `pi` once in a terminal, sign in, then start the build again.",
        FailureKind::Billing => "Check the account's plan, credits and spending limit in the provider pi uses, then start the build again.",
        FailureKind::RateLimited => "Wait a minute, or choose another model in Settings, then start the build again.",
        FailureKind::UsageLimit => {
            "This plan's allowance is used up for now: it comes back in hours, not minutes. Wait for the reset time the provider names, or choose another model in Settings, then start the build again."
        }
        FailureKind::Transient => "This looks temporary: the build's next attempt may get through. If it keeps happening, choose another model in Settings.",
        FailureKind::MissingProgram | FailureKind::ToolsUnavailable | FailureKind::Configuration | FailureKind::Unknown => {
            "Run `pi` once in this project's folder to see the same answer without Craft Anything, then start the build again."
        }
    }
}

/// pi's RPC command that answers with every configured model and the thinking levels each
/// actually exposes (`thinkingLevelMap`).
const MODELS_REQUEST: &str = "{\"type\":\"get_available_models\"}\n";

/// Most bytes of a model list this adapter reads.
const MODELS_MAX_BYTES: usize = 4 * 1024 * 1024;

/// Longest a model-catalogue query may take.
const QUERY_TIMEOUT: Duration = Duration::from_secs(15);

/// Run one of pi's own queries (its model catalogue), with a deadline, drained output and the
/// run's stop.
fn query(
    program: &str,
    args: &[String],
    stdin: Option<&str>,
    directory: Option<&std::path::Path>,
    stop: &Stop,
) -> Result<craft_anything_platform::Output, ToolError> {
    let mut proc = PiCli.process(program).args(args.to_vec());
    if let Some(dir) = directory {
        proc = proc.cwd(dir);
    }
    if let Some(input) = stdin {
        proc = proc.stdin_bytes(input);
    }
    let stop = stop.clone();
    proc.timeout(QUERY_TIMEOUT)
        .idle_timeout(Duration::from_secs(10))
        .stdout_cap(MODELS_MAX_BYTES)
        .stderr_cap(64 * 1024)
        .cancel_when(move || stop.is_stopped())
        .remediation("Check Pi on this computer (`pi --list-models`), then reload the models.")
        .run()
}

/// The `get_available_models` answer, as the shared model picker's entries. `None` when the
/// answer's line isn't there, so the caller can ask the table instead.
pub(crate) fn parse_rpc_models(bytes: &[u8]) -> Result<Option<Vec<models::AgentModel>>, ToolError> {
    if bytes.len() >= MODELS_MAX_BYTES {
        return Err(ToolError::new(ErrorCode::OverLimit, "Pi's model list is too large."));
    }
    let text = String::from_utf8_lossy(bytes);
    let Some(response) = text.lines().find_map(|line| {
        let v: Value = serde_json::from_str(line.trim()).ok()?;
        (v.get("type").and_then(Value::as_str) == Some("response") && v.get("command").and_then(Value::as_str) == Some("get_available_models")).then_some(v)
    }) else {
        return Ok(None);
    };
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(ToolError::new(ErrorCode::GarbledOutput, "Pi couldn't list its models.")
            .with_remediation("Check Pi on this computer (`pi --list-models`), then reload the models."));
    }
    let Some(items) = response.pointer("/data/models").and_then(Value::as_array) else {
        return Err(ToolError::new(ErrorCode::GarbledOutput, "Pi returned an unreadable model list."));
    };
    let mut out = std::collections::BTreeMap::new();
    for item in items {
        if item.get("type").and_then(Value::as_str).is_some_and(|kind| kind != "chat") {
            continue;
        }
        let Some(provider) = item.get("provider").and_then(Value::as_str).filter(|s| !s.is_empty()) else { continue };
        let Some(model) = item.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()) else { continue };
        let label = item.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(model);
        let reasoning = item.get("reasoning").and_then(Value::as_bool).unwrap_or(false);
        let map = item.get("thinkingLevelMap").and_then(Value::as_object);
        let id = format!("{provider}/{model}");
        let thinking = supported_thinking(reasoning, map);
        out.insert(id.clone(), models::AgentModel { id, name: format!("{label} ({provider})"), thinking });
    }
    Ok(Some(out.into_values().collect()))
}

/// The table `--list-models` prints: a header of six columns, then one row per model. Rows that
/// aren't a model (sign-in guidance, a broken line) are skipped, so a pi with no models reads as
/// an empty list rather than an error.
pub(crate) fn parse_list_models(bytes: &[u8]) -> Result<Vec<models::AgentModel>, ToolError> {
    if bytes.len() >= MODELS_MAX_BYTES {
        return Err(ToolError::new(ErrorCode::OverLimit, "Pi's model list is too large."));
    }
    let text = String::from_utf8_lossy(bytes);
    let mut out = std::collections::BTreeMap::new();
    let mut header = false;
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if !header {
            let is_header = cols.len() == 6
                && cols[0].eq_ignore_ascii_case("provider")
                && cols[1].eq_ignore_ascii_case("model")
                && cols[2].eq_ignore_ascii_case("context")
                && cols[3].eq_ignore_ascii_case("max-out")
                && cols[4].eq_ignore_ascii_case("thinking")
                && cols[5].eq_ignore_ascii_case("images");
            if is_header {
                header = true;
            }
            continue;
        }
        let [provider, model, _context, _max_out, thinking, images] = cols.as_slice() else { continue };
        if provider.is_empty() || model.is_empty() || !is_yes_no(thinking) || !is_yes_no(images) {
            continue;
        }
        let id = format!("{provider}/{model}");
        // The table says only yes or no: without `thinkingLevelMap`, `xhigh` and `max` can't be
        // confirmed for a model, so the base depths up to `high` are what it is offered.
        let thinking = if thinking.eq_ignore_ascii_case("yes") {
            vec![crate::Effort::None, crate::Effort::Minimal, crate::Effort::Low, crate::Effort::Medium, crate::Effort::High]
        } else {
            Vec::new()
        };
        out.insert(id.clone(), models::AgentModel { id, name: format!("{model} ({provider})"), thinking });
    }
    Ok(out.into_values().collect())
}

/// The thinking depths a model exposes, from pi's `reasoning` flag and `thinkingLevelMap`: a null
/// entry is a depth the model doesn't take, and `xhigh`/`max` are only there when it does.
fn supported_thinking(reasoning: bool, map: Option<&serde_json::Map<String, Value>>) -> Vec<crate::Effort> {
    if !reasoning {
        return Vec::new();
    }
    crate::Effort::CLI_ALL
        .into_iter()
        .filter(|&effort| match map.and_then(|m| m.get(thinking_level(effort))) {
            Some(Value::Null) => false,
            None if matches!(effort, crate::Effort::Xhigh | crate::Effort::Max) => false,
            _ => true,
        })
        .collect()
}

fn is_yes_no(text: &str) -> bool {
    text.eq_ignore_ascii_case("yes") || text.eq_ignore_ascii_case("no")
}

/// The text of a tool result: pi's blocks, joined.
fn result_text(result: Option<&Value>) -> String {
    let Some(result) = result else { return String::new() };
    match result.get("content").and_then(Value::as_array) {
        Some(blocks) => blocks.iter().filter_map(|b| b.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
        None => result.as_str().unwrap_or_default().to_string(),
    }
}

/// Our server as a builder gets it: listing every tool, because this builder has no editor or
/// shell of its own and the build tools (`build_file_write`, `build_command_run`) are the only way
/// it can change the project. Clean room still refuses what it may not read: listing a tool never
/// makes it available.
fn builder_server(mcp: &McpCommand) -> McpCommand {
    let mut server = mcp.clone();
    if !server.args.iter().any(|a| a == "--all-tools") {
        server.args.push("--all-tools".into());
    }
    server
}

/// pi's own folder: `PI_CODING_AGENT_DIR`, else `~/.pi/agent`. A folder chosen for the backend in
/// Settings is not applied: pi reads that variable itself, and a second place to look would put
/// sessions where pi does not.
fn agent_dir() -> Option<std::path::PathBuf> {
    match std::env::var_os("PI_CODING_AGENT_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => Some(std::path::PathBuf::from(dir)),
        None => dirs::home_dir().map(|home| home.join(".pi").join("agent")),
    }
}

/// An id pi would use: a UUID, or whatever `--session-id` was given. Anything else (a hand-edited
/// project file) names no session.
fn session_id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// The folder pi gives a working folder: the leading separator removed and `/`, `\` and `:` read
/// as `-`, between two `--` (`/w/P` is `--w-P--`, as pi's own session files are named).
fn session_folder(cwd: &std::path::Path) -> String {
    let text = cwd.to_string_lossy();
    let text = text.strip_prefix('/').unwrap_or(&text);
    format!("--{}--", text.replace(['/', '\\', ':'], "-"))
}

/// Folders under pi's `sessions/` looked through for one session, so a big history stays bounded.
const SESSION_FOLDERS: usize = 10_000;

/// The file pi keeps session `id` in under `dir`, if this computer has it: the newest one, since a
/// run that didn't find a put-back copy starts a second file under the same id.
fn session_file_in(dir: &std::path::Path, id: &str) -> Option<std::path::PathBuf> {
    if !session_id_ok(id) {
        return None;
    }
    let suffix = format!("_{id}.jsonl");
    let mut newest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for folder in std::fs::read_dir(dir.join("sessions")).ok()?.flatten().take(SESSION_FOLDERS) {
        let Ok(files) = std::fs::read_dir(folder.path()) else { continue };
        for path in files.flatten().map(|f| f.path()) {
            let named = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(&suffix));
            if !named || !path.is_file() {
                continue;
            }
            let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) else { continue };
            if newest.as_ref().is_none_or(|(at, _)| modified > *at) {
                newest = Some((modified, path));
            }
        }
    }
    newest.map(|(_, path)| path)
}

/// Where a kept copy put back for `cwd` belongs: pi's folder for that working folder, named after
/// the session's own timestamp so pi reads it as one of its own.
fn kept_session_path(dir: &std::path::Path, id: &str, stamp: &str, cwd: &std::path::Path) -> std::path::PathBuf {
    dir.join("sessions").join(session_folder(cwd)).join(format!("{stamp}_{id}.jsonl"))
}

/// A stamp for a kept session that names no timestamp of its own.
const KEPT_STAMP: &str = "1970-01-01T00-00-00-000Z";

/// Put a kept copy of session `id` back where pi looks for it in `cwd`. `Ok(false)`: it's here
/// already. A copy of another session is refused rather than restored.
fn restore_session_in(dir: &std::path::Path, id: &str, saved: &[u8], cwd: &std::path::Path) -> Result<bool, ToolError> {
    if !session_id_ok(id) {
        return Err(ToolError::invalid("The project names its builder's session with an id pi doesn't use."));
    }
    if session_file_in(dir, id).is_some() {
        return Ok(false);
    }
    let header = saved.split(|b| *b == b'\n').next().and_then(|line| serde_json::from_slice::<Value>(line).ok());
    let Some(header) = header.filter(|h| h.get("id").and_then(Value::as_str) == Some(id)) else {
        return Err(ToolError::invalid("The conversation kept in the project is of another session."));
    };
    let stamp = header
        .get("timestamp")
        .and_then(Value::as_str)
        .map(|t| t.replace([':', '.'], "-"))
        .filter(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == 'T'))
        .unwrap_or_else(|| KEPT_STAMP.to_string());
    // `/tmp` is a link to `/private/tmp` on macOS, and `/var` to `/private/var`: pi's own working
    // folder is the real one, so the copy has to land there or the run won't find it.
    let cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let path = kept_session_path(dir, id, &stamp, &cwd);
    craft_anything_platform::fs::write_atomic_private(&path, saved)?;
    Ok(true)
}

/// What a new session needs to know of an earlier one: the newest exchanges in the kept copy of a
/// session, each clipped, keeping at most `max` characters. pi's entries are a tree (`id`/
/// `parentId`); the newest are the tail of the file, which is the branch that was last written.
fn pi_handoff(saved: &str, max: usize) -> Option<String> {
    let clip = |text: &str| {
        let text = text.trim();
        if text.chars().count() > HANDOFF_ENTRY { format!("{}…", text.chars().take(HANDOFF_ENTRY).collect::<String>()) } else { text.to_string() }
    };
    let texts = |content: Option<&Value>| -> String {
        content
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut entries = Vec::new();
    for v in saved.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
        if v.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(message) = v.get("message") else { continue };
        let content = message.get("content");
        match message.get("role").and_then(Value::as_str) {
            Some("user") => {
                let text = content.and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| texts(content));
                if !text.trim().is_empty() {
                    entries.push(format!("Craft Anything asked you: {}", clip(&text)));
                }
            }
            Some("assistant") => {
                let text = texts(content);
                if !text.trim().is_empty() {
                    entries.push(format!("You answered: {}", clip(&text)));
                }
            }
            _ => {}
        }
    }
    let mut kept = Vec::new();
    let mut used = 0usize;
    for entry in entries.iter().rev() {
        let n = entry.chars().count().saturating_add(2);
        if used.saturating_add(n) > max {
            break;
        }
        used = used.saturating_add(n);
        kept.push(entry.as_str());
    }
    kept.reverse();
    (!kept.is_empty()).then(|| kept.join("\n\n"))
}

/// The extension file this run loads, or a path that isn't there when the folder can't take one,
/// so the run fails at startup instead of building without the tool server it was promised
/// ([`check_tool_server`](AgentAdapter::check_tool_server) reports the everyday causes first).
fn extension_file(dir: Option<&std::path::Path>, mcp: &McpCommand) -> String {
    let Some(dir) = dir else { return missing_extension() };
    let path = dir.join(EXTENSION_FILE);
    match craft_anything_platform::write_atomic_private(&path, extension_js(mcp).as_bytes()) {
        Ok(()) => path.display().to_string(),
        Err(e) => {
            log::warn!("pi's tool-server extension couldn't be written: {}", e.message);
            missing_extension()
        }
    }
}

/// A path no run wrote anything to.
fn missing_extension() -> String {
    std::env::temp_dir().join(format!("craft-anything-pi-{}-missing.js", std::process::id())).display().to_string()
}

/// A folder of this process's own for one run, in the system temporary folder: two builds at once
/// never share one, and because it's owner-only nothing another user could prepare in a shared
/// temporary folder is ever loaded through.
fn extension_dir() -> Result<std::path::PathBuf, ToolError> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("craft-anything-pi-{}-{n}", std::process::id()));
    // `create_dir` refuses a path that is already there, so a folder prepared beforehand is never
    // used (`create_dir_all` would accept one).
    private_dir().create(&dir).map_err(|e| {
        ToolError::io(dir.display(), e).with_remediation(
            "Free space on the drive that holds the temporary folder, or point TMP at one that can be written, then start the build again.".to_string(),
        )
    })?;
    Ok(dir)
}

/// A builder for one run's own folder, owner-only from the moment it exists.
#[cfg(unix)]
fn private_dir() -> std::fs::DirBuilder {
    use std::os::unix::fs::DirBuilderExt as _;
    let mut builder = std::fs::DirBuilder::new();
    builder.mode(0o700);
    builder
}

#[cfg(not(unix))]
fn private_dir() -> std::fs::DirBuilder {
    std::fs::DirBuilder::new()
}

/// The arguments that ask Craft Anything about one action: `core-action-approve` with the run's
/// project and mode. `--inbox`, `--all-tools` and `--target` aren't options of that tool, and the
/// inbox is read from the project.
fn approve_args(mcp: &McpCommand) -> Vec<String> {
    let mut args = vec!["core-action-approve".to_string()];
    let mut rest = mcp.args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--project" => {
                args.push(arg.clone());
                if let Some(value) = rest.next() {
                    args.push(value.clone());
                }
            }
            "--deep" | "--clean-room" => args.push(arg.clone()),
            _ => {}
        }
    }
    args
}

/// Our tool server as the extension this run loads: the registration, declared directly so its
/// tools are in the model's list, and the approval bridge for pi's own changing tools. Under JSON
/// quoting rather than shell quoting.
fn extension_js(mcp: &McpCommand) -> String {
    let server = serde_json::Value::String(SERVER.to_string());
    let config = serde_json::json!({
        "command": mcp.command,
        "args": mcp.args,
        "exposure": "direct",
        "description": "Craft Anything's tools for this run",
    });
    let cli = serde_json::Value::String(mcp.command.clone());
    let approve = serde_json::Value::Array(approve_args(mcp).iter().map(|a| serde_json::Value::String(a.clone())).collect());
    format!(
        "// Craft Anything's tool server and approval bridge for this run. Generated; safe to delete.\n\
         import {{ spawn }} from \"node:child_process\";\n\
         const CLI = {cli};\n\
         const APPROVE = {approve};\n\
         // A tool's input can be a whole written file, so it goes in on stdin: a command line stops\n\
         // at about 32,000 characters on Windows, which refused every large write. The wait is\n\
         // longer than the approval's own 30 minutes, so only a stuck CLI reaches it; running out\n\
         // blocks the action.\n\
         const WAIT_MS = 35 * 60 * 1000;\n\
         function ask(payload) {{\n  \
           return new Promise((resolve, reject) => {{\n    \
             const child = spawn(CLI, [...APPROVE, \"--json\", \"-\"], {{ stdio: [\"pipe\", \"pipe\", \"pipe\"], windowsHide: true }});\n    \
             let out = \"\";\n    \
             let err = \"\";\n    \
             const timer = setTimeout(() => {{ child.kill(); reject(new Error(\"Craft Anything didn't answer within 35 minutes\")); }}, WAIT_MS);\n    \
             child.stdout.on(\"data\", (d) => {{ out += d; }});\n    \
             child.stderr.on(\"data\", (d) => {{ err += d; }});\n    \
             child.on(\"error\", (e) => {{ clearTimeout(timer); reject(e); }});\n    \
             child.on(\"close\", (code) => {{\n      \
               clearTimeout(timer);\n      \
               if (code === 0) resolve(out);\n      \
               else reject(new Error(err.trim() || (\"exit code \" + code)));\n    \
             }});\n    \
             child.stdin.on(\"error\", () => {{}});\n    \
             child.stdin.end(payload);\n  \
           }});\n\
         }}\n\
         // pi's own changing tools, in the shape Craft Anything's policy reads (Claude Code's).\n\
         const SHAPE = {{\n  \
           bash: (i) => [\"Bash\", {{ command: i.command }}],\n  \
           powershell: (i) => [\"PowerShell\", {{ command: i.command }}],\n  \
           write: (i) => [\"Write\", {{ file_path: i.path, content: i.content }}],\n  \
           edit: (i) => [\"Edit\", {{ file_path: i.path, old_string: i.oldText, new_string: i.newText }}],\n\
         }};\n\
         export default function (pi) {{\n  \
           pi.registerMcpServer({server}, {config});\n  \
           // Nothing changes without asking Craft Anything first. Readings and our own tools pass.\n  \
           pi.on(\"tool_call\", async (event) => {{\n    \
             const name = String(event.toolName ?? \"\");\n    \
             if (name.startsWith(\"mcp__\")) return;\n    \
             const shape = SHAPE[name];\n    \
             if (!shape) return;\n    \
             const [tool, input] = shape(event.input ?? {{}});\n    \
             let verdict;\n    \
             try {{\n      \
               const stdout = await ask(JSON.stringify({{ tool_name: tool, input }}));\n      \
               verdict = JSON.parse(stdout)?.result;\n    \
             }} catch (e) {{\n      \
               return {{ block: true, reason: \"Craft Anything couldn't check this action, so it wasn't done: \" + (e?.message ?? e) }};\n    \
             }}\n    \
             if (verdict?.behavior === \"allow\") return;\n    \
             return {{ block: true, reason: String(verdict?.message ?? \"Craft Anything refused this action.\") }};\n  \
           }});\n\
         }}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Effort;

    /// A tool server command for a run's project.
    fn mcp() -> McpCommand {
        McpCommand {
            command: "/Applications/Craft Anything.app/Contents/MacOS/craft-anything-cli".into(),
            args: vec!["mcp".into(), "--project".into(), "/w/P/.cany".into(), "--deep".into()],
            notes: None,
        }
    }

    /// The arguments for one message in the runner-provided folder.
    fn args_in(
        options: &BackendOptions,
        builder: bool,
        question: bool,
        reads_checked: bool,
        resume: Option<&str>,
        run_dir: Option<&Path>,
    ) -> (Vec<String>, McpCommand) {
        let mcp = mcp();
        let cx = ArgsCx { options, prompt: "make a game", preamble: "PRE ", mcp: &mcp, resume, builder, reads_checked, question, message_file: None, run_dir };
        (PiCli.args(&cx), mcp.clone())
    }

    /// A real run-directory guard, held by the test until the run would be over.
    fn args(options: &BackendOptions, builder: bool, question: bool, reads_checked: bool, resume: Option<&str>) -> (Vec<String>, McpCommand, RunDir) {
        let run = RunDir::make().expect("a run folder");
        let (args, mcp) = args_in(options, builder, question, reads_checked, resume, Some(&run.0));
        (args, mcp, run)
    }

    /// The value after `flag`.
    fn after<'a>(args: &'a [String], flag: &str) -> &'a str {
        let i = args.iter().position(|a| a == flag).expect("the flag is there");
        args.get(i + 1).map(String::as_str).expect("a value after it")
    }

    /// The extension file this run loads. `-e` also names pi's own built-in MCP support, which
    /// `--no-extensions` turns off and this run has to load back.
    fn extension_arg(args: &[String]) -> std::path::PathBuf {
        args.windows(2)
            .filter(|pair| pair[0] == "-e")
            .map(|pair| std::path::PathBuf::from(&pair[1]))
            .find(|path| path.file_name().and_then(|n| n.to_str()) == Some(EXTENSION_FILE))
            .expect("the run loads its own extension")
    }

    /// The message never enters argv, and the stream is the JSON one.
    #[test]
    fn the_message_stays_out_of_argv_and_the_stream_is_json() {
        let (a, _, _run) = args(&BackendOptions::default(), true, false, false, None);
        assert_eq!(a[0..4], ["-p", "--mode", "json", "--append-system-prompt"], "{a:?}");
        assert_eq!(after(&a, "--append-system-prompt"), "PRE ");
        assert!(PiCli.prompt_on_stdin(), "the message must go in on stdin");
        assert!(!a.iter().any(|arg| arg.contains("make a game")), "{a:?}");
        assert!(PiCli.streams_json());
    }

    /// A resumed session continues the conversation; a question starts a fresh one, so it leaves
    /// the builder's conversation as it was.
    #[test]
    fn a_resumed_build_continues_the_session_and_a_question_does_not() {
        let (a, _, _run) = args(&BackendOptions::default(), true, false, false, Some("s-1"));
        assert_eq!(after(&a, "--session-id"), "s-1");
        let (q, _, _run) = args(&BackendOptions::default(), false, true, false, Some("s-1"));
        assert!(!q.iter().any(|x| x == "--session-id"), "a question starts fresh: {q:?}");
    }

    /// The model and thinking effort a person chose reach the CLI.
    #[test]
    fn a_chosen_model_and_effort_reach_the_cli() {
        let options = BackendOptions { cli_model: Some("magpie/foxcode/claude-sonnet-5-5".into()), cli_thinking: Some(Effort::High), ..Default::default() };
        let (a, _, _run) = args(&options, true, false, false, None);
        assert_eq!(after(&a, "--model"), "magpie/foxcode/claude-sonnet-5-5");
        assert_eq!(after(&a, "--thinking"), "high");
    }

    /// pi calls its lowest thinking level `off`: `--thinking none` is refused and falls back to
    /// pi's own default, so the person's choice would be quietly ignored.
    #[test]
    fn the_lowest_thinking_level_is_called_off() {
        let options = BackendOptions { cli_thinking: Some(Effort::None), ..Default::default() };
        let (a, _, _run) = args(&options, true, false, false, None);
        assert_eq!(after(&a, "--thinking"), "off");
        assert_eq!(thinking_level(Effort::Minimal), "minimal");
        assert_eq!(thinking_level(Effort::Xhigh), "xhigh");
        let folder = extension_arg(&a).parent().expect("a folder").to_path_buf();
        let _ = std::fs::remove_dir_all(folder);
    }

    /// Nothing of the person's or the project's loads beside this run: no discovered extensions,
    /// no context files, no trust-gated project files — and pi's own MCP support is loaded back,
    /// since `--no-extensions` turns it off too and `registerMcpServer` would fail without it.
    #[test]
    fn other_extensions_and_project_files_stay_out_of_a_run() {
        let (a, _, _run) = args(&BackendOptions::default(), true, false, false, None);
        for flag in ["--no-extensions", "--no-context-files", "--no-approve"] {
            assert!(a.iter().any(|x| x == flag), "{flag} is passed: {a:?}");
        }
        let loaded: Vec<&str> = a.windows(2).filter(|p| p[0] == "-e").map(|p| p[1].as_str()).collect();
        assert_eq!(loaded.first(), Some(&"builtin:mcp"), "the built-in MCP support comes back: {a:?}");
        assert!(loaded.iter().any(|p| p.ends_with(EXTENSION_FILE)), "and then ours: {a:?}");
        let folder = extension_arg(&a).parent().expect("a folder").to_path_buf();
        let _ = std::fs::remove_dir_all(folder);
    }

    /// A question runs fresh and ephemerally, with no server of anyone's, so it leaves pi's
    /// session list and the person's own servers alone.
    #[test]
    fn a_question_is_ephemeral_and_loads_no_server() {
        let (q, _, _run) = args(&BackendOptions::default(), false, true, false, None);
        for flag in ["--no-session", "--no-mcp", "--no-extensions", "--no-context-files", "--no-approve"] {
            assert!(q.iter().any(|x| x == flag), "{flag} is passed: {q:?}");
        }
        assert!(!q.iter().any(|x| x == "-e"), "no extension is loaded: {q:?}");
        assert!(!PiCli.questions_fork_session(), "pi's question starts fresh, so it isn't the builder's");
        assert!(PiCli.resumes(), "a build still continues its session");
    }

    /// A saved argument can't reopen what a run holds shut: no saved tools, extension, mode or
    /// session, and nothing after a `--` (pi would read that as the message).
    #[test]
    fn saved_arguments_cannot_override_the_run() {
        let options = BackendOptions {
            args: vec![
                "--tools".into(),
                "read,bash".into(),
                "--extension=x.js".into(),
                "--session".into(),
                "old".into(),
                "--model".into(),
                "kept".into(),
                "--offline".into(),
                "--".into(),
                "not-a-message".into(),
            ],
            ..Default::default()
        };
        let (q, _, _run) = args(&options, false, true, true, None);
        for gone in ["--tools", "--extension", "--session", "read,bash", "x.js", "old", "--", "not-a-message"] {
            assert!(!q.iter().any(|x| x == gone), "{gone} must not reach the run: {q:?}");
        }
        assert!(q.iter().any(|x| x == "--offline"), "other saved arguments stay: {q:?}");
        assert!(q.iter().any(|x| x == "--model"), "a chosen model stays: {q:?}");
        // The run's own limits are still the only ones.
        assert!(q.iter().any(|x| x == "--no-tools"), "a Clean room question gets none: {q:?}");
        assert!(q.iter().any(|x| x == "--no-extensions"), "{q:?}");
    }

    /// Dropping the runner's guard removes the extension folder, however the turn ended.
    #[test]
    fn the_runs_extension_folder_is_removed_after_the_turn() {
        let (a, _, run) = args(&BackendOptions::default(), true, false, false, None);
        let folder = extension_arg(&a).parent().expect("a folder").to_path_buf();
        assert_eq!(folder, run.0, "the extension lives in the runner's folder");
        assert!(folder.exists(), "the run's folder is there while it runs");
        drop(run);
        assert!(!folder.exists(), "and gone when the turn is over: {folder:?}");
    }

    /// A remote build would be handed a path on this computer, so it is refused in words a person
    /// can act on instead of starting a pi that can't see its tools.
    #[test]
    fn a_build_over_ssh_is_refused() {
        let stop = crate::model::Stop::for_tool(Default::default());
        let err = PiCli.check_remote_options(&BackendOptions::default(), "example.invalid", &stop).expect_err("refused");
        assert_eq!(err.code, ErrorCode::NotSupported);
        assert!(err.message.contains("SSH"), "{}", err.message);
        assert!(err.remediation.as_deref().unwrap_or_default().contains("this computer"), "{err:?}");
    }

    /// The run's own tool server, as an extension in a folder only its owner can read.
    #[test]
    fn the_tool_server_arrives_as_an_extension_in_the_runs_own_folder() {
        let (a, mcp, _run) = args(&BackendOptions::default(), true, false, false, None);
        let path = extension_arg(&a);
        assert!(path.exists(), "{path:?}");
        let written = std::fs::read_to_string(&path).expect("read the extension");
        assert!(written.contains("registerMcpServer"), "{written}");
        assert!(written.contains(SERVER), "the run's own name: {written}");
        assert!(written.contains("\"exposure\":\"direct\""), "declared to the model: {written}");
        assert!(written.contains(&mcp.command), "{written}");
        assert!(written.contains("--project"), "the run's own arguments: {written}");
        // A folder of the run's own, so nothing another user prepared is loaded through it.
        let folder = path.parent().expect("a folder of its own").to_path_buf();
        let own = format!("craft-anything-run-{}-", std::process::id());
        assert!(folder.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&own)), "{folder:?}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let bits = |p: &std::path::Path| std::fs::metadata(p).expect("metadata").permissions().mode() & 0o777;
            assert_eq!(bits(&folder), 0o700, "folder {:o}", bits(&folder));
            assert_eq!(bits(&path), 0o600, "extension {:o}", bits(&path));
        }
        // Two runs never share one.
        let (b, _, _run) = args(&BackendOptions::default(), true, false, false, None);
        let other = extension_arg(&b).parent().expect("a folder").to_path_buf();
        assert_ne!(folder, other);
        for dir in [folder, other] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// A Deep builder edits and runs with pi's own tools (the bridge gates them), and `codemode` is
    /// out because it could call another server's tools by name.
    #[test]
    fn a_deep_builder_may_edit_and_run_but_not_reach_other_servers() {
        let (a, _, _run) = args(&BackendOptions::default(), true, false, false, None);
        let tools: Vec<&str> = after(&a, "--tools").split(',').collect();
        for own in ["codemode", "tool_search"] {
            assert!(!tools.contains(&own), "{own} must not be there: {tools:?}");
        }
        for kept in READ_TOOLS.iter().chain(CHANGING_TOOLS) {
            assert!(tools.contains(kept), "{kept} belongs to a Deep builder: {tools:?}");
        }
        assert!(tools.iter().any(|t| t.starts_with(TOOL_PREFIX)), "our tools: {tools:?}");
    }

    /// Every changing tool pi offers goes through Craft Anything's own approval first, from the
    /// same generated file, so nothing outside the repository is installed into pi.
    #[test]
    fn the_extension_holds_the_approval_bridge() {
        let (a, mcp, _run) = args(&BackendOptions::default(), true, false, false, None);
        let path = extension_arg(&a);
        let written = std::fs::read_to_string(&path).expect("read the extension");
        for shape in ["bash:", "powershell:", "write:", "edit:"] {
            assert!(written.contains(shape), "{shape} is gated: {written}");
        }
        assert!(written.contains("pi.on(\"tool_call\""), "{written}");
        assert!(written.contains("\"core-action-approve\""), "{written}");
        assert!(written.contains(&format!("\"{}\"", mcp.args[2])), "the run's project is passed: {written}");
        // A written file's content can be far past Windows' command-line limit, so the action's
        // input goes in on stdin; the call has a deadline past the approval's own wait, and
        // running out blocks the action.
        assert!(written.contains("\"--json\", \"-\""), "the input goes in on stdin: {written}");
        assert!(!written.contains("--tool-name") && !written.contains("--input"), "nothing of the action is in argv: {written}");
        assert!(written.contains("const WAIT_MS = 35 * 60 * 1000;"), "a deadline past the 30-minute wait: {written}");
        assert!(written.contains("child.kill()") && written.contains("block: true"), "running out blocks it: {written}");
        assert!(!written.contains("execFile"), "spawn replaces the size-capped call: {written}");
        assert!(written.contains("child.stdin.end(payload)"), "the payload is written: {written}");
        let bridge = written.lines().find(|l| l.trim_start().starts_with("const APPROVE")).expect("the approval command");
        assert!(bridge.contains("\"core-action-approve\""), "{bridge}");
        assert!(!bridge.contains("--all-tools") && !bridge.contains("--inbox"), "only the approval tool's own options: {bridge}");
        assert!(bridge.contains("--deep") || bridge.contains("--clean-room"), "the run's mode: {bridge}");
        let folder = path.parent().expect("a folder").to_path_buf();
        let _ = std::fs::remove_dir_all(folder);
    }

    /// A builder has no editor of its own, so its tool server lists the build tools.
    #[test]
    fn a_builders_server_lists_every_tool() {
        let (a, _, _run) = args(&BackendOptions::default(), true, false, false, None);
        let path = extension_arg(&a);
        let written = std::fs::read_to_string(&path).expect("read the extension");
        assert!(written.contains("--all-tools"), "{written}");
        let folder = path.parent().expect("a folder").to_path_buf();
        let _ = std::fs::remove_dir_all(folder);
        // A question has no server, and an already-complete list isn't doubled.
        let complete = McpCommand { command: "cli".into(), args: vec!["mcp".into(), "--all-tools".into()], notes: None };
        let server = builder_server(&complete);
        assert_eq!(server.args.iter().filter(|a| *a == "--all-tools").count(), 1, "{:?}", server.args);
    }

    /// In Clean room nothing of the original is read through the agent's own file tools.
    #[test]
    fn a_clean_room_builder_gets_no_file_tools() {
        let (a, _, _run) = args(&BackendOptions::default(), true, false, true, None);
        assert_eq!(after(&a, "--tools"), format!("{TOOL_PREFIX}*"));
        let (q, _, _run) = args(&BackendOptions::default(), false, true, true, None);
        assert!(q.iter().any(|x| x == "--no-tools"), "a Clean room question gets no tools: {q:?}");
        assert!(!q.iter().any(|x| x == "-e"), "and no tool server: {q:?}");
    }

    /// A Deep-mode question reads with the agent's own tools and never has our server.
    #[test]
    fn a_question_reads_with_the_agents_own_tools() {
        let (q, _, _run) = args(&BackendOptions::default(), false, true, false, None);
        assert_eq!(after(&q, "--tools"), READ_TOOLS.join(","));
        assert!(!q.iter().any(|x| x == "-e"), "{q:?}");
    }

    /// The agent is registered under its own id, not handed to the plain adapter.
    #[test]
    fn registered_not_the_plain_adapter() {
        assert!(AgentAdapters::built_in().for_agent("pi").ids().contains(&"pi"));
        assert_eq!(PiCli.required_flags(), (&["--help"][..], &["--mode", "--session-id", "--no-extensions", "--no-context-files", "--no-approve"][..]));
        assert!(!PiCli.plain_fallback(), "the plain form would boot the terminal UI");
        assert!(PiCli.run_files(), "the runner provides and removes the extension folder");
    }

    /// A folder that can't take the extension leaves a path that isn't there, rather than a build
    /// that quietly has no tools.
    #[test]
    fn a_folder_that_cannot_take_the_extension_leaves_a_path_that_is_not_there() {
        let missing = missing_extension();
        assert!(!std::path::Path::new(&missing).exists(), "{missing}");
    }

    /// If the runner cannot provide a folder, no extension is written elsewhere and left behind.
    #[test]
    fn without_a_run_folder_no_extension_is_written() {
        let (a, _) = args_in(&BackendOptions::default(), true, false, false, None, None);
        let path = a.windows(2).filter(|pair| pair[0] == "-e").map(|pair| std::path::PathBuf::from(&pair[1])).next_back().expect("the missing extension path");
        assert_eq!(path, std::path::PathBuf::from(missing_extension()));
        assert!(!path.exists(), "nothing was written outside the run: {path:?}");
    }

    /// pi's JSON lines: the session id, the text, both ends of a tool call, and the usage.
    #[test]
    fn text_tool_calls_and_usage_come_from_the_json_lines() {
        let mut p = StreamParser::new(Arc::new(PiCli));
        let header = serde_json::json!({"type":"session","version":3,"id":"01a1bf2c-e839-7317-aef8-423ca8393cdc","cwd":"/w/P"});
        assert!(PiCli.parse(&mut p, &header).is_empty());
        assert_eq!(p.session_id.as_deref(), Some("01a1bf2c-e839-7317-aef8-423ca8393cdc"));
        let text = serde_json::json!({"type":"message_update","usage":{"input":10,"output":0},"assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Hello"}});
        assert!(matches!(PiCli.parse(&mut p, &text).as_slice(), [AgentEvent::TextDelta { text }] if text == "Hello"));
        let thinking = serde_json::json!({"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","contentIndex":0,"delta":"hmm"}});
        assert!(PiCli.parse(&mut p, &thinking).is_empty(), "thinking is not the answer");
        let start = serde_json::json!({"type":"tool_execution_start","toolCallId":"call_1","toolName":"mcp__craft_anything_run__build_file_write","args":{"path":"app/src/main.rs"}});
        assert!(
            matches!(PiCli.parse(&mut p, &start).as_slice(), [AgentEvent::ToolStarted { tool, .. }] if tool == "build_file_write"),
            "our server's prefix is dropped from the name"
        );
        let end = serde_json::json!({"type":"tool_execution_end","toolCallId":"call_1","toolName":"mcp__craft_anything_run__build_file_write","result":{"content":[{"type":"text","text":"Wrote app/src/main.rs"}]},"isError":false});
        assert!(
            matches!(PiCli.parse(&mut p, &end).as_slice(), [AgentEvent::ToolFinished { ok: true, summary, .. }] if summary.contains("Wrote app/src/main.rs")),
            "the result's text is the summary"
        );
        let done = serde_json::json!({"type":"message_end","message":{"role":"assistant","content":[],"usage":{"input":5952,"output":2,"cacheRead":1280,"cacheWrite":0,"reasoning":0,"totalTokens":7234,"cost":{"total":0.25}}}});
        assert!(PiCli.parse(&mut p, &done).is_empty());
        assert_eq!((p.input_tokens, p.output_tokens), (7232, 2), "cached reads count as input");
        assert_eq!(p.cost_usd, Some(0.25));
        assert!(p.saw_result);
        let again = serde_json::json!({"type":"message_end","message":{"role":"user","content":"hi","usage":{"input":99,"output":0}}});
        assert!(PiCli.parse(&mut p, &again).is_empty(), "the person's own message has no usage to count");
        assert_eq!(p.input_tokens, 7232);
    }

    /// A retry that gave up becomes an error in words, with the fix, and a typed failure for the
    /// stop rules.
    #[test]
    fn a_final_retry_failure_becomes_an_error_in_words() {
        let mut p = StreamParser::new(Arc::new(PiCli));
        let line = serde_json::json!({"type":"auto_retry_end","success":false,"attempt":3,"finalError":"401 Unauthorized"});
        let events = PiCli.parse(&mut p, &line);
        assert!(matches!(events.as_slice(), [AgentEvent::Error { message, remediation: Some(_), .. }] if message.contains("401")), "{events:?}");
        assert_eq!(p.failure.map(|f| f.kind), Some(FailureKind::Authentication));
        for (why, kind) in [
            ("429 Too Many Requests", FailureKind::RateLimited),
            ("429: {\"type\":\"insufficient_quota\",\"code\":14018}", FailureKind::Billing),
            ("HTTP 14018 is not a status", FailureKind::Unknown),
            ("Claude usage limit reached. It resets at 3pm", FailureKind::UsageLimit),
            ("insufficient credit", FailureKind::Billing),
            ("upstream timed out", FailureKind::Transient),
            ("the model said no", FailureKind::Unknown),
        ] {
            assert_eq!(classify(why), kind, "{why}");
        }
    }

    /// pi answers the model query over RPC with each model's thinking levels; a null level is one
    /// it doesn't take, and `xhigh`/`max` are only there when it does.
    #[test]
    fn the_model_catalogue_reads_pis_rpc_answer() {
        let answer = serde_json::json!({"type":"response","command":"get_available_models","success":true,"data":{"models":[
            {"id":"deepseek-flash","name":"DeepSeek V4.1 Flash","provider":"deepseek","reasoning":true,
             "thinkingLevelMap":{"minimal":null,"low":"low","medium":null,"high":"high","max":"max"}},
            {"id":"plain","name":"Plain","provider":"p","reasoning":false},
            {"id":"picture","provider":"p","type":"image"}
        ]}});
        let list = parse_rpc_models(format!("{answer}\n").as_bytes()).expect("read").expect("an answer");
        let ids: Vec<&str> = list.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["deepseek/deepseek-flash", "p/plain"], "{list:?}");
        assert_eq!(list[0].name, "DeepSeek V4.1 Flash (deepseek)");
        assert_eq!(list[0].thinking, [Effort::None, Effort::Low, Effort::High, Effort::Max], "only the levels it maps");
        assert!(list[1].thinking.is_empty(), "a model without reasoning has none");
        // No answer line: the caller asks the table instead.
        assert!(parse_rpc_models(b"Warning: no models are signed in\n").expect("read").is_none());
    }

    /// The table a pi without that answer prints: six columns, one row per model, and anything
    /// else (sign-in guidance, a broken line) skipped.
    #[test]
    fn the_model_catalogue_reads_pis_table() {
        let table = "No models are signed in yet. Run `pi` to sign in.\n\
provider      model            context  max-out  thinking  images\n\
deepseek      deepseek-flash   1M       384K     yes       no\n\
p             plain            128K     8K       no        yes\n\
broken        row\n\
magpie        agy-cli/x        250K     64K      yes       yes\n";
        let list = parse_list_models(table.as_bytes()).expect("read");
        let ids: Vec<&str> = list.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["deepseek/deepseek-flash", "magpie/agy-cli/x", "p/plain"], "{list:?}");
        assert_eq!(list[0].name, "deepseek-flash (deepseek)");
        assert_eq!(list[0].thinking, [Effort::None, Effort::Minimal, Effort::Low, Effort::Medium, Effort::High]);
        assert!(list[2].thinking.is_empty(), "no reasoning, no depths");
        assert!(parse_list_models(b"nothing here\n").expect("read").is_empty());
    }

    /// The catalogue a run would have: a stand-in pi answers the RPC query, and one that doesn't
    /// falls back to the table. A chosen model and depth are checked against it.
    #[test]
    fn a_chosen_model_and_depth_are_checked_against_the_runs_catalogue() {
        let dir = temp_dir("catalogue");
        let answer = serde_json::json!({"type":"response","command":"get_available_models","success":true,"data":{"models":[
            {"id":"m","name":"M","provider":"p","reasoning":true,"thinkingLevelMap":{"minimal":null,"low":"low","medium":null,"high":"high"}}
        ]}});
        let script = dir.join("pi");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ncase \" $* \" in\n  *\" --mode rpc \"*) cat >/dev/null; printf '%s\\n' '{answer}'; ;;\n  *\" --list-models \"*) printf '%s\\n' 'provider model context max-out thinking images' 'p table 1M 1K yes no'; ;;\nesac\n"
            ),
        )
        .expect("a stand-in");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755));
        }
        let config = BackendConfig { name: "Pi".into(), kind: crate::BackendKind::AgentCli, model: "pi".into(), base_url: None, ssh_host: None, enabled: true };
        let options = BackendOptions { command: Some(script.display().to_string()), ..Default::default() };
        let list = PiCli.model_catalog(&config, &options, None, &Stop::default()).expect("a catalogue");
        assert_eq!(list.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["p/m"], "{list:?}");
        assert_eq!(list[0].thinking, [Effort::None, Effort::Low, Effort::High], "the null levels and the absent xhigh/max");

        let chosen = BackendOptions { cli_model: Some("p/m".into()), cli_thinking: Some(Effort::High), ..options.clone() };
        assert!(PiCli.validate_model(&config, &chosen, None, &Stop::default()).is_ok());
        let too_deep = BackendOptions { cli_thinking: Some(Effort::Medium), ..chosen.clone() };
        let err = PiCli.validate_model(&config, &too_deep, None, &Stop::default()).expect_err("medium isn't mapped");
        assert!(err.message.contains("p/m"), "{}", err.message);
        let gone = BackendOptions { cli_model: Some("p/nope".into()), cli_thinking: None, ..options.clone() };
        assert!(PiCli.validate_model(&config, &gone, None, &Stop::default()).is_err(), "a model it doesn't have");

        // A pi that can't answer over RPC still lists its models in the table.
        let quiet = dir.join("quiet-pi");
        std::fs::write(&quiet, "#!/bin/sh\ncase \" $* \" in\n  *\" --list-models \"*) printf '%s\\n' 'provider model context max-out thinking images' 'p table 1M 1K yes no'; ;;\nesac\n").expect("a stand-in");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(&quiet, std::fs::Permissions::from_mode(0o755));
        }
        let quiet_options = BackendOptions { command: Some(quiet.display().to_string()), ..Default::default() };
        let list = PiCli.model_catalog(&config, &quiet_options, None, &Stop::default()).expect("the table");
        assert_eq!(list.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["p/table"], "{list:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A resumed conversation pi can't find is started again under the same id: the person is
    /// told, and the id is kept (pi's new conversation has it, so clearing it would start another).
    #[test]
    fn a_lost_session_is_reported_and_its_id_kept() {
        let mut p = StreamParser::new(Arc::new(PiCli));
        let events = PiCli.parse_stderr(&mut p, "Warning: No project session found with id 'abc'; creating a new session with that id.");
        assert!(matches!(events.as_slice(), [AgentEvent::Status { text }] if text.contains("started a new one")), "{events:?}");
        assert!(!p.session_missing, "pi's new session has the id it was given, so it isn't forgotten");
        let mut fresh = StreamParser::new(Arc::new(PiCli));
        assert!(PiCli.parse_stderr(&mut fresh, "some other warning").is_empty());
        assert!(!fresh.session_missing);
    }

    /// A folder of a test's own, so a run never writes over another's.
    fn temp_dir(name: &str) -> std::path::PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!("craft-anything-pi-test-{}-{}-{name}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).expect("a test folder");
        dir
    }

    /// The kept copy of a session, as the project would hold it.
    fn kept(id: &str) -> String {
        format!(
            "{{\"type\":\"session\",\"version\":3,\"id\":\"{id}\",\"timestamp\":\"2026-11-09T07:55:11.094Z\",\"cwd\":\"/w/P\"}}\n\
             {{\"type\":\"message\",\"id\":\"a\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"build a clock\"}}]}}}}\n\
             {{\"type\":\"message\",\"id\":\"b\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"Writing app/index.html\"}},{{\"type\":\"toolCall\",\"id\":\"c\",\"name\":\"build_file_write\",\"arguments\":{{}}}}]}}}}\n"
        )
    }

    /// pi names a working folder between two `--`, with its separators read as `-`.
    #[test]
    fn a_working_folder_is_named_the_way_pi_names_it() {
        assert_eq!(session_folder(std::path::Path::new("/private/tmp/p")), "--private-tmp-p--");
        assert_eq!(session_folder(std::path::Path::new(r"C:\w\p")), "--C--w-p--");
    }

    /// A project carried to another computer gets its builder's conversation back, in the folder
    /// pi looks for it in; a copy of another session is refused.
    #[test]
    fn a_session_travels_with_the_project() {
        let dir = temp_dir("session");
        let id = "01a1bf4d-d376-7776-9210-8cdc5ee15094";
        let cwd = std::path::Path::new("/w/P");
        assert!(session_file_in(&dir, id).is_none(), "nothing here yet");
        assert!(restore_session_in(&dir, id, kept(id).as_bytes(), cwd).expect("put back"), "it was put back");
        let path = session_file_in(&dir, id).expect("found by id");
        assert!(path.ends_with(format!("2026-11-09T07-55-11-094Z_{id}.jsonl")), "{path:?}");
        assert_eq!(path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()), Some("--w-P--"), "{path:?}");
        assert!(!restore_session_in(&dir, id, kept(id).as_bytes(), cwd).expect("already here"), "a second copy is nothing to do");
        // A copy of another session is refused where the session isn't here: nothing is written
        // under this one's name.
        let empty = temp_dir("wrong-session");
        let other = kept("11111111-2222-3333-4444-555555555555");
        assert!(restore_session_in(&empty, id, other.as_bytes(), cwd).is_err(), "another session's file is refused");
        assert!(session_file_in(&empty, id).is_none(), "and nothing was put there");
        let _ = std::fs::remove_dir_all(&empty);
        // An id pi would not use names no session.
        assert!(session_file_in(&dir, "../../etc/passwd").is_none());
        assert!(restore_session_in(&dir, "a/b", kept(id).as_bytes(), cwd).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The copy lands in the folder pi really uses: a linked path (`/tmp` on macOS) is resolved.
    #[test]
    fn a_copy_lands_in_the_real_folder_pi_uses() {
        let dir = temp_dir("canonical");
        let cwd = dir.join("project");
        std::fs::create_dir_all(&cwd).expect("a working folder");
        let id = "01a1bf4d-d376-7776-9210-8cdc5ee15094";
        assert!(restore_session_in(&dir, id, kept(id).as_bytes(), &cwd).expect("put back"));
        let real = std::fs::canonicalize(&cwd).expect("the real path");
        let path = session_file_in(&dir, id).expect("found by id");
        let folder = path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str());
        assert_eq!(folder, Some(session_folder(&real).as_str()), "{path:?}");
        // A second run writing under the same id is the one a copy keeps.
        let newer = path.with_file_name(format!("2099-01-01T00-00-00-000Z_{id}.jsonl"));
        std::fs::write(&newer, kept(id)).expect("a newer file");
        assert_eq!(session_file_in(&dir, id).as_deref(), Some(newer.as_path()), "the newest file wins");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What a new conversation is told of the earlier one: the newest exchanges, clipped.
    #[test]
    fn a_handoff_keeps_the_newest_exchange_and_leaves_out_tool_calls() {
        let text = pi_handoff(&kept("x"), 16_000).expect("something to tell");
        assert!(text.contains("Craft Anything asked you: build a clock"), "{text}");
        assert!(text.contains("You answered: Writing app/index.html"), "{text}");
        assert!(!text.contains("build_file_write"), "a tool call is not an answer: {text}");
        assert!(pi_handoff("", 16_000).is_none());
        // Only the newest entry fits when the room is small.
        let narrow = pi_handoff(&kept("x"), 40).expect("one entry");
        assert_eq!(narrow, "You answered: Writing app/index.html", "{narrow}");
    }

    /// A server, a tool or a flag with a quote in it must not break the extension's JavaScript.
    #[test]
    fn quotes_are_escaped_for_javascript() {
        let mcp = McpCommand {
            command: r#"C:\it's "here"\craft-anything-cli.exe"#.into(),
            args: vec!["mcp".into(), "--project".into(), r"C:\it's here".into()],
            notes: None,
        };
        let js = extension_js(&mcp);
        assert!(js.contains(r#""C:\\it's \"here\"\\craft-anything-cli.exe""#), "{js}");
        assert!(js.contains(r#""C:\\it's here""#), "{js}");
        assert!(js.contains(&format!("registerMcpServer(\"{SERVER}\"")), "{js}");
        // The bridge's own command survives the quoting too.
        assert!(js.contains(r#""core-action-approve""#), "{js}");
        assert!(js.contains(r#""C:\\it's here""#), "the project argument is quoted: {js}");
    }
}
