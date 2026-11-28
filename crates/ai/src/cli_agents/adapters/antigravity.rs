//! Antigravity (`agy`). Flags are checked against `agy --help`.
//!
//! Antigravity runs prompts non-interactively with `--output-format stream-json -p <prompt>`.
//! In builder mode, `--dangerously-skip-permissions` is passed so tool requests proceed without prompting.

use super::*;

const MAX_MODELS_OUTPUT: usize = 1024 * 1024;
const MAX_MODELS: usize = 1_000;

pub struct Antigravity;

impl AgentAdapter for Antigravity {
    fn ids(&self) -> &'static [&'static str] {
        &["antigravity", "agy"]
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
        stop.check()?;
        let program = super::super::program_of(config, options)?;
        let (cmd, args) = match config.ssh_host.as_deref().filter(|h| !h.trim().is_empty()) {
            Some(host) => ("ssh".into(), crate::ssh::ssh_args(host, &[program, "models".into()])),
            None => (program, vec!["models".into()]),
        };
        let mut process = Process::new(&cmd).args(&args);
        if let Some(dir) = directory.filter(|_| config.ssh_host.as_deref().is_none_or(|h| h.trim().is_empty())) {
            process = process.cwd(dir);
        }
        let stop = stop.clone();
        let out = process
            .timeout(Duration::from_secs(20))
            .idle_timeout(Duration::from_secs(15))
            .stdout_cap(MAX_MODELS_OUTPUT)
            .stderr_cap(64 * 1024)
            .cancel_when(move || stop.is_stopped())
            .remediation("Check that Antigravity (`agy`) is installed and signed in, then reload the models.")
            .run()?;
        if !out.success {
            let name = Path::new(&cmd).file_name().map_or_else(|| cmd.clone(), |n| n.to_string_lossy().into_owned());
            return Err(craft_anything_platform::process::nonzero_error_with(
                &name,
                &out,
                Some(&craft_anything_platform::process::printable_command(OsStr::new(&cmd), &args)),
            ));
        }
        parse_models(&out.stdout)
    }

    fn validate_model(&self, config: &BackendConfig, options: &BackendOptions, directory: Option<&Path>, stop: &Stop) -> Result<(), ToolError> {
        if options.cli_model.is_none() && options.cli_thinking.is_none() {
            return Ok(());
        }
        if options.args.iter().any(|a| a == "--model" || a.starts_with("--model=")) {
            return Err(ToolError::invalid("Choose Antigravity's model in its model setting, rather than in extra arguments."));
        }
        models::validate(options, &self.model_catalog(config, options, directory, stop)?)
    }

    fn args(&self, cx: &ArgsCx) -> Vec<String> {
        let mut a = cx.options.args.clone();
        if cx.builder {
            a.push("--dangerously-skip-permissions".into());
        }
        if let Some(m) = &cx.options.cli_model {
            a.extend(["--model".into(), m.clone()]);
        }
        if let Some(r) = cx.resume {
            a.extend(["--conversation".into(), r.to_string()]);
        }
        a.extend(["--output-format".into(), "stream-json".into(), "-p".into(), format!("{}{}", cx.preamble, cx.prompt)]);
        a
    }

    fn required_flags(&self) -> (&'static [&'static str], &'static [&'static str]) {
        (&["--help"], &["--output-format", "stream-json"])
    }

    fn streams_json(&self) -> bool {
        true
    }

    fn resumes(&self) -> bool {
        true
    }

    fn parse(&self, p: &mut StreamParser, v: &Value) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        let event = v.get("event").and_then(Value::as_str).unwrap_or_default();
        match event {
            "init" => {
                if let Some(id) = v.get("conversation_id").and_then(Value::as_str) {
                    p.session_id = Some(id.to_string());
                }
            }
            "step_update" => {
                let Some(su) = v.get("step_update") else { return out };
                if let Some(id) = su.get("conversation_id").and_then(Value::as_str) {
                    p.session_id.get_or_insert_with(|| id.to_string());
                }
                let step_type = su.get("step_type").and_then(Value::as_str).unwrap_or_default();
                let state = su.get("state").and_then(Value::as_str).unwrap_or_default();
                match step_type {
                    "agent_response" => {
                        if let Some(text) = su.get("text_delta").and_then(Value::as_str).filter(|t| !t.is_empty()) {
                            out.push(AgentEvent::TextDelta { text: text.to_string() });
                        }
                    }
                    "tool" => {
                        let id = su.get("step_index").map(|i| i.to_string()).unwrap_or_default();
                        let name = su.get("tool_name").or_else(|| su.pointer("/tool_info/name")).and_then(Value::as_str).unwrap_or_default();
                        if state == "ACTIVE" {
                            let input = su.pointer("/tool_info/parameters").cloned().unwrap_or(Value::Null);
                            out.push(p.started(&id, name, input));
                        } else if state == "DONE" || state == "ERROR" {
                            if !p.tool_names.contains_key(&id) {
                                let input = su.pointer("/tool_info/parameters").cloned().unwrap_or(Value::Null);
                                out.push(p.started(&id, name, input));
                            }
                            let ok = state != "ERROR" && su.pointer("/tool_info/error").is_none();
                            let text = su.pointer("/tool_info/output").or_else(|| su.pointer("/tool_info/error")).map(content_text).unwrap_or_default();
                            out.push(p.finished(&id, ok, &text));
                        }
                    }
                    _ => {}
                }
            }
            "result" => {
                p.saw_result = true;
                if let Some(res) = v.get("result") {
                    if let Some(u) = res.get("usage") {
                        let in_tok = u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
                        let cache_tok = u.get("cache_read_tokens").and_then(Value::as_u64).unwrap_or(0);
                        p.input_tokens = in_tok.saturating_add(cache_tok);
                        let out_tok = u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
                        let think_tok = u.get("thinking_tokens").and_then(Value::as_u64).unwrap_or(0);
                        p.output_tokens = out_tok.saturating_add(think_tok);
                    }
                    if res.get("status").and_then(Value::as_str) == Some("ERROR") {
                        let reply = res.get("response").and_then(Value::as_str).unwrap_or_default();
                        let why = res.get("error").and_then(Value::as_str).unwrap_or("run failed");
                        if reply.trim().is_empty() {
                            out.push(AgentEvent::Error { message: format!("Antigravity: {why}"), remediation: None, log: None });
                        } else {
                            // Antigravity keeps reporting a conversation's old error (its `error_id`
                            // names a step from an earlier turn) on every later result, even after a
                            // turn answers in full. A reply that came back ends the turn as done;
                            // the words stay visible without failing work that succeeded.
                            out.push(AgentEvent::Status {
                                text: format!("Antigravity replied, yet still reports an earlier error of the conversation: {why}"),
                            });
                        }
                    }
                }
            }
            "error" => {
                let msg = v.pointer("/error/message").or_else(|| v.get("error")).and_then(Value::as_str).unwrap_or("unknown error");
                out.push(AgentEvent::Error { message: format!("Antigravity: {msg}"), remediation: None, log: None });
            }
            _ => {}
        }
        out
    }
}

