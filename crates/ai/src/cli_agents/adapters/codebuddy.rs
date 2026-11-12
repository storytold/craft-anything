//! CodeBuddy Code (`codebuddy --acp`), as one more ACP agent.
//!
//! Sign-in, sessions, models and credentials stay in CodeBuddy Code; this drives its Agent Client
//! Protocol mode, which Craft Anything speaks for every agent ([`super::super::acp`]). A session
//! takes our MCP server in `session/new`, and its changes come to us as permission requests, so a
//! build's approvals stay the app's policy. Its own file, shell and web tools do **not** ask —
//! checked live against 2.183.0, where a `Read` ran without one — so only a deep build is allowed:
//! a Clean room run, a deep chat or a question couldn't hold it back.
//!
//! Two of its tools can't be refused at all: `ListMcpResources` and `ReadMcpResource` read another
//! server's resources whatever the tool list says. A question is therefore handed no server.
//!
//! Its headless `stream-json` (`-p`) is the other way to reach it; it has no prompt to ask with
//! there, so a build has to run with its approvals off, and this is what the app uses instead.

use super::*;

/// The agent id and the names it is known by.
pub const IDS: &[&str] = &["codebuddy", "code-buddy", "cbc"];

/// CodeBuddy Code over ACP.
pub struct CodeBuddy;

impl AgentAdapter for CodeBuddy {
    fn ids(&self) -> &'static [&'static str] {
        IDS
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

    fn model_catalog_depends_on_selection(&self) -> bool {
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
        Ok(catalogue(&metadata, options))
    }

    fn model_in_use(&self, options: Option<&BackendOptions>) -> Option<String> {
        // No override means the CLI's own choice, not an implicit request for a model.
        Some(options.and_then(|options| options.cli_model.clone()).unwrap_or_else(|| "CodeBuddy Code default".into()))
    }

    fn model_choices(&self, config: &BackendConfig, options: &BackendOptions) -> Result<Vec<String>, ToolError> {
        super::super::acp_model_choices(config, options)
    }

    fn plain_fallback(&self) -> bool {
        false
    }

    fn check_tool_server(&self, stop: &CancelToken) -> Result<(), ToolError> {
        cli_can_start(craft_anything_platform::self_cli_path().as_deref(), stop)
    }

    /// A build asks us before it changes anything. Everything else couldn't be held back: its own
    /// reading, commands and fetching never ask (checked live, 2.183.0), and a question can't
    /// refuse its resource readers.
    fn validate_access(&self, access: Access) -> Result<(), ToolError> {
        match access {
            Access::Build => Ok(()),
            Access::Analyse | Access::CleanRoomAnalyse | Access::CleanRoomBuild | Access::CleanRoomQuestion | Access::Question => Err(ToolError::invalid(
                "CodeBuddy Code reads and runs commands without asking, so a Clean room run, a deep chat, or a question can't hold it back.",
            )
            .with_remediation("Use a deep build. CodeBuddy asks us before an edit, but not before a read or a command.")),
        }
    }

    fn acp_session_config(&self, options: &BackendOptions) -> Vec<(String, String)> {
        // A build is asked about through ours; its own tool hooks stay out of it.
        let mut config = vec![("mode".to_string(), "default".to_string())];
        config.extend(options.cli_model.as_deref().filter(|m| !m.trim().is_empty()).map(|m| ("model".to_string(), m.to_string())));
        config
    }

    /// Whether it still names an account it signed in to (see [`names_account`]).
    fn signed_in_before(&self) -> bool {
        config_dir().is_some_and(|dir| names_account(&dir))
    }

    fn sign_in_fix(&self) -> Option<String> {
        Some("Run the selected CodeBuddy program with `login` in a terminal, finish signing in, then test it again in Settings → AI.".into())
    }
}

