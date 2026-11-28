//! Pinned builds as tools: pin a project's build (a frozen or light copy), list pins with what's
//! running, open, stop, rename and remove them. Play shares their run list, so a program runs once
//! per folder unless another copy is asked for. A pinned website is served instead. See
//! `crate::pins`.

use crate::pins::{self, Opened, PinMode, Runs, Sites};
use crate::tool::{Ctx, Effects, Requires, ToolOutput, ToolSpec, Toolset, parse_params, schema_of};
use crate::workspace::Workspace;
use craft_anything_evidence::{ErrorCode, ToolError};
use craft_anything_platform::Launch;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            id: "project_pin_create",
            title: "Pin a project's build",
            summary: "Keep the project's current build at hand to open later, whatever the builder changes next. `frozen` (the default) copies the app's files and its program into the project's pinned/ folder, without compiler output or dependencies it doesn't start from, so rebuilds can't change or break it. `light` copies the program and the code too but hard-links dependencies and media to the project's, saving their space: replacing or deleting those in the project leaves the pin's as they were, but a file rewritten in place changes in both. With `estimate`, only says what each would take. A website's pin copies the built site (its media shared by a light pin).",
            beginner: "Keeps this version so you can always open it.",
            toolset: Toolset::Projects,
            effects: Effects::WRITES_FILESYSTEM,
            requires: Requires::Nothing,
            input_schema: schema_of::<CreateParams>,
            run: project_pin_create,
        },
        ToolSpec {
            id: "project_pins_list",
            title: "List pinned builds",
            summary: "The pins of one project, or of every project in the project folders from settings: id, name, frozen or light, the round it was pinned at, its size, and whether it's running (started in this session; a website's pin also gives the address it's served at).",
            beginner: "Shows the versions you kept.",
            toolset: Toolset::Projects,
            effects: Effects::READ_ONLY,
            requires: Requires::Nothing,
            input_schema: schema_of::<ListParams>,
            run: project_pins_list,
        },
        ToolSpec {
            id: "project_pin_open",
            title: "Open a pinned build",
            summary: "Start a pinned build in its own window; a pinned website is served from its copy on a loopback port of its own, and its address returned (`url`; `another` doesn't apply). The website is served for this session; with `wait`, the call itself serves it until it's cancelled (Ctrl+C in the CLI, whose session ends with the command). If it's already running, brings its window to the front (macOS; `brought_to_front` says whether that was asked of the system) and says so instead of starting a second copy, unless `another` is true (copies of one pin share the files they save beside themselves). A start that quits within two seconds is reported with its exit code and last output. It runs the code the builder wrote, so it needs the person's consent.",
            beginner: "Opens a version you kept.",
            toolset: Toolset::Projects,
            effects: Effects::LAUNCHES_PROCESS.union(Effects::EXECUTES_FOREIGN_CODE).union(Effects::CHANGES_UI_STATE),
            requires: Requires::Nothing,
            input_schema: schema_of::<OpenParams>,
            run: project_pin_open,
        },
        ToolSpec {
            id: "project_pin_stop",
            title: "Stop a pinned build",
            summary: "Close what a pin started in this session (every copy of it), or stop serving a pinned website.",
            beginner: "Closes a version you opened.",
            toolset: Toolset::Projects,
            effects: Effects::MUTATES_SESSION,
            requires: Requires::Nothing,
            input_schema: schema_of::<PinParams>,
            run: project_pin_stop,
        },
        ToolSpec {
            id: "project_pin_rename",
            title: "Rename a pinned build",
            summary: "Give a pin a new name.",
            beginner: "Renames a version you kept.",
            toolset: Toolset::Projects,
            effects: Effects::WRITES_FILESYSTEM,
            requires: Requires::Nothing,
            input_schema: schema_of::<RenameParams>,
            run: project_pin_rename,
        },
        ToolSpec {
            id: "project_pin_delete",
            title: "Remove a pinned build",
            summary: "Remove a pin. Its copy goes with it, including anything the program saved beside itself there, so `confirm` must be true; a running pin is refused until it's stopped.",
            beginner: "Removes a version you kept.",
            toolset: Toolset::Projects,
            effects: Effects::WRITES_FILESYSTEM.union(Effects::MAY_DISCARD_DATA),
            requires: Requires::Nothing,
            input_schema: schema_of::<DeleteParams>,
            run: project_pin_delete,
        },
    ]
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateParams {
    /// The project's folder.
    pub project: PathBuf,
    /// `frozen` (default): a copy of its own; `light`: shares dependencies and media with the
    /// project. `live` was retired and is refused.
    #[serde(default)]
    pub mode: PinMode,
    /// Its name (default: the project's name and round).
    #[serde(default)]
    pub name: Option<String>,
    /// Only say what each kind of copy would take; pin nothing.
    #[serde(default)]
    pub estimate: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListParams {
    /// Only this project's pins (default: every project's).
    #[serde(default)]
    pub project: Option<PathBuf>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PinParams {
    /// The project's folder.
    pub project: PathBuf,
    /// The pin's id (from project_pins_list).
    pub pin: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenParams {
    /// The project's folder.
    pub project: PathBuf,
    /// The pin's id (from project_pins_list).
    pub pin: u32,
    /// Start another copy even if one is running.
    #[serde(default)]
    pub another: bool,
    /// For a website, serve until the call is cancelled instead of returning at once (for the CLI).
    #[serde(default)]
    pub wait: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameParams {
    /// The project's folder.
    pub project: PathBuf,
    /// The pin's id.
    pub pin: u32,
    /// Its new name.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteParams {
    /// The project's folder.
    pub project: PathBuf,
    /// The pin's id.
    pub pin: u32,
    /// Must be true: a pin's copy, and what it saved, can't be brought back.
    #[serde(default)]
    pub confirm: bool,
}

fn open_project(project: &Path) -> Result<Workspace, ToolError> {
    Workspace::open(project).map_err(|e| {
        ToolError::new(ErrorCode::NotFound, format!("{} isn't a project folder: {}", project.display(), e.message))
            .with_remediation("Pass a folder from project_list (it holds craft.json).")
    })
}

/// The Godot a Godot project opens in.
pub(crate) fn godot(ctx: &Ctx<'_>) -> Option<PathBuf> {
    let store = ctx.session.settings.engine_store_dir();
    craft_anything_engines::game::inventory(&craft_anything_engines::game::Places::here(store))
        .best(craft_anything_engines::game::Builder::Godot)
        .map(|i| i.program.clone())
}

/// This session's run list (made on first use).
pub(crate) fn runs(ctx: &mut Ctx<'_>) -> Arc<Runs> {
    match ctx.session.ext.get::<Runs>() {
        Some(r) => r,
        None => {
            let r = Arc::new(Runs::default());
            ctx.session.ext.insert(r.clone());
            r
        }
    }
}

/// This session's served pinned websites (made on first use).
fn sites(ctx: &mut Ctx<'_>) -> Arc<Sites> {
    match ctx.session.ext.get::<Sites>() {
        Some(s) => s,
        None => {
            let s = Arc::new(Sites::default());
            ctx.session.ext.insert(s.clone());
            s
        }
    }
}

/// Start what `app` holds through the run list, logging to `log`, and describe what happened. A
/// program that quit straight away with an error is an error carrying its last output.
pub(crate) fn start(ctx: &mut Ctx<'_>, app: &Path, log: &Path, another: bool, label: &str) -> Result<Value, ToolError> {
    let Some((program, args)) = crate::outputs::launch_now(app, godot(ctx).as_deref()) else {
        return Err(ToolError::new(ErrorCode::NotFound, "There's nothing to start: no built program here.")
            .with_remediation("Finish a build first (rebuild), or open the folder to look."));
    };
    start_program(ctx, app, log, Again { another, to_front: false }, label, (program, args))
}

/// What to do when the program already runs from that folder.
struct Again {
    /// Start a second copy anyway.
    another: bool,
    /// Otherwise bring the running one's window to the front.
    to_front: bool,
}

/// Start a program from `app` through the run list, logging to `log`. The program is made absolute
/// first because the child runs with `app` as its working directory.
fn start_program(ctx: &mut Ctx<'_>, app: &Path, log: &Path, again: Again, label: &str, (program, args): (PathBuf, Vec<String>)) -> Result<Value, ToolError> {
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir).map_err(|e| ToolError::io(dir.display(), e))?;
    }
    let program = crate::outputs::absolute_program(&program);
    let launch = Launch::new(&program).args(&args).cwd(app).hide_console().log_to(log).label(label).remediation("Open the folder and start it by hand.");
    let (launched, quit) = match runs(ctx).open(app, launch, again.another)? {
        Opened::AlreadyRunning(l) => {
            let brought_to_front = again.to_front && craft_anything_platform::window::bring_to_front(l.pid());
            return Ok(json!({"already_running": true, "brought_to_front": brought_to_front, "pid": l.pid(), "started": program.display().to_string()}));
        }
        Opened::Started { launched, quit } => (launched, quit),
    };
    let last_output = pins::tail(log);
    match quit {
        Some(ended) if ended.code != Some(0) => {
            let code = ended.code.map_or_else(|| "it was stopped".to_string(), |c| format!("exit code {c}"));
            let said = if last_output.is_empty() { String::new() } else { format!(" It printed:\n{last_output}") };
            Err(ToolError::new(ErrorCode::ProgramFailed, format!("{label} closed straight away ({code}).{said}"))
                .with_remediation(format!("Its whole output is in {}.", log.display())))
        }
        _ => Ok(json!({
            "already_running": false,
            "started": program.display().to_string(),
            "args": args,
            "pid": launched.pid(),
            "closed_at_once": quit.map(|e| json!({"code": e.code, "after_ms": e.after.as_millis() as u64})),
            "last_output": quit.map(|_| last_output),
        })),
    }
}

fn project_pin_create(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: CreateParams = parse_params(params)?;
    let ws = open_project(&p.project).map_err(|e| e.with_details(json!({"project": &p.project, "estimate": p.estimate})))?;
    let godot = godot(ctx);
    if p.estimate {
        let size = pins::estimate(&ws, godot.as_deref()).map_err(|e| e.with_details(json!({"project": &ws.root, "estimate": true})))?;
        return Ok(ToolOutput::Value(json!({"project": &ws.root, "estimate": size})));
    }
    let cancel = ctx.cancel.clone();
    let mut last = std::time::Instant::now();
    let mut progress = |done: u64, total: u64| {
        if last.elapsed().as_millis() >= 250 || done == total {
            last = std::time::Instant::now();
            let fraction = if total == 0 { 1.0 } else { done as f32 / total as f32 };
            ctx.report(Some(fraction), format!("Copying the build: {} of {} MB", done / 1_000_000, total / 1_000_000));
        }
        !cancel.is_cancelled()
    };
    let pin = pins::create(&ws, p.mode, p.name.as_deref(), godot.as_deref(), &mut progress).map_err(|e| e.with_details(json!({"project": &ws.root})))?;
    Ok(ToolOutput::Value(json!({"pin": pin, "folder": pin.dir(&ws).display().to_string()})))
}

fn project_pins_list(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: ListParams = parse_params(params)?;
    let projects = match &p.project {
        Some(dir) => vec![open_project(dir)?],
        None => Workspace::list_in(&ctx.session.settings.project_locations()),
    };
    let (runs, sites) = (runs(ctx), sites(ctx));
    let mut out = Vec::new();
    let mut copied_bytes = 0u64;
    for ws in &projects {
        for pin in pins::list(ws) {
            copied_bytes = copied_bytes.saturating_add(pin.bytes);
            let running = runs.running(&pin.app_dir(ws));
            let url = sites.url(&pin.app_dir(ws));
            out.push(json!({
                "project": ws.root.display().to_string(),
                "project_name": ws.manifest.name,
                "pin": pin,
                "folder": pin.dir(ws).display().to_string(),
                "running": !running.is_empty() || url.is_some(),
                "url": url,
                "pids": running.iter().map(|l| l.pid()).collect::<Vec<_>>(),
            }));
        }
    }
    Ok(ToolOutput::Value(json!({"count": out.len(), "pins": out, "copied_bytes": copied_bytes})))
}

fn project_pin_open(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: OpenParams = parse_params(params)?;
    let ws = open_project(&p.project)?;
    let pin = pins::get(&ws, p.pin)?;
    if pin.site {
        let (sites, dir) = (sites(ctx), pin.app_dir(&ws));
        let (url, already_running) = sites.open(&dir)?;
        if p.wait {
            ctx.report(None, format!("Serving {url} (stop to end)"));
            while !ctx.cancel.is_cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            // What was served before this call keeps being served for whoever opened it.
            if !already_running {
                sites.stop(&dir);
            }
        }
        return Ok(ToolOutput::Value(json!({"pin": pin.id, "already_running": already_running, "url": url})));
    }
    let launch = pin.launch(&ws, godot(ctx).as_deref(), &ctx.session.cargo())?;
    let mut out = start_program(ctx, &pin.app_dir(&ws), &pin.log_path(&ws), Again { another: p.another, to_front: true }, &pin.name, launch)?;
    if let Some(o) = out.as_object_mut() {
        o.insert("pin".into(), json!(pin.id));
    }
    Ok(ToolOutput::Value(out))
}

fn project_pin_stop(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: PinParams = parse_params(params)?;
    let ws = open_project(&p.project)?;
    let pin = pins::get(&ws, p.pin)?;
    let stopped = runs(ctx).stop(&pin.app_dir(&ws)) + sites(ctx).stop(&pin.app_dir(&ws));
    Ok(ToolOutput::Value(json!({"pin": pin.id, "stopped": stopped})))
}

fn project_pin_rename(_ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: RenameParams = parse_params(params)?;
    let ws = open_project(&p.project)?;
    Ok(ToolOutput::Value(json!({"pin": pins::rename(&ws, p.pin, &p.name)?})))
}

fn project_pin_delete(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: DeleteParams = parse_params(params)?;
    if !p.confirm {
        return Err(ToolError::invalid("Removing a pin can't be undone.").with_remediation("Pass confirm: true to remove it."));
    }
    let ws = open_project(&p.project)?;
    let pin = pins::get(&ws, p.pin)?;
    // An old live pin runs the project's app: what's running there isn't the pin's to keep.
    let served = sites(ctx).url(&pin.app_dir(&ws)).is_some();
    if pin.mode.keeps_copy() && (served || !runs(ctx).running(&pin.app_dir(&ws)).is_empty()) {
        return Err(ToolError::invalid(format!("{} is open.", pin.name)).with_remediation("Close it (project_pin_stop), then remove it."));
    }
    pins::delete(&ws, p.pin)?;
    Ok(ToolOutput::Value(json!({"removed": p.pin})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::TargetRef;
    use crate::{CallOptions, Caller, Consent, Engine};

    #[cfg(unix)]
    #[test]
    fn opening_a_pin_keeps_the_program_selected_before_copying() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, SystemTime};

        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Game", "a packaged game", TargetRef::None).unwrap();
        let build = ws.app_dir().join("Build");
        std::fs::create_dir_all(build.join("obj")).unwrap();
        let game = build.join("game");
        let helper = build.join("obj/helper");
        for (path, output, seconds) in [(&game, "selected-game", 20), (&helper, "wrong-helper", 10)] {
            std::fs::write(path, format!("#!/bin/sh\nprintf '%s\\n' '{output}'\n")).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
            let times = std::fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds));
            std::fs::File::options().write(true).open(path).unwrap().set_times(times).unwrap();
        }
        assert_eq!(crate::outputs::packaged_program(&build), Some(game));
        let e = Engine::new(crate::testing::session());
        let call = |id: &str, params: Value| {
            e.call(id, params, CallOptions::new(Caller::Cli).with_consent(Consent::Granted)).map(|out| out.to_json()["result"].clone())
        };
        for mode in ["frozen", "light"] {
            let made = call("project_pin_create", json!({"project": ws.root, "mode": mode})).unwrap();
            let id = made["pin"]["id"].as_u64().unwrap() as u32;
            let pin = pins::get(&ws, id).unwrap();
            let copy = pin.app_dir(&ws);
            let future = std::fs::FileTimes::new().set_modified(SystemTime::now() + Duration::from_secs(60));
            std::fs::File::options().write(true).open(copy.join("Build/obj/helper")).unwrap().set_times(future).unwrap();
            assert_eq!(crate::outputs::packaged_program(&copy.join("Build")), Some(copy.join("Build/obj/helper")));
            let opened = call("project_pin_open", json!({"project": ws.root, "pin": id})).unwrap();
            assert_eq!(opened["started"], copy.join("Build/game").display().to_string());
            assert_eq!(pins::tail(&pin.log_path(&ws)), "selected-game");
            std::fs::remove_file(copy.join("Build/game")).unwrap();
            assert_eq!(call("project_pin_open", json!({"project": ws.root, "pin": id})).unwrap_err().code, ErrorCode::NotFound);
        }
    }

    /// A project opened by a relative folder starts its built program from an absolute path, not
    /// from the program path applied to the workspace's working folder twice.
    #[cfg(unix)]
    #[test]
    fn a_relative_project_plays_its_built_program() {
        use std::os::unix::fs::PermissionsExt;

        let cwd = std::env::current_dir().unwrap();
        let d = tempfile::tempdir_in(&cwd).unwrap();
        let relative = d.path().strip_prefix(&cwd).unwrap();
        let ws = Workspace::create(relative, "Game", "a game", TargetRef::None).unwrap();
        let app = ws.app_dir();
        std::fs::create_dir_all(app.join("target/debug")).unwrap();
        std::fs::write(app.join("Cargo.toml"), "[package]\nname = \"game\"\n").unwrap();
        let program = app.join("target/debug/game");
        std::fs::write(&program, "#!/bin/sh\nprintf 'played\\n'\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();

        let e = Engine::new(crate::testing::session());
        let result =
            e.call("project_play", json!({"project": ws.root}), CallOptions::new(Caller::Cli).with_consent(Consent::Granted)).unwrap().to_json()["result"]
                .clone();
        assert_eq!(result["already_running"], false);
        assert_eq!(result["closed_at_once"]["code"], 0, "{result}");
        assert!(std::path::Path::new(result["started"].as_str().unwrap()).is_absolute(), "{result}");

        let made = e
            .call("project_pin_create", json!({"project": ws.root}), CallOptions::new(Caller::Cli).with_consent(Consent::Granted))
            .unwrap()
            .to_json()["result"]
            .clone();
        let opened = e
            .call("project_pin_open", json!({"project": ws.root, "pin": made["pin"]["id"]}), CallOptions::new(Caller::Cli).with_consent(Consent::Granted))
            .unwrap()
            .to_json()["result"]
            .clone();
        assert_eq!(opened["closed_at_once"]["code"], 0, "{opened}");
        assert!(std::path::Path::new(opened["started"].as_str().unwrap()).is_absolute(), "{opened}");
    }

    /// Opening a pin that's already running starts no second copy and asks for its window to come
    /// to the front; a program with no window (this stand-in) can't, and the result says so.
    #[cfg(unix)]
    #[test]
    fn opening_a_running_pin_again_asks_for_its_window() {
        use std::os::unix::fs::PermissionsExt;

        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Game", "a game", TargetRef::None).unwrap();
        let app = ws.app_dir();
        std::fs::create_dir_all(app.join("target/debug")).unwrap();
        std::fs::write(app.join("Cargo.toml"), "[package]\nname = \"game\"\n").unwrap();
        let program = app.join("target/debug/game");
        std::fs::write(&program, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();

        let e = Engine::new(crate::testing::session());
        let call =
            |id: &str, params: Value| e.call(id, params, CallOptions::new(Caller::Cli).with_consent(Consent::Granted)).unwrap().to_json()["result"].clone();
        let pin = call("project_pin_create", json!({"project": ws.root}))["pin"]["id"].clone();
        let first = call("project_pin_open", json!({"project": ws.root, "pin": pin}));
        assert_eq!(first["already_running"], false, "{first}");
        let again = call("project_pin_open", json!({"project": ws.root, "pin": pin}));
        assert_eq!(again["already_running"], true, "{again}");
        assert_eq!(again["pid"], first["pid"], "no second copy");
        assert_eq!(again["brought_to_front"], false, "a program without a window can't come to the front: {again}");
        call("project_pin_stop", json!({"project": ws.root, "pin": pin}));
    }

    #[test]
    fn pin_list_rename_and_remove_from_the_cli() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Snake", "a snake game", TargetRef::None).unwrap();
        let app = ws.app_dir();
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::write(app.join("Cargo.toml"), "[package]\nname = \"snake\"\n").unwrap();
        std::fs::write(app.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::create_dir_all(app.join("target/debug")).unwrap();
        let program = app.join("target/debug").join(if cfg!(windows) { "snake.exe" } else { "snake" });
        std::fs::write(&program, b"program").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let e = Engine::new(crate::testing::session());
        let call = |id: &str, p: Value| e.call(id, p, CallOptions::new(Caller::Cli).with_consent(Consent::Granted)).map(|o| o.to_json()["result"].clone());

        let est = call("project_pin_create", json!({"project": ws.root, "estimate": true})).unwrap();
        assert_eq!(est["project"].as_str(), ws.root.to_str(), "an estimate names the project it was for");
        assert!(est["estimate"]["bytes"].as_u64().unwrap() > 0);
        assert!(pins::list(&ws).is_empty(), "an estimate pins nothing");
        let made = call("project_pin_create", json!({"project": ws.root})).unwrap();
        assert_eq!(made["pin"]["mode"], "frozen");
        let light = call("project_pin_create", json!({"project": ws.root, "mode": "light", "name": "Latest"})).unwrap();

        let list = call("project_pins_list", json!({"project": ws.root})).unwrap();
        assert_eq!(list["count"], 2);
        assert_eq!(list["copied_bytes"], made["pin"]["bytes"].as_u64().unwrap() + light["pin"]["bytes"].as_u64().unwrap());
        assert_eq!(list["pins"][1]["pin"]["name"], "Latest");
        assert_eq!(list["pins"][0]["running"], false);

        call("project_pin_rename", json!({"project": ws.root, "pin": 1, "name": "Keeper"})).unwrap();
        assert!(call("project_pin_delete", json!({"project": ws.root, "pin": 1})).is_err(), "removing needs confirm");
        call("project_pin_delete", json!({"project": ws.root, "pin": 1, "confirm": true})).unwrap();
        let list = call("project_pins_list", json!({"project": ws.root})).unwrap();
        assert_eq!(list["count"], 1);
        assert_eq!(list["copied_bytes"], light["pin"]["bytes"]);
        assert_eq!(call("project_pin_open", json!({"project": ws.root, "pin": 1})).unwrap_err().code, ErrorCode::NotFound);
    }

    /// What a loopback server sends for `/` at `url`.
    fn fetch(url: &str) -> String {
        use std::io::{Read, Write};
        let address = url.trim_start_matches("http://").trim_end_matches('/');
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        stream.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
        stream.write_all(format!("GET / HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes()).unwrap();
        let mut out = String::new();
        stream.read_to_string(&mut out).unwrap();
        out
    }

    /// Each pinned website is served from its own copy on a port of its own, says so when it's
    /// served already, can't be removed while served, and stops when asked.
    #[test]
    fn a_pinned_website_is_served_from_its_copy_on_a_port_of_its_own() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Site", "a page", TargetRef::None).unwrap();
        let dist = ws.app_dir().join("dist");
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(dist.join("index.html"), "first build").unwrap();
        let e = Engine::new(crate::testing::session());
        let call = |id: &str, p: Value| e.call(id, p, CallOptions::new(Caller::Cli).with_consent(Consent::Granted)).map(|o| o.to_json()["result"].clone());

        let first = call("project_pin_create", json!({"project": ws.root})).unwrap()["pin"]["id"].clone();
        std::fs::write(dist.join("index.html"), "second build").unwrap();
        let second = call("project_pin_create", json!({"project": ws.root})).unwrap()["pin"]["id"].clone();
        std::fs::write(dist.join("index.html"), "the project now").unwrap();
        let one = call("project_pin_open", json!({"project": ws.root, "pin": first})).unwrap();
        let two = call("project_pin_open", json!({"project": ws.root, "pin": second})).unwrap();
        assert_eq!(one["already_running"], false);
        let (one_url, two_url) = (one["url"].as_str().unwrap(), two["url"].as_str().unwrap());
        assert_ne!(one_url, two_url, "each pin is served on a port of its own");
        assert!(fetch(one_url).ends_with("first build"));
        assert!(fetch(two_url).ends_with("second build"));

        let again = call("project_pin_open", json!({"project": ws.root, "pin": first})).unwrap();
        assert_eq!((again["already_running"].as_bool(), again["url"].as_str()), (Some(true), Some(one_url)));
        let list = call("project_pins_list", json!({"project": ws.root})).unwrap();
        assert_eq!((list["pins"][0]["running"].as_bool(), list["pins"][0]["url"].as_str()), (Some(true), Some(one_url)));
        assert!(call("project_pin_delete", json!({"project": ws.root, "pin": first, "confirm": true})).is_err(), "a served pin isn't removed");

        assert_eq!(call("project_pin_stop", json!({"project": ws.root, "pin": first})).unwrap()["stopped"], 1);
        let list = call("project_pins_list", json!({"project": ws.root})).unwrap();
        assert_eq!(list["pins"][0]["running"], false);
        assert!(list["pins"][0]["url"].is_null());
        assert_eq!(list["pins"][1]["running"], true, "stopping one pin leaves the other served");
        call("project_pin_delete", json!({"project": ws.root, "pin": first, "confirm": true})).unwrap();
    }

    /// A CLI run ends its session with the command, so `wait` keeps a pinned website served for
    /// as long as the call goes on, and stops serving it once the call is cancelled.
    #[test]
    fn a_pinned_website_opened_with_wait_is_served_until_the_cli_call_is_cancelled() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Site", "a page", TargetRef::None).unwrap();
        let dist = ws.app_dir().join("dist");
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(dist.join("index.html"), "pinned build").unwrap();
        let e = Arc::new(Engine::new(crate::testing::session()));
        let opts = || CallOptions::new(Caller::Cli).with_consent(Consent::Granted);
        let pin = e.call("project_pin_create", json!({"project": ws.root}), opts()).unwrap().to_json()["result"]["pin"]["id"].clone();
        let served = Arc::new(Sites::default());
        e.session().ext.insert(served.clone());
        let dir = pins::get(&ws, pin.as_u64().unwrap() as u32).unwrap().app_dir(&ws);

        let cancel = crate::CancelToken::default();
        let (engine, token, project) = (e.clone(), cancel.clone(), ws.root.clone());
        let waiting = std::thread::spawn(move || {
            engine.call("project_pin_open", json!({"project": project, "pin": pin, "wait": true}), opts().with_cancel(token)).map(|o| o.to_json())
        });
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let url = loop {
            if let Some(url) = served.url(&dir) {
                break url;
            }
            assert!(std::time::Instant::now() < until, "it starts serving");
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert!(fetch(&url).ends_with("pinned build"), "served while the call goes on");
        assert!(!waiting.is_finished());
        cancel.cancel();
        let done = waiting.join().unwrap().unwrap();
        assert_eq!(done["result"]["url"].as_str(), Some(url.as_str()));
        assert!(served.url(&dir).is_none(), "serving stops with the call");
    }

    #[test]
    fn an_estimate_error_names_the_project_it_was_for() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Empty", "nothing built", TargetRef::None).unwrap();
        let e = Engine::new(crate::testing::session());
        let err = e
            .call("project_pin_create", json!({"project": &ws.root, "estimate": true}), CallOptions::new(Caller::Cli).with_consent(Consent::Granted))
            .unwrap_err();
        let details = err.details.as_ref().expect("an estimate error names its project");
        assert_eq!(details["project"].as_str(), ws.root.to_str());
        assert_eq!(details["estimate"].as_bool(), Some(true));
    }

    #[test]
    fn an_open_project_error_on_an_estimate_names_the_project() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("missing");
        let e = Engine::new(crate::testing::session());
        let err = e
            .call("project_pin_create", json!({"project": &root, "estimate": true}), CallOptions::new(Caller::Cli).with_consent(Consent::Granted))
            .unwrap_err();
        let details = err.details.as_ref().expect("an estimate open error names its project");
        assert_eq!(details["project"].as_str(), root.to_str());
        assert_eq!(details["estimate"].as_bool(), Some(true));
    }
}
