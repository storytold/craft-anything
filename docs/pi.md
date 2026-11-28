# Pi

Craft Anything can use an installed pi CLI as the builder of a Deep or Clean room build. This is
separate from the provider pi is signed in to: no API key is imported, and Craft Anything never
reads pi's settings, sign-in or sessions.

## Setup

1. Install pi. Its program is `pi`, usually on `PATH` (Homebrew installs it in `/opt/homebrew/bin`).
2. Sign in the way pi does: run `pi` once in a terminal and sign in to the provider its default
   model uses. The sign-in belongs to pi's own configuration; a build stops with pi's own message
   when the provider refuses it.
3. In Settings → AI, detect and add **Pi**, then choose it for **Build with this**. `ai detect`
   offers it as soon as `pi` is found, and `ai test "Pi"` runs one tiny request.

The equivalent backend entry in `config.toml` is:

```toml
[[ai.backends]]
name = "Pi"
kind = "agent_cli"
model = "pi"
enabled = true
```

`model` names the agent. A model pi should use for this backend goes in the backend's
`cli_model`, as with the other agent apps (`craft-anything-cli ai model "Pi" <model>`); the
thinking depth goes in through `--thinking` when one is chosen.

## Models and thinking

Settings → AI lists pi's own models, asked of pi itself. `pi --mode rpc` answers
`get_available_models` with each model's `thinkingLevelMap`, so the picker offers exactly the
depths that model takes; a pi that doesn't answer falls back to `pi --offline --list-models`,
whose table only says whether a model reasons (so the depths it offers stop at `high`). Both are
asked with `--no-extensions`, because every run turns extensions off: a model an extension
supplied could not be used by a build anyway. A chosen model and depth are checked against that
same list before each run, so a model pi no longer offers, or a depth it doesn't take, stops with
a plain fix instead of being passed on and refused mid-build.

## What a run does

- One message per turn: `pi -p --mode json` with the message on stdin (never in argv, so a long
  brief stays clear of Windows' command-line limit). Its JSONL events are read line by line: the
  session header's id, `message_update` text deltas, both ends of every tool call, each assistant
  message's usage, and a retry that gave up as an error in plain words. Checked against pi 1.6.0
  (2026-11-25); 1.5.0 read the same records.
- The conversation continues with `--session-id`, which pi creates on the first turn and resumes
  afterwards; it is pi's project-local "this conversation, or a new one with this id". `--session`
  is pi's global lookup by path or id and can reach another folder's conversation, so it isn't used.
- Nothing of the person's or the project's loads beside the run: `--no-extensions` (a user's
  extensions and one a builder wrote into the project), `--no-context-files` (`AGENTS.md`,
  `CLAUDE.md`), and `--no-approve` (trust-gated project files even where a trust decision was saved).
  `--no-extensions` turns off pi's built-in extensions too, MCP support among them since 0.99, so
  `-e builtin:mcp` loads that one back by name — without it `registerMcpServer` fails and the build
  has no Craft Anything tools.
- A question about a build starts a fresh session with reading tools only (or none in Clean room),
  as `--no-session --no-mcp`: it leaves the builder's conversation and pi's session list untouched,
  and connects no server of anyone's. Its answer is therefore never marked as coming from the
  builder's session.
- A backend's saved extra arguments can't change any of that: a `--tools`, `--extension`, `--mode`,
  `--session`/`--session-id`, `--session-dir`, `--no-mcp` or `--approve`/`--no-approve` there is
  dropped before the run's own arguments, and so is anything after a bare `--` (pi would read it as
  the message). Other saved arguments, such as `--model` or `--offline`, are kept.
- A run over SSH is refused before it starts: the extension file is written on this computer, so a
  pi on the host would be handed a path it can't see. It says so in plain words instead.

## Sessions that travel with the project

