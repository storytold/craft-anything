//! Pinned builds: a build of a project kept at hand, to open whatever the builder does next.
//!
//! ```text
//! <project>/pinned/
//!   <n>/pin.json       the pin (name, frozen or light, the round it was pinned at)
//!   <n>/app/           a pin's copy of the app: its files and the program, without what a
//!                      build makes again
//!   <n>/last-run.log   what it printed the last time it was opened
//!   play.log           the same for Play
//! ```
//!
//! A **frozen** pin (the default) runs its own copy, so a rebuild can't change or break it, and a
//! running copy never locks the files the builder replaces. A **light** pin copies the program
//! and the app's code the same way but hard-links its dependencies and media to the project's, so
//! those take no more space. A rebuild replaces or deletes files, which leaves the pin's as they
//! were; only a file rewritten in place changes in both. Programs a pin or Play started are kept
//! in [`Runs`], one at a time per folder unless another is asked for.
//!
//! Where copies clone (APFS on macOS, Btrfs or XFS on Linux, ReFS on Windows), a media file with
//! the same bytes as one in an earlier frozen pin is copied from that pin's file rather than from
//! the project's, so the two share their blocks until one is written to; each pin still keeps a
//! file of its own, so writing to one never changes another. Elsewhere (NTFS, ext4) the match
//! would only read each file again and save nothing, so it isn't made.
//!
//! A pin of a **website** copies the built site (`dist/`, `build/`, `out/` or the app itself) into
//! `app/`, and opening it serves that copy on a loopback port of its own ([`Sites`]).

use crate::inbox::now_ms;
use crate::outputs::CargoEnv;
use crate::versioned;
use crate::workspace::{self, Workspace};
use craft_anything_evidence::{ErrorCode, ToolError};
use craft_anything_platform::serve::StaticServer;
use craft_anything_platform::sync::Mutex;
use craft_anything_platform::{Ended, Launch, Launched};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::OnceCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::PoisonError;
use std::time::Duration;

/// The folder in a project that holds its pins.
pub const PINNED: &str = "pinned";
pub const PIN_FILE: &str = "pin.json";
/// Where a pin keeps a program it copied from outside the app (a moved Cargo target directory).
const PIN_PROGRAM: &str = "pin-program";
/// The current `pin.json` format version.
pub const PIN_VERSION: u32 = 3;
/// Version 2 remembers the program selected for an argument-free launch inside the copied app.
/// Version 3 marks a website's pin (`site`): a build that doesn't know it would take the pin for
/// a program and save it as one.
const PIN_STEPS: &[versioned::Step<Value>] = &[pin_v1_to_v2, pin_v2_to_v3];

fn pin_v1_to_v2(doc: &mut Value) -> Result<(), String> {
    let object = doc.as_object_mut().ok_or_else(|| "a pin must be an object".to_string())?;
    object.insert("program".into(), Value::Null);
    Ok(())
}

fn pin_v2_to_v3(doc: &mut Value) -> Result<(), String> {
    let object = doc.as_object_mut().ok_or_else(|| "a pin must be an object".to_string())?;
    object.insert("site".into(), Value::Bool(false));
    Ok(())
}
const _: () = assert!(PIN_STEPS.len() as u32 + 1 == PIN_VERSION, "one step per pin.json version after the first");
/// A start that ends within this long is reported as having quit straight away.
pub const QUICK_EXIT: Duration = Duration::from_secs(2);
/// Most pins one project keeps.
const MAX_PINS: u32 = 999;
/// How much of the end of a run's log is read back.
const TAIL_BYTES: u64 = 8 * 1024;
const TAIL_LINES: usize = 20;
/// Inside a folder a copy leaves out, the folders where a start needs only the files beside its
/// program (a Rust build's `target/release/`); in any other (`node_modules` for Electron) it needs
/// the whole folder.
const PROGRAM_FILES_ONLY: &[&str] = &["target"];
/// Never copied into a pin.
const NEVER_COPIED: &[&str] = &[".git"];
/// What a compiler leaves beside a program that nothing needs to run it. Debug symbols alone were
/// 85% of a typical Rust pin (a 242 MB `.pdb` beside a 49 MB program).
const NOT_NEEDED_TO_RUN: &[&str] = &["pdb", "ilk", "exp", "lib", "d", "rlib", "rmeta"];
/// File kinds a program only reads, which a light pin shares with the project. What a program may
/// write to (a save, a database) isn't here: a pin writing to a shared file would change the
/// project's.
const SHARED_KINDS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "gif", "tga", "dds", "ktx", "ktx2", "exr", "hdr", "mp3", "ogg", "wav", "flac", "mp4", "webm", "mov", "glb", "gltf", "fbx",
    "obj", "ttf", "otf", "woff", "woff2", "pak", "pck",
];

/// How a pin keeps its build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PinMode {
    /// Its own copy of the build, which nothing else changes (the default).
    #[default]
    Frozen,
    /// Its own program and code, sharing dependencies and media with the project.
    Light,
    /// The project's app where it is, as the latest build left it. No longer made (it only
    /// opened what Play opens); pins already saved this way still work.
    Live,
}

impl PinMode {
    /// Has a folder of its own to run from.
    pub fn keeps_copy(self) -> bool {
        self != PinMode::Live
    }
}

/// One pin (`pinned/<id>/pin.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pin {
    pub version: u32,
    pub id: u32,
    pub name: String,
    pub mode: PinMode,
    pub created_ms: u64,
    /// The build round it was pinned at, when the project had checked one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round: Option<u32>,
    /// What its copy takes on disk.
    #[serde(default)]
    pub bytes: u64,
    /// What a light pin shares with the project.
    #[serde(default)]
    pub shared_bytes: u64,
    /// Bytes that were meant to be shared (hard-linked) but had to be copied instead, because the
    /// drive or the file didn't allow hard links. Only meaningful for light pins.
    #[serde(default)]
    pub copied_shared_bytes: u64,
    /// The selected program relative to the copied app, for a launch without arguments.
    /// Older pins and launches through an external runtime keep the existing discovery path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<PathBuf>,
    /// A website: its copy is the built site, served rather than started.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub site: bool,
}

impl Pin {
    pub fn dir(&self, ws: &Workspace) -> PathBuf {
        pin_dir(ws, self.id)
    }

    /// The folder its program runs from: its copy, or the project's app for a live pin.
    /// For a website, the folder that is served.
    pub fn app_dir(&self, ws: &Workspace) -> PathBuf {
        match self.mode {
            PinMode::Frozen | PinMode::Light => self.dir(ws).join("app"),
            PinMode::Live => ws.app_dir(),
        }
    }

    pub fn log_path(&self, ws: &Workspace) -> PathBuf {
        self.dir(ws).join("last-run.log")
    }

    /// Resolve the recorded program within the copy; old pins retain their discovery behavior. A
    /// saved Live pin finds the project's build where `cargo` says Cargo puts it.
    pub(crate) fn launch(&self, ws: &Workspace, godot: Option<&Path>, cargo: &CargoEnv) -> Result<(PathBuf, Vec<String>), ToolError> {
        let app = self.app_dir(ws);
        if self.mode.keeps_copy()
            && let Some(relative) = &self.program
        {
            if relative.as_os_str().is_empty() || !relative.components().all(|part| matches!(part, std::path::Component::Normal(_))) {
                return Err(ToolError::invalid("The pin's recorded program must be a relative path inside its app."));
            }
            let program = app.join(relative);
            if !program.is_file() || !crate::outputs::is_program(&program) {
                return Err(ToolError::new(ErrorCode::NotFound, "The program recorded by this pin is missing or cannot be started.")
                    .with_remediation("Restore the pin's program or pin a working build again."));
            }
            let resolved = program.canonicalize().map_err(|e| ToolError::io(program.display(), e))?;
            let root = app.canonicalize().map_err(|e| ToolError::io(app.display(), e))?;
            if !resolved.starts_with(root) {
                return Err(ToolError::invalid("The pin's recorded program points outside its app."));
            }
            return Ok((program, Vec::new()));
        }
        // A pin from before programs were recorded: its own copy decides, not where Cargo builds
        // the project now (a global target-dir would point at the latest build). A saved Live pin
        // has no copy: it starts the project's app, whose build may be wherever Cargo put it.
        let found = if self.mode.keeps_copy() { crate::outputs::launch_inside(&app, godot) } else { crate::outputs::launch_in(&app, godot, cargo) };
        found.ok_or_else(|| {
            ToolError::new(ErrorCode::NotFound, "There's nothing to start: no built program in this pin.")
                .with_remediation("Pin a working build or open the folder to look.")
        })
    }
}

fn pin_dir(ws: &Workspace, id: u32) -> PathBuf {
    ws.root.join(PINNED).join(id.to_string())
}

/// Where Play's output goes.
pub fn play_log(ws: &Workspace) -> PathBuf {
    ws.root.join(PINNED).join("play.log")
}

/// Read a `pin.json` of any version this build understands; a newer one is read as far as its
/// fields are known. Only a missing version means version 1.
pub fn read_pin(bytes: &[u8]) -> Result<Pin, ToolError> {
    let mut doc: Value = serde_json::from_slice(bytes).map_err(|e| ToolError::invalid(format!("it isn't valid JSON: {e}")))?;
    let found = match doc.get("version") {
        None => 1,
        Some(v) => v.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| ToolError::invalid(format!("its version ({v}) isn't a whole number")))?,
    };
    if found <= PIN_VERSION {
        let reached = versioned::upgrade(&mut doc, found, 1, PIN_STEPS).map_err(|e| ToolError::invalid(format!("it couldn't be brought up to date: {e}")))?;
        if let Some(obj) = doc.as_object_mut() {
            obj.insert("version".into(), Value::from(reached));
        }
    }
    serde_json::from_value(doc).map_err(|e| ToolError::invalid(e.to_string()))
}

fn save(ws: &Workspace, pin: &Pin) -> Result<(), ToolError> {
    let json = serde_json::to_vec_pretty(pin).map_err(|e| ToolError::internal(e.to_string()))?;
    crate::project::write_atomic(&pin.dir(ws).join(PIN_FILE), &json)
}

/// A project's pins, oldest first. Folders without a readable pin.json (a copy still being made,
/// or one that stopped part-way) are skipped.
pub fn list(ws: &Workspace) -> Vec<Pin> {
    let mut pins: Vec<Pin> = std::fs::read_dir(ws.root.join(PINNED))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.parse::<u32>().is_ok()))
        .filter_map(|e| std::fs::read(e.path().join(PIN_FILE)).ok())
        .filter_map(|b| read_pin(&b).ok())
        .collect();
    pins.sort_by_key(|p| p.id);
    pins
}

