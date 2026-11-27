//! `craft-anything-cli`: every tool as a subcommand, the MCP server, setup, doctor and headless chat.
//!
//! Exit codes: 0 success, 1 the operation failed (a structured error is printed), 2 usage error,
//! 130 interrupted.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[macro_use]
mod io;

mod ai_cmd;
mod error;
mod output;
mod rebuild_cmd;
mod setup_cmd;
mod tool_args;

use clap::{Parser, Subcommand};
use craft_anything_engine::project::{Project, Purpose};
use craft_anything_engine::{CallOptions, Caller, CancelToken, Consent, Engine, Requires, Session, Settings, ToolOutput};
use craft_anything_evidence::ToolError;
use error::UsageError;
pub use io::Io;
use output::Format;
use serde_json::{Value, json};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub static INTERRUPTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Parser)]
#[command(
    name = "craft-anything-cli",
    version,
    propagate_version = true,
    about = "Craft Anything on the command line: every tool, the MCP server, setup and doctor."
)]
#[command(after_help = "Every tool is also a subcommand: `craft-anything-cli commands` lists them, `craft-anything-cli <tool> --help` shows its options.\n\
When a tool has its own --format option (core-target-open, ...), --format after the tool name goes to the tool; pick the output with --output-format (or --format before the tool name). `report` has no format option of its own: use `report --html` for HTML.\n\
`--json` before a tool name, or bare after it, means `--format json`; `--json '{..}'`, `--json=@file.json` or `--json -` after a tool name gives the tool's parameters. Each option may be given once (list options repeat), a value that starts with - is written `--option=-value`, and `--` ends the options.")]
pub struct Cli {
    /// Where state lives and whose home `setup` configures: this computer's, unless a test gives another.
    #[arg(skip)]
    pub context: Context,
    /// Output format. `--output-format` always means this, even after a tool that has its own `--format`.
    #[arg(long, global = true, value_enum, default_value_t = Format::Json, alias = "output-format")]
    pub format: Format,
    /// Shorthand for --format json.
    #[arg(long, global = true, conflicts_with = "format")]
    pub json: bool,
    /// A project folder (`*.cany`): evidence, memory and the journal are kept there. Created if missing.
    #[arg(long, global = true)]
    pub project: Option<PathBuf>,
    /// Allow tools that run code or change a device without asking.
    #[arg(long, global = true)]
    pub allow_run: bool,
    /// Deep mode for this run only: tools may read a target's code, files and page source.
    #[arg(long, global = true, conflicts_with = "clean_room")]
    pub deep: bool,
    /// Clean room for this run only, whatever Settings say: nothing inside a target is read.
    #[arg(long, global = true)]
    pub clean_room: bool,
    /// Print JSON on one line instead of indented.
    #[arg(long, global = true)]
    pub compact: bool,
    /// Print the log on stderr at this level and above. Written before the command name.
    #[arg(long, value_enum, ignore_case = true, env = "CRAFT_ANYTHING_LOG", value_name = "LEVEL")]
    pub log: Option<LogLevel>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the MCP server on stdin/stdout.
    Mcp {
        /// List every tool instead of the Essentials plus enabled toolsets.
        #[arg(long)]
        all_tools: bool,
        /// Run in restricted mode for shared hosts: turn off machine-acting tools,
        /// and turn off web and capture tools by default.
        #[arg(long)]
        restricted: bool,
        /// Turn on web tools (web_site_open) in restricted mode.
        #[arg(long)]
        allow_web: bool,
        /// Turn on capture tools (web_site_capture, web_page_states_capture) in restricted mode.
        #[arg(long)]
        allow_capture: bool,
        /// A folder where questions and approvals for the person are posted (`core_person_ask`,
        /// `core_action_approve`); the Craft Anything app answers them.
        #[arg(long)]
        inbox: Option<PathBuf>,
        /// Target metadata from the desktop (JSON); repeat for each target, active target last.
        #[arg(long = "target")]
        targets: Vec<String>,
    },
    /// Connect Craft Anything to your coding-agent apps.
    Setup(setup_cmd::SetupArgs),
    /// Check the install, registrations, engines, AI backends and the MCP server.
    Doctor(setup_cmd::DoctorArgs),
    /// Remove Craft Anything's registrations and skill from agent apps.
    Uninstall(setup_cmd::UninstallArgs),
    /// List every tool.
    Commands,
    /// What this install can do: toolsets, tool availability, engines.
    Capabilities,
    /// Open a file and run a first look at it.
    Triage { path: String },
    /// Write a readable investigation report (Markdown or HTML) from findings and evidence.
    Report {
        /// Optional target to open first (its sha256 ties evidence to a file). Omit to report on
        /// the open project's memory and ledger.
        path: Option<String>,
        /// Write a self-contained HTML page instead of Markdown.
        #[arg(long)]
        html: bool,
        /// Write to this file instead of printing to stdout.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Leave out the evidence-ledger appendix.
        #[arg(long)]
        no_evidence: bool,
    },
    /// Ask the AI agent a question about a target (streams to stdout).
    Chat(ai_cmd::ChatArgs),
    /// Study a website, app or game and have a coding agent rebuild it in a new project folder.
    Rebuild(rebuild_cmd::RebuildArgs),
    /// Manage AI backends.
    Ai {
        #[command(subcommand)]
        command: ai_cmd::AiCommand,
    },
    /// For a coding agent's hook, run after each of its tool calls: hand over the notes the person
    /// added while it works (printed as the hook's JSON; nothing when none wait).
    #[command(hide = true)]
    BuilderNotes { dir: PathBuf },
    /// Run a tool by name: `craft-anything-cli <tool> [path] [--key value] [--json '{..}' | @file.json | -]`.
    #[command(external_subcommand)]
    Tool(Vec<String>),
}

