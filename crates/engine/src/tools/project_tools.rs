//! Projects (what the person is making) as tools, so every client — the app, the CLI, an MCP
//! agent — can list them, read one with its conversation, and answer its questions without the
//! app. Ids follow docs/tool-ids.md.

use crate::feed;
use crate::inbox::{Answer, Inbox};
use crate::tool::{Ctx, Effects, Requires, ToolOutput, ToolSpec, Toolset, parse_params, schema_of};
use crate::workspace::{ACTION_KINDS, ActionKind, Approval, USAGE_FILE, Workspace};
use craft_anything_evidence::{ErrorCode, ToolError};
use craft_anything_platform::sync::Mutex;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, PoisonError};

/// Most feed items one call returns.
const MAX_ITEMS: usize = 200;

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            id: "project_list",
            title: "List projects",
            summary: "List the projects in every project folder from settings (where new projects go, then `project_folders`), or in one folder, newest first: name, folder, app folder, request, target, stack, status, the latest check and how many questions are waiting.",
            beginner: "Shows everything you're making.",
            toolset: Toolset::Projects,
            effects: Effects::READ_ONLY,
            requires: Requires::Nothing,
            input_schema: schema_of::<ListParams>,
            run: project_list,
        },
        ToolSpec {
            id: "project_show",
            title: "Show a project",
            summary: "One project: its request, target, stack, status, checks, questions still waiting for an answer, and its conversation (paged with offset/limit, newest last).",
            beginner: "Shows one project and what happened in it.",
            toolset: Toolset::Projects,
            effects: Effects::READ_ONLY,
            requires: Requires::Nothing,
            input_schema: schema_of::<ShowParams>,
            run: project_show,
        },
        ToolSpec {
            id: "project_inbox_answer",
            title: "Answer a project's question",
            summary: "Answer a question or approval a project is waiting on (an id from project_show): a choice or text for questions, allow (and always) for approvals. The answer reaches a running build at once and is kept in the project's conversation.",
            beginner: "Answers a question the builder asked you.",
            toolset: Toolset::Projects,
            effects: Effects::WRITES_FILESYSTEM.union(Effects::CHANGES_UI_STATE),
            requires: Requires::Nothing,
            input_schema: schema_of::<AnswerParams>,
            run: project_inbox_answer,
        },
        ToolSpec {
            id: "project_play",
            title: "Play a project's result",
            summary: "Start the finished game or app a project built (its program, a Unity or Unreal package, or a Godot project in Godot), in its own window. If it's already running, says so instead of starting a second copy, unless `another` is true. A start that quits within two seconds is reported with its exit code and last output. It runs the code the builder wrote, so it needs the person's consent. Websites use project_preview instead. Known limit on Windows: when the caller's output is a pipe, the started program inherits it, so a caller reading that output to its end waits until the program exits (#207).",
            beginner: "Opens the game or app that was made.",
            toolset: Toolset::Projects,
            effects: Effects::LAUNCHES_PROCESS.union(Effects::EXECUTES_FOREIGN_CODE),
            requires: Requires::Nothing,
            input_schema: schema_of::<PlayParams>,
            run: project_play,
        },
        ToolSpec {
            id: "project_preview",
            title: "Preview a project's website",
            summary: "Serve a project's built website on this computer only (loopback) and return its address. It keeps serving for this session; with `wait`, the call itself serves until it's cancelled (Ctrl+C in the CLI).",
            beginner: "Shows the website that was made.",
            toolset: Toolset::Projects,
            effects: Effects::MUTATES_SESSION,
            requires: Requires::Nothing,
            input_schema: schema_of::<PreviewParams>,
            run: project_preview,
        },
        ToolSpec {
            id: "project_open_folder",
            title: "Open a project's folder",
            summary: "Open the project's folder (or its app/ folder) in the file manager, or one of its pinned builds' folders.",
            beginner: "Opens the project's files.",
            toolset: Toolset::Projects,
            effects: Effects::LAUNCHES_PROCESS,
            requires: Requires::Nothing,
            input_schema: schema_of::<FolderParams>,
            run: project_open_folder,
        },
        ToolSpec {
            id: "project_rename",
            title: "Rename a project",
            summary: "Give a project a new name (its folder stays where it is).",
            beginner: "Renames something you're making.",
            toolset: Toolset::Projects,
            effects: Effects::WRITES_FILESYSTEM,
            requires: Requires::Nothing,
            input_schema: schema_of::<RenameParams>,
            run: project_rename,
        },
        ToolSpec {
            id: "project_plan_fold",
            title: "Fold a project's plan",
            summary: "Fold the builder's plan card away (#504). The steps stay in the project's conversation, and a later run's plan shows again. Call it between builds: a build saves its conversation as it goes, so a fold made during one can be overwritten by that build's next save.",
            beginner: "Hides the builder's checklist until it makes a new one.",
            toolset: Toolset::Projects,
            effects: Effects::WRITES_FILESYSTEM,
            requires: Requires::Nothing,
            input_schema: schema_of::<FoldPlanParams>,
            run: project_plan_fold,
        },
        ToolSpec {
            id: "project_duplicate",
            title: "Duplicate a project",
            summary: "Copy a project beside it under a new name, to try something else from where it is: what was built, the pictures of the original and the study come along; the conversation, questions and usage don't, and neither do dependencies or compiler output (the next build makes them again). An app edited in place is copied into the copy's app/ folder.",
            beginner: "Makes a copy to try something different.",
            toolset: Toolset::Projects,
            effects: Effects::WRITES_FILESYSTEM,
            requires: Requires::Nothing,
            input_schema: schema_of::<DuplicateParams>,
            run: project_duplicate,
        },
        ToolSpec {
            id: "project_delete",
            title: "Delete a project",
            summary: "Delete a project: everything Craft Anything made in its folder (its records, checks, and what was built in app/). The person's own files stay (an app edited in place, the files a project started from). Can't be undone, so `confirm` must be true; stop a build first.",
            beginner: "Deletes something you made. Your own files are kept.",
            toolset: Toolset::Projects,
            effects: Effects::WRITES_FILESYSTEM,
            requires: Requires::Nothing,
            input_schema: schema_of::<DeleteParams>,
            run: project_delete,
        },
        ToolSpec {
            id: "project_questions_set",
            title: "Choose what the builder's questions get",
            summary: "Choose whether the builder's questions (core_person_ask) in a project are shown to the person to answer (`ask`) or answered with \"decide yourself and say what you chose\" (`decide`); `default` follows the app's setting (`questions` in settings, which follows Auto mode until set). With no mode, says what applies now. Takes effect from the builder's next question, also mid-build.",
            beginner: "Chooses whether the builder asks you or decides by itself.",
            toolset: Toolset::Projects,
            effects: Effects::WRITES_FILESYSTEM,
            requires: Requires::Nothing,
            input_schema: schema_of::<QuestionsParams>,
            run: project_questions_set,
        },
        ToolSpec {
            id: "project_approvals_set",
            title: "Choose what the builder's approvals get",
            summary: "Choose, for one kind of builder action in a project, whether it goes ahead (`allow`), waits for the person's OK on a card (`ask`) or is refused (`refuse`); `default` follows Auto mode. Kinds: `web` (reading websites, searching), `commands` (outside the usual build commands), `outside_files` (changing files outside the project), `publish` (git push, npm publish, gh pr create…), `delete_outside`, `system` (system changes and files, Craft Anything's own records), `other_tools`. With no kind, says what every kind gets now. Takes effect from the builder's next action, also mid-build. Only the person changes these: an agent's call may only make a kind stricter.",
            beginner: "Chooses what the builder may do without asking you.",
            toolset: Toolset::Projects,
            effects: Effects::WRITES_FILESYSTEM,
            requires: Requires::Nothing,
            input_schema: schema_of::<ApprovalsParams>,
            run: project_approvals_set,
        },
        ToolSpec {
            id: "project_build_finish",
            title: "Call the build done",
            summary: "The person is happy with the build as it is (#210): its status becomes Ready and the conversation says they called it done, so a paused, stopped or failed build no longer offers to carry on. Everything so far is kept, and a new message carries on from here. It doesn't stop a build that is still running: pause it first.",
            beginner: "Keeps the build as it is and stops offering to carry on.",
            toolset: Toolset::Projects,
            effects: Effects::WRITES_FILESYSTEM,
            requires: Requires::Nothing,
            input_schema: schema_of::<FinishParams>,
            run: project_build_finish,
        },
        ToolSpec {
            id: "project_assets_budget",
            title: "Set a project's asset budget",
            summary: "Set what a project may spend on generated pictures and 3D models (US dollars), or read it with no amount. Shows what's been spent so far.",
            beginner: "Changes how much a project may spend on generated art.",
            toolset: Toolset::Projects,
            effects: Effects::WRITES_FILESYSTEM,
            requires: Requires::Nothing,
            input_schema: schema_of::<BudgetParams>,
            run: project_assets_budget,
        },
        ToolSpec {
            id: "project_assets_allow",
            title: "Let a project generate assets",
            summary: "Turn on or off whether the builder may generate pictures and 3D models for a project (paid per asset from its asset budget, and only with an asset provider's key), or read it with no value. Off until the person turns it on.",
            beginner: "Chooses whether your game may get generated pictures and 3D models.",
            toolset: Toolset::Projects,
            effects: Effects::WRITES_FILESYSTEM,
            requires: Requires::Nothing,
            input_schema: schema_of::<AllowAssetsParams>,
            run: project_assets_allow,
        },
    ]
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameParams {
    /// The project's folder.
    pub project: PathBuf,
    /// Its new name.
    pub name: String,
}

