//! The person's installed Kimi Code CLI (`kimi`), driven over ACP (`kimi acp`). Sign-in stays in
//! `kimi login`; Craft Anything checks that a saved credential file exists and does not read it.
//!
//! Why ACP and not `kimi -p`: the print mode takes the message only as the value of `-p/--prompt`
//! (it reads nothing from stdin and has no file option), so a long brief would pass Windows'
//! command-line limit (#402); it also has no flag that hands a run our tool server (its servers
//! live in the person's own `mcp.json`) and it approves file edits and commands by itself. Over
//! ACP the message travels in the session, our server is handed over in `session/new`
//! (`mcpServers`), and tool approvals come back as `session/request_permission`.
//!
//! What the published sources say (MoonshotAI/kimi-code, read 2026-11-20): `kimi acp` answers
//! `session/new` only when signed in (it fails with ACP's `authRequired`, -32000, otherwise),
//! advertises a terminal `login` auth method the host can't complete for the person, and takes a
//! model through `session/set_config_option` with `configId: "model"`. No live run has checked that
//! our server's tools reach a session or which edits ask first, so access is held to a deep build.

use super::*;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Kimi Code's ACP-only adapter. `acp = false` or an SSH host fails closed: the plain runner would
/// open the interactive terminal interface, which has no approval channel back to Craft Anything.
pub struct KimiCode;

impl AgentAdapter for KimiCode {
    fn ids(&self) -> &'static [&'static str] {
        // The program is `kimi`; a backend saved under that name is this agent too.
        &["kimi-code", "kimi"]
    }

    fn args(&self, _cx: &ArgsCx) -> Vec<String> {
        // The native runner refuses `requires_acp` before it asks for arguments.
        Vec::new()
    }

    fn requires_acp(&self) -> bool {
        true
    }

    fn has_model_catalog(&self) -> bool {
        true
    }

    fn model_catalog(
        &self,
        config: &BackendConfig,
        options: &BackendOptions,
        directory: Option<&Path>,
        stop: &Stop,
    ) -> Result<Vec<models::AgentModel>, ToolError> {
        let metadata = super::super::acp_model_metadata(config, options, directory, stop)?;
        Ok(catalogue(&metadata))
    }

    /// Only the options are checked here: the model goes to `session/set_config_option`, and a name
    /// Kimi doesn't know fails there with its own message, so a catalogue that came back empty
    /// can't refuse a model Kimi would take.
    fn validate_model(&self, _config: &BackendConfig, options: &BackendOptions, _directory: Option<&Path>, stop: &Stop) -> Result<(), ToolError> {
        stop.check()?;
        self.validate_options(options)
    }

    fn plain_fallback(&self) -> bool {
        false
    }

    /// Questions reach [`validate_access`](Self::validate_access), which refuses them with a reason,
    /// rather than being turned away with no word about Kimi.
    fn answers_questions(&self) -> bool {
        true
    }

    fn model_in_use(&self, options: Option<&BackendOptions>) -> Option<String> {
        Some(chosen_model(options).unwrap_or_else(|| "Kimi Code default".into()))
    }

    fn model_choices(&self, config: &BackendConfig, options: &BackendOptions) -> Result<Vec<String>, ToolError> {
        Ok(self.model_catalog(config, options, None, &Stop::default())?.into_iter().map(|model| model.id).collect())
    }

    fn check_tool_server(&self, stop: &CancelToken) -> Result<(), ToolError> {
        cli_can_start(craft_anything_platform::self_cli_path().as_deref(), stop)
    }

    /// Extra arguments would land before `acp`, and `--yolo`/`--auto` there would approve
    /// everything; none are passed, and [`validate_options`](Self::validate_options) refuses them.
    fn acp_options(&self, _options: &BackendOptions, _access: Access) -> Vec<String> {
        Vec::new()
    }

    fn validate_options(&self, options: &BackendOptions) -> Result<(), ToolError> {
        if options.args.is_empty() {
            Ok(())
        } else {
            Err(ToolError::invalid("Extra Kimi Code arguments could bypass Craft Anything's approval checks.")
                .with_remediation("Remove this backend's extra arguments. Set its program and model through the backend options instead."))
        }
    }

    /// Kimi's own reads don't ask permission, and its saved permission mode (`yolo`, `auto`) can
    /// edit and run commands without a request, so Clean room, deep chat and questions promise
    /// limits it can't be held to. A deep build is allowed; an edit is gated only when Kimi sends
    /// `session/request_permission`.
    fn validate_access(&self, access: Access) -> Result<(), ToolError> {
        match access {
            Access::Build => Ok(()),
            Access::Analyse | Access::CleanRoomAnalyse | Access::CleanRoomBuild | Access::CleanRoomQuestion | Access::Question => {
                Err(ToolError::invalid("Kimi Code reads, and may edit, without asking, so a Clean room run, a deep chat, or a question can't hold it back.")
                    .with_remediation("Use a deep build. Kimi asks before an edit only when its own permission mode sends a request."))
            }
        }
    }

    fn acp_session_config(&self, options: &BackendOptions) -> Vec<(String, String)> {
        chosen_model(Some(options)).map(|model| ("model".to_string(), model)).into_iter().collect()
    }

    fn sign_in_fix(&self) -> Option<String> {
        Some("Sign in with `kimi login` in a terminal, then try again. Kimi Code keeps the sign-in; Craft Anything never sees it.".into())
    }

    fn signed_in(&self, _program: &str) -> Option<bool> {
        let home = std::env::var_os("KIMI_CODE_HOME");
        credentials_present(credentials_dir(home.as_deref(), dirs::home_dir().as_deref()))
    }
}