impl Cli {
    pub fn format(&self) -> Format {
        if self.json { Format::Json } else { self.format }
    }

    /// The mode `--deep` or `--clean-room` asks for, if either.
    pub fn deep_for_run(&self) -> Option<bool> {
        (self.deep || self.clean_room).then_some(self.deep)
    }
}

/// How much of the log goes to stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    fn filter(self) -> log::LevelFilter {
        match self {
            Self::Off => log::LevelFilter::Off,
            Self::Error => log::LevelFilter::Error,
            Self::Warn => log::LevelFilter::Warn,
            Self::Info => log::LevelFilter::Info,
            Self::Debug => log::LevelFilter::Debug,
            Self::Trace => log::LevelFilter::Trace,
        }
    }
}

/// Send the log to stderr at `level`. The logger is process-wide, so only the first call in a
/// process sets it.
fn init_logging(level: Option<LogLevel>) {
    struct Stderr(log::LevelFilter);
    impl log::Log for Stderr {
        fn enabled(&self, m: &log::Metadata<'_>) -> bool {
            m.level() <= self.0
        }
        fn log(&self, r: &log::Record<'_>) {
            if self.enabled(r.metadata()) {
                eprintln!("[{}] {}: {}", r.level(), r.target(), r.args());
            }
        }
        fn flush(&self) {}
    }
    let level = level.unwrap_or(LogLevel::Off).filter();
    if level != log::LevelFilter::Off && log::set_logger(Box::leak(Box::new(Stderr(level)))).is_ok() {
        log::set_max_level(level);
    }
}

/// Print an error: the structured error on stdout, so a script can parse it, and `error: <message>`
/// on stderr. Exit code 1.
pub fn fail(io: &mut Io<'_>, e: &ToolError, f: Format) -> i32 {
    io.emit(&json!({"error": e}), f);
    io.error(&e.message);
    1
}

/// A usage error: `error: <message>` on stderr, exit code 2.
pub fn usage(io: &mut Io<'_>, e: impl Into<UsageError>) -> i32 {
    io.error(e.into());
    2
}

