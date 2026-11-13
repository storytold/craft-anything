# Using CodeBuddy Code as the builder

Craft Anything can build with the person's own local CodeBuddy Code (`codebuddy`, also installed
as `cbc`). This page records what the integration does, why it is shaped this way, and what
remains to confirm.

## How it works

- The `codebuddy` adapter (`crates/ai/src/cli_agents/adapters/codebuddy.rs`) drives CodeBuddy over
  the Agent Client Protocol (`codebuddy --acp`), as the app drives Grok Build and Copilot. It is
  the only path: an SSH host, an `acp = false` backend, or an ACP failure is reported, not retried
  through the CLI.
- One process serves the whole conversation: `initialize`, `session/new` with our MCP server
  (`craft-anything-cli mcp --project …`), then `session/prompt` per message, and `session/load`
  to continue a saved session (its `agentCapabilities.loadSession` is true). The model is picked
  with `session/set_config_option` on the session it opens.
- Permissions: a build runs in CodeBuddy's `default` permission mode, so a change it wants to make
  reaches our approval policy (`session/request_permission`) instead of being decided by the agent.
  Its requests put the command under `rawInput.command` beside its own description of it (as a
  string, or as words); the policy reads the command from every shape agents have been seen to
  send and never from the description. A refusal carries the policy's reason back as the request's
  `message`, so a builder that is told to stay inside the project or use a build command has
  something to act on. CodeBuddy ends its turn on any refusal it receives (its trace records it as
  `cancel_reason: "user_rejected"`), whatever the reason: so a request it can't approve, like a
  command that skips its sandbox, must not be one a build depends on.
- Checked live against CodeBuddy Code 2.183.0 (2026-11-12): `--acp` answers with
  `protocolVersion: 1` and `loadSession: true`; `session/new` takes our server; a prompt's MCP call
  and a `Write` both sent a permission request, which we answered; a `Read` ran **without** one,
  and in `acceptEdits` mode a write did too.
- Because its own reading doesn't ask, only a **deep build** is allowed. A Clean room run, a deep
  chat and a question are refused with that reason. Two of its tools (`ListMcpResources`,
  `ReadMcpResource`) can't be refused at all, which is why a question is never handed a server.
- Sign-in stays in the CLI: `codebuddy login`. Its folder is `~/.codebuddy`, or
  `CODEBUDDY_CONFIG_DIR` when set.
- A turn whose agent stops talking ends by itself: the same limits the per-message runner uses
  apply to the conversation (a chat 15 minutes, a builder 8 hours without a line), after which
  `session/cancel` is sent, the turn is reported as stopped, and an agent that doesn't answer
  within 10 seconds is killed. Before this a stalled turn kept the conversation busy for as long
  as its process lived.
- Its headless `stream-json` (`codebuddy -p --output-format stream-json`) is a viable native path,
  but has no permission-prompt tool (there a build must run with approvals off), so it isn't used.

## MCP registration

`craft-anything-cli setup --client codebuddy` writes our server into
`~/.codebuddy/.mcp.json` (or the folder `CODEBUDDY_CONFIG_DIR` names, or `--project` scope when
run inside a project). The entry is the plain `{command, args}` shape:

```json
{"mcpServers": {"craft-anything": {"command": "/path/to/craft-anything-cli", "args": ["mcp"]}}}
```

`codebuddy mcp add craft-anything -s user -- <path>/craft-anything-cli mcp` does the same from
CodeBuddy's own CLI.

## Setup

1. Install CodeBuddy Code (`npm install -g @tencent-ai/codebuddy-code`, or its installer). Its
   program is `codebuddy`.
2. Sign in once with `codebuddy login`.
3. In Settings → AI, detect and add **CodeBuddy Code**, then choose it for **Build with this**.
   If `codebuddy` is installed somewhere else, use **Choose...**.

The equivalent backend entry in `config.toml` is:

```toml
[[ai.backends]]
name = "CodeBuddy Code"
kind = "agent_cli"
model = "codebuddy"
enabled = true
```

## Still to confirm

- A live deep build end to end through the app (the ACP exchange above was checked scripted).
- Whether reasoning depth can be set on the session: its `configOptions` list effort options, but
  no run has shown a chosen depth reaching the session, so none is offered in the picker.
- Whether `session/prompt` takes notes mid-turn (none is passed).
- The list of models loads from a metadata session (checked live, 2.183.0); a backend may be
  configured under `codebuddy`, `code-buddy` or `cbc`.