/// The model the backend names, trimmed; `None` keeps Kimi's own `default_model`.
fn chosen_model(options: Option<&BackendOptions>) -> Option<String> {
    options.and_then(|options| options.cli_model.as_deref()).map(str::trim).filter(|model| !model.is_empty()).map(str::to_string)
}

/// `KIMI_CODE_HOME/credentials` when that variable is set, otherwise `~/.kimi-code/credentials`:
/// where `kimi login` keeps its OAuth credentials (`credentials/<name>.json`).
fn credentials_dir(kimi_home: Option<&OsStr>, user_home: Option<&Path>) -> Option<PathBuf> {
    let dir = match kimi_home {
        Some(home) if !home.is_empty() => PathBuf::from(home),
        _ => user_home?.join(".kimi-code"),
    };
    Some(dir.join("credentials"))
}

/// `Some(true)` when the folder holds a `.json` file. Only names are looked at: no file is opened.
/// Missing is unknown (`None`), not "signed out": a provider key in Kimi's `config.toml` signs in
/// without a credential file, and the run reports an authentication problem itself. At most a few
/// hundred entries are looked at.
fn credentials_present(dir: Option<PathBuf>) -> Option<bool> {
    let entries = std::fs::read_dir(dir?).ok()?;
    let found = entries
        .take(512)
        .filter_map(Result::ok)
        .any(|entry| entry.path().extension().is_some_and(|ext| ext == "json") && entry.file_type().is_ok_and(|kind| kind.is_file()));
    found.then_some(true)
}