/// Where the CLI keeps its state and whose home `setup` configures. The default is this computer's
/// (`settings::config_dir()`, the real home, the system key store): a run behaves as it always did.
/// Tests give a folder of their own, so the commands that save settings, keys and client
/// configuration can run in-process without touching the person's setup.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// The settings folder (config.toml, ai-keys.toml, logs); `None`: `settings::config_dir()`.
    pub config_dir: Option<std::path::PathBuf>,
    /// The machine `setup`, `doctor` and `uninstall` configure; `None`: [`Host::current`].
    pub host: Option<craft_anything_setup::Host>,
    /// The program and arguments `setup` registers with each client; `None`: this computer's
    /// `craft-anything-cli mcp` ([`craft_anything_setup::default_command`]).
    pub mcp_command: Option<Vec<String>>,
    /// What standard input holds, for the commands that read answers or a key from it; `None`:
    /// the real standard input.
    pub stdin: Option<Stdin>,
}

/// A fixed standard input, read once from the front.
#[derive(Debug, Clone, Default)]
pub struct Stdin(Arc<craft_anything_platform::sync::Mutex<std::io::Cursor<Vec<u8>>>>);

impl Stdin {
    pub fn new(text: &str) -> Self {
        Self(Arc::new(craft_anything_platform::sync::Mutex::new(std::io::Cursor::new(text.as_bytes().to_vec()))))
    }
}

impl Context {
    /// Whether answers can be asked for: a fixed input always can; the real one only on a terminal.
    pub fn can_ask(&self) -> bool {
        use std::io::IsTerminal as _;
        self.stdin.is_some() || (std::io::stdin().is_terminal() && std::io::stderr().is_terminal())
    }

    /// One line of input (without its end), empty at the end of the input.
    pub fn read_line(&self) -> String {
        use std::io::BufRead as _;
        let mut s = String::new();
        let _ = match &self.stdin {
            Some(fixed) => fixed.0.lock().map_or(Ok(0), |mut c| c.read_line(&mut s)),
            None => std::io::stdin().lock().read_line(&mut s),
        };
        s.trim_end_matches(['\r', '\n']).to_string()
    }

    /// Up to `limit` bytes of input, to its end.
    pub fn read_to_string(&self, limit: u64) -> std::io::Result<String> {
        use std::io::Read as _;
        let mut s = String::new();
        match &self.stdin {
            Some(fixed) => match fixed.0.lock() {
                Ok(mut c) => (&mut *c).take(limit).read_to_string(&mut s)?,
                Err(_) => 0,
            },
            None => std::io::stdin().take(limit).read_to_string(&mut s)?,
        };
        Ok(s)
    }

    /// The settings folder this run uses.
    pub fn config_dir(&self) -> std::path::PathBuf {
        self.config_dir.clone().unwrap_or_else(craft_anything_engine::settings::config_dir)
    }

    /// The machine setup works on.
    pub fn host(&self) -> craft_anything_setup::Host {
        self.host.clone().unwrap_or_else(craft_anything_setup::Host::current)
    }

    /// What `setup` registers.
    pub fn mcp_command(&self) -> Result<Vec<String>, ToolError> {
        match &self.mcp_command {
            Some(command) => Ok(command.clone()),
            None => craft_anything_setup::default_command(),
        }
    }

    /// The API keys: the system store, or a file in the settings folder when one was given.
    pub fn key_store(&self) -> craft_anything_ai::KeyStore {
        match &self.config_dir {
            Some(dir) => craft_anything_ai::KeyStore::file_only(dir.join("ai-keys.toml")),
            None => craft_anything_ai::KeyStore::system(),
        }
    }

    /// An AI hub over `engine`, with this run's settings file and keys.
    pub fn hub(&self, engine: Arc<Engine>) -> craft_anything_ai::AiHub {
        match &self.config_dir {
            Some(dir) => craft_anything_ai::AiHub::with_stores(engine, dir.join("config.toml"), self.key_store()),
            None => craft_anything_ai::AiHub::new(engine),
        }
    }
}

/// The engine with settings loaded, the run's mode, and the project (if any) opened.
pub fn make_engine(project: Option<&Path>, deep_for_run: Option<bool>) -> Result<Arc<Engine>, ToolError> {
    make_engine_in(&Context::default(), project, deep_for_run)
}

