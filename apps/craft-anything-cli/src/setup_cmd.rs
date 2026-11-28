//! `setup`, `doctor`, `uninstall`.

use crate::output::Format;
use crate::{Io, fail};
use clap::Args;
use craft_anything_setup::{ActionKind, CheckStatus, DoctorRequest, PlanRequest};
use serde_json::{Value, json};

#[derive(Debug, Args)]
pub struct SetupArgs {
    /// Show what would change without writing anything.
    #[arg(long)]
    pub dry_run: bool,
    /// Client to configure (repeatable): claude_code, codex, cursor, …
    #[arg(long = "client")]
    pub clients: Vec<String>,
    /// Configure every detected client.
    #[arg(long)]
    pub all_detected: bool,
    /// Apply without asking.
    #[arg(long, short = 'y')]
    pub yes: bool,
    /// Don't install the skill.
    #[arg(long)]
    pub no_skill: bool,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Only check these clients (repeatable).
    #[arg(long = "client")]
    pub clients: Vec<String>,
}

#[derive(Debug, Args)]
pub struct UninstallArgs {}

fn human(f: Format) -> bool {
    matches!(f, Format::Text | Format::Md)
}

fn confirm(io: &mut Io<'_>, context: &crate::Context, question: &str) -> bool {
    if !context.can_ask() {
        return false;
    }
    errp!(io, "{question} [y/N] ");
    matches!(context.read_line().trim(), "y" | "Y" | "yes")
}

fn print_plan(io: &mut Io<'_>, plan: &craft_anything_setup::Plan) {
    io.line(&format!("Register: {}", plan.command.join(" ")));
    for a in &plan.actions {
        let v = serde_json::to_value(a).unwrap_or(Value::Null);
        io.line(&format!("[{}] {} — {}", v["action"].as_str().unwrap_or_default(), a.display, v["path"].as_str().unwrap_or_default()));
        if let Some(r) = v["reason"].as_str() {
            io.line(&format!("    {r}"));
        }
        if let Some(r) = v["remediation"].as_str() {
            io.line(&format!("    → {r}"));
        }
        if matches!(a.action, ActionKind::Create | ActionKind::Update) {
            for l in v["preview"].as_str().unwrap_or_default().lines() {
                io.line(&format!("    {l}"));
            }
        }
    }
    for s in &plan.skills {
        let v = serde_json::to_value(s).unwrap_or(Value::Null);
        io.line(&format!("[skill] {}", v["dir"].as_str().unwrap_or_default()));
    }
    for n in &plan.notes {
        io.line(&format!("note: {n}"));
    }
}

pub fn setup(io: &mut Io<'_>, context: &crate::Context, a: &SetupArgs, f: Format) -> i32 {
    let host = context.host();
    let command = match context.mcp_command() {
        Ok(c) => c,
        Err(e) => return fail(io, &e, f),
    };
    let req = PlanRequest { clients: a.clients.clone(), all_detected: a.all_detected, command, include_skill: !a.no_skill };
    let plan = match craft_anything_setup::plan(&host, &req) {
        Ok(p) => p,
        Err(e) => return fail(io, &e, f),
    };
    let status = if plan.blocked {
        "blocked"
    } else if a.dry_run {
        "planned"
    } else if plan.change_count() == 0 {
        "unchanged"
    } else {
        ""
    };
    if !status.is_empty() {
        if human(f) {
            print_plan(io, &plan);
            io.line(&format!("status: {status}"));
        } else {
            io.emit(&json!({"status": status, "plan": plan}), f);
        }
        return if plan.blocked { 1 } else { 0 };
    }
    let approved = a.yes || {
        if human(f) {
            print_plan(io, &plan);
        }
        confirm(io, context, &format!("Apply these {} changes?", plan.change_count()))
    };
    if !approved {
        if human(f) {
            io.line("status: needs_confirmation (re-run with --yes to apply)");
        } else {
            io.emit(&json!({"status": "needs_confirmation", "plan": plan}), f);
        }
        return 1;
    }
    let report = craft_anything_setup::apply(&plan);
    if human(f) {
        for r in &report.results {
            let v = serde_json::to_value(r).unwrap_or(Value::Null);
            io.line(&format!(
                "[{}] {} {}",
                v["outcome"].as_str().unwrap_or_default(),
                v["client"].as_str().unwrap_or_default(),
                v["error"].as_str().unwrap_or_default()
            ));
        }
        io.line(&format!("status: {}", report.status));
        if !report.restart.is_empty() {
            io.line(&format!("Restart {} to load Craft Anything.", report.restart.join(", ")));
        }
    } else {
        io.emit(&serde_json::to_value(&report).unwrap_or(Value::Null), f);
    }
    if matches!(report.status, "configured" | "unchanged") { 0 } else { 1 }
}

pub fn doctor(io: &mut Io<'_>, context: &crate::Context, a: &DoctorArgs, f: Format) -> i32 {
    let host = context.host();
    let req = DoctorRequest { clients: a.clients.clone(), extra: crate::ai_cmd::doctor_checks(context), ..DoctorRequest::default() };
    let report = craft_anything_setup::doctor(&host, &req);
    if human(f) {
        for c in &report.checks {
            let st = match c.status {
                CheckStatus::Ok => "ok",
                CheckStatus::Warn => "warn",
                CheckStatus::Fail => "FAIL",
                CheckStatus::Info => "info",
            };
            io.line(&format!("[{st}] {}: {}", c.name, c.message));
            if let Some(r) = &c.remediation {
                io.line(&format!("       → {r}"));
            }
        }
        io.line(if report.healthy { "healthy" } else { "problems found" });
    } else {
        io.emit(&serde_json::to_value(&report).unwrap_or(Value::Null), f);
    }
    if report.healthy { 0 } else { 1 }
}

pub fn uninstall(io: &mut Io<'_>, context: &crate::Context, _a: &UninstallArgs, f: Format) -> i32 {
    let report = craft_anything_setup::uninstall(&context.host());
    // Coding-agent sessions a build left running when the app was killed, which the agent's own
    // service would otherwise resume after we're gone (#1115).
    let logs = craft_anything_ai::failure_log::dir_in(&context.config_dir());
    let sessions = craft_anything_ai::cli_agents::unfinished::sweep(
        &craft_anything_ai::cli_agents::unfinished::dir_in(&logs),
        &craft_anything_ai::cli_agents::adapters::AgentAdapters::default(),
    );
    let mut v = serde_json::to_value(&report).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.insert("unfinished_sessions".into(), json!(sessions));
    }
    if human(f) {
        for s in &sessions {
            io.line(s);
        }
        for r in &report.removed_entries {
            let e = serde_json::to_value(r).unwrap_or(Value::Null);
            io.line(&format!("removed {} ({})", e["client"].as_str().unwrap_or_default(), e["path"].as_str().unwrap_or_default()));
        }
        for p in &report.removed_skill_files {
            io.line(&format!("removed {}", p.display()));
        }
        for b in &report.blocking {
            io.line(&format!("blocked by {}: {}", b.display, b.reason.as_deref().unwrap_or("its config can't be changed safely")));
            if let Some(fix) = &b.remediation {
                io.line(&format!("  {fix}"));
            }
        }
        io.line(&format!("status: {}", report.status));
    } else {
        io.emit(&v, f);
    }
    if matches!(report.status, "removed" | "nothing_to_remove") { 0 } else { 1 }
}