pub fn get(ws: &Workspace, id: u32) -> Result<Pin, ToolError> {
    list(ws)
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| ToolError::new(ErrorCode::NotFound, format!("This project has no pin {id}.")).with_remediation("Use project_pins_list to see its pins."))
}

/// What a pin of the project's build would take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Size {
    /// A frozen copy.
    pub bytes: u64,
    pub files: u64,
    /// A light copy.
    pub light_bytes: u64,
}

/// One file of a pin's copy.
struct Planned {
    from: PathBuf,
    to: PathBuf,
    len: u64,
    /// A light pin may share it with the project.
    shared: bool,
    /// Made as a link to this path, relative to where it goes, when a link can be made.
    link: Option<PathBuf>,
}

/// What a copy of `app` keeps of the folders a copy normally leaves out, for its start.
enum Keep {
    Whole(PathBuf),
    FilesIn(PathBuf),
}

/// The selected program's path under the app when it is inside it, from relative or absolute paths.
fn inside_app(app: &Path, program: &Path) -> Option<PathBuf> {
    let app = workspace::normalized(app);
    let program = workspace::normalized(program);
    program.strip_prefix(&app).ok().map(Path::to_path_buf)
}

/// The program a pin records: its path in the app, or the pin-owned copy of one built outside it.
fn pin_program(app: &Path, program: &Path) -> Option<PathBuf> {
    inside_app(app, program).or_else(|| program.file_name().map(|name| Path::new(PIN_PROGRAM).join(name)))
}

/// What a pin copies: the app without what a build makes again, the project's own records (an app
/// edited in place) or `.git`; plus what starting it needs from those folders.
struct Plan {
    app: PathBuf,
    keep: Vec<Keep>,
    /// The program's folder: what's in it is always copied, since a running program locks it.
    program_dir: Option<PathBuf>,
    /// A program from outside the app: its folder, where it goes in the pin, and its file name.
    external: Option<(PathBuf, PathBuf, std::ffi::OsString)>,
}

impl Plan {
    fn new(ws: &Workspace, program: &Path, copy_external: bool) -> Plan {
        let app = ws.app_dir();
        let program_abs = workspace::normalized(program);
        let relative = inside_app(&app, &program_abs);
        let in_app_parent = relative.as_ref().and_then(|rel| rel.parent()).map(|rel| app.join(rel));
        let mut keep = Vec::new();
        if let Some(rel) = &relative
            && let Some(first) = rel.components().next()
        {
            let top = app.join(first);
            let only_files = first.as_os_str().to_str().is_some_and(|n| PROGRAM_FILES_ONLY.contains(&n));
            match &in_app_parent {
                Some(dir) if only_files => keep.push(Keep::FilesIn(dir.clone())),
                _ if workspace::rebuilt(&top) => keep.push(Keep::Whole(top)),
                _ => {}
            }
        }
        // A Godot project runs from its imported resources.
        let imported = app.join(".godot");
        if app.join("project.godot").is_file() && imported.is_dir() {
            keep.push(Keep::Whole(imported));
        }
        let external = if relative.is_none() && copy_external {
            match (program_abs.parent(), program_abs.file_name()) {
                (Some(source), Some(name)) => Some((source.to_path_buf(), PathBuf::from(PIN_PROGRAM), name.to_os_string())),
                _ => None,
            }
        } else {
            None
        };
        let program_dir = external.as_ref().map(|(source, _, _)| source.clone()).or(in_app_parent);
        Plan { app, keep, program_dir, external }
    }

    /// Every (file, where it goes under `to`) the copy makes, in order.
    fn files(&self, ws: &Workspace, to: &Path) -> Result<Vec<Planned>, ToolError> {
        let (mut own, mut dependencies) = (Vec::new(), Vec::new());
        let skip = |p: &Path| workspace::rebuilt(p) || ws.is_own_record(p) || never_copied(p) || stray_copy(p);
        walk(&self.app, to, &skip, 0, &mut own)?;
        for k in &self.keep {
            let (from, whole) = match k {
                Keep::Whole(d) => (d, true),
                Keep::FilesIn(d) => (d, false),
            };
            let rel = from.strip_prefix(&self.app).map_err(|_| ToolError::internal("a kept folder isn't inside the app"))?;
            if whole {
                walk(from, &to.join(rel), &|p| never_copied(p) || stray_copy(p), 0, &mut dependencies)?;
            } else {
                for e in std::fs::read_dir(from).map_err(|e| ToolError::io(from.display(), e))?.flatten() {
                    if let Ok(m) = e.metadata()
                        && m.is_file()
                        && !compiler_leftover(&e.path())
                        && !stray_copy(&e.path())
                    {
                        own.push((e.path(), to.join(rel).join(e.file_name()), m.len()));
                    }
                }
            }
        }
        if let Some((source, dest, name)) = &self.external {
            for e in std::fs::read_dir(source).map_err(|e| ToolError::io(source.display(), e))?.flatten() {
                let path = e.path();
                let file_name = e.file_name();
                // A shared target directory holds other projects' programs too: copy the one this
                // pin starts and the files beside it, not every executable in the folder.
                let selected = file_name.as_os_str() == name.as_os_str();
                if !selected && crate::outputs::is_program(&path) {
                    continue;
                }
                if let Ok(m) = e.metadata()
                    && m.is_file()
                    && (selected || (!compiler_leftover(&path) && !stray_copy(&path)))
                {
                    own.push((path, to.join(dest).join(file_name), m.len()));
                }
            }
        }
        let beside_program = |p: &Path| p.parent() == self.program_dir.as_deref();
        let own = own.into_iter().map(|f| (f, true));
        let dependencies = dependencies.into_iter().map(|f| (f, false));
        Ok(own
            .chain(dependencies)
            .map(|((from, to, len), is_own)| {
                let shared = !beside_program(&from) && (!is_own || media(&from));
                Planned { from, to, len, shared, link: None }
            })
            .collect())
    }
}

fn media(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| SHARED_KINDS.contains(&e.to_ascii_lowercase().as_str()))
}

fn never_copied(p: &Path) -> bool {
    p.file_name().and_then(|n| n.to_str()).is_some_and(|n| NEVER_COPIED.contains(&n))
}

/// Every file of a built website under `to`: all of it is the site's, so nothing in it is left out
/// as compiler output. A light pin shares its media.
fn site_files(ws: &Workspace, site: &Path, to: &Path) -> Result<Vec<Planned>, ToolError> {
    let mut found = Vec::new();
    // The preview server serves a link that stays inside the site as what it points to; the pin
    // must too, so those are copied as their targets.
    let root = std::fs::canonicalize(site).map_err(|e| ToolError::io(site.display(), e))?;
    let mut within = Within { chain: vec![root.clone()], root, to: to.to_path_buf(), entries: 0, links: HashMap::new() };
    walk_in(site, to, &|p| ws.is_own_record(p) || never_copied(p), Some(&mut within), 0, &mut found)?;
    Ok(found.into_iter().map(|(from, to, len)| Planned { shared: media(&from), link: within.links.remove(&to), from, to, len }).collect())
}

fn compiler_leftover(p: &Path) -> bool {
    let kind = p.extension().and_then(|e| e.to_str()).is_some_and(|e| NOT_NEEDED_TO_RUN.contains(&e.to_ascii_lowercase().as_str()));
    kind || p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(".cargo-"))
}

/// An old copy of a program, left behind when a build replaced one that was running
/// (`snake.running-144900.exe`): a pin copies the program, not each copy of it. It only counts as
/// one when the program it copies is still beside it, so a helper that happens to be named like
/// one stays.
fn stray_copy(p: &Path) -> bool {
    let Some(name) = p.file_name().and_then(|n| n.to_str()) else { return false };
    let Some((base, rest)) = name.rsplit_once(".running-") else { return false };
    let (digits, ext) = rest.split_once('.').map_or((rest, None), |(d, e)| (d, Some(e)));
    if base.is_empty() || digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let original = match ext {
        Some(e) => format!("{base}.{e}"),
        None => base.to_string(),
    };
    let original = p.with_file_name(original);
    original.is_file() && crate::outputs::is_program(&original)
}

/// How deep a copy goes.
const WALK_DEPTH: usize = 64;
/// Most files and folders a website's copy walks, counting each time a link leads to one again:
/// a folder linked from each of 24 folders nested in one another is walked 2^24 times.
const WALK_ENTRIES: usize = 100_000;

fn walk(from: &Path, to: &Path, skip: &dyn Fn(&Path) -> bool, depth: usize, out: &mut Vec<(PathBuf, PathBuf, u64)>) -> Result<(), ToolError> {
    walk_in(from, to, skip, None, depth, out)
}

/// The canonical folder whose links [`walk_in`] follows, and the real folders it is walking into.
struct Within {
    root: PathBuf,
    chain: Vec<PathBuf>,
    /// Where `root` is copied to.
    to: PathBuf,
    /// Files and folders walked so far, up to [`WALK_ENTRIES`].
    entries: usize,
    /// Each copy of a linked file named unlike it (`main -> build/app.js`), and the way from that
    /// copy's folder to the file's own copy: the server picks a file's kind by its name, so the
    /// copy stays a link to it, as the preview serves it.
    links: HashMap<PathBuf, PathBuf>,
}

