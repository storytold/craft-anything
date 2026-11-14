//! Oh My Pi (`omp acp`).
//!
//! `omp` speaks ACP natively (`omp acp`) and lists all configured models via `omp models --json`.
//! Launch flags (`--model`, `--thinking`, `--approval-mode`, `--tools`/`--no-tools`) are passed
//! before the `acp` subcommand so `omp` forwards them into the ACP server session.

use super::*;
use crate::Effort;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

const MAX_OUTPUT: usize = 8 * 1024 * 1024;
const MAX_MODELS: usize = 10_000;

/// OMP's ACP-only adapter. `acp = false` or an SSH host fails closed: running plain `omp <prompt>`
/// without `-p` opens the interactive TUI with no approval channel back to Craft Anything.
pub struct Omp;

impl AgentAdapter for Omp {
    fn ids(&self) -> &'static [&'static str] {
        &["omp"]
    }

    fn args(&self, _cx: &ArgsCx) -> Vec<String> {
        // `requires_acp()` fails closed in the native runner before `args()` is reached.
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
        discover(config, options, directory, stop)
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

    fn model_in_use(&self, options: Option<&BackendOptions>) -> Option<String> {
        Some(
            options
                .and_then(|options| options.cli_model.as_deref().map(str::trim).filter(|model| !model.is_empty()).map(str::to_string))
                .unwrap_or_else(|| "OMP default".into()),
        )
    }

    fn model_choices(&self, config: &BackendConfig, options: &BackendOptions) -> Result<Vec<String>, ToolError> {
        Ok(self.model_catalog(config, options, None, &Stop::default())?.into_iter().map(|model| model.id).collect())
    }

    fn check_tool_server(&self, stop: &CancelToken) -> Result<(), ToolError> {
        cli_can_start(craft_anything_platform::self_cli_path().as_deref(), stop)
    }

    fn validate_options(&self, options: &BackendOptions) -> Result<(), ToolError> {
        if options.args.is_empty() {
            Ok(())
        } else {
            Err(ToolError::invalid("Extra OMP arguments could bypass Craft Anything's tool or approval checks.")
                .with_remediation("Remove this backend's extra arguments. Set its program, model and thinking through the backend options instead."))
        }
    }

    /// OMP v18.19.0 cannot disable ambient project custom-tool discovery (`.omp/tools/`, `.claude/tools/`)
    /// via `--no-tools`, and `always-ask-cached` / `always-ask` allow remembered grants and read-tier
    /// tools without asking the ACP client. Refuse Clean room modes rather than claiming isolation
    /// that OMP cannot guarantee.
    fn validate_access(&self, access: Access) -> Result<(), ToolError> {
        match access {
            Access::Build | Access::Analyse | Access::Question => Ok(()),
            Access::CleanRoomBuild | Access::CleanRoomAnalyse | Access::CleanRoomQuestion => Err(ToolError::invalid(
                "OMP reads files and discovers project tools without asking our Clean room checks, so a Clean room run can't hold it back.",
            )
            .with_remediation("Use Deep mode with OMP, or choose an agent that enforces Clean room boundaries.")),
        }
    }

    fn acp_options(&self, options: &BackendOptions, access: Access) -> Vec<String> {
        let mut args: Vec<String> = match access {
            Access::CleanRoomQuestion => ["--no-tools", "--approval-mode", "always-ask", "--no-session"].map(String::from).into(),
            Access::Question => ["--tools", "read,grep,glob", "--approval-mode", "always-ask", "--no-session"].map(String::from).into(),
            Access::CleanRoomAnalyse => ["--no-tools", "--approval-mode", "always-ask"].map(String::from).into(),
            Access::Analyse => ["--tools", "read,grep,glob,lsp", "--approval-mode", "always-ask"].map(String::from).into(),
            Access::CleanRoomBuild | Access::Build => ["--approval-mode", "always-ask"].map(String::from).into(),
        };
        if let Some(model) = options.cli_model.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
            args.extend(["--model".into(), model.to_string()]);
        }
        if let Some(effort) = chosen_effort(options) {
            args.extend(["--thinking".into(), effort]);
        }
        args
    }

    fn acp_session_config(&self, options: &BackendOptions) -> Vec<(String, String)> {
        options.cli_model.as_deref().map(str::trim).filter(|model| !model.is_empty()).map(|model| vec![("model".into(), model.to_string())]).unwrap_or_default()
    }

    fn sign_in_fix(&self) -> Option<String> {
        Some("Configure a provider or run `omp login` in a terminal, then try again. OMP manages its own provider keys and ~/.omp/agent/models.yml.".into())
    }
}

