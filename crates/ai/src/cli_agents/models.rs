//! Model discovery through the selected agent's own CLI, in the same execution context as a run.
//! Only public model metadata is retained; provider headers and credentials are never returned.

use crate::model::{Effort, Stop};
use crate::{BackendConfig, BackendEntry, BackendOptions};
use craft_anything_evidence::{ErrorCode, ToolError};
use craft_anything_platform::{Output, Process, process};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentModel {
    /// The selectable reference (provider/id), including aliases defined in OpenCode.
    pub id: String,
    pub name: String,
    pub thinking: Vec<Effort>,
}

/// A snapshot of the context a picker queried. Also used to reject stale background results.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelContext {
    pub backend: BackendEntry,
    pub directory: Option<PathBuf>,
}

const MAX_OUTPUT: usize = 8 * 1024 * 1024;
const MAX_MODELS: usize = 10_000;

#[derive(Deserialize)]
struct Response {
    data: Vec<WireModel>,
}

#[derive(Deserialize)]
struct WireModel {
    id: String,
    #[serde(rename = "providerID")]
    provider: String,
    name: String,
    enabled: bool,
    #[serde(rename = "type")]
    kind: Option<String>,
    variants: Vec<Variant>,
}

#[derive(Deserialize)]
struct Variant {
    id: String,
}

/// Describe an invalid catalogue without including potentially sensitive raw CLI output.
fn malformed() -> ToolError {
    ToolError::new(ErrorCode::GarbledOutput, "OpenCode returned an unreadable model list.")
        .with_remediation("Use OpenCode V2 on the selected computer and try loading its models again.")
}

/// Parse a bounded V2 catalogue into sorted, deduplicated enabled chat models and known depths.
/// Reject malformed references or oversized responses; discard unrelated provider metadata.
pub(crate) fn parse_models(bytes: &[u8]) -> Result<Vec<AgentModel>, ToolError> {
    if bytes.len() >= MAX_OUTPUT {
        return Err(ToolError::new(ErrorCode::OverLimit, "OpenCode's model list is too large."));
    }
    let response: Response = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if response.data.len() > MAX_MODELS {
        return Err(ToolError::new(ErrorCode::OverLimit, "OpenCode returned too many models."));
    }
    let mut models = std::collections::BTreeMap::new();
    for m in response.data {
        if !m.enabled || m.kind.as_deref().is_some_and(|k| k != "chat") {
            continue;
        }
        if m.id.is_empty() || m.provider.is_empty() || m.id.contains('#') || m.provider.contains(['/', '#']) {
            return Err(malformed());
        }
        let id = format!("{}/{}", m.provider, m.id);
        let thinking = Effort::ALL.into_iter().filter(|d| m.variants.iter().any(|v| v.id == d.as_str())).collect();
        models.insert(id.clone(), AgentModel { id, name: m.name, thinking });
    }
    Ok(models.into_values().collect())
}

/// Only connection flags belong on both `api` and `run`; prompt/run flags never leak into discovery.
/// Without a server the person chose, discovery asks a private server (`--standalone`), as runs do:
/// plain `api` starts OpenCode's shared service, which resumes every unfinished session on the
/// computer, in any project, and carries on with its commands (#1115).
fn connection_args(options: &BackendOptions) -> Result<Vec<String>, ToolError> {
    let mut result = Vec::new();
    let mut args = options.args.iter();
    while let Some(arg) = args.next() {
        if arg == "--standalone" || arg.starts_with("--server=") {
            result.push(arg.clone());
        } else if arg == "--server" {
            let server =
                args.next().filter(|s| !s.is_empty() && !s.starts_with('-')).ok_or_else(|| ToolError::invalid("OpenCode's --server needs an address."))?;
            result.extend([arg.clone(), server.clone()]);
        }
    }
    if result.is_empty() {
        result.push("--standalone".into());
    }
    Ok(result)
}

/// Build the model-list operation with connection flags shared by discovery and execution.
pub(crate) fn query_args(options: &BackendOptions) -> Result<Vec<String>, ToolError> {
    let mut args = vec!["api".into(), "model.list".into()];
    args.extend(connection_args(options)?);
    Ok(args)
}