/// [`make_engine`] over `context`'s settings folder.
pub fn make_engine_in(context: &Context, project: Option<&Path>, deep_for_run: Option<bool>) -> Result<Arc<Engine>, ToolError> {
    craft_anything_platform::sync::start_watchdog(context.config_dir().join("logs"));
    let mut session = match &context.config_dir {
        // Folders the settings don't name stay inside it too, like `Session::isolated`: nothing a
        // run makes lands in the person's own projects or engine folders.
        Some(dir) => {
            let mut settings = Settings::load_from(&dir.join("config.toml"));
            settings.projects_dir.get_or_insert_with(|| dir.join("projects"));
            settings.engines.store_dir.get_or_insert_with(|| dir.join("engines"));
            Session::for_a_run_from(settings, Some(dir.clone()), None)
        }
        None => Session::new(Settings::load()),
    };
    session.deep_for_run = deep_for_run;
    if let Some(dir) = project {
        session.open_project(open_or_create_project(dir)?)?;
    }
    let plugins = craft_anything_engine::plugins::install(&mut session);
    let mut layer_tools = craft_anything_ai::tools();
    layer_tools.extend(craft_anything_setup::tools::specs());
    Ok(Engine::shared_with(session, layer_tools, plugins))
}

fn open_or_create_project(dir: &Path) -> Result<Project, ToolError> {
    let dir = std::path::absolute(dir).map_err(|e| ToolError::io(dir.display(), e))?;
    if dir.join("project.toml").is_file() {
        return Project::open(&dir);
    }
    let name = dir.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "project".into());
    Project::create(&dir, &name, Purpose::default())
}

pub fn call_options(cli: &Cli, cancel: &CancelToken) -> CallOptions<'static> {
    let consent = if cli.allow_run { Consent::Granted } else { Consent::NotAsked };
    CallOptions::new(caller()).with_consent(consent).with_cancel(cancel.clone())
}

/// The person, unless a coding agent Craft Anything started is running this CLI (it then calls
/// tools as an agent, which can't loosen a project's approvals).
fn caller() -> Caller {
    if std::env::var_os(craft_anything_engine::AGENT_ENV).is_some() { Caller::Agent } else { Caller::Cli }
}

/// A cancel token that trips on Ctrl-C.
pub fn interrupt_token() -> CancelToken {
    let token = CancelToken::default();
    let t = token.clone();
    let _ = std::thread::Builder::new().name("ctrl-c-watch".into()).spawn(move || {
        while !INTERRUPTED.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        t.cancel();
    });
    token
}

/// The clap command: what `--help`, the completions and the man pages are made of. An empty
/// variable counts as unset (RFC-0018 rule 1.7), where clap would hand its empty value on.
pub fn command() -> clap::Command {
    fn blank_envs(command: clap::Command) -> clap::Command {
        command
            .mut_args(|arg| {
                let blank = arg.get_env().and_then(std::env::var_os).is_some_and(|v| v.to_str().is_some_and(|v| v.trim().is_empty()));
                if blank { arg.env(None::<&str>) } else { arg }
            })
            .mut_subcommands(blank_envs)
    }
    blank_envs(<Cli as clap::CommandFactory>::command())
}

/// A bad value that came from `CRAFT_ANYTHING_LOG`, not from the command line, is reported under the
/// variable's name (RFC-0018 rule 1.7): the person never typed the flag.
fn env_value_error(e: &clap::Error, args: &[OsString]) -> Option<String> {
    use clap::error::{ContextKind, ErrorKind};
    if !matches!(e.kind(), ErrorKind::InvalidValue) {
        return None;
    }
    let arg = e.get(ContextKind::InvalidArg)?.to_string();
    if !arg.starts_with("--log") || args.iter().any(|a| a.to_str().is_some_and(|a| a == "--log" || a.starts_with("--log="))) {
        return None;
    }
    let value = e.get(ContextKind::InvalidValue)?;
    let valid = e.get(ContextKind::ValidValue).map(ToString::to_string).unwrap_or_default();
    Some(format!("CRAFT_ANYTHING_LOG: invalid value '{value}' (expected one of {valid})"))
}

/// Run the command line `args` (the program name first), writing results to `out` and errors,
/// warnings and progress to `err`. Returns the exit code: 0, 1 when the command failed, 2 for a
/// usage error, 130 when Ctrl-C ended it.
pub fn run<I, T>(args: I, out: &mut dyn Write, err: &mut dyn Write) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    run_in(args, out, err, Context::default())
}