/// Kimi's models, in the order its session lists them, its current one included. Thinking isn't
/// offered: Kimi keeps it in its own settings, and no run has checked its session option.
fn catalogue(metadata: &super::super::acp::ModelMetadata) -> Vec<models::AgentModel> {
    let mut ordered: Vec<String> = Vec::new();
    for id in metadata.models.iter().chain(std::iter::once(&metadata.current_model)) {
        if !id.is_empty() && !ordered.contains(id) {
            ordered.push(id.clone());
        }
    }
    ordered.into_iter().map(|id| models::AgentModel { name: id.clone(), thinking: Vec::new(), id }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kimi_code_is_acp_only_and_registered() {
        let registry = AgentAdapters::built_in();
        let adapter = registry.for_agent("kimi-code");
        assert!(adapter.ids().contains(&"kimi-code"), "registered, not the plain adapter");
        assert!(registry.for_agent("kimi").ids().contains(&"kimi-code"), "the program's name finds it too");
        assert!(adapter.requires_acp() && !adapter.plain_fallback() && adapter.answers_questions());
        assert!(adapter.has_model_catalog() && !adapter.supports_model_tuning());
        assert!(!adapter.acp_mcp_stdio_type(), "the server goes as ACP's plain stdio entry");
        assert!(adapter.acp_trailing().is_empty());
        assert_eq!(super::super::acp::acp_args("kimi-code"), Some(&["acp"][..]));
        // The model goes by session option, never as a flag before `acp`.
        assert!(super::super::acp::model_args("kimi-code", Some("kimi-code/k3")).is_empty());
        assert!(super::super::acp::key_auth_method("kimi-code").is_none(), "its own sign-in, never a stored key");
        assert_eq!(super::super::program_for("kimi-code"), "kimi");
        assert_eq!(super::super::display_for("kimi-code"), "Kimi Code CLI");
    }

    #[test]
    fn nothing_before_acp_and_the_model_goes_by_session_option() {
        let defaults = BackendOptions::default();
        assert!(KimiCode.acp_options(&defaults, Access::Build).is_empty());
        assert!(KimiCode.acp_mode_options(&defaults, Access::Build).is_empty());
        assert!(KimiCode.acp_session_config(&defaults).is_empty(), "no model: Kimi's own default_model");
        assert_eq!(KimiCode.model_in_use(None).as_deref(), Some("Kimi Code default"));
        let options = BackendOptions { cli_model: Some("  kimi-code/kimi-for-coding  ".into()), args: vec!["--yolo".into()], ..Default::default() };
        // Saved extra arguments are refused and never copied in front of `acp`.
        let refused = KimiCode.validate_options(&options).expect_err("extra arguments are refused");
        assert!(refused.message.contains("approval"), "{}", refused.message);
        assert!(KimiCode.acp_options(&options, Access::Build).is_empty());
        assert_eq!(KimiCode.acp_session_config(&options), [("model".to_string(), "kimi-code/kimi-for-coding".to_string())]);
        assert_eq!(KimiCode.model_in_use(Some(&options)).as_deref(), Some("kimi-code/kimi-for-coding"));
        let blank = BackendOptions { cli_model: Some("  ".into()), ..Default::default() };
        assert!(KimiCode.acp_session_config(&blank).is_empty());
        assert!(KimiCode.validate_options(&blank).is_ok());
    }

    #[test]
    fn only_a_deep_build_is_allowed() {
        assert!(KimiCode.validate_access(Access::Build).is_ok());
        for access in [Access::Analyse, Access::CleanRoomAnalyse, Access::CleanRoomBuild, Access::CleanRoomQuestion, Access::Question] {
            let error = KimiCode.validate_access(access).expect_err("only a deep build");
            assert!(error.message.contains("Clean room") && error.message.contains("deep chat") && error.message.contains("question"), "{}", error.message);
            assert!(error.remediation.as_deref().is_some_and(|fix| fix.contains("deep build")));
        }
        let fix = KimiCode.sign_in_fix().expect("a sign-in fix");
        assert!(fix.contains("kimi login") && fix.contains("never sees"), "{fix}");
    }

    #[test]
    fn the_credentials_folder_follows_kimi_code_home() {
        let home = Path::new("/home/u");
        assert_eq!(credentials_dir(None, Some(home)), Some(PathBuf::from("/home/u/.kimi-code/credentials")));
        assert_eq!(credentials_dir(Some(OsStr::new("")), Some(home)), Some(PathBuf::from("/home/u/.kimi-code/credentials")));
        assert_eq!(credentials_dir(Some(OsStr::new("/x/kimi")), Some(home)), Some(PathBuf::from("/x/kimi/credentials")));
        assert_eq!(credentials_dir(Some(OsStr::new("/x/kimi")), None), Some(PathBuf::from("/x/kimi/credentials")));
        assert_eq!(credentials_dir(None, None), None);
    }

    #[test]
    fn a_saved_credential_counts_as_signed_in_and_anything_else_is_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let creds = dir.path().join("credentials");
        assert_eq!(credentials_present(Some(creds.clone())), None, "no folder: unknown, not signed out");
        std::fs::create_dir_all(creds.join("mcp")).unwrap();
        assert_eq!(credentials_present(Some(creds.clone())), None, "an empty folder (MCP sign-ins only) is unknown");
        std::fs::write(creds.join("notes.txt"), "x").unwrap();
        std::fs::create_dir(creds.join("folder.json")).unwrap();
        assert_eq!(credentials_present(Some(creds.clone())), None, "a folder named .json or another file isn't a credential");
        std::fs::write(creds.join("kimi-code.json"), "{}").unwrap();
        assert_eq!(credentials_present(Some(creds)), Some(true));
        assert_eq!(credentials_present(None), None);
    }

    fn kimi_config() -> BackendConfig {
        BackendConfig { name: "Kimi Code".into(), kind: crate::BackendKind::AgentCli, model: "kimi-code".into(), base_url: None, ssh_host: None, enabled: true }
    }

    /// A deep build of the Kimi Code backend, run with `program`.
    fn deep_build(program: &Path, cli_model: Option<&str>, inbox: &Path) -> CliSpec {
        let options = BackendOptions { command: Some(program.display().to_string()), cli_model: cli_model.map(str::to_string), ..Default::default() };
        let mut spec = CliSpec::new(kimi_config(), options, false, None);
        spec.deep = true;
        spec.project = inbox.parent().map(Path::to_path_buf);
        spec.builder = Some(super::super::super::Builder { investigation: None, inbox: inbox.to_path_buf() });
        spec
    }

    fn finish(c: &mut dyn Conversation) -> Vec<AgentEvent> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut all = Vec::new();
        while !all.iter().any(|e| matches!(e, AgentEvent::TurnDone { .. })) {
            assert!(std::time::Instant::now() < deadline, "no TurnDone: {all:?}");
            all.extend(c.poll());
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        all
    }

    /// A stand-in `kimi` answers ACP the way the published `kimi acp` does: a terminal `login`
    /// method in `initialize` (which the host never runs), a session from `session/new`, the model
    /// by `session/set_config_option`, then the turn. It records its arguments and every request.
    #[cfg(unix)]
    fn stand_in(dir: &Path, new_session_reply: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let program = dir.join("kimi");
        let script = format!(
            r#"#!/bin/sh
d=$(dirname "$0")
printf '%s\n' "$@" > "$d/argv"
IFS= read -r line; printf '%s\n' "$line" >> "$d/requests"
echo '{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":1,"agentCapabilities":{{"loadSession":true}},"authMethods":[{{"id":"login","type":"terminal","name":"Login with Kimi account","args":["--login"]}}]}}}}'
IFS= read -r line; printf '%s\n' "$line" >> "$d/requests"
echo '{new_session_reply}'
IFS= read -r line; printf '%s\n' "$line" >> "$d/requests"
echo '{{"jsonrpc":"2.0","id":3,"result":{{"configOptions":[{{"id":"model","currentValue":"kimi-code/kimi-for-coding"}}]}}}}'
IFS= read -r line; printf '%s\n' "$line" >> "$d/requests"
echo '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"k-1","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"Built it."}}}}}}}}'
echo '{{"jsonrpc":"2.0","id":4,"result":{{"stopReason":"end_turn"}}}}'
"#
        );
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        program
    }

    /// A deep build goes over ACP: `kimi acp` and nothing else in argv, the whole message (longer
    /// than Windows' command line allows) in `session/prompt`, our server in `session/new`, and
    /// the chosen model set on the session before the prompt.
    #[cfg(unix)]
    #[test]
    fn a_deep_build_runs_kimi_acp_with_our_server_and_the_message_in_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = dir.path().join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let program = stand_in(dir.path(), r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"k-1"}}"#);
        let spec = deep_build(&program, Some("kimi-code/kimi-for-coding"), &inbox);
        let mut c = super::super::super::acp_for(&spec).expect("Kimi Code goes over ACP by default");
        let long = format!("Build the page. {}", "x".repeat(40_000));
        c.send(&long);
        let events = finish(&mut c);
        assert!(!events.iter().any(|e| matches!(e, AgentEvent::Error { .. })), "{events:?}");
        assert!(events.iter().any(|e| matches!(e, AgentEvent::TextDelta { text } if text == "Built it.")), "{events:?}");

        assert_eq!(std::fs::read_to_string(dir.path().join("argv")).unwrap(), "acp\n", "the message never goes in argv");
        let requests: Vec<Value> = std::fs::read_to_string(dir.path().join("requests")).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let methods: Vec<&str> = requests.iter().map(|r| r["method"].as_str().unwrap_or_default()).collect();
        assert_eq!(methods, ["initialize", "session/new", "session/set_config_option", "session/prompt"], "no `authenticate`: sign-in stays in `kimi login`");
        let server = &requests[1]["params"]["mcpServers"][0];
        assert_eq!(server["name"], MCP_NAME);
        assert!(server.get("type").is_none(), "{server}");
        assert_eq!(requests[2]["params"], json!({"sessionId": "k-1", "configId": "model", "value": "kimi-code/kimi-for-coding"}));
        let text = requests[3]["params"]["prompt"][0]["text"].as_str().unwrap();
        assert!(text.ends_with(&long), "the whole message reaches the session");
    }

    /// `kimi acp` refuses `session/new` with ACP's `authRequired` when nobody signed in: the turn
    /// fails, says so, and names `kimi login`.
    #[cfg(unix)]
    #[test]
    fn a_session_refused_for_sign_in_fails_the_turn_with_kimi_login() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = dir.path().join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let program = stand_in(dir.path(), r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"Authentication required"}}"#);
        let mut c = super::super::super::acp_for(&deep_build(&program, None, &inbox)).expect("ACP");
        c.send("Build the page.");
        let events = finish(&mut c);
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Error { remediation: Some(fix), .. } if fix.contains("kimi login"))), "{events:?}");
        assert_eq!(c.last_failure().map(|f| f.kind), Some(crate::FailureKind::Authentication));
    }

    /// No `kimi` to start: the turn ends with an error rather than a hang.
    #[test]
    fn a_missing_kimi_is_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = dir.path().join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let spec = deep_build(&dir.path().join("no-such-kimi-xyz"), None, &inbox);
        let mut c = super::super::super::acp_for(&spec).expect("ACP");
        c.send("Build the page.");
        let events = finish(&mut c);
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Error { .. })), "{events:?}");
    }

    /// Off ACP (`acp = false`) or over SSH there is no approval channel, so the run refuses
    /// before anything starts, and a chat is refused before its process starts too.
    #[test]
    fn without_acp_or_outside_a_deep_build_nothing_starts() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("kimi-never-started");
        let mut off = deep_build(&program, None, dir.path());
        off.options.acp = Some(false);
        assert!(super::super::super::acp_for(&off).is_none());
        let mut c = CliConversation::new(off);
        c.send("Build the page.");
        let events = finish(&mut c);
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Error { message, .. } if message.contains("requires a local ACP connection"))), "{events:?}");

        let mut chat = CliSpec::new(kimi_config(), BackendOptions { command: Some(program.display().to_string()), ..Default::default() }, false, None);
        chat.deep = true;
        let mut c = super::super::super::acp_for(&chat).expect("ACP");
        c.send("What does it do?");
        let events = finish(&mut c);
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Error { message, .. } if message.contains("deep chat"))), "{events:?}");
        assert!(!program.exists());
    }

    #[test]
    fn the_catalogue_keeps_kimis_order_and_its_current_model_once() {
        let metadata = super::super::super::acp::ModelMetadata {
            models: vec!["kimi-code/k3".into(), "kimi-code/kimi-for-coding".into(), "".into(), "kimi-code/k3".into()],
            current_model: "kimi-code/kimi-for-coding-highspeed".into(),
            config_options: Vec::new(),
        };
        let ids: Vec<String> = catalogue(&metadata).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["kimi-code/k3", "kimi-code/kimi-for-coding", "kimi-code/kimi-for-coding-highspeed"]);
        assert!(catalogue(&metadata).iter().all(|m| m.thinking.is_empty()), "thinking isn't offered");
    }
}
