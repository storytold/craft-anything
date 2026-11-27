# The command line

`craft-anything-cli` runs every tool as a subcommand, the MCP server (`mcp`), setup, doctor and a
headless chat. **Follows [RFC-0018](https://github.com/storytold/craft-contrib/blob/main/rfcs/0018-cli-conventions.md)**
(command-line conventions), except for the rules listed at the end.

`--help` is the reference for every option; this page only says what `--help` can't.

```sh
craft-anything-cli --help
craft-anything-cli commands                  # every tool
craft-anything-cli core-journal-read --help  # one tool's options, from its schema
craft-anything-cli triage ./app.apk
```

## Exit codes and streams

| Code | Meaning |
|---|---|
| 0 | Success. |
| 1 | The command failed. The structured error (`{"error": …}`) is on stdout, so a script can read it, and `error: <message>` is on stderr. |
| 2 | Usage error: the command line was refused before any work started. `error: <message>` on stderr. |
| 128 + signal | A signal ended it: 130 for Ctrl-C (a second Ctrl-C leaves at once). |

Results go to stdout; errors, warnings, progress and the log go to stderr. `mcp` writes only the
protocol to stdout. JSON is indented; `--compact` prints it on one line (`--format jsonl` is
always one line per item).

## Strict options

These are usage errors (exit 2), checked before the tool or the command does anything:

- an unknown option, or an option given twice (list options, such as `doctor --client`, repeat);
- a value of the wrong type or malformed JSON;
- an option that conflicts with another, or sets a parameter that `--json` already set;
- a value that is another option: write a value that starts with `-` as `--option=-value`
  (negative numbers and `-` alone are values); `--` ends the options.

A tool's own options come from its schema (`craft-anything-cli <tool> --help`); the built-in
commands' come from `--help`.

### `--json`

- Before a tool name, or bare after it, `--json` is the shorthand for `--format json`
  (it can't be combined with `--format`).
- After a tool name, `--json '{…}'`, `--json=@params.json` and `--json -` (standard input) give
  the tool's parameters. A value that starts with `{`, `@` or is `-` after `--json` is taken as
  parameters; anything else leaves `--json` as the shorthand.
- `--format` after a tool name goes to the tool when it has an option of that name; `--output-format`
  always means the output format.

## Environment

| Variable | Flag | Notes |
|---|---|---|
| `CRAFT_ANYTHING_LOG` | `--log <off\|error\|warn\|info\|debug\|trace>` | Log lines on stderr. An empty value is unset; the flag wins; a bad value is a usage error naming the variable. |

`CRAFT_ANYTHING_CONFIG_DIR` (where settings live) and `CRAFT_ANYTHING_AGENT` (set by the coding
agent Craft Anything starts, so its tool calls count as an agent's) are read by the engine; they are
not fallbacks for flags. No variable sets a destructive option, grants file or network access, or
holds a secret that is printed. An API key is read from the variable a flag names
(`ai add --key-env NAME`) or from standard input (`--key-stdin`), never from a fixed variable.

## Shell completions and man pages

The Linux packages (deb, rpm, tarball, AppImage) and the FreeBSD tarball install the bash, zsh and
fish completions and a man page for the command and each built-in subcommand (`man craft-anything-cli`,
`man craft-anything-cli-setup`). They are generated from the clap definition by the
`man-and-completions` example (`cargo run -p craft-anything-cli --example man-and-completions -- <dir>`,
which also writes elvish and PowerShell files that no package installs). macOS and Windows install
none, as RFC-0018 allows. Tools are not clap subcommands, so they have no pages or completions of
their own; `craft-anything-cli <tool> --help` documents each.

## Rules not met yet

- **1.5, paths:** the path arguments of tools, `triage` and `report` are read as text, so a file
  name that isn't valid Unicode doesn't reach them. Options that clap types as paths (`--project`,
  `--out`, `--inbox`) are paths.
- **1.3, `--json` on every subcommand that a program reads:** `--format json` (the default) does that
  today; there is no `--json` on `setup`, `doctor`, `ai` and the others beyond the global shorthand.
- **2.1, types for every value:** tool options are typed from the tool's JSON schema at run time,
  because tools are registered by the engine, not declared in clap.
- **3, ast-grep rules:** not on yet; they wait for the shared rules in craft-common-actions
  (storytold/craft-common-actions#380).