/// [`walk`], also following each link whose target is inside `within`'s root, from that target;
/// other links are left out, and so are links to what `skip` leaves out or to a folder being walked.
fn walk_in(
    from: &Path,
    to: &Path,
    skip: &dyn Fn(&Path) -> bool,
    mut within: Option<&mut Within>,
    depth: usize,
    out: &mut Vec<(PathBuf, PathBuf, u64)>,
) -> Result<(), ToolError> {
    if depth > WALK_DEPTH {
        return Err(ToolError::new(ErrorCode::OverLimit, format!("{} is nested more than {WALK_DEPTH} folders deep.", from.display())));
    }
    for entry in std::fs::read_dir(from).map_err(|e| ToolError::io(from.display(), e))? {
        let entry = entry.map_err(|e| ToolError::io(from.display(), e))?;
        let (mut path, mut kind) = (entry.path(), entry.file_type().map_err(|e| ToolError::io(entry.path().display(), e))?);
        if skip(&path) {
            continue;
        }
        if let Some(w) = within.as_deref_mut() {
            w.entries += 1;
            if w.entries > WALK_ENTRIES {
                return Err(ToolError::new(
                    ErrorCode::OverLimit,
                    format!("{} holds more than {WALK_ENTRIES} files and folders, counting each one its links lead to.", w.root.display()),
                )
                .with_remediation("Remove the links that lead to the same folders again, then pin it."));
            }
        }
        let linked = kind.is_symlink();
        if linked {
            let Some(w) = within.as_deref() else { continue };
            let Some(target) = std::fs::canonicalize(&path).ok().filter(|t| t.starts_with(&w.root)) else { continue };
            // A link into `.git` or the project's records, or back up to a folder being walked
            // (`self -> .`, which would never end), isn't followed.
            if target.ancestors().take_while(|a| *a != w.root).any(skip) || w.chain.contains(&target) {
                continue;
            }
            kind = std::fs::metadata(&target).map_err(|e| ToolError::io(target.display(), e))?.file_type();
            path = target;
        }
        let dest = to.join(entry.file_name());
        if kind.is_dir() {
            match within.as_deref_mut() {
                Some(w) => {
                    let real = if linked { path.clone() } else { w.chain.last().map_or_else(|| path.clone(), |r| r.join(entry.file_name())) };
                    w.chain.push(real);
                    let walked = walk_in(&path, &dest, skip, Some(&mut *w), depth + 1, out);
                    w.chain.pop();
                    walked?;
                }
                None => walk_in(&path, &dest, skip, None, depth + 1, out)?,
            }
        } else if kind.is_file() {
            if linked
                && let Some(w) = within.as_deref_mut()
                && !same_extension(Path::new(&entry.file_name()), &path)
                && let (Ok(real), Ok(here)) = (path.strip_prefix(&w.root), to.strip_prefix(&w.to))
            {
                let up: PathBuf = here.components().map(|_| "..").collect();
                w.links.insert(dest.clone(), up.join(real));
            }
            let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            out.push((path, dest, len));
        }
    }
    Ok(())
}

fn same_extension(a: &Path, b: &Path) -> bool {
    let extension = |p: &Path| p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
    extension(a) == extension(b)
}

/// What a build in `app` is: a website's folder, or a program and its arguments (see
/// `outputs::launch_now`).
enum Built {
    Site(PathBuf),
    Program(PathBuf, Vec<String>),
}

fn built_in(app: &Path, godot: Option<&Path>) -> Result<Built, ToolError> {
    if let Some(site) = crate::outputs::site_dir(app) {
        return Ok(Built::Site(site));
    }
    let (program, args) = crate::outputs::launch_now(app, godot).ok_or_else(|| {
        ToolError::new(ErrorCode::NotFound, "There's nothing to pin yet: no built program in this project.")
            .with_remediation("Finish a build first (rebuild), then pin it.")
    })?;
    Ok(Built::Program(program, args))
}

/// The files a pin of `built` copies, under `to`.
fn planned(ws: &Workspace, built: &Built, to: &Path) -> Result<Vec<Planned>, ToolError> {
    match built {
        Built::Site(site) => site_files(ws, site, to),
        Built::Program(program, args) => Plan::new(ws, program, args.is_empty()).files(ws, to),
    }
}

/// What a pin of the project's current build would take on disk.
pub fn estimate(ws: &Workspace, godot: Option<&Path>) -> Result<Size, ToolError> {
    let files = planned(ws, &built_in(&ws.app_dir(), godot)?, Path::new(""))?;
    let bytes = |only_copied: bool| files.iter().filter(|f| !(only_copied && f.shared)).fold(0u64, |n, f| n.saturating_add(f.len));
    Ok(Size { bytes: bytes(false), files: files.len() as u64, light_bytes: bytes(true) })
}

/// Pin the project's current build. `progress(done, total)` hears how far the copy is (bytes) and
/// stops it by returning false, which removes what was copied.
pub fn create(ws: &Workspace, mode: PinMode, name: Option<&str>, godot: Option<&Path>, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<Pin, ToolError> {
    if !mode.keeps_copy() {
        return Err(ToolError::invalid("Live pins aren't made any more: one only opened the latest build, as Play does.")
            .with_remediation("Pin it as frozen or light."));
    }
    let built = built_in(&ws.app_dir(), godot)?;
    let recorded_program = match &built {
        Built::Program(program, args) if args.is_empty() => pin_program(&ws.app_dir(), program),
        _ => None,
    };
    let site = matches!(built, Built::Site(_));
    let parent = ws.root.join(PINNED);
    std::fs::create_dir_all(&parent).map_err(|e| ToolError::io(parent.display(), e))?;
    let mut id = list(ws).iter().map(|p| p.id).max().unwrap_or(0).saturating_add(1);
    // create_dir (not _all) fails if the folder exists, so two pins never share one.
    let dir = loop {
        let d = parent.join(id.to_string());
        match std::fs::create_dir(&d) {
            Ok(()) => break d,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && id < MAX_PINS => id += 1,
            Err(e) => return Err(ToolError::io(d.display(), e)),
        }
    };
    let round = ws.manifest.checks.iter().map(|c| c.round).max();
    let project = ws.manifest.name.as_str();
    let name = match (name.map(str::trim).filter(|n| !n.is_empty()), round) {
        (Some(n), _) => n.to_string(),
        (None, Some(r)) => format!("{project} · round {r}"),
        (None, None) => format!("{project} · pin {id}"),
    };
    let mut pin = Pin {
        version: PIN_VERSION,
        id,
        name,
        mode,
        created_ms: now_ms(),
        round,
        bytes: 0,
        shared_bytes: 0,
        copied_shared_bytes: 0,
        program: recorded_program,
        site,
    };
    let made = (|| {
        let files = planned(ws, &built, &pin.app_dir(ws))?;
        let pinned = if craft_anything_platform::fs::clones_in(&ws.root.join(PINNED)) { PinnedMedia::of(ws) } else { PinnedMedia::default() };
        (pin.bytes, pin.shared_bytes, pin.copied_shared_bytes) = copy_files(&files, mode, &pinned, progress, &|from, to| std::fs::hard_link(from, to))?;
        // Only copies are made here, and a copy's program is looked for inside it, where no
        // Cargo environment applies.
        if !site {
            pin.launch(ws, godot, &CargoEnv::default())?;
        } else if !pin.app_dir(ws).join("index.html").is_file() {
            // The site was found by an index.html that leads out of it, which the copy leaves out.
            return Err(ToolError::new(ErrorCode::NotFound, "There's nothing to serve in this pin: the website's index.html isn't part of the site.")
                .with_remediation("Make index.html a file inside the built site (not a link out of it), then pin it."));
        }
        save(ws, &pin)
    })();
    if let Err(e) = made {
        // Nothing half-made stays behind: no pin.json means no pin, and the copy goes.
        if let Err(rm) = std::fs::remove_dir_all(&dir) {
            log::warn!("a pin that wasn't finished couldn't be removed from {}: {rm}", dir.display());
        }
        return Err(e);
    }
    Ok(pin)
}

/// Make a pin's copy of a program's build, with the way a file is linked given as a parameter, so
/// a test can make linking fail without needing a drive that can't link.
#[cfg(test)]
fn copy_linking(
    ws: &Workspace,
    program: &Path,
    copy_external: bool,
    to: &Path,
    mode: PinMode,
    progress: &mut dyn FnMut(u64, u64) -> bool,
    link: &dyn Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<(u64, u64, u64), ToolError> {
    copy_files(&Plan::new(ws, program, copy_external).files(ws, to)?, mode, &PinnedMedia::of(ws), progress, link)
}

/// Media already in the project's frozen pins, by size, so a new pin copies a file with the same
/// bytes from there where copies clone (see the module docs). Only frozen pins: a light pin's media are the project's
/// own files, which a rebuild may rewrite.
#[derive(Default)]
struct PinnedMedia(HashMap<u64, Vec<(PathBuf, LazyDigest)>>);

/// A file's SHA-256, read when first asked for; `None` when it can't be read.
type LazyDigest = OnceCell<Option<[u8; 32]>>;

impl PinnedMedia {
    fn of(ws: &Workspace) -> PinnedMedia {
        let mut found = Vec::new();
        for pin in list(ws).iter().filter(|p| p.mode == PinMode::Frozen) {
            let app = pin.app_dir(ws);
            // A pin that can't be read through only shares what was read of it.
            if let Err(e) = walk(&app, &app, &|_| false, 0, &mut found) {
                log::debug!("pin {} is only partly shared with: {}", pin.id, e.message);
            }
        }
        let mut by_len: HashMap<u64, Vec<_>> = HashMap::new();
        for (path, _, len) in found {
            if len > 0 && media(&path) {
                by_len.entry(len).or_default().push((path, OnceCell::new()));
            }
        }
        PinnedMedia(by_len)
    }

    /// A pinned file with the same bytes as `from`, and their digest; each pinned file is read for
    /// its digest once at most.
    fn same_as(&self, from: &Path, len: u64) -> Option<(&Path, [u8; 32])> {
        let candidates = self.0.get(&len)?;
        let wanted = digest(from)?;
        candidates.iter().find(|(path, d)| *d.get_or_init(|| digest(path)) == Some(wanted)).map(|(path, _)| (path.as_path(), wanted))
    }
}

fn digest(p: &Path) -> Option<[u8; 32]> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(p).ok()?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash).ok()?;
    Some(hash.finalize().into())
}