fn omp_effort_arg(effort: Effort) -> &'static str {
    match effort {
        Effort::None => "off",
        other => other.as_str(),
    }
}

fn parse_omp_effort(value: &str) -> Option<Effort> {
    if value.trim().eq_ignore_ascii_case("off") { Some(Effort::None) } else { Effort::parse(value) }
}

fn chosen_effort(options: &BackendOptions) -> Option<String> {
    options
        .cli_thinking
        .map(omp_effort_arg)
        .or_else(|| {
            options.cli_reasoning_effort.map(|effort| match effort {
                crate::config::CliReasoningEffort::None => "off",
                other => other.as_str(),
            })
        })
        .or_else(|| options.effort.map(omp_effort_arg))
        .map(str::to_string)
}
#[derive(Deserialize)]
struct OmpModelsResponse {
    models: Vec<OmpWireModel>,
}

#[derive(Deserialize)]
struct OmpWireModel {
    id: String,
    #[serde(default)]
    selector: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    thinking: Option<Vec<String>>,
}

fn malformed() -> ToolError {
    ToolError::new(ErrorCode::GarbledOutput, "OMP returned an unreadable model list.")
        .with_remediation("Check `omp models --json` in a terminal, then try loading OMP models again.")
}

pub(crate) fn parse_models(bytes: &[u8]) -> Result<Vec<models::AgentModel>, ToolError> {
    if bytes.len() >= MAX_OUTPUT {
        return Err(ToolError::new(ErrorCode::OverLimit, "OMP's model list is too large."));
    }
    let response: OmpModelsResponse = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if response.models.len() > MAX_MODELS {
        return Err(ToolError::new(ErrorCode::OverLimit, "OMP returned too many models."));
    }
    let mut out = BTreeMap::new();
    for m in response.models {
        if m.kind.as_deref().is_some_and(|k| k != "chat") {
            continue;
        }
        let id = match m.selector.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(sel) => sel.to_string(),
            None => match (m.provider.as_deref().map(str::trim).filter(|p| !p.is_empty()), m.id.trim()) {
                (Some(p), raw) if !raw.is_empty() => format!("{p}/{raw}"),
                (_, raw) if !raw.is_empty() => raw.to_string(),
                _ => return Err(malformed()),
            },
        };
        let name = m.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&id).to_string();
        let thinking = m.thinking.unwrap_or_default().into_iter().filter_map(|t| parse_omp_effort(&t)).collect();
        out.insert(id.clone(), models::AgentModel { id, name, thinking });
    }
    Ok(out.into_values().collect())
}

