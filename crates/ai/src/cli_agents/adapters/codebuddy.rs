//! CodeBuddy CLI (`codebuddy --acp`), driven over ACP with our MCP tools. Sign-in stays in the
//! CLI or in `CODEBUDDY_API_KEY`; Craft Anything never reads either.

use super::*;

/// CodeBuddy's ACP-only adapter. `acp = false` and an SSH host fail closed: the documented way to
/// drive CodeBuddy from another program is `--acp`, which is a local session.
pub struct CodeBuddy;

impl AgentAdapter for CodeBuddy {
    fn ids(&self) -> &'static [&'static str] {
        &["codebuddy"]
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

    /// The context-window tiers CodeBuddy offers depend on the chosen model.
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

    fn validate_model(&self, config: &BackendConfig, options: &BackendOptions, directory: Option<&Path>, stop: &Stop) -> Result<(), ToolError> {
        stop.check()?;
        self.validate_options(options)?;
        if options.cli_model.is_none() && options.cli_thinking.is_none() {
            return Ok(());
        }
        models::validate(options, &self.model_catalog(config, options, directory, stop)?)
    }

    fn plain_fallback(&self) -> bool {
        false
    }

    fn answers_questions(&self) -> bool {
        true
    }

    /// No override means the model CodeBuddy itself is set to, not a guessed one.
    fn model_in_use(&self, options: Option<&BackendOptions>) -> Option<String> {
        Some(
            options
                .and_then(|options| options.cli_model.as_deref().map(str::trim).filter(|model| !model.is_empty()).map(str::to_string))
                .unwrap_or_else(|| "CodeBuddy default".into()),
        )
    }

    fn model_choices(&self, config: &BackendConfig, options: &BackendOptions) -> Result<Vec<String>, ToolError> {
        Ok(self.model_catalog(config, options, None, &Stop::default())?.into_iter().map(|model| model.id).collect())
    }

    /// Reasoning is the shared thinking picker. CodeBuddy's context window is an ACP config
    /// option whose values (200K, 1M) this adapter does not pass, so the control stays off.
    fn supports_model_tuning(&self) -> bool {
        false
    }

    fn check_tool_server(&self, stop: &CancelToken) -> Result<(), ToolError> {
        cli_can_start(craft_anything_platform::self_cli_path().as_deref(), stop)
    }

    /// `--acp` is the whole command line. An extra argument could turn off the CLI's own
    /// permission asks, which is the only channel our approvals have here.
    fn acp_options(&self, _options: &BackendOptions, _access: Access) -> Vec<String> {
        Vec::new()
    }

    fn validate_options(&self, options: &BackendOptions) -> Result<(), ToolError> {
        if options.args.is_empty() {
            Ok(())
        } else {
            Err(ToolError::invalid("Extra CodeBuddy arguments could bypass Craft Anything's approval checks.")
                .with_remediation("Remove this backend's extra arguments. Set its program and model through the backend options instead."))
        }
    }

    /// A builder asks before every tool call: `default` sends each one to
    /// `session/request_permission`, and our policy answers that call only (`allow_once`). A chat
    /// and a question run in `plan`, where the CLI refuses every write and command itself. Both
    /// were checked against CodeBuddy 2.182.0.
    fn acp_mode_options(&self, _options: &BackendOptions, access: Access) -> Vec<String> {
        let mode = match access {
            Access::Build => "default",
            // Refused by `validate_access`; the most restrictive mode is still the safe answer.
            Access::Analyse | Access::Question | Access::CleanRoomAnalyse | Access::CleanRoomBuild | Access::CleanRoomQuestion => "plan",
        };
        vec!["--permission-mode".into(), mode.into()]
    }

    /// The adapter declares no file or terminal capability, so CodeBuddy reads files itself in
    /// every mode. `plan` stops it writing or running commands, which is what a chat and a
    /// question need; it does not stop a read, so Clean room cannot keep the original out of reach.
    fn validate_access(&self, access: Access) -> Result<(), ToolError> {
        match access {
            Access::Build | Access::Analyse | Access::Question => Ok(()),
            Access::CleanRoomAnalyse | Access::CleanRoomBuild | Access::CleanRoomQuestion => {
                Err(ToolError::invalid("CodeBuddy reads files on its own in every permission mode, so a Clean room run can't keep the original out of reach.")
                    .with_remediation(
                        "Use a deep build, a deep chat or a question. CodeBuddy's plan mode stops every write and command, but not its own reads.",
                    ))
            }
        }
    }

    /// The model and thinking depth are session config options (`model` and `thought_level`);
    /// `--acp` takes neither. The model goes first, because switching it can reset the depth.
    fn acp_session_config(&self, options: &BackendOptions) -> Vec<(String, String)> {
        let mut config = Vec::new();
        if let Some(model) = options.cli_model.as_deref().map(str::trim).filter(|model| !model.is_empty()) {
            config.push(("model".to_string(), model.to_string()));
        }
        let effort = options
            .cli_thinking
            .map(|effort| effort.as_str())
            .or_else(|| options.cli_reasoning_effort.map(|effort| effort.as_str()))
            .or_else(|| options.effort.map(|effort| effort.as_str()));
        if let Some(effort) = effort {
            config.push(("thought_level".to_string(), effort.to_string()));
        }
        config
    }

    /// Where to sign in, without guessing at a login subcommand: the CLI has no `login` command,
    /// and says `/login` (a slash command in its own session). A key in the environment works too
    /// (the China build also needs `CODEBUDDY_INTERNET_ENVIRONMENT=internal`).
    fn sign_in_fix(&self) -> Option<String> {
        Some(
            "Sign in with `/login` in a CodeBuddy CLI session in a terminal, or set CODEBUDDY_API_KEY for the computer Craft Anything runs on (with CODEBUDDY_INTERNET_ENVIRONMENT=internal for the China build), then try again. Craft Anything never sees the sign-in."
                .into(),
        )
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codebuddy_is_acp_only_and_registered() {
        let adapter = AgentAdapters::built_in().for_agent("codebuddy");
        assert!(adapter.ids().contains(&"codebuddy"), "registered, not the plain adapter");
        assert!(adapter.requires_acp() && !adapter.plain_fallback());
        assert!(adapter.has_model_catalog() && adapter.model_catalog_depends_on_selection());
        assert!(!adapter.supports_model_tuning());
        assert_eq!(super::super::acp::acp_args("codebuddy"), Some(&["--acp"][..]));
        assert!(super::super::acp::model_args("codebuddy", Some("cb-model")).is_empty(), "--acp takes no model flag");
    }

    #[test]
    fn the_session_carries_the_chosen_model_and_depth_in_that_order() {
        assert!(CodeBuddy.acp_session_config(&BackendOptions::default()).is_empty());
        // A blank saved choice is no choice, not a model named "".
        let blank = BackendOptions { cli_model: Some("  ".into()), ..Default::default() };
        assert!(CodeBuddy.acp_session_config(&blank).is_empty());
        let options = BackendOptions { cli_model: Some("  hy3-x  ".into()), ..Default::default() };
        assert_eq!(CodeBuddy.acp_session_config(&options), [("model".into(), "hy3-x".into())]);
        let both = BackendOptions { cli_model: Some("hy3-x".into()), cli_thinking: Some(crate::Effort::Low), ..Default::default() };
        assert_eq!(CodeBuddy.acp_session_config(&both), [("model".into(), "hy3-x".into()), ("thought_level".into(), "low".into())]);
        assert!(CodeBuddy.acp_options(&both, Access::Build).is_empty(), "--acp is the whole command line");
        assert_eq!(CodeBuddy.acp_trailing(), &[] as &[&str]);
    }

    /// Every mode CodeBuddy offers turns prompts off by default; the process is held to one that
    /// asks (`default`) for a build, or cannot write at all (`plan`) for a chat and a question.
    #[test]
    fn each_access_mode_pins_a_permission_mode() {
        let options = BackendOptions::default();
        assert_eq!(CodeBuddy.acp_mode_options(&options, Access::Build), ["--permission-mode", "default"]);
        for access in [Access::Analyse, Access::Question] {
            assert_eq!(CodeBuddy.acp_mode_options(&options, access), ["--permission-mode", "plan"]);
        }
        // Refused before a run, yet the arguments are still the restrictive ones.
        for access in [Access::CleanRoomAnalyse, Access::CleanRoomBuild, Access::CleanRoomQuestion] {
            assert_eq!(CodeBuddy.acp_mode_options(&options, access), ["--permission-mode", "plan"]);
        }
    }

    #[test]
    fn extra_arguments_are_refused_before_they_can_approve_everything() {
        assert!(CodeBuddy.validate_options(&BackendOptions::default()).is_ok());
        let options = BackendOptions { args: vec!["--dangerously-skip-permissions".into()], ..Default::default() };
        let error = CodeBuddy.validate_options(&options).unwrap_err();
        assert!(error.message.contains("Extra CodeBuddy arguments"), "{}", error.message);
        assert!(error.remediation.unwrap_or_default().contains("Remove this backend's extra arguments"));
    }

    #[test]
    fn clean_room_is_refused_and_the_other_modes_run() {
        for access in [Access::Build, Access::Analyse, Access::Question] {
            assert!(CodeBuddy.validate_access(access).is_ok(), "{access:?}");
        }
        for access in [Access::CleanRoomAnalyse, Access::CleanRoomBuild, Access::CleanRoomQuestion] {
            let error = CodeBuddy.validate_access(access).expect_err("the CLI's own reads can't be held back");
            assert!(error.message.contains("Clean room") && error.message.contains("reads files on its own"), "{}", error.message);
            assert!(error.remediation.as_deref().is_some_and(|fix| fix.contains("plan mode")));
        }
    }

    #[test]
    fn the_default_model_is_the_clis_own() {
        assert_eq!(CodeBuddy.model_in_use(None).as_deref(), Some("CodeBuddy default"));
        let options = BackendOptions { cli_model: Some("cb-model".into()), ..Default::default() };
        assert_eq!(CodeBuddy.model_in_use(Some(&options)).as_deref(), Some("cb-model"));
        let fix = CodeBuddy.sign_in_fix().unwrap();
        assert!(fix.contains("/login") && fix.contains("CODEBUDDY_API_KEY") && fix.contains("CODEBUDDY_INTERNET_ENVIRONMENT") && fix.contains("never sees"));
    }

    #[test]
    fn the_catalogue_offers_each_model_once_with_the_selected_ones_thinking() {
        let metadata = super::super::acp::ModelMetadata {
            models: vec!["cb-fast".into(), "cb-pro".into(), "cb-fast".into()],
            current_model: "cb-pro".into(),
            config_options: vec![json!({"id": "thought_level", "options": [
                {"value": "low", "name": "Low"},
                {"value": "high", "name": "High"},
                {"value": "unsupported", "name": "Unknown"}
            ]})],
        };
        let mut options = BackendOptions { cli_model: Some("cb-pro".into()), ..Default::default() };
        let models = catalogue(&metadata, &options);
        assert_eq!(models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(), ["cb-fast", "cb-pro"]);
        assert!(models.iter().find(|model| model.id == "cb-fast").unwrap().thinking.is_empty());
        assert_eq!(models.iter().find(|model| model.id == "cb-pro").unwrap().thinking, [crate::Effort::Low, crate::Effort::High]);
        options.cli_thinking = Some(crate::Effort::High);
        assert!(models::validate(&options, &models).is_ok());
        options.cli_thinking = Some(crate::Effort::Max);
        assert!(models::validate(&options, &models).is_err());
    }
}
