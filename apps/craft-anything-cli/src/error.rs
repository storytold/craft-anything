//! What can go wrong on the command line. A [`UsageError`] is the person's to fix (exit 2); a tool's
//! own failure is a `ToolError` (exit 1).

use thiserror::Error;

/// A command line that can't be run: the person's to fix, so the exit code is 2.
#[derive(Debug, Error)]
pub enum UsageError {
    #[error("missing command")]
    MissingCommand,
    #[error("--deep and --clean-room can't be used together")]
    DeepAndCleanRoom,
    #[error("unknown command {0:?}. Run `craft-anything-cli commands` to list tools, or --help for built-in commands.")]
    UnknownCommand(String),
    #[error("--project needs a folder")]
    ProjectNeedsFolder,
    #[error("--output-format needs one of json, jsonl, md, text, csv, tsv")]
    OutputFormat,
    #[error("{command} needs a target: craft-anything-cli {command} <path> …")]
    NeedsTarget { command: String },
    #[error("--kind must be one of anthropic, openai, openrouter, openai_compatible, agent_cli")]
    AiKind,
    #[error("--key-env or --key-stdin is needed to replace a key")]
    KeyNeeded,
    #[error(transparent)]
    Key(#[from] KeyError),
    #[error(transparent)]
    ToolArgs(#[from] crate::tool_args::ToolArgsError),
}

/// Reading an API key named by `--key-env` or sent with `--key-stdin`.
#[derive(Debug, Error)]
pub enum KeyError {
    #[error("environment variable {0} is not set")]
    EnvNotSet(String),
    #[error("reading the key from stdin: {0}")]
    Stdin(#[source] std::io::Error),
    #[error("no key on stdin")]
    EmptyStdin,
}