/// Select the configured executable and catalogue location, quoting SSH arguments for the remote shell.
/// Local queries use the supplied directory or current directory; SSH queries expand remote `$PWD`.
fn query_command(config: &BackendConfig, options: &BackendOptions, directory: Option<&Path>) -> Result<(String, Vec<String>), ToolError> {
    let program = super::program_of(config, options)?;
    let mut args = query_args(options)?;
    match config.ssh_host.as_deref().filter(|h| !h.trim().is_empty()) {
        Some(host) => {
            let mut remote = vec![program];
            remote.extend(args);
            let mut args = crate::ssh::ssh_args(host, &remote);
            // Expand only the remote shell's working directory. No local path is sent to SSH.
            // The operation-id form accepts deep-object query parameters; raw `get` does not.
            if let Some(command) = args.last_mut() {
                command.push_str(" --param \"location[directory]=$PWD\"");
            }
            Ok(("ssh".into(), args))
        }
        None => {
            let directory = match directory {
                Some(dir) => dir.to_path_buf(),
                None => std::env::current_dir().map_err(|e| ToolError::io(".", e))?,
            };
            args.extend(["--param".into(), format!("location[directory]={}", directory.display())]);
            Ok((program, args))
        }
    }
}

/// Prepare an unstarted catalogue process with bounded output, deadlines and a local-only working directory.
pub(crate) fn query_process(config: &BackendConfig, options: &BackendOptions, directory: Option<&Path>) -> Result<Process, ToolError> {
    let (program, args) = query_command(config, options, directory)?;
    let mut process = Process::new(program).args(args);
    if let Some(dir) = directory.filter(|_| config.ssh_host.as_deref().is_none_or(|h| h.trim().is_empty())) {
        process = process.cwd(dir);
    }
    Ok(process.timeout(Duration::from_secs(20)).idle_timeout(Duration::from_secs(15)).stdout_cap(MAX_OUTPUT).stderr_cap(64 * 1024))
}

/// Query the selected installation, propagating cancellation, process failures and invalid catalogue errors.
pub(crate) fn discover(config: &BackendConfig, options: &BackendOptions, directory: Option<&Path>, stop: &Stop) -> Result<Vec<AgentModel>, ToolError> {
    if let Some(models) = from_private_server(config, options, directory, stop)? {
        return Ok(models);
    }
    let query = || query_once(config, options, directory, stop, None);
    let out = match query() {
        // A private server can answer before its plugins have started (#1859): ask once more.
        Err(e) if still_starting(&e) => {
            pause(stop, STARTING_PAUSE)?;
            query()?
        }
        r => r?,
    };
    parse_models(&out.stdout)
}

/// How long to give OpenCode's plugins before asking again (#1859).
const STARTING_PAUSE: Duration = Duration::from_secs(2);

/// How long a private server gets to say where it listens, and then to load its models (#2372).
const SERVER_START: Duration = Duration::from_secs(10);
const CATALOGUE_WAIT: Duration = Duration::from_secs(10);

/// OpenCode 2.4 answers a new private server's first model list before its models have loaded, so a
/// one-shot `--standalone` list is always empty (#2372). Locally, with no server the person chose,
/// one private server (`serve` on 127.0.0.1, its own password) is asked until its list arrives, then
/// stopped. `None` when it doesn't start, as with an OpenCode without `serve`: the one-shot query is used.
fn from_private_server(config: &BackendConfig, options: &BackendOptions, directory: Option<&Path>, stop: &Stop) -> Result<Option<Vec<AgentModel>>, ToolError> {
    // shortcut: over SSH the one-shot query stays, so an OpenCode 2.4 there still lists nothing; upgrade with a remote serve.
    if config.ssh_host.as_deref().is_some_and(|h| !h.trim().is_empty()) || connection_args(options)? != ["--standalone"] {
        return Ok(None);
    }
    let program = super::program_of(config, options)?;
    let (tx, rx) = std::sync::mpsc::channel();
    let mut serve = Process::new(&program)
        .args(["serve", "--hostname", "127.0.0.1", "--port", "0"])
        // Without one set, OpenCode makes a password and prints it.
        .env_remove("OPENCODE_SERVER_PASSWORD")
        .on_stdout_line(move |l| {
            let _ = tx.send(l.to_string());
        })
        .timeout(SERVER_START + CATALOGUE_WAIT + Duration::from_secs(30))
        .stdout_cap(64 * 1024)
        .stderr_cap(64 * 1024);
    if let Some(dir) = directory {
        serve = serve.cwd(dir);
    }
    let Ok(mut server) = serve.spawn() else { return Ok(None) };
    let found = ask_private_server(&mut server, &rx, config, options, directory, stop);
    // Dropping only flags the kill; a CLI that exits next would leave the server running.
    server.kill_and_wait(Duration::from_secs(5));
    found
}