/// Parse `agy models` stdout (`<id>\t<display name>` when piped, or space-padded columns).
/// Preserves the CLI's ordering, deduplicates by id, and rejects empty or oversized output.
fn parse_models(bytes: &[u8]) -> Result<Vec<models::AgentModel>, ToolError> {
    if bytes.len() >= MAX_MODELS_OUTPUT {
        return Err(ToolError::new(ErrorCode::OverLimit, "Antigravity's model list is too large."));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ToolError::new(ErrorCode::GarbledOutput, "Antigravity returned an unreadable model list.")
            .with_remediation("Run `agy models` in a terminal to verify your Antigravity sign-in, then reload the models.")
    })?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let (id, name) = match line.split_once('\t') {
            Some((id, name)) => (id.trim(), name.trim()),
            None => match line.split_once(char::is_whitespace) {
                Some((id, rest)) => (id.trim(), rest.trim()),
                None => (line, line),
            },
        };
        if id.is_empty() || id.chars().any(|c| c.is_whitespace() || c.is_control()) {
            continue;
        }
        if !seen.insert(id.to_string()) {
            continue;
        }
        if out.len() >= MAX_MODELS {
            return Err(ToolError::new(ErrorCode::OverLimit, "Antigravity returned too many models."));
        }
        let display = if name.is_empty() { id } else { name };
        out.push(models::AgentModel { id: id.to_string(), name: display.to_string(), thinking: Vec::new() });
    }
    if out.is_empty() {
        return Err(ToolError::new(ErrorCode::GarbledOutput, "Antigravity returned an empty or unreadable model list.")
            .with_remediation("Run `agy models` in a terminal to verify your Antigravity sign-in, then reload the models."));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(builder: bool, resume: Option<&str>, options: &BackendOptions) -> Vec<String> {
        let mcp = McpCommand { command: "craft-anything-cli".into(), args: vec!["mcp".into()], notes: None };
        Antigravity.args(&ArgsCx {
            options,
            prompt: "analyse repo",
            preamble: "PRE ",
            mcp: &mcp,
            resume,
            builder,
            reads_checked: false,
            question: false,
            message_file: None,
        })
    }

    #[test]
    fn antigravity_args_construction() {
        let default_opts = BackendOptions::default();
        assert_eq!(args(false, None, &default_opts), ["--output-format", "stream-json", "-p", "PRE analyse repo"]);

        let builder_opts =
            BackendOptions { cli_model: Some("Gemini 3.8 Flash (Medium)".into()), args: vec!["--add-dir".into(), "/repo".into()], ..Default::default() };
        assert_eq!(
            args(true, Some("conv-123"), &builder_opts),
            [
                "--add-dir",
                "/repo",
                "--dangerously-skip-permissions",
                "--model",
                "Gemini 3.8 Flash (Medium)",
                "--conversation",
                "conv-123",
                "--output-format",
                "stream-json",
                "-p",
                "PRE analyse repo"
            ]
        );
        assert!(Antigravity.resumes());
        assert_eq!(Antigravity.required_flags(), (&["--help"][..], &["--output-format", "stream-json"][..]));
        assert!(AgentAdapters::built_in().for_agent("antigravity").ids().contains(&"antigravity"));
        assert!(AgentAdapters::built_in().for_agent("agy").ids().contains(&"agy"));
    }

    #[test]
    fn antigravity_stream_parsing() {
        let adapter: Arc<dyn AgentAdapter> = Arc::new(Antigravity);
        let mut p = StreamParser::new(adapter);

        let init = json!({
            "event": "init",
            "conversation_id": "conv-456",
            "init": {"cwd": "/tmp", "permission_mode": "always-proceed"}
        });
        assert!(p.line(&init.to_string()).is_empty());
        assert_eq!(p.session_id.as_deref(), Some("conv-456"));

        let text_step = json!({
            "event": "step_update",
            "step_update": {
                "conversation_id": "conv-456",
                "step_index": 1,
                "state": "ACTIVE",
                "step_type": "agent_response",
                "text_delta": "Hello from Antigravity!"
            }
        });
        let evs = p.line(&text_step.to_string());
        assert_eq!(evs.len(), 1);
        match &evs[0] {
            AgentEvent::TextDelta { text } => assert_eq!(text, "Hello from Antigravity!"),
            other => panic!("expected TextDelta, got {other:?}"),
        }

        let tool_active = json!({
            "event": "step_update",
            "step_update": {
                "step_index": 2,
                "state": "ACTIVE",
                "step_type": "tool",
                "tool_name": "run_command",
                "tool_info": {
                    "name": "run_command",
                    "parameters": {"CommandLine": "cargo check"}
                }
            }
        });
        let evs = p.line(&tool_active.to_string());
        assert_eq!(evs.len(), 1);
        match &evs[0] {
            AgentEvent::ToolStarted { call_id, tool, input } => {
                assert_eq!(call_id, "2");
                assert_eq!(tool, "run_command");
                assert_eq!(input["CommandLine"], "cargo check");
            }
            other => panic!("expected ToolStarted, got {other:?}"),
        }

        let tool_done = json!({
            "event": "step_update",
            "step_update": {
                "step_index": 2,
                "state": "DONE",
                "step_type": "tool",
                "tool_name": "run_command",
                "tool_info": {
                    "name": "run_command",
                    "parameters": {"CommandLine": "cargo check"},
                    "output": "Finished dev profile"
                }
            }
        });
        let evs = p.line(&tool_done.to_string());
        assert_eq!(evs.len(), 1);
        match &evs[0] {
            AgentEvent::ToolFinished { call_id, tool, ok, summary, .. } => {
                assert_eq!(call_id, "2");
                assert_eq!(tool, "run_command");
                assert!(ok);
                assert_eq!(summary, "Finished dev profile");
            }
            other => panic!("expected ToolFinished, got {other:?}"),
        }

        let result = json!({
            "event": "result",
            "result": {
                "conversation_id": "conv-456",
                "status": "SUCCESS",
                "response": "Done!",
                "usage": {
                    "input_tokens": 100,
                    "output_tokens": 50,
                    "thinking_tokens": 10,
                    "cache_read_tokens": 20,
                    "total_tokens": 180
                }
            }
        });
        let evs = p.line(&result.to_string());
        assert!(evs.is_empty());
        assert!(p.saw_result);
        assert_eq!(p.input_tokens, 120);
        assert_eq!(p.output_tokens, 60);

        let err = json!({
            "event": "error",
            "error": "authentication expired"
        });
        let evs = p.line(&err.to_string());
        assert_eq!(evs.len(), 1);
        match &evs[0] {
            AgentEvent::Error { message, .. } => assert!(message.contains("authentication expired")),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn an_error_result_that_carries_a_reply_keeps_the_turn_alive() {
        let adapter: Arc<dyn AgentAdapter> = Arc::new(Antigravity);
        let mut p = StreamParser::new(adapter);
        let result = json!({
            "event": "result",
            "result": {
                "conversation_id": "conv-456",
                "status": "ERROR",
                "response": "I rebuilt the app and committed.\n",
                "error": "API error (attempt 2): request failed: stream idle timeout (5m0s): context deadline exceeded",
                "usage": {"input_tokens": 10, "output_tokens": 5, "thinking_tokens": 0, "cache_read_tokens": 0, "total_tokens": 15}
            }
        });
        let evs = p.line(&result.to_string());
        assert_eq!(evs.len(), 1);
        match &evs[0] {
            AgentEvent::Status { text } => assert!(text.contains("stream idle timeout"), "the stale error stays visible: {text}"),
            other => panic!("expected Status, got {other:?}"),
        }
        let no_reply = json!({
            "event": "result",
            "result": {"status": "ERROR", "response": "", "error": "Eligibility check failed: EOF"}
        });
        let evs = p.line(&no_reply.to_string());
        assert!(matches!(evs[0], AgentEvent::Error { .. }), "an error with nothing delivered still fails the turn");
    }

    #[test]
    fn antigravity_model_catalog_parsing() {
        let piped = b"gemini-4-neon-high\tGemini 4 Neon (High)\n\
                      gemini-4-neon-medium\tGemini 4 Neon (Medium)\n\
                      gemini-3.8-flash-medium\tGemini 3.8 Flash (Medium)\n\
                      gemini-4-neon-high\tGemini 4 Neon (High)\n";
        let list = parse_models(piped).unwrap();
        assert_eq!(
            list,
            vec![
                models::AgentModel { id: "gemini-4-neon-high".into(), name: "Gemini 4 Neon (High)".into(), thinking: Vec::new() },
                models::AgentModel { id: "gemini-4-neon-medium".into(), name: "Gemini 4 Neon (Medium)".into(), thinking: Vec::new() },
                models::AgentModel { id: "gemini-3.8-flash-medium".into(), name: "Gemini 3.8 Flash (Medium)".into(), thinking: Vec::new() },
            ]
        );

        let spaced = b"gemini-4-neon-high              Gemini 4 Neon (High)\n\nclaude-sonnet-5-5-medium        Claude Sonnet 5.5 (Medium)\n";
        let list = parse_models(spaced).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "gemini-4-neon-high");
        assert_eq!(list[0].name, "Gemini 4 Neon (High)");
        assert_eq!(list[1].id, "claude-sonnet-5-5-medium");
        assert_eq!(list[1].name, "Claude Sonnet 5.5 (Medium)");

        let err = parse_models(b"   \n\n").unwrap_err();
        assert_eq!(err.code, ErrorCode::GarbledOutput);
    }

    #[test]
    fn antigravity_has_model_catalog_and_validates_extra_args() {
        assert!(Antigravity.has_model_catalog());
        let config = BackendConfig {
            name: "Antigravity".into(),
            kind: crate::BackendKind::AgentCli,
            model: "antigravity".into(),
            base_url: None,
            ssh_host: None,
            enabled: true,
        };
        let default_opts = BackendOptions::default();
        assert!(Antigravity.validate_model(&config, &default_opts, None, &Stop::default()).is_ok());

        let conflicting = BackendOptions {
            cli_model: Some("gemini-4-neon-high".into()),
            args: vec!["--model".into(), "gemini-3.8-flash-low".into()],
            ..Default::default()
        };
        let err = Antigravity.validate_model(&config, &conflicting, None, &Stop::default()).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadInput);
    }
}
