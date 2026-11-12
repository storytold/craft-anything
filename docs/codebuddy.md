# CodeBuddy CLI

Craft Anything can drive the installed CodeBuddy CLI for a deep build through the Agent Client
Protocol (ACP). The adapter exists because `--acp` is CodeBuddy's documented way to work from
another program, and that mode hands permission requests back to us.

## Setup

1. Install the CodeBuddy CLI so `codebuddy` is on `PATH` (or pick the program with **Choose...**).
2. Sign in. Either set `CODEBUDDY_API_KEY` for the computer Craft Anything runs on, or sign in with
   the CLI itself in a terminal. The China build also needs
   `CODEBUDDY_INTERNET_ENVIRONMENT=internal` for a key to authenticate; the overseas build leaves it
   unset, and the iOA build uses `ioa`. Craft Anything reads neither value and never sees the
   sign-in.
3. In Settings → AI, detect and add **CodeBuddy**, then choose it for **Build with this**. A chat
   and a question also work, running in plan mode where the CLI refuses writes and commands.

The equivalent backend entry in `config.toml` is:

```toml
[[ai.backends]]
name = "CodeBuddy"
kind = "agent_cli"
model = "codebuddy"
enabled = true
```

`model` names the agent. ACP is on for this agent. Do not set `acp = false` or an SSH host: those
are refused rather than starting a run with no approval channel. Extra `args` are refused too: the
documented command line is a single `--acp` flag, and an unknown flag could turn off the CLI's own
permission asks.

## What runs

A deep build starts:

```sh
codebuddy --acp --permission-mode default
```

A deep chat and a question use `--permission-mode plan` instead. There is no subcommand, no
trailing word, and no model flag on the command line.

## Model and thinking

Use the shared model picker (Settings → AI → **Change...**), or set `cli_model` on the same backend
entry. The list comes from the session's advertised models; the model itself is applied with
`session/set_config_option` (config `model`) before the session is ready, because `--acp` takes no
model flag. Thinking choices are whatever the selected model advertises, and a level the model does
not list is refused.

```toml
cli_model = "claude-sonnet-4"
cli_thinking = "high"
```

The session also advertised `sandbox` and `thought_level`. Thinking uses `thought_level`, and the
model is sent first because changing it can reset the depth. Nothing here advertised a
`context_window`, so the context control stays off.

## Tools, MCP and Clean room

Builds attach Craft Anything's MCP server on `session/new` as a stdio server named
`craft-anything`, with no `"type"` field (only Grok Build requires one). CodeBuddy also keeps the
MCP servers from your own configuration.

A deep build runs with `--permission-mode default`, so every tool call comes to
`session/request_permission` and the approval policy decides that call only. A deep chat and a
question run with `--permission-mode plan`, where the CLI refuses every write and command itself.
Clean room is refused before CodeBuddy starts: the adapter declares no file or terminal capability,
so the CLI reads files on its own in every mode, and `plan` does not stop a read.

This client answers a permission request with the `allow_once` option, found by its `kind` rather
than the CLI's own id (`allow`), so a grant never becomes `allow_always`.

CodeBuddy reaches every MCP tool through one wrapper, `DeferExecuteTool`, naming the real tool in
its input (`mcp__craft-anything__build_file_write`). The approval policy unwraps it, so it judges
`build_file_write` rather than the wrapper's name.

## What was checked

Against CodeBuddy CLI 2.182.0 on macOS, signed in:

- `initialize` answered `protocolVersion: 1` and `loadSession: true`.
- `session/new` answered with four config options: `mode` (permission mode), `model`, `sandbox`
  and `thought_level`. The `model` and `thought_level` ids are the ones this adapter uses, and a
  chosen model was validated against the live catalogue.
- `--permission-mode plan` really refuses writes: asked to run `mkdir`, the agent answered that
  plan mode forbids it and made no tool call.
- `--permission-mode default` really asks: a `Bash` call produced `session/request_permission` with
  `allow_always`, `allow_once` (id `allow`) and `reject_once` (id `reject`).
- Our MCP server was attached on the session and its tools appeared: with `--all-tools` the agent
  listed all of them, including `build_file_write`.

A whole build did not finish. Two builder turns ran; the agent called `ToolSearch` and
`DeferExecuteTool` but never got a file into `app/`, and the run stopped at its unchanged-turns
limit. `--permission-mode bypassPermissions` changed nothing, so approvals were not the cause: the
model spent its turns exploring (`ls`, its own `ToolSearch`, the cargo registry) instead of
writing. Two turns of one model say nothing final about CodeBuddy as a builder.

Unit tests cover the argument refusal, the access modes, the session config queue, the model
catalogue, the wrapper unwrapping, and the grants table.