/// Its models, at the depth the chosen one offers (the live list names its effort options).
fn catalogue(metadata: &super::super::acp::ModelMetadata, options: &BackendOptions) -> Vec<models::AgentModel> {
    let tuning = crate::model_options::ModelTuning::from_metadata(metadata.current_model.clone(), &metadata.config_options);
    let active = options.cli_model.as_deref().map(str::trim).filter(|model| !model.is_empty()).unwrap_or(metadata.current_model.as_str());
    let thinking: Vec<_> = tuning.reasoning.iter().filter_map(|choice| crate::Effort::parse(choice.value.as_str())).collect();
    let mut ids = metadata.models.clone();
    if !metadata.current_model.is_empty() {
        ids.push(metadata.current_model.clone());
    }
    let mut ordered = Vec::new();
    for id in ids {
        if !id.is_empty() && !ordered.contains(&id) {
            ordered.push(id);
        }
    }
    ordered.into_iter().map(|id| models::AgentModel { name: id.clone(), thinking: if id == active { thinking.clone() } else { Vec::new() }, id }).collect()
}

/// Its folder: `CODEBUDDY_CONFIG_DIR` when it's set (the person's choice), else `~/.codebuddy`.
fn config_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("CODEBUDDY_CONFIG_DIR").filter(|v| !v.is_empty()).map(std::path::PathBuf::from).or_else(|| dirs::home_dir().map(|h| h.join(".codebuddy")))
}

/// Whether `folder` names the account it was signed in to (`oauthAccount`, as the JRPC wrapper
/// writes it, or `account`): its settings and local storage say so, and its session files.
fn names_account(folder: &std::path::Path) -> bool {
    let named = |text: &str| {
        serde_json::from_str::<Value>(text).is_ok_and(|v| {
            v.get("oauthAccount").is_some_and(Value::is_object)
                || v.get("account").is_some_and(|a| a.is_object() || a.as_str().is_some_and(|s| !s.trim().is_empty()))
        })
    };
    ["credentials.json", "auth.json", "config.json", "settings.json"].iter().any(|f| read_small(&folder.join(f)).is_some_and(|t| named(&t)))
        || std::fs::read_dir(folder.join("sessions")).into_iter().flatten().take(20).flatten().any(|e| read_small(&e.path()).is_some_and(|t| named(&t)))
}

/// A file's text, up to 2 MB; anything bigger (or unreadable) says nothing.
fn read_small(file: &std::path::Path) -> Option<String> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(file).ok()?.take(2 << 20).read_to_string(&mut text).ok()?;
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_runs_over_acp_and_no_other_way() {
        let adapters = AgentAdapters::built_in();
        for id in IDS {
            let adapter = adapters.for_agent(id);
            assert!(adapter.ids().contains(&"codebuddy"), "{id} must reach its adapter");
            assert!(adapter.requires_acp(), "{id} is only driven over ACP");
        }
        assert!(!CodeBuddy.plain_fallback());
        // No question: with no server it would read files with its own tools, which never ask.
        assert!(CodeBuddy.has_model_catalog() && !CodeBuddy.answers_questions());
    }

    #[test]
    fn only_a_deep_build_is_allowed() {
        assert!(CodeBuddy.validate_access(Access::Build).is_ok());
        for access in [Access::Analyse, Access::CleanRoomAnalyse, Access::CleanRoomBuild, Access::CleanRoomQuestion, Access::Question] {
            let error = CodeBuddy.validate_access(access).expect_err("refused");
            assert!(error.message.contains("without asking"), "{access:?}: {}", error.message);
        }
    }

    #[test]
    fn a_build_is_asked_through_our_policy_at_the_chosen_model() {
        let options = BackendOptions { cli_model: Some("claude-sonnet-5".into()), ..Default::default() };
        assert_eq!(CodeBuddy.acp_session_config(&options), [("mode".into(), "default".into()), ("model".into(), "claude-sonnet-5".into())]);
        assert_eq!(CodeBuddy.acp_session_config(&BackendOptions::default()), [("mode".into(), "default".into())]);
    }

    #[test]
    fn an_account_named_in_its_folder_counts_as_signed_in() {
        let dir = std::env::temp_dir().join(format!("ca-cb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a folder");
        assert!(!names_account(&dir), "nothing there names an account");
        std::fs::write(dir.join("settings.json"), json!({"account": {"email": "a@b"}}).to_string()).expect("write");
        assert!(names_account(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