pi keeps each conversation as `sessions/--<folder>--/<timestamp>_<id>.jsonl` under its own folder
(`PI_CODING_AGENT_DIR`, else `~/.pi/agent`). The project keeps a copy of it (`agent-session.jsonl`,
written after every turn), and a project moved to another computer puts it back in the folder pi
looks in for that project's working folder, named after the conversation's own timestamp, so the
same conversation carries on (#451). A folder reached through a link — `/tmp` is `/private/tmp` on
macOS — is resolved first, because pi names the real folder; without that the copy lands beside it
and pi starts a second conversation under the same id. When the copy is of another session, or
nothing can be put back, the id is dropped and the next turn starts from the brief, what is in
`app/`, and the newest exchanges kept in the copy (`handoff`). A resumed id pi still can't find is
started again as a fresh conversation under that same id, with only a warning on stderr; that
warning is reported to the person, and the id is kept, because pi's new conversation has it and
clearing it would start yet another one. A question is answered from the files anyway, so it is
never presented as the builder's own conversation.

## The tool server

pi has no `--mcp-config` flag the way Claude Code does. Servers come from `mcp.json` — which a
project only gets to define once it is trusted — or from an extension, so ours arrives as a
one-function extension the run loads with `-e`:

```js
export default function (pi) {
  pi.registerMcpServer("craft-anything-run", { command: "…/craft-anything-cli", args: ["mcp", …], exposure: "direct" });
}
```

- The file is written into a folder of the run's own under the system temporary folder,
  owner-only on Unix (`0700` folder, `0600` file), so nothing another user could prepare in a
  shared temporary folder is ever loaded through it. A folder that can't take it stops the build
  before it starts, with a fix. The folder is removed when the turn ends, however it ends
  (finished, stopped, timed out or failed first), so a build leaves nothing behind.
- The server is named `craft-anything-run`, not `craft-anything`: pi refuses two clients with one
  name, and a `mcp.json` entry wins over an extension's, so a person's own registration is never
  replaced or reached. Its tools appear as `mcp__craft_anything_run__<tool>`, and Craft Anything
  shortens them back to the tool ids the rest of the app matches on.
- `exposure: "direct"` is what puts the tools in the model's list; pi's default exposure keeps
  them behind `codemode`, where a builder never sees their names.
- A builder's server is given `--all-tools`: this builder has no editor or shell of its own, so
  `build_file_write`, `build_file_edit` and `build_command_run` are how it changes the project.
  Clean room still refuses what it may not read — listing a tool never makes it available.

## What a builder may do

`--tools` in pi filters MCP tools as well as built-in ones when one of its entries starts with
`mcp__`, so the run is held to:

| Access | Tools |
|---|---|
| Build (Deep) | `read`, `grep`, `find`, `ls`, `edit`, `write`, `bash`, `powershell` — each change and command approved by the bridge below — and this run's `mcp__craft_anything_run__*` |
| Build (Clean room) | this run's tools only: no file tools of the agent's own |
| Question (Deep) | `read`, `grep`, `find`, `ls`; no tool server |
| Question (Clean room) | `--no-tools` |

`codemode` and `tool_search` are left out on purpose: `codemode` calls a server's tools by name
whatever the active tool set says, so leaving it in would reach the servers this run isn't meant
to use. No other server connects: a question passes `--no-mcp`, and a build's `--tools` names
`mcp__craft_anything_run__*`, which filters MCP tools down to ours, so a person's own `mcp.json`
servers are neither declared nor reachable.

## Approvals

pi has no `--permission-prompt-tool`, so the same generated extension carries the bridge itself:
its `tool_call` handler maps pi's changing tools (`bash`, `powershell`, `write`, `edit`) into
Claude Code's shape (`Bash {command}`, `Write {file_path, content}`, `Edit {file_path, …}`) and runs
`craft-anything-cli core-action-approve` with this run's project and mode. The action's input goes
in on stdin (`--json -`), not on the command line: a written file's content can be far past the
point where Windows refuses to start a program, so a large write would otherwise be refused.
`allow` lets the call go on; `deny` blocks it and hands the policy's own words back to the model,
so it knows what to do instead. Reads and our tools pass straight through — ours carry their own
consent.

The call has a deadline of 35 minutes, a little past the approval policy's own 30-minute wait for
the person. Running out kills the call and blocks the action with the error in words, so a stuck
CLI can neither hang the builder nor let the action through.

Nothing is installed into pi's own configuration: the bridge lives in the per-run extension file
described above and is deleted with it. A run that cannot write that file stops before it starts.

## What was verified

- Unit tests: arguments (message on stdin, model and effort, resume, tool lists per access mode),
  the extension's contents and permissions, two runs never sharing a folder, JSONL parsing (text,
  both ends of a tool call, usage, a failed retry), and quoting.
- Live on macOS 15 (pi 1.5.0, Craft Anything 0.4.0, 2026-11-09): `ai detect`, `ai add "Pi"` and
  `ai test "Pi"` (370 ms); one build from a request (`--rounds 1`) and one with `--rounds 0` both
  reached Ready, with `app/index.html` and `app/styles.css` written through `build_file_write` and
  `build_file_edit`, `build_checklist` and `rebuild_check` called, and a refused website approval
  reported back as such. `craft.json` records `agent_session_by: "pi"`.
- A session that travels: a build, its project copied to another folder, every local pi session for
  it deleted, then `--continue-in <copy>` — pi continued the restored file (the follow-up prompt is
  the file's second user message) and the change reached `app/index.html`.
- The approval bridge, in a Deep build: five `bash` calls and two `write` calls ran after the policy
  allowed them (`cargo --version` reached the page's footer as its exact output), and one action the
  policy refused ("Run a scripted web session") was blocked and reported to the builder.
- Not verified: a Clean room build end to end (its tool list is unit-tested only), a live build
  with a model and depth chosen in Settings (both are read from pi and checked), a sign-in
  pre-check (`pi auth check` needs a provider or a model the adapter isn't given), and pi versions
  other than 1.5.0 and 1.6.0.
- Checked on pi 1.6.0 (2026-11-25, macOS): the generated extension loads and its tool is declared
  (`mcp__craft_anything_run__*`), `--no-extensions -e builtin:mcp` keeps the person's and the
  project's extensions and `AGENTS.md` out while our server still connects, `--tools` with an
  `mcp__` entry filters other servers' tools out, `--session-id` creates and resumes in pi's usual
  folder, and `-p` reads the message from stdin. The approval call took a 300,000-character input on
  stdin, refused a malformed payload, and its deadline blocked a CLI that never answered. A
  provider's error code `14018` is read as the used-up plan it is, not as a `401` bad key (a
  status code counts only as its own number). `ai model "Pi"` listed pi's 57 configured models,
  each with its own depths (`xhigh`/`max` only where mapped, none for a model that doesn't reason).
- A first live run on 1.5.0 left a folder in the temporary folder after every turn; the run's
  folder is now removed when the turn ends.