/// [`run`] over `context`: the same command line, with state kept where the context says.
pub fn run_in<I, T>(args: I, out: &mut dyn Write, err: &mut dyn Write, context: Context) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let mut io = Io { out, err, pretty: true };
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let matches = match command().try_get_matches_from(&args) {
        Ok(m) => m,
        Err(e) if !e.use_stderr() => {
            io.raw(&e.render().to_string());
            return 0;
        }
        Err(e) => {
            match env_value_error(&e, &args) {
                Some(named) => io.error(named),
                None => {
                    let _ = io.err.write_all(e.render().to_string().as_bytes());
                }
            }
            return 2;
        }
    };
    let mut cli = match <Cli as clap::FromArgMatches>::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => {
            let _ = io.err.write_all(e.render().to_string().as_bytes());
            return 2;
        }
    };
    cli.context = context;
    io.pretty = !cli.compact;
    init_logging(cli.log);
    let code = dispatch(&mut io, cli);
    if INTERRUPTED.load(Ordering::SeqCst) { 130 } else { code }
}

#[cfg(test)]
mod tests;

fn dispatch(io: &mut Io<'_>, cli: Cli) -> i32 {
    let f = cli.format();
    match &cli.command {
        // Runs after every tool call of a builder: no engine, nothing but the hook's JSON on stdout.
        Command::BuilderNotes { dir } => {
            use craft_anything_ai::rebuild::notes;
            if let Some(out) = notes::hook_output(&notes::claim(dir)) {
                outln!(io, "{out}");
            }
            0
        }
        Command::Mcp { all_tools, restricted, allow_web, allow_capture, inbox, targets } => {
            let engine = match make_engine_in(&cli.context, cli.project.as_deref(), cli.deep_for_run()) {
                Ok(e) => e,
                Err(e) => {
                    errln!(io, "craft-anything mcp: {}", e.message);
                    return 1;
                }
            };
            if let Some(dir) = inbox {
                engine.session().ext.insert(Arc::new(craft_anything_engine::inbox::InboxDir(dir.clone())));
                // A builder can generate game assets with the person's keys (one per asset
                // provider), read here from the keychain so none appears on a command line or in
                // a log.
                let providers = engine.session().ext.get::<craft_anything_engines::assets::AssetProviders>().map(|p| p.as_ref().clone()).unwrap_or_default();
                let keys = craft_anything_ai::keys::asset_keys(&cli.context.key_store(), &providers);
                engine.session().ext.insert(Arc::new(keys));
            }
            let mut opts = craft_anything_mcp::ServerOptions::from_env();
            // Share target metadata without reading original files in Clean room.
            let targets: Result<Vec<craft_anything_engine::target::TargetContext>, _> = targets.iter().map(|target| serde_json::from_str(target)).collect();
            let restored = targets
                .map_err(|error| ToolError::invalid(format!("Invalid shared target: {error}")))
                .and_then(|targets| engine.session().restore_targets(&targets));
            if let Err(error) = restored {
                errln!(io, "craft-anything mcp: {}", error.message);
                return 1;
            }
            opts.all_tools |= *all_tools;
            opts.restricted |= *restricted;
            opts.allow_web |= *allow_web;
            opts.allow_capture |= *allow_capture;
            opts.allow_run = cli.allow_run;
            match craft_anything_mcp::serve_stdio(engine, opts) {
                Ok(()) => 0,
                Err(e) => {
                    errln!(io, "craft-anything mcp: {e}");
                    1
                }
            }
        }
        Command::Setup(a) => setup_cmd::setup(io, &cli.context, a, f),
        Command::Doctor(a) => setup_cmd::doctor(io, &cli.context, a, f),
        Command::Uninstall(a) => setup_cmd::uninstall(io, &cli.context, a, f),
        Command::Commands => commands(io, &cli.context, f),
        Command::Capabilities => capabilities(io, &cli, f),
        Command::Triage { path } => triage(io, &cli, path, f),
        Command::Report { path, html, out, no_evidence } => {
            report(io, &cli, path.as_deref(), if *html { "html" } else { "markdown" }, out.as_deref(), *no_evidence, f)
        }
        Command::Chat(a) => ai_cmd::chat(io, &cli, a),
        Command::Rebuild(a) => rebuild_cmd::rebuild(io, &cli, a),
        Command::Ai { command } => ai_cmd::ai(io, &cli, command, f),
        Command::Tool(args) => tool(io, &cli, args, f),
    }
}

