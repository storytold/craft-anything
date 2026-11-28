# CodeBuddy CLI

Craft Anything can drive the installed CodeBuddy CLI for a deep build through the Agent Client
Protocol (ACP). The adapter exists because `--acp` is CodeBuddy's documented way to work from
another program, and that mode hands permission requests back to us.

## Setup

1. Install the CodeBuddy CLI so `codebuddy` is on `PATH` (or pick the program with **Choose...**).
2. Sign in. Either set `CODEBUDDY_API_KEY` in this computer's environment, or sign in with the CLI
   itself in a terminal. The China build also needs `CODEBUDDY_INTERNET_ENVIRONMENT=internal` for a
   key to authenticate; the overseas build leaves it unset, and the iOA build uses `ioa`. Craft
   Anything reads neither value and never sees the sign-in.
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

`model` names the agent. ACP is on for this agent, on this computer. Do not set
`acp = false`: it is refused rather than starting a run with no approval channel. Extra `args` are
refused too: the documented command line is a single `--acp` flag, and an unknown flag could turn
off the CLI's own permission asks.

## What runs

A deep build starts:

```sh
codebuddy --acp --permission-mode default
```

A deep chat and a question use `--permission-mode plan` instead. There is no subcommand, no
trailing word, and no model flag on the command line.

## On another computer

Not yet. Detect offers CodeBuddy on this computer only, and a backend with an `ssh_host` set is
refused with "CodeBuddy requires a local ACP connection". The reason is the check, not the session:
nothing can yet see what a build on a host wrote, so a working remote build would stop as unchanged
(the open roadmap line below).

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

A deep build runs with `--permission-mode default`. A tool call that none of CodeBuddy's own `allow`
rules resolves first comes to `session/request_permission`, and the approval policy decides that
call only: `permissions.allow` entries from your settings or the project, and reads inside trusted
directories, run without a request. A deep chat and a question run with `--permission-mode plan`,
where the CLI refuses every write and command itself. Clean room is refused before CodeBuddy
starts: the adapter declares no file or terminal capability, so the CLI reads files on its own in
every mode, and `plan` does not stop a read.

This client answers a permission request with a one-time option, found by its `kind` rather than
the CLI's own id (`allow`): `allow_once` for a grant, so a grant never becomes `allow_always` (a
request that offers only an always-grant is cancelled), and `reject_once` for a refusal, whatever
order the options come in (an agent's persistent refusal is taken only when it offers no one-time
one).

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

A whole local build ran on 2026-11-25 (2.185.0, macOS): a Rust rebuild of a small Python guessing
game wrote `app/`, `cargo build --release` passed, the check passed 4 of 5 (the mechanics
comparison stays unverified), and the program runs. Two earlier attempts had ended after two turns
that wrote nothing; the cause was the project refusing the builder's own tools (`other_tools`, the
one kind with no rule of its own) in Auto mode, so its `Running` calls were refused. Allowing that
kind for the project let it write.

The ACP session also ran over `ssh` on 2026-11-25 (CodeBuddy 2.186.0 on an x86_64 Linux host, after
a sign-in there): a question ran the whole ssh path with the folder that host reported, `doctor`
answered for the remote backend, the remote helper answered an MCP `initialize`, and a whole build
wrote `app/` on the host, where `cargo build` and its screenshot and self-test checks passed. What
could not run there is the comparison against the original: the project record and the target live
on this computer, the stall check watches this computer's `app/` (which stays empty while the
builder writes on the host), and the builder's `rebuild_check` found no project record on the host
— the run stops as unchanged. That is why the remote case is not offered yet, and it is open in
the roadmap.

All of this ran on macOS. Windows was not tried: CodeBuddy installs through npm there, and npm
shims on Windows have failed starts here before (see `docs/pitfalls.md`, "Found on Windows, but
\"isn't on your PATH\" when run" — `which` finds a `gemini.cmd`-style shim but a bare name starts
only `.exe`). Until it is tried, say unknown rather than assume either way.

Unit tests cover the argument refusal, the access modes, the session config queue, the model
catalogue, the wrapper unwrapping, and the grants table.