fn project_rename(_ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: RenameParams = parse_params(params)?;
    let mut ws = open(&p.project)?;
    ws.rename(&p.name)?;
    Ok(ToolOutput::Value(json!({"project": ws.root, "name": ws.manifest.name})))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FoldPlanParams {
    /// The project's folder.
    pub project: PathBuf,
}

fn project_plan_fold(_ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: FoldPlanParams = parse_params(params)?;
    let ws = open(&p.project)?;
    // `folded` is false when there's no plan to fold, or it's folded already.
    let folded = crate::feed::dismiss_plan_saved(&ws)?;
    Ok(ToolOutput::Value(json!({"project": ws.root, "folded": folded})))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DuplicateParams {
    /// The project's folder.
    pub project: PathBuf,
    /// The copy's name (default: "<name> copy").
    #[serde(default)]
    pub name: Option<String>,
}

fn project_duplicate(_ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: DuplicateParams = parse_params(params)?;
    let ws = open(&p.project)?;
    let name = p.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("{} copy", ws.manifest.name));
    let copy = ws.duplicate(&name)?;
    let mut result = json!({"project": copy.root, "name": copy.manifest.name});
    if let Some(warning) = copy.creation_warning {
        result["creation_warning"] = warning.into();
    }
    Ok(ToolOutput::Value(result))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteParams {
    /// The project's folder.
    pub project: PathBuf,
    /// Must be true: deleting can't be undone.
    #[serde(default)]
    pub confirm: bool,
}

fn project_delete(_ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: DeleteParams = parse_params(params)?;
    if !p.confirm {
        return Err(ToolError::invalid("Deleting a project can't be undone.").with_remediation("Pass confirm: true to delete it."));
    }
    let ws = open(&p.project)?;
    let kept = ws.manifest.app_in_root || !ws.manifest.start_files.is_empty();
    ws.delete()?;
    Ok(ToolOutput::Value(json!({"deleted": p.project, "your_files_kept": kept})))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BudgetParams {
    /// The project's folder.
    pub project: PathBuf,
    /// The new budget in US dollars (0 to 10 000); leave out to only read it.
    #[serde(default)]
    pub budget_usd: Option<f64>,
}

fn project_assets_budget(_ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: BudgetParams = parse_params(params)?;
    let mut ws = open(&p.project)?;
    if let Some(b) = p.budget_usd {
        if !b.is_finite() || !(0.0..=10_000.0).contains(&b) {
            return Err(ToolError::invalid("budget_usd must be between 0 and 10000."));
        }
        ws.set_person(|m| m.asset_budget_usd = b)?;
    }
    Ok(ToolOutput::Value(json!({"budget_usd": ws.manifest.asset_budget_usd, "spent_usd": ws.assets_spent()})))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AllowAssetsParams {
    /// The project's folder.
    pub project: PathBuf,
    /// true lets the builder generate assets, false stops it; leave out to only read it.
    #[serde(default)]
    pub allow: Option<bool>,
}

fn project_assets_allow(_ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: AllowAssetsParams = parse_params(params)?;
    let mut ws = open(&p.project)?;
    if let Some(allow) = p.allow {
        ws.set_person(|m| m.generate_assets = allow)?;
    }
    Ok(ToolOutput::Value(json!({"generate_assets": ws.manifest.generate_assets})))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectParams {
    /// The project's folder.
    pub project: PathBuf,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewParams {
    /// The project's folder.
    pub project: PathBuf,
    /// Serve until the call is cancelled instead of returning at once (for the CLI).
    #[serde(default)]
    pub wait: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FolderParams {
    /// The project's folder.
    pub project: PathBuf,
    /// Open the app/ folder (what was built) instead of the project folder.
    #[serde(default)]
    pub app: bool,
    /// Open a pinned build's folder.
    #[serde(default)]
    pub pin: Option<u32>,
}

/// Websites being previewed in this session, by project folder (kept serving until it ends).
#[derive(Default)]
struct Previews(Mutex<HashMap<PathBuf, craft_anything_platform::serve::StaticServer>>);

/// What a project's questions should get.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuestionsChoice {
    Ask,
    Decide,
    /// The app's setting.
    Default,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FinishParams {
    /// The project's folder.
    pub project: PathBuf,
}

/// What the conversation and the project say when the person calls a build done.
pub const CALLED_DONE: &str = "Ready: you called it done. Everything so far is kept; a new message carries on from here.";

fn project_build_finish(_ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: FinishParams = parse_params(params)?;
    finish_build_after_feed_save(open(&p.project)?, |_| {})
}

fn finish_build_after_feed_save(mut ws: Workspace, after_feed_save: impl FnOnce(&Workspace)) -> Result<ToolOutput, ToolError> {
    use crate::workspace::{Status, Step};
    ws.ensure_writable()?;
    let mut items = feed::load(&ws);
    // A previous attempt may have saved the conversation before the status write failed.
    if !matches!(items.last(), Some(feed::FeedItem::Step { step: Step::Done, text }) if text == CALLED_DONE) {
        items.push(feed::FeedItem::Step { step: Step::Done, text: CALLED_DONE.into() });
    }
    feed::save(&ws.root, &mut items)?;
    after_feed_save(&ws);
    ws.set_status(Status::Ready, CALLED_DONE)?;
    Ok(ToolOutput::Value(json!({"project": ws.root, "status": ws.manifest.status})))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QuestionsParams {
    /// The project's folder.
    pub project: PathBuf,
    /// `ask`, `decide` or `default`; none just reads what applies.
    #[serde(default)]
    pub mode: Option<QuestionsChoice>,
}

fn project_questions_set(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    use crate::workspace::Questions;
    let p: QuestionsParams = parse_params(params)?;
    let mut ws = open(&p.project)?;
    if let Some(mode) = p.mode {
        let chosen = match mode {
            QuestionsChoice::Ask => Some(Questions::Ask),
            QuestionsChoice::Decide => Some(Questions::Decide),
            QuestionsChoice::Default => None,
        };
        ws.set_person(|m| m.questions = chosen)?;
    }
    let settings = &ctx.session.settings;
    Ok(ToolOutput::Value(json!({
        "project": ws.root,
        "questions": settings.questions_in(Some(&ws.manifest)),
        "chosen": ws.manifest.questions,
        "app_default": settings.questions(),
    })))
}

/// What one kind of a project's builder actions should get.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalChoice {
    Allow,
    Ask,
    Refuse,
    /// What Auto mode gives it.
    Default,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApprovalsParams {
    /// The project's folder.
    pub project: PathBuf,
    /// The kind of action (`publish`, `web`…); none just reads what every kind gets.
    #[serde(default)]
    pub kind: Option<ActionKind>,
    /// `allow`, `ask`, `refuse` or `default`; needed with `kind`.
    #[serde(default)]
    pub mode: Option<ApprovalChoice>,
}

fn project_approvals_set(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: ApprovalsParams = parse_params(params)?;
    let mut ws = open(&p.project)?;
    let auto_mode = ctx.session.settings.auto_mode;
    let mut set = None;
    match (p.kind, p.mode) {
        (Some(kind), Some(mode)) => {
            set = Some(kind);
            let chosen = match mode {
                ApprovalChoice::Allow => Some(Approval::Allow),
                ApprovalChoice::Ask => Some(Approval::Ask),
                ApprovalChoice::Refuse => Some(Approval::Refuse),
                ApprovalChoice::Default => None,
            };
            // A builder (or any agent) can't let itself do more: what it's held to is the person's
            // to loosen, from the app or the CLI. Checked with Auto mode on and off, so switching it
            // later doesn't hand over what the agent changed (an unchosen kind set to its default
            // now, or a refusal given back to a default that asks).
            let agent = matches!(ctx.caller, crate::Caller::Mcp | crate::Caller::Agent);
            let looser = [true, false]
                .into_iter()
                .any(|auto| chosen.unwrap_or_else(|| kind.default_in(auto)).strictness() < ws.manifest.approval(kind, auto).strictness());
            if agent && looser {
                return Err(ToolError::new(ErrorCode::NoPermission, format!("An agent can't loosen what \"{}\" gets in this project.", kind.name()))
                    .with_remediation("Ask the person to change it in the app (the project's approvals) or with the CLI."));
            }
            ws.set_person(|m| match chosen {
                Some(a) => {
                    m.approvals.insert(kind, a);
                }
                None => {
                    m.approvals.remove(&kind);
                }
            })?;
        }
        (Some(_), None) => return Err(ToolError::invalid("mode is needed with kind: allow, ask, refuse or default")),
        (None, Some(_)) => return Err(ToolError::invalid("kind is needed with mode")),
        (None, None) => {}
    }
    let kinds: Vec<Value> = ACTION_KINDS
        .iter()
        .map(|r| {
            json!({
                "kind": r.kind,
                "what": r.what,
                "gets": ws.manifest.approval(r.kind, auto_mode),
                "chosen": ws.manifest.approvals.get(&r.kind),
                "default": r.kind.default_in(auto_mode),
            })
        })
        .collect();
    // The kind just set and what it gets now, so a client can say what changed.
    let set = set.map(|k| json!({"kind": k, "gets": ws.manifest.approval(k, auto_mode), "chosen": ws.manifest.approvals.get(&k)}));
    Ok(ToolOutput::Value(json!({"project": ws.root, "auto_mode": auto_mode, "set": set, "kinds": kinds})))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlayParams {
    /// The project's folder.
    pub project: PathBuf,
    /// Start another copy even if one is running.
    #[serde(default)]
    pub another: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListParams {
    /// Only this folder (default: every project folder from settings).
    #[serde(default)]
    pub projects_dir: Option<PathBuf>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShowParams {
    /// The project's folder.
    pub project: PathBuf,
    /// Skip this many conversation items from the start.
    #[serde(default)]
    pub offset: Option<usize>,
    /// Return at most this many items (default and maximum 200; the newest when no offset).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnswerParams {
    /// The project's folder.
    pub project: PathBuf,
    /// The question's or approval's id.
    pub id: String,
    /// The chosen suggestion (questions).
    #[serde(default)]
    pub choice: Option<String>,
    /// A typed answer (questions), or a reason (approvals).
    #[serde(default)]
    pub text: Option<String>,
    /// Allow the action (approvals).
    #[serde(default)]
    pub allow: Option<bool>,
    /// Allow this kind of action for the rest of the project (approvals).
    #[serde(default)]
    pub always: bool,
}

fn open(project: &std::path::Path) -> Result<Workspace, ToolError> {
    Workspace::open(project).map_err(|e| {
        let why = e.message.clone();
        ToolError::new(ErrorCode::NotFound, format!("{} isn't a project folder: {why}", project.display()))
            .with_remediation("Pass a folder from project_list (it holds craft.json).")
    })
}

fn summary(ws: &Workspace) -> Value {
    let m = &ws.manifest;
    // An unreadable usage.json is never shown as zeros: `usage` is null and `usage_unreadable` says why.
    let usage = ws.usage();
    let usage_unreadable = usage.is_unreadable();
    let mut out = json!({
        "name": m.name,
        "folder": ws.root.display().to_string(),
        "app_folder": ws.app_dir().display().to_string(),
        "request": m.request,
        "target": m.target,
        "stack": m.stack,
        "status": m.status,
        "status_label": m.status.label(),
        "note": m.note,
        "updated_ms": m.updated_ms,
        "last_check": m.checks.last(),
        "waiting": Inbox::new(ws.inbox_dir()).pending().len(),
        "usage": usage.known(),
        "usage_unreadable": usage_unreadable,
    });
    if usage_unreadable && let Some(o) = out.as_object_mut() {
        o.insert("usage_note".into(), json!(format!("The project's {USAGE_FILE} couldn't be read, so what its builds used and spent isn't known.")));
    }
    out
}

fn project_list(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: ListParams = parse_params(params)?;
    let settings = &ctx.session.settings;
    let locations = p.projects_dir.map_or_else(|| settings.project_locations(), |d| vec![d]);
    let projects: Vec<Value> = Workspace::list_in(&locations).iter().map(summary).collect();
    Ok(ToolOutput::Value(json!({
        "projects_dir": settings.projects_dir().display().to_string(),
        "locations": locations.iter().map(|l| l.display().to_string()).collect::<Vec<_>>(),
        "count": projects.len(),
        "projects": projects,
    })))
}

fn project_show(_ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: ShowParams = parse_params(params)?;
    let ws = open(&p.project)?;
    let items = feed::for_display(&ws);
    let limit = p.limit.unwrap_or(MAX_ITEMS).clamp(1, MAX_ITEMS);
    let offset = p.offset.unwrap_or_else(|| items.len().saturating_sub(limit)).min(items.len());
    let page: Vec<&feed::FeedItem> = items.iter().skip(offset).take(limit).collect();
    let mut out = summary(&ws);
    if let Some(o) = out.as_object_mut() {
        o.insert("checks".into(), json!(ws.manifest.checks));
        o.insert("last_reply".into(), json!(ws.manifest.last_reply));
        o.insert("questions".into(), json!(Inbox::new(ws.inbox_dir()).pending()));
        o.insert("conversation".into(), json!({"total": items.len(), "offset": offset, "items": page}));
    }
    Ok(ToolOutput::Value(out))
}

fn project_inbox_answer(_ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: AnswerParams = parse_params(params)?;
    let ws = open(&p.project)?;
    let inbox = Inbox::new(ws.inbox_dir());
    let answer = Answer { choice: p.choice, text: p.text, allow: p.allow, always: p.always };
    let shown = match answer.allow {
        Some(true) if answer.always => "Always allowed".to_string(),
        Some(true) => "Allowed".to_string(),
        Some(false) => "Not allowed".to_string(),
        None => answer.as_text(),
    };
    inbox.answer_recorded(&ws, &p.id, &answer, &shown)?;
    Ok(ToolOutput::Value(json!({"answered": p.id, "shown": shown})))
}

fn project_play(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: PlayParams = parse_params(params)?;
    let ws = open(&p.project)?;
    let app = ws.app_dir();
    if crate::outputs::site_dir(&app).is_some() {
        return Err(ToolError::new(ErrorCode::NotSupported, "This project made a website.").with_remediation("Use project_preview to see it."));
    }
    let out = super::pin_tools::start(ctx, &app, &crate::pins::play_log(&ws), p.another, "the finished project")?;
    Ok(ToolOutput::Value(out))
}

fn project_preview(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: PreviewParams = parse_params(params)?;
    let ws = open(&p.project)?;
    let Some(site) = crate::outputs::site_dir(&ws.app_dir()) else {
        return Err(ToolError::new(ErrorCode::NotFound, "There's no built website in this project yet.")
            .with_remediation("Finish a build first (rebuild); a game or app is started with project_play."));
    };
    let previews = match ctx.session.ext.get::<Previews>() {
        Some(p) => p,
        None => {
            let p = Arc::new(Previews::default());
            ctx.session.ext.insert(p.clone());
            p
        }
    };
    let url = {
        let mut map = previews.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(server) = map.get(&ws.root) {
            server.url()
        } else {
            let server = craft_anything_platform::serve::serve_dir(&site).map_err(|e| ToolError::io(site.display(), e))?;
            let url = server.url();
            map.insert(ws.root.clone(), server);
            url
        }
    };
    if p.wait {
        ctx.report(None, format!("Serving {url} (stop to end)"));
        while !ctx.cancel.is_cancelled() {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        previews.0.lock().unwrap_or_else(PoisonError::into_inner).remove(&ws.root);
    }
    Ok(ToolOutput::Value(json!({"url": url, "site": site.display().to_string()})))
}

fn project_open_folder(_ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: FolderParams = parse_params(params)?;
    let ws = open(&p.project)?;
    let dir = folder_to_open(&ws, p.app, p.pin)?;
    craft_anything_platform::open_path(&dir)?;
    Ok(ToolOutput::Value(json!({"opened": dir.display().to_string()})))
}

/// The folder `project_open_folder` opens: the project, its app, or a pinned build's copy.
fn folder_to_open(ws: &Workspace, app: bool, pin: Option<u32>) -> Result<PathBuf, ToolError> {
    Ok(match (pin, app) {
        (Some(id), app) => {
            let pin = crate::pins::get(ws, id)?;
            if app { pin.app_dir(ws) } else { pin.dir(ws) }
        }
        (None, true) => ws.app_dir(),
        (None, false) => ws.root.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbox::RequestKind;
    use crate::workspace::TargetRef;
    use crate::{CallOptions, Caller, Engine};

    /// #210: "enough now". A paused build becomes Ready, says so in its conversation, and keeps
    /// everything it had.
    #[test]
    fn a_build_called_done_is_ready_and_says_so() {
        use crate::workspace::{Status, Step};
        let d = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(d.path(), "Site", "A website", TargetRef::None).unwrap();
        ws.set_status(Status::Stopped, "Paused.").unwrap();
        let e = Engine::new(crate::testing::session());
        let r = e.call("project_build_finish", json!({"project": ws.root}), CallOptions::new(Caller::Cli)).unwrap().to_json();
        assert_eq!(r["result"]["status"], "ready", "{r}");
        let ws = Workspace::open(&ws.root).unwrap();
        assert_eq!((ws.manifest.status, ws.manifest.note.as_str()), (Status::Ready, CALLED_DONE));
        let items = feed::load(&ws);
        assert!(
            matches!(items.as_slice(), [feed::FeedItem::User(a), feed::FeedItem::Step { step: Step::Done, text }] if a.text() == "A website" && text == CALLED_DONE),
            "{items:?}"
        );
        let err = e.call("project_build_finish", json!({"project": d.path().join("nope")}), CallOptions::new(Caller::Cli)).unwrap_err();
        assert!(err.message.contains("isn't a project folder"), "{err:?}");
    }

    #[test]
    fn finishing_a_build_cannot_overwrite_a_project_recreated_after_its_feed_save() {
        use crate::workspace::{ExistingFiles, MANIFEST, Status, Step};
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("site");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("index.html"), b"Keep the app").unwrap();
        let mut original = Workspace::create_at(&root, "Original", "Original request", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        original.set_status(Status::Stopped, "The original is paused.").unwrap();
        let mut replacement_bytes = None;

        let error = finish_build_after_feed_save(original, |old| {
            assert!(matches!(feed::load(old).last(), Some(feed::FeedItem::Step { step: Step::Done, text }) if text == CALLED_DONE));
            let old_generation = old.manifest.generation.clone();
            old.clone().delete().unwrap();
            let mut replacement = Workspace::create_at(&root, "Replacement", "Replacement request", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
            assert_ne!(replacement.manifest.generation, old_generation);
            replacement.set_status(Status::Stopped, "The replacement is paused.").unwrap();
            feed::save(&root, &mut feed::load(&replacement)).unwrap();
            replacement_bytes = Some((std::fs::read(root.join(MANIFEST)).unwrap(), std::fs::read(root.join(feed::FEED_FILE)).unwrap()));
        })
        .unwrap_err();

        assert_eq!(error.code, craft_anything_evidence::ErrorCode::FileChanged);
        let (manifest, conversation) = replacement_bytes.unwrap();
        assert_eq!(std::fs::read(root.join(MANIFEST)).unwrap(), manifest);
        assert_eq!(std::fs::read(root.join(feed::FEED_FILE)).unwrap(), conversation);
        let replacement = Workspace::open(&root).unwrap();
        assert_eq!((replacement.manifest.request.as_str(), replacement.manifest.status), ("Replacement request", Status::Stopped));
        feed::save(&root, &mut feed::load(&replacement)).unwrap();
        assert_eq!(std::fs::read(root.join("index.html")).unwrap(), b"Keep the app");
    }

    #[test]
    fn a_failed_conversation_save_keeps_a_builds_status_unchanged() {
        use crate::workspace::{MANIFEST, Status, Step};
        let d = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(d.path(), "Site", "A website", TargetRef::None).unwrap();
        ws.set_status(Status::Stopped, "Paused.").unwrap();
        let original_manifest = std::fs::read(ws.root.join(MANIFEST)).unwrap();
        let path = ws.root.join(feed::FEED_FILE);
        std::fs::create_dir(&path).unwrap();
        let engine = Engine::new(crate::testing::session());
        let finish = || engine.call("project_build_finish", json!({"project": ws.root}), CallOptions::new(Caller::Cli));

        assert!(finish().is_err(), "the unreadable conversation must prevent completion");
        assert_eq!(std::fs::read(ws.root.join(MANIFEST)).unwrap(), original_manifest, "a failed feed save must not mark the project Ready");
        assert!(path.is_dir(), "the unreadable conversation is left alone");

        std::fs::remove_dir(&path).unwrap();
        finish().unwrap();
        let ws = Workspace::open(&ws.root).unwrap();
        assert_eq!((ws.manifest.status, ws.manifest.note.as_str()), (Status::Ready, CALLED_DONE));
        let items = feed::load(&ws);
        assert!(matches!(&items[0], feed::FeedItem::User(user) if user.text() == "A website"));
        assert_eq!(items.iter().filter(|item| matches!(item, feed::FeedItem::Step { step: Step::Done, text } if text == CALLED_DONE)).count(), 1);
    }

    #[test]
    fn a_read_only_project_cannot_change_its_conversation_when_called_done() {
        use crate::workspace::{MANIFEST, MANIFEST_VERSION, Status};
        let valid = serde_json::to_vec(&json!({"version": feed::FEED_VERSION, "items": [{"User": "Keep this conversation"}]})).unwrap();
        for (case, original) in [("missing", None), ("valid", Some(valid.as_slice())), ("damaged", Some(b"damaged conversation".as_slice()))] {
            let directory = tempfile::tempdir().unwrap();
            let mut ws = Workspace::create(directory.path(), "Site", "A website", TargetRef::None).unwrap();
            ws.set_status(Status::Stopped, "Paused.").unwrap();
            let manifest_path = ws.root.join(MANIFEST);
            let mut document: Value = serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
            document["version"] = json!(MANIFEST_VERSION + 1);
            document["future_setting"] = json!({"keep": true});
            let manifest = serde_json::to_vec_pretty(&document).unwrap();
            std::fs::write(&manifest_path, &manifest).unwrap();
            let feed_path = ws.root.join(feed::FEED_FILE);
            if let Some(bytes) = original {
                std::fs::write(&feed_path, bytes).unwrap();
            }
            assert!(Workspace::open(&ws.root).unwrap().is_read_only());
            let entries = || {
                let mut entries: Vec<_> = std::fs::read_dir(&ws.root).unwrap().map(|entry| entry.unwrap().file_name()).collect();
                entries.sort();
                entries
            };
            let before = entries();
            let engine = Engine::new(crate::testing::session());

            let error = engine.call("project_build_finish", json!({"project": ws.root}), CallOptions::new(Caller::Cli)).unwrap_err();

            assert_eq!(error.code, craft_anything_evidence::ErrorCode::NotSupported, "{case}: {error:?}");
            assert!(error.remediation.is_some(), "{case}: the person is told how to open this project for editing");
            assert_eq!(std::fs::read(&manifest_path).unwrap(), manifest, "{case}: the newer manifest remains byte-for-byte unchanged");
            assert_eq!(std::fs::read(&feed_path).ok().as_deref(), original, "{case}: no Done entry was saved");
            assert_eq!(entries(), before, "{case}: no recovery backup or other record was created");
        }
    }

    #[test]
    fn a_build_finish_retry_does_not_repeat_a_saved_done_entry() {
        use crate::workspace::{MANIFEST, Status, Step};
        let d = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(d.path(), "Site", "A website", TargetRef::None).unwrap();
        ws.set_status(Status::Stopped, "Paused.").unwrap();
        let original_manifest = std::fs::read(ws.root.join(MANIFEST)).unwrap();
        let mut items = feed::load(&ws);
        items.push(feed::FeedItem::Step { step: Step::Done, text: CALLED_DONE.into() });
        feed::save(&ws.root, &mut items).unwrap();
        let saved_feed = std::fs::read(ws.root.join(feed::FEED_FILE)).unwrap();

        // Exercise a real status-write failure after the conversation was saved, without
        // relying on permissions or a race between the tool's two writes.
        std::fs::remove_file(ws.root.join(MANIFEST)).unwrap();
        std::fs::create_dir(ws.root.join(MANIFEST)).unwrap();
        assert!(ws.set_status(Status::Ready, CALLED_DONE).is_err());
        assert_eq!(std::fs::read(ws.root.join(feed::FEED_FILE)).unwrap(), saved_feed);
        std::fs::remove_dir(ws.root.join(MANIFEST)).unwrap();
        std::fs::write(ws.root.join(MANIFEST), original_manifest).unwrap();

        let engine = Engine::new(crate::testing::session());
        let finish = || engine.call("project_build_finish", json!({"project": ws.root}), CallOptions::new(Caller::Cli));
        finish().unwrap();
        finish().unwrap();
        let ws = Workspace::open(&ws.root).unwrap();
        assert_eq!((ws.manifest.status, ws.manifest.note.as_str()), (Status::Ready, CALLED_DONE));
        let items = feed::load(&ws);
        assert_eq!(items.iter().filter(|item| matches!(item, feed::FeedItem::Step { step: Step::Done, text } if text == CALLED_DONE)).count(), 1);
        assert_eq!(std::fs::read(ws.root.join(feed::FEED_FILE)).unwrap(), saved_feed);
    }

    #[test]
    fn list_show_and_answer_a_project_without_the_app() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Snake", "A snake game", TargetRef::None).unwrap();
        let q = Inbox::new(ws.inbox_dir()).post(RequestKind::Question, "Blue or red?", "", vec!["Blue".into(), "Red".into()], true, None).unwrap();
        let e = Engine::new(crate::testing::session());
        let call = |id: &str, p: Value| e.call(id, p, CallOptions::new(Caller::Cli)).map(|o| o.to_json()["result"].clone());

        let list = call("project_list", json!({"projects_dir": d.path()})).unwrap();
        assert_eq!(list["count"], 1);
        assert_eq!(list["projects"][0]["name"], "Snake");
        assert_eq!(list["projects"][0]["waiting"], 1);

        let show = call("project_show", json!({"project": ws.root})).unwrap();
        assert_eq!(show["questions"][0]["id"], q.as_str());
        assert_eq!(show["conversation"]["items"][0]["User"], "A snake game");

        let r = call("project_inbox_answer", json!({"project": ws.root, "id": q, "choice": "Blue"})).unwrap();
        assert_eq!(r["shown"], "Blue");
        assert!(Inbox::new(ws.inbox_dir()).pending().is_empty(), "answered");
        let again = call("project_inbox_answer", json!({"project": ws.root, "id": q, "choice": "Red"}));
        assert!(again.is_err(), "a question is answered once");
        assert!(call("project_show", json!({"project": d.path().join("nope")})).is_err());
    }

    /// An unreadable usage.json is said to be unknown, never shown as zeros.
    #[test]
    fn a_projects_unreadable_usage_is_unknown_not_zero() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Snake", "A snake game", TargetRef::None).unwrap();
        let e = Engine::new(crate::testing::session());
        let show = || e.call("project_show", json!({"project": ws.root}), CallOptions::new(Caller::Cli)).unwrap().to_json()["result"].clone();

        let none = show();
        assert_eq!((none["usage_unreadable"].clone(), none["usage"]["cost_usd"].clone()), (json!(false), json!(0.0)), "{none}");
        assert!(none.get("usage_note").is_none());

        std::fs::write(ws.root.join(USAGE_FILE), b"{ not json").unwrap();
        let bad = show();
        assert_eq!(bad["usage_unreadable"], true, "{bad}");
        assert!(bad["usage"].is_null(), "no zeros stand in for it: {bad}");
        assert!(bad["usage_note"].as_str().unwrap().contains("isn't known"));
        let list = e.call("project_list", json!({"projects_dir": d.path()}), CallOptions::new(Caller::Cli)).unwrap().to_json();
        assert_eq!(list["result"]["projects"][0]["usage_unreadable"], true);

        std::fs::write(ws.root.join(USAGE_FILE), br#"{"version":1,"cost_usd":2.5,"priced":true}"#).unwrap();
        let known = show();
        assert_eq!((known["usage_unreadable"].clone(), known["usage"]["cost_usd"].clone()), (json!(false), json!(2.5)), "{known}");
    }

    #[test]
    fn an_answer_stays_pending_when_the_conversation_cannot_be_saved() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Snake", "A snake game", TargetRef::None).unwrap();
        let inbox = Inbox::new(ws.inbox_dir());
        let id = inbox.post(RequestKind::Question, "Blue or red?", "", vec![], true, None).unwrap();
        let path = ws.root.join(feed::FEED_FILE);
        std::fs::create_dir(&path).unwrap();
        let engine = Engine::new(crate::testing::session());
        let answer = || engine.call("project_inbox_answer", json!({"project": ws.root, "id": id, "choice": "Blue"}), CallOptions::new(Caller::Cli));
        assert!(answer().is_err(), "a failed conversation save must reach the caller");
        assert_eq!(inbox.pending().len(), 1, "the person can retry the same request");
        assert!(!inbox.dir().join(format!("{id}.answer.json")).exists());
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, br#"{"version":5,"items":[{"User":"Keep the earlier conversation"}]}"#).unwrap();
        answer().unwrap();
        assert!(inbox.pending().is_empty());
        let saved = feed::load(&ws);
        assert!(matches!(&saved[0], feed::FeedItem::User(user) if user.text() == "Keep the earlier conversation"));
        assert_eq!(
            saved.iter().filter(|item| matches!(item, feed::FeedItem::Ask { request, answer: Some(value) } if request.id == id && value == "Blue")).count(),
            1
        );
    }

    #[test]
    fn a_projects_asset_budget_is_read_and_set() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Game", "A game", TargetRef::None).unwrap();
        let e = Engine::new(crate::testing::session());
        let call = |p: Value| e.call("project_assets_budget", p, CallOptions::new(Caller::Cli)).map(|o| o.to_json()["result"].clone());
        assert_eq!(call(json!({"project": ws.root})).unwrap()["budget_usd"], 5.0);
        assert_eq!(call(json!({"project": ws.root, "budget_usd": 12.5})).unwrap()["budget_usd"], 12.5);
        assert_eq!(Workspace::open(&ws.root).unwrap().manifest.asset_budget_usd, 12.5);
        assert!(call(json!({"project": ws.root, "budget_usd": -1})).is_err());
        // #1825: a model wrote the number as text; it is read as the number it is, not refused.
        assert_eq!(call(json!({"project": ws.root, "budget_usd": "7.5"})).unwrap()["budget_usd"], 7.5);
    }

    /// #504: the plan card is folded by a tool too (the app's button calls it), and says whether
    /// it folded anything.
    #[test]
    fn a_plan_is_folded_by_its_tool_and_says_whether_it_did() {
        use crate::feed::{FeedItem, PlanStep, save, visible_plan};
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Game", "A game", TargetRef::None).unwrap();
        let steps = vec![PlanStep { text: "Lay out".into(), doing: "Laying out".into(), status: crate::feed::PlanStatus::Todo }];
        let mut initial = feed::load(&ws);
        initial.push(FeedItem::Plan(steps));
        save(&ws.root, &mut initial).unwrap();
        let e = Engine::new(crate::testing::session());
        let fold = || e.call("project_plan_fold", json!({"project": ws.root}), CallOptions::new(Caller::Cli)).map(|o| o.to_json()["result"]["folded"].clone());
        assert_eq!(fold().unwrap(), true);
        assert!(visible_plan(&crate::feed::load(&ws)).is_none());
        assert_eq!(fold().unwrap(), false, "already folded");
    }

    #[test]
    fn a_project_generates_assets_only_once_allowed() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Game", "A game", TargetRef::None).unwrap();
        let e = Engine::new(crate::testing::session());
        let call = |p: Value| e.call("project_assets_allow", p, CallOptions::new(Caller::Cli)).map(|o| o.to_json()["result"].clone());
        assert_eq!(call(json!({"project": ws.root})).unwrap()["generate_assets"], false, "off until the person turns it on");
        assert_eq!(call(json!({"project": ws.root, "allow": true})).unwrap()["generate_assets"], true);
        assert!(Workspace::open(&ws.root).unwrap().manifest.generate_assets, "saved with the project");
        assert_eq!(call(json!({"project": ws.root, "allow": false})).unwrap()["generate_assets"], false);
        assert!(call(json!({"project": ws.root, "allow": "yes"})).is_err());
    }

    #[test]
    fn a_built_site_is_previewed_and_nothing_to_play_says_so() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Site", "A page", TargetRef::None).unwrap();
        let e = Engine::new(crate::testing::session());
        let granted = || CallOptions::new(Caller::Cli).with_consent(crate::Consent::Granted);
        let call = |id: &str, p: Value| e.call(id, p, granted()).map(|o| o.to_json()["result"].clone());
        assert!(call("project_preview", json!({"project": ws.root})).is_err(), "nothing built yet");
        assert!(call("project_play", json!({"project": ws.root})).is_err(), "nothing to start yet");
        std::fs::create_dir_all(ws.app_dir().join("dist")).unwrap();
        std::fs::write(ws.app_dir().join("dist").join("index.html"), b"<h1>hi</h1>").unwrap();
        let first = call("project_preview", json!({"project": ws.root})).unwrap();
        let url = first["url"].as_str().unwrap().to_string();
        assert!(url.starts_with("http://127.0.0.1:"), "{url}");
        assert_eq!(call("project_preview", json!({"project": ws.root})).unwrap()["url"], url.as_str(), "one server per project");
        let err = call("project_play", json!({"project": ws.root})).unwrap_err();
        assert!(err.remediation.unwrap_or_default().contains("project_preview"));
        // Playing runs the builder's code: refused without consent.
        assert!(e.call("project_play", json!({"project": ws.root}), CallOptions::new(Caller::Mcp)).is_err());
    }

    #[test]
    fn a_pinned_builds_folder_is_opened_from_its_project() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Snake", "A snake game", TargetRef::None).unwrap();
        let pin = ws.root.join(crate::pins::PINNED).join("7");
        std::fs::create_dir_all(pin.join("app")).unwrap();
        std::fs::write(pin.join(crate::pins::PIN_FILE), r#"{"version":1,"id":7,"name":"Pinned","mode":"frozen","created_ms":0}"#).unwrap();
        assert_eq!(folder_to_open(&ws, false, Some(7)).unwrap(), pin);
        assert_eq!(folder_to_open(&ws, true, Some(7)).unwrap(), pin.join("app"));
        assert_eq!(folder_to_open(&ws, false, None).unwrap(), ws.root);
        assert_eq!(folder_to_open(&ws, true, None).unwrap(), ws.app_dir());
        assert!(folder_to_open(&ws, false, Some(8)).is_err(), "no such pin");
    }
}