fn commands(io: &mut Io<'_>, context: &Context, f: Format) -> i32 {
    // The same engine every command gets: settings, the AI layer's tools and plugins.
    let engine = match make_engine_in(context, None, None) {
        Ok(e) => e,
        Err(e) => return fail(io, &e, f),
    };
    if f == Format::Json || f == Format::Jsonl {
        let v: Vec<Value> = engine
            .registry()
            .all()
            .iter()
            .map(|s| json!({"command": s.cli_name(), "tool": s.id, "toolset": s.toolset.id(), "title": s.title, "summary": s.summary, "needs_target": s.requires == Requires::Target}))
            .collect();
        io.emit(&Value::Array(v), f);
    } else {
        for t in engine.registry().toolsets() {
            let specs: Vec<_> = engine.registry().in_toolset(t).collect();
            if specs.is_empty() {
                continue;
            }
            io.line(&format!("{}:", t.title()));
            for s in specs {
                io.line(&format!("  {:<34} {}", s.cli_name(), s.title));
            }
        }
    }
    0
}

fn capabilities(io: &mut Io<'_>, cli: &Cli, f: Format) -> i32 {
    let engine = match make_engine_in(&cli.context, cli.project.as_deref(), cli.deep_for_run()) {
        Ok(e) => e,
        Err(e) => return fail(io, &e, f),
    };
    let session = engine.session();
    let toolsets: Vec<Value> = engine
        .registry()
        .toolsets()
        .iter()
        .map(|t| {
            let tools: Vec<Value> = engine
                .registry()
                .in_toolset(*t)
                .map(|s| {
                    let a = engine.availability(s, &session);
                    // Before anything is open a target tool is still offered (needs_target says what
                    // it waits for), but not one the mode refuses (#249).
                    let refused = session.clean_room() && !craft_anything_engine::clean_room::allows(s.id);
                    let waived = s.requires == Requires::Target && !refused;
                    json!({"tool": s.id, "available": a.available || waived, "needs_target": s.requires == Requires::Target, "reason": if waived { None } else { a.reason }})
                })
                .collect();
            json!({"id": t.id(), "title": t.title(), "tools": tools})
        })
        .collect();
    let engines: Vec<Value> = craft_anything_engines::ENGINES
        .iter()
        .map(|e| {
            let found = session.settings.engine_path(e.id);
            json!({"id": e.id, "name": e.name, "needed_for": e.needed_for, "found": found.is_some(), "path": found, "installable_here": e.host_pin().is_some()})
        })
        .collect();
    io.emit(
        &json!({
            "version": env!("CARGO_PKG_VERSION"),
            "analysis_mode": if session.clean_room() { "clean-room" } else { "deep" },
            "tools": engine.registry().len(),
            "toolsets": toolsets,
            "engines": engines,
            "host": craft_anything_evidence::host_triple(),
        }),
        f,
    );
    0
}

/// open → op → close for one target. `open` carries extra `core_target_open` params (raw-binary
/// architecture and entry point, a forced format, a Deep language pin).
fn with_target(
    engine: &Engine,
    cli: &Cli,
    cancel: &CancelToken,
    path: &str,
    open: &serde_json::Map<String, Value>,
    op: impl FnOnce() -> Result<ToolOutput, ToolError>,
) -> Result<ToolOutput, ToolError> {
    let abs = std::path::absolute(path).map_err(|e| ToolError::io(path, e))?;
    let mut params = open.clone();
    params.insert("path".into(), json!(abs));
    engine.call("core_target_open", Value::Object(params), call_options(cli, cancel))?;
    let out = op();
    let _ = engine.call("core_target_close", json!({}), call_options(cli, cancel));
    out
}