/// Read where the started server listens and its password, then ask it until its models arrive.
/// A successful empty list is the answer once [`CATALOGUE_WAIT`] is up; with no successful answer
/// by then, the last "still starting" or timed-out request's error is.
fn ask_private_server(
    server: &mut craft_anything_platform::RunningProcess,
    rx: &std::sync::mpsc::Receiver<String>,
    config: &BackendConfig,
    options: &BackendOptions,
    directory: Option<&Path>,
    stop: &Stop,
) -> Result<Option<Vec<AgentModel>>, ToolError> {
    let (mut url, mut password) = (None, None);
    let started = std::time::Instant::now();
    while url.is_none() || password.is_none() {
        stop.check()?;
        if started.elapsed() > SERVER_START || server.is_finished() {
            return Ok(None);
        }
        if let Ok(line) = rx.recv_timeout(Duration::from_millis(100)) {
            if let Some(u) = line.strip_prefix("server listening on ") {
                url = Some(u.trim().to_string());
            } else if let Some(p) = line.strip_prefix("server password ") {
                password = Some(p.trim().to_string());
            }
        }
    }
    let (Some(url), Some(password)) = (url, password) else { return Ok(None) };
    let at_server = BackendOptions { args: vec!["--server".into(), url], ..options.clone() };
    let until = std::time::Instant::now() + CATALOGUE_WAIT;
    loop {
        let left = until.saturating_duration_since(std::time::Instant::now()).max(Duration::from_secs(1));
        let answer = match query_once(config, &at_server, directory, stop, Some((&password, left))) {
            Ok(out) => Ok(parse_models(&out.stdout)?),
            Err(e) if still_starting(&e) || e.code == ErrorCode::TimedOut => Err(e),
            Err(e) => return Err(e),
        };
        if answer.as_ref().is_ok_and(|m| !m.is_empty()) || std::time::Instant::now() >= until {
            return answer.map(Some);
        }
        pause(stop, Duration::from_millis(250))?;
    }
}

/// Ask for the model list once. OpenCode prints its typed error (`{"_tag":…,"message":…}`) on
/// stdout but only `HTTP 503 Service Unavailable` on stderr, and a failed run's error keeps only
/// stderr (#1859), so the typed answer is read here, named in the message and kept in the details.
/// `private`: the password and time limit for a private server's request (#2372).
fn query_once(
    config: &BackendConfig,
    options: &BackendOptions,
    directory: Option<&Path>,
    stop: &Stop,
    private: Option<(&str, Duration)>,
) -> Result<Output, ToolError> {
    let (program, args) = query_command(config, options, directory)?;
    let stop = stop.clone();
    let mut process = query_process(config, options, directory)?;
    if let Some((password, limit)) = private {
        process = process.env("OPENCODE_SERVER_PASSWORD", password).timeout(limit).idle_timeout(limit);
    }
    let out = process
        .cancel_when(move || stop.is_stopped())
        .remediation("Check OpenCode V2 and its provider setup on the selected computer, then reload the models.")
        .run()?;
    if out.success {
        return Ok(out);
    }
    let name = Path::new(&program).file_name().map_or_else(|| program.clone(), |n| n.to_string_lossy().into_owned());
    let mut err = process::nonzero_error_with(&name, &out, Some(&process::printable_command(OsStr::new(&program), &args)));
    if let Some((tag, message)) = typed_error(&out.stdout).or_else(|| typed_error(&out.stderr)) {
        let code = out.code.map_or_else(|| "a signal".into(), |c| c.to_string());
        err.message = format!("{name} stopped with exit code {code}: {message}");
        if tag == STARTING_TAG {
            let rest = err.remediation.take().unwrap_or_default();
            err.remediation = Some(format!("OpenCode's plugins hadn't finished starting. Try again in a moment.\n\n{rest}"));
        }
        if let Some(serde_json::Value::Object(d)) = err.details.as_mut() {
            d.insert("opencode_error".into(), json!({"tag": tag, "message": message}));
        }
    }
    Err(err)
}