fn discover(config: &BackendConfig, options: &BackendOptions, directory: Option<&Path>, stop: &Stop) -> Result<Vec<models::AgentModel>, ToolError> {
    let program = super::program_of(config, options)?;
    let args = vec!["models".to_string(), "--json".to_string()];
    let mut process = match config.ssh_host.as_deref().filter(|h| !h.trim().is_empty()) {
        Some(host) => {
            let mut remote = vec![program];
            remote.extend(args);
            Process::new("ssh").args(crate::ssh::ssh_args(host, &remote))
        }
        None => {
            let mut p = Process::new(program).args(args);
            if let Some(dir) = directory {
                p = p.cwd(dir);
            }
            p
        }
    };
    let stop = stop.clone();
    process = process.timeout(Duration::from_secs(25)).idle_timeout(Duration::from_secs(20)).stdout_cap(MAX_OUTPUT).stderr_cap(64 * 1024);
    let out = process
        .cancel_when(move || stop.is_stopped())
        .remediation("Check that `omp` is installed and `omp models --json` succeeds in a terminal, then reload the models.")
        .run_checked()?;
    parse_models(&out.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omp_is_acp_only_and_registered() {
        let adapter = AgentAdapters::built_in().for_agent("omp");
        assert!(adapter.ids().contains(&"omp"));
        assert!(adapter.requires_acp() && !adapter.plain_fallback() && adapter.answers_questions());
        assert!(adapter.has_model_catalog());
        assert_eq!(super::super::acp::acp_args("omp"), Some(&["acp"][..]));
        assert_eq!(program_for("omp"), "omp");
        assert_eq!(display_for("omp"), "OMP");
    }

    #[test]
    fn parses_omp_models_json_chat_only_with_thinking() {
        let json = br#"{
            "models": [
                {"provider":"openrouter","kind":"chat","id":"deepseek/deepseek-chat","selector":"openrouter/deepseek/deepseek-chat","name":"DeepSeek Chat","thinking":["off","low","high","max"]},
                {"provider":"anthropic","kind":"chat","id":"claude-sonnet-4-6","selector":"anthropic/claude-sonnet-4-6","name":"Claude Sonnet 4.6","thinking":null},
                {"provider":"openai","kind":"embedding","id":"text-embedding-3-small","selector":"openai/text-embedding-3-small","name":"Embed"}
            ]
        }"#;
        let models = parse_models(json).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "anthropic/claude-sonnet-4-6");
        assert!(models[0].thinking.is_empty());
        assert_eq!(models[1].id, "openrouter/deepseek/deepseek-chat");
        assert_eq!(models[1].name, "DeepSeek Chat");
        assert_eq!(models[1].thinking, [Effort::None, Effort::Low, Effort::High, Effort::Max]);
    }

    #[test]
    fn access_modes_restrict_tools_approvals_and_clean_room() {
        for access in [Access::CleanRoomBuild, Access::CleanRoomAnalyse, Access::CleanRoomQuestion] {
            let err = Omp.validate_access(access).expect_err("clean room must be refused for omp");
            assert!(err.remediation.as_deref().is_some_and(|r| r.contains("Deep mode")));
        }
        for access in [Access::Build, Access::Analyse, Access::Question] {
            assert!(Omp.validate_access(access).is_ok());
        }

        let opts = BackendOptions { cli_model: Some("openrouter/deepseek/deepseek-chat".into()), cli_thinking: Some(Effort::High), ..Default::default() };
        let deep_q = Omp.acp_options(&opts, Access::Question);
        assert!(deep_q.windows(2).any(|w| w == ["--tools", "read,grep,glob"]));
        assert!(deep_q.windows(2).any(|w| w == ["--approval-mode", "always-ask"]));
        assert!(deep_q.contains(&"--no-session".to_string()));
        assert!(deep_q.windows(2).any(|w| w == ["--model", "openrouter/deepseek/deepseek-chat"]));
        assert!(deep_q.windows(2).any(|w| w == ["--thinking", "high"]));

        let deep_a = Omp.acp_options(&opts, Access::Analyse);
        assert!(deep_a.windows(2).any(|w| w == ["--tools", "read,grep,glob,lsp"]));
        assert!(deep_a.windows(2).any(|w| w == ["--approval-mode", "always-ask"]));

        let deep_b = Omp.acp_options(&opts, Access::Build);
        assert!(deep_b.windows(2).any(|w| w == ["--approval-mode", "always-ask"]));
        assert!(!deep_b.iter().any(|a| a == "always-ask-cached"));

        let off_opts = BackendOptions { cli_thinking: Some(Effort::None), ..Default::default() };
        assert!(Omp.acp_options(&off_opts, Access::Build).windows(2).any(|w| w == ["--thinking", "off"]));
        let off_reason = BackendOptions { cli_reasoning_effort: Some(crate::config::CliReasoningEffort::None), ..Default::default() };
        assert!(Omp.acp_options(&off_reason, Access::Build).windows(2).any(|w| w == ["--thinking", "off"]));
    }

    #[test]
    fn refuses_extra_args_that_could_bypass_approvals() {
        let blocked = BackendOptions { args: vec!["--yolo".into()], ..Default::default() };
        assert!(Omp.validate_options(&blocked).is_err());
        assert!(Omp.validate_options(&BackendOptions::default()).is_ok());
    }
}