fn triage(io: &mut Io<'_>, cli: &Cli, path: &str, f: Format) -> i32 {
    let engine = match make_engine_in(&cli.context, cli.project.as_deref(), cli.deep_for_run()) {
        Ok(e) => e,
        Err(e) => return fail(io, &e, f),
    };
    let cancel = interrupt_token();
    let tool = ["core_target_triage", "core_overview_show"].into_iter().find(|t| engine.registry().get(t).is_some());
    let r = with_target(&engine, cli, &cancel, path, &Default::default(), || match tool {
        Some(t) => engine.call(t, json!({}), call_options(cli, &cancel)),
        None => {
            let mut v = engine.call("core_session_show", json!({}), call_options(cli, &cancel))?.to_json();
            if let Some(m) = v.as_object_mut() {
                m.insert("note".into(), json!("No triage tool is registered in this build yet; showing what was opened."));
            }
            Ok(ToolOutput::Value(v))
        }
    });
    match r {
        Ok(out) => {
            io.emit(&out.to_json(), f);
            0
        }
        Err(e) => fail(io, &e, f),
    }
}

fn report(io: &mut Io<'_>, cli: &Cli, path: Option<&str>, format: &str, out_file: Option<&Path>, no_evidence: bool, f: Format) -> i32 {
    let engine = match make_engine_in(&cli.context, cli.project.as_deref(), cli.deep_for_run()) {
        Ok(e) => e,
        Err(e) => return fail(io, &e, f),
    };
    let cancel = interrupt_token();
    // The report tool writes the file itself when given an absolute path; otherwise it returns the
    // text and we print it (raw, so the Markdown/HTML is usable directly).
    let out_abs = match out_file {
        Some(p) => match std::path::absolute(p) {
            Ok(a) => Some(a),
            Err(e) => return fail(io, &ToolError::io(p.display(), e), f),
        },
        None => None,
    };
    let mut params = json!({"format": format, "include_evidence": !no_evidence});
    if let (Some(a), Some(m)) = (&out_abs, params.as_object_mut()) {
        m.insert("path".into(), json!(a));
    }
    let run = || engine.call("core_report_write", params.clone(), call_options(cli, &cancel));
    let result = match path {
        Some(p) => with_target(&engine, cli, &cancel, p, &Default::default(), run),
        None => run(),
    };
    match result {
        Ok(output) => {
            let v = output.to_json();
            match (out_abs, v["result"]["report"].as_str()) {
                // Printed report: emit the raw text so it can be piped to a file or viewer.
                (None, Some(text)) => io.line(text),
                // Wrote a file (or an unexpected shape): show the structured result.
                _ => io.emit(&v, f),
            }
            0
        }
        Err(e) => fail(io, &e, f),
    }
}

/// Takes `--project <folder>` (or `--project=<folder>`) out of a tool's arguments, up to a `--`. It may be
/// given once, and its value is not another flag.
fn take_project(args: &mut Vec<String>) -> Result<Option<PathBuf>, UsageError> {
    let end = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let mut found = None;
    let mut i = 0;
    while i < end.min(args.len()) {
        let (len, value) = match args[i].as_str() {
            "--project" => match args.get(i + 1) {
                Some(v) if i + 1 < end && !v.starts_with('-') => (2, v.clone()),
                _ => return Err(UsageError::ProjectNeedsFolder),
            },
            a if a.starts_with("--project=") => (1, a["--project=".len()..].to_string()),
            _ => {
                i += 1;
                continue;
            }
        };
        if found.is_some() {
            return Err(tool_args::ToolArgsError::Duplicate("project".into()).into());
        }
        if value.is_empty() {
            return Err(UsageError::ProjectNeedsFolder);
        }
        found = Some(PathBuf::from(value));
        args.drain(i..i + len);
    }
    Ok(found)
}