/// Copy `files`; what it took and what a light pin shares with the project. A file that can't be
/// linked (a drive without hard links, a file with as many as it can have) is copied. A media file
/// equal to one in `pinned` is copied from there, with the project file's permissions, and again
/// from the project when the copy turns out to hold other bytes (that file changed after it was
/// read).
fn copy_files(
    files: &[Planned],
    mode: PinMode,
    pinned: &PinnedMedia,
    progress: &mut dyn FnMut(u64, u64) -> bool,
    link: &dyn Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<(u64, u64, u64), ToolError> {
    let total = files.iter().fold(0u64, |n, f| n.saturating_add(f.len));
    let (mut done, mut shared, mut copied_shared) = (0u64, 0u64, 0u64);
    for f in files {
        if !progress(done, total) {
            return Err(ToolError::new(ErrorCode::Cancelled, "Pinning was stopped; nothing was kept."));
        }
        if let Some(d) = f.to.parent() {
            std::fs::create_dir_all(d).map_err(|e| ToolError::io(d.display(), e))?;
        }
        // A drive or system that can't make the link gets a copy instead.
        if f.link.as_deref().is_some_and(|target| symlink_file(target, &f.to).is_ok()) {
            done = done.saturating_add(f.len);
            continue;
        }
        if mode == PinMode::Light && f.shared && link(&f.from, &f.to).is_ok() {
            shared = shared.saturating_add(f.len);
        } else {
            // Never a link to another pin's file: a frozen pin's copy is its own.
            let same = if f.shared && media(&f.from) { pinned.same_as(&f.from, f.len) } else { None };
            let from_pin = same.is_some_and(|(pinned, wanted)| copy_from_pin(pinned, &f.from, &f.to, wanted));
            if !from_pin {
                std::fs::copy(&f.from, &f.to).map_err(|e| {
                    ToolError::io(f.from.display(), e)
                        .with_remediation("Check there's room on the drive for the copy, or pin it as a light copy, which takes less.")
                })?;
            }
            if mode == PinMode::Light && f.shared {
                copied_shared = copied_shared.saturating_add(f.len);
            }
        }
        done = done.saturating_add(f.len);
    }
    progress(total, total);
    Ok((total.saturating_sub(shared), shared, copied_shared))
}

#[cfg(unix)]
fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Windows makes one only in Developer Mode or for an admin.
#[cfg(windows)]
fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(not(any(unix, windows)))]
fn symlink_file(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// Copy `pinned` to `to` with the permissions of `from`, the project's file, if the copy holds
/// `wanted`; otherwise nothing is left at `to`, so the project's file can be copied there instead.
fn copy_from_pin(pinned: &Path, from: &Path, to: &Path, wanted: [u8; 32]) -> bool {
    let copied = craft_anything_platform::fs::clone_or_copy(pinned, to).is_ok()
        && digest(to) == Some(wanted)
        && std::fs::metadata(from).and_then(|m| std::fs::set_permissions(to, m.permissions())).is_ok();
    if !copied
        && let Err(e) = std::fs::remove_file(to)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::debug!("couldn't remove {}: {e}", to.display());
    }
    copied
}

/// Give a pin a new name.
pub fn rename(ws: &Workspace, id: u32, name: &str) -> Result<Pin, ToolError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ToolError::invalid("A pin needs a name."));
    }
    let mut pin = get(ws, id)?;
    // Saving would drop the fields this build doesn't know (docs/file-formats.md).
    if pin.version > PIN_VERSION {
        return Err(ToolError::unsupported("This pin was saved by a newer Craft Anything, so it isn't changed.")
            .with_remediation("Update Craft Anything to rename it."));
    }
    pin.name = name.to_string();
    save(ws, &pin)?;
    Ok(pin)
}

/// Remove a pin and its copy, with anything it saved there; what it shared stays with the project.
pub fn delete(ws: &Workspace, id: u32) -> Result<(), ToolError> {
    let pin = get(ws, id)?;
    let dir = pin.dir(ws);
    // pin.json first: a delete that stops part-way leaves no half-pin in the list.
    let file = dir.join(PIN_FILE);
    std::fs::remove_file(&file).map_err(|e| ToolError::io(file.display(), e))?;
    std::fs::remove_dir_all(&dir).map_err(|e| ToolError::io(dir.display(), e).with_remediation("Close what it opened, then remove it again."))
}

/// The last lines a run printed.
pub fn tail(log: &Path) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(log) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if f.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES))).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    if f.take(TAIL_BYTES).read_to_end(&mut buf).is_err() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&buf);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines.iter().skip(lines.len().saturating_sub(TAIL_LINES)).copied().collect::<Vec<_>>().join("\n")
}

/// What opening something did.
#[derive(Debug, Clone)]
pub enum Opened {
    /// It was already running, and no other copy was asked for.
    AlreadyRunning(Launched),
    /// Started; `quit` when it ended within [`QUICK_EXIT`].
    Started { launched: Launched, quit: Option<Ended> },
}

/// Programs started from pins and Play in this session, by the folder they run from. Starts
/// after a restart aren't known (a pin then shows as not running).
#[derive(Debug, Default)]
pub struct Runs(Mutex<HashMap<String, Vec<Launched>>>);

/// One key per folder (case and slashes don't matter on Windows).
/// One folder, however its path was written (relative, with `..`, through a link), is one key.
fn key(dir: &Path) -> String {
    let dir = followed(dir);
    let s = dir.to_string_lossy();
    if cfg!(windows) { s.to_lowercase().replace('\\', "/").trim_end_matches('/').to_string() } else { s.trim_end_matches('/').to_string() }
}

/// `dir` made absolute, its deepest part that exists with links followed and the rest with `..`
/// resolved as written, so `/a/link/../pin` is where the link leads and a removed folder keeps
/// the path it had.
fn followed(dir: &Path) -> PathBuf {
    let dir = workspace::absolute(dir);
    let mut rest = Vec::new();
    let mut at = dir.as_path();
    let mut real = loop {
        if let Ok(real) = std::fs::canonicalize(at) {
            break real;
        }
        let (Some(parent), Some(last)) = (at.parent(), at.components().next_back()) else { break at.to_path_buf() };
        rest.push(last);
        at = parent;
    };
    real.extend(rest.iter().rev());
    workspace::normalized(&real)
}

impl Runs {
    /// What's still running from `dir`.
    pub fn running(&self, dir: &Path) -> Vec<Launched> {
        let mut map = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let k = key(dir);
        let alive: Vec<Launched> = map.get(&k).into_iter().flatten().filter(|l| l.is_running()).cloned().collect();
        if alive.is_empty() {
            map.remove(&k);
        } else {
            map.insert(k, alive.clone());
        }
        alive
    }

    /// Start `launch` from `dir`, unless something already runs from there and `another` isn't
    /// asked for. Waits up to [`QUICK_EXIT`] to see whether it quits straight away.
    pub fn open(&self, dir: &Path, launch: Launch, another: bool) -> Result<Opened, ToolError> {
        let launched = {
            let mut map = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            let list = map.entry(key(dir)).or_default();
            list.retain(Launched::is_running);
            if !another && let Some(first) = list.first() {
                return Ok(Opened::AlreadyRunning(first.clone()));
            }
            let launched = launch.start_watched()?;
            list.push(launched.clone());
            launched
        };
        let quit = launched.wait_timeout(QUICK_EXIT);
        Ok(Opened::Started { launched, quit })
    }

    /// Stop everything running from `dir`; how many were stopped.
    pub fn stop(&self, dir: &Path) -> usize {
        let list = self.0.lock().unwrap_or_else(PoisonError::into_inner).remove(&key(dir)).unwrap_or_default();
        list.iter().filter(|l| l.is_running()).filter(|l| l.stop()).count()
    }
}

/// Pinned websites served in this session, by the folder they're served from: each on a loopback
/// port of its own, until it's stopped or the session ends.
#[derive(Default)]
pub struct Sites(Mutex<HashMap<String, StaticServer>>);

