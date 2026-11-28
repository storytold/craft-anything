//! `rebuild`: the whole study → build → check workflow, headless. Questions and approvals are
//! answered on the terminal, or automatically with `--unattended` (first choice; approvals are
//! refused, so nothing outside the project happens without a person).

use crate::{Cli, INTERRUPTED, Io, make_engine_in};
use clap::Args;
use craft_anything_ai::rebuild::{self, RebuildRun, RunEvent, RunOptions};
use craft_anything_ai::{AgentEvent, AiHub, BackendKind};
use craft_anything_engine::inbox::{Answer, Request, RequestKind};
use craft_anything_engine::workspace::Workspace;
use craft_anything_evidence::ToolError;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[derive(Debug, Args)]
pub struct RebuildArgs {
    /// A web address, an absolute path to an app, game or file, or `-` for none. A folder that
    /// already holds a project keeps its saved target.
    pub target: String,
    /// What to make, in your words; a folder that already holds a project gets it as a follow-up.
    pub request: String,
    /// Where the new project folder goes (default: the projects folder from settings).
    #[arg(long, conflicts_with = "project_folder")]
    pub projects_dir: Option<PathBuf>,
    /// Start the new project in this folder itself (it's added to the project list). If it
    /// already has files, --existing says what they are; if it already holds a project, its saved
    /// target, settings and stack are kept and the request becomes a follow-up.
    #[arg(long, conflicts_with = "continue_in")]
    pub project_folder: Option<PathBuf>,
    /// With --project-folder that has files and isn't a project yet: `edit` (they're the app,
    /// edited in place) or `start-from` (material to start from; the app is built in app/ beside
    /// them).
    #[arg(long, requires = "project_folder", value_parser = ["edit", "start-from"])]
    pub existing: Option<String>,
    /// Continue an existing project folder; the request becomes a follow-up message.
    #[arg(long)]
    pub continue_in: Option<PathBuf>,
    /// With --continue-in: carry on where the project's last run stopped (paused, or the app or
    /// the computer stopped); the request is ignored.
    #[arg(long, requires = "continue_in")]
    pub resume: bool,
    /// The coding agent to use (an enabled backend name from `ai list`, or an installed agent
    /// such as `Claude Code`).
    #[arg(long)]
    pub backend: Option<String>,
    /// Refinement rounds after the first build.
    #[arg(long, default_value_t = 2)]
    pub rounds: u32,
    /// Answer questions with the first choice and refuse approvals, without asking.
    #[arg(long)]
    pub unattended: bool,
    /// What to build with: an engine (unity, unreal, godot, rust) or a stack's full name.
    /// New projects only; otherwise it's chosen from the request and target.
    #[arg(long)]
    pub build_with: Option<String>,
    /// Let the builder generate pictures and 3D models for a new game (needs a fal.ai key; paid
    /// within the project's asset budget). Change it later with `project-assets-allow`.
    #[arg(long)]
    pub generate_assets: bool,
}

pub fn rebuild(io: &mut Io<'_>, cli: &Cli, a: &RebuildArgs) -> i32 {
    match run(io, cli, a) {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(e) => {
            errln!(io, "rebuild: {}", e.message);
            if let Some(r) = e.remediation {
                errln!(io, "  {r}");
            }
            1
        }
    }
}

/// Add the agent app this run asks for, or the first one found when no enabled AI can build.
fn ensure_agent(io: &mut Io<'_>, hub: &AiHub, requested: Option<&str>) -> Result<(), ToolError> {
    let configured = hub.backends();
    // An enabled backend can build, so a run that names none needs no agent app.
    if requested.is_none() && configured.iter().any(|b| b.enabled) {
        return Ok(());
    }
    // A name that is configured is left alone (including a backend that is off): don't add a
    // second one under the same name; the run itself refuses a backend that is off.
    if requested.is_some_and(|name| configured.iter().any(|b| b.name == name)) {
        return Ok(());
    }
    // An explicit name may be an installed agent that isn't added yet: add only that one. With no
    // name, add the first agent found.
    if let Some(d) = hub.detect().into_iter().find(|d| d.suggestion.kind == BackendKind::AgentCli && requested.is_none_or(|name| d.suggestion.name == name)) {
        errln!(io, "Using {} ({}).", d.suggestion.name, d.reason);
        hub.add_backend(d.suggestion, None)?;
    }
    Ok(())
}