/// OpenCode's tag for a server that is still starting its plugins.
const STARTING_TAG: &str = "ServiceUnavailableError";

/// The last `{"_tag":…,"message":…}` line OpenCode printed, as (tag, message).
fn typed_error(output: &[u8]) -> Option<(String, String)> {
    String::from_utf8_lossy(output).lines().rev().find_map(|line| {
        let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
        let tag = v.get("_tag")?.as_str()?;
        let message = v.get("message").and_then(serde_json::Value::as_str).unwrap_or(tag);
        Some((tag.chars().take(100).collect(), message.chars().take(500).collect()))
    })
}

/// OpenCode's typed answer while it's still starting: `{"_tag":"ServiceUnavailableError",…}`.
fn still_starting(e: &ToolError) -> bool {
    e.code == ErrorCode::ProgramFailed
        && e.details.as_ref().and_then(|d| d.pointer("/opencode_error/tag")).and_then(serde_json::Value::as_str) == Some(STARTING_TAG)
}

/// Wait `total`, ending early with the stop's error when it's stopped.
fn pause(stop: &Stop, total: Duration) -> Result<(), ToolError> {
    let until = std::time::Instant::now() + total;
    while std::time::Instant::now() < until {
        stop.check()?;
        std::thread::sleep(Duration::from_millis(50));
    }
    stop.check()
}

/// Validate explicit selections against the current instance before saving or starting a run.
pub(crate) fn validate(options: &BackendOptions, models: &[AgentModel]) -> Result<(), ToolError> {
    let Some(id) = options.cli_model.as_deref() else {
        return if options.cli_thinking.is_none() { Ok(()) } else { Err(ToolError::invalid("Choose a model before choosing its thinking depth.")) };
    };
    let Some(model) = models.iter().find(|m| m.id == id) else {
        return Err(ToolError::invalid(format!("The selected model {id:?} is no longer available."))
            .with_remediation("Reload the models on this computer and choose an available model or the agent's default."));
    };
    if let Some(depth) = options.cli_thinking.filter(|d| !model.thinking.contains(d)) {
        return Err(ToolError::invalid(format!("{} does not offer {} thinking.", model.id, depth.title()))
            .with_remediation("Choose a thinking depth listed for this model, or use its default."));
    }
    Ok(())
}

/// Shared by the hub and headless tools. An empty model means OpenCode chooses its own default.
pub fn select(context: &ModelContext, model: Option<&str>, thinking: Option<Effort>) -> Result<BackendOptions, ToolError> {
    select_with_stop(context, model, thinking, &Stop::default())
}