impl Sites {
    /// The address `dir` is served at, if it is.
    pub fn url(&self, dir: &Path) -> Option<String> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).get(&key(dir)).map(StaticServer::url)
    }

    /// Serve `dir` unless it's served already; its address, and whether it already was.
    pub fn open(&self, dir: &Path) -> Result<(String, bool), ToolError> {
        let mut map = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(server) = map.get(&key(dir)) {
            return Ok((server.url(), true));
        }
        let server = craft_anything_platform::serve::serve_dir(dir).map_err(|e| ToolError::io(dir.display(), e))?;
        let url = server.url();
        map.insert(key(dir), server);
        Ok((url, false))
    }

    /// Stop serving `dir`; how many servers were stopped.
    pub fn stop(&self, dir: &Path) -> usize {
        let server = self.0.lock().unwrap_or_else(PoisonError::into_inner).remove(&key(dir));
        usize::from(server.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{CheckResult, TargetRef};

    /// A project whose app is a Rust workspace with a built program, a dependency folder and a
    /// file the app keeps beside itself.
    fn built(dir: &Path) -> Workspace {
        let mut ws = Workspace::create(dir, "Snake", "a snake game", TargetRef::None).unwrap();
        ws.manifest.checks.push(CheckResult::new("build", 3, true, "Build succeeded."));
        ws.save().unwrap();
        let app = ws.app_dir();
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::write(app.join("Cargo.toml"), "[package]\nname = \"snake\"\n[dependencies]\neframe = \"0.1\"\n").unwrap();
        std::fs::write(app.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::create_dir_all(app.join("assets")).unwrap();
        std::fs::write(app.join("assets/level.txt"), "####").unwrap();
        let release = app.join("target/release");
        std::fs::create_dir_all(release.join("deps")).unwrap();
        std::fs::write(release.join(exe("snake")), b"program").unwrap();
        std::fs::write(release.join("deps/big.rlib"), vec![0u8; 4096]).unwrap();
        std::fs::write(release.join("snake.pdb"), vec![0u8; 4096]).unwrap();
        std::fs::write(release.join(".cargo-lock"), "").unwrap();
        std::fs::create_dir_all(app.join(".git")).unwrap();
        std::fs::write(app.join(".git/HEAD"), "ref").unwrap();
        mark_program(&release.join(exe("snake")));
        ws
    }

    fn exe(name: &str) -> String {
        if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
    }

    #[cfg(unix)]
    fn mark_program(p: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(not(unix))]
    fn mark_program(_: &Path) {}

    #[test]
    fn recorded_program_paths_cannot_escape_the_pin() {
        let d = tempfile::tempdir().unwrap();
        let ws = built(d.path());
        let mut pin = create(&ws, PinMode::Frozen, None, None, &mut |_, _| true).unwrap();
        assert_eq!(pin.program, Some(PathBuf::from("target/release").join(exe("snake"))));
        for relative in [PathBuf::new(), PathBuf::from("../other"), d.path().join("outside")] {
            pin.program = Some(relative);
            assert_eq!(pin.launch(&ws, None, &CargoEnv::default()).unwrap_err().code, ErrorCode::BadInput);
        }
    }

    /// A pin made before programs were recorded starts its own copy's program, even when a Cargo
    /// setting above it (or `CARGO_TARGET_DIR`) names a build folder with a newer build of the
    /// project in it.
    #[test]
    fn an_older_pin_starts_its_own_copy_not_the_projects_newest_build() {
        let d = tempfile::tempdir().unwrap();
        let ws = built(d.path());
        let mut pin = create(&ws, PinMode::Frozen, None, None, &mut |_, _| true).unwrap();
        pin.program = None;
        let own = pin.app_dir(&ws).join("target/release").join(exe("snake"));
        assert_eq!(pin.launch(&ws, None, &CargoEnv::default()).unwrap().0, own);
        std::fs::create_dir_all(ws.root.join(".cargo")).unwrap();
        std::fs::write(ws.root.join(".cargo/config.toml"), "[build]\ntarget-dir = \"shared-out\"\n").unwrap();
        let newer = ws.root.join("shared-out/release").join(exe("snake"));
        std::fs::create_dir_all(newer.parent().unwrap()).unwrap();
        std::fs::write(&newer, b"the project's next build").unwrap();
        mark_program(&newer);
        std::fs::write(newer.with_extension("d"), format!("{}: {}\n", newer.display(), ws.app_dir().join("src/main.rs").display())).unwrap();
        assert_eq!(pin.launch(&ws, None, &CargoEnv::default()).unwrap().0, own, "the pin's own copy, not the project's newest build");
    }

    /// A saved Live pin (made before they were retired) starts the project's own app, so it finds
    /// the project's build wherever Cargo puts it, as Play does: here a target directory set above
    /// the project.
    #[test]
    fn a_saved_live_pin_finds_the_projects_build_outside_the_app() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Snake", "a snake game", TargetRef::None).unwrap();
        let app = ws.app_dir();
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::write(app.join("Cargo.toml"), "[package]\nname = \"live-probe\"\n").unwrap();
        std::fs::write(app.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::create_dir_all(app.join(".cargo")).unwrap();
        std::fs::write(app.join(".cargo/config.toml"), "[build]\ntarget-dir = \"../shared-out\"\n").unwrap();
        let shared = ws.root.join("shared-out/debug");
        std::fs::create_dir_all(&shared).unwrap();
        let program = shared.join(exe("live-probe"));
        std::fs::write(&program, b"program").unwrap();
        mark_program(&program);
        std::fs::write(shared.join("live-probe.d"), format!("{}: {}\n", program.display(), app.join("src/main.rs").display())).unwrap();
        let live = read_pin(br#"{"id":2,"name":"A","mode":"live","created_ms":5}"#).unwrap();
        assert_eq!(live.app_dir(&ws), app, "a Live pin runs the project's own app");
        assert_eq!(live.launch(&ws, None, &CargoEnv::default()).unwrap().0, program);
    }

    /// A saved Live pin of a workspace member's project: Cargo builds the member into the
    /// workspace's `target/`, outside the app, which the file lookup reads; another member's
    /// program of the same name there doesn't count.
    #[test]
    fn a_saved_live_pin_finds_a_workspace_members_build() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Snake", "a snake game", TargetRef::None).unwrap();
        std::fs::write(ws.root.join("Cargo.toml"), "[workspace]\nmembers = [\"app\"]\nresolver = \"2\"\n").unwrap();
        let app = ws.app_dir();
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::write(app.join("Cargo.toml"), "[package]\nname = \"member-probe\"\n").unwrap();
        std::fs::write(app.join("src/main.rs"), "fn main() {}\n").unwrap();
        let debug = ws.root.join("target/debug");
        std::fs::create_dir_all(&debug).unwrap();
        let program = debug.join(exe("member-probe"));
        std::fs::write(&program, b"program").unwrap();
        mark_program(&program);
        let other = d.path().join("other/src/main.rs");
        std::fs::create_dir_all(other.parent().unwrap()).unwrap();
        std::fs::write(&other, "fn main() {}\n").unwrap();
        let record = |source: &Path| std::fs::write(debug.join("member-probe.d"), format!("{}: {}\n", program.display(), source.display())).unwrap();
        let live = read_pin(br#"{"id":2,"name":"A","mode":"live","created_ms":5}"#).unwrap();
        record(&other);
        assert!(live.launch(&ws, None, &CargoEnv::default()).is_err(), "another project's program of that name isn't started");
        record(&app.join("src/main.rs"));
        assert_eq!(live.launch(&ws, None, &CargoEnv::default()).unwrap().0, crate::workspace::normalized(&program));
    }

    /// A saved Live pin looks where the environment it's given says Cargo builds, not where this
    /// process's environment does.
    #[test]
    fn a_saved_live_pin_looks_where_its_given_environment_says() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Snake", "a snake game", TargetRef::None).unwrap();
        let app = ws.app_dir();
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::write(app.join("Cargo.toml"), "[package]\nname = \"env-probe\"\n").unwrap();
        std::fs::write(app.join("src/main.rs"), "fn main() {}\n").unwrap();
        let shared = d.path().join("shared");
        let program = shared.join("debug").join(exe("env-probe"));
        std::fs::create_dir_all(program.parent().unwrap()).unwrap();
        std::fs::write(&program, b"program").unwrap();
        mark_program(&program);
        std::fs::write(shared.join("debug/env-probe.d"), format!("{}: {}\n", program.display(), app.join("src/main.rs").display())).unwrap();
        let live = read_pin(br#"{"id":2,"name":"A","mode":"live","created_ms":5}"#).unwrap();
        let given = CargoEnv { target_dir: Some(shared.clone().into()), ..CargoEnv::default() };
        assert_eq!(live.launch(&ws, None, &given).unwrap().0, crate::workspace::normalized(&program));
        assert!(live.launch(&ws, None, &CargoEnv::default()).is_err(), "no environment names that folder");
    }

    /// R5.7: a named program built outside the app (a moved Cargo target directory) is copied into
    /// the pin and recorded there, so a frozen pin doesn't depend on the external build folder.
    #[test]
    fn a_pin_copies_a_program_built_outside_the_app_and_records_the_copy() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Snake", "a snake game", TargetRef::None).unwrap();
        let app = ws.app_dir();
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::write(app.join("Cargo.toml"), "[package]\nname = \"pin-probe\"\n").unwrap();
        std::fs::write(app.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::create_dir_all(app.join(".cargo")).unwrap();
        std::fs::write(app.join(".cargo/config.toml"), "[build]\ntarget-dir = \"../shared-out\"\n").unwrap();
        let shared = ws.root.join("shared-out/debug");
        std::fs::create_dir_all(&shared).unwrap();
        let program = shared.join(exe("pin-probe"));
        std::fs::write(&program, b"program").unwrap();
        mark_program(&program);
        // What Cargo writes beside the program: the sources it was built from.
        std::fs::write(shared.join("pin-probe.d"), format!("{}: {}\n", program.display(), app.join("src/main.rs").display())).unwrap();
        let other = shared.join(exe("other-project"));
        std::fs::write(&other, b"other").unwrap();
        mark_program(&other);
        assert_eq!(crate::outputs::built_program(&app), Some(program.clone()), "the named external program is found");

        let pin = create(&ws, PinMode::Frozen, None, None, &mut |_, _| true).unwrap();
        let copy = pin.app_dir(&ws).join(PIN_PROGRAM).join(exe("pin-probe"));
        assert_eq!(pin.program, Some(PathBuf::from(PIN_PROGRAM).join(exe("pin-probe"))), "the pin records its copy");
        assert!(copy.is_file(), "the program is copied into the pin");
        assert!(!pin.app_dir(&ws).join(PIN_PROGRAM).join(exe("other-project")).exists(), "another project's program is not");
        let (started, args) = pin.launch(&ws, None, &CargoEnv::default()).unwrap();
        assert_eq!((started, args), (copy.clone(), Vec::new()));
        std::fs::remove_file(&program).unwrap();
        assert_eq!(pin.launch(&ws, None, &CargoEnv::default()).unwrap().0, copy, "the frozen pin keeps its own copy");
    }

    #[test]
    fn a_frozen_pin_copies_the_app_and_its_program_but_not_what_a_build_makes() {
        let d = tempfile::tempdir().unwrap();
        let ws = built(d.path());
        let size = estimate(&ws, None).unwrap();
        let pin = create(&ws, PinMode::Frozen, None, None, &mut |_, _| true).unwrap();
        assert_eq!(pin.name, "Snake · round 3");
        assert_eq!(pin.bytes, size.bytes);
        let copy = pin.app_dir(&ws);
        assert!(copy.join("assets/level.txt").is_file());
        assert!(copy.join("target/release").join(exe("snake")).is_file(), "the program comes along");
        assert!(!copy.join("target/release/deps").exists(), "compiler output stays behind");
        assert!(!copy.join("target/release/snake.pdb").exists() && !copy.join("target/release/.cargo-lock").exists(), "and so do debug symbols");
        assert!(!copy.join(".git").exists());
        assert_eq!(crate::outputs::built_program(&copy), Some(copy.join("target/release").join(exe("snake"))));
        assert_eq!(list(&ws), vec![pin.clone()]);

        // The project's build changes; the pin doesn't.
        std::fs::write(ws.app_dir().join("assets/level.txt"), "....").unwrap();
        assert_eq!(std::fs::read_to_string(copy.join("assets/level.txt")).unwrap(), "####");

        // It isn't copied with the project, and goes with it.
        let dup = ws.duplicate("Snake 2").unwrap();
        assert!(!dup.root.join(PINNED).exists() && !dup.app_dir().join(PINNED).exists());
        ws.clone().delete().unwrap();
        assert!(!ws.root.join(PINNED).exists());
    }

    #[test]
    fn pins_keep_obj_models_and_leave_out_managed_compiler_intermediates() {
        const MODEL: &str = "o triangle\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        for mode in [PinMode::Frozen, PinMode::Light] {
            let d = tempfile::tempdir().unwrap();
            let ws = built(d.path());
            let app = ws.app_dir();
            for folder in ["assets/obj", "assets/models"] {
                std::fs::create_dir_all(app.join(folder)).unwrap();
                std::fs::write(app.join(folder).join("hero.obj"), MODEL).unwrap();
            }
            for extension in ["csproj", "fsproj", "vbproj"] {
                let project = app.join(extension);
                std::fs::create_dir_all(project.join("obj")).unwrap();
                std::fs::write(project.join(format!("Game.{extension}")), "<Project />").unwrap();
                std::fs::write(project.join("obj/intermediate.bin"), [7; 256]).unwrap();
            }
            let size = estimate(&ws, None).unwrap();
            let pin = create(&ws, mode, None, None, &mut |_, _| true).unwrap();
            let copy = pin.app_dir(&ws);
            for folder in ["assets/obj", "assets/models"] {
                assert!(copy.join(folder).join("hero.obj").is_file(), "{mode:?}: {folder} keeps its runtime model");
                assert_eq!(std::fs::read_to_string(copy.join(folder).join("hero.obj")).unwrap(), MODEL, "{mode:?}: {folder}");
            }
            for extension in ["csproj", "fsproj", "vbproj"] {
                assert!(copy.join(extension).join(format!("Game.{extension}")).is_file(), "the project source stays");
                assert!(!copy.join(extension).join("obj").exists(), "compiler intermediates stay behind");
            }
            assert!(!copy.join("target/release/deps").exists());
            assert_eq!(pin.bytes + pin.shared_bytes, size.bytes, "the estimate includes both models");
            if mode == PinMode::Light {
                assert_eq!(pin.shared_bytes, (2 * MODEL.len()) as u64, "both runtime models can be shared");
            } else {
                assert_eq!(pin.shared_bytes, 0);
            }
            std::fs::remove_file(app.join("assets/obj/hero.obj")).unwrap();
            assert_eq!(std::fs::read_to_string(copy.join("assets/obj/hero.obj")).unwrap(), MODEL, "the pin survives removal from the project");
        }
    }

    #[test]
    fn a_pin_leaves_out_old_copies_of_its_program_but_keeps_helpers() {
        let d = tempfile::tempdir().unwrap();
        let ws = built(d.path());
        let release = ws.app_dir().join("target/release");
        let before = estimate(&ws, None).unwrap();
        let program = |name: &str, body: &[u8]| {
            let p = release.join(exe(name));
            std::fs::write(&p, body).unwrap();
            mark_program(&p);
        };
        // What a build leaves when it replaces a program that was running, and two files that only
        // look like one: a helper with nothing beside it, and a copy whose program is gone.
        program("snake.running-144900", &[0u8; 4096]);
        program("snake-tool.running-1", b"helper");
        program("ghost.running-7", b"gone");
        let after = estimate(&ws, None).unwrap();
        assert_eq!(after.files, before.files + 2, "the old copy isn't counted, the two others are");
        assert_eq!(after.bytes, before.bytes + 6 + 4, "and its size isn't either");
        let pin = create(&ws, PinMode::Frozen, None, None, &mut |_, _| true).unwrap();
        assert_eq!(pin.bytes, after.bytes);
        let copy = pin.app_dir(&ws).join("target/release");
        assert!(copy.join(exe("snake")).is_file());
        assert!(!copy.join(exe("snake.running-144900")).exists(), "the old copy of the program is left out");
        assert!(copy.join(exe("snake-tool.running-1")).is_file(), "a helper with nothing it copies beside it stays");
        assert!(copy.join(exe("ghost.running-7")).is_file(), "a copy whose program is gone is all there is to run");
    }

    #[test]
    fn a_light_pin_shares_media_and_survives_a_rebuild_replacing_or_deleting_them() {
        let d = tempfile::tempdir().unwrap();
        let ws = built(d.path());
        let app = ws.app_dir();
        std::fs::write(app.join("assets/sky.PNG"), "old sky").unwrap();
        std::fs::write(app.join("assets/song.ogg"), "old song").unwrap();
        std::fs::write(app.join("assets/save.db"), "a save").unwrap();
        std::fs::write(app.join("target/release/icon.png"), "beside the program").unwrap();
        let size = estimate(&ws, None).unwrap();
        assert_eq!(size.bytes - size.light_bytes, 15, "the two media files, not the save or what's beside the program");
        let pin = create(&ws, PinMode::Light, Some("  Playable  "), None, &mut |_, _| true).unwrap();
        assert_eq!((pin.name.as_str(), pin.bytes, pin.shared_bytes), ("Playable", size.light_bytes, 15));
        let copy = pin.app_dir(&ws);
        assert_eq!(crate::outputs::built_program(&copy), Some(copy.join("target/release").join(exe("snake"))));
        assert_eq!(list(&ws), vec![pin.clone()]);

        // What a program can write to, its program and its code are its own.
        std::fs::write(app.join("assets/save.db"), "changed").unwrap();
        std::fs::write(app.join("assets/level.txt"), "....").unwrap();
        std::fs::write(app.join("target/release/icon.png"), "changed").unwrap();
        assert_eq!(std::fs::read_to_string(copy.join("assets/save.db")).unwrap(), "a save");
        assert_eq!(std::fs::read_to_string(copy.join("assets/level.txt")).unwrap(), "####");
        assert_eq!(std::fs::read_to_string(copy.join("target/release/icon.png")).unwrap(), "beside the program");

        // A rebuild replaces a file (writing beside it, then renaming) or deletes it.
        std::fs::write(app.join("assets/sky.new"), "new sky").unwrap();
        std::fs::rename(app.join("assets/sky.new"), app.join("assets/sky.PNG")).unwrap();
        std::fs::remove_file(app.join("assets/song.ogg")).unwrap();
        assert_eq!(std::fs::read_to_string(copy.join("assets/sky.PNG")).unwrap(), "old sky");
        assert_eq!(std::fs::read_to_string(copy.join("assets/song.ogg")).unwrap(), "old song");

        delete(&ws, pin.id).unwrap();
        assert!(!pin.dir(&ws).exists());
        assert_eq!(std::fs::read_to_string(app.join("assets/save.db")).unwrap(), "changed", "removing a pin leaves the project alone");
    }

    #[test]
    fn a_shared_file_that_cannot_be_linked_is_counted_as_copied() {
        let d = tempfile::tempdir().unwrap();
        let ws = built(d.path());
        let app = ws.app_dir();
        std::fs::write(app.join("assets/sky.PNG"), "old sky").unwrap();
        std::fs::write(app.join("assets/song.ogg"), "old song").unwrap();
        // The control: on a drive that links, the same files are shared and none is counted as copied.
        let linked = d.path().join("pin-linked");
        let (linked_bytes, shared, copied) =
            copy_linking(&ws, &app, true, &linked, PinMode::Light, &mut |_, _| true, &|from, to| std::fs::hard_link(from, to)).unwrap();
        assert_eq!((shared, copied), (15, 0), "the two media files are shared");

        // A drive (or a file) that refuses the link: the same bytes are copied instead, and said so.
        let no_links = |_: &Path, _: &Path| Err(std::io::Error::other("no hard links here"));
        let to = d.path().join("pin-copied");
        let (bytes, shared, copied) = copy_linking(&ws, &app, true, &to, PinMode::Light, &mut |_, _| true, &no_links).unwrap();
        assert_eq!((shared, copied), (0, 15), "the two media files were meant to be shared and were copied");
        assert_eq!(bytes.saturating_sub(linked_bytes), 15, "what the copy takes grows by the bytes it had to copy");
        assert_eq!(std::fs::read_to_string(to.join("assets/sky.PNG")).unwrap(), "old sky", "the copy is still usable");
    }

    /// Media with the same bytes as a file in an earlier frozen pin are copied from it, whatever
    /// they are called; a light pin's files, the project's own, are never the source.
    #[test]
    fn a_pins_equal_media_are_copied_from_an_earlier_frozen_pin() {
        let d = tempfile::tempdir().unwrap();
        let ws = built(d.path());
        let app = ws.app_dir();
        std::fs::write(app.join("assets/sky.png"), vec![7u8; 4096]).unwrap();
        std::fs::write(app.join("assets/song.ogg"), "first song").unwrap();
        let first = create(&ws, PinMode::Frozen, None, None, &mut |_, _| true).unwrap();
        create(&ws, PinMode::Light, None, None, &mut |_, _| true).unwrap();

        // The next build keeps the sky, adds the same picture under another name and changes the
        // song, keeping its size.
        std::fs::write(app.join("assets/sky-again.png"), vec![7u8; 4096]).unwrap();
        std::fs::write(app.join("assets/song.ogg"), "other song").unwrap();
        let pinned = PinnedMedia::of(&ws);
        let sky = first.app_dir(&ws).join("assets/sky.png");
        for name in ["sky.png", "sky-again.png"] {
            assert_eq!(pinned.same_as(&app.join("assets").join(name), 4096).map(|(p, _)| p), Some(sky.as_path()), "{name}");
        }
        assert_eq!(pinned.same_as(&app.join("assets/song.ogg"), 10), None, "same size, other bytes");
        assert_eq!(pinned.0.values().flatten().count(), 2, "only the frozen pin's picture and song");
    }

    /// Frozen pins with the same media each keep a file of their own: one pin's program writing to
    /// it, or a pin being removed, leaves the other's as it was.
    #[test]
    fn frozen_pins_of_one_project_keep_their_own_files() {
        let d = tempfile::tempdir().unwrap();
        let ws = built(d.path());
        std::fs::write(ws.app_dir().join("assets/sky.png"), vec![7u8; 4096]).unwrap();
        let first = create(&ws, PinMode::Frozen, None, None, &mut |_, _| true).unwrap();
        let second = create(&ws, PinMode::Frozen, None, None, &mut |_, _| true).unwrap();
        assert_eq!((second.shared_bytes, second.bytes), (0, first.bytes), "a frozen pin shares nothing it could be changed through");
        let (one, two) = (first.app_dir(&ws).join("assets/sky.png"), second.app_dir(&ws).join("assets/sky.png"));
        std::io::Write::write_all(&mut std::fs::OpenOptions::new().write(true).open(&one).unwrap(), b"repainted").unwrap();
        assert_eq!(std::fs::read(&two).unwrap(), vec![7u8; 4096], "written in place in the first pin only");
        delete(&ws, first.id).unwrap();
        assert_eq!(std::fs::read(&two).unwrap(), vec![7u8; 4096]);
    }

    /// A pinned file that changed after it was read for its digest isn't what the new pin gets.
    #[test]
    fn a_pinned_file_changed_since_it_was_read_is_copied_from_the_project() {
        let d = tempfile::tempdir().unwrap();
        let ws = built(d.path());
        let app = ws.app_dir();
        std::fs::write(app.join("assets/sky.png"), vec![7u8; 4096]).unwrap();
        let first = create(&ws, PinMode::Frozen, None, None, &mut |_, _| true).unwrap();
        let pinned = PinnedMedia::of(&ws);
        assert!(pinned.same_as(&app.join("assets/sky.png"), 4096).is_some());
        std::fs::write(first.app_dir(&ws).join("assets/sky.png"), vec![8u8; 4096]).unwrap();
        let to = d.path().join("pin-copy");
        let files = Plan::new(&ws, &app, true).files(&ws, &to).unwrap();
        copy_files(&files, PinMode::Frozen, &pinned, &mut |_, _| true, &|from, to| std::fs::hard_link(from, to)).unwrap();
        assert_eq!(std::fs::read(to.join("assets/sky.png")).unwrap(), vec![7u8; 4096], "the project's bytes");
    }

    /// A file copied from an earlier pin gets the project file's permissions, not that pin's.
    #[cfg(unix)]
    #[test]
    fn a_file_copied_from_an_earlier_pin_keeps_the_projects_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let ws = built(d.path());
        let app = ws.app_dir();
        let sky = app.join("assets/sky.png");
        std::fs::write(&sky, vec![7u8; 4096]).unwrap();
        let first = create(&ws, PinMode::Frozen, None, None, &mut |_, _| true).unwrap();
        std::fs::set_permissions(first.app_dir(&ws).join("assets/sky.png"), std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::set_permissions(&sky, std::fs::Permissions::from_mode(0o644)).unwrap();
        let to = d.path().join("pin-copy");
        let files = Plan::new(&ws, &app, true).files(&ws, &to).unwrap();
        copy_files(&files, PinMode::Frozen, &PinnedMedia::of(&ws), &mut |_, _| true, &|from, to| std::fs::hard_link(from, to)).unwrap();
        assert_eq!(std::fs::metadata(to.join("assets/sky.png")).unwrap().permissions().mode() & 0o777, 0o644);
    }

    #[test]
    fn an_old_pin_without_copied_shared_bytes_still_loads() {
        let old = read_pin(br#"{"version":1,"id":2,"name":"A","mode":"light","created_ms":5,"bytes":10,"shared_bytes":8}"#).unwrap();
        assert_eq!(old.copied_shared_bytes, 0, "a pin written before copied_shared_bytes existed reads as zero");
        assert_eq!((old.bytes, old.shared_bytes), (10, 8));
    }

    #[test]
    fn a_light_pin_shares_an_engines_imports_and_electrons_packages_but_copies_its_program() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Hop", "a game", TargetRef::None).unwrap();
        let app = ws.app_dir();
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("project.godot"), "config").unwrap();
        std::fs::create_dir_all(app.join(".godot/imported")).unwrap();
        std::fs::write(app.join(".godot/imported/sky.ctex"), "imported").unwrap();
        let godot = d.path().join("godot.exe");
        let pin = create(&ws, PinMode::Light, None, Some(&godot), &mut |_, _| true).unwrap();
        assert_eq!((pin.bytes, pin.shared_bytes), (6, 8), "project.godot copied, the import shared");

        let ws = Workspace::create(d.path(), "Notes", "an app", TargetRef::None).unwrap();
        let app = ws.app_dir();
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("package.json"), r#"{"dependencies":{"electron":"1"}}"#).unwrap();
        let dist = app.join("node_modules/electron/dist");
        std::fs::create_dir_all(dist.join("resources")).unwrap();
        std::fs::create_dir_all(app.join("node_modules/lib")).unwrap();
        std::fs::write(app.join("node_modules/lib/index.js"), "12345").unwrap();
        std::fs::write(dist.join("resources/default.asar"), "123").unwrap();
        let program = if cfg!(target_os = "macos") { dist.join("Electron.app/Contents/MacOS/Electron") } else { dist.join(exe("electron")) };
        std::fs::create_dir_all(program.parent().unwrap()).unwrap();
        std::fs::write(&program, b"electron").unwrap();
        mark_program(&program);
        let pin = create(&ws, PinMode::Light, None, None, &mut |_, _| true).unwrap();
        assert_eq!(pin.shared_bytes, 8, "the packages, not the program");
        assert!(pin.bytes >= 8);
    }

    #[test]
    fn a_live_pin_is_no_longer_made_and_an_old_one_still_opens_the_projects_app() {
        let d = tempfile::tempdir().unwrap();
        let ws = built(d.path());
        assert_eq!(create(&ws, PinMode::Live, None, None, &mut |_, _| true).unwrap_err().code, ErrorCode::BadInput);
        let old = read_pin(br#"{"id":2,"name":"A","mode":"live","created_ms":5}"#).unwrap();
        assert_eq!(old.app_dir(&ws), ws.app_dir());
    }

    #[test]
    fn a_pin_of_an_app_edited_in_place_leaves_the_projects_records_out() {
        let d = tempfile::tempdir().unwrap();
        let folder = d.path().join("mine");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("notes.txt"), "mine").unwrap();
        let ws = Workspace::create_at(&folder, "Mine", "edit it", TargetRef::None, Some(crate::workspace::ExistingFiles::Edit)).unwrap();
        std::fs::write(folder.join("Cargo.toml"), "[package]\nname = \"mine\"\n").unwrap();
        std::fs::create_dir_all(folder.join("src")).unwrap();
        std::fs::write(folder.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::create_dir_all(folder.join("target/debug")).unwrap();
        std::fs::write(folder.join("target/debug").join(exe("mine")), b"program").unwrap();
        mark_program(&folder.join("target/debug").join(exe("mine")));
        let first = create(&ws, PinMode::Frozen, None, None, &mut |_, _| true).unwrap();
        let second = create(&ws, PinMode::Frozen, None, None, &mut |_, _| true).unwrap();
        assert_eq!((first.id, second.id), (1, 2));
        let copy = second.app_dir(&ws);
        assert!(copy.join("notes.txt").is_file());
        assert!(!copy.join(crate::workspace::MANIFEST).exists(), "the project's records aren't part of the app");
        assert!(!copy.join(PINNED).exists(), "a pin never copies other pins");
    }

    /// A website's pin copies the built site alone, which a rebuild of the site leaves as it was;
    /// a light one shares the site's media.
    #[test]
    fn a_website_is_pinned_as_a_copy_of_its_built_site() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Site", "a page", TargetRef::None).unwrap();
        let app = ws.app_dir();
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::write(app.join("package.json"), "{}").unwrap();
        std::fs::write(app.join("src/main.js"), "go()").unwrap();
        let dist = app.join("dist");
        std::fs::create_dir_all(dist.join("build")).unwrap();
        std::fs::write(dist.join("index.html"), "<h1>first</h1>").unwrap();
        std::fs::write(dist.join("build/app.js"), "go()").unwrap();
        std::fs::write(dist.join("logo.png"), vec![0u8; 4096]).unwrap();

        let frozen = create(&ws, PinMode::Frozen, None, None, &mut |_, _| true).unwrap();
        assert!(frozen.site);
        let copy = frozen.app_dir(&ws);
        assert!(copy.join("index.html").is_file() && copy.join("build/app.js").is_file(), "the whole built site is copied");
        assert!(!copy.join("src").exists() && !copy.join("dist").exists(), "only the built site is copied");
        std::fs::remove_file(dist.join("index.html")).unwrap();
        std::fs::write(dist.join("index.html"), "<h1>second</h1>").unwrap();
        assert_eq!(std::fs::read_to_string(copy.join("index.html")).unwrap(), "<h1>first</h1>");
        assert!(get(&ws, frozen.id).unwrap().site, "saved as a website");

        let size = estimate(&ws, None).unwrap();
        assert!(size.light_bytes < size.bytes, "a light copy shares the media: {size:?}");
        let light = create(&ws, PinMode::Light, None, None, &mut |_, _| true).unwrap();
        assert!(light.site && light.shared_bytes + light.copied_shared_bytes >= 4096, "the logo is shared, or counted as copied: {light:?}");
    }

    /// A link inside the built site is served by the preview, so the pin serves it too, as the
    /// file or folder it points to; one pointing out of the site is served by neither.
    #[cfg(unix)]
    #[test]
    fn a_websites_pin_serves_the_links_inside_the_site() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Site", "a page", TargetRef::None).unwrap();
        let dist = ws.app_dir().join("dist");
        std::fs::create_dir_all(dist.join("build")).unwrap();
        std::fs::write(dist.join("index.html"), "<h1>site</h1>").unwrap();
        std::fs::write(dist.join("build/app.js"), "go()").unwrap();
        std::fs::write(dist.join("logo.png"), "logo").unwrap();
        std::fs::write(d.path().join("private.txt"), "not the site's").unwrap();
        std::fs::create_dir_all(dist.join("img")).unwrap();
        symlink("../logo.png", dist.join("img/logo.png")).unwrap();
        symlink("build", dist.join("assets")).unwrap();
        symlink(d.path().join("private.txt"), dist.join("private.txt")).unwrap();

        let pin = create(&ws, PinMode::Frozen, None, None, &mut |_, _| true).unwrap();
        let served = std::fs::canonicalize(pin.app_dir(&ws)).unwrap();
        let serves = |path: &str| craft_anything_platform::serve::resolve(&served, path).map(|f| std::fs::read_to_string(f).unwrap());
        assert_eq!(serves("/img/logo.png").as_deref(), Some("logo"), "a linked file");
        assert_eq!(serves("/assets/app.js").as_deref(), Some("go()"), "a linked folder");
        assert!(!served.join("private.txt").exists(), "a link out of the site isn't copied");
    }

    /// A site whose index.html leads out of it has nothing its pin could serve, so it isn't pinned.
    #[cfg(unix)]
    #[test]
    fn a_website_whose_index_leads_out_of_it_is_not_pinned() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Site", "a page", TargetRef::None).unwrap();
        let dist = ws.app_dir().join("dist");
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(d.path().join("outside.html"), "<h1>not the site's</h1>").unwrap();
        symlink(d.path().join("outside.html"), dist.join("index.html")).unwrap();

        assert_eq!(create(&ws, PinMode::Frozen, None, None, &mut |_, _| true).unwrap_err().code, ErrorCode::NotFound);
        assert!(list(&ws).is_empty());
        assert_eq!(std::fs::read_dir(ws.root.join(PINNED)).unwrap().count(), 0, "the half-made pin is gone");
    }

    /// A link named unlike the file it leads to is served as that file's kind, as the preview
    /// serves it, not as an unknown file.
    #[cfg(unix)]
    #[test]
    fn a_websites_pin_serves_a_link_named_unlike_its_file_as_that_files_kind() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Site", "a page", TargetRef::None).unwrap();
        let dist = ws.app_dir().join("dist");
        std::fs::create_dir_all(dist.join("build")).unwrap();
        std::fs::write(dist.join("index.html"), "<h1>site</h1>").unwrap();
        std::fs::write(dist.join("build/app.js"), "go()").unwrap();
        symlink("build/app.js", dist.join("main")).unwrap();
        symlink("build", dist.join("assets")).unwrap();
        symlink("app.js", dist.join("build/latest")).unwrap();

        let pin = create(&ws, PinMode::Frozen, None, None, &mut |_, _| true).unwrap();
        let served = std::fs::canonicalize(pin.app_dir(&ws)).unwrap();
        let file = |path: &str| craft_anything_platform::serve::resolve(&served, path).unwrap();
        assert_eq!(file("/main"), served.join("build/app.js"), "served as the .js file it leads to");
        assert_eq!(file("/assets/latest"), served.join("build/app.js"), "from inside a linked folder too");
        assert_eq!(std::fs::read_to_string(file("/main")).unwrap(), "go()");
    }

    /// What a site's copy leaves out stays out through a link, and a link back up the site doesn't
    /// make its copy go round for ever.
    #[cfg(unix)]
    #[test]
    fn a_websites_pin_follows_no_link_into_what_it_leaves_out_or_round_a_loop() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Site", "a page", TargetRef::None).unwrap();
        let dist = ws.app_dir().join("dist");
        std::fs::create_dir_all(dist.join(".git")).unwrap();
        std::fs::create_dir_all(dist.join("docs/guide")).unwrap();
        std::fs::write(dist.join("index.html"), "<h1>site</h1>").unwrap();
        std::fs::write(dist.join(".git/HEAD"), "ref: main").unwrap();
        std::fs::write(dist.join("docs/guide/page.html"), "guide").unwrap();
        symlink(".git", dist.join("repository")).unwrap();
        symlink(".git/HEAD", dist.join("head")).unwrap();
        symlink(".", dist.join("self")).unwrap();
        symlink("../..", dist.join("docs/guide/up")).unwrap();
        symlink("guide", dist.join("docs/again")).unwrap();

        let pin = create(&ws, PinMode::Frozen, None, None, &mut |_, _| true).unwrap();
        let app = pin.app_dir(&ws);
        assert!(!app.join("repository").exists() && !app.join("head").exists(), "nothing of .git");
        assert!(!app.join("self").exists() && !app.join("docs/guide/up").exists(), "no loop");
        assert_eq!(std::fs::read_to_string(app.join("docs/again/page.html")).unwrap(), "guide", "a link that isn't a loop is still copied");
        assert_eq!(std::fs::read_to_string(app.join("docs/guide/page.html")).unwrap(), "guide");
    }

    /// A link to the next folder down in each of many nested folders doubles the walk at every
    /// level; past a limit the pin is refused, while a few such links are still copied.
    #[cfg(unix)]
    #[test]
    fn a_websites_pin_refuses_links_that_multiply_its_files_but_copies_a_few() {
        use std::os::unix::fs::symlink;
        let nest = |levels: usize| {
            let d = tempfile::tempdir().unwrap();
            let ws = Workspace::create(d.path(), "Site", "a page", TargetRef::None).unwrap();
            let mut dir = ws.app_dir().join("dist");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("index.html"), "<h1>site</h1>").unwrap();
            for _ in 0..levels {
                std::fs::create_dir(dir.join("n")).unwrap();
                symlink("n", dir.join("alias")).unwrap();
                dir = dir.join("n");
            }
            std::fs::write(dir.join("page.html"), "deep").unwrap();
            (d, ws)
        };

        let (_d, ws) = nest(20);
        assert_eq!(estimate(&ws, None).unwrap_err().code, ErrorCode::OverLimit);

        let (_d, ws) = nest(3);
        assert_eq!(estimate(&ws, None).unwrap().files, 1 + (1 << 3), "the index, and the deep page by each way to it");
        let app = create(&ws, PinMode::Frozen, None, None, &mut |_, _| true).unwrap().app_dir(&ws);
        assert_eq!(std::fs::read_to_string(app.join("alias/alias/alias/page.html")).unwrap(), "deep");
        assert_eq!(std::fs::read_to_string(app.join("n/alias/n/page.html")).unwrap(), "deep");
    }

    /// One folder is one running server or program, however its path is written.
    #[test]
    fn a_folder_has_one_key_however_it_is_written() {
        let here = std::env::current_dir().unwrap();
        assert_eq!(key(Path::new("pins/../pins/1")), key(&here.join("pins/1")));
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_reached_through_a_link_has_its_real_key_and_keeps_it_once_removed() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        std::fs::create_dir_all(b.join("sub")).unwrap();
        std::fs::create_dir_all(b.join("pin")).unwrap();
        std::fs::create_dir_all(a.join("pin")).unwrap();
        std::os::unix::fs::symlink(b.join("sub"), a.join("link")).unwrap();
        let through = a.join("link/../pin");
        assert_eq!(key(&through), key(&b.join("pin")), "the link leads to b");
        assert_ne!(key(&through), key(&a.join("pin")));
        let before = key(&b.join("pin"));
        std::fs::remove_dir(b.join("pin")).unwrap();
        assert_eq!(key(&b.join("pin")), before, "a removed folder keeps its key");
    }

    #[test]
    fn a_stopped_copy_leaves_nothing_behind_and_nothing_to_pin_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let ws = built(d.path());
        let e = create(&ws, PinMode::Frozen, None, None, &mut |_, _| false).unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled);
        assert!(list(&ws).is_empty());
        assert_eq!(std::fs::read_dir(ws.root.join(PINNED)).unwrap().count(), 0, "the half-made copy is gone");

        let empty = Workspace::create(d.path(), "Empty", "nothing yet", TargetRef::None).unwrap();
        assert_eq!(create(&empty, PinMode::Frozen, None, None, &mut |_, _| true).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn pins_are_renamed_and_removed_and_unreadable_ones_skipped() {
        let d = tempfile::tempdir().unwrap();
        let ws = built(d.path());
        let pin = create(&ws, PinMode::Frozen, None, None, &mut |_, _| true).unwrap();
        assert_eq!(rename(&ws, pin.id, "Best one").unwrap().name, "Best one");
        assert!(rename(&ws, pin.id, " ").is_err());
        let file = pin.dir(&ws).join(PIN_FILE);
        let mut newer: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        newer["version"] = Value::from(PIN_VERSION + 1);
        newer["colour"] = Value::from("red");
        let newer = serde_json::to_vec(&newer).unwrap();
        std::fs::write(&file, &newer).unwrap();
        assert_eq!(rename(&ws, pin.id, "Other").unwrap_err().code, ErrorCode::NotSupported);
        assert_eq!(std::fs::read(&file).unwrap(), newer, "a newer pin is left as it was");
        std::fs::create_dir_all(ws.root.join(PINNED).join("7")).unwrap();
        std::fs::write(ws.root.join(PINNED).join("7").join(PIN_FILE), "{").unwrap();
        assert_eq!(list(&ws).len(), 1);
        delete(&ws, pin.id).unwrap();
        assert!(list(&ws).is_empty() && !pin.dir(&ws).exists());
        assert_eq!(delete(&ws, pin.id).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn an_older_or_newer_pin_file_is_read() {
        let old = read_pin(br#"{"id":2,"name":"A","mode":"live","created_ms":5}"#).unwrap();
        assert_eq!((old.version, old.mode, old.bytes), (PIN_VERSION, PinMode::Live, 0));
        assert_eq!(old.program, None);
        let frozen = read_pin(br#"{"version":1,"id":3,"name":"Old copy","mode":"frozen","created_ms":5,"program":"formerly-unknown"}"#).unwrap();
        assert_eq!(frozen.version, PIN_VERSION);
        assert_eq!(frozen.program, None, "the migration cannot invent the old pin's selected program");
        let newer = read_pin(br#"{"version":9,"id":2,"name":"A","mode":"frozen","created_ms":5,"colour":"red"}"#).unwrap();
        assert_eq!(newer.version, 9, "a newer pin keeps its version");
        assert!(read_pin(b"[]").is_err());
        let program = read_pin(br#"{"version":2,"id":4,"name":"A","mode":"frozen","created_ms":5,"program":"snake"}"#).unwrap();
        assert_eq!((program.version, program.site), (PIN_VERSION, false), "a version 2 pin is a program's");
        assert_eq!(program.program.as_deref(), Some(Path::new("snake")), "the step keeps what version 2 recorded");
        for bad in ["4294967296", "-1", "\"2\"", "null", "1.5"] {
            let text = format!(r#"{{"version":{bad},"id":2,"name":"A","mode":"frozen","created_ms":5}}"#);
            assert!(read_pin(text.as_bytes()).is_err(), "version {bad} isn't read as version 1");
        }
    }

    #[test]
    fn the_end_of_a_log_is_its_last_lines() {
        let d = tempfile::tempdir().unwrap();
        let log = d.path().join("run.log");
        let text: String = (0..100).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&log, text).unwrap();
        let t = tail(&log);
        assert!(t.starts_with("line 80") && t.ends_with("line 99"), "{t}");
        assert_eq!(tail(&d.path().join("none.log")), "");
    }

    fn shell(script: &str) -> Launch {
        if cfg!(windows) { Launch::new("cmd").args(["/C", script]).hide_console() } else { Launch::new("sh").args(["-c", script]) }
    }

    #[test]
    fn one_copy_runs_per_folder_unless_another_is_asked_for() {
        let d = tempfile::tempdir().unwrap();
        let runs = Runs::default();
        let long = if cfg!(windows) { "ping -n 30 127.0.0.1 > NUL" } else { "sleep 30" };
        let Opened::Started { quit: None, .. } = runs.open(d.path(), shell(long), false).unwrap() else { panic!("it should be running") };
        assert!(matches!(runs.open(d.path(), shell(long), false).unwrap(), Opened::AlreadyRunning(_)));
        assert!(matches!(runs.open(d.path(), shell(long), true).unwrap(), Opened::Started { quit: None, .. }));
        assert_eq!(runs.running(d.path()).len(), 2);
        assert_eq!(runs.stop(d.path()), 2);
        assert!(runs.running(d.path()).is_empty());

        // A start that quits at once says so, and doesn't count as running.
        let log = d.path().join("run.log");
        let Opened::Started { quit: Some(ended), .. } = runs.open(d.path(), shell("echo no window here && exit 4").log_to(&log), false).unwrap() else {
            panic!("it should have quit")
        };
        assert_eq!(ended.code, Some(4));
        assert!(tail(&log).contains("no window here"));
        assert!(runs.running(d.path()).is_empty());
    }
}