/// The `rebuild` command: open or make the project, start the run, and print its events until it ends.
fn run(io: &mut Io<'_>, cli: &Cli, a: &RebuildArgs) -> Result<bool, ToolError> {
    let engine = make_engine_in(&cli.context, None, cli.deep_for_run())?;
    let hub = cli.context.hub(engine.clone());
    // Before the build starts, while no session lock is held (#1225).
    if let Err(e) = hub.ensure_session_id() {
        log::warn!("couldn't save the OpenCode Go session id: {e}");
    }
    if a.build_with.is_some() && a.continue_in.is_some() {
        return Err(ToolError::invalid("--build-with is for new projects; a project keeps the stack it was built with.")
            .with_remediation("Leave out --build-with, or start a new project."));
    }
    if a.generate_assets && a.continue_in.is_some() {
        return Err(ToolError::invalid("--generate-assets is for new projects.").with_remediation("Use project-assets-allow for an existing project."));
    }
    ensure_agent(io, &hub, a.backend.as_deref())?;
    // Refuse an unknown or switched-off name before opening or making a project: a command that
    // can't build must not leave a new folder behind.
    if let Some(name) = a.backend.as_deref() {
        rebuild::builder_backend(&hub.config(), Some(name))?;
    }
    let (ws, message) = match &a.continue_in {
        Some(dir) => (Workspace::open(&std::path::absolute(dir).map_err(|e| ToolError::io(dir.display(), e))?)?, (!a.resume).then(|| a.request.clone())),
        None => {
            use craft_anything_ai::rebuild::project::Place;
            use craft_anything_engine::workspace::ExistingFiles;
            let settings = engine.session().settings.clone();
            let existing = a.existing.as_deref().map(|e| if e == "edit" { ExistingFiles::Edit } else { ExistingFiles::StartFrom });
            let folder = a.project_folder.as_deref().map(std::path::absolute).transpose().map_err(|e| ToolError::io("--project-folder", e))?;
            let attached = folder.as_ref().is_some_and(|f| rebuild::project::has_project(f));
            if attached {
                if a.existing.is_some() {
                    return Err(ToolError::invalid("This folder already holds a project; use --continue-in to continue it."));
                }
                if a.build_with.is_some() {
                    return Err(ToolError::invalid("--build-with is for new projects; a project keeps the stack it was built with.")
                        .with_remediation("Leave out --build-with, or start a new project."));
                }
                if a.generate_assets {
                    return Err(
                        ToolError::invalid("--generate-assets is for new projects.").with_remediation("Use project-assets-allow for an existing project.")
                    );
                }
            }
            let place = match (&folder, &a.projects_dir) {
                (Some(f), _) => Place::At(f, existing),
                (None, Some(d)) => Place::In(d),
                (None, None) => Place::Projects,
            };
            let ws = rebuild::project::new_project(&settings, &a.target, &a.request, place, a.build_with.as_deref(), a.generate_assets, a.backend.as_deref())?;
            if folder.is_some() {
                engine.session().remember_project(&ws.root)?;
            }
            (ws, rebuild::project::attached_request(attached, &a.request))
        }
    };
    outln!(io, "Project: {}", ws.root.display());
    // The saved builder may be off or gone; say who builds once the run has started, so a failed
    // start doesn't claim an AI that never ran.
    let replacement = rebuild::builder_replacement(&hub.config(), &ws, a.backend.as_deref());
    // A follow-up shows in the project's feed as the person typed it.
    let shown = message.clone();
    let good_enough = engine.session().settings.build.good_enough;
    let opts = RunOptions {
        backend: a.backend.clone(),
        message,
        shown,
        continuing: a.resume,
        max_rounds: a.rounds,
        good_enough,
        beginner: !cli.json,
        clean_room: Some(engine.session().clean_room()),
        ..RunOptions::default()
    };
    let mut run = RebuildRun::start(&hub, ws, opts)?;
    if let Some(replacement) = replacement {
        errln!(io, "{}", rebuild::replaced_builder_notice(&replacement.saved, &replacement.used));
    }
    let mut ok = false;
    let mut line_open = false;
    loop {
        if INTERRUPTED.load(Ordering::SeqCst) {
            run.stop();
        }
        for e in run.poll() {
            match e {
                RunEvent::Step { step, text } => {
                    end_line(io, &mut line_open);
                    outln!(io, "[{}] {text}", step.label());
                }
                RunEvent::Agent(AgentEvent::TextDelta { text }) => {
                    outp!(io, "{text}");
                    line_open = !text.ends_with('\n');
                }
                RunEvent::Agent(AgentEvent::ToolStarted { tool, input, .. }) => {
                    end_line(io, &mut line_open);
                    outln!(io, "  · {}", rebuild::activity::describe(&tool, &input, &[]).words());
                }
                RunEvent::Agent(AgentEvent::Error { message, remediation, log }) => {
                    end_line(io, &mut line_open);
                    outln!(io, "  ! {message}");
                    if let Some(r) = remediation {
                        outln!(io, "{r}");
                    }
                    if let Some(l) = log {
                        outln!(io, "  log: {}", l.display());
                    }
                }
                RunEvent::Agent(_) => {}
                RunEvent::Note { shown, .. } => {
                    end_line(io, &mut line_open);
                    outln!(io, "  + {shown}");
                }
                RunEvent::Ask(r) => {
                    end_line(io, &mut line_open);
                    let answer = if a.unattended { auto_answer(io, &r) } else { prompt_answer(io, &cli.context, &r) };
                    run.answer(&r.id, &answer)?;
                }
                RunEvent::Checked(c) => {
                    end_line(io, &mut line_open);
                    match c.similarity {
                        Some(s) => outln!(io, "[Check {}] {s:.0}% alike — {}", c.round, c.note),
                        None => outln!(io, "[Check {}] {}", c.round, c.note),
                    }
                }
                RunEvent::Preview(url) => outln!(io, "Preview: {url}"),
                RunEvent::Finished { ok: o, message, paused } => {
                    end_line(io, &mut line_open);
                    outln!(
                        io,
                        "{}",
                        if o {
                            format!("Done. {message}")
                        } else if paused {
                            message
                        } else {
                            format!("Stopped: {message}")
                        }
                    );
                    ok = o;
                }
            }
        }
        if !run.is_running() && run.poll().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(ok)
}

fn end_line(io: &mut Io<'_>, open: &mut bool) {
    if *open {
        outln!(io);
        *open = false;
    }
}

pub(crate) fn auto_answer(io: &mut Io<'_>, r: &Request) -> Answer {
    match r.kind {
        RequestKind::Question => {
            outln!(io, "? {} → {} (unattended)", r.title, r.choices.first().map(String::as_str).unwrap_or("no preference"));
            Answer {
                choice: r.choices.first().cloned(),
                text: r.choices.is_empty().then(|| "No preference; use your best judgement.".to_string()),
                ..Default::default()
            }
        }
        RequestKind::Approval => {
            outln!(io, "? {} → refused (unattended)", r.title);
            Answer { allow: Some(false), text: Some("Unattended run: actions outside the project aren't allowed.".into()), ..Default::default() }
        }
    }
}

fn read_line(context: &crate::Context) -> String {
    context.read_line().trim().to_string()
}

pub(crate) fn prompt_answer(io: &mut Io<'_>, context: &crate::Context, r: &Request) -> Answer {
    outln!(io, "\n? {}", r.title);
    if !r.detail.is_empty() {
        outln!(io, "  {}", r.detail.lines().take(12).collect::<Vec<_>>().join("\n  "));
    }
    match r.kind {
        RequestKind::Question => {
            for (i, c) in r.choices.iter().enumerate() {
                outln!(io, "  {}) {c}", i + 1);
            }
            outp!(io, "Your answer (number or text): ");
            let line = read_line(context);
            match line.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|i| r.choices.get(i)) {
                Some(c) => Answer { choice: Some(c.clone()), ..Default::default() },
                None => Answer { text: Some(line), ..Default::default() },
            }
        }
        RequestKind::Approval => {
            outp!(io, "Allow? [y]es / [a]lways for this project / [n]o: ");
            let line = read_line(context).to_lowercase();
            Answer { allow: Some(line.starts_with('y') || line.starts_with('a')), always: line.starts_with('a'), ..Default::default() }
        }
    }
}