/// Return validated options for a catalogue-capable agent without persisting them.
/// Empty model names select the default; cancellation is checked before any discovery I/O.
pub fn select_with_stop(context: &ModelContext, model: Option<&str>, thinking: Option<Effort>, stop: &Stop) -> Result<BackendOptions, ToolError> {
    stop.check()?;
    let adapter = super::adapter_for(&context.backend.config.model);
    if context.backend.config.kind != crate::BackendKind::AgentCli || !adapter.has_model_catalog() {
        return Err(ToolError::invalid("This agent does not offer a live model and thinking catalogue."));
    }
    let mut options = context.backend.options.clone();
    options.cli_model = model.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    options.cli_thinking = thinking;
    if adapter.requires_acp() {
        options.cli_reasoning_effort = None;
        options.effort = None;
        if options.cli_model != context.backend.options.cli_model {
            options.cli_context = None;
        }
    }
    adapter.validate_model(&context.backend.config, &options, context.directory.as_deref(), stop)?;
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An owner's cancellation survives reset and prevents model-selection I/O.
    #[test]
    fn a_cancelled_tool_stops_selection_before_io() {
        let cancel = craft_anything_engine::CancelToken::default();
        let stop = Stop::for_tool(cancel.clone());
        cancel.cancel();
        assert!(stop.is_stopped());
        stop.reset();
        assert!(stop.is_stopped(), "a conversation reset cannot revive its cancelled owner");
        let context = ModelContext { backend: BackendEntry { config: config(), options: BackendOptions::default() }, directory: None };
        assert_eq!(select_with_stop(&context, None, None, &stop).unwrap_err().code, ErrorCode::Cancelled);
    }

    /// Build catalogue metadata with an alias distinct from its upstream model id.
    fn model(id: &str, enabled: bool, kind: &str, variants: &[&str]) -> serde_json::Value {
        json!({"id":id,"modelID":"upstream-name","providerID":"test","name":id,"enabled":enabled,"type":kind,
            "variants":variants.iter().map(|v| json!({"id":v})).collect::<Vec<_>>()})
    }

    /// Filtering retains selectable aliases and supported depths while rejecting malformed or oversized input.
    #[test]
    fn only_enabled_chat_models_and_their_actual_variants_are_offered() {
        let text = json!({"data":[
            model("alias",true,"chat", &["max","low","none","xhigh","high","medium"]),
            model("hidden",false,"chat", &["max"]), model("picture",true,"image", &[]),
            model("small",true,"chat", &["low","high"]), model("alias",true,"chat", &["low","medium","high","xhigh","max"])
        ]});
        let list = parse_models(text.to_string().as_bytes()).unwrap();
        assert_eq!(list.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["test/alias", "test/small"]);
        assert_eq!(list[0].thinking, Effort::ALL);
        assert_eq!(list[1].thinking, [Effort::Low, Effort::High]);
        assert!(parse_models(br#"{"data":[]}"#).unwrap().is_empty());
        for invalid in [b"not json".as_slice(), br#"{"data":[{}]}"#, br#"{}"#] {
            assert_eq!(parse_models(invalid).unwrap_err().code, ErrorCode::GarbledOutput);
        }
        assert_eq!(parse_models(&vec![b' '; MAX_OUTPUT]).unwrap_err().code, ErrorCode::OverLimit);
    }

    /// Explicit unavailable models and unsupported depths fail instead of silently changing the selection.
    #[test]
    fn a_selection_never_falls_back_to_another_model_or_thinking_depth() {
        let list = vec![AgentModel { id: "test/model".into(), name: "Test".into(), thinking: vec![Effort::High] }];
        let mut options = BackendOptions::default();
        assert!(validate(&options, &list).is_ok());
        options.cli_thinking = Some(Effort::High);
        assert!(validate(&options, &list).is_err());
        options.cli_model = Some("test/model".into());
        assert!(validate(&options, &list).is_ok());
        options.cli_thinking = Some(Effort::Max);
        assert!(validate(&options, &list).is_err());
        options.cli_model = Some("test/removed".into());
        options.cli_thinking = None;
        assert!(validate(&options, &list).is_err());
    }

    /// An isolated local backend for catalogue fixtures.
    fn config() -> BackendConfig {
        BackendConfig { name: "OpenCode".into(), kind: crate::BackendKind::AgentCli, model: "opencode".into(), base_url: None, ssh_host: None, enabled: true }
    }

    /// Discovery preserves the chosen executable and server, omitting run-only flags and local paths over SSH.
    #[test]
    fn remote_discovery_uses_the_remote_program_and_connection_context() {
        let mut cfg = config();
        cfg.ssh_host = Some("test-box".into());
        let options = BackendOptions {
            command: Some("/opt/agent's tools/opencode".into()),
            args: ["--agent", "build", "--server", "http://localhost:4096"].map(String::from).into(),
            ..Default::default()
        };
        let (program, args) = query_command(&cfg, &options, None).unwrap();
        assert_eq!(program, "ssh");
        assert!(args.iter().any(|a| a == "BatchMode=yes"));
        assert!(args.iter().any(|a| a == "test-box"));
        assert_eq!(
            args.last().unwrap(),
            "'/opt/agent'\\''s tools/opencode' api model.list --server http://localhost:4096 --param \"location[directory]=$PWD\""
        );
        // Locally a chosen path must exist: a missing one falls back to an installed `opencode`.
        cfg.ssh_host = None;
        let folder = tempfile::tempdir().unwrap();
        let chosen = folder.path().join("agent's tools").join("opencode");
        std::fs::create_dir_all(chosen.parent().unwrap()).unwrap();
        std::fs::write(&chosen, b"").unwrap();
        let chosen = chosen.display().to_string();
        let options = BackendOptions { command: Some(chosen.clone()), ..options };
        let (program, args) = query_command(&cfg, &options, Some(Path::new("/work/test"))).unwrap();
        assert_eq!(program, chosen);
        assert_eq!(args, ["api", "model.list", "--server", "http://localhost:4096", "--param", "location[directory]=/work/test"]);
    }

    /// A shell stand-in checks remote directory expansion and quoting of paths with special characters.
    #[cfg(unix)]
    #[test]
    fn a_remote_query_expands_only_the_remote_working_directory() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let remote = dir.path().join("remote & project # one");
        std::fs::create_dir(&remote).unwrap();
        let program = dir.path().join("agent's program");
        std::fs::write(&program, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut cfg = config();
        cfg.ssh_host = Some("test-box".into());
        let options = BackendOptions { command: Some(program.display().to_string()), ..Default::default() };
        let (_, args) = query_command(&cfg, &options, Some(Path::new("/local/ignored"))).unwrap();
        let output = Process::new("sh").arg("-c").arg(args.last().unwrap()).cwd(&remote).timeout(Duration::from_secs(2)).run_checked().unwrap();
        // The shell resolves aliases in its working directory (macOS /var points to /private/var).
        let remote = remote.canonicalize().unwrap();
        assert_eq!(
            output.stdout_text().lines().collect::<Vec<_>>(),
            ["api", "model.list", "--standalone", "--param", &format!("location[directory]={}", remote.display())]
        );
    }

    /// #1115: plain `api` starts OpenCode's shared service, which resumes unfinished sessions in other
    /// projects. Discovery uses a private server unless the person chose one, and never adds it twice.
    #[test]
    fn discovery_never_starts_the_shared_service() {
        let with = |args: &[&str]| query_args(&BackendOptions { args: args.iter().map(|a| a.to_string()).collect(), ..Default::default() }).unwrap();
        assert_eq!(with(&[]), ["api", "model.list", "--standalone"]);
        assert_eq!(with(&["--agent", "build"]), ["api", "model.list", "--standalone"], "run-only flags don't count as a choice");
        assert_eq!(with(&["--standalone"]), ["api", "model.list", "--standalone"]);
        assert_eq!(with(&["--server", "http://localhost:4096"]), ["api", "model.list", "--server", "http://localhost:4096"]);
        assert_eq!(with(&["--server=http://localhost:4096"]), ["api", "model.list", "--server=http://localhost:4096"]);
    }

    /// A local CLI stand-in verifies project-specific discovery, malformed output, exit errors and deadlines.
    #[cfg(unix)]
    #[test]
    fn discovery_reads_the_selected_program_in_the_project_and_reports_errors() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("fake agent");
        let script = "#!/bin/sh\n[ \"$1 $2\" = 'api model.list' ] || exit 9\ncat models.json\n";
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let options = BackendOptions { command: Some(program.display().to_string()), ..Default::default() };
        let data = json!({"data":[model("project-only",true,"chat", &["high"])]});
        std::fs::write(dir.path().join("models.json"), data.to_string()).unwrap();
        assert_eq!(discover(&config(), &options, Some(dir.path()), &Stop::default()).unwrap()[0].id, "test/project-only");
        std::fs::write(dir.path().join("models.json"), "invalid").unwrap();
        assert_eq!(discover(&config(), &options, Some(dir.path()), &Stop::default()).unwrap_err().code, ErrorCode::GarbledOutput);
        std::fs::write(&program, "#!/bin/sh\nexit 7\n").unwrap();
        assert!(discover(&config(), &options, Some(dir.path()), &Stop::default()).is_err());
        std::fs::write(&program, "#!/bin/sh\nsleep 5\n").unwrap();
        let err = query_process(&config(), &options, Some(dir.path())).unwrap().timeout(Duration::from_millis(50)).run_checked().unwrap_err();
        assert_eq!(err.code, ErrorCode::TimedOut);
    }

    /// #1859: OpenCode's 503 while its plugins start is asked about once more; the second answer is used.
    #[cfg(unix)]
    #[test]
    fn a_server_still_starting_is_asked_once_more() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("opencode");
        let busy = r#"{"_tag":"ServiceUnavailableError","message":"Location plugins did not finish activating","service":"plugin"}"#;
        let script = format!(
            "#!/bin/sh\n[ \"$1\" = serve ] && exit 1\nprintf x >> tries\nif [ ! -f started ]; then touch started; echo '{busy}'; echo 'HTTP 503 Service Unavailable' >&2; exit 1; fi\ncat models.json\n"
        );
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let options = BackendOptions { command: Some(program.display().to_string()), ..Default::default() };
        std::fs::write(dir.path().join("models.json"), json!({"data":[model("m",true,"chat", &[])]}).to_string()).unwrap();
        assert_eq!(discover(&config(), &options, Some(dir.path()), &Stop::default()).unwrap()[0].id, "test/m");
        assert_eq!(std::fs::read_to_string(dir.path().join("tries")).unwrap(), "xx", "asked twice");
        // Still starting the second time: that answer is the error, and it isn't asked a third time.
        std::fs::write(dir.path().join("tries"), "").unwrap();
        std::fs::write(
            &program,
            format!("#!/bin/sh\n[ \"$1\" = serve ] && exit 1\nprintf x >> tries\necho '{busy}'\necho 'HTTP 503 Service Unavailable' >&2\nexit 1\n"),
        )
        .unwrap();
        let err = discover(&config(), &options, Some(dir.path()), &Stop::default()).unwrap_err();
        assert!(still_starting(&err), "{err:?}");
        assert!(err.message.ends_with(": Location plugins did not finish activating"), "OpenCode's reason, not the HTTP status: {}", err.message);
        assert!(err.remediation.as_deref().unwrap_or_default().starts_with("OpenCode's plugins hadn't finished starting."), "{err:?}");
        assert_eq!(std::fs::read_to_string(dir.path().join("tries")).unwrap(), "xx");
    }

    /// #2372: a private server's first list is empty until its models load; it is asked until they arrive.
    #[cfg(unix)]
    #[test]
    fn a_private_server_is_asked_until_its_models_have_loaded() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("opencode");
        let script = "#!/bin/sh\ncase \"$1\" in\nserve) echo 'server listening on http://127.0.0.1:9'; echo 'server password pw'; exec sleep 30;;\n\
            api) [ \"$3\" = --server ] && [ \"$OPENCODE_SERVER_PASSWORD\" = pw ] || exit 9; printf x >> tries\n\
            if [ \"$(cat tries)\" = x ]; then echo '{\"data\":[]}'; else cat models.json; fi;;\nesac\n";
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let options = BackendOptions { command: Some(program.display().to_string()), ..Default::default() };
        std::fs::write(dir.path().join("models.json"), json!({"data":[model("m",true,"chat", &[])]}).to_string()).unwrap();
        assert_eq!(discover(&config(), &options, Some(dir.path()), &Stop::default()).unwrap()[0].id, "test/m");
        assert_eq!(std::fs::read_to_string(dir.path().join("tries")).unwrap(), "xx", "the empty first answer isn't the list");
    }

    /// Only OpenCode's typed "still starting" answer is asked again, not any failure.
    #[test]
    fn only_a_server_still_starting_is_asked_again() {
        let failed = |tag: &str| {
            ToolError::new(ErrorCode::ProgramFailed, "opencode stopped with exit code 1.").with_details(json!({"opencode_error": {"tag": tag, "message": "m"}}))
        };
        assert!(still_starting(&failed("ServiceUnavailableError")));
        assert!(!still_starting(&failed("ProviderAuthError")));
        let prose = ToolError::new(ErrorCode::ProgramFailed, "x").with_details(json!({"output_tail": "ServiceUnavailableError in some prose"}));
        assert!(!still_starting(&prose), "the typed tag, not the word");
        // The typed line is found among log lines, and prose or untagged JSON isn't taken for one.
        let printed = b"level=INFO message=x\n{\"_tag\":\"ServiceUnavailableError\",\"message\":\"Location plugins did not finish activating\"}\n\n";
        assert_eq!(typed_error(printed), Some(("ServiceUnavailableError".into(), "Location plugins did not finish activating".into())));
        assert_eq!(typed_error(b"HTTP 503 Service Unavailable\n[1,2]\n{\"message\":\"no tag\"}\n"), None);
        assert!(!still_starting(&ToolError::new(ErrorCode::TimedOut, "timed out")));
        // A stop during the pause ends it with the stop's error.
        let stop = Stop::default();
        stop.stop();
        assert!(pause(&stop, Duration::from_secs(5)).is_err());
    }
}