fn tool(io: &mut Io<'_>, cli: &Cli, args: &[String], f: Format) -> i32 {
    let Some((name, rest)) = args.split_first() else { return usage(io, UsageError::MissingCommand) };
    // Global flags written after the tool name land here; pick them out, each at most once, up to
    // a `--` (after which everything is the tool's). `--format` and `--project` are left to
    // tool_args when the tool has an option of that name (`project-show --project <folder>`);
    // otherwise they're the global flags.
    let mut f = f;
    let (mut allow_run, mut deep, mut clean_room, mut compact) = (cli.allow_run, cli.deep, cli.clean_room, cli.compact);
    let mut seen = Vec::new();
    let mut rest_args = Vec::new();
    let mut it = rest.iter().peekable();
    while let Some(a) = it.next() {
        let step = match a.as_str() {
            "--" => {
                rest_args.push(a.clone());
                rest_args.extend(it.by_ref().cloned());
                break;
            }
            "--allow-run" => tool_args::once(&mut seen, "allow-run").map(|()| allow_run = true),
            "--deep" => tool_args::once(&mut seen, "deep").map(|()| deep = true),
            "--clean-room" => tool_args::once(&mut seen, "clean-room").map(|()| clean_room = true),
            "--compact" => tool_args::once(&mut seen, "compact").map(|()| compact = true),
            // Bare, `--json` is the shorthand for `--format json`; with parameters after it (`{`,
            // `@file` or `-`) it is the tool's own `--json` parameters, which tool_args reads.
            "--json" if it.peek().is_none_or(|n| !tool_args::is_json_params(n)) => tool_args::once(&mut seen, "json-format").map(|()| f = Format::Json),
            _ => {
                rest_args.push(a.clone());
                Ok(())
            }
        };
        if let Err(e) = step {
            return usage(io, e);
        }
    }
    io.pretty = !compact;
    // The engine first (tools from higher layers are only known through it), with the project
    // given before the tool name.
    if deep && clean_room {
        return usage(io, UsageError::DeepAndCleanRoom);
    }
    let mut cli = Cli {
        format: f,
        json: false,
        project: cli.project.clone(),
        allow_run,
        deep,
        clean_room,
        compact,
        log: cli.log,
        command: Command::Commands,
        context: cli.context.clone(),
    };
    let mut engine = match make_engine_in(&cli.context, cli.project.as_deref(), cli.deep_for_run()) {
        Ok(e) => e,
        Err(e) => return fail(io, &e, f),
    };
    let Some(spec) = engine.registry().get(name).copied() else {
        return usage(io, UsageError::UnknownCommand(name.clone()));
    };
    let schema = engine.registry().input_schema(&spec);
    let project_before = cli.project.clone();
    let tool_has_project = schema.pointer("/properties/project").is_some();
    if !tool_has_project {
        match take_project(&mut rest_args) {
            Ok(Some(dir)) => cli.project = Some(dir),
            Ok(None) => {}
            Err(e) => return usage(io, e),
        }
    }
    if !tool_has_project && cli.project.as_deref() != project_before.as_deref() {
        engine = match make_engine_in(&cli.context, cli.project.as_deref(), cli.deep_for_run()) {
            Ok(e) => e,
            Err(e) => return fail(io, &e, f),
        };
    }
    let cli = &cli;
    let rest = rest_args.as_slice();
    let inv = match tool_args::parse(&spec, &schema, rest) {
        Ok(i) => i,
        Err(e) => return usage(io, e),
    };
    if inv.version {
        io.line(&format!("craft-anything-cli {} {}", spec.cli_name(), env!("CARGO_PKG_VERSION")));
        return 0;
    }
    if let Some(v) = &inv.output_format {
        match <Format as clap::ValueEnum>::from_str(v, true) {
            Ok(v) => f = v,
            Err(_) => return usage(io, UsageError::OutputFormat),
        }
    }
    if inv.help {
        io.line(&tool_args::help(&spec, &schema));
        return 0;
    }
    let cancel = interrupt_token();
    let result = match (&inv.open_path, spec.requires) {
        (Some(path), _) => with_target(&engine, cli, &cancel, path, &inv.open_params, || engine.call(spec.id, inv.params.clone(), call_options(cli, &cancel))),
        (None, Requires::Target) => {
            return usage(io, UsageError::NeedsTarget { command: spec.cli_name().to_string() });
        }
        (None, _) => engine.call(spec.id, inv.params.clone(), call_options(cli, &cancel)),
    };
    match result {
        Ok(out) => {
            io.emit(&out.to_json(), f);
            0
        }
        Err(e) => fail(io, &e, f),
    }
}
