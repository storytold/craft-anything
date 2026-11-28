//! A workspace is one thing the person is making: a folder under their projects directory that
//! holds what we learned about the target and what we built from it.
//!
//! ```text
//! <projects>/<slug>/
//!   craft.json           the manifest (request, target, status, history)
//!   investigation.cany/  evidence, memory and journal (a regular project)
//!   capture/             what we recorded from the target (screenshots, blueprint)
//!   reference/           pictures of the original the person added (the look to match)
//!   app/                 the generated project (its own git repository)
//!   checks/              comparisons of the build against the target, one folder per round
//!   inbox/               questions and approvals waiting for the person
//!   build-activity.log   what the runs did, line by line (with .old.log, the one before it)
//! ```
//!
//! A project can also start in a folder the person picks ([`Workspace::create_at`]). When it
//! already has their files, they're either the app itself (`app_in_root`: the builder edits them in
//! place and the folders above sit beside them) or material to start from, with the app in app/.

use crate::inbox::{next_counter, now_ms};
use crate::project::write_atomic;
use crate::versioned::{self, upgrade};
use craft_anything_evidence::{ErrorCode, ToolError};
use craft_anything_platform::cancel::CancelToken;
use craft_anything_platform::fs::read_file_capped;
use craft_anything_platform::sync::Mutex;
use same_file::Handle as FileHandle;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::PoisonError;

pub mod progress;

pub const MANIFEST: &str = "craft.json";

/// Folders a copy of a project leaves out: what a build makes again, which can be gigabytes.
const REBUILT: &[&str] = &["node_modules", "target", ".godot", "Library", "Temp", ".venv", "__pycache__", ".next", ".turbo"];
/// An `obj` directory is ambiguous: these neighbouring project files identify managed build output.
const MANAGED_PROJECTS: &[&str] = &["csproj", "fsproj", "vbproj"];
/// How deep a copy of a project's folders goes.
const COPY_DEPTH: usize = 64;
const CLEANUP_PREFIX: &str = ".craft-copy-cleanup-";

/// This name is ambiguous recovery state, not proof that Craft owns its contents.
fn check_cleanup_name(path: &Path) -> Result<(), ToolError> {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else { return Ok(()) };
    let Some((prefix, nonce)) = name.split_at_checked(CLEANUP_PREFIX.len()) else { return Ok(()) };
    let matches = if cfg!(windows) { prefix.eq_ignore_ascii_case(CLEANUP_PREFIX) } else { prefix == CLEANUP_PREFIX };
    if matches && nonce.len() == 16 && nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ToolError::invalid(format!("{} may contain retained project records, so creating or copying a project here has stopped.", path.display()))
            .with_remediation("Inspect this directory and move it to a safe location outside the project before retrying. Its contents may be personal files; they have not been removed or silently omitted."));
    }
    Ok(())
}

fn check_pending_cleanups(root: &Path, own: Option<&CreationCleanup>) -> Result<(), ToolError> {
    for entry in std::fs::read_dir(root).map_err(|error| ToolError::io(root.display(), error))? {
        let entry = entry.map_err(|error| ToolError::io(root.display(), error))?;
        if !entry.file_type().map_err(|error| ToolError::io(entry.path().display(), error))?.is_dir() {
            continue;
        }
        if let Some(own) = own.filter(|own| own.name == entry.file_name()) {
            // A matching name alone never exempts a replaced or occupied reservation.
            own.empty_path(root)?;
        } else {
            check_cleanup_name(&entry.path())?;
        }
    }
    Ok(())
}

pub(crate) fn rebuilt(p: &Path) -> bool {
    let Some(name) = p.file_name().and_then(|n| n.to_str()) else { return false };
    if name != "obj" {
        return REBUILT.contains(&name) && p.is_dir();
    }
    if !p.is_dir() {
        return false;
    }
    // Runtime models commonly live in assets/obj. Without evidence that a folder is compiler
    // output, keep it; an unreadable or unusually large parent must not cause data to be omitted.
    p.parent().and_then(|parent| std::fs::read_dir(parent).ok()).is_some_and(|entries| {
        entries.take(4096).flatten().any(|entry| {
            entry.path().extension().and_then(|e| e.to_str()).is_some_and(|ext| MANAGED_PROJECTS.iter().any(|project| ext.eq_ignore_ascii_case(project)))
                && entry.file_type().is_ok_and(|kind| kind.is_file())
        })
    })
}

/// Copy into a new folder (links are skipped), leaving out what `skip` says.
/// Every destination entry must be new; checking a replaced root later cannot undo an overwrite.
fn copy_tree(from: &Path, to: &Path, skip: &dyn Fn(&Path) -> bool, depth: usize, backups: Option<&CopyBackups<'_>>) -> Result<(), ToolError> {
    if depth > COPY_DEPTH {
        return Err(ToolError::new(ErrorCode::OverLimit, format!("{} is nested more than {COPY_DEPTH} folders deep.", from.display())));
    }
    std::fs::create_dir(to).map_err(|e| ToolError::io(to.display(), e))?;
    for entry in std::fs::read_dir(from).map_err(|e| ToolError::io(from.display(), e))? {
        let entry = entry.map_err(|e| ToolError::io(from.display(), e))?;
        let (path, kind) = (entry.path(), entry.file_type().map_err(|e| ToolError::io(entry.path().display(), e))?);
        if skip(&path) || kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            // Every copied tree can contain a retained inbox, including reference and
            // capture folders. An ambiguous cleanup directory is an error, not a skip.
            check_cleanup_name(&path)?;
        }
        let dest = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&path, &dest, skip, depth + 1, backups)?;
        } else if kind.is_file() {
            if let Some(backups) = backups.filter(|backups| path.parent() == Some(backups.root))
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| crate::feed::DAMAGED_BACKUP_NAMES.iter().any(|slot| name.eq_ignore_ascii_case(slot)))
            {
                copy_personal_backup(backups, &path, &dest)?;
            } else {
                copy_new_file(&path, &dest)?;
            }
        }
    }
    Ok(())
}

/// Open an exclusive output before streaming bytes, retaining the source file's permissions.
#[cfg(not(windows))]
fn copy_new_file(from: &Path, to: &Path) -> Result<(), ToolError> {
    let (mut source, metadata) = craft_anything_platform::fs::open_regular_file(from)?;
    let permissions = metadata.permissions();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        options.mode(permissions.mode());
    }
    let mut output = options.open(to).map_err(|error| ToolError::io(to.display(), error))?;
    std::io::copy(&mut source, &mut output).map_err(|error| ToolError::io(format!("{} to {}", from.display(), to.display()), error))?;
    output.set_permissions(permissions).map_err(|error| ToolError::io(to.display(), error))
}

#[cfg(windows)]
fn copy_new_file(from: &Path, to: &Path) -> Result<(), ToolError> {
    let (source, metadata) = craft_anything_platform::fs::open_regular_file(from)?;
    copy_new_windows_file(from, to, &source, &metadata)
}

/// `std::fs::copy` retains NTFS named streams. Reserve the output exclusively and deny
/// delete sharing on both names while it runs, so it can only overwrite our new output.
#[cfg(windows)]
fn copy_new_windows_file(from: &Path, to: &Path, source: &std::fs::File, metadata: &std::fs::Metadata) -> Result<(), ToolError> {
    use std::os::windows::fs::OpenOptionsExt as _;
    let output = std::fs::OpenOptions::new().write(true).create_new(true).share_mode(1 | 2).open(to).map_err(|error| ToolError::io(to.display(), error))?;
    copy_windows_into_reserved(from, to, source, metadata, &output)
}

/// Preserve all Windows streams while copying into an output exclusively reserved by the
/// caller. Both pinned names must still identify the supplied handles before CopyFile runs.
#[cfg(windows)]
pub(crate) fn copy_windows_into_reserved(
    from: &Path,
    to: &Path,
    source: &std::fs::File,
    metadata: &std::fs::Metadata,
    output: &std::fs::File,
) -> Result<(), ToolError> {
    use std::os::windows::fs::OpenOptionsExt as _;
    const SHARE_READ_WRITE: u32 = 1 | 2;
    let pinned = std::fs::OpenOptions::new().read(true).share_mode(SHARE_READ_WRITE).open(from).map_err(|error| ToolError::io(from.display(), error))?;
    let pinned = FileHandle::from_file(pinned).map_err(|error| ToolError::io(from.display(), error))?;
    let opened = FileHandle::from_file(source.try_clone().map_err(|error| ToolError::io(from.display(), error))?)
        .map_err(|error| ToolError::io(from.display(), error))?;
    if pinned != opened {
        return Err(copy_changed(from));
    }
    check_copy_entry(from, metadata)?;
    let pinned_output = std::fs::OpenOptions::new().write(true).share_mode(SHARE_READ_WRITE).open(to).map_err(|error| ToolError::io(to.display(), error))?;
    let pinned_output = FileHandle::from_file(pinned_output).map_err(|error| ToolError::io(to.display(), error))?;
    let reserved =
        FileHandle::from_file(output.try_clone().map_err(|error| ToolError::io(to.display(), error))?).map_err(|error| ToolError::io(to.display(), error))?;
    if pinned_output != reserved {
        return Err(copy_changed(to));
    }
    let copied = std::fs::copy(from, to).map_err(|error| ToolError::io(format!("{} to {}", from.display(), to.display()), error))?;
    if copied != metadata.len() {
        return Err(copy_changed(from));
    }
    check_copy_entry(from, metadata)?;
    output.set_permissions(metadata.permissions()).map_err(|error| ToolError::io(to.display(), error))
}

struct CopyBackups<'a> {
    root: &'a Path,
    entries: Vec<(PathBuf, std::fs::Metadata)>,
    owned: Vec<PathBuf>,
    owned_identities: Vec<(PathBuf, FileHandle)>,
}

impl<'a> CopyBackups<'a> {
    fn discover(root: &'a Path) -> Result<Self, ToolError> {
        Self::discover_with_cancel(root, &CancelToken::default())
    }

    fn discover_with_cancel(root: &'a Path, stop: &CancelToken) -> Result<Self, ToolError> {
        let mut entries = Vec::new();
        for name in crate::feed::DAMAGED_BACKUP_NAMES {
            stop.check()?;
            let path = root.join(name);
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) => entries.push((path, metadata)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(ToolError::io(path.display(), error)),
            }
        }
        let owned = crate::feed::owned_backups_with_cancel(root, stop)?;
        // Keep verified identities across discovery hooks. A replaced staged proof with
        // identical bytes must not relabel the original recovery file as personal content.
        let mut owned_identities = Vec::new();
        for path in &owned {
            let (mut file, metadata) = craft_anything_platform::fs::open_regular_file(path)?;
            if !crate::feed::owns_open_backup_with_cancel(root, path, &mut file, stop)? {
                return Err(copy_changed(path));
            }
            check_copy_entry(path, &metadata)?;
            let identity = FileHandle::from_file(file).map_err(|error| ToolError::io(path.display(), error))?;
            owned_identities.push((path.clone(), identity));
        }
        Ok(Self { root, entries, owned, owned_identities })
    }

    /// A spelling alone is not ownership. On a case-sensitive filesystem an uppercase
    /// personal file can coexist with a reserved slot, even with identical bytes.
    fn slot_for(&self, path: &Path, file: &std::fs::File) -> Result<Option<PathBuf>, ToolError> {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else { return Ok(None) };
        let Some(slot) = crate::feed::DAMAGED_BACKUP_NAMES.iter().find(|slot| name.eq_ignore_ascii_case(slot)) else { return Ok(None) };
        let slot = self.root.join(slot);
        if slot == path {
            return Ok(Some(slot));
        }
        match std::fs::symlink_metadata(&slot) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(ToolError::io(slot.display(), error)),
        }
        let (reserved, metadata) = craft_anything_platform::fs::open_regular_file(&slot)?;
        let reserved = FileHandle::from_file(reserved).map_err(|error| ToolError::io(slot.display(), error))?;
        let opened = FileHandle::from_file(file.try_clone().map_err(|error| ToolError::io(path.display(), error))?)
            .map_err(|error| ToolError::io(path.display(), error))?;
        check_copy_entry(&slot, &metadata)?;
        Ok((reserved == opened).then_some(slot))
    }

    fn owns_open(&self, path: &Path, file: &mut std::fs::File, metadata: &std::fs::Metadata) -> Result<bool, ToolError> {
        self.owns_open_with_cancel(path, file, metadata, &CancelToken::default())
    }

    fn owns_missing_alias(&self, path: &Path, file: &mut std::fs::File, stop: &CancelToken) -> Result<bool, ToolError> {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else { return Ok(false) };
        let Some(slot) = crate::feed::DAMAGED_BACKUP_NAMES.iter().find(|slot| name.eq_ignore_ascii_case(slot)) else { return Ok(false) };
        let canonical = self.root.join(slot);
        match std::fs::symlink_metadata(&canonical) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(ToolError::io(canonical.display(), error)),
            Ok(_) => return Ok(false), // A distinct personal spelling can coexist with this slot.
        }
        let known = self.owned_identities.iter().find_map(|(path, identity)| (path == &canonical).then_some(identity));
        match crate::feed::classify_open_backup_alias_with_cancel(self.root, slot, file, known, stop)? {
            Some(owned) => Ok(owned),
            None if self.owned.contains(&canonical) => Err(copy_changed(path)),
            None => Ok(false),
        }
    }

    fn owns_open_with_cancel(&self, path: &Path, file: &mut std::fs::File, metadata: &std::fs::Metadata, stop: &CancelToken) -> Result<bool, ToolError> {
        stop.check()?;
        check_copy_entry(path, metadata)?;
        let slot = self.slot_for(path, file)?;
        let owned = match &slot {
            Some(slot) => crate::feed::owns_open_backup_with_cancel(self.root, slot, file, stop)?,
            None => self.owns_missing_alias(path, file, stop)?,
        };
        check_copy_entry(path, metadata)?;
        if let Some(slot) = &slot {
            check_copy_entry(slot, metadata)?;
        }
        // A recovery copy is written before its receipt. A new or changed unrecognized
        // entry needs a fresh attempt; losing a receipt cannot turn known recovery into app data.
        if !owned
            && let Some(slot) = &slot
            && (self.owned.iter().any(|path| path == slot) || !self.entries.iter().any(|(path, before)| path == slot && same_copy_entry(before, metadata)))
        {
            return Err(copy_changed(path));
        }
        Ok(owned)
    }
}

/// Recovery may add or replace an entry after discovery. Inspect and copy one open handle;
/// observed changes stop the copy, rather than silently omitting personal content.
fn copy_personal_backup(backups: &CopyBackups<'_>, from: &Path, to: &Path) -> Result<(), ToolError> {
    let (mut file, metadata) = craft_anything_platform::fs::open_regular_file(from)?;
    if backups.owns_open(from, &mut file, &metadata)? {
        return Ok(());
    }
    #[cfg(windows)]
    {
        copy_new_windows_file(from, to, &file, &metadata)
    }
    #[cfg(not(windows))]
    {
        use std::io::Read as _;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
            options.mode(metadata.permissions().mode());
        }
        let mut output = options.open(to).map_err(|error| ToolError::io(to.display(), error))?;
        let copied = std::io::copy(&mut (&mut file).take(metadata.len()), &mut output).map_err(|error| ToolError::io(from.display(), error))?;
        let mut extra = [0u8; 1];
        if copied != metadata.len() || file.read(&mut extra).map_err(|error| ToolError::io(from.display(), error))? != 0 {
            return Err(copy_changed(from));
        }
        check_copy_entry(from, &metadata)?;
        output.set_permissions(metadata.permissions()).map_err(|error| ToolError::io(to.display(), error))?;
        Ok(())
    }
}

fn copy_changed(path: &Path) -> ToolError {
    ToolError::new(ErrorCode::FileChanged, format!("{} changed while the project's files were being checked.", path.display()))
        .with_remediation("Let conversation recovery or edits finish, then retry. The original project is unchanged.")
}

fn check_copy_entry(path: &Path, opened: &std::fs::Metadata) -> Result<(), ToolError> {
    let current = std::fs::symlink_metadata(path).map_err(|error| ToolError::io(path.display(), error))?;
    if same_copy_entry(&current, opened) { Ok(()) } else { Err(copy_changed(path)) }
}

fn same_copy_entry(current: &std::fs::Metadata, opened: &std::fs::Metadata) -> bool {
    let same_metadata = current.is_file()
        && opened.is_file()
        && !current.file_type().is_symlink()
        && !opened.file_type().is_symlink()
        && current.len() == opened.len()
        && current.modified().ok() == opened.modified().ok()
        && current.created().ok() == opened.created().ok();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        same_metadata
            && current.dev() == opened.dev()
            && current.ino() == opened.ino()
            && current.ctime() == opened.ctime()
            && current.ctime_nsec() == opened.ctime_nsec()
    }
    #[cfg(not(unix))]
    {
        same_metadata
    }
}

/// Ghidra was stopped for this project's builds (`tools::deep::STOPPED`), and its lock file.
pub const GHIDRA_STOPPED: &str = "ghidra-stopped";
pub const GHIDRA_STOP_LOCK: &str = "ghidra-stop.lock";

/// The current `craft.json` format version. Bumping it means appending a step to [`MANIFEST_STEPS`].
pub const MANIFEST_VERSION: u32 = 7;

/// The steps that bring an older `craft.json` forward; step `i` goes from version `i` to `i + 1`.
const MANIFEST_STEPS: &[versioned::Step<Value>] =
    &[manifest_v0_to_v1, manifest_v1_to_v2, manifest_v2_to_v3, manifest_v3_to_v4, manifest_v4_to_v5, manifest_v5_to_v6, manifest_v6_to_v7];
const _: () = assert!(MANIFEST_STEPS.len() == MANIFEST_VERSION as usize, "one step per craft.json version");

/// Version 0 (no `version` field) has the same shape as version 1; only the number is new.
fn manifest_v0_to_v1(doc: &mut Value) -> Result<(), String> {
    if doc.is_object() { Ok(()) } else { Err("the manifest isn't a JSON object".into()) }
}

/// Version 2 adds where the app is (`app_in_root`) and the files a picked folder started with;
/// both default to a project made the usual way. The number is new so an older build, which would
/// look for the app in app/, opens such a project read-only instead of saving over it.
fn manifest_v1_to_v2(doc: &mut Value) -> Result<(), String> {
    if doc.is_object() { Ok(()) } else { Err("the manifest isn't a JSON object".into()) }
}

/// Version 3 adds the person's approval choices (`approvals`), none by default. The number is new
/// so an older build, which would drop them when it saves (a refused kind would then follow Auto
/// mode again), opens such a project read-only instead.
fn manifest_v2_to_v3(doc: &mut Value) -> Result<(), String> {
    if doc.is_object() { Ok(()) } else { Err("the manifest isn't a JSON object".into()) }
}

/// Version 4 adds the mode a saved builder session ran in (`agent_session_deep`). A version-3
/// manifest predates the field; a session it saved is the legacy case this build treats as Deep,
/// so the step fills `true` for it. A mode an early build of this change already wrote is kept.
fn manifest_v3_to_v4(doc: &mut Value) -> Result<(), String> {
    let obj = doc.as_object_mut().ok_or("the manifest isn't a JSON object")?;
    let has_session = obj.get("agent_session").and_then(Value::as_str).is_some_and(|s| !s.is_empty());
    if has_session && !obj.contains_key("agent_session_deep") {
        obj.insert("agent_session_deep".into(), Value::Bool(true));
    }
    Ok(())
}

/// Version 5 adds the mode the current `capture/` was studied in (`capture_deep`). A version-4
/// manifest predates the field; a capture it left is the legacy case this build treats as Deep,
/// so the step fills `true` for it. A mode an early build of this change already wrote is kept.
fn manifest_v4_to_v5(doc: &mut Value) -> Result<(), String> {
    let obj = doc.as_object_mut().ok_or("the manifest isn't a JSON object")?;
    if !obj.contains_key("capture_deep") {
        obj.insert("capture_deep".into(), Value::Bool(true));
    }
    Ok(())
}

/// Version 6 adds the toolchain a move to another language was confirmed with (`toolchain`), none
/// by default. The number is new so an older build, which would drop it when it saves (and a
/// Java port's checks could go back to Maven), opens such a project read-only instead.
fn manifest_v5_to_v6(doc: &mut Value) -> Result<(), String> {
    if doc.is_object() { Ok(()) } else { Err("the manifest isn't a JSON object".into()) }
}

/// Version 7 keeps a random project generation. Older projects keep none: reading a manifest
/// must not create a different identity on each open or require writing a read-only project.
/// New projects always get a generation, so even a reused folder and timestamp differ. The
/// version makes older builds read-only instead of silently dropping the identity on save.
fn manifest_v6_to_v7(doc: &mut Value) -> Result<(), String> {
    if doc.is_object() { Ok(()) } else { Err("the manifest isn't a JSON object".into()) }
}

/// Read a `craft.json` document of any version this build understands. A newer manifest is read
/// as far as its fields are known and keeps its version, so [`Workspace::save`] refuses to write it.
pub fn read_manifest(bytes: &[u8]) -> Result<Manifest, ToolError> {
    let mut doc: Value = serde_json::from_slice(bytes).map_err(|e| ToolError::invalid(format!("it isn't valid JSON: {e}")))?;
    let found = match doc.get("version") {
        None | Some(Value::Null) => 0,
        Some(v) => v.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| ToolError::invalid(format!("its version ({v}) isn't a whole number")))?,
    };
    if found > MANIFEST_VERSION {
        return serde_json::from_value(doc).map_err(|e| {
            ToolError::unsupported(format!(
                "This project was made by a newer Craft Anything (craft.json version {found}; this one reads up to {MANIFEST_VERSION}) and can't be read: {e}"
            ))
            .with_remediation("Update Craft Anything to open it.")
        });
    }
    let reached = upgrade(&mut doc, found, 0, MANIFEST_STEPS).map_err(|e| ToolError::invalid(format!("it couldn't be brought up to date: {e}")))?;
    if let Some(obj) = doc.as_object_mut() {
        obj.insert("version".into(), Value::from(reached));
    }
    serde_json::from_value(doc).map_err(|e| ToolError::invalid(e.to_string()))
}

/// The JSON Schema of `craft.json` as published in `docs/schemas/craft.schema.json`.
pub fn manifest_schema() -> String {
    let schema = schemars::generate::SchemaSettings::draft2020_12().into_generator().into_root_schema_for::<Manifest>();
    serde_json::to_string_pretty(&schema).map(|s| s + "\n").unwrap_or_default()
}

/// What is being rebuilt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TargetRef {
    Website {
        url: String,
    },
    /// An app, game, executable, archive or folder on this computer.
    Local {
        path: PathBuf,
    },
    /// Only a description.
    None,
}

impl TargetRef {
    /// A URL becomes a website; an existing path a local target; anything else nothing.
    pub fn parse(s: &str) -> TargetRef {
        let t = s.trim().trim_matches('"');
        if t.starts_with("http://") || t.starts_with("https://") {
            return TargetRef::Website { url: t.to_string() };
        }
        let p = PathBuf::from(t);
        if !t.is_empty() && p.is_absolute() && p.exists() { TargetRef::Local { path: p } } else { TargetRef::None }
    }

    pub fn label(&self) -> String {
        match self {
            TargetRef::Website { url } => url.clone(),
            TargetRef::Local { path } => path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string()),
            TargetRef::None => String::new(),
        }
    }
}

/// Where a workspace is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    New,
    Inspecting,
    Building,
    Checking,
    /// Waiting for the person (a question, an approval, or after an error).
    NeedsYou,
    Ready,
    Stopped,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::New => "New",
            Status::Inspecting => "Studying",
            Status::Building => "Building",
            Status::Checking => "Checking",
            Status::NeedsYou => "Needs you",
            Status::Ready => "Ready",
            Status::Stopped => "Paused",
        }
    }
}

/// The four stages the person sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Study,
    Build,
    Check,
    Done,
}

impl Step {
    pub const ALL: [Step; 4] = [Step::Study, Step::Build, Step::Check, Step::Done];

    pub fn label(self) -> &'static str {
        match self {
            Step::Study => "Study",
            Step::Build => "Build",
            Step::Check => "Check",
            Step::Done => "Ready",
        }
    }
}

/// One check of the build: whether it works and, when it could be compared with the original,
/// how alike it is. Decisions read `passed` and `similarity`, never `note` (which is for people).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(from = "CheckRecord")]
pub struct CheckResult {
    pub round: u32,
    /// Which check ran: [`CheckResult::BUILD`], [`CheckResult::WEB_VISUAL`], or another check's id.
    pub check: String,
    /// The build works: it built, or it was served and compared.
    pub passed: bool,
    /// 0..=100, higher is closer. `None` when the build wasn't compared.
    pub similarity: Option<f64>,
    pub note: String,
    /// Files the check made, relative to the project folder (screenshots…).
    pub artifacts: Vec<PathBuf>,
    pub at_ms: u64,
    /// This check's own bar for `similarity` (e.g. likeness to the person's pictures), instead of
    /// the run's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub good_enough: Option<f64>,
    /// Each page compared, for a website with several (`similarity` is their mean).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<PageLikeness>,
    /// What the check meant to compare but couldn't ("/about at 768 px couldn't be pictured").
    /// Missing evidence never counts as alike, so a check with any isn't good enough.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unknowns: Vec<String>,
    /// The things the build must do, each judged on its own; one that fails keeps the check from
    /// being good enough, however alike the rest looks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub claims: Vec<Claim>,
}

/// One thing a build must do, checked on its own ("The start page doesn't scroll sideways at
/// 390 px").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Claim {
    pub title: String,
    pub passed: bool,
    /// Why it failed, in plain words (empty when it passed).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

impl Claim {
    pub fn new(title: impl Into<String>, passed: bool, detail: impl Into<String>) -> Claim {
        Claim { title: title.into(), passed, detail: if passed { String::new() } else { detail.into() } }
    }
}

/// How alike one page of a website and its rebuild are.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PageLikeness {
    /// What people see: "start page" or the page's path ("/about").
    pub name: String,
    /// Its screenshots' folder inside `capture/` and `checks/round-N/`: "" for the start page,
    /// `routes/<slug>` for another.
    pub dir: String,
    /// 0..=100, as [`CheckResult::similarity`].
    pub similarity: f64,
}

impl CheckResult {
    /// The project's own build ran (cargo, npm, or the game engine's).
    pub const BUILD: &'static str = "build";
    /// The built site was served and its screenshots compared with the original's.
    pub const WEB_VISUAL: &'static str = "web-visual";

    pub fn new(check: &str, round: u32, passed: bool, note: impl Into<String>) -> CheckResult {
        CheckResult {
            round,
            check: check.into(),
            passed,
            similarity: None,
            note: note.into(),
            artifacts: Vec::new(),
            at_ms: crate::inbox::now_ms(),
            good_enough: None,
            pages: Vec::new(),
            unknowns: Vec::new(),
            claims: Vec::new(),
        }
    }

    /// Passed with nothing left unknown and every claim holding, and (when compared) at least as
    /// alike as its own bar, else `good_enough`.
    pub fn good(&self, good_enough: f64) -> bool {
        self.passed
            && self.unknowns.is_empty()
            && self.claims.iter().all(|c| c.passed)
            && self.similarity.is_none_or(|s| s >= self.good_enough.unwrap_or(good_enough))
    }

    /// The claims that don't hold yet.
    pub fn failed_claims(&self) -> impl Iterator<Item = &Claim> {
        self.claims.iter().filter(|c| !c.passed)
    }
}

/// A check as stored. Projects from before `check` and `passed` existed are read the way their
/// results were judged then: compared means it worked, else the note said "Build succeeded".
#[derive(Deserialize, JsonSchema)]
struct CheckRecord {
    round: u32,
    #[serde(default)]
    check: Option<String>,
    #[serde(default)]
    passed: Option<bool>,
    similarity: Option<f64>,
    note: String,
    #[serde(default)]
    artifacts: Vec<PathBuf>,
    at_ms: u64,
    #[serde(default)]
    good_enough: Option<f64>,
    #[serde(default)]
    pages: Vec<PageLikeness>,
    #[serde(default)]
    unknowns: Vec<String>,
    #[serde(default)]
    claims: Vec<Claim>,
}

impl From<CheckRecord> for CheckResult {
    /// Turn a stored check record into the result the manifest keeps.
    fn from(r: CheckRecord) -> CheckResult {
        let compared = r.similarity.is_some();
        CheckResult {
            round: r.round,
            check: r.check.unwrap_or_else(|| (if compared { CheckResult::WEB_VISUAL } else { CheckResult::BUILD }).into()),
            passed: r.passed.unwrap_or(compared || r.note.starts_with("Build succeeded")),
            similarity: r.similarity,
            note: r.note,
            artifacts: r.artifacts,
            at_ms: r.at_ms,
            good_enough: r.good_enough,
            pages: r.pages,
            unknowns: r.unknowns,
            claims: r.claims,
        }
    }
}

/// A workspace's manifest (`craft.json`): what the person asked for, the target, status and history.
/// Its schema is published in `docs/schemas/craft.schema.json` (`cargo xtask schemas`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Manifest {
    /// The craft.json format version. Missing means 0: written before files had versions.
    #[serde(default)]
    pub version: u32,
    pub name: String,
    /// What the person asked for, in their words.
    pub request: String,
    pub target: TargetRef,
    /// The stack chosen for the build ("React + TypeScript (Vite)", "Rust + Bevy"…).
    #[serde(default)]
    pub stack: String,
    /// The toolchain a move to `stack` was confirmed with ("Gradle", for a Java port that wrote
    /// build.gradle.kts beside an old pom.xml), when the stack has more than one; none: not recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolchain: Option<String>,
    #[serde(default)]
    pub status: Status,
    /// One line on what's happening or what went wrong.
    #[serde(default)]
    pub note: String,
    pub created_ms: u64,
    pub updated_ms: u64,
    /// Opaque identity assigned when a project is created, independent of its folder and clock.
    /// None means a project from before generations were kept; ordinary saves preserve it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<String>,
    #[serde(default)]
    pub checks: Vec<CheckResult>,
    /// The coding agent's session, so a later request continues the same conversation.
    #[serde(default)]
    pub agent_session: Option<String>,
    /// The coding agent that saved `agent_session` (`claude-code`, `codex`…); none: saved before
    /// this was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session_by: Option<String>,
    /// The mode `agent_session` was saved in (`true`: Deep); none: saved before the mode was
    /// kept, so it is only resumed in Deep (it may have read the original there).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session_deep: Option<bool>,
    /// The mode the current `capture/` was studied in (`true`: Deep); none: no study by this
    /// build recorded it (a failed study, or a capture copied in), so a run studies again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_deep: Option<bool>,
    /// Kinds of agent action the person allowed for this workspace ("bash:npm", "web_fetch"…).
    #[serde(default)]
    pub allowed_actions: Vec<String>,
    /// The agent's last plain-words summary, shown when the project is reopened.
    #[serde(default)]
    pub last_reply: String,
    /// What generated pictures and 3D models may cost before the person is asked (US dollars).
    #[serde(default = "default_asset_budget")]
    pub asset_budget_usd: f64,
    /// The person lets the builder generate pictures and 3D models (paid, so off until they turn
    /// it on; it also needs a key for an asset provider).
    #[serde(default)]
    pub generate_assets: bool,
    /// The app is the project folder itself: the person's own files, which the builder edits in
    /// place, instead of app/.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub app_in_root: bool,
    /// The names in the person's folder when the project started there (at most
    /// [`START_FILES_SHOWN`]), so the builder knows what to start from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub start_files: Vec<String>,
    /// The AI the person chose to build this project (a backend's name); none: the default
    /// builder. The person's, like the budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builder: Option<String>,
    /// What the builder's questions get in this project; none: the app's default
    /// (`Settings::questions`). The person's, like the budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub questions: Option<Questions>,
    /// What the builder's approvals get in this project, by kind of action; a kind that isn't here
    /// follows Auto mode ([`ActionKind::default_in`]). The person's, like the budget.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    #[schemars(schema_with = "approvals_schema")]
    pub approvals: std::collections::BTreeMap<ActionKind, Approval>,
    /// The builder finished its turn and the run's checks haven't reported on it yet, so a run
    /// that picks this up only needs to check: nothing is left to build.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub awaiting_check: bool,
    /// Checks in a row, across runs, that found the build not working (it didn't build or serve).
    /// A run stops when this reaches the limit; a new request from the person starts it again.
    /// Kept in the project so "Try again" can't restart the count.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub not_working_checks: u32,
    /// The builder has read the original's code (a decompiled function) in this project, in any run.
    /// Deep mode reminds a builder that hasn't; kept here because every request starts a new run, so
    /// a run-only flag repeated the reminder after each request. An older build drops it when it
    /// saves, which costs at most one more reminder, so the format version stays.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub decompiled: bool,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl Manifest {
    /// The person's own project: their files are the app, edited in place, with nothing to recreate.
    pub fn own_project(&self) -> bool {
        self.app_in_root && self.target == TargetRef::None
    }

    /// The manifest for a new project, at the current format version.
    fn new(name: &str, request: &str, target: TargetRef) -> Result<Manifest, ToolError> {
        let mut generation = [0u8; 16];
        getrandom::fill(&mut generation).map_err(|error| {
            ToolError::internal(format!("A new project identity could not be generated: {error}"))
                .with_remediation("Retry creating the project when the operating system's random source is available. No project was created.")
        })?;
        let now = now_ms();
        Ok(Manifest {
            version: MANIFEST_VERSION,
            name: name.trim().to_string(),
            request: request.trim().to_string(),
            target,
            stack: String::new(),
            status: Status::New,
            note: String::new(),
            created_ms: now,
            updated_ms: now,
            generation: Some(hex::encode(generation)),
            checks: Vec::new(),
            agent_session: None,
            agent_session_by: None,
            toolchain: None,
            agent_session_deep: None,
            capture_deep: None,
            allowed_actions: Vec::new(),
            last_reply: String::new(),
            asset_budget_usd: ASSET_BUDGET_USD,
            generate_assets: false,
            app_in_root: false,
            start_files: Vec::new(),
            builder: None,
            questions: None,
            approvals: Default::default(),
            awaiting_check: false,
            not_working_checks: 0,
            decompiled: false,
        })
    }

    /// The saved session, if `agent` (`claude-code`, `codex`…) saved it: another agent can't
    /// continue it (#469). One saved before the agent was kept counts.
    pub fn agent_session_for(&self, agent: &str) -> Option<&str> {
        self.agent_session.as_deref().filter(|_| self.agent_session_by.as_deref().is_none_or(|a| a == agent))
    }

    /// The session `agent` saved in this mode. One saved before the mode was kept counts as
    /// Deep: resuming it in Clean room could carry what it read there.
    pub fn agent_session_for_mode(&self, agent: &str, deep: bool) -> Option<&str> {
        self.agent_session_for(agent).filter(|_| self.agent_session_deep.unwrap_or(true) == deep)
    }

    /// Take the fields the person owns from `other` (what's on disk).
    fn take_person_fields(&mut self, other: &Manifest) {
        self.asset_budget_usd = other.asset_budget_usd;
        self.generate_assets = other.generate_assets;
        self.allowed_actions = other.allowed_actions.clone();
        self.builder.clone_from(&other.builder);
        self.questions = other.questions;
        self.approvals.clone_from(&other.approvals);
        self.name.clone_from(&other.name);
    }
}

/// The starting budget for generated assets.
pub const ASSET_BUDGET_USD: f64 = 5.0;

fn default_asset_budget() -> f64 {
    ASSET_BUDGET_USD
}

/// One generated asset (a picture or a 3D model) and what it cost.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetRecord {
    /// Where it was saved, relative to app/.
    pub file: String,
    /// "image" or "model".
    pub kind: String,
    /// What it was made from (the description, or the picture it was made from).
    pub prompt: String,
    pub cost_usd: f64,
    pub at_ms: u64,
}

/// The list of generated assets, in the workspace folder.
const ASSETS: &str = "assets.json";

/// What a project's builds and questions have taken and used, every run together (`usage.json`).
/// A rebuild turn and a question can add to it; everything else reads it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectUsage {
    /// The usage.json format version ([`USAGE_VERSION`]).
    pub version: u32,
    /// How long the builds have worked, in milliseconds.
    pub work_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// At least one turn didn't report tokens. The counts only include reported usage.
    pub tokens_incomplete: bool,
    /// Everything reported, in US dollars: what API keys were charged plus coding agents' own
    /// estimates at API prices.
    pub cost_usd: f64,
    /// The part of `cost_usd` charged to an API key. The rest is an estimate, which is all files
    /// from before this field held.
    pub api_billed_usd: f64,
    /// Some of it came with a price; agents that report none leave tokens only.
    pub priced: bool,
}

/// What a project's runs did, line by line; the run is its only writer.
pub const ACTIVITY_LOG: &str = "build-activity.log";
/// The log before it, set aside when it grew long.
pub const ACTIVITY_LOG_OLD: &str = "build-activity.old.log";

/// Where [`ProjectUsage`] is kept in the project folder.
pub const USAGE_FILE: &str = "usage.json";
pub const USAGE_VERSION: u32 = 1;
/// A build's turn and a question can record their cost at the same time. The save is
/// read-modify-write, so both go through one lock or one would overwrite the other.
static USAGE_UPDATE: Mutex<()> = Mutex::new(());
/// A real usage.json is a few hundred bytes; anything this big isn't one.
const MAX_USAGE_BYTES: u64 = 1024 * 1024;

/// What reading a project's usage.json found.
#[derive(Debug, Clone, PartialEq)]
pub enum UsageRead {
    /// There is no file: nothing has been used yet.
    None,
    Known(ProjectUsage),
    /// The file is there but couldn't be read or understood, so what was used isn't known.
    /// It is never shown as zero.
    Unreadable,
}

impl UsageRead {
    /// The usage as far as it's known: `None` when the file couldn't be read.
    pub fn known(self) -> Option<ProjectUsage> {
        match self {
            UsageRead::None => Some(ProjectUsage::default()),
            UsageRead::Known(u) => Some(u),
            UsageRead::Unreadable => None,
        }
    }

    pub fn is_unreadable(&self) -> bool {
        matches!(self, UsageRead::Unreadable)
    }
}

impl ProjectUsage {
    /// Read the saved usage of the project at `root`. Only a missing file means nothing was used;
    /// any other failure, or a file that doesn't parse, is [`UsageRead::Unreadable`].
    pub fn read(root: &Path) -> UsageRead {
        match read_file_capped(&root.join(USAGE_FILE), MAX_USAGE_BYTES) {
            Err(e) if e.code == ErrorCode::NotFound => UsageRead::None,
            Err(_) => UsageRead::Unreadable,
            Ok(bytes) => serde_json::from_slice(&bytes).map_or(UsageRead::Unreadable, UsageRead::Known),
        }
    }

    /// Change the saved usage (the run's job). A file from a newer Craft Anything, or one that
    /// can't be read, is left alone: writing over it would lose what it counted.
    pub fn update(root: &Path, f: impl FnOnce(&mut ProjectUsage)) -> Result<(), ToolError> {
        let _updating = USAGE_UPDATE.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(mut u) = ProjectUsage::read(root).known() else {
            return Err(ToolError::invalid(format!("{USAGE_FILE} couldn't be read, so it isn't changed.")));
        };
        if u.version > USAGE_VERSION {
            return Err(ToolError::unsupported(format!("{USAGE_FILE} was written by a newer Craft Anything, so it isn't changed.")));
        }
        f(&mut u);
        u.version = USAGE_VERSION;
        let json = serde_json::to_vec_pretty(&u).map_err(|e| ToolError::internal(e.to_string()))?;
        write_atomic(&root.join(USAGE_FILE), &json)
    }

    /// One agent turn's tokens and reported cost (a price that isn't a sensible number is skipped).
    /// `estimate`: the cost is the agent's own guess at API prices, not a key's charge.
    pub fn add_turn(&mut self, input: u64, output: u64, cost_usd: Option<f64>, estimate: bool) {
        self.input_tokens = self.input_tokens.saturating_add(input);
        self.output_tokens = self.output_tokens.saturating_add(output);
        if let Some(c) = cost_usd.filter(|c| c.is_finite() && *c >= 0.0) {
            self.cost_usd += c;
            if !estimate {
                self.api_billed_usd += c;
            }
            self.priced = true;
        }
    }

    /// The part of the cost that is a coding agent's estimate rather than a key's charge.
    pub fn estimated_usd(&self) -> f64 {
        (self.cost_usd - self.api_billed_usd).max(0.0)
    }

    pub fn tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    pub fn used_anything(&self) -> bool {
        self.tokens() > 0 || self.tokens_incomplete || self.priced
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    pub root: PathBuf,
    pub manifest: Manifest,
    /// Nonfatal cleanup guidance from this successful creation, never saved in the manifest.
    pub creation_warning: Option<String>,
}

/// The absolute form of a project path, for a child that changes into the project folder. An
/// already-absolute path is kept; one that can't be made absolute is returned unchanged.
pub fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// `path` with `.` and `..` resolved lexically (no symlinks), made absolute when relative.
pub fn normalized(path: &Path) -> PathBuf {
    let path = absolute(path);
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// A folder-name slug: lower-case words joined by dashes, at most 40 characters.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 40 {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "project".into() } else { out }
}

/// A short name from a request and target: the site's host, the file's stem, or the first words.
pub fn suggest_name(request: &str, target: &TargetRef) -> String {
    match target {
        TargetRef::Website { url } => {
            let host = url.split("://").nth(1).unwrap_or(url).split(['/', '?', '#', ':']).next().unwrap_or_default();
            host.trim_start_matches("www.").to_string()
        }
        TargetRef::Local { path } => path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        TargetRef::None => request.split_whitespace().take(5).collect::<Vec<_>>().join(" "),
    }
}

/// Claim a fresh directory without publishing a project manifest yet.
fn create_workspace_root(projects_dir: &Path, name: &str) -> Result<(PathBuf, FileHandle), ToolError> {
    create_workspace_root_after_mkdir(projects_dir, name, |_| {})
}

fn create_workspace_root_after_mkdir(projects_dir: &Path, name: &str, after_mkdir: impl FnOnce(&Path)) -> Result<(PathBuf, FileHandle), ToolError> {
    std::fs::create_dir_all(projects_dir).map_err(|e| ToolError::io(projects_dir.display(), e))?;
    let base = slug(name);
    let mut root = projects_dir.join(&base);
    let mut n = 2;
    while let Err(e) = std::fs::create_dir(&root) {
        if e.kind() != std::io::ErrorKind::AlreadyExists || n > 999 {
            return Err(ToolError::io(root.display(), e));
        }
        root = projects_dir.join(format!("{base}-{n}"));
        n += 1;
    }
    after_mkdir(&root);
    let directory = FileHandle::from_path(&root).map_err(|error| ToolError::io(root.display(), error))?;
    CreationReservation::check_directory(&root, &directory)?;
    if let Some(entry) = std::fs::read_dir(&root).map_err(|error| ToolError::io(root.display(), error))?.next() {
        entry.map_err(|error| ToolError::io(root.display(), error))?;
        return Err(copy_changed(&root));
    }
    Ok((root, directory))
}

/// Every initial creator claims the existing reserved inbox name before publishing a manifest.
/// After publication, keep that directory until project deletion: removing it would race a
/// feed writer creating its persistent lock. An interrupted creation remains visibly reserved.
struct CreationReservation {
    directory: FileHandle,
    inbox: FileHandle,
    /// Initial creators hold the same exclusive inode as feed initializers until publication
    /// or cleanup finishes. An initializer may instead pass its held handle to release_claimed.
    coordination: Option<FileHandle>,
    cleanup: Option<CreationCleanup>,
}

/// Reserve cleanup space before publication or manifest removal makes allocation too late.
/// Keep a relative name so a failed duplicate can move its root without losing this identity.
pub(crate) struct CreationCleanup {
    name: std::ffi::OsString,
    directory: FileHandle,
}

impl CreationCleanup {
    fn reserve(root: &Path, allocate: &dyn Fn(&Path) -> Result<PathBuf, ToolError>) -> Result<Self, ToolError> {
        let path = allocate(root)?;
        let opened = (|| {
            let name = path.file_name().filter(|_| path.parent() == Some(root)).ok_or_else(|| copy_changed(&path))?.to_os_string();
            let directory = FileHandle::from_path(&path).map_err(|error| ToolError::io(path.display(), error))?;
            CreationReservation::check_directory(&path, &directory)?;
            Ok(Self { name, directory })
        })();
        opened.map_err(|error: ToolError| {
            error
                .with_remediation(format!("Inspect the reserved cleanup directory at {} before retrying; its identity could not be confirmed.", path.display()))
        })
    }

    fn checked_path(&self, root: &Path) -> Result<PathBuf, ToolError> {
        let path = root.join(&self.name);
        CreationReservation::check_directory(&path, &self.directory)?;
        Ok(path)
    }

    fn empty_path(&self, root: &Path) -> Result<PathBuf, ToolError> {
        let path = self.checked_path(root)?;
        if let Some(entry) = std::fs::read_dir(&path).map_err(|error| ToolError::io(path.display(), error))?.next() {
            entry.map_err(|error| ToolError::io(path.display(), error))?;
            return Err(ToolError::invalid(format!("{} is not empty; its contents were kept.", path.display())));
        }
        Ok(path)
    }

    fn remove_empty(&self, root: &Path) -> Result<(), ToolError> {
        let path = self.empty_path(root)?;
        std::fs::remove_dir(&path).map_err(|error| ToolError::io(path.display(), error))
    }
}

/// The public name is released only after its old coordination entries have been retained.
/// On Windows an open descendant prevents the directory move, even with delete sharing.
struct ClaimedInbox {
    path: PathBuf,
    detached: Vec<(std::ffi::OsString, PathBuf)>,
}

impl ClaimedInbox {
    fn coordination_path(&self, name: &std::ffi::OsStr) -> PathBuf {
        self.detached
            .iter()
            .find(|(original, _)| original == name)
            .or_else(|| self.detached.iter().find(|(original, _)| cfg!(windows) && original.to_string_lossy().eq_ignore_ascii_case(&name.to_string_lossy())))
            .map_or_else(|| self.path.join(name), |(_, path)| path.clone())
    }

    fn check_after_detach(&self) -> Result<(), ToolError> {
        if !self.detached.is_empty()
            && let Some(entry) = std::fs::read_dir(&self.path).map_err(|error| ToolError::io(self.path.display(), error))?.next()
        {
            let entry = entry.map_err(|error| ToolError::io(self.path.display(), error))?;
            return Err(copy_changed(&entry.path()));
        }
        Ok(())
    }

    fn release(&self, before_remove: impl FnOnce(), remove_owned: impl FnOnce() -> Result<(), ToolError>) -> Result<(), ToolError> {
        self.check_after_detach()?;
        before_remove();
        // Removing the empty directory closes a late initializer's directory-relative
        // create window. On failure every detached lock link must remain inspectable.
        #[cfg(windows)]
        std::fs::remove_dir(&self.path).map_err(|error| ToolError::io(self.path.display(), error))?;
        remove_owned()?;
        #[cfg(not(windows))]
        std::fs::remove_dir(&self.path).map_err(|error| ToolError::io(self.path.display(), error))?;
        Ok(())
    }

    fn claim(
        root: &Path,
        directory: &FileHandle,
        inbox: &FileHandle,
        cleanup: &CreationCleanup,
        held: Option<&FileHandle>,
        after_detach: impl FnOnce(),
    ) -> Result<Self, ToolError> {
        let public = root.join("inbox");
        let private = cleanup.empty_path(root)?;
        let check = || -> Result<(), ToolError> {
            CreationReservation::check_directory(root, directory)?;
            CreationReservation::check_directory(&public, inbox)?;
            CreationReservation::check_manifest_absent(root)?;
            cleanup.checked_path(root)?;
            Ok(())
        };
        check()?;
        #[cfg(windows)]
        let detached = Self::detach_coordination(&public, &private, held, &check)?;
        #[cfg(not(windows))]
        let detached = {
            let _ = held;
            Vec::new()
        };
        after_detach();
        // A delayed initializer may have populated the public directory, or a new manifest
        // may have appeared. Never restore the old lock over a new entry in either case.
        #[cfg(windows)]
        check()?;
        #[cfg(not(windows))]
        cleanup.empty_path(root)?;
        let path = private.join("reservation");
        std::fs::rename(&public, &path).map_err(|error| ToolError::io(public.display(), error))?;
        CreationReservation::check_directory(root, directory)?;
        cleanup.checked_path(root)?;
        CreationReservation::check_directory(&path, inbox)?;
        Ok(Self { path, detached })
    }

    #[cfg(windows)]
    fn detach_coordination(
        public: &Path,
        private: &Path,
        held: Option<&FileHandle>,
        check: &impl Fn() -> Result<(), ToolError>,
    ) -> Result<Vec<(std::ffi::OsString, PathBuf)>, ToolError> {
        let Some(held) = held else { return Ok(Vec::new()) };
        crate::feed::check_coordination_entry(&public.join(crate::feed::PROJECT_LOCK_FILE), held)?;
        let mut names = Vec::new();
        for entry in std::fs::read_dir(public).map_err(|error| ToolError::io(public.display(), error))? {
            let entry = entry.map_err(|error| ToolError::io(public.display(), error))?;
            let metadata = std::fs::symlink_metadata(entry.path()).map_err(|error| ToolError::io(entry.path().display(), error))?;
            if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != 0 {
                continue;
            }
            let current = FileHandle::from_path(entry.path()).map_err(|error| ToolError::io(entry.path().display(), error))?;
            if &current == held {
                crate::feed::check_coordination_entry(&entry.path(), held)?;
                names.push(entry.file_name());
            }
        }
        let mut detached = Vec::new();
        for (index, name) in names.into_iter().enumerate() {
            check()?;
            let public_entry = public.join(&name);
            let retained = private.join(format!("coordination-{index}"));
            Self::detach_entry(&public_entry, &retained, held, || {})?;
            check()?;
            detached.push((name, retained));
        }
        Ok(detached)
    }

    #[cfg(windows)]
    fn detach_entry(public: &Path, retained: &Path, held: &FileHandle, before_move: impl FnOnce()) -> Result<(), ToolError> {
        // Canonicalize only the existing parent: Windows returns its extended-length form,
        // while the final entry remains unresolved so a late replacement link is moved as-is.
        let native_path = |path: &Path| -> Result<PathBuf, ToolError> {
            let parent = path.parent().ok_or_else(|| copy_changed(path))?;
            let name = path.file_name().ok_or_else(|| copy_changed(path))?;
            std::fs::canonicalize(parent).map(|parent| parent.join(name)).map_err(|error| ToolError::io(parent.display(), error))
        };
        let public = native_path(public)?;
        let retained = native_path(retained)?;
        crate::feed::check_coordination_entry(&public, held)?;
        before_move();
        // This is one Windows move without replacement, not link followed by public unlink.
        // A source changed after the check is moved but preserved; an occupied target is
        // never overwritten. The original OS lock remains held through either result.
        atomicwrites::move_atomic(&public, &retained).map_err(|error| ToolError::io(retained.display(), error))?;
        crate::feed::check_coordination_entry(&retained, held)
    }
}

impl CreationReservation {
    fn claim(root: &Path, directory: FileHandle) -> Result<Self, ToolError> {
        Self::claim_with_cleanup(root, directory, &CopyDestination::cleanup_directory)
    }

    fn claim_with_cleanup(root: &Path, directory: FileHandle, allocate: &dyn Fn(&Path) -> Result<PathBuf, ToolError>) -> Result<Self, ToolError> {
        Self::claim_with_creation_lock(root, directory, allocate, crate::feed::lock_creation)
    }

    fn claim_with_creation_lock(
        root: &Path,
        directory: FileHandle,
        allocate: &dyn Fn(&Path) -> Result<PathBuf, ToolError>,
        acquire: impl FnOnce(&Path, &FileHandle) -> Result<FileHandle, Box<crate::feed::CreationLockFailure>>,
    ) -> Result<Self, ToolError> {
        Self::check_directory(root, &directory)?;
        Self::check_manifest_absent(root)?;
        // Reserve before mkdir(inbox): a failed allocation must not strand a new reservation.
        let cleanup = CreationCleanup::reserve(root, allocate)?;
        let cleanup_path = root.join(&cleanup.name);
        let inbox_path = root.join("inbox");
        let mut captured_inbox = None;
        let mut coordination = None;
        let claimed = (|| {
            Self::check_directory(root, &directory)?;
            Self::check_manifest_absent(root)?;
            cleanup.empty_path(root)?;
            #[cfg(unix)]
            let builder = {
                use std::os::unix::fs::DirBuilderExt as _;
                let mut builder = std::fs::DirBuilder::new();
                builder.mode(0o700);
                builder
            };
            #[cfg(not(unix))]
            let builder = std::fs::DirBuilder::new();
            builder.create(&inbox_path).map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    ToolError::invalid(format!("{} is already reserved for a project creation.", root.display()))
                        .with_remediation("Let the other creation finish, or inspect this folder before choosing another one. Its files have not been removed.")
                } else {
                    ToolError::io(inbox_path.display(), error)
                }
            })?;
            let inbox = captured_inbox.insert(FileHandle::from_path(&inbox_path).map_err(|error| ToolError::io(inbox_path.display(), error))?);
            Self::check_directory(root, &directory)?;
            Self::check_manifest_absent(root)?;
            coordination = Some(match acquire(root, inbox) {
                Ok(file) => file,
                Err(failure) => {
                    let error = failure.error.clone();
                    let recovered: Result<_, ToolError> = (|| {
                        Self::check_directory(root, &directory)?;
                        Self::check_manifest_absent(root)?;
                        let file = failure.recover(root, inbox)?;
                        Self::check_directory(root, &directory)?;
                        Self::check_manifest_absent(root)?;
                        Ok(file)
                    })();
                    match recovered {
                        Ok(file) => coordination = file,
                        Err(recovery) => {
                            let guidance = format!("The initial coordination lock could not be recovered safely: {}", recovery.for_people());
                            let guidance = match &error.remediation {
                                Some(previous) => format!("{previous} {guidance}"),
                                None => guidance,
                            };
                            return Err(error.with_remediation(guidance));
                        }
                    }
                    return Err(error);
                }
            });
            Self::check_directory(root, &directory)?;
            Self::check_directory(&inbox_path, inbox)?;
            Self::check_manifest_absent(root)
        })();
        if let Err(error) = claimed {
            let released = match captured_inbox {
                Some(inbox) => Self { directory, inbox, coordination, cleanup: Some(cleanup) }.release_empty_with_cleanup(root, || {}, allocate),
                None => Self::check_directory(root, &directory).and_then(|()| cleanup.remove_empty(root)),
            };
            return Err(match released {
                Ok(()) => error,
                Err(release) => {
                    let guidance = format!("{} Inspect {} and {} before retrying.", release.for_people(), inbox_path.display(), cleanup_path.display());
                    let guidance = match &error.remediation {
                        Some(previous) => format!("{previous} {guidance}"),
                        None => guidance,
                    };
                    error.with_remediation(guidance)
                }
            });
        }
        let inbox = captured_inbox.ok_or_else(|| ToolError::internal("The creation reservation has no directory identity."))?;
        Ok(Self { directory, inbox, coordination, cleanup: Some(cleanup) })
    }

    fn finish_publication(
        &self,
        ws: &Workspace,
        remove: impl FnOnce(&CreationCleanup, &Path) -> Result<(), ToolError>,
    ) -> Result<Option<ToolError>, ToolError> {
        let check = || {
            Self::check_directory(&ws.root, &self.directory)?;
            Self::check_directory(&ws.inbox_dir(), &self.inbox)?;
            if let Some(lock) = &self.coordination {
                crate::feed::check_coordination_entry(&ws.inbox_dir().join(crate::feed::PROJECT_LOCK_FILE), lock)?;
            }
            let published = Workspace::open(&ws.root)?.manifest;
            if published.generation != ws.manifest.generation || published.created_ms != ws.manifest.created_ms {
                return Err(copy_changed(&ws.root.join(MANIFEST)));
            }
            Ok(())
        };
        check()?;
        let warning = self.cleanup.as_ref().and_then(|cleanup| {
            remove(cleanup, &ws.root).err().map(|error| {
                let guidance = format!(
                    "The project manifest was published. Keep the project and inspect the reserved cleanup directory at {} before retrying cleanup.",
                    ws.root.join(&cleanup.name).display()
                );
                let guidance = match &error.remediation {
                    Some(previous) => format!("{previous} {guidance}"),
                    None => guidance,
                };
                error.with_remediation(guidance)
            })
        });
        // Only private-directory cleanup is nonfatal. A changed published project still fails.
        check()?;
        Ok(warning)
    }

    fn check(&self, root: &Path) -> Result<(), ToolError> {
        Self::check_directory(root, &self.directory)?;
        Self::check_directory(&root.join("inbox"), &self.inbox)?;
        if let Some(lock) = &self.coordination {
            crate::feed::check_coordination_entry(&root.join("inbox").join(crate::feed::PROJECT_LOCK_FILE), lock)?;
        }
        Self::check_manifest_absent(root)?;
        if let Some(cleanup) = &self.cleanup {
            cleanup.empty_path(root)?;
        }
        Ok(())
    }

    /// A failed initial write can release only this attempt's empty coordination entries.
    /// A visible manifest, unreadable state or changed identity leaves it for inspection.
    fn release_empty(&self, root: &Path) -> Result<(), ToolError> {
        self.release_empty_after_check(root, || {})
    }

    fn release_empty_after_check(&self, root: &Path, after_check: impl FnOnce()) -> Result<(), ToolError> {
        self.release_empty_with_cleanup(root, after_check, &CopyDestination::cleanup_directory)
    }

    fn release_empty_with_cleanup(
        &self,
        root: &Path,
        after_check: impl FnOnce(),
        allocate: &dyn Fn(&Path) -> Result<PathBuf, ToolError>,
    ) -> Result<(), ToolError> {
        self.check(root)?;
        let inbox = root.join("inbox");
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&inbox).map_err(|error| ToolError::io(inbox.display(), error))? {
            let entry = entry.map_err(|error| ToolError::io(inbox.display(), error))?;
            let Some(lock) = &self.coordination else {
                return Err(ToolError::invalid(format!("{} is not empty; its contents were kept.", inbox.display())));
            };
            crate::feed::check_coordination_entry(&entry.path(), lock)?;
            entries.push(entry.file_name());
        }
        self.release_claimed_with_cleanup(root, after_check, self.coordination.as_ref(), allocate, |claimed| {
            if let Some(lock) = &self.coordination {
                for name in entries {
                    let path = claimed.coordination_path(&name);
                    crate::feed::check_coordination_entry(&path, lock)?;
                    std::fs::remove_file(&path).map_err(|error| ToolError::io(path.display(), error))?;
                }
            }
            Ok(())
        })
    }

    fn release_claimed(
        &self,
        root: &Path,
        after_check: impl FnOnce(),
        held_lock: Option<&FileHandle>,
        remove_owned: impl FnOnce(&ClaimedInbox) -> Result<(), ToolError>,
    ) -> Result<(), ToolError> {
        self.release_claimed_with_cleanup(root, after_check, held_lock, &CopyDestination::cleanup_directory, remove_owned)
    }

    fn release_claimed_with_cleanup(
        &self,
        root: &Path,
        after_check: impl FnOnce(),
        held_lock: Option<&FileHandle>,
        allocate: &dyn Fn(&Path) -> Result<PathBuf, ToolError>,
        remove_owned: impl FnOnce(&ClaimedInbox) -> Result<(), ToolError>,
    ) -> Result<(), ToolError> {
        let _coordination = crate::feed::lock_inbox_for_cleanup(root, held_lock)?;
        let inbox = root.join("inbox");
        let temporary;
        let cleanup = match &self.cleanup {
            Some(cleanup) => cleanup,
            None => {
                temporary = CreationCleanup::reserve(root, allocate)?;
                &temporary
            }
        };
        let private = cleanup.empty_path(root)?;
        let moved = (|| {
            self.check(root)?;
            ClaimedInbox::claim(root, &self.directory, &self.inbox, cleanup, held_lock.or(_coordination.as_ref()), after_check)
        })();
        let claimed = moved.map_err(|error| match cleanup.remove_empty(root) {
            Ok(()) => error,
            Err(cleanup) => error.with_remediation(format!(
                "Inspect the inbox at {} and all retained entries in {} before retrying: {}. Nothing was restored over a new entry.",
                inbox.display(),
                private.display(),
                cleanup.for_people()
            )),
        })?;
        let release = (|| {
            Self::check_directory(root, &self.directory)?;
            cleanup.checked_path(root)?;
            Self::check_directory(&claimed.path, &self.inbox)?;
            Self::check_manifest_absent(root)?;
            // Never recurse: partial writes and unexpected entries must survive cleanup.
            claimed.release(|| {}, || remove_owned(&claimed))
        })();
        release.map_err(|error| {
            ToolError::new(
                error.code,
                format!("{} Cleanup stopped in {}. Inspect any remaining reservation at {}.", error.message, private.display(), claimed.path.display()),
            )
            .with_remediation(format!(
                "Inspect {} and its retained coordination entries before retrying. No directory contents were removed recursively.",
                private.display()
            ))
        })?;
        cleanup.remove_empty(root)
    }

    fn check_directory(path: &Path, expected: &FileHandle) -> Result<(), ToolError> {
        let metadata = std::fs::symlink_metadata(path).map_err(|error| ToolError::io(path.display(), error))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(copy_changed(path));
        }
        let current = FileHandle::from_path(path).map_err(|error| ToolError::io(path.display(), error))?;
        if &current == expected { Ok(()) } else { Err(copy_changed(path)) }
    }

    fn check_manifest_absent(root: &Path) -> Result<(), ToolError> {
        let path = root.join(MANIFEST);
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(copy_changed(&path)),
            Err(error) => Err(ToolError::io(path.display(), error)),
        }
    }
}

/// A failed feed initializer may release only an inbox it created, never an existing reservation.
pub(crate) fn release_created_inbox(
    root: &Path,
    directory: FileHandle,
    inbox: FileHandle,
    created_lock: Option<&FileHandle>,
    held_lock: Option<&FileHandle>,
) -> Result<(), ToolError> {
    let creation = CreationReservation { directory, inbox, coordination: None, cleanup: None };
    creation.check(root)?;
    creation.release_claimed(
        root,
        || {},
        held_lock,
        |claimed| {
            if let Some(expected) = created_lock {
                let path = claimed.coordination_path(std::ffi::OsStr::new(crate::feed::PROJECT_LOCK_FILE));
                let metadata = std::fs::symlink_metadata(&path).map_err(|error| ToolError::io(path.display(), error))?;
                let opened = expected.as_file().metadata().map_err(|error| ToolError::io(path.display(), error))?;
                if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != 0 || !opened.is_file() || opened.len() != 0 {
                    return Err(copy_changed(&path));
                }
                let current = FileHandle::from_path(&path).map_err(|error| ToolError::io(path.display(), error))?;
                if &current != expected {
                    return Err(copy_changed(&path));
                }
                std::fs::remove_file(&path).map_err(|error| ToolError::io(path.display(), error))?;
            }
            Ok(())
        },
    )
}

/// Once a deleter releases the public inbox name, it cleans only its privately claimed entry.
pub(crate) fn finish_deleted_inbox(
    root: &Path,
    directory: &FileHandle,
    inbox: &FileHandle,
    lock: &FileHandle,
    cleanup: &CreationCleanup,
    after_claim: impl FnOnce(),
) -> Result<(), ToolError> {
    let public = root.join("inbox");
    let private = root.join(&cleanup.name);
    let moved = (|| {
        CreationReservation::check_directory(root, directory)?;
        CreationReservation::check_directory(&public, inbox)?;
        CreationReservation::check_manifest_absent(root)?;
        crate::feed::check_coordination_entry(&public.join(crate::feed::PROJECT_LOCK_FILE), lock)?;
        ClaimedInbox::claim(root, directory, inbox, cleanup, Some(lock), || {})
    })();
    let claimed = moved.map_err(|error| {
        let guidance = match cleanup.remove_empty(root) {
            Ok(()) => format!("The inbox was not privately claimed. Inspect {} before retrying cleanup.", public.display()),
            Err(cleanup) => format!(
                "Inspect {} and all retained entries in {} before retrying cleanup: {}. Nothing was restored over a new entry.",
                public.display(),
                private.display(),
                cleanup.for_people()
            ),
        };
        let guidance = match &error.remediation {
            Some(previous) => format!("{previous} {guidance}"),
            None => guidance,
        };
        error.with_remediation(guidance)
    })?;
    after_claim();
    let removed = (|| {
        CreationReservation::check_directory(root, directory)?;
        cleanup.checked_path(root)?;
        CreationReservation::check_directory(&claimed.path, inbox)?;
        // A new creator may now publish at the public name. Do not inspect or remove its inbox.
        let mut entries = claimed.detached.iter().map(|(_, path)| path.clone()).collect::<Vec<_>>();
        for entry in std::fs::read_dir(&claimed.path).map_err(|error| ToolError::io(claimed.path.display(), error))? {
            entries.push(entry.map_err(|error| ToolError::io(claimed.path.display(), error))?.path());
        }
        for path in &entries {
            crate::feed::check_coordination_entry(path, lock)?;
        }
        claimed.release(
            || {},
            || {
                for path in entries {
                    crate::feed::check_coordination_entry(&path, lock)?;
                    std::fs::remove_file(&path).map_err(|error| ToolError::io(path.display(), error))?;
                }
                Ok(())
            },
        )
    })();
    removed.map_err(|error| {
        error.with_remediation(format!(
            "Inspect retained entries in {}, including any remaining reservation at {}. No directory contents were removed recursively.",
            private.display(),
            claimed.path.display()
        ))
    })?;
    cleanup.remove_empty(root)
}

/// Keep the first opened destination identity through copying and cleanup. A held identity
/// detects later replacements but never authorizes recursive deletion of its contents.
struct CopyDestination {
    root: PathBuf,
    creation: CreationReservation,
}

impl CopyDestination {
    fn create(parent: &Path, name: &str) -> Result<Self, ToolError> {
        Self::create_after_root(parent, name, |_| {})
    }

    fn create_after_root(parent: &Path, name: &str, after_root: impl FnOnce(&Path)) -> Result<Self, ToolError> {
        let (root, directory) = create_workspace_root(parent, name)?;
        after_root(&root);
        // Another creator can win between mkdir(root) and the reservation. On failure,
        // leave that directory alone: its contents may already belong to the winner.
        let creation = CreationReservation::claim(&root, directory)?;
        Ok(Self { root, creation })
    }

    fn check_identity(&self) -> Result<(), ToolError> {
        self.check_identity_at(&self.root)
    }

    fn check_identity_at(&self, path: &Path) -> Result<(), ToolError> {
        self.creation.check(path)
    }

    fn remove(self) -> Result<(), ToolError> {
        self.remove_after_check(|| {})
    }

    fn remove_after_check(self, after_check: impl FnOnce()) -> Result<(), ToolError> {
        self.check_identity()?;
        #[cfg(windows)]
        let directory = {
            // Windows cannot move the root with any child directory handle open. Release
            // only this reservation first; retain the root identity through its later claim.
            self.creation.release_empty(&self.root)?;
            let CreationReservation { directory, inbox, coordination, cleanup } = self.creation;
            drop((inbox, coordination, cleanup));
            directory
        };
        after_check();
        #[cfg(windows)]
        {
            CreationReservation::check_directory(&self.root, &directory)?;
            CreationReservation::check_manifest_absent(&self.root)?;
        }
        let parent = self.root.parent().ok_or_else(|| ToolError::invalid("The copy destination has no parent folder."))?;
        let private = Self::cleanup_directory(parent)?;
        let claimed = private.join("claimed");
        if let Err(error) = std::fs::rename(&self.root, &claimed) {
            let error = ToolError::io(self.root.display(), error);
            return match std::fs::remove_dir(&private) {
                Ok(()) => Err(error),
                Err(cleanup) => Err(error.with_remediation(format!(
                    "The destination was not claimed. Inspect the private cleanup folder at {} before retrying: {cleanup}",
                    private.display()
                ))),
            };
        }
        // The public pathname can be replaced after the first check. Validate again after
        // moving its entry into this attempt's private directory.
        #[cfg(not(windows))]
        let checked = self.check_identity_at(&claimed);
        #[cfg(windows)]
        let checked = CreationReservation::check_directory(&claimed, &directory).and_then(|()| CreationReservation::check_manifest_absent(&claimed));
        checked.map_err(|error| {
            ToolError::new(error.code, format!("{} The moved entry is preserved at {}.", error.message, claimed.display()))
                .with_remediation("Inspect the preserved entry and move it to a safe location before retrying. No directory contents were removed.")
        })?;
        #[cfg(not(windows))]
        self.creation.release_empty(&claimed)?;
        // mkdir cannot return a directory handle. A replacement before the first open may
        // have been captured, so that identity never authorizes recursively deleting files.
        std::fs::remove_dir(&claimed).map_err(|error| {
            ToolError::io(claimed.display(), error)
                .with_remediation(format!("Cleanup stopped. Inspect the remaining contents at {} before retrying.", claimed.display()))
        })?;
        std::fs::remove_dir(&private).map_err(|error| ToolError::io(private.display(), error))
    }

    fn cleanup_directory(parent: &Path) -> Result<PathBuf, ToolError> {
        use std::hash::BuildHasher as _;
        for _ in 0..16 {
            let nonce = std::collections::hash_map::RandomState::new().hash_one((std::process::id(), now_ms(), next_counter()));
            let path = parent.join(format!("{CLEANUP_PREFIX}{nonce:016x}"));
            #[cfg(unix)]
            let builder = {
                use std::os::unix::fs::DirBuilderExt as _;
                let mut builder = std::fs::DirBuilder::new();
                builder.mode(0o700);
                builder
            };
            #[cfg(not(unix))]
            let builder = std::fs::DirBuilder::new();
            match builder.create(&path) {
                Ok(()) => return Ok(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(ToolError::io(path.display(), error)),
            }
        }
        Err(ToolError::new(ErrorCode::FileError, "A private folder for cleaning up the unfinished copy could not be created.")
            .with_remediation("The copy destination was not moved. Inspect it before retrying; the original project is unchanged."))
    }
}

/// Every copy normally holds the publisher lock. A caller denied write access can hold an
/// existing lock shared through a read-only handle. A source without a lock can instead be
/// read and rechecked before publication, retaining any existing inbox's directory identity.
enum CopySource {
    Locked(crate::feed::ProjectFeedGuard),
    ReadOnly(crate::feed::ProjectCopyGuard),
    WithoutCoordination { directory: FileHandle, inbox: Option<FileHandle> },
}

impl CopySource {
    fn check(&self, ws: &Workspace) -> Result<(), ToolError> {
        match self {
            Self::Locked(guard) => guard.check_workspace(ws),
            Self::ReadOnly(guard) => guard.check_workspace(ws),
            Self::WithoutCoordination { directory, inbox } => {
                CreationReservation::check_directory(&ws.root, directory)?;
                let manifest = Workspace::open(&ws.root)?.manifest;
                if manifest.generation != ws.manifest.generation || manifest.created_ms != ws.manifest.created_ms {
                    return Err(copy_changed(&ws.root));
                }
                let path = match inbox {
                    Some(inbox) => {
                        CreationReservation::check_directory(&ws.inbox_dir(), inbox)?;
                        ws.inbox_dir().join(crate::feed::PROJECT_LOCK_FILE)
                    }
                    None => ws.inbox_dir(),
                };
                match std::fs::symlink_metadata(&path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Ok(_) => Err(copy_changed(&path)),
                    Err(error) => Err(ToolError::io(path.display(), error)),
                }
            }
        }
    }
}

impl Workspace {
    /// Create a new workspace folder under `projects_dir` (a fresh, unique folder).
    pub fn create(projects_dir: &Path, name: &str, request: &str, target: TargetRef) -> Result<Workspace, ToolError> {
        Self::create_with_publisher(projects_dir, name, request, target, Self::publish_manifest)
    }

    fn create_with_publisher(
        projects_dir: &Path,
        name: &str,
        request: &str,
        target: TargetRef,
        publish: impl FnOnce(&mut Self, &CreationReservation) -> Result<(), ToolError>,
    ) -> Result<Workspace, ToolError> {
        let manifest = Manifest::new(name, request, target)?;
        let (root, directory) = create_workspace_root(projects_dir, name)?;
        let creation = CreationReservation::claim(&root, directory)?;
        let mut ws = Workspace { root, manifest, creation_warning: None };
        ws.publish_with_cleanup(&creation, publish, &CopyDestination::cleanup_directory)?;
        Ok(ws)
    }

    /// Rename the project (its name, not its folder). The name is the person's: a run saving the
    /// project meanwhile keeps it.
    pub fn rename(&mut self, name: &str) -> Result<(), ToolError> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(ToolError::invalid("A project needs a name."));
        }
        self.set_person(|m| m.name = name)
    }

    /// A copy of the project beside it, named `name`, to try something else from where this one
    /// is: what was built, the pictures of the original and the study, not the conversation,
    /// questions or usage. An app edited in place is copied into the copy's app/. What a build
    /// makes again (dependencies, compiler output) is left out.
    pub fn duplicate(&self, name: &str) -> Result<Workspace, ToolError> {
        self.duplicate_after_discovery(name, |_| {})
    }

    fn duplicate_after_discovery(&self, name: &str, after_discovery: impl FnOnce(&[PathBuf])) -> Result<Workspace, ToolError> {
        self.duplicate_with_hooks(name, after_discovery, |_| {})
    }

    fn duplicate_with_hooks(&self, name: &str, after_discovery: impl FnOnce(&[PathBuf]), before_cleanup: impl FnOnce(&Path)) -> Result<Workspace, ToolError> {
        // This guard is not reentrant; no source feed writer runs inside the copy.
        let source = self.lock_copy_source()?;
        self.duplicate_from_source(name, source, after_discovery, before_cleanup)
    }

    fn duplicate_from_source(
        &self,
        name: &str,
        source: CopySource,
        after_discovery: impl FnOnce(&[PathBuf]),
        before_cleanup: impl FnOnce(&Path),
    ) -> Result<Workspace, ToolError> {
        self.duplicate_with_publisher(name, source, after_discovery, before_cleanup, Self::publish_manifest)
    }

    fn duplicate_with_publisher(
        &self,
        name: &str,
        source: CopySource,
        after_discovery: impl FnOnce(&[PathBuf]),
        before_cleanup: impl FnOnce(&Path),
        publish: impl FnOnce(&mut Self, &CreationReservation) -> Result<(), ToolError>,
    ) -> Result<Workspace, ToolError> {
        let parent = self.root.parent().ok_or_else(|| ToolError::invalid(format!("{} has no parent folder to copy it into.", self.root.display())))?;
        let m = &self.manifest;
        let in_place = m.app_in_root;
        let app = self.app_dir();
        // Recovery must publish its backup and receipt together before discovery starts.
        source.check(self)?;
        if in_place {
            check_pending_cleanups(&self.root, None)?;
        }
        // Fail a discovery read before creating a project that would appear as an empty copy.
        // This is only a preflight: each backup-shaped file is classified again when opened.
        let backups = if in_place { Some(CopyBackups::discover(&self.root)?) } else { None };
        let generated_backups = backups.as_ref().map(|backups| backups.owned.as_slice()).unwrap_or(&[]);
        after_discovery(generated_backups);
        let manifest = Manifest::new(name, &m.request, m.target.clone())?;
        let destination = CopyDestination::create(parent, name)?;
        let mut copy = Workspace { root: destination.root.clone(), manifest, creation_warning: None };
        let copied = (|| {
            if app.is_dir() {
                copy_tree(
                    &app,
                    &copy.app_dir(),
                    &|p| {
                        let backup_name = crate::feed::DAMAGED_BACKUP_NAMES.iter().any(|name| inside(&self.root.join(name), p));
                        rebuilt(p) || (in_place && self.is_own_record(p) && !backup_name)
                    },
                    0,
                    backups.as_ref(),
                )?;
            }
            for dir in ["reference", "capture"] {
                let from = self.root.join(dir);
                if from.is_dir() {
                    copy_tree(&from, &copy.root.join(dir), &rebuilt, 0, None)?;
                }
            }
            copy.manifest.stack.clone_from(&m.stack);
            copy.manifest.checks.clone_from(&m.checks);
            copy.manifest.last_reply.clone_from(&m.last_reply);
            // Ready stays ready; anything else can carry on (with a new conversation).
            copy.manifest.status = if m.status == Status::Ready { Status::Ready } else { Status::Stopped };
            copy.manifest.asset_budget_usd = m.asset_budget_usd;
            copy.manifest.generate_assets = m.generate_assets;
            copy.manifest.builder.clone_from(&m.builder);
            copy.manifest.allowed_actions.clone_from(&m.allowed_actions);
            copy.manifest.questions = m.questions;
            copy.manifest.approvals.clone_from(&m.approvals);
            source.check(self)?;
            if in_place {
                check_pending_cleanups(&self.root, None)?;
            }
            destination.check_identity()?;
            // The manifest makes the project discoverable, so it is written only after
            // every copied file and all initial settings are complete.
            publish(&mut copy, &destination.creation)
        })();
        if let Err(error) = copied {
            before_cleanup(&destination.root);
            let destination_root = destination.root.clone();
            return match destination.remove() {
                Ok(()) => Err(error),
                Err(cleanup) => {
                    let guidance = format!(
                        "The unfinished copy could not be removed safely; its files were left alone. {} Inspect the reported retained path, or {} if no retained path is reported, before retrying. The original project is unchanged.",
                        cleanup.for_people(),
                        destination_root.display()
                    );
                    let guidance = match &error.remediation {
                        Some(previous) => format!("{previous} {guidance}"),
                        None => guidance,
                    };
                    Err(error.with_remediation(guidance))
                }
            };
        }
        Ok(copy)
    }

    fn lock_copy_source(&self) -> Result<CopySource, ToolError> {
        self.copy_source_after_lock(crate::feed::lock_project(&self.root))
    }

    fn copy_source_after_lock(&self, locked: Result<crate::feed::ProjectFeedGuard, ToolError>) -> Result<CopySource, ToolError> {
        match locked {
            Ok(guard) => Ok(CopySource::Locked(guard)),
            Err(error) if error.code == ErrorCode::NoPermission => {
                let inbox = self.inbox_dir();
                let inbox = match std::fs::symlink_metadata(&inbox) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                        let identity = FileHandle::from_path(&inbox).map_err(|error| ToolError::io(inbox.display(), error))?;
                        CreationReservation::check_directory(&inbox, &identity)?;
                        let path = inbox.join(crate::feed::PROJECT_LOCK_FILE);
                        match std::fs::symlink_metadata(&path) {
                            Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => Some(identity),
                            Err(error) => return Err(ToolError::io(path.display(), error)),
                            Ok(_) => return crate::feed::lock_project_for_copy(&self.root).map(CopySource::ReadOnly),
                        }
                    }
                    Ok(_) => return Err(copy_changed(&inbox)),
                    Err(error) => return Err(ToolError::io(inbox.display(), error)),
                };
                // Mode bits cannot determine this caller's access (for example, another
                // user's 0755 directory). The failed operation is the permission evidence.
                let directory = FileHandle::from_path(&self.root).map_err(|error| ToolError::io(self.root.display(), error))?;
                let source = CopySource::WithoutCoordination { directory, inbox };
                source.check(self)?;
                Ok(source)
            }
            Err(error) => Err(error),
        }
    }

    /// Delete the project: everything Craft Anything made in its folder (its records, checks and
    /// what was built in app/). The person's own files stay: an app edited in place, and the
    /// files a project started from. A folder left empty is removed. This can't be undone.
    pub fn delete(self) -> Result<(), ToolError> {
        self.delete_after_discovery(|_| {})
    }

    fn delete_after_discovery(self, after_discovery: impl FnOnce(&[PathBuf])) -> Result<(), ToolError> {
        self.delete_with_remover(after_discovery, |p| {
            let r = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
            match r {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(ToolError::io(p.display(), e)),
                _ => Ok(()),
            }
        })
    }

    fn delete_with_remover(self, after_discovery: impl FnOnce(&[PathBuf]), mut gone: impl FnMut(&Path) -> Result<(), ToolError>) -> Result<(), ToolError> {
        self.delete_with_hooks(after_discovery, &mut gone, || {})
    }

    fn delete_with_hooks(
        self,
        after_discovery: impl FnOnce(&[PathBuf]),
        gone: impl FnMut(&Path) -> Result<(), ToolError>,
        after_claim: impl FnOnce(),
    ) -> Result<(), ToolError> {
        self.delete_with_cleanup(after_discovery, gone, after_claim, &CopyDestination::cleanup_directory)
    }

    fn delete_with_cleanup(
        self,
        after_discovery: impl FnOnce(&[PathBuf]),
        mut gone: impl FnMut(&Path) -> Result<(), ToolError>,
        after_claim: impl FnOnce(),
        allocate: &dyn Fn(&Path) -> Result<PathBuf, ToolError>,
    ) -> Result<(), ToolError> {
        if !self.root.join(MANIFEST).is_file() {
            return Err(ToolError::invalid(format!("{} isn't a project folder.", self.root.display())));
        }
        let feed = crate::feed::lock_project(&self.root)?;
        // The caller may still hold a workspace loaded before this folder was reused.
        // Acquiring the replacement project's lock must not authorize deleting its data.
        feed.check_workspace(&self)?;
        // A legacy project can have personal files at a name reserved by a later version.
        // Read the receipts before removing the inbox, and never recursively remove backups.
        let backups = crate::feed::owned_backups(&self.root)?;
        after_discovery(&backups);
        let inbox = self.inbox_dir();
        let inbox_identity = FileHandle::from_path(&inbox).map_err(|error| ToolError::io(inbox.display(), error))?;
        // Reserve before removing any project data: allocation failure must leave the
        // build and conversation intact, not just retain a manifest for an emptied project.
        let cleanup = CreationCleanup::reserve(&self.root, allocate).map_err(|error| {
            let guidance = format!("The project manifest was kept. Inspect {} and restore cleanup access or space before retrying deletion.", inbox.display());
            let guidance = match &error.remediation {
                Some(previous) => format!("{previous} {guidance}"),
                None => guidance,
            };
            error.with_remediation(guidance)
        })?;
        let removed = (|| {
            crate::feed::remove_owned_backups(&self.root, &backups)?;
            if !self.manifest.app_in_root {
                gone(&self.app_dir())?;
            }
            // Keep the coordination inode until craft.json is gone. Waiting writers then
            // see an unavailable project before they can start another transaction.
            for name in OWN_NAMES {
                if *name == MANIFEST || *name == "inbox" || crate::feed::DAMAGED_BACKUP_NAMES.contains(name) {
                    continue;
                }
                gone(&self.root.join(name))?;
            }
            // Fallible payload cleanup keeps the project discoverable and its lock inode
            // intact. Only the private inbox claim follows manifest removal.
            for entry in std::fs::read_dir(&inbox).map_err(|error| ToolError::io(inbox.display(), error))? {
                let entry = entry.map_err(|error| ToolError::io(inbox.display(), error))?;
                if !feed.is_lock_entry(&entry.path())? {
                    gone(&entry.path())?;
                }
            }
            feed.check_workspace(&self)?;
            CreationReservation::check_directory(&inbox, &inbox_identity)?;
            cleanup.empty_path(&self.root)?;
            let lock = inbox.join(crate::feed::PROJECT_LOCK_FILE);
            if !feed.is_lock_entry(&lock)? {
                return Err(copy_changed(&lock));
            }
            gone(&self.root.join(MANIFEST))
        })();
        if let Err(error) = removed {
            let released = feed.check_workspace(&self).and_then(|()| cleanup.remove_empty(&self.root));
            let guidance = match released {
                Ok(()) => format!("Inspect {} and {} before retrying deletion.", self.root.join(MANIFEST).display(), inbox.display()),
                Err(cleanup_error) => format!(
                    "Inspect {} and {}, together with the reserved cleanup folder at {}: {}",
                    self.root.join(MANIFEST).display(),
                    inbox.display(),
                    self.root.join(&cleanup.name).display(),
                    cleanup_error.for_people()
                ),
            };
            let guidance = match &error.remediation {
                Some(previous) => format!("{previous} {guidance}"),
                None => guidance,
            };
            return Err(error.with_remediation(guidance));
        }
        feed.finish_deletion(&inbox_identity, &cleanup, after_claim)?;
        if std::fs::read_dir(&self.root).is_ok_and(|mut d| d.next().is_none()) {
            let _ = std::fs::remove_dir(&self.root);
        }
        Ok(())
    }

    /// Start a project in `folder` itself, a folder the person picked. A missing or empty folder
    /// is used as it is. One that has their files needs `existing` to say what those files are:
    /// the app, edited in place, or material to start from, with the app built in app/ beside
    /// them. A folder that's already a project, or that has something named like a file Craft
    /// Anything keeps there, is refused, so nothing of theirs is ever replaced. A selected
    /// directory link is resolved once; the returned workspace keeps that real directory path.
    pub fn create_at(folder: &Path, name: &str, request: &str, target: TargetRef, existing: Option<ExistingFiles>) -> Result<Workspace, ToolError> {
        Self::create_at_with_publisher(folder, name, request, target, existing, Self::publish_manifest)
    }

    fn create_at_with_publisher(
        folder: &Path,
        name: &str,
        request: &str,
        target: TargetRef,
        existing: Option<ExistingFiles>,
        publish: impl FnOnce(&mut Self, &CreationReservation) -> Result<(), ToolError>,
    ) -> Result<Workspace, ToolError> {
        Self::create_at_with_cleanup(folder, name, request, target, existing, publish, &CopyDestination::cleanup_directory)
    }

    fn create_at_with_cleanup(
        folder: &Path,
        name: &str,
        request: &str,
        target: TargetRef,
        existing: Option<ExistingFiles>,
        publish: impl FnOnce(&mut Self, &CreationReservation) -> Result<(), ToolError>,
        allocate: &dyn Fn(&Path) -> Result<PathBuf, ToolError>,
    ) -> Result<Workspace, ToolError> {
        // A person may select a directory link. Resolve that choice before inspecting its
        // contents and reserve the real directory; a later link change must not redirect us.
        let selected = match std::fs::symlink_metadata(folder) {
            Ok(metadata) if metadata.file_type().is_symlink() => Some(std::fs::canonicalize(folder).map_err(|error| ToolError::io(folder.display(), error))?),
            Ok(_) => None,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(ToolError::io(folder.display(), error)),
        };
        let folder = selected.as_deref().unwrap_or(folder);
        let mut manifest = Manifest::new(name, request, target)?;
        match folder_state(folder)? {
            FolderState::Empty => std::fs::create_dir_all(folder).map_err(|e| ToolError::io(folder.display(), e))?,
            FolderState::Project => {
                return Err(ToolError::invalid(format!("{} is already a project.", folder.display()))
                    .with_remediation("Open it to keep working on it, or pick another folder for a new one."));
            }
            FolderState::HasFiles { names, .. } => {
                let Some(existing) = existing else {
                    return Err(ToolError::invalid(format!("{} already has files.", folder.display())).with_remediation(
                        "Say what they are: the app itself, to edit in place (existing: edit), or material to start from, with the app built in app/ beside them (existing: start_from).",
                    ));
                };
                let clash = checked_clashes(folder, existing)?;
                if !clash.is_empty() {
                    return Err(ToolError::invalid(format!(
                        "{} already has {}, which Craft Anything would keep its own files in.",
                        folder.display(),
                        clash.join(", ")
                    ))
                    .with_remediation("Pick another folder, or move those out of it first; Craft Anything never replaces your files."));
                }
                manifest.app_in_root = existing == ExistingFiles::Edit;
                manifest.start_files = names;
            }
        }
        check_pending_cleanups(folder, None)?;
        let directory = FileHandle::from_path(folder).map_err(|error| ToolError::io(folder.display(), error))?;
        let creation = CreationReservation::claim_with_cleanup(folder, directory, allocate)?;
        let mut ws = Workspace { root: folder.to_path_buf(), manifest, creation_warning: None };
        ws.publish_with_cleanup(&creation, publish, allocate)?;
        Ok(ws)
    }

    pub fn open(root: &Path) -> Result<Workspace, ToolError> {
        let path = root.join(MANIFEST);
        let bytes = read_file_capped(&path, u64::MAX)?;
        let manifest = read_manifest(&bytes)
            .map_err(|e| if e.code == ErrorCode::NotSupported { e } else { ToolError::invalid(format!("{} is unreadable: {}", path.display(), e.message)) })?;
        Ok(Workspace { root: root.to_path_buf(), manifest, creation_warning: None })
    }

    /// Made by a newer Craft Anything: it can be looked at but not changed (saving is refused, so
    /// fields this build doesn't know are never lost).
    pub fn is_read_only(&self) -> bool {
        self.manifest.version > MANIFEST_VERSION
    }

    /// Refuse all writes before a tool changes any records belonging to a newer project.
    pub(crate) fn ensure_writable(&self) -> Result<(), ToolError> {
        if self.is_read_only() {
            return Err(ToolError::unsupported(format!(
                "{} was made by a newer Craft Anything (craft.json version {}), so this one doesn't change it.",
                self.manifest.name, self.manifest.version
            ))
            .with_remediation("Update Craft Anything to keep working on this project. Nothing was written."));
        }
        Ok(())
    }

    /// Re-read the manifest from disk (another process may have changed it).
    pub fn reload(&mut self) -> Result<(), ToolError> {
        *self = Workspace::open(&self.root)?;
        Ok(())
    }

    /// Save the manifest. The fields the person owns (see [`Workspace::set_person`]) are kept as
    /// they are on disk, so a writer holding an older copy (a run that started before the person
    /// raised the budget) can't undo their change. This acquires the non-reentrant project lock.
    pub fn save(&self) -> Result<(), ToolError> {
        self.write_manifest(|disk| {
            let mut manifest = self.manifest.clone();
            manifest.take_person_fields(disk);
            manifest
        })
        .map(|_| ())
    }

    /// Change the fields the person owns (the asset budget, whether assets are generated, the
    /// actions always allowed): start from what's on disk, apply `f` and save. This is the only
    /// way they change.
    pub fn set_person(&mut self, f: impl FnOnce(&mut Manifest)) -> Result<(), ToolError> {
        self.manifest = self.write_manifest(|disk| {
            let mut manifest = disk.clone();
            f(&mut manifest);
            manifest
        })?;
        Ok(())
    }

    fn publish_with_cleanup(
        &mut self,
        creation: &CreationReservation,
        publish: impl FnOnce(&mut Self, &CreationReservation) -> Result<(), ToolError>,
        allocate: &dyn Fn(&Path) -> Result<PathBuf, ToolError>,
    ) -> Result<(), ToolError> {
        publish(self, creation).map_err(|error| match creation.release_empty_with_cleanup(&self.root, || {}, allocate) {
            Ok(()) => error,
            Err(cleanup) => {
                let guidance = format!(
                    "The creation reservation could not be fully released: {}. Inspect {} and {} together with any retained path reported above before retrying.",
                    cleanup.for_people(),
                    self.root.join(MANIFEST).display(),
                    self.inbox_dir().display()
                );
                let guidance = match &error.remediation {
                    Some(previous) => format!("{previous} {guidance}"),
                    None => guidance,
                };
                error.with_remediation(guidance)
            }
        })
    }

    /// Only the creator holding the folder reservation publishes its initial identity.
    /// An ordinary save must never recreate a deleted manifest through this path.
    fn publish_manifest(&mut self, creation: &CreationReservation) -> Result<(), ToolError> {
        let root = self.root.clone();
        self.publish_manifest_using(creation, write_atomic, CreationCleanup::remove_empty, |warning| {
            log::warn!("Project published at {}: {}", root.display(), warning.for_people());
        })
    }

    fn publish_manifest_using(
        &mut self,
        creation: &CreationReservation,
        write: impl FnOnce(&Path, &[u8]) -> Result<(), ToolError>,
        remove: impl FnOnce(&CreationCleanup, &Path) -> Result<(), ToolError>,
        report: impl FnOnce(&ToolError),
    ) -> Result<(), ToolError> {
        self.ensure_writable()?;
        creation.check(&self.root)?;
        check_pending_cleanups(&self.root, creation.cleanup.as_ref())?;
        let bytes = serde_json::to_vec_pretty(&self.manifest).map_err(|e| ToolError::internal(e.to_string()))?;
        write(&self.root.join(MANIFEST), &bytes)?;
        if let Some(warning) = creation.finish_publication(self, remove)? {
            report(&warning);
            self.creation_warning = Some(warning.for_people());
        }
        Ok(())
    }

    fn write_manifest(&self, change: impl FnOnce(&Manifest) -> Manifest) -> Result<Manifest, ToolError> {
        self.ensure_writable()?;
        let guard = crate::feed::lock_project(&self.root)?;
        // A pathname can now name a replacement project. Do not adopt its identity or
        // merge any fields until the workspace loaded by this caller has been checked.
        guard.check_workspace(self)?;
        let disk = Workspace::open(&self.root)?;
        disk.ensure_writable()?;
        let mut m = change(&disk.manifest);
        // Creation owns these fields. The checked disk record is the same project;
        // neither a cached save nor a person-setting closure changes its identity.
        m.generation = disk.manifest.generation;
        m.created_ms = disk.manifest.created_ms;
        let bytes = serde_json::to_vec_pretty(&m).map_err(|e| ToolError::internal(e.to_string()))?;
        // Keep the guard through the atomic replacement, excluding project deletion.
        write_atomic(&self.root.join(MANIFEST), &bytes)?;
        Ok(m)
    }

    /// Set the status and note, and save.
    pub fn set_status(&mut self, status: Status, note: impl Into<String>) -> Result<(), ToolError> {
        self.manifest.status = status;
        self.manifest.note = note.into();
        self.manifest.updated_ms = now_ms();
        self.save()
    }

    pub fn investigation_dir(&self) -> PathBuf {
        self.root.join("investigation.cany")
    }
    pub fn capture_dir(&self) -> PathBuf {
        self.root.join("capture")
    }
    /// The app: app/, or the project folder itself when the person's files are edited in place.
    pub fn app_dir(&self) -> PathBuf {
        if self.manifest.app_in_root { self.root.clone() } else { self.root.join("app") }
    }
    /// Where the app is, in words for messages: "app/", or "the project folder" (#174).
    pub fn app_place(&self) -> &'static str {
        if self.manifest.app_in_root { "the project folder" } else { "app/" }
    }

    /// One of Craft Anything's own records in this project (the manifest, the feed, the inbox,
    /// the person's pictures…), which the builder never changes, even when the app is the
    /// project folder itself. checks/ isn't one: the builder keeps its notes there.
    pub fn is_own_record(&self, p: &Path) -> bool {
        let own = |root: &Path, p: &Path| OWN_NAMES.iter().filter(|n| **n != "checks").any(|n| inside(&root.join(n), p));
        // Also where a link would really take it.
        own(&self.root, p) || matches!((real_path(&self.root), real_path(p)), (Some(root), Some(p)) if own(&root, &p))
    }
    /// Separate from capture/, which a website study replaces.
    pub fn reference_dir(&self) -> PathBuf {
        self.root.join("reference")
    }

    /// The person's pictures of the original, in the order they were added.
    pub fn references(&self) -> Vec<PathBuf> {
        pictures_in(&self.reference_dir())
    }

    /// Copy pictures of the original into reference/ as picture-N.ext. Returns how many were
    /// added; files that aren't PNG, JPEG, WebP or GIF are skipped, and a picture over 20 MB
    /// refuses the whole call before anything is copied.
    pub fn add_references(&self, files: &[PathBuf]) -> Result<usize, ToolError> {
        copy_pictures(&self.reference_dir(), files).map(|v| v.len())
    }

    /// Every generated asset so far, oldest first.
    pub fn assets(&self) -> Vec<AssetRecord> {
        std::fs::read(self.root.join(ASSETS)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// What the builds have taken and used so far, or that it can't be known.
    pub fn usage(&self) -> UsageRead {
        ProjectUsage::read(&self.root)
    }

    /// What generated assets have cost so far.
    pub fn assets_spent(&self) -> f64 {
        // A fold from 0.0: an empty `sum()` of floats is -0.0, which prints as "$-0.00".
        self.assets().iter().fold(0.0, |spent, a| spent + a.cost_usd)
    }

    /// Add a generated asset to the list.
    pub fn record_asset(&self, rec: AssetRecord) -> Result<(), ToolError> {
        let mut all = self.assets();
        all.push(rec);
        let json = serde_json::to_vec_pretty(&all).map_err(|e| ToolError::internal(e.to_string()))?;
        write_atomic(&self.root.join(ASSETS), &json)
    }

    /// Pictures attached to messages (what the person wants changed), kept apart from the
    /// pictures of the original.
    pub fn attachments_dir(&self) -> PathBuf {
        self.root.join("attachments")
    }

    /// Copy a message's pictures into attachments/. Returns where each one went.
    pub fn add_attachments(&self, files: &[PathBuf]) -> Result<Vec<PathBuf>, ToolError> {
        copy_pictures(&self.attachments_dir(), files)
    }
    pub fn checks_dir(&self) -> PathBuf {
        self.root.join("checks")
    }
    /// The builder's sheet of how the rebuild plays (`crate::mechanics`).
    pub fn mechanics_path(&self) -> PathBuf {
        self.app_dir().join(crate::mechanics::MECHANICS)
    }
    pub fn inbox_dir(&self) -> PathBuf {
        self.root.join("inbox")
    }
    /// A coding agent's session saved for this project in `deep` mode. A session saved before the
    /// mode was kept counts as Deep, since it may have read inside the original.
    pub fn builder_session_in(&self, deep: bool) -> bool {
        self.manifest.agent_session.as_ref().is_some_and(|_| self.manifest.agent_session_deep.unwrap_or(true) == deep)
    }

    /// The API builder's conversation, kept so a follow-up continues it (`ai::context`).
    pub fn transcript_path(&self) -> PathBuf {
        self.root.join("transcript.json")
    }

    /// A copy of the coding agent's own session (Claude Code's), kept after every turn so the
    /// conversation travels with the project folder to another computer.
    pub fn agent_session_path(&self) -> PathBuf {
        self.root.join(AGENT_SESSION_FILE)
    }

    /// Every workspace under `projects_dir`, most recently updated first. Folders without a
    /// readable manifest are skipped.
    pub fn list(projects_dir: &Path) -> Vec<Workspace> {
        Workspace::list_in(&[projects_dir.to_path_buf()])
    }

    /// Every workspace in `locations`, most recently updated first, each once. A location is a
    /// folder of projects or a project itself (one started in a folder the person picked).
    pub fn list_in(locations: &[PathBuf]) -> Vec<Workspace> {
        let mut roots: Vec<PathBuf> = Vec::new();
        for loc in locations {
            if loc.join(MANIFEST).is_file() {
                roots.push(loc.clone());
            } else if let Ok(rd) = std::fs::read_dir(loc) {
                roots.extend(rd.flatten().map(|e| e.path()).filter(|p| p.join(MANIFEST).is_file()));
            }
        }
        // The same project reached twice (a project listed, and its parent folder listed too).
        let mut seen = std::collections::HashSet::new();
        roots.retain(|r| seen.insert(std::fs::canonicalize(r).unwrap_or_else(|_| r.clone())));
        let mut v: Vec<Workspace> = roots.iter().filter_map(|r| Workspace::open(r).ok()).collect();
        v.sort_by_key(|w| std::cmp::Reverse(w.manifest.updated_ms));
        v
    }
}

/// The coding agent's session, copied into the project ([`Workspace::agent_session_path`]).
pub const AGENT_SESSION_FILE: &str = "agent-session.jsonl";

/// The names Craft Anything keeps in a project folder beside app/. A folder the person picks
/// mustn't already have any of them.
pub const OWN_NAMES: &[&str] = &{
    const RECORDS: &[&str] = &[
        MANIFEST,
        GHIDRA_STOPPED,
        GHIDRA_STOP_LOCK,
        crate::feed::FEED_FILE,
        crate::questions::QUESTIONS_FILE,
        USAGE_FILE,
        ACTIVITY_LOG,
        ACTIVITY_LOG_OLD,
        ASSETS,
        "transcript.json",
        "extract-transcript.json",
        AGENT_SESSION_FILE,
        "inbox",
        "checks",
        "capture",
        "reference",
        "attachments",
        "investigation.cany",
        crate::pins::PINNED,
    ];
    const BACKUPS: &[&str] = crate::feed::DAMAGED_BACKUP_NAMES;
    // Keep the public slice and every ownership consumer on the writer's exact name set.
    let mut names = [""; RECORDS.len() + BACKUPS.len()];
    let mut index = 0;
    while index < RECORDS.len() {
        names[index] = RECORDS[index];
        index += 1;
    }
    let mut index = 0;
    while index < BACKUPS.len() {
        names[RECORDS.len() + index] = BACKUPS[index];
        index += 1;
    }
    names
};

/// `p` is `dir` or inside it (case-insensitive on Windows, where paths are).
pub fn inside(dir: &Path, p: &Path) -> bool {
    if cfg!(windows) {
        let (d, p) = (dir.to_string_lossy().to_lowercase().replace('\\', "/"), p.to_string_lossy().to_lowercase().replace('\\', "/"));
        p == d || p.starts_with(&format!("{}/", d.trim_end_matches('/')))
    } else {
        p.starts_with(dir)
    }
}

/// Where `p` really is: its deepest part that exists, with every link in it followed, then the
/// rest as written (parts that don't exist yet can't be links). `None` when an existing part
/// can't be followed, such as a link to nothing, since writing there would follow it.
pub fn real_path(p: &Path) -> Option<PathBuf> {
    let mut existing = p;
    let mut missing = Vec::new();
    while existing.symlink_metadata().is_err() {
        missing.push(existing.file_name()?);
        existing = existing.parent()?;
    }
    let mut real = std::fs::canonicalize(existing).ok()?;
    real.extend(missing.iter().rev());
    Some(real)
}

/// [`inside`], after following links: `p` is inside `dir` where the file system really puts
/// them both, so a link inside `dir` can't lead out of it. Only the real paths are compared: `p`
/// may already be partly resolved (`..` after a link) while `dir` is as written, and a `dir`
/// that sits behind a link (macOS's `/var` → `/private/var`) is still the same folder.
pub fn really_inside(dir: &Path, p: &Path) -> bool {
    matches!((real_path(dir), real_path(p)), (Some(d), Some(p)) if inside(&d, &p))
}

/// The other side of [`really_inside`], for places that must be kept out of: `p` may land in
/// `dir` as written, once links are followed (also links in `dir`'s own path, such as macOS's
/// `/etc` → `/private/etc`), or because a link in `p` can't be followed to tell.
pub fn may_reach(dir: &Path, p: &Path) -> bool {
    inside(dir, p)
        || match real_path(p) {
            Some(real) => inside(dir, &real) || real_path(dir).is_some_and(|d| inside(&d, &real)),
            None => true,
        }
}

/// How many of a picked folder's names are kept for the builder.
pub const START_FILES_SHOWN: usize = 40;

/// What a folder picked for a new project already holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum FolderState {
    /// Missing or empty: the project starts there as usual.
    Empty,
    /// Already a Craft Anything project.
    Project,
    /// The person's files: the first names (sorted, at most [`START_FILES_SHOWN`]) and how many.
    HasFiles { names: Vec<String>, count: usize },
}

/// What the builder's questions (`core_person_ask`) get.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Questions {
    /// Shown to the person as a card; the builder waits for the answer.
    Ask,
    /// Answered at once with "decide yourself and say what you chose".
    Decide,
}

/// A kind of builder action that asks for approval, which a project can decide about ahead of time
/// (`Manifest::approvals`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// Reading websites and searching the web.
    Web,
    /// Commands outside the usual build commands.
    Commands,
    /// Changing files outside the project.
    OutsideFiles,
    /// Pushing to a git server or publishing anywhere (`git push`, `npm publish`, `gh pr create`…).
    Publish,
    /// Deleting anything outside the project or the temp folder.
    DeleteOutside,
    /// Changing the system, its settings or users, system files, or Craft Anything's own records.
    System,
    /// Tools with no rule of their own.
    OtherTools,
}

/// What an action of one [`ActionKind`] gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Approval {
    /// Done without asking.
    Allow,
    /// A card for the person, and the builder waits for the answer.
    Ask,
    /// Not done, without asking: the builder is told to find another way.
    Refuse,
}

/// `Manifest::approvals` in the published schema: only the kinds' names as keys, which a map's
/// own schema leaves open.
fn approvals_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    let names: Vec<&str> = ACTION_KINDS.iter().map(|r| r.name).collect();
    schemars::json_schema!({
        "type": "object",
        "propertyNames": { "enum": names },
        "additionalProperties": generator.subschema_for::<Approval>(),
    })
}

/// One row of [`ACTION_KINDS`].
#[derive(Debug, Clone, Copy)]
pub struct KindRow {
    pub kind: ActionKind,
    /// Its name in tools and slash commands.
    pub name: &'static str,
    /// What it covers, as the builder is told when it's refused ("pushing or publishing anything").
    pub what: &'static str,
    /// What it gets when the project hasn't chosen, with Auto mode on and off.
    pub auto: Approval,
    pub manual: Approval,
}

/// Every kind. Auto mode lets through what can be undone and refuses the rest; without it,
/// everything but reading the web asks.
pub const ACTION_KINDS: &[KindRow] = &[
    KindRow { kind: ActionKind::Web, name: "web", what: "reading the web", auto: Approval::Allow, manual: Approval::Allow },
    KindRow { kind: ActionKind::Commands, name: "commands", what: "commands outside the usual build commands", auto: Approval::Allow, manual: Approval::Ask },
    KindRow { kind: ActionKind::OutsideFiles, name: "outside_files", what: "changing files outside the project", auto: Approval::Allow, manual: Approval::Ask },
    KindRow { kind: ActionKind::Publish, name: "publish", what: "pushing or publishing anything", auto: Approval::Refuse, manual: Approval::Ask },
    KindRow {
        kind: ActionKind::DeleteOutside,
        name: "delete_outside",
        what: "deleting anything outside the project or the temp folder",
        auto: Approval::Refuse,
        manual: Approval::Ask,
    },
    KindRow {
        kind: ActionKind::System,
        name: "system",
        what: "changing the system, its settings or users, system files, or Craft Anything's own records",
        auto: Approval::Refuse,
        manual: Approval::Ask,
    },
    KindRow {
        kind: ActionKind::OtherTools,
        name: "other_tools",
        what: "tools with no approval rule of their own",
        auto: Approval::Refuse,
        manual: Approval::Ask,
    },
];

impl ActionKind {
    fn row(self) -> Option<&'static KindRow> {
        ACTION_KINDS.iter().find(|r| r.kind == self)
    }

    /// Its name in tools and slash commands (`publish`).
    pub fn name(self) -> &'static str {
        self.row().map_or("", |r| r.name)
    }

    /// What it covers, in words for the builder.
    pub fn what(self) -> &'static str {
        self.row().map_or("", |r| r.what)
    }

    /// The kind named `name`, in any case.
    pub fn named(name: &str) -> Option<ActionKind> {
        ACTION_KINDS.iter().find(|r| r.name.eq_ignore_ascii_case(name)).map(|r| r.kind)
    }

    /// What it gets when the project hasn't chosen.
    pub fn default_in(self, auto_mode: bool) -> Approval {
        self.row().map_or(Approval::Ask, |r| if auto_mode { r.auto } else { r.manual })
    }
}

impl Approval {
    /// How much it holds back: allow, then ask, then refuse.
    pub fn strictness(self) -> u8 {
        match self {
            Approval::Allow => 0,
            Approval::Ask => 1,
            Approval::Refuse => 2,
        }
    }
}

impl Manifest {
    /// What an action of `kind` gets in this project.
    pub fn approval(&self, kind: ActionKind, auto_mode: bool) -> Approval {
        self.approvals.get(&kind).copied().unwrap_or_else(|| kind.default_in(auto_mode))
    }
}

/// What the files already in a picked folder are to a new project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExistingFiles {
    /// The app itself: the builder edits them in place.
    Edit,
    /// Material to start from: the builder reads them and builds the app in app/ beside them.
    StartFrom,
}

/// What's already in `folder` under a name Craft Anything would keep its own files in (app/ too
/// when the app goes beside the person's files). An unverifiable entry is also a conflict in
/// the folder chooser; creation uses `checked_clashes` to report the actual I/O error.
pub fn clashes(folder: &Path, existing: ExistingFiles) -> Vec<&'static str> {
    OWN_NAMES
        .iter()
        .copied()
        .chain((existing == ExistingFiles::StartFrom).then_some("app"))
        .filter(|name| !matches!(std::fs::symlink_metadata(folder.join(name)), Err(error) if error.kind() == std::io::ErrorKind::NotFound))
        .collect()
}

fn checked_clashes(folder: &Path, existing: ExistingFiles) -> Result<Vec<&'static str>, ToolError> {
    let mut found = Vec::new();
    for name in OWN_NAMES.iter().copied().chain((existing == ExistingFiles::StartFrom).then_some("app")) {
        let path = folder.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(_) => found.push(name),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(ToolError::io(path.display(), error)),
        }
    }
    Ok(found)
}

/// What `folder` holds, for starting a project there.
pub fn folder_state(folder: &Path) -> Result<FolderState, ToolError> {
    if folder.join(MANIFEST).is_file() {
        return Ok(FolderState::Project);
    }
    let rd = match std::fs::read_dir(folder) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(FolderState::Empty),
        Err(e) => return Err(ToolError::io(folder.display(), e)),
    };
    let mut names: Vec<String> = rd
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()).map_err(|error| ToolError::io(folder.display(), error)))
        .collect::<Result<_, _>>()?;
    if names.is_empty() {
        return Ok(FolderState::Empty);
    }
    names.sort();
    let count = names.len();
    names.truncate(START_FILES_SHOWN);
    Ok(FolderState::HasFiles { names, count })
}

/// Picture types the builder agent can open and look at.
pub fn is_picture(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| ["png", "jpg", "jpeg", "webp", "gif"].contains(&e.to_ascii_lowercase().as_str()))
}

/// The pictures in `dir`, in the order they were added.
fn pictures_in(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_file() && is_picture(p)).collect();
    v.sort_by_key(|p| picture_number(p));
    v
}

/// Copy pictures into `dir` as picture-N.ext, numbering on from what's there. Files that aren't
/// PNG, JPEG, WebP or GIF are skipped; a picture over 20 MB is refused before anything is copied.
fn copy_pictures(dir: &Path, files: &[PathBuf]) -> Result<Vec<PathBuf>, ToolError> {
    // Check the whole list first: a mixed add must not leave the valid pictures half-copied when
    // one is too large.
    for f in files {
        if picture_too_large(f) {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| f.display().to_string());
            return Err(ToolError::new(ErrorCode::OverLimit, format!("{name} can't be attached: a message takes pictures up to 20 MB."))
                .with_remediation("Attach a smaller picture, or save it as a JPEG."));
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| ToolError::io(dir.display(), e))?;
    let mut next = pictures_in(dir).iter().map(|p| picture_number(p)).max().unwrap_or(0);
    let mut added = Vec::new();
    for f in files {
        if !attachable_picture(f) {
            continue;
        }
        next += 1;
        let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
        let to = dir.join(format!("picture-{next}.{ext}"));
        std::fs::copy(f, &to).map_err(|e| ToolError::io(to.display(), e))?;
        added.push(to);
    }
    Ok(added)
}

/// N in `picture-N.ext` (0 for anything else), for ordering and numbering.
fn picture_number(p: &Path) -> u32 {
    p.file_stem().and_then(|s| s.to_str()).and_then(|s| s.strip_prefix("picture-")).and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// The most pixels a pasted picture may have (a 16K screen): more is refused before encoding.
const MAX_PASTED_PIXELS: u64 = 15360 * 8640;

/// The largest picture a message sends ([`copy_pictures`] refuses bigger files).
const MAX_PICTURE_BYTES: u64 = 20 * 1024 * 1024;

/// Whether the file starts the way a PNG, JPEG, GIF or WebP picture does, whatever its name says.
/// Reads its first bytes, so not while drawing.
pub fn picture_content(p: &Path) -> bool {
    use std::io::Read as _;
    let mut head = Vec::with_capacity(12);
    if std::fs::File::open(p).and_then(|f| f.take(12).read_to_end(&mut head)).is_err() {
        return false;
    }
    head.starts_with(b"\x89PNG\r\n\x1a\n")
        || head.starts_with(&[0xFF, 0xD8, 0xFF])
        || head.starts_with(b"GIF87a")
        || head.starts_with(b"GIF89a")
        || (head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WEBP".as_slice()))
}

/// Whether `p` is a picture larger than a message can take. Reads its size, so not while drawing.
pub fn picture_too_large(p: &Path) -> bool {
    is_picture(p) && p.metadata().is_ok_and(|m| m.is_file() && m.len() > MAX_PICTURE_BYTES)
}

/// A picture file a message can take: a picture ([`is_picture`]) of at most 20 MB. Reads its
/// size, so not while drawing.
pub fn attachable_picture(p: &Path) -> bool {
    is_picture(p) && p.metadata().is_ok_and(|m| m.is_file() && m.len() <= MAX_PICTURE_BYTES)
}

/// Where pictures pasted from the clipboard wait until their message is sent: this person's own
/// cache folder (never a temp folder other users can read).
pub fn pasted_dir() -> PathBuf {
    dirs::cache_dir().or_else(|| dirs::home_dir().map(|h| h.join(".cache"))).unwrap_or_else(std::env::temp_dir).join("Craft Anything").join("pasted")
}

/// Save a picture pasted from the clipboard (8-bit RGBA, `width`×`height`) as `pasted-<time>-<n>.png`
/// in `dir`, readable only by this person, where it waits until its message is sent (which
/// copies it into the project). Pastes from earlier days are cleared out first, except those in
/// `keep` (still attached to a draft or a message).
pub fn save_pasted_picture(dir: &Path, width: u32, height: u32, rgba: Vec<u8>, keep: &[PathBuf]) -> Result<PathBuf, ToolError> {
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if pixels == 0 || pixels > MAX_PASTED_PIXELS {
        return Err(ToolError::new(ErrorCode::OverLimit, format!("the pasted picture is {width}×{height}; at most a 16K screen fits")));
    }
    if pixels.checked_mul(4) != Some(rgba.len() as u64) {
        return Err(ToolError::invalid("the pasted picture's pixels don't match its size"));
    }
    // Encoded against the limit, so a picture that won't fit never takes a second copy's memory.
    let image = crate::tools::web_common::Image { width, height, rgba };
    let max = usize::try_from(MAX_PICTURE_BYTES).unwrap_or(usize::MAX);
    let Some(png) = crate::tools::web_common::encode_png_within(&image, max)? else {
        return Err(ToolError::new(ErrorCode::OverLimit, "the pasted picture is over 20 MB as a PNG; a message takes pictures up to 20 MB")
            .with_remediation("Paste a smaller part of the screen, or save it as a JPEG and attach the file."));
    };
    std::fs::create_dir_all(dir).map_err(|e| ToolError::io(dir.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    forget_old_pastes(dir, keep);
    // The counter makes the name unique within this process, so a paste never reuses a name that
    // forget_old_pastes just freed in the same millisecond; create_new still guards two app copies.
    let ms = now_ms();
    let c = next_counter();
    let mut n = 1u32;
    let path = loop {
        let path = if n == 1 { dir.join(format!("pasted-{ms}-{c}.png")) } else { dir.join(format!("pasted-{ms}-{c}-{n}.png")) };
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => break path,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n < 1000 => n = n.saturating_add(1),
            Err(e) => return Err(ToolError::io(path.display(), e)),
        }
    };
    craft_anything_platform::fs::write_atomic_private(&path, &png)?;
    Ok(path)
}

/// Remove pastes over a day old from `dir` (only ours: `pasted-….png`), except those in `keep`.
fn forget_old_pastes(dir: &Path, keep: &[PathBuf]) {
    let day = std::time::Duration::from_secs(24 * 60 * 60);
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        let ours = p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("pasted-") && n.ends_with(".png"));
        let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|age| age > day);
        if ours && old && !keep.contains(&p) {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// The default projects folder: `Documents/Craft Anything`, else `~/Craft Anything`.
pub fn default_projects_dir() -> PathBuf {
    dirs::document_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from(".")).join("Craft Anything")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    fn detach_creation_lock_for_replacement(root: &Path, creation: &CreationReservation, retained: &Path) {
        // Keep the coordination inode held, but move it outside the directory this fixture
        // replaces: Windows does not permit a directory move with an open descendant file.
        let held = creation.coordination.as_ref().unwrap();
        ClaimedInbox::detach_entry(&root.join("inbox").join(crate::feed::PROJECT_LOCK_FILE), retained, held, || {}).unwrap();
        assert_eq!(FileHandle::from_path(retained).unwrap(), *held);
    }

    fn rename_creation_root_for_replacement(root: &Path, displaced: &Path) {
        #[cfg(windows)]
        {
            // Keep every reservation guard alive. Windows allows moving an open directory,
            // but not its parent, so temporarily move its held descendants outside the root.
            let retained = tempfile::tempdir_in(root.parent().unwrap()).unwrap();
            let lock_name = crate::feed::PROJECT_LOCK_FILE;
            let lock = FileHandle::from_path(root.join("inbox").join(lock_name)).unwrap();
            let retained_lock = retained.path().join(lock_name);
            ClaimedInbox::detach_entry(&root.join("inbox").join(lock_name), &retained_lock, &lock, || {}).unwrap();
            let mut directories = vec![(std::ffi::OsString::from("inbox"), FileHandle::from_path(root.join("inbox")).unwrap())];
            for entry in std::fs::read_dir(root).unwrap() {
                let entry = entry.unwrap();
                if entry.file_name().to_string_lossy().starts_with(CLEANUP_PREFIX) {
                    directories.push((entry.file_name(), FileHandle::from_path(entry.path()).unwrap()));
                }
            }
            for (name, _) in &directories {
                std::fs::rename(root.join(name), retained.path().join(name)).unwrap();
            }
            std::fs::rename(root, displaced).unwrap();
            for (name, identity) in &directories {
                let path = displaced.join(name);
                std::fs::rename(retained.path().join(name), &path).unwrap();
                assert_eq!(FileHandle::from_path(&path).unwrap(), *identity);
            }
            // Restore the original lock only after moving the original inbox back. The
            // displaced project retains the same entries and the same locked inode.
            ClaimedInbox::detach_entry(&retained_lock, &displaced.join("inbox").join(lock_name), &lock, || {}).unwrap();
        }
        #[cfg(not(windows))]
        std::fs::rename(root, displaced).unwrap();
    }

    #[test]
    fn a_message_takes_picture_files_up_to_20_mb() {
        let dir = tempfile::tempdir().unwrap();
        let small = dir.path().join("small.png");
        std::fs::write(&small, b"png").unwrap();
        let big = dir.path().join("big.PNG");
        std::fs::File::create(&big).unwrap().set_len(MAX_PICTURE_BYTES + 1).unwrap();
        let exact = dir.path().join("exact.png");
        std::fs::File::create(&exact).unwrap().set_len(MAX_PICTURE_BYTES).unwrap();
        let text = dir.path().join("notes.txt");
        std::fs::write(&text, b"hi").unwrap();
        let folder = dir.path().join("folder.png");
        std::fs::create_dir(&folder).unwrap();
        assert!(attachable_picture(&small));
        for no in [&big, &text, &folder, &dir.path().join("gone.png")] {
            assert!(!attachable_picture(no), "{}", no.display());
        }
        assert!(picture_too_large(&big));
        assert!(attachable_picture(&exact));
        assert!(!picture_too_large(&exact), "exactly 20 MB is allowed");
        assert!(!picture_too_large(&small));
        let attachments = dir.path().join("attachments");
        let err = copy_pictures(&attachments, &[small, big]).unwrap_err();
        assert_eq!(err.code, ErrorCode::OverLimit, "{err:?}");
        assert!(!attachments.join("picture-1.png").exists(), "nothing is copied before the refusal");
    }

    #[test]
    fn an_oversized_picture_is_refused_by_references_and_attachments() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::create(dir.path(), "Game", "Copy it", TargetRef::None).unwrap();
        let small = dir.path().join("small.png");
        std::fs::write(&small, b"png").unwrap();
        let big = dir.path().join("big.png");
        std::fs::File::create(&big).unwrap().set_len(MAX_PICTURE_BYTES + 1).unwrap();

        let err = ws.add_references(&[small, big.clone()]).unwrap_err();
        assert_eq!(err.code, ErrorCode::OverLimit, "{err:?}");
        assert!(ws.references().is_empty(), "nothing is copied before the refusal");

        let err = ws.add_attachments(&[big]).unwrap_err();
        assert_eq!(err.code, ErrorCode::OverLimit, "{err:?}");
        assert!(!ws.attachments_dir().join("picture-1.png").exists());
    }

    #[test]
    fn a_picture_is_told_by_its_first_bytes_not_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let file = |name: &str, bytes: &[u8]| {
            let p = dir.path().join(name);
            std::fs::write(&p, bytes).unwrap();
            p
        };
        for yes in [
            file("a.png", b"\x89PNG\r\n\x1a\n...."),
            file("b.jpg", &[0xFF, 0xD8, 0xFF, 0xE0]),
            file("c.gif", b"GIF89a.."),
            file("d.webp", b"RIFF\x10\0\0\0WEBPVP8 "),
            file("e.png", &[0xFF, 0xD8, 0xFF, 0xDB]),
        ] {
            assert!(picture_content(&yes), "{}", yes.display());
        }
        for no in [file("f.png", b"png"), file("g.webp", b"RIFF\0\0\0\0WAVE"), file("h.png", b""), dir.path().join("gone.png")] {
            assert!(!picture_content(&no), "{}", no.display());
        }
    }

    #[test]
    fn a_pasted_picture_is_saved_as_a_png_and_bad_sizes_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let p = save_pasted_picture(dir.path(), 2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255], &[]).unwrap();
        assert!(is_picture(&p));
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(crate::tools::web_common::png_dimensions(&bytes).unwrap(), (2, 1));
        let second = save_pasted_picture(dir.path(), 2, 1, vec![0; 8], &[]).unwrap();
        assert_ne!(p, second, "two pastes in one millisecond don't overwrite each other");
        assert!(save_pasted_picture(dir.path(), 2, 2, vec![0; 8], &[]).is_err(), "pixels that don't match the size");
        assert!(save_pasted_picture(dir.path(), 0, 0, Vec::new(), &[]).is_err());
        assert!(save_pasted_picture(dir.path(), u32::MAX, u32::MAX, Vec::new(), &[]).is_err());
        assert_eq!(pictures_in(dir.path()).len(), 2, "fresh pastes are kept");
        // A day later, a paste clears out old ones, but not one still attached somewhere.
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 24 * 60 * 60);
        for f in [&p, &second] {
            std::fs::File::options().write(true).open(f).unwrap().set_modified(old).unwrap();
        }
        let third = save_pasted_picture(dir.path(), 2, 1, vec![0; 8], std::slice::from_ref(&second)).unwrap();
        // In the same millisecond as the first, the third takes the name its removal freed.
        assert!((third == p || !p.exists()) && second.exists() && third.exists());
        assert_ne!(std::fs::read(&third).ok(), Some(bytes), "the first paste is gone");
        let mut names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
        names.sort();
        assert_eq!(names.len(), 2, "no temp files left behind: {names:?}");
    }

    #[test]
    fn a_projects_usage_adds_up_across_runs_and_a_newer_file_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(ProjectUsage::read(dir.path()), UsageRead::None, "nothing yet");
        for cost in [Some(1.25), None, Some(f64::NAN), Some(2.0)] {
            ProjectUsage::update(dir.path(), |u| {
                u.work_ms += 1_000;
                u.add_turn(100, 10, cost, true);
            })
            .unwrap();
        }
        let u = ProjectUsage::read(dir.path()).known().unwrap();
        assert_eq!((u.version, u.work_ms, u.tokens(), u.cost_usd, u.priced), (USAGE_VERSION, 4_000, 440, 3.25, true));
        std::fs::write(dir.path().join(USAGE_FILE), r#"{"version": 99, "work_ms": 5}"#).unwrap();
        assert!(ProjectUsage::update(dir.path(), |u| u.work_ms = 0).is_err());
        assert_eq!(ProjectUsage::read(dir.path()).known().unwrap().work_ms, 5, "read as far as it's known, never written over");
    }

    /// An unreadable usage.json is unknown, never zeros, and a run doesn't write over it.
    #[test]
    fn an_unreadable_usage_file_is_unknown_and_left_alone() {
        let unreadable = |root: &Path, why: &str| {
            assert_eq!(ProjectUsage::read(root), UsageRead::Unreadable, "{why}");
            assert!(ProjectUsage::read(root).known().is_none(), "{why}");
            assert!(ProjectUsage::update(root, |u| u.add_turn(1, 1, Some(1.0), false)).is_err(), "{why}");
        };
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(USAGE_FILE);
        for bytes in [&b"{ not json"[..], b"", b"null", b"\xff\xfe"] {
            std::fs::write(&file, bytes).unwrap();
            unreadable(dir.path(), &format!("{bytes:?}"));
            assert_eq!(std::fs::read(&file).unwrap(), bytes, "the bytes are left as they were");
        }

        // A folder where the file should be.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(USAGE_FILE)).unwrap();
        unreadable(dir.path(), "a folder");
        assert!(dir.path().join(USAGE_FILE).is_dir());

        // Too big to be a real one.
        let dir = tempfile::tempdir().unwrap();
        std::fs::File::create(dir.path().join(USAGE_FILE)).unwrap().set_len(MAX_USAGE_BYTES + 1).unwrap();
        unreadable(dir.path(), "too big");

        // A file it may not read.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join(USAGE_FILE);
            std::fs::write(&file, br#"{"version":1,"cost_usd":2.0}"#).unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
            // Root reads it anyway; the case only means something where the permission holds.
            if std::fs::read(&file).is_err() {
                unreadable(dir.path(), "no read permission");
            }
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }

        // A readable one is known, with the fields it has.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(USAGE_FILE), br#"{"version":1,"cost_usd":2.0,"priced":true}"#).unwrap();
        let UsageRead::Known(u) = ProjectUsage::read(dir.path()) else { panic!("a readable file is known") };
        assert_eq!((u.cost_usd, u.priced), (2.0, true));
    }

    /// A build turn and a question can save their cost together: neither may overwrite the other.
    #[test]
    fn concurrent_usage_updates_all_count() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let start = std::sync::Barrier::new(8);
        std::thread::scope(|s| {
            for _ in 0..8 {
                let start = &start;
                s.spawn(move || {
                    start.wait();
                    for _ in 0..25 {
                        ProjectUsage::update(root, |u| u.add_turn(0, 0, Some(0.01), true)).unwrap();
                    }
                });
            }
        });
        let u = ProjectUsage::read(root).known().unwrap();
        assert!((u.cost_usd - 2.0).abs() < 1e-6, "{u:?}");
        assert!(u.priced);
    }

    #[test]
    fn a_run_holding_an_old_copy_keeps_the_persons_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut run = Workspace::create(dir.path(), "game", "A snake game", TargetRef::None).unwrap();
        // While the run works, the person raises the budget, turns assets on and always allows npm.
        let mut person = Workspace::open(&run.root).unwrap();
        person
            .set_person(|m| {
                m.asset_budget_usd = 20.0;
                m.generate_assets = true;
                m.allowed_actions.push("bash:npm".into());
            })
            .unwrap();
        // The run saves its own progress from the copy it started with.
        run.manifest.status = Status::Building;
        run.manifest.note = "Building".into();
        run.save().unwrap();
        let disk = Workspace::open(&run.root).unwrap().manifest;
        assert_eq!((disk.asset_budget_usd, disk.generate_assets), (20.0, true));
        assert_eq!(disk.allowed_actions, ["bash:npm"]);
        assert_eq!((disk.status, disk.note.as_str()), (Status::Building, "Building"), "the run's own fields are saved");
        // A setter on the old copy starts from what's on disk too.
        run.set_person(|m| m.asset_budget_usd = 25.0).unwrap();
        let disk = Workspace::open(&run.root).unwrap().manifest;
        assert_eq!((disk.asset_budget_usd, disk.generate_assets), (25.0, true));
    }

    #[test]
    fn a_stale_person_change_keeps_the_latest_build_progress() {
        for rename in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut person = Workspace::create(directory.path(), "Project", "Build a clock", TargetRef::None).unwrap();
            let mut run = Workspace::open(&person.root).unwrap();
            run.manifest.status = Status::Ready;
            run.manifest.note = "The build passed".into();
            run.manifest.checks.push(CheckResult::new(CheckResult::BUILD, 3, true, "Compiled"));
            run.manifest.agent_session = Some("test-session".into());
            run.manifest.agent_session_by = Some("codex".into());
            run.manifest.agent_session_deep = Some(false);
            run.manifest.last_reply = "The clock is ready".into();
            run.save().unwrap();
            let mut expected = Workspace::open(&person.root).unwrap().manifest;
            if rename {
                person.rename("Renamed project").unwrap();
                expected.name = "Renamed project".into();
            } else {
                person.set_person(|manifest| manifest.asset_budget_usd = 25.0).unwrap();
                expected.asset_budget_usd = 25.0;
            }
            assert_eq!(
                serde_json::to_value(&person.manifest).unwrap(),
                serde_json::to_value(&expected).unwrap(),
                "person-setting writes must keep the current build fields"
            );
            assert_eq!(serde_json::to_value(Workspace::open(&person.root).unwrap().manifest).unwrap(), serde_json::to_value(expected).unwrap());
        }
    }

    #[test]
    fn stale_manifest_updates_cannot_recreate_or_adopt_a_replacement_project() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("index.html"), b"Keep the app").unwrap();
        let mut old = Workspace::create_at(directory.path(), "Original", "Old request", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        old.clone().delete().unwrap();

        assert!(old.save().is_err(), "an update cannot publish a deleted project's initial manifest again");
        assert!(!old.root.join(MANIFEST).exists());
        assert!(!old.inbox_dir().exists(), "refusing the missing project does not create a coordination folder");

        let replacement = Workspace::create_at(directory.path(), "Replacement", "New request", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        let bytes = std::fs::read(replacement.root.join(MANIFEST)).unwrap();
        assert_eq!(old.save().unwrap_err().code, ErrorCode::FileChanged);
        assert_eq!(old.rename("A stale rename").unwrap_err().code, ErrorCode::FileChanged);
        assert_eq!(std::fs::read(replacement.root.join(MANIFEST)).unwrap(), bytes);
        assert_eq!(std::fs::read(replacement.root.join("index.html")).unwrap(), b"Keep the app");
    }

    #[test]
    fn manifest_updates_keep_project_deletion_outside_their_write() {
        use std::sync::mpsc;
        use std::time::Duration;
        let directory = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(directory.path(), "Original", "Old request", TargetRef::None).unwrap();
        let deleting = ws.clone();
        let (start_tx, start_rx) = mpsc::channel();
        let (deleted_tx, deleted_rx) = mpsc::channel();
        let mut deleted_during_update = false;
        std::thread::scope(|scope| {
            scope.spawn(move || {
                start_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                deleted_tx.send(deleting.delete()).unwrap();
            });
            let mut early = None;
            ws.set_person(|manifest| {
                start_tx.send(()).unwrap();
                early = deleted_rx.recv_timeout(Duration::from_millis(200)).ok();
                deleted_during_update = early.is_some();
                manifest.name = "Renamed".into();
            })
            .unwrap();
            early.unwrap_or_else(|| deleted_rx.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
        });

        assert!(!deleted_during_update, "the manifest merge and replacement must share the deletion guard");
        assert!(!ws.root.exists(), "the completed deletion cannot be undone by a late manifest write");
    }

    #[test]
    fn slugs_and_names() {
        assert_eq!(slug("  My Cool Site!! "), "my-cool-site");
        assert_eq!(slug("***"), "project");
        assert!(slug(&"a".repeat(100)).len() <= 40);
        assert_eq!(suggest_name("x", &TargetRef::Website { url: "https://www.example.com/a?b".into() }), "example.com");
        assert_eq!(suggest_name("Make a snake game in Rust please", &TargetRef::None), "Make a snake game in");
    }

    #[test]
    fn targets_parse() {
        assert_eq!(TargetRef::parse(" https://a.com "), TargetRef::Website { url: "https://a.com".into() });
        assert_eq!(TargetRef::parse("just words"), TargetRef::None);
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(TargetRef::parse(&dir.path().display().to_string()), TargetRef::Local { path: dir.path().to_path_buf() });
    }

    #[test]
    fn pictures_of_the_original_are_copied_numbered_and_kept_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::create(dir.path(), "Game", "Copy it", TargetRef::None).unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        for f in ["a.webp", "b.PNG", "notes.txt"] {
            std::fs::write(src.join(f), "x").unwrap();
        }
        assert_eq!(ws.add_references(&[src.join("a.webp"), src.join("notes.txt"), src.join("missing.png")]).unwrap(), 1);
        assert_eq!(ws.add_references(&[src.join("b.PNG")]).unwrap(), 1);
        let names: Vec<_> = ws.references().iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["picture-1.webp", "picture-2.png"]);
        // A message's pictures go to attachments/, numbered on their own.
        let got = ws.add_attachments(&[src.join("b.PNG"), src.join("notes.txt")]).unwrap();
        assert_eq!(got, vec![ws.attachments_dir().join("picture-1.png")]);
        assert_eq!(ws.references().len(), 2, "attachments aren't pictures of the original");
    }

    #[test]
    fn create_list_reload() {
        let dir = tempfile::tempdir().unwrap();
        let a = Workspace::create(dir.path(), "Example", "Clone it", TargetRef::Website { url: "https://example.com".into() }).unwrap();
        let mut b = Workspace::create(dir.path(), "Example", "Again", TargetRef::None).unwrap();
        assert_ne!(a.root, b.root, "same name gets a fresh folder");
        assert!(b.root.ends_with("example-2"));
        std::thread::sleep(std::time::Duration::from_millis(5));
        b.set_status(Status::Building, "Writing the page").unwrap();
        let all = Workspace::list(dir.path());
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].root, b.root, "most recently updated first");
        let mut a2 = a.clone();
        a2.reload().unwrap();
        assert_eq!(a2.manifest.request, "Clone it");
        std::fs::create_dir(dir.path().join("junk")).unwrap();
        assert_eq!(Workspace::list(dir.path()).len(), 2, "folders without a manifest are skipped");
    }

    #[test]
    fn project_copies_keep_runtime_obj_folders() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::create(dir.path(), "Models", "a model viewer", TargetRef::None).unwrap();
        let app = ws.app_dir();
        std::fs::create_dir_all(app.join("assets/obj")).unwrap();
        std::fs::write(app.join("assets/obj/hero.obj"), "v 0 0 0\n").unwrap();
        std::fs::create_dir_all(app.join("managed/obj")).unwrap();
        std::fs::write(app.join("managed/Viewer.csproj"), "<Project />").unwrap();
        std::fs::write(app.join("managed/obj/intermediate.bin"), [7; 256]).unwrap();
        let copy = ws.duplicate("Models copy").unwrap().app_dir();
        assert!(copy.join("assets/obj/hero.obj").is_file(), "a project copy keeps its runtime model");
        assert_eq!(std::fs::read_to_string(copy.join("assets/obj/hero.obj")).unwrap(), "v 0 0 0\n");
        assert!(copy.join("managed/Viewer.csproj").is_file());
        assert!(!copy.join("managed/obj").exists());
    }

    /// Older check records are judged by the fields they have.
    #[test]
    fn checks_from_older_projects_are_judged_as_they_were() {
        let old = |json: &str| serde_json::from_str::<CheckResult>(json).unwrap();
        let built = old(r#"{"round":1,"similarity":null,"note":"Build succeeded.","at_ms":5}"#);
        assert!(built.passed && built.check == CheckResult::BUILD && built.artifacts.is_empty());
        let failed = old(r#"{"round":2,"similarity":null,"note":"Build failed: error[E0308]","at_ms":5}"#);
        assert!(!failed.passed);
        let compared = old(r#"{"round":1,"similarity":71.0,"note":"1440 px: 71% alike","at_ms":5}"#);
        assert!(compared.passed && compared.check == CheckResult::WEB_VISUAL);
        // Written back with every field, and read the same.
        let again: CheckResult = serde_json::from_str(&serde_json::to_string(&compared).unwrap()).unwrap();
        assert_eq!(again, compared);
        assert!(compared.pages.is_empty() && !serde_json::to_string(&compared).unwrap().contains("pages"));
        let page = PageLikeness { name: "/about".into(), dir: "routes/about".into(), similarity: 80.0 };
        let multi = CheckResult { pages: vec![page], ..compared };
        let again: CheckResult = serde_json::from_str(&serde_json::to_string(&multi).unwrap()).unwrap();
        assert_eq!(again, multi, "each page's likeness is kept");
    }

    /// A craft.json as written before versions existed (and before checks were structured).
    const MANIFEST_V0: &str = r#"{
  "name": "Example",
  "request": "Clone it",
  "target": { "kind": "website", "url": "https://example.com" },
  "stack": "React + TypeScript (Vite)",
  "status": "ready",
  "note": "",
  "created_ms": 1700000000000,
  "updated_ms": 1700000005000,
  "checks": [ { "round": 1, "similarity": 71.0, "note": "1440 px: 71% alike", "at_ms": 1700000004000 } ],
  "agent_session": "abc",
  "allowed_actions": ["bash:npm"],
  "last_reply": "Done."
}"#;

    /// A version-0 manifest opens as current, with its legacy session counting as Deep.
    #[test]
    fn manifests_from_before_versions_open_and_are_saved_as_current() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(MANIFEST), MANIFEST_V0).unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        assert_eq!(ws.manifest.version, MANIFEST_VERSION);
        assert_eq!(ws.manifest.agent_session_deep, Some(true), "a session from before the mode was kept counts as Deep");
        assert_eq!(ws.manifest.capture_deep, Some(true), "a capture from before the mode was kept counts as Deep");
        assert_eq!(ws.manifest.agent_session_for_mode("claude-code", false), None);
        assert_eq!(ws.manifest.status, Status::Ready);
        assert_eq!(ws.manifest.asset_budget_usd, ASSET_BUDGET_USD);
        assert!(!ws.manifest.generate_assets, "assets are generated only once the person turns it on");
        assert!(ws.manifest.checks[0].passed && !ws.is_read_only());
        ws.save().unwrap();
        let saved: Value = serde_json::from_slice(&std::fs::read(dir.path().join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(saved["version"], MANIFEST_VERSION);
        assert_eq!(Workspace::open(dir.path()).unwrap().manifest, ws.manifest);
    }

    #[test]
    fn project_generations_survive_updates_and_are_new_for_copies() {
        let directory = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(directory.path(), "Project", "Test", TargetRef::None).unwrap();
        let generation = ws.manifest.generation.clone().unwrap();
        assert_eq!(generation.len(), 32);
        assert!(generation.bytes().all(|byte| byte.is_ascii_hexdigit()));
        ws.manifest.generation = None; // A cached manifest cannot erase the saved identity.
        assert_eq!(ws.set_status(Status::Building, "Working").unwrap_err().code, ErrorCode::FileChanged);
        assert_eq!(Workspace::open(&ws.root).unwrap().manifest.generation.as_ref(), Some(&generation));
        ws.reload().unwrap();
        ws.set_status(Status::Building, "Working").unwrap();
        assert_eq!(Workspace::open(&ws.root).unwrap().manifest.generation.as_ref(), Some(&generation));
        ws.set_person(|manifest| manifest.name = "Renamed".into()).unwrap();
        assert_eq!(Workspace::open(&ws.root).unwrap().manifest.generation.as_ref(), Some(&generation));
        let copy = Workspace::open(&ws.root).unwrap().duplicate("Copy").unwrap();
        assert!(copy.manifest.generation.is_some());
        assert_ne!(copy.manifest.generation.as_ref(), Some(&generation));
        assert_eq!(Workspace::open(&copy.root).unwrap().manifest.generation, copy.manifest.generation);
    }

    #[test]
    fn a_version_5_project_keeps_its_legacy_identity_without_writing_on_open() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Legacy", "Test", TargetRef::None).unwrap();
        let path = ws.root.join(MANIFEST);
        let mut document = serde_json::to_value(&ws.manifest).unwrap();
        document["version"] = Value::from(5);
        document.as_object_mut().unwrap().remove("generation");
        let original = serde_json::to_vec(&document).unwrap();
        std::fs::write(&path, &original).unwrap();

        let mut opened = Workspace::open(&ws.root).unwrap();
        assert_eq!(opened.manifest.version, MANIFEST_VERSION);
        assert!(opened.manifest.generation.is_none());
        assert_eq!(opened.manifest, Workspace::open(&ws.root).unwrap().manifest);
        assert_eq!(std::fs::read(&path).unwrap(), original, "opening a legacy project is read-only migration");
        opened.set_status(Status::Building, "Working").unwrap();
        let saved: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(saved["version"], MANIFEST_VERSION);
        assert!(saved.get("generation").is_none(), "ordinary saves do not invent a new identity for a legacy project");
    }

    #[test]
    fn version_6_toolchains_and_existing_generations_survive_migration_and_save() {
        const V6: &str = r#"{
  "version": 6,
  "name": "Java port",
  "request": "Move this app to Java",
  "target": { "kind": "none" },
  "stack": "Java",
  "toolchain": "Gradle",
  "created_ms": 1700000000000,
  "updated_ms": 1700000005000
}"#;
        for generation in [None, Some("0123456789abcdef0123456789abcdef")] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join(MANIFEST);
            // Early builds of this change also wrote generation at version 6. Keep it,
            // while ordinary version-6 projects retain their absent legacy identity.
            let original = generation.map_or_else(|| V6.to_owned(), |value| V6.replacen('{', &format!(r#"{{ "generation": "{value}","#), 1));
            std::fs::write(&path, &original).unwrap();
            let mut opened = Workspace::open(directory.path()).unwrap();
            assert_eq!(opened.manifest.version, MANIFEST_VERSION);
            assert_eq!(opened.manifest.toolchain.as_deref(), Some("Gradle"));
            assert_eq!(opened.manifest.generation.as_deref(), generation);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original, "opening only migrates in memory");
            opened.set_status(Status::Building, "Working").unwrap();
            let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(saved["version"], MANIFEST_VERSION);
            assert_eq!(saved["stack"], "Java");
            assert_eq!(saved["toolchain"], "Gradle");
            assert_eq!(saved.get("generation").and_then(Value::as_str), generation);
            assert_eq!(Workspace::open(directory.path()).unwrap().manifest, opened.manifest);
        }
    }

    /// Approval choices survive an old manifest, a stale save and a copy.
    #[test]
    fn approval_choices_survive_an_old_manifest_a_stale_save_and_a_copy() {
        // A version-2 craft.json (before approvals) opens with none chosen and is saved as current.
        let dir = tempfile::tempdir().unwrap();
        let v2 = MANIFEST_V0.replacen('{', r#"{ "version": 2,"#, 1);
        std::fs::write(dir.path().join(MANIFEST), &v2).unwrap();
        let mut run = Workspace::open(dir.path()).unwrap();
        assert_eq!((run.manifest.version, run.manifest.approvals.is_empty(), run.is_read_only()), (MANIFEST_VERSION, true, false));
        run.save().unwrap();
        let saved: Value = serde_json::from_slice(&std::fs::read(dir.path().join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(saved["version"], MANIFEST_VERSION);
        assert!(saved.get("approvals").is_none(), "nothing chosen, nothing written");
        // The person chooses while a run holds an older copy; the run's save keeps the choice.
        Workspace::open(dir.path())
            .unwrap()
            .set_person(|m| {
                m.approvals.insert(ActionKind::Publish, Approval::Ask);
                m.approvals.insert(ActionKind::Commands, Approval::Refuse);
            })
            .unwrap();
        run.manifest.note = "Building".into();
        run.save().unwrap();
        let disk = Workspace::open(dir.path()).unwrap().manifest;
        assert_eq!(disk.approvals.get(&ActionKind::Publish), Some(&Approval::Ask));
        assert_eq!(disk.approval(ActionKind::Commands, true), Approval::Refuse);
        let saved: Value = serde_json::from_slice(&std::fs::read(dir.path().join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(saved["approvals"], serde_json::json!({"commands": "refuse", "publish": "ask"}));
        // A copy keeps them.
        let copy = Workspace::open(dir.path()).unwrap().duplicate("Example copy").unwrap();
        assert_eq!(Workspace::open(&copy.root).unwrap().manifest.approvals, disk.approvals);
        // An unknown kind isn't read as something else.
        let unknown = MANIFEST_V0.replacen('{', r#"{ "version": 3, "approvals": {"everything": "allow"},"#, 1);
        assert!(read_manifest(unknown.as_bytes()).is_err());
    }

    #[test]
    fn newer_manifests_open_read_only_and_are_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let newer = MANIFEST_V0.replacen('{', r#"{ "version": 99, "a_field_from_the_future": [1, 2],"#, 1);
        std::fs::write(dir.path().join(MANIFEST), &newer).unwrap();
        let mut ws = Workspace::open(dir.path()).unwrap();
        assert!(ws.is_read_only());
        assert_eq!(ws.manifest.name, "Example");
        let err = ws.set_status(Status::Building, "x").unwrap_err();
        assert_eq!(err.code, ErrorCode::NotSupported);
        assert!(err.message.contains("newer") && err.remediation.is_some());
        assert_eq!(std::fs::read_to_string(dir.path().join(MANIFEST)).unwrap(), newer, "the file is untouched");
        assert_eq!(Workspace::list(dir.path().parent().unwrap()).iter().filter(|w| w.root == dir.path()).count(), 1, "still listed");

        // A newer manifest whose shape this build can't read at all is refused, in plain words.
        std::fs::write(dir.path().join(MANIFEST), r#"{"version": 99, "name": {"parts": []}}"#).unwrap();
        let err = Workspace::open(dir.path()).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotSupported);
        assert!(err.message.contains("newer Craft Anything") && err.remediation.is_some());
    }

    #[test]
    fn broken_versions_are_unreadable_not_crashes() {
        for bad in [r#"{"version": "one"}"#, r#"{"version": -1}"#, r#"{"version": 5000000000}"#, "[1, 2]", "", "{"] {
            assert!(read_manifest(bad.as_bytes()).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_published_schema_is_current() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/schemas/craft.schema.json");
        let published = std::fs::read_to_string(path).unwrap_or_default().replace("\r\n", "\n");
        assert!(published == manifest_schema(), "docs/schemas/craft.schema.json is stale: run `cargo xtask schemas`");
        let schema: Value = serde_json::from_str(&published).unwrap();
        assert!(schema["properties"]["version"].is_object() && schema["properties"]["checks"].is_object());
    }

    #[test]
    fn good_means_working_and_alike_enough() {
        let web = |s: f64| CheckResult { similarity: Some(s), ..CheckResult::new(CheckResult::WEB_VISUAL, 1, true, "") };
        assert!(web(92.0).good(90.0));
        assert!(!web(80.0).good(90.0));
        assert!(CheckResult::new(CheckResult::BUILD, 1, true, "Build succeeded.").good(90.0), "nothing to compare: working is enough");
        assert!(!CheckResult::new(CheckResult::BUILD, 1, false, "Build failed").good(0.0));
        // A check with its own bar (likeness to the person's pictures) uses it instead.
        let pics = CheckResult { similarity: Some(70.0), good_enough: Some(60.0), ..CheckResult::new("app-screenshot", 1, true, "") };
        assert!(pics.good(90.0), "70% clears its own 60% bar");
        assert!(!CheckResult { good_enough: Some(80.0), ..pics.clone() }.good(0.0));
        assert!(!CheckResult { unknowns: vec!["/about couldn't be pictured".into()], ..pics.clone() }.good(0.0), "missing evidence is never good enough");
        let held = Claim::new("No sideways scroll at 390 px", true, "it's 512 px wide");
        assert!(held.detail.is_empty(), "a claim that holds has nothing to explain");
        let broken = Claim::new("Every file loads", false, "/logo.png (404)");
        assert!(CheckResult { claims: vec![held.clone()], ..pics.clone() }.good(0.0));
        let c = CheckResult { claims: vec![held, broken], ..pics };
        assert!(!c.good(0.0), "a failed claim never rolls up into a pass");
        assert_eq!(c.failed_claims().map(|c| c.detail.as_str()).collect::<Vec<_>>(), ["/logo.png (404)"]);
    }

    #[test]
    fn a_project_starts_in_a_picked_folder_that_is_missing_or_empty() {
        let d = tempfile::tempdir().unwrap();
        for folder in [d.path().join("new"), d.path().join("empty")] {
            if folder.ends_with("empty") {
                std::fs::create_dir(&folder).unwrap();
            }
            assert_eq!(folder_state(&folder).unwrap(), FolderState::Empty);
            let ws = Workspace::create_at(&folder, "Notes", "A notes app", TargetRef::None, None).unwrap();
            assert_eq!(ws.root, folder, "the folder itself, not a new one inside it");
            assert_eq!(ws.app_dir(), folder.join("app"));
            assert!(ws.manifest.start_files.is_empty() && !ws.manifest.app_in_root);
            assert_eq!(folder_state(&folder).unwrap(), FolderState::Project);
            let again = Workspace::create_at(&folder, "Notes", "x", TargetRef::None, None).unwrap_err();
            assert!(again.message.contains("already a project"), "{}", again.message);
        }
    }

    #[test]
    fn a_folder_with_files_says_what_they_are_and_keeps_them() {
        let d = tempfile::tempdir().unwrap();
        let mine = |name: &str| {
            let f = d.path().join(name);
            std::fs::create_dir_all(f.join("src")).unwrap();
            std::fs::write(f.join("Cargo.toml"), "[package]").unwrap();
            std::fs::write(f.join("src/main.rs"), "fn main() {}").unwrap();
            f
        };
        let edit = mine("edit");
        assert_eq!(folder_state(&edit).unwrap(), FolderState::HasFiles { names: vec!["Cargo.toml".into(), "src".into()], count: 2 });
        let e = Workspace::create_at(&edit, "Mine", "Add tabs", TargetRef::None, None).unwrap_err();
        assert!(e.remediation.unwrap_or_default().contains("existing: edit"), "it says how to answer");
        assert!(!edit.join(MANIFEST).exists(), "nothing written until it's answered");

        let ws = Workspace::create_at(&edit, "Mine", "Add tabs", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        assert!(ws.manifest.app_in_root);
        assert_eq!(ws.app_dir(), edit, "their files are the app");
        assert_eq!(ws.manifest.start_files, ["Cargo.toml", "src"]);
        assert_eq!(std::fs::read_to_string(edit.join("src/main.rs")).unwrap(), "fn main() {}", "their files are left as they were");
        let reopened = Workspace::open(&edit).unwrap();
        assert_eq!(reopened.app_dir(), edit, "it's saved");

        let start = mine("start");
        let ws = Workspace::create_at(&start, "Mine", "Add tabs", TargetRef::None, Some(ExistingFiles::StartFrom)).unwrap();
        assert!(!ws.manifest.app_in_root);
        assert_eq!(ws.app_dir(), start.join("app"), "the app goes beside them");
        assert_eq!(ws.manifest.start_files, ["Cargo.toml", "src"]);
    }

    #[test]
    fn a_folder_with_names_craft_anything_uses_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let checks = d.path().join("checks-here");
        std::fs::create_dir_all(checks.join("checks")).unwrap();
        for how in [ExistingFiles::Edit, ExistingFiles::StartFrom] {
            assert_eq!(clashes(&checks, how), ["checks"]);
            let e = Workspace::create_at(&checks, "x", "x", TargetRef::None, Some(how)).unwrap_err();
            assert!(e.message.contains("checks"), "{}", e.message);
        }
        // Their own app/ is only in the way when the app would be built in app/.
        let app = d.path().join("app-here");
        std::fs::create_dir_all(app.join("app")).unwrap();
        assert_eq!(clashes(&app, ExistingFiles::StartFrom), ["app"]);
        assert!(clashes(&app, ExistingFiles::Edit).is_empty());
        assert!(Workspace::create_at(&app, "x", "x", TargetRef::None, Some(ExistingFiles::Edit)).is_ok());
        assert!(!checks.join(MANIFEST).exists());
    }

    #[test]
    fn projects_are_listed_from_every_location_once() {
        let d = tempfile::tempdir().unwrap();
        let (many, own) = (d.path().join("many"), d.path().join("elsewhere").join("mine"));
        let a = Workspace::create(&many, "Alpha", "x", TargetRef::None).unwrap();
        let b = Workspace::create(&many, "Beta", "x", TargetRef::None).unwrap();
        let c = Workspace::create_at(&own, "Mine", "x", TargetRef::None, None).unwrap();
        let roots = |locs: &[PathBuf]| {
            let mut v: Vec<PathBuf> = Workspace::list_in(locs).into_iter().map(|w| w.root).collect();
            v.sort();
            v
        };
        let mut all = vec![a.root.clone(), b.root.clone(), c.root.clone()];
        all.sort();
        // A folder of projects, a project itself, the same project again and a folder that's gone.
        assert_eq!(roots(&[many.clone(), own.clone(), a.root.clone(), d.path().join("gone")]), all);
        assert_eq!(roots(std::slice::from_ref(&many)), [a.root.clone(), b.root.clone()]);
        assert_eq!(Workspace::list(&many).len(), 2);
    }

    #[test]
    fn a_version_1_manifest_reads_as_a_usual_project_and_new_fields_are_saved_only_when_set() {
        let v1 = br#"{"version":1,"name":"Old","request":"x","target":{"kind":"none"},"created_ms":1,"updated_ms":1}"#;
        let m = read_manifest(v1).unwrap();
        assert_eq!(m.version, MANIFEST_VERSION, "brought forward");
        assert!(!m.app_in_root && m.start_files.is_empty());
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "New", "x", TargetRef::None).unwrap();
        let saved: Value = serde_json::from_slice(&std::fs::read(ws.root.join(MANIFEST)).unwrap()).unwrap();
        assert!(saved.get("app_in_root").is_none() && saved.get("start_files").is_none(), "a usual project's craft.json is unchanged");
    }

    /// A project's own records are recognized even when the app is the folder.
    #[test]
    fn craft_anythings_records_are_its_own_even_when_the_app_is_the_folder() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("index.html"), "<p>hi</p>").unwrap();
        let ws = Workspace::create_at(d.path(), "Site", "x", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        for own in ["craft.json", "feed.json", "inbox/a.json", "reference/picture-1.png", "investigation.cany/x"] {
            assert!(ws.is_own_record(&ws.root.join(own)), "{own}");
        }
        for theirs in ["index.html", "src/main.rs", "checks/round-1/notes.md", "craft.json.bak"] {
            assert!(!ws.is_own_record(&ws.root.join(theirs)), "{theirs}");
        }
        if cfg!(windows) {
            assert!(ws.is_own_record(&ws.root.join("CRAFT.JSON")), "paths on Windows ignore case");
        }
    }

    /// R2.9: version 3 predates the saved session's mode; a session is migrated as Deep, while a
    /// mode an early build of the change already wrote is kept.
    #[test]
    fn a_version_3_session_is_migrated_as_deep_without_overwriting_a_recorded_mode() {
        let v3 = MANIFEST_V0.replacen('{', r#"{ "version": 3,"#, 1);
        assert_session_migrated(&v3, Some(true));
        let recorded = v3.replace(r#""agent_session": "abc","#, r#""agent_session": "abc", "agent_session_deep": false,"#);
        assert_session_migrated(&recorded, Some(false));
    }

    /// R2.9: version 4 predates the capture's mode. A capture it left is recorded as Deep (the
    /// legacy case), and a mode an early build of the change already wrote is kept.
    #[test]
    fn a_version_4_capture_mode_is_kept_and_an_older_one_migrates_as_deep() {
        let v4 = MANIFEST_V0.replacen('{', r#"{ "version": 4,"#, 1);
        assert_eq!(read_manifest(v4.as_bytes()).unwrap().capture_deep, Some(true), "a capture from before the field counts as Deep");
        let recorded = v4.replace(r#""agent_session": "abc","#, r#""agent_session": "abc", "capture_deep": false,"#);
        assert_eq!(read_manifest(recorded.as_bytes()).unwrap().capture_deep, Some(false), "a recorded mode is kept");

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(MANIFEST), &v4).unwrap();
        let mut ws = Workspace::open(dir.path()).unwrap();
        assert_eq!((ws.manifest.version, ws.manifest.capture_deep), (MANIFEST_VERSION, Some(true)));
        ws.save().unwrap();
        let saved: Value = serde_json::from_slice(&std::fs::read(dir.path().join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(saved["version"], MANIFEST_VERSION);
        assert_eq!(saved["capture_deep"], true, "the legacy capture is recorded as Deep");
        ws.manifest.capture_deep = Some(false);
        ws.save().unwrap();
        let saved: Value = serde_json::from_slice(&std::fs::read(dir.path().join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(saved["capture_deep"], false);
    }

    /// Read a literal `craft.json` sample, check both modes and the saved document.
    fn assert_session_migrated(doc: &str, want: Option<bool>) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(MANIFEST), doc).unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        assert_eq!((ws.manifest.version, ws.manifest.agent_session_deep), (MANIFEST_VERSION, want));
        let deep = ws.manifest.agent_session_for_mode("claude-code", true);
        let clean = ws.manifest.agent_session_for_mode("claude-code", false);
        assert_eq!((deep, clean), if want == Some(false) { (None, Some("abc")) } else { (Some("abc"), None) });
        ws.save().unwrap();
        let saved: Value = serde_json::from_slice(&std::fs::read(dir.path().join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(saved["version"], MANIFEST_VERSION);
        assert_eq!(saved["agent_session_deep"], want == Some(true));
    }

    /// Review of #2099: a version-5 build can't save over a recorded toolchain, and a version-5
    /// manifest opens here unchanged but for its number.
    #[test]
    fn a_recorded_toolchain_is_kept_from_older_builds() {
        let d = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(d.path(), "Game", "a game", TargetRef::None).unwrap();
        ws.manifest.toolchain = Some("Gradle".into());
        ws.save().unwrap();
        let saved: Value = serde_json::from_slice(&std::fs::read(ws.root.join("craft.json")).unwrap()).unwrap();
        assert_eq!((saved["version"].as_u64(), saved["toolchain"].as_str()), (Some(u64::from(MANIFEST_VERSION)), Some("Gradle")));
        // A version-5 build reads up to 5, so it opens this one read-only (newer_manifests_open_read_only_and_are_never_overwritten).
        let mut old = saved.clone();
        old["version"] = 5.into();
        old.as_object_mut().unwrap().remove("toolchain");
        let m = read_manifest(&serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!((m.version, m.toolchain), (MANIFEST_VERSION, None), "brought forward with no toolchain");
    }

    /// Review of #2099: a build holding a copy read before a newer build brought the file forward
    /// (both running at once) doesn't save over it, by any of the ways it saves.
    #[test]
    fn a_copy_read_before_a_newer_build_saved_doesnt_overwrite_it() {
        let d = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(d.path(), "Game", "a game", TargetRef::None).unwrap();
        let original = std::fs::read(ws.root.join(MANIFEST)).unwrap();
        let mut newer: Value = serde_json::from_slice(&original).unwrap();
        newer["version"] = (MANIFEST_VERSION + 1).into();
        newer["a_field_from_the_future"] = "kept".into();
        let newer = serde_json::to_string_pretty(&newer).unwrap();
        std::fs::write(ws.root.join(MANIFEST), &newer).unwrap();
        assert!(!ws.is_read_only(), "this copy was read before");
        let err = ws.set_status(Status::Building, "x").unwrap_err();
        assert_eq!(err.code, ErrorCode::NotSupported);
        assert!(err.message.contains(&format!("version {}", MANIFEST_VERSION + 1)), "{}", err.message);
        assert_eq!(ws.save().unwrap_err().code, ErrorCode::NotSupported);
        assert_eq!(ws.set_person(|m| m.note = "x".into()).unwrap_err().code, ErrorCode::NotSupported);
        assert_eq!(std::fs::read_to_string(ws.root.join(MANIFEST)).unwrap(), newer, "the file is untouched");
        // An unreadable manifest cannot prove that this is still the loaded project.
        std::fs::write(ws.root.join(MANIFEST), b"not json").unwrap();
        assert!(ws.save().is_err());
        assert_eq!(std::fs::read(ws.root.join(MANIFEST)).unwrap(), b"not json");
        // Its own readable version on disk still saves.
        std::fs::write(ws.root.join(MANIFEST), original).unwrap();
        ws.save().unwrap();
    }

    /// #469: a saved session is only for the agent that saved it.
    #[test]
    fn a_session_is_only_for_the_agent_that_saved_it() {
        let mut m = Manifest::new("x", "A thing", TargetRef::None).unwrap();
        assert_eq!(m.agent_session_for("codex"), None);
        m.agent_session = Some("s1".into());
        assert_eq!(m.agent_session_for("codex"), Some("s1"), "one saved before the agent was kept counts");
        m.agent_session_by = Some("claude-code".into());
        assert_eq!(m.agent_session_for("claude-code"), Some("s1"));
        assert_eq!(m.agent_session_for("codex"), None);
        assert_eq!(m.agent_session_for_mode("claude-code", true), Some("s1"), "one saved before the mode was kept counts as Deep");
        assert_eq!(m.agent_session_for_mode("claude-code", false), None, "Deep may have read the original, so Clean room starts fresh");
        m.agent_session_deep = Some(false);
        assert_eq!(m.agent_session_for_mode("claude-code", false), Some("s1"));
        assert_eq!(m.agent_session_for_mode("claude-code", true), None);
        m.agent_session_deep = Some(true);
        assert_eq!(m.agent_session_for_mode("claude-code", true), Some("s1"));
        m.agent_session_deep = None;
        let text = serde_json::to_string(&m).unwrap();
        let back: Manifest = serde_json::from_str(&text).unwrap();
        assert_eq!((back.agent_session_by.as_deref(), back.agent_session_deep), (Some("claude-code"), None));
    }

    /// A saved session is history only in the mode it ran in; one from before the mode was kept
    /// counts as Deep.
    #[test]
    fn a_saved_session_counts_as_history_only_in_its_mode() {
        let d = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(d.path(), "x", "A thing", TargetRef::None).unwrap();
        assert!(!ws.builder_session_in(true) && !ws.builder_session_in(false), "nothing saved yet");
        ws.manifest.agent_session = Some("s1".into());
        assert!(ws.builder_session_in(true), "one saved before the mode was kept counts as Deep");
        assert!(!ws.builder_session_in(false), "Deep may have read the original");
        ws.manifest.agent_session_deep = Some(false);
        assert!(!ws.builder_session_in(true));
        assert!(ws.builder_session_in(false));
    }

    #[test]
    fn copy_tree_refuses_an_existing_destination_directory() {
        let directory = tempfile::tempdir().unwrap();
        let from = directory.path().join("source");
        let to = directory.path().join("destination");
        std::fs::create_dir(&from).unwrap();
        std::fs::create_dir(&to).unwrap();
        std::fs::write(from.join("shared.txt"), b"Source bytes").unwrap();
        std::fs::write(to.join("shared.txt"), b"Keep existing personal bytes").unwrap();

        let result = copy_tree(&from, &to, &|_| false, 0, None);

        assert_eq!(std::fs::read(to.join("shared.txt")).unwrap(), b"Keep existing personal bytes");
        assert_eq!(std::fs::read(from.join("shared.txt")).unwrap(), b"Source bytes");
        assert!(result.is_err(), "copying never merges into an existing destination folder");
    }

    #[test]
    fn copy_tree_refuses_a_file_added_after_destination_creation() {
        let directory = tempfile::tempdir().unwrap();
        let from = directory.path().join("source");
        let to = directory.path().join("destination");
        std::fs::create_dir(&from).unwrap();
        std::fs::write(from.join("shared.txt"), b"Source bytes").unwrap();

        let result = copy_tree(
            &from,
            &to,
            &|_| {
                std::fs::write(to.join("shared.txt"), b"Keep this newly arrived file").unwrap();
                false
            },
            0,
            None,
        );

        assert_eq!(std::fs::read(to.join("shared.txt")).unwrap(), b"Keep this newly arrived file");
        assert_eq!(std::fs::read(from.join("shared.txt")).unwrap(), b"Source bytes");
        assert!(result.is_err(), "a fresh parent does not authorize replacing a file that appeared inside it");
    }

    #[test]
    fn copy_tree_preserves_personal_bytes_when_the_destination_root_is_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let from = directory.path().join("source");
        std::fs::create_dir(&from).unwrap();
        std::fs::write(from.join("shared.txt"), b"Source bytes").unwrap();
        let destination = CopyDestination::create(directory.path(), "Copy").unwrap();
        let displaced = directory.path().join("displaced-copy");
        let root = destination.root.clone();
        let to = destination.root.join("app");

        let copied = copy_tree(
            &from,
            &to,
            &|_| {
                rename_creation_root_for_replacement(&destination.root, &displaced);
                std::fs::create_dir(&destination.root).unwrap();
                std::fs::create_dir(&to).unwrap();
                std::fs::write(to.join("shared.txt"), b"Keep the replacement's personal bytes").unwrap();
                false
            },
            0,
            None,
        )
        .and_then(|()| destination.check_identity());

        assert!(copied.is_err());
        assert!(destination.remove().is_err(), "failed cleanup must leave the replacement root intact");
        assert_eq!(std::fs::read(to.join("shared.txt")).unwrap(), b"Keep the replacement's personal bytes");
        assert_eq!(std::fs::read(from.join("shared.txt")).unwrap(), b"Source bytes");
        assert!(!root.join(MANIFEST).exists());
        assert!(displaced.join("inbox").is_dir());
        assert!(displaced.join("app").is_dir());
    }

    #[test]
    fn copy_tree_keeps_nested_file_bytes_and_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let from = directory.path().join("source");
        let to = directory.path().join("destination");
        std::fs::create_dir_all(from.join("src")).unwrap();
        std::fs::write(from.join("src/main.rs"), b"fn main() {}\n").unwrap();
        std::fs::write(from.join("run.sh"), b"#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(from.join("src/main.rs"), std::fs::Permissions::from_mode(0o444)).unwrap();
            std::fs::set_permissions(from.join("run.sh"), std::fs::Permissions::from_mode(0o751)).unwrap();
        }

        copy_tree(&from, &to, &|_| false, 0, None).unwrap();

        for relative in ["src/main.rs", "run.sh"] {
            assert_eq!(std::fs::read(to.join(relative)).unwrap(), std::fs::read(from.join(relative)).unwrap());
            let before = std::fs::metadata(from.join(relative)).unwrap().permissions();
            let after = std::fs::metadata(to.join(relative)).unwrap().permissions();
            assert_eq!(after.readonly(), before.readonly());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                assert_eq!(after.mode(), before.mode());
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn new_file_copies_preserve_ntfs_streams_and_refuse_occupied_destinations() {
        fn stream(path: &Path) -> PathBuf {
            let mut name = path.as_os_str().to_owned();
            name.push(":Zone.Identifier");
            name.into()
        }
        for name in ["document.txt", "feed.json.broken"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("source");
            std::fs::create_dir(&root).unwrap();
            let from = root.join(name);
            let to = directory.path().join("copy.txt");
            std::fs::write(&from, b"Generated ordinary content").unwrap();
            std::fs::write(stream(&from), b"Generated alternate stream").unwrap();
            let backups = CopyBackups::discover(&root).unwrap();
            let copy = || {
                if name == "feed.json.broken" { copy_personal_backup(&backups, &from, &to) } else { copy_new_file(&from, &to) }
            };

            copy().unwrap();
            assert_eq!(std::fs::read(&to).unwrap(), b"Generated ordinary content");
            assert_eq!(std::fs::read(stream(&to)).unwrap(), b"Generated alternate stream");

            std::fs::write(&to, b"Keep personal bytes").unwrap();
            std::fs::write(stream(&to), b"Keep personal alternate stream").unwrap();
            assert!(copy().is_err(), "an occupied output is never overwritten");
            assert_eq!(std::fs::read(&to).unwrap(), b"Keep personal bytes");
            assert_eq!(std::fs::read(stream(&to)).unwrap(), b"Keep personal alternate stream");
        }
    }

    /// A rename survives a stale save, a copy takes the build but not the conversation (nor its
    /// kept session), and a delete leaves the person's own files.
    #[test]
    fn a_project_is_renamed_duplicated_and_deleted_without_touching_the_persons_files() {
        let d = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(d.path(), "Snake", "A snake game", TargetRef::None).unwrap();
        std::fs::create_dir_all(ws.app_dir().join("src")).unwrap();
        std::fs::write(ws.app_dir().join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::create_dir_all(ws.app_dir().join("target/debug")).unwrap();
        std::fs::write(ws.app_dir().join("target/debug/big.bin"), "x").unwrap();
        std::fs::create_dir_all(ws.reference_dir()).unwrap();
        std::fs::write(ws.reference_dir().join("picture-1.png"), "x").unwrap();
        ws.manifest.agent_session = Some("old".into());
        ws.save().unwrap();
        std::fs::write(ws.agent_session_path(), "{}\n").unwrap();

        ws.rename("  Snake II ").unwrap();
        assert_eq!(Workspace::open(&ws.root).unwrap().manifest.name, "Snake II");
        assert!(ws.rename(" ").is_err());
        // A run saving its own copy keeps the person's new name.
        let mut stale = ws.clone();
        stale.manifest.name = "Snake".into();
        stale.save().unwrap();
        assert_eq!(Workspace::open(&ws.root).unwrap().manifest.name, "Snake II");

        let copy = ws.duplicate("Snake III").unwrap();
        assert_ne!(copy.root, ws.root);
        assert_eq!(std::fs::read_to_string(copy.app_dir().join("src/main.rs")).unwrap(), "fn main() {}");
        assert!(copy.reference_dir().join("picture-1.png").is_file());
        assert!(!copy.app_dir().join("target").exists(), "compiler output is made again, not copied");
        assert!(copy.manifest.agent_session.is_none(), "a copy has its own conversation");
        assert!(!copy.agent_session_path().exists(), "and doesn't carry the original's");

        ws.delete().unwrap();
        assert!(!d.path().join("snake").exists(), "a project of its own goes with its folder");

        // An app edited in place: Craft Anything's records go, the person's files stay.
        let mine = d.path().join("mine");
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::write(mine.join("index.html"), "<p>hi</p>").unwrap();
        let ws = Workspace::create_at(&mine, "Site", "x", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        std::fs::create_dir_all(ws.inbox_dir()).unwrap();
        std::fs::write(ws.agent_session_path(), "{}\n").unwrap();
        let copy = ws.duplicate("Site copy").unwrap();
        assert!(copy.app_dir().join("index.html").is_file() && !copy.manifest.app_in_root, "the copy's app is in its app/");
        assert!(!copy.app_dir().join(MANIFEST).exists(), "without the original's records");
        ws.delete().unwrap();
        assert_eq!(std::fs::read_to_string(mine.join("index.html")).unwrap(), "<p>hi</p>");
        assert!(!mine.join(MANIFEST).exists() && !mine.join("inbox").exists() && !mine.join(AGENT_SESSION_FILE).exists());
    }

    #[test]
    fn a_picked_folder_with_a_recovery_backup_is_left_alone() {
        for index in 0..16 {
            let name = if index == 0 { "feed.json.broken".into() } else { format!("feed.json.broken.{index}") };
            for existing in [ExistingFiles::Edit, ExistingFiles::StartFrom] {
                let d = tempfile::tempdir().unwrap();
                let original = b"An older recovery copy belonging to the person";
                std::fs::write(d.path().join(&name), original).unwrap();

                let error = Workspace::create_at(d.path(), "Site", "A website", TargetRef::None, Some(existing)).unwrap_err();

                assert!(error.message.contains(&name), "{error:?}");
                assert_eq!(clashes(d.path(), existing), [name.as_str()]);
                assert_eq!(std::fs::read(d.path().join(&name)).unwrap(), original);
                assert!(!d.path().join(MANIFEST).exists(), "no project is created over the person's recovery file");
            }
        }
    }

    #[test]
    fn deleting_a_project_removes_every_recovered_feed_backup() {
        for in_place in [false, true] {
            let d = tempfile::tempdir().unwrap();
            let ws = if in_place {
                std::fs::write(d.path().join("index.html"), "<p>Keep my app</p>").unwrap();
                Workspace::create_at(d.path(), "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap()
            } else {
                Workspace::create(d.path(), "Site", "A website", TargetRef::None).unwrap()
            };
            let root = ws.root.clone();
            let mut backups = Vec::new();
            for index in 0..16 {
                let original = format!("damaged conversation {index}");
                std::fs::write(root.join(crate::feed::FEED_FILE), original.as_bytes()).unwrap();
                let mut items = crate::feed::load(&ws);
                crate::feed::save(&root, &mut items).unwrap();
                let name = if index == 0 { "feed.json.broken".into() } else { format!("feed.json.broken.{index}") };
                let backup = root.join(name);
                assert_eq!(std::fs::read(&backup).unwrap(), original.as_bytes());
                backups.push(backup);
            }
            if in_place {
                std::fs::write(root.join("feed.json.broken.notes"), "my notes").unwrap();
                std::fs::write(root.join("feed.json.broken.16"), "my other file").unwrap();
            }

            ws.delete().unwrap();

            for backup in backups {
                assert!(!backup.exists(), "the deleted conversation remains at {}", backup.display());
            }
            if in_place {
                assert_eq!(std::fs::read_to_string(root.join("index.html")).unwrap(), "<p>Keep my app</p>");
                assert_eq!(std::fs::read_to_string(root.join("feed.json.broken.notes")).unwrap(), "my notes");
                assert_eq!(std::fs::read_to_string(root.join("feed.json.broken.16")).unwrap(), "my other file");
                assert!(!root.join(MANIFEST).exists());
            } else {
                assert!(!root.exists(), "a project with no personal files is removed with its folder");
            }
        }
    }

    #[test]
    fn deleting_a_legacy_project_preserves_personal_files_at_exact_backup_names() {
        for recover in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            std::fs::write(directory.path().join("index.html"), "Keep my app").unwrap();
            let ws = Workspace::create_at(directory.path(), "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
            let root = ws.root.clone();
            // An existing project may predate these reserved names. A name alone cannot
            // establish that its folder or file was generated by conversation recovery.
            let personal_directory = root.join("feed.json.broken.1");
            std::fs::create_dir(&personal_directory).unwrap();
            std::fs::write(personal_directory.join("notes.txt"), "Keep these notes").unwrap();
            let personal_file = root.join("feed.json.broken.2");
            std::fs::write(&personal_file, "Keep this file").unwrap();
            let generated = root.join("feed.json.broken");
            if recover {
                std::fs::write(root.join(crate::feed::FEED_FILE), b"damaged conversation").unwrap();
                let mut feed = crate::feed::load(&ws);
                crate::feed::save(&root, &mut feed).unwrap();
                assert_eq!(std::fs::read(&generated).unwrap(), b"damaged conversation");
            }

            ws.delete().unwrap();

            assert_eq!(std::fs::read_to_string(personal_directory.join("notes.txt")).unwrap(), "Keep these notes");
            assert_eq!(std::fs::read_to_string(&personal_file).unwrap(), "Keep this file");
            assert_eq!(std::fs::read_to_string(root.join("index.html")).unwrap(), "Keep my app");
            assert!(!generated.exists(), "a backup known to have been generated by this project is removed");
            assert!(!root.join(MANIFEST).exists());
        }
    }

    #[test]
    fn deleting_a_project_preserves_a_replacement_after_backup_discovery() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Site", "A website", TargetRef::None).unwrap();
        let root = ws.root.clone();
        std::fs::write(root.join(crate::feed::FEED_FILE), b"damaged conversation").unwrap();
        crate::feed::save(&root, &mut crate::feed::load(&ws)).unwrap();
        let backup = root.join("feed.json.broken");
        let replacement = root.join("personal-copy");
        std::fs::write(&replacement, b"Keep the personal replacement").unwrap();

        let result = ws.delete_after_discovery(|selected| {
            assert_eq!(selected, std::slice::from_ref(&backup));
            std::fs::rename(&replacement, &backup).unwrap();
        });

        assert!(result.is_err(), "deletion must stop after discovering that the selected entry was replaced");
        assert_eq!(std::fs::read(&backup).unwrap(), b"Keep the personal replacement");
        assert!(root.join(MANIFEST).is_file(), "a failed deletion leaves the project available for recovery");
    }

    #[test]
    fn duplicating_a_legacy_project_keeps_personal_backup_named_entries() {
        for recover in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let original = directory.path().join("original");
            std::fs::create_dir(&original).unwrap();
            std::fs::write(original.join("index.html"), "Keep my app").unwrap();
            let ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
            std::fs::create_dir(original.join("feed.json.broken.1")).unwrap();
            std::fs::write(original.join("feed.json.broken.1/notes.txt"), "Keep these notes").unwrap();
            std::fs::write(original.join("feed.json.broken.2"), "Keep this file").unwrap();
            if recover {
                std::fs::write(original.join(crate::feed::FEED_FILE), b"damaged conversation").unwrap();
                crate::feed::save(&original, &mut crate::feed::load(&ws)).unwrap();
            }

            let copy = ws.duplicate("Copy").unwrap();

            assert_eq!(std::fs::read(copy.app_dir().join("feed.json.broken.1/notes.txt")).unwrap(), b"Keep these notes");
            assert_eq!(std::fs::read(copy.app_dir().join("feed.json.broken.2")).unwrap(), b"Keep this file");
            assert_eq!(std::fs::read(copy.app_dir().join("index.html")).unwrap(), b"Keep my app");
            assert!(!copy.app_dir().join("feed.json.broken").exists(), "a verified generated backup is not part of the copied app");
            assert!(!copy.app_dir().join("inbox").exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_backup_ownership_read_error_does_not_create_a_duplicate() {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("index.html"), "Keep my app").unwrap();
        let ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        std::fs::write(original.join(crate::feed::FEED_FILE), b"damaged conversation").unwrap();
        crate::feed::save(&original, &mut crate::feed::load(&ws)).unwrap();
        let receipt = original.join("inbox/feed-backups/feed.json.broken.json");
        let permissions = std::fs::metadata(&receipt).unwrap().permissions();
        std::fs::set_permissions(&receipt, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Privileged users can still read the file; restore permissions before any assertion.
        let result = std::fs::read(&receipt).is_err().then(|| ws.duplicate("Copy"));
        std::fs::set_permissions(&receipt, permissions).unwrap();
        let Some(result) = result else { return };

        assert!(result.is_err());
        assert!(!directory.path().join("copy").exists(), "ownership discovery must fail before creating a discoverable destination project");
        assert_eq!(std::fs::read(original.join("feed.json.broken")).unwrap(), b"damaged conversation");
        let copy = ws.duplicate("Copy").unwrap();
        assert_eq!(copy.root, directory.path().join("copy"), "retry does not allocate another folder");
        assert_eq!(std::fs::read(copy.app_dir().join("index.html")).unwrap(), b"Keep my app");
        assert!(!copy.app_dir().join("feed.json.broken").exists());
    }

    #[test]
    fn duplication_keeps_personal_replacements_after_backup_discovery() {
        for directory_replacement in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let original = directory.path().join("original");
            std::fs::create_dir(&original).unwrap();
            std::fs::write(original.join("index.html"), "Keep my app").unwrap();
            let ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
            std::fs::write(original.join(crate::feed::FEED_FILE), b"damaged conversation").unwrap();
            crate::feed::save(&original, &mut crate::feed::load(&ws)).unwrap();
            let backup = original.join("feed.json.broken");

            let result = ws.duplicate_after_discovery("Copy", |selected| {
                assert_eq!(selected, std::slice::from_ref(&backup));
                std::fs::remove_file(&backup).unwrap();
                if directory_replacement {
                    std::fs::create_dir(&backup).unwrap();
                    std::fs::write(backup.join("notes.txt"), "Keep the personal replacement").unwrap();
                } else {
                    std::fs::write(&backup, "Keep the personal replacement").unwrap();
                }
            });
            let copy = match result {
                Ok(copy) => copy,
                Err(error) => {
                    assert_eq!(error.code, ErrorCode::FileChanged, "an uncertain changed entry requires a fresh copy");
                    ws.duplicate("Retry").unwrap()
                }
            };

            let relative = if directory_replacement { "feed.json.broken/notes.txt" } else { "feed.json.broken" };
            assert_eq!(std::fs::read(copy.app_dir().join(relative)).unwrap(), b"Keep the personal replacement");
            assert_eq!(std::fs::read(original.join(relative)).unwrap(), b"Keep the personal replacement");
        }
    }

    #[test]
    fn duplication_excludes_an_interrupted_staged_backup_without_a_receipt() {
        for linked in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let original = directory.path().join("original");
            std::fs::create_dir(&original).unwrap();
            std::fs::write(original.join("index.html"), b"Keep my app").unwrap();
            let ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
            let pending = original.join("inbox/feed-backups/pending");
            std::fs::create_dir_all(&pending).unwrap();
            let staged = pending.join("feed.json.broken");
            let backup = original.join("feed.json.broken");
            let bytes = b"Unfinished recovery publication";
            std::fs::write(&staged, bytes).unwrap();
            let receipt = serde_json::json!({
                "format": "craft-anything.feed-backup",
                "version": 1,
                "backup": "feed.json.broken",
                "length": bytes.len(),
                "sha256": craft_anything_evidence::sha256_hex(bytes)
            });
            std::fs::write(pending.join("feed.json.broken.json"), serde_json::to_vec(&receipt).unwrap()).unwrap();
            if linked {
                std::fs::hard_link(&staged, &backup).unwrap();
            } else {
                std::fs::write(&backup, b"Unfinished recovery publication").unwrap();
            }

            let copy = ws.duplicate("Copy").unwrap();

            assert_eq!(std::fs::read(copy.app_dir().join("index.html")).unwrap(), b"Keep my app");
            if linked {
                assert!(!copy.app_dir().join("feed.json.broken").exists(), "the private staging identity survives an exited publisher");
            } else {
                assert_eq!(
                    std::fs::read(copy.app_dir().join("feed.json.broken")).unwrap(),
                    b"Unfinished recovery publication",
                    "equal personal bytes are not the staged file"
                );
            }
            assert_eq!(std::fs::read(&backup).unwrap(), b"Unfinished recovery publication");
            assert!(!original.join("inbox/feed-backups/feed.json.broken.json").exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn duplication_reads_a_non_writable_legacy_source_without_creating_an_inbox() {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("index.html"), b"Keep my app").unwrap();
        let mut ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        // Model a legacy project created before initial inbox reservations.
        ws.manifest.generation = None;
        std::fs::write(original.join(MANIFEST), serde_json::to_vec_pretty(&ws.manifest).unwrap()).unwrap();
        std::fs::remove_file(ws.inbox_dir().join(crate::feed::PROJECT_LOCK_FILE)).unwrap();
        std::fs::remove_dir(ws.inbox_dir()).unwrap();
        assert!(!ws.inbox_dir().exists());
        std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o555)).unwrap();
        let probe = original.join("permission-probe");
        let denied = match std::fs::write(&probe, b"") {
            Err(error) => error.kind() == std::io::ErrorKind::PermissionDenied,
            Ok(()) => {
                std::fs::remove_file(&probe).unwrap();
                false
            }
        };
        let copied = ws.duplicate("Copy");
        std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o755)).unwrap();
        if !denied {
            return; // Privileged callers can still use the coordinated path.
        }
        let copy = copied.unwrap();
        assert_eq!(std::fs::read(copy.app_dir().join("index.html")).unwrap(), b"Keep my app");
        assert!(!ws.inbox_dir().exists(), "a read-only copy does not write source coordination records");
    }

    #[cfg(unix)]
    #[test]
    fn duplication_uses_denied_access_instead_of_owner_write_bits() {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("index.html"), b"Keep my app").unwrap();
        let mut ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        // Model a legacy project created before initial inbox reservations.
        ws.manifest.generation = None;
        std::fs::write(original.join(MANIFEST), serde_json::to_vec_pretty(&ws.manifest).unwrap()).unwrap();
        std::fs::remove_file(ws.inbox_dir().join(crate::feed::PROJECT_LOCK_FILE)).unwrap();
        std::fs::remove_dir(ws.inbox_dir()).unwrap();
        std::fs::write(original.join("feed.json.broken"), b"A personal file with a reserved name").unwrap();
        std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!std::fs::metadata(&original).unwrap().permissions().readonly());
        // Inject the actual lock operation's result, as for a readable directory owned by
        // another user. This also exercises the fallback when the tests run as root.
        let denied = ToolError::io(ws.inbox_dir().display(), std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        let source = ws.copy_source_after_lock(Err(denied)).unwrap();

        let copy = ws.duplicate_from_source("Copy", source, |_| {}, |_| {}).unwrap();

        assert_eq!(std::fs::read(copy.app_dir().join("index.html")).unwrap(), b"Keep my app");
        assert_eq!(std::fs::read(copy.app_dir().join("feed.json.broken")).unwrap(), b"A personal file with a reserved name");
        assert!(!ws.inbox_dir().exists(), "the permission fallback does not write source coordination records");
    }

    #[test]
    fn duplication_reads_a_legacy_source_on_a_read_only_filesystem() {
        for in_place in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let original = directory.path().join("original");
            let mut ws = if in_place {
                std::fs::create_dir(&original).unwrap();
                std::fs::write(original.join("index.html"), b"Keep my app").unwrap();
                Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap()
            } else {
                let ws = Workspace::create(directory.path(), "Original", "A website", TargetRef::None).unwrap();
                std::fs::create_dir(ws.app_dir()).unwrap();
                std::fs::write(ws.app_dir().join("index.html"), b"Keep my app").unwrap();
                ws
            };
            // A legacy project has no coordination directory before its first feed write.
            ws.manifest.generation = None;
            std::fs::write(ws.root.join(MANIFEST), serde_json::to_vec_pretty(&ws.manifest).unwrap()).unwrap();
            std::fs::remove_file(ws.inbox_dir().join(crate::feed::PROJECT_LOCK_FILE)).unwrap();
            std::fs::remove_dir(ws.inbox_dir()).unwrap();
            let denied = || ToolError::io(ws.inbox_dir().display(), std::io::Error::from(std::io::ErrorKind::ReadOnlyFilesystem));
            let source = ws.copy_source_after_lock(Err(denied())).unwrap();

            let copy = ws.duplicate_from_source("Copy", source, |_| {}, |_| {}).unwrap();

            assert_eq!(std::fs::read(copy.app_dir().join("index.html")).unwrap(), b"Keep my app");
            assert!(!ws.inbox_dir().exists(), "the read-only fallback creates no coordination entries");
            let unrelated = ToolError::io(ws.inbox_dir().display(), std::io::Error::from(std::io::ErrorKind::Other));
            assert!(matches!(ws.copy_source_after_lock(Err(unrelated)), Err(error) if error.code == ErrorCode::FileError));
            std::fs::create_dir(ws.inbox_dir()).unwrap();
            let source = ws.copy_source_after_lock(Err(denied())).unwrap();
            let copy = ws.duplicate_from_source("Copy with inbox", source, |_| {}, |_| {}).unwrap();
            assert_eq!(std::fs::read(copy.app_dir().join("index.html")).unwrap(), b"Keep my app");
            assert!(ws.inbox_dir().is_dir());
            assert!(!ws.inbox_dir().join(crate::feed::PROJECT_LOCK_FILE).exists());
        }
    }

    #[test]
    fn duplication_reads_an_existing_coordination_file_without_write_access() {
        for in_place in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let original = directory.path().join("original");
            let ws = if in_place {
                std::fs::create_dir(&original).unwrap();
                std::fs::write(original.join("index.html"), b"Keep my app").unwrap();
                Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap()
            } else {
                let ws = Workspace::create(directory.path(), "Original", "A website", TargetRef::None).unwrap();
                std::fs::create_dir(ws.app_dir()).unwrap();
                std::fs::write(ws.app_dir().join("index.html"), b"Keep my app").unwrap();
                ws
            };
            drop(crate::feed::lock_project(&ws.root).unwrap());
            let path = ws.inbox_dir().join(crate::feed::PROJECT_LOCK_FILE);
            let identity = FileHandle::from_path(&path).unwrap();
            let manifest = std::fs::read(ws.root.join(MANIFEST)).unwrap();
            for denial in [std::io::ErrorKind::PermissionDenied, std::io::ErrorKind::ReadOnlyFilesystem] {
                // Exercise the actual denied-write fallback even for privileged test runners.
                let denied = ToolError::io(path.display(), std::io::Error::from(denial));
                let source = ws.copy_source_after_lock(Err(denied)).unwrap();
                let copy = ws.duplicate_from_source("Copy", source, |_| {}, |_| {}).unwrap();

                assert_eq!(std::fs::read(copy.app_dir().join("index.html")).unwrap(), b"Keep my app");
                assert_eq!(std::fs::read(ws.root.join(MANIFEST)).unwrap(), manifest);
                assert_eq!(identity, FileHandle::from_path(&path).unwrap());
                assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
            }
        }
    }

    #[test]
    fn duplication_reads_a_legacy_inbox_without_creating_a_coordination_file() {
        for (in_place, legacy) in [(false, false), (false, true), (true, false), (true, true)] {
            for denial in [std::io::ErrorKind::PermissionDenied, std::io::ErrorKind::ReadOnlyFilesystem] {
                let directory = tempfile::tempdir().unwrap();
                let original = directory.path().join("original");
                let mut ws = if in_place {
                    std::fs::create_dir(&original).unwrap();
                    std::fs::write(original.join("index.html"), b"Keep my app").unwrap();
                    Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap()
                } else {
                    let ws = Workspace::create(directory.path(), "Original", "A website", TargetRef::None).unwrap();
                    std::fs::create_dir(ws.app_dir()).unwrap();
                    std::fs::write(ws.app_dir().join("index.html"), b"Keep my app").unwrap();
                    ws
                };
                if legacy {
                    ws.manifest.generation = None;
                }
                let manifest = serde_json::to_vec_pretty(&ws.manifest).unwrap();
                std::fs::write(ws.root.join(MANIFEST), &manifest).unwrap();
                let inbox = ws.inbox_dir();
                let lock = inbox.join(crate::feed::PROJECT_LOCK_FILE);
                std::fs::remove_file(&lock).unwrap();
                std::fs::write(inbox.join("question.json"), b"Keep this original question").unwrap();
                let identity = FileHandle::from_path(&inbox).unwrap();
                let denied = ToolError::io(lock.display(), std::io::Error::from(denial));
                let source = ws.copy_source_after_lock(Err(denied)).unwrap();

                let copy = ws.duplicate_from_source("Copy", source, |_| {}, |_| {}).unwrap();

                assert_eq!(std::fs::read(copy.app_dir().join("index.html")).unwrap(), b"Keep my app");
                assert!(!copy.app_dir().join("inbox").exists());
                assert!(!copy.inbox_dir().join("question.json").exists());
                assert_eq!(std::fs::read(inbox.join("question.json")).unwrap(), b"Keep this original question");
                assert_eq!(std::fs::read(ws.root.join(MANIFEST)).unwrap(), manifest);
                assert_eq!(FileHandle::from_path(&inbox).unwrap(), identity);
                assert!(!lock.exists(), "a read-only copy must not create source coordination records");

                let displaced = ws.root.join("old-inbox");
                std::fs::rename(&inbox, &displaced).unwrap();
                std::fs::write(&inbox, b"Keep this non-directory entry").unwrap();
                let denied = ToolError::io(lock.display(), std::io::Error::from(denial));
                assert!(matches!(ws.copy_source_after_lock(Err(denied)), Err(error) if error.code == ErrorCode::FileChanged));
                assert_eq!(std::fs::read(&inbox).unwrap(), b"Keep this non-directory entry");
                assert_eq!(std::fs::read(displaced.join("question.json")).unwrap(), b"Keep this original question");
            }
        }
    }

    #[test]
    fn an_uncoordinated_legacy_inbox_copy_rejects_publishers_and_identity_changes() {
        for change in ["publisher-live", "publisher-finished", "inbox", "generation", "timestamp"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("original");
            std::fs::create_dir(&root).unwrap();
            std::fs::write(root.join("index.html"), b"Keep my app").unwrap();
            let ws = Workspace::create_at(&root, "Site", "Test", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
            std::fs::remove_file(ws.inbox_dir().join(crate::feed::PROJECT_LOCK_FILE)).unwrap();
            std::fs::write(ws.inbox_dir().join("question.json"), b"Keep this question").unwrap();
            let denied = ToolError::io(ws.inbox_dir().display(), std::io::Error::from(std::io::ErrorKind::PermissionDenied));
            let source = ws.copy_source_after_lock(Err(denied)).unwrap();
            let publisher = std::cell::RefCell::new(None);

            let copied = ws.duplicate_from_source(
                "Copy",
                source,
                |_| match change {
                    "publisher-live" => {
                        publisher.replace(Some(crate::feed::lock_project(&root).unwrap()));
                    }
                    "publisher-finished" => drop(crate::feed::lock_project(&root).unwrap()),
                    "inbox" => {
                        std::fs::rename(ws.inbox_dir(), root.join("old-inbox")).unwrap();
                        std::fs::create_dir(ws.inbox_dir()).unwrap();
                    }
                    _ => {
                        let mut manifest = ws.manifest.clone();
                        if change == "generation" {
                            manifest.generation = Manifest::new("Next", "Test", TargetRef::None).unwrap().generation;
                        } else {
                            manifest.created_ms += 1;
                        }
                        std::fs::write(root.join(MANIFEST), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
                    }
                },
                |_| {},
            );

            assert_eq!(copied.unwrap_err().code, ErrorCode::FileChanged, "{change}");
            assert!(!directory.path().join("copy").join(MANIFEST).exists(), "{change}");
            let inbox = if change == "inbox" { root.join("old-inbox") } else { ws.inbox_dir() };
            assert_eq!(std::fs::read(inbox.join("question.json")).unwrap(), b"Keep this question");
            assert_eq!(std::fs::read(root.join("index.html")).unwrap(), b"Keep my app");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_legacy_copy_rejects_linked_inboxes_and_replaced_roots() {
        for change in ["inbox-link", "root"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("original");
            std::fs::create_dir(&root).unwrap();
            std::fs::write(root.join("index.html"), b"Keep the original app").unwrap();
            let ws = Workspace::create_at(&root, "Site", "Test", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
            std::fs::remove_file(ws.inbox_dir().join(crate::feed::PROJECT_LOCK_FILE)).unwrap();
            std::fs::write(ws.inbox_dir().join("question.json"), b"Keep the original question").unwrap();
            let denied = || ToolError::io(ws.inbox_dir().display(), std::io::Error::from(std::io::ErrorKind::PermissionDenied));
            let displaced = directory.path().join("displaced");
            if change == "inbox-link" {
                std::fs::rename(ws.inbox_dir(), &displaced).unwrap();
                std::os::unix::fs::symlink(&displaced, ws.inbox_dir()).unwrap();
                assert!(matches!(ws.copy_source_after_lock(Err(denied())), Err(error) if error.code == ErrorCode::FileChanged));
                assert_eq!(std::fs::read(displaced.join("question.json")).unwrap(), b"Keep the original question");
                assert!(!displaced.join(crate::feed::PROJECT_LOCK_FILE).exists());
            } else {
                let source = ws.copy_source_after_lock(Err(denied())).unwrap();
                let copied = ws.duplicate_from_source(
                    "Copy",
                    source,
                    |_| {
                        std::fs::rename(&root, &displaced).unwrap();
                        std::fs::create_dir(&root).unwrap();
                        std::fs::create_dir(ws.inbox_dir()).unwrap();
                        std::fs::copy(displaced.join(MANIFEST), root.join(MANIFEST)).unwrap();
                        std::fs::write(root.join("index.html"), b"Keep the replacement app").unwrap();
                    },
                    |_| {},
                );
                assert_eq!(copied.unwrap_err().code, ErrorCode::FileChanged);
                assert!(!directory.path().join("copy").join(MANIFEST).exists());
                assert_eq!(std::fs::read(displaced.join("inbox/question.json")).unwrap(), b"Keep the original question");
                assert_eq!(std::fs::read(root.join("index.html")).unwrap(), b"Keep the replacement app");
                assert!(!ws.inbox_dir().join(crate::feed::PROJECT_LOCK_FILE).exists());
            }
        }
    }

    #[test]
    fn an_unlocked_copy_rejects_a_publisher_that_starts_after_discovery() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("index.html"), b"Keep my app").unwrap();
        let mut ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        // Model a legacy project created before initial inbox reservations.
        ws.manifest.generation = None;
        std::fs::write(original.join(MANIFEST), serde_json::to_vec_pretty(&ws.manifest).unwrap()).unwrap();
        std::fs::remove_file(ws.inbox_dir().join(crate::feed::PROJECT_LOCK_FILE)).unwrap();
        std::fs::remove_dir(ws.inbox_dir()).unwrap();
        let denied = ToolError::io(ws.inbox_dir().display(), std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        let source = ws.copy_source_after_lock(Err(denied)).unwrap();
        let publisher = std::cell::RefCell::new(None);

        let result = ws.duplicate_from_source(
            "Copy",
            source,
            |_| {
                publisher.replace(Some(crate::feed::lock_project(&original).unwrap()));
            },
            |_| {},
        );

        assert_eq!(result.unwrap_err().code, ErrorCode::FileChanged);
        assert!(!directory.path().join("copy").exists(), "an uncoordinated copy cannot publish a project after a publisher starts");
        assert!(publisher.borrow().is_some(), "the publisher remains active through the final source check");
        assert!(ws.inbox_dir().is_dir());
        assert_eq!(std::fs::read(original.join("index.html")).unwrap(), b"Keep my app");
        let denied = ToolError::io(ws.inbox_dir().display(), std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert!(matches!(ws.copy_source_after_lock(Err(denied)), Err(error) if error.code == ErrorCode::TimedOut));
    }

    #[test]
    fn reused_folders_and_timestamps_reject_old_conversations_and_archives() {
        for legacy in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let original = directory.path().join("original");
            std::fs::create_dir(&original).unwrap();
            std::fs::write(original.join("index.html"), b"Keep my app").unwrap();
            let mut ws = Workspace::create_at(&original, "Old", "Old request", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
            if legacy {
                let mut document = serde_json::to_value(&ws.manifest).unwrap();
                document["version"] = Value::from(5);
                document.as_object_mut().unwrap().remove("generation");
                std::fs::write(original.join(MANIFEST), serde_json::to_vec(&document).unwrap()).unwrap();
                ws.reload().unwrap();
            }
            let previous_directory = FileHandle::from_path(&original).unwrap();
            let created_ms = ws.manifest.created_ms;
            let generation = ws.manifest.generation.clone();
            let mut old = crate::feed::load(&ws);
            let snapshot = crate::feed::RecoverySnapshot::from_loaded(&old, serde_json::json!({}));
            ws.delete().unwrap();
            let mut replacement = Workspace::create_at(&original, "New", "New request", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
            replacement.manifest.created_ms = created_ms;
            // Model a reused timestamp directly; ordinary saves cannot change creation identity.
            std::fs::write(original.join(MANIFEST), serde_json::to_vec_pretty(&replacement.manifest).unwrap()).unwrap();
            assert_eq!(previous_directory, FileHandle::from_path(&original).unwrap(), "in-place deletion keeps the same directory");
            assert_ne!(replacement.manifest.generation, generation);
            let manifest = std::fs::read(original.join(MANIFEST)).unwrap();
            old.push(crate::feed::FeedItem::User(crate::feed::UserItem::Text("Late old reply".into())));

            assert_eq!(crate::feed::save(&original, &mut old).unwrap_err().code, ErrorCode::FileChanged);
            assert_eq!(crate::feed::write_recovery_snapshot(&original, &snapshot).unwrap_err().code, ErrorCode::FileChanged);

            assert!(!original.join(crate::feed::FEED_FILE).exists());
            assert!(!original.join("inbox").join(crate::feed::RECOVERY_ARCHIVE_DIR).exists());
            assert_eq!(std::fs::read(original.join(MANIFEST)).unwrap(), manifest);
            assert_eq!(std::fs::read(original.join("index.html")).unwrap(), b"Keep my app");
        }
    }

    #[test]
    fn a_stale_workspace_cannot_delete_a_replacement_project() {
        let directory = tempfile::tempdir().unwrap();
        let old = Workspace::create(directory.path(), "Original", "Old request", TargetRef::None).unwrap();
        let root = old.root.clone();
        std::fs::write(root.join("personal.txt"), b"Keep this folder").unwrap();
        old.clone().delete().unwrap();
        std::fs::create_dir(root.join("app")).unwrap();
        std::fs::write(root.join("app/index.html"), b"This is now a personal app folder").unwrap();
        let mut replacement = Workspace::create_at(&root, "Replacement", "New request", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        replacement.manifest.created_ms = old.manifest.created_ms;
        std::fs::write(root.join(MANIFEST), serde_json::to_vec_pretty(&replacement.manifest).unwrap()).unwrap();
        assert!(!old.manifest.app_in_root);
        assert!(replacement.manifest.app_in_root);
        crate::feed::save(&root, &mut crate::feed::load(&replacement)).unwrap();
        let manifest = std::fs::read(root.join(MANIFEST)).unwrap();
        let feed = std::fs::read(root.join(crate::feed::FEED_FILE)).unwrap();

        assert_eq!(old.delete().unwrap_err().code, ErrorCode::FileChanged);

        assert_eq!(std::fs::read(root.join(MANIFEST)).unwrap(), manifest);
        assert_eq!(std::fs::read(root.join(crate::feed::FEED_FILE)).unwrap(), feed);
        assert_eq!(std::fs::read(root.join("app/index.html")).unwrap(), b"This is now a personal app folder");
        assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep this folder");
        assert!(replacement.inbox_dir().is_dir());
    }

    #[test]
    fn a_stale_ordinary_workspace_cannot_duplicate_a_replacement_project() {
        let directory = tempfile::tempdir().unwrap();
        let old = Workspace::create(directory.path(), "Original", "Old request", TargetRef::None).unwrap();
        let root = old.root.clone();
        std::fs::write(root.join("personal.txt"), b"Keep this folder").unwrap();
        old.clone().delete().unwrap();
        std::fs::create_dir(root.join("app")).unwrap();
        std::fs::write(root.join("app/index.html"), b"This is now a personal app folder").unwrap();
        let mut replacement = Workspace::create_at(&root, "Replacement", "New request", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        replacement.manifest.created_ms = old.manifest.created_ms;
        std::fs::write(root.join(MANIFEST), serde_json::to_vec_pretty(&replacement.manifest).unwrap()).unwrap();
        let manifest = std::fs::read(root.join(MANIFEST)).unwrap();
        assert!(!old.manifest.app_in_root);
        assert_ne!(old.manifest.generation, replacement.manifest.generation);

        assert_eq!(old.duplicate("Copy").unwrap_err().code, ErrorCode::FileChanged);

        assert!(!directory.path().join("copy").exists(), "a stale source cannot publish the replacement's app with old settings");
        assert_eq!(std::fs::read(root.join(MANIFEST)).unwrap(), manifest);
        assert_eq!(std::fs::read(root.join("app/index.html")).unwrap(), b"This is now a personal app folder");
    }

    #[test]
    fn deleted_in_place_projects_reject_loaded_feed_writers() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("index.html"), b"Keep my app").unwrap();
        let ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        let mut stale = crate::feed::load(&ws);
        ws.delete().unwrap();
        stale.push(crate::feed::FeedItem::User(crate::feed::UserItem::Text("Late conversation".into())));

        assert!(crate::feed::save(&original, &mut stale).is_err());

        assert!(!original.join(crate::feed::FEED_FILE).exists());
        assert!(!original.join("inbox").exists());
        assert_eq!(std::fs::read(original.join("index.html")).unwrap(), b"Keep my app");
    }

    #[test]
    fn inbox_payload_cleanup_failure_keeps_project_deletion_retryable() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("index.html"), b"Keep my app").unwrap();
        let ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        let payload = ws.inbox_dir().join("unavailable-payload");
        std::fs::create_dir(&payload).unwrap();
        std::fs::write(payload.join("partial"), b"The unfinished inbox write").unwrap();
        drop(crate::feed::lock_project(&original).unwrap());
        let lock = ws.inbox_dir().join(crate::feed::PROJECT_LOCK_FILE);
        let identity = FileHandle::from_path(&lock).unwrap();
        let manifest = std::fs::read(original.join(MANIFEST)).unwrap();

        let result = ws.delete_with_remover(
            |_| {},
            |path| {
                if payload.starts_with(path) {
                    return Err(ToolError::io(path.display(), std::io::Error::from(std::io::ErrorKind::PermissionDenied)));
                }
                let removed = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
                match removed {
                    Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(ToolError::io(path.display(), error)),
                    _ => Ok(()),
                }
            },
        );

        assert_eq!(result.unwrap_err().code, ErrorCode::NoPermission);
        assert_eq!(std::fs::read(original.join(MANIFEST)).unwrap(), manifest);
        assert_eq!(identity, FileHandle::from_path(&lock).unwrap());
        assert_eq!(std::fs::read(payload.join("partial")).unwrap(), b"The unfinished inbox write");
        Workspace::open(&original).unwrap().delete().unwrap();
        assert!(!original.join(MANIFEST).exists());
        assert!(!original.join("inbox").exists());
        assert_eq!(std::fs::read(original.join("index.html")).unwrap(), b"Keep my app");
    }

    #[test]
    fn deletion_preclaim_failures_keep_the_manifest_and_inbox_retryable() {
        for failure in [std::io::ErrorKind::PermissionDenied, std::io::ErrorKind::StorageFull] {
            for fail_reservation in [true, false] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path().join("selected");
                std::fs::create_dir(&root).unwrap();
                std::fs::write(root.join("index.html"), b"Keep my app").unwrap();
                let ws = Workspace::create_at(&root, "Original", "Test", TargetRef::None, Some(ExistingFiles::StartFrom)).unwrap();
                std::fs::create_dir_all(ws.app_dir()).unwrap();
                let build = ws.app_dir().join("generated.html");
                std::fs::write(&build, b"Keep the generated build until cleanup can start").unwrap();
                let mut conversation = crate::feed::load(&ws);
                conversation.push(crate::feed::FeedItem::User(crate::feed::UserItem::Text("Keep this conversation until cleanup can start".into())));
                crate::feed::save(&root, &mut conversation).unwrap();
                let feed_path = root.join(crate::feed::FEED_FILE);
                let feed_bytes = std::fs::read(&feed_path).unwrap();
                let manifest_path = root.join(MANIFEST);
                let manifest = std::fs::read(&manifest_path).unwrap();
                let inbox = ws.inbox_dir();
                let lock = inbox.join(crate::feed::PROJECT_LOCK_FILE);
                let identity = FileHandle::from_path(&lock).unwrap();
                let result = ws.delete_with_cleanup(
                    |_| {},
                    |path| {
                        if !fail_reservation && path == manifest_path {
                            return Err(ToolError::io(path.display(), std::io::Error::from(failure)));
                        }
                        let removed = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
                        match removed {
                            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(ToolError::io(path.display(), error)),
                            _ => Ok(()),
                        }
                    },
                    || {},
                    &|parent| {
                        if fail_reservation {
                            Err(ToolError::io(parent.display(), std::io::Error::from(failure)))
                        } else {
                            CopyDestination::cleanup_directory(parent)
                        }
                    },
                );

                let error = result.unwrap_err();
                assert_eq!(error.code, if failure == std::io::ErrorKind::PermissionDenied { ErrorCode::NoPermission } else { ErrorCode::FileError });
                assert_eq!(std::fs::read(&manifest_path).unwrap(), manifest);
                assert_eq!(FileHandle::from_path(&lock).unwrap(), identity);
                assert!(error.for_people().contains(&inbox.display().to_string()));
                assert!(!std::fs::read_dir(&root).unwrap().any(|entry| entry.unwrap().file_name().to_string_lossy().starts_with(".craft-copy-cleanup-")));
                assert_eq!(std::fs::read(root.join("index.html")).unwrap(), b"Keep my app");
                if fail_reservation {
                    assert_eq!(std::fs::read(&build).unwrap(), b"Keep the generated build until cleanup can start");
                    assert_eq!(std::fs::read(&feed_path).unwrap(), feed_bytes);
                }
                Workspace::open(&root).unwrap().delete().unwrap();
                assert!(!manifest_path.exists());
                assert!(!inbox.exists());
                assert!(!build.exists());
                assert!(!feed_path.exists());
                assert_eq!(std::fs::read(root.join("index.html")).unwrap(), b"Keep my app");
            }
        }
    }

    #[test]
    fn deletion_reuses_reserved_cleanup_space_after_removing_the_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("selected");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("index.html"), b"Keep my app").unwrap();
        let ws = Workspace::create_at(&root, "Original", "Test", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        let manifest = root.join(MANIFEST);
        let allocations = std::cell::Cell::new(0);
        let late_allocations = std::cell::Cell::new(0);
        ws.delete_with_cleanup(
            |_| {},
            |path| {
                let removed = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
                match removed {
                    Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(ToolError::io(path.display(), error)),
                    _ => Ok(()),
                }
            },
            || {},
            &|parent| {
                if !manifest.exists() {
                    late_allocations.set(late_allocations.get() + 1);
                    return Err(ToolError::io(parent.display(), std::io::Error::from(std::io::ErrorKind::StorageFull)));
                }
                allocations.set(allocations.get() + 1);
                CopyDestination::cleanup_directory(parent)
            },
        )
        .unwrap();

        assert_eq!(allocations.get(), 1);
        assert_eq!(late_allocations.get(), 0, "coordination cleanup does not allocate after deletion becomes undiscoverable");
        assert!(!manifest.exists());
        assert!(!root.join("inbox").exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        assert_eq!(std::fs::read(root.join("index.html")).unwrap(), b"Keep my app");
    }

    #[test]
    fn deletion_keeps_lock_file_aliases_until_the_manifest_is_removed() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Original", "A website", TargetRef::None).unwrap();
        drop(crate::feed::lock_project(&ws.root).unwrap());
        let manifest = ws.root.join(MANIFEST);
        let inbox = ws.inbox_dir();
        let alias = inbox.join("another-lock-name");
        std::fs::hard_link(inbox.join(crate::feed::PROJECT_LOCK_FILE), &alias).unwrap();

        ws.delete_with_remover(
            |_| {},
            |path| {
                if path == alias || path == inbox {
                    assert!(!manifest.exists(), "another spelling of the held inode is not an ordinary payload");
                }
                let removed = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
                match removed {
                    Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(ToolError::io(path.display(), error)),
                    _ => Ok(()),
                }
            },
        )
        .unwrap();

        assert!(!manifest.exists());
        assert!(!inbox.exists());
    }

    #[cfg(unix)]
    #[test]
    fn deletion_preserves_a_coordination_file_changed_after_locking() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Original", "A website", TargetRef::None).unwrap();
        let root = ws.root.clone();
        let lock = ws.inbox_dir().join(crate::feed::PROJECT_LOCK_FILE);
        let manifest = std::fs::read(root.join(MANIFEST)).unwrap();

        // Advisory Unix locks permit this external in-place mutation. Windows exclusive
        // locks reject writes through a second handle; the replacement control below
        // exercises changed coordination there without writing the locked inode.
        let result = ws.delete_after_discovery(|_| std::fs::write(&lock, b"Keep these unexpected bytes").unwrap());

        assert!(result.is_err());
        assert_eq!(std::fs::read(root.join(MANIFEST)).unwrap(), manifest);
        assert_eq!(std::fs::read(&lock).unwrap(), b"Keep these unexpected bytes");
    }

    #[test]
    fn deletion_preserves_a_replaced_coordination_file_after_locking() {
        for during_allocation in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let ws = Workspace::create(directory.path(), "Original", "A website", TargetRef::None).unwrap();
            let root = ws.root.clone();
            let lock = ws.inbox_dir().join(crate::feed::PROJECT_LOCK_FILE);
            let manifest = std::fs::read(root.join(MANIFEST)).unwrap();
            let identity = FileHandle::from_path(&lock).unwrap();
            let replacement = directory.path().join("replacement.lock");
            let retained = directory.path().join("retained.lock");
            std::fs::write(&replacement, b"Keep these replacement bytes").unwrap();
            let replace_lock = || {
                std::fs::rename(&lock, &retained).unwrap();
                std::fs::rename(&replacement, &lock).unwrap();
            };

            let result = ws.delete_with_cleanup(
                |_| {
                    if !during_allocation {
                        replace_lock();
                    }
                },
                |path| {
                    let removed = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
                    match removed {
                        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(ToolError::io(path.display(), error)),
                        _ => Ok(()),
                    }
                },
                || {},
                &|parent| {
                    let private = CopyDestination::cleanup_directory(parent)?;
                    if during_allocation {
                        replace_lock();
                    }
                    Ok(private)
                },
            );

            assert!(result.is_err());
            assert_eq!(std::fs::read(root.join(MANIFEST)).unwrap(), manifest);
            assert_eq!(std::fs::read(&lock).unwrap(), b"Keep these replacement bytes");
            assert_eq!(FileHandle::from_path(&retained).unwrap(), identity);
            assert_eq!(std::fs::metadata(&retained).unwrap().len(), 0);
            assert!(!std::fs::read_dir(&root).unwrap().any(|entry| entry.unwrap().file_name().to_string_lossy().starts_with(".craft-copy-cleanup-")));
        }
    }

    #[test]
    fn a_failed_initializer_cannot_release_a_deleters_active_inbox() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Original", "A website", TargetRef::None).unwrap();
        let root = ws.root.clone();
        std::fs::write(root.join("personal.txt"), b"Keep this folder").unwrap();
        // A legacy initializer creates the inbox, then deletion gets its coordination lock.
        std::fs::remove_file(ws.inbox_dir().join(crate::feed::PROJECT_LOCK_FILE)).unwrap();
        std::fs::remove_dir(ws.inbox_dir()).unwrap();
        std::fs::create_dir(ws.inbox_dir()).unwrap();
        let mut initializer = Some((FileHandle::from_path(&root).unwrap(), FileHandle::from_path(ws.inbox_dir()).unwrap()));

        ws.delete_with_remover(
            |_| {},
            |path| {
                let removed = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
                if let Err(error) = removed
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    return Err(ToolError::io(path.display(), error));
                }
                if path == root.join(MANIFEST) {
                    let expected = FileHandle::from_path(root.join("inbox")).unwrap();
                    let (directory, inbox) = initializer.take().unwrap();
                    let error = release_created_inbox(&root, directory, inbox, None, None).unwrap_err();
                    assert_eq!(expected, FileHandle::from_path(root.join("inbox")).unwrap(), "a busy deleter keeps the public reservation");
                    assert_eq!(error.code, ErrorCode::TimedOut);
                    assert!(Workspace::create_at(&root, "Too soon", "New request", TargetRef::None, Some(ExistingFiles::Edit)).is_err());
                }
                Ok(())
            },
        )
        .unwrap();

        Workspace::create_at(&root, "Replacement", "New request", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep this folder");
    }

    #[test]
    fn deletion_never_returns_to_the_public_inbox_after_claiming_it() {
        for external_publication in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let ws = Workspace::create(directory.path(), "Original", "A website", TargetRef::None).unwrap();
            let root = ws.root.clone();
            std::fs::write(root.join("personal.txt"), b"Keep this folder").unwrap();
            let mut replacement = None;

            ws.delete_with_hooks(
                |_| {},
                |path| {
                    let removed = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
                    match removed {
                        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(ToolError::io(path.display(), error)),
                        _ => Ok(()),
                    }
                },
                || {
                    assert!(Workspace::create_at(&root, "Too soon", "New request", TargetRef::None, Some(ExistingFiles::Edit)).is_err());
                    assert!(!root.join(MANIFEST).exists());
                    assert!(!root.join("inbox").exists());
                    if external_publication {
                        // An older build or external writer can still publish after the
                        // public name is released. Install an independently created project
                        // so the old cleanup must preserve a real new generation and lock.
                        let mut prepared = Workspace::create(directory.path(), "Replacement", "New request", TargetRef::None).unwrap();
                        prepared.manifest.app_in_root = true;
                        prepared.save().unwrap();
                        std::fs::write(prepared.inbox_dir().join("new-request"), b"Keep the new request").unwrap();
                        std::fs::rename(prepared.inbox_dir(), root.join("inbox")).unwrap();
                        std::fs::rename(prepared.root.join(MANIFEST), root.join(MANIFEST)).unwrap();
                        let lock = std::fs::OpenOptions::new().read(true).write(true).open(root.join("inbox/feed.lock")).unwrap();
                        lock.try_lock().unwrap();
                        replacement = Some((
                            std::fs::read(root.join(MANIFEST)).unwrap(),
                            FileHandle::from_path(root.join("inbox")).unwrap(),
                            FileHandle::from_path(root.join("inbox/feed.lock")).unwrap(),
                            lock,
                        ));
                    }
                },
            )
            .unwrap();

            if let Some((manifest, inbox, lock, _held)) = replacement {
                assert_eq!(std::fs::read(root.join(MANIFEST)).unwrap(), manifest);
                assert_eq!(FileHandle::from_path(root.join("inbox")).unwrap(), inbox);
                assert_eq!(FileHandle::from_path(root.join("inbox/feed.lock")).unwrap(), lock);
                assert_eq!(std::fs::read(root.join("inbox/new-request")).unwrap(), b"Keep the new request");
                let contender = std::fs::OpenOptions::new().read(true).write(true).open(root.join("inbox/feed.lock")).unwrap();
                assert!(matches!(contender.try_lock(), Err(std::fs::TryLockError::WouldBlock)));
            } else {
                Workspace::create_at(&root, "Retry", "New request", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
            }
            assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep this folder");
        }
    }

    #[test]
    fn initial_creators_hold_the_coordination_lock_through_publication_and_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        for publish in [false, true] {
            let root = directory.path().join(if publish { "published" } else { "failed" });
            std::fs::create_dir(&root).unwrap();
            let result = Workspace::create_at_with_publisher(&root, "Project", "Test", TargetRef::None, None, |ws, creation| {
                let file = std::fs::OpenOptions::new().read(true).write(true).open(ws.inbox_dir().join("feed.lock")).unwrap();
                assert!(matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)));
                if publish { ws.publish_manifest(creation) } else { Err(ToolError::internal("Initial publication failed")) }
            });
            if publish {
                let ws = result.unwrap();
                assert!(root.join(MANIFEST).is_file());
                assert_eq!(std::fs::metadata(root.join("inbox/feed.lock")).unwrap().len(), 0);
                drop(crate::feed::lock_project(&ws.root).unwrap());
            } else {
                assert!(result.is_err());
                assert!(!root.join(MANIFEST).exists());
                assert!(!root.join("inbox").exists());
                Workspace::create_at(&root, "Retry", "Test", TargetRef::None, None).unwrap();
            }
        }
    }

    #[test]
    fn failed_creation_lock_acquisition_recovers_only_its_verified_exclusive_inode() {
        for case in ["transient", "busy", "replaced", "foreign"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("selected");
            std::fs::create_dir(&root).unwrap();
            std::fs::write(root.join("personal.txt"), b"Keep the selected folder").unwrap();
            let busy = std::cell::RefCell::new(None);
            let original = std::cell::RefCell::new(None);
            let error = CreationReservation::claim_with_creation_lock(
                &root,
                FileHandle::from_path(&root).unwrap(),
                &CopyDestination::cleanup_directory,
                |root, inbox| {
                    if case == "foreign" {
                        std::fs::File::create_new(root.join("inbox/feed.lock")).unwrap();
                    }
                    crate::feed::lock_creation_using(root, inbox, |_, path, _| {
                        original.replace(Some(FileHandle::from_path(path).unwrap()));
                        if case == "busy" {
                            let other = std::fs::File::options().read(true).write(true).open(path).unwrap();
                            other.try_lock().unwrap();
                            busy.replace(Some(other));
                        } else if case == "replaced" {
                            std::fs::rename(path, root.join("retained.lock")).unwrap();
                            std::fs::write(path, b"Keep the replacement lock").unwrap();
                        }
                        Err(ToolError::io(path.display(), std::io::Error::other("Injected initial lock failure"))
                            .with_remediation("Keep the initial error guidance."))
                    })
                },
            )
            .err()
            .expect("the initial acquisition reports its original failure");

            assert_eq!(error.code, ErrorCode::FileError);
            assert!(error.message.contains("Injected initial lock failure"));
            assert!(error.for_people().contains("Keep the initial error guidance."));
            assert!(!root.join(MANIFEST).exists());
            assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep the selected folder");
            if case == "transient" {
                assert!(!root.join("inbox").exists(), "a recovered original inode releases its failed creation reservation");
                assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
                Workspace::create_at(&root, "Retry", "Test", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
            } else {
                assert!(root.join("inbox").is_dir(), "busy or unconfirmed coordination is preserved at its public path");
                assert!(error.for_people().contains(&root.join("inbox").display().to_string()));
                if case == "replaced" {
                    assert_eq!(std::fs::read(root.join("inbox/feed.lock")).unwrap(), b"Keep the replacement lock");
                    assert_eq!(FileHandle::from_path(root.join("retained.lock")).unwrap(), *original.borrow().as_ref().unwrap());
                } else {
                    assert_eq!(FileHandle::from_path(root.join("inbox/feed.lock")).unwrap(), *original.borrow().as_ref().unwrap());
                    assert_eq!(std::fs::metadata(root.join("inbox/feed.lock")).unwrap().len(), 0);
                }
                assert!(Workspace::create_at(&root, "Other", "Test", TargetRef::None, Some(ExistingFiles::Edit)).is_err());
            }
        }
    }

    #[test]
    fn published_creation_and_copy_return_success_with_a_cleanup_warning() {
        for place in ["managed", "picked", "copy"] {
            let directory = tempfile::tempdir().unwrap();
            let source = Workspace::create(directory.path(), "Source", "A website", TargetRef::None).unwrap();
            std::fs::create_dir(source.app_dir()).unwrap();
            std::fs::write(source.app_dir().join("index.html"), b"Keep the copied app").unwrap();
            let source_manifest = std::fs::read(source.root.join(MANIFEST)).unwrap();
            let selected = directory.path().join("selected");
            std::fs::create_dir(&selected).unwrap();
            std::fs::write(selected.join("personal.txt"), b"Keep the selected files").unwrap();
            let retained = std::cell::RefCell::new(PathBuf::new());
            let warning = std::cell::RefCell::new(None);
            let publish = |ws: &mut Workspace, creation: &CreationReservation| {
                ws.publish_manifest_using(
                    creation,
                    write_atomic,
                    |cleanup, root| {
                        let path = cleanup.empty_path(root)?;
                        retained.replace(path.clone());
                        Err(ToolError::io(path.display(), std::io::Error::from(std::io::ErrorKind::PermissionDenied))
                            .with_remediation("Inspect the cleanup access settings."))
                    },
                    |error| {
                        warning.replace(Some(error.clone()));
                    },
                )
            };
            let result = match place {
                "copy" => source.duplicate_with_publisher(
                    "Copy",
                    source.lock_copy_source().unwrap(),
                    |_| {},
                    |_| panic!("a published copy must not enter rollback"),
                    publish,
                ),
                "picked" => Workspace::create_at_with_publisher(&selected, "Selected", "A website", TargetRef::None, Some(ExistingFiles::Edit), publish),
                _ => Workspace::create_with_publisher(directory.path(), "Managed", "A website", TargetRef::None, publish),
            };
            assert!(result.is_ok(), "a verified published project must remain successful when only empty cleanup fails: {result:?}");
            let ws = result.unwrap();

            let retained = retained.into_inner();
            let warning = warning.into_inner().expect("cleanup failure is separately reported");
            assert!(retained.is_dir());
            assert_eq!(std::fs::read_dir(&retained).unwrap().count(), 0);
            assert!(warning.for_people().contains(&retained.display().to_string()));
            assert!(warning.for_people().contains("Inspect the cleanup access settings."));
            assert!(warning.for_people().contains("manifest was published"));
            assert_eq!(Workspace::open(&ws.root).unwrap().manifest.generation, ws.manifest.generation);
            drop(crate::feed::lock_project(&ws.root).unwrap());
            assert_eq!(std::fs::read(source.root.join(MANIFEST)).unwrap(), source_manifest);
            assert_eq!(std::fs::read(selected.join("personal.txt")).unwrap(), b"Keep the selected files");
            if place == "copy" {
                assert_eq!(std::fs::read(ws.app_dir().join("index.html")).unwrap(), b"Keep the copied app");
            }
        }
    }

    #[test]
    fn publication_identity_changes_and_write_failures_are_not_cleanup_warnings() {
        for case in ["write", "root", "manifest"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("selected");
            std::fs::create_dir(&root).unwrap();
            std::fs::write(root.join("personal.txt"), b"Keep the original files").unwrap();
            let displaced = directory.path().join("displaced");
            let reported = std::cell::Cell::new(false);
            let result = Workspace::create_at_with_publisher(&root, "Selected", "A website", TargetRef::None, Some(ExistingFiles::Edit), |ws, creation| {
                ws.publish_manifest_using(
                    creation,
                    |path, bytes| {
                        if case == "write" {
                            return Err(ToolError::io(path.display(), std::io::Error::from(std::io::ErrorKind::StorageFull)));
                        }
                        write_atomic(path, bytes)
                    },
                    |_, root| {
                        assert_ne!(case, "write", "failed manifest writes cannot reach cleanup or warning reporting");
                        if case == "root" {
                            rename_creation_root_for_replacement(root, &displaced);
                            std::fs::create_dir(root).unwrap();
                            std::fs::write(root.join("personal.txt"), b"Keep the replacement files").unwrap();
                        } else {
                            let replacement = Manifest::new("Replacement", "Another project", TargetRef::None).unwrap();
                            write_atomic(&root.join(MANIFEST), &serde_json::to_vec_pretty(&replacement).unwrap()).unwrap();
                        }
                        Err(ToolError::io(root.display(), std::io::Error::from(std::io::ErrorKind::PermissionDenied)))
                    },
                    |_| reported.set(true),
                )
            });

            assert!(result.is_err(), "an identity or initial write error is not downgraded to a cleanup warning");
            let error = result.unwrap_err();

            assert!(!reported.get(), "an identity or initial write error is not downgraded to a cleanup warning");
            if case == "write" {
                assert_eq!(error.code, ErrorCode::FileError);
                assert!(!root.join(MANIFEST).exists());
                assert!(!root.join("inbox").exists());
                Workspace::create_at(&root, "Retry", "Test", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
            } else {
                assert_eq!(error.code, ErrorCode::FileChanged);
                if case == "root" {
                    assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep the replacement files");
                    assert_eq!(std::fs::read(displaced.join("personal.txt")).unwrap(), b"Keep the original files");
                    assert_eq!(Workspace::open(&displaced).unwrap().manifest.name, "Selected");
                } else {
                    assert_eq!(Workspace::open(&root).unwrap().manifest.name, "Replacement");
                    assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep the original files");
                }
            }
        }
    }

    #[test]
    fn failed_creation_releases_only_same_file_coordination_spellings() {
        let directory = tempfile::tempdir().unwrap();
        for same_file in [false, true] {
            let root = directory.path().join(if same_file { "same-file" } else { "personal-file" });
            std::fs::create_dir(&root).unwrap();
            let alias = root.join("inbox/FEED.LOCK");
            let distinct = std::cell::Cell::new(false);
            let failed = Workspace::create_at_with_publisher(&root, "Project", "Test", TargetRef::None, None, |ws, _creation| {
                if same_file {
                    match std::fs::hard_link(ws.inbox_dir().join("feed.lock"), &alias) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(error) => panic!("create a coordination spelling control: {error}"),
                    }
                } else {
                    match std::fs::File::create_new(&alias) {
                        Ok(mut file) => {
                            use std::io::Write as _;
                            file.write_all(b"Keep this distinct entry").unwrap();
                            distinct.set(true);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(error) => panic!("create a distinct spelling control: {error}"),
                    }
                }
                Err(ToolError::internal("Initial publication failed"))
            });
            assert!(failed.is_err());
            assert!(!root.join(MANIFEST).exists());
            if distinct.get() {
                assert_eq!(std::fs::read(&alias).unwrap(), b"Keep this distinct entry");
                assert!(root.join("inbox/feed.lock").exists());
            } else {
                assert!(!root.join("inbox").exists());
                Workspace::create_at(&root, "Retry", "Test", TargetRef::None, None).unwrap();
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_rollback_distinguishes_detached_coordination_spellings() {
        let directory = tempfile::tempdir().unwrap();
        let (root, identity) = create_workspace_root(directory.path(), "Project").unwrap();
        let reservation = CreationReservation::claim(&root, identity).unwrap();
        let cleanup = reservation.cleanup.as_ref().unwrap();
        let held = reservation.coordination.as_ref().unwrap();
        let mut claimed = ClaimedInbox::claim(&root, &reservation.directory, &reservation.inbox, cleanup, Some(held), || {}).unwrap();
        let first = claimed.coordination_path(std::ffi::OsStr::new("feed.lock"));
        let second = cleanup.checked_path(&root).unwrap().join("coordination-alias");
        let witness = root.join("held-lock-witness");
        std::fs::hard_link(&first, &second).unwrap();
        std::fs::hard_link(&first, &witness).unwrap();
        // Model the entries retained from a case-sensitive inbox without requiring the
        // runner to enable that NTFS directory attribute. Both links use the real held inode.
        claimed.detached.push(("FEED.LOCK".into(), second.clone()));
        assert_eq!(FileHandle::from_path(&first).unwrap(), FileHandle::from_path(&second).unwrap());

        let released = claimed.release(
            || {},
            || {
                for name in ["feed.lock", "FEED.LOCK"] {
                    let path = claimed.coordination_path(std::ffi::OsStr::new(name));
                    crate::feed::check_coordination_entry(&path, held)?;
                    std::fs::remove_file(&path).map_err(|error| ToolError::io(path.display(), error))?;
                }
                Ok(())
            },
        );

        released.expect("each spelling must remove its own detached entry");
        assert!(!first.exists());
        assert!(!second.exists());
        cleanup.remove_empty(&root).unwrap();
        let contender = std::fs::OpenOptions::new().read(true).write(true).open(&witness).unwrap();
        assert!(matches!(contender.try_lock(), Err(std::fs::TryLockError::WouldBlock)));
        assert_eq!(FileHandle::from_path(&witness).unwrap(), *held);
    }

    #[test]
    fn deletion_preserves_inbox_entries_arriving_after_payload_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Project", "Test", TargetRef::None).unwrap();
        let root = ws.root.clone();
        let late = root.join("inbox/late-question.json");
        let result = ws.delete_with_remover(
            |_| {},
            |path| {
                if path == root.join(MANIFEST) {
                    std::fs::write(&late, b"Keep the late question").unwrap();
                }
                let removed = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
                match removed {
                    Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(ToolError::io(path.display(), error)),
                    _ => Ok(()),
                }
            },
        );

        let error = result.unwrap_err();
        let kept = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("reservation"))
            .find(|path| path.join("late-question.json").is_file())
            .expect("a late inbox entry survives in the reported private directory");
        assert_eq!(std::fs::read(kept.join("late-question.json")).unwrap(), b"Keep the late question");
        assert!(!root.join(MANIFEST).exists());
        assert!(error.remediation.as_deref().unwrap().contains(&kept.display().to_string()));

        let personal = root.join(".craft-copy-cleanup-personal/reservation");
        std::fs::create_dir_all(&personal).unwrap();
        std::fs::write(personal.join("notes.txt"), b"Keep this personal folder").unwrap();
        let error = Workspace::create_at(&root, "Replacement", "New project", TargetRef::None, Some(ExistingFiles::Edit)).unwrap_err();
        let retained = kept.parent().unwrap();
        assert!(error.for_people().contains(&retained.display().to_string()));
        assert!(!root.join(MANIFEST).exists());

        // Model a project already recreated by the earlier build, then reopen it: no
        // in-memory cleanup ownership can be assumed when deciding whether to copy.
        let mut manifest = Manifest::new("Replacement", "New project", TargetRef::None).unwrap();
        manifest.app_in_root = true;
        std::fs::write(root.join(MANIFEST), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
        let replacement = Workspace::open(&root).unwrap();
        let error = replacement.duplicate("Copy").unwrap_err();
        assert!(error.for_people().contains(&retained.display().to_string()));
        assert!(!directory.path().join("copy").exists());
        assert_eq!(std::fs::read(kept.join("late-question.json")).unwrap(), b"Keep the late question");

        let safe = directory.path().join("retained-recovery");
        std::fs::rename(retained, &safe).unwrap();
        let copy = replacement.duplicate("Copy").unwrap();

        assert!(!copy.app_dir().join(kept.strip_prefix(&root).unwrap()).join("late-question.json").exists());
        assert_eq!(std::fs::read(copy.app_dir().join(".craft-copy-cleanup-personal/reservation/notes.txt")).unwrap(), b"Keep this personal folder");
        assert_eq!(std::fs::read(safe.join("reservation/late-question.json")).unwrap(), b"Keep the late question");
    }

    #[test]
    fn cleanup_names_block_ambiguous_directories_without_omitting_personal_files() {
        for (name, is_directory, blocked) in [
            (".craft-copy-cleanup-0123456789abcdef", true, true),
            (".craft-copy-cleanup-0123456789abcdef", false, false),
            (".craft-copy-cleanup-0123456789abcde", true, false),
            (".craft-copy-cleanup-0123456789abcdef-notes", true, false),
            (".craft-copy-cleanup-0123456789abcdeg", true, false),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("original");
            std::fs::create_dir(&root).unwrap();
            let entry = root.join(name);
            let personal = if is_directory {
                std::fs::create_dir(&entry).unwrap();
                entry.join("personal.txt")
            } else {
                entry.clone()
            };
            std::fs::write(&personal, b"Keep all these personal bytes").unwrap();
            let started = Workspace::create_at(&root, "Site", "Test", TargetRef::None, Some(ExistingFiles::Edit));
            if blocked {
                assert!(started.unwrap_err().for_people().contains(&entry.display().to_string()));
                assert!(!root.join(MANIFEST).exists());
                assert!(!root.join("inbox").exists());
                assert_eq!(std::fs::read(&personal).unwrap(), b"Keep all these personal bytes");
                let safe = directory.path().join("personal-recovery-name");
                std::fs::rename(&entry, &safe).unwrap();
                std::fs::write(root.join("index.html"), b"Keep my app").unwrap();
                let ws = Workspace::create_at(&root, "Site", "Test", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
                std::fs::rename(&safe, &entry).unwrap();
                assert!(ws.duplicate("Copy").unwrap_err().for_people().contains(&entry.display().to_string()));
                assert!(!directory.path().join("copy").exists());
                assert_eq!(std::fs::read(&personal).unwrap(), b"Keep all these personal bytes");
            } else {
                let copy = started.unwrap().duplicate("Copy").unwrap();
                assert_eq!(std::fs::read(copy.app_dir().join(personal.strip_prefix(&root).unwrap())).unwrap(), b"Keep all these personal bytes");
                assert_eq!(std::fs::read(&personal).unwrap(), b"Keep all these personal bytes");
            }
        }
    }

    #[test]
    fn reference_and_capture_copies_refuse_nested_cleanup_directories() {
        for folder in ["reference", "capture"] {
            let directory = tempfile::tempdir().unwrap();
            let ws = Workspace::create(directory.path(), "Original", "A website", TargetRef::None).unwrap();
            let retained = ws.root.join(folder).join("nested").join(".craft-copy-cleanup-0123456789abcdef");
            std::fs::create_dir_all(&retained).unwrap();
            std::fs::write(retained.join("question.json"), b"Keep this retained question").unwrap();

            let error = ws.duplicate("Copy").unwrap_err();

            assert!(error.for_people().contains(&retained.display().to_string()));
            assert_eq!(std::fs::read(retained.join("question.json")).unwrap(), b"Keep this retained question");
            assert!(!directory.path().join("copy").join(MANIFEST).exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn opening_a_project_refuses_a_named_pipe_manifest_without_waiting() {
        use std::os::unix::fs::FileTypeExt as _;
        use std::time::Duration;
        const FIXTURE: &str = "CRAFT_TEST_OPEN_MANIFEST_FIFO_ROOT";
        if let Some(root) = std::env::var_os(FIXTURE) {
            let root = Path::new(&root);
            let path = root.join(MANIFEST);
            let original = root.join("original-craft.json");
            std::fs::rename(&path, &original).unwrap();
            let before = std::fs::read(&original).unwrap();
            craft_anything_platform::Process::new("mkfifo").args([path.as_os_str()]).timeout(Duration::from_secs(2)).run_checked().unwrap();

            assert_eq!(Workspace::open(root).unwrap_err().code, ErrorCode::BadInput);
            assert!(std::fs::symlink_metadata(&path).unwrap().file_type().is_fifo());
            assert_eq!(std::fs::read(&original).unwrap(), before);
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let ws = Workspace::create(directory.path(), "Project", "A website", TargetRef::None).unwrap();
        craft_anything_platform::Process::new(std::env::current_exe().unwrap())
            .args(["--exact", "workspace::tests::opening_a_project_refuses_a_named_pipe_manifest_without_waiting", "--test-threads=1"])
            .env(FIXTURE, &ws.root)
            .timeout(Duration::from_secs(5))
            .stdout_cap(64 * 1024)
            .stderr_cap(64 * 1024)
            .run_checked()
            .expect("opening a project must not block on a pipe replacing craft.json");
    }

    #[test]
    fn in_place_copy_rejects_cleanup_directories_arriving_after_discovery() {
        for parent in ["", "nested", "inbox", "node_modules"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("original");
            std::fs::create_dir(&root).unwrap();
            std::fs::write(root.join("index.html"), b"Keep my app").unwrap();
            let ws = Workspace::create_at(&root, "Site", "Test", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
            let retained = root.join(parent).join(".craft-copy-cleanup-0123456789abcdef");

            let copied = ws.duplicate_after_discovery("Copy", |_| {
                std::fs::create_dir_all(&retained).unwrap();
                std::fs::write(retained.join("question.json"), b"Keep this retained question").unwrap();
            });

            if matches!(parent, "inbox" | "node_modules") {
                let copy = copied.unwrap();
                assert!(!copy.app_dir().join(parent).exists(), "already excluded records and build output need no additional traversal");
                assert_eq!(std::fs::read(copy.app_dir().join("index.html")).unwrap(), b"Keep my app");
            } else {
                assert!(copied.unwrap_err().for_people().contains(&retained.display().to_string()));
                assert!(!directory.path().join("copy").join(MANIFEST).exists());
            }
            assert_eq!(std::fs::read(retained.join("question.json")).unwrap(), b"Keep this retained question");
            assert_eq!(std::fs::read(root.join("index.html")).unwrap(), b"Keep my app");
        }
    }

    #[test]
    fn creation_rechecks_cleanup_state_before_publishing_its_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("selected");
        let retained = root.join(".craft-copy-cleanup-0123456789abcdef");
        let result = Workspace::create_at_with_publisher(&root, "Selected", "Test", TargetRef::None, None, |ws, creation| {
            std::fs::create_dir(&retained).unwrap();
            std::fs::write(retained.join("personal.txt"), b"Keep this newly arrived directory").unwrap();
            ws.publish_manifest(creation)
        });

        assert!(result.unwrap_err().for_people().contains(&retained.display().to_string()));
        assert!(!root.join(MANIFEST).exists());
        assert!(!root.join("inbox").exists());
        assert_eq!(std::fs::read(retained.join("personal.txt")).unwrap(), b"Keep this newly arrived directory");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1, "only this attempt's empty reservation is released");
    }

    #[test]
    fn project_deletion_waits_for_an_active_feed_transaction() {
        use std::sync::mpsc;
        use std::time::Duration;
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("index.html"), b"Keep my app").unwrap();
        let ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        let (holding_tx, holding_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (deleted_tx, deleted_rx) = mpsc::channel();
        let writer_root = original.clone();
        let mut finished_while_writing = false;
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let _writer = crate::feed::lock_project(&writer_root).unwrap();
                holding_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                std::fs::write(writer_root.join(crate::feed::FEED_FILE), br#"{"version":6,"items":[]}"#).unwrap();
            });
            holding_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            scope.spawn(move || deleted_tx.send(ws.delete()).unwrap());
            let early = deleted_rx.recv_timeout(Duration::from_millis(200));
            finished_while_writing = early.is_ok();
            release_tx.send(()).unwrap();
            early.unwrap_or_else(|_| deleted_rx.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
        });

        assert!(!finished_while_writing, "deletion cannot finish while a feed writer still owns its transaction");
        assert!(!original.join(crate::feed::FEED_FILE).exists());
        assert!(!original.join(MANIFEST).exists());
        assert!(!original.join("inbox").exists());
        assert_eq!(std::fs::read(original.join("index.html")).unwrap(), b"Keep my app");
    }

    #[test]
    fn duplication_excludes_backups_created_after_discovery() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("index.html"), "Keep my app").unwrap();
        let ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        std::fs::write(original.join(crate::feed::FEED_FILE), b"newly damaged conversation").unwrap();
        crate::feed::save(&original, &mut crate::feed::load(&ws)).unwrap();
        let backup = original.join("feed.json.broken");
        let receipt = original.join("inbox/feed-backups/feed.json.broken.json");
        let anchor = original.join("inbox/feed-backups/pending/feed.json.broken");
        let pending_receipt = original.join("inbox/feed-backups/pending/feed.json.broken.json");
        std::fs::remove_file(&backup).unwrap();
        std::fs::remove_file(&receipt).unwrap();

        let copy = ws
            .duplicate_after_discovery("Copy", |selected| {
                assert!(selected.is_empty());
                // Simulate an outside writer without recursively acquiring the source
                // transaction lock already held by duplication.
                // A v2 publication retains the private inode; writing equal bytes into
                // a new inode would intentionally describe a personal replacement.
                std::fs::hard_link(&anchor, &backup).unwrap();
                std::fs::hard_link(&pending_receipt, &receipt).unwrap();
            })
            .unwrap();

        assert_eq!(std::fs::read(original.join("feed.json.broken")).unwrap(), b"newly damaged conversation");
        assert!(!copy.app_dir().join("feed.json.broken").exists(), "a newly generated backup is not part of the app");
        assert_eq!(std::fs::read(copy.app_dir().join("index.html")).unwrap(), b"Keep my app");
    }

    #[test]
    fn duplication_waits_for_a_new_backups_ownership_record() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("index.html"), "Keep my app").unwrap();
        let ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        // Withhold both receipt entries to simulate an incomplete outside publication.
        // The new public entry stays uncertain until its actual identity proof returns.
        std::fs::write(original.join(crate::feed::FEED_FILE), b"damaged conversation").unwrap();
        crate::feed::save(&original, &mut crate::feed::load(&ws)).unwrap();
        let backup = original.join("feed.json.broken");
        let receipt = original.join("inbox/feed-backups/feed.json.broken.json");
        let anchor = original.join("inbox/feed-backups/pending/feed.json.broken");
        let pending_receipt = original.join("inbox/feed-backups/pending/feed.json.broken.json");
        let ownership = std::fs::read(&receipt).unwrap();
        std::fs::remove_file(&backup).unwrap();
        std::fs::remove_file(&receipt).unwrap();
        std::fs::remove_file(&pending_receipt).unwrap();

        let result = ws.duplicate_after_discovery("Copy", |selected| {
            assert!(selected.is_empty());
            std::fs::hard_link(&anchor, &backup).unwrap();
        });

        assert_eq!(result.unwrap_err().code, ErrorCode::FileChanged);
        assert!(!directory.path().join("copy/app/feed.json.broken").exists(), "do not copy recovery bytes before their receipt is published");
        assert!(!directory.path().join("copy").exists(), "an unsuccessful attempt releases the destination it created");
        assert_eq!(Workspace::list(directory.path()).len(), 1, "an incomplete duplicate is not a discoverable project");
        assert_eq!(std::fs::read(&backup).unwrap(), b"damaged conversation");
        std::fs::write(&pending_receipt, ownership).unwrap();
        std::fs::hard_link(&pending_receipt, &receipt).unwrap();
        let copy = ws.duplicate("Copy").unwrap();
        assert_eq!(copy.root, directory.path().join("copy"), "retry uses the originally requested folder");
        assert!(!copy.app_dir().join("feed.json.broken").exists());
        assert_eq!(std::fs::read(copy.app_dir().join("index.html")).unwrap(), b"Keep my app");
    }

    #[test]
    fn duplication_does_not_copy_a_backup_whose_receipt_is_still_being_published() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("index.html"), "Keep my app").unwrap();
        let ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        std::fs::write(original.join(crate::feed::FEED_FILE), b"damaged conversation").unwrap();
        crate::feed::save(&original, &mut crate::feed::load(&ws)).unwrap();
        let receipt = original.join("inbox/feed-backups/feed.json.broken.json");
        let ownership = std::fs::read(&receipt).unwrap();
        std::fs::remove_file(&receipt).unwrap();
        // The recovery writer has completed its backup before discovery, but still owns
        // the transaction while publishing its receipt. Use the real OS lock protocol.
        let writing = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(original.join("inbox/feed.lock")).unwrap();
        writing.lock().unwrap();

        let result = ws.duplicate("Copy");

        assert!(result.is_err(), "a busy recovery must not be mistaken for a stable personal file");
        assert!(!directory.path().join("copy").exists());
        assert_eq!(std::fs::read(original.join("feed.json.broken")).unwrap(), b"damaged conversation");
        std::fs::write(receipt, ownership).unwrap();
        drop(writing);
        let copy = ws.duplicate("Copy").unwrap();
        assert_eq!(copy.root, directory.path().join("copy"));
        assert!(!copy.app_dir().join("feed.json.broken").exists());
        assert_eq!(std::fs::read(copy.app_dir().join("index.html")).unwrap(), b"Keep my app");
    }

    #[test]
    fn a_failed_duplicate_preserves_an_existing_project_with_the_requested_name() {
        let directory = tempfile::tempdir().unwrap();
        let existing = Workspace::create(directory.path(), "Copy", "Keep this project", TargetRef::None).unwrap();
        std::fs::create_dir_all(existing.app_dir()).unwrap();
        std::fs::write(existing.app_dir().join("keep.txt"), b"Keep the other project").unwrap();
        let original = directory.path().join("original");
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("index.html"), "Keep my app").unwrap();
        let ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        let backup = original.join("feed.json.broken");

        let result = ws.duplicate_after_discovery("Copy", |_| std::fs::write(&backup, b"pending recovery copy").unwrap());

        let error = result.unwrap_err();
        let original_error = copy_changed(&backup);
        assert_eq!(error.code, original_error.code);
        assert_eq!(error.message, original_error.message);
        let remediation = error.remediation.as_deref().unwrap();
        assert!(remediation.contains(original_error.remediation.as_deref().unwrap()), "the copy failure keeps its original recovery advice");
        let kept = std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("claimed"))
            .find(|path| path.join("app").is_dir())
            .expect("the unfinished app is retained at its reported private path");
        assert!(remediation.contains(&kept.display().to_string()), "the person can find the retained copy");
        assert!(remediation.contains("remaining contents"), "the inner cleanup inspection advice is retained too");
        assert!(remediation.contains(&directory.path().join("copy-2").display().to_string()), "the originally requested path remains available as context");
        assert!(!directory.path().join("copy-2").exists());
        assert_eq!(Workspace::list(directory.path()).len(), 2);
        assert_eq!(std::fs::read(existing.app_dir().join("keep.txt")).unwrap(), b"Keep the other project");
        assert_eq!(Workspace::open(&existing.root).unwrap().manifest.request, "Keep this project");
        std::fs::remove_file(backup).unwrap();
        let copy = ws.duplicate("Copy").unwrap();
        assert_eq!(copy.root, directory.path().join("copy-2"));
        assert_eq!(std::fs::read(copy.app_dir().join("index.html")).unwrap(), b"Keep my app");
    }

    #[cfg(unix)]
    #[test]
    fn a_person_selected_directory_link_keeps_its_resolved_project() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        let selected = directory.path().join("selected");
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("index.html"), b"Keep this selected app").unwrap();
        symlink(&original, &selected).unwrap();

        let mut ws = Workspace::create_at(&selected, "Selected", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();

        assert_eq!(ws.root, std::fs::canonicalize(&original).unwrap());
        assert!(ws.manifest.app_in_root);
        assert_eq!(std::fs::read(ws.app_dir().join("index.html")).unwrap(), b"Keep this selected app");
        let replacement = Workspace::create(directory.path(), "Replacement", "Keep this project", TargetRef::None).unwrap();
        let replacement_manifest = std::fs::read(replacement.root.join(MANIFEST)).unwrap();
        std::fs::remove_file(&selected).unwrap();
        symlink(&replacement.root, &selected).unwrap();
        ws.rename("Still the selected app").unwrap();
        assert_eq!(Workspace::open(&original).unwrap().manifest.name, "Still the selected app");
        assert_eq!(std::fs::read(replacement.root.join(MANIFEST)).unwrap(), replacement_manifest);
    }

    #[test]
    fn an_unfinished_copy_destination_refuses_another_project_creator() {
        for with_app in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let destination = CopyDestination::create(directory.path(), "Copy").unwrap();
            let root = destination.root.clone();
            if with_app {
                std::fs::create_dir(destination.root.join("app")).unwrap();
                std::fs::write(destination.root.join("app/index.html"), b"Copy in progress").unwrap();
            }
            assert!(!destination.root.join(MANIFEST).exists());

            let result = Workspace::create_at(&destination.root, "Other project", "Keep this project", TargetRef::None, Some(ExistingFiles::Edit));

            assert!(result.is_err(), "a not-yet-published copy cannot be adopted as an existing app");
            assert!(!destination.root.join(MANIFEST).exists());
            if with_app {
                let error = destination.remove().unwrap_err();
                let kept = std::fs::read_dir(directory.path())
                    .unwrap()
                    .filter_map(Result::ok)
                    .map(|entry| entry.path().join("claimed"))
                    .find(|path| path.join("app/index.html").is_file())
                    .expect("unfinished copy files are kept in the reported private folder");
                assert_eq!(std::fs::read(kept.join("app/index.html")).unwrap(), b"Copy in progress");
                assert!(error.message.contains(&kept.display().to_string()));
                let retry = CopyDestination::create(directory.path(), "Copy").unwrap();
                assert_eq!(retry.root, root);
                retry.remove().unwrap();
            } else {
                destination.remove().unwrap();
            }
            assert!(!root.exists());
        }
    }

    #[test]
    fn an_unfinished_duplicate_keeps_its_reservation_until_failure_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("index.html"), b"Keep my app").unwrap();
        let ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();

        let result = ws.duplicate_with_hooks(
            "Copy",
            |_| std::fs::write(original.join("feed.json.broken"), b"pending recovery copy").unwrap(),
            |destination| {
                assert!(!destination.join(MANIFEST).exists());
                let other = Workspace::create_at(destination, "Other project", "Keep this project", TargetRef::None, Some(ExistingFiles::Edit));
                assert!(other.is_err(), "cleanup must not remove a destination another project creator can adopt");
            },
        );

        assert_eq!(result.unwrap_err().code, ErrorCode::FileChanged);
        assert!(!directory.path().join("copy").exists());
        assert_eq!(std::fs::read(original.join("index.html")).unwrap(), b"Keep my app");
        assert_eq!(Workspace::list(directory.path()).len(), 1);
    }

    #[test]
    fn a_copy_that_loses_its_creation_reservation_preserves_the_winner() {
        let directory = tempfile::tempdir().unwrap();
        let result = CopyDestination::create_after_root(directory.path(), "Copy", |root| {
            let winner = Workspace::create_at(root, "Winner", "Keep this project", TargetRef::None, None).unwrap();
            std::fs::write(winner.root.join("personal.txt"), b"Keep the winning creator's files").unwrap();
        });

        assert!(result.is_err());
        let root = directory.path().join("copy");
        assert_eq!(Workspace::open(&root).unwrap().manifest.name, "Winner");
        assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep the winning creator's files");
        assert!(root.join("inbox").is_dir());
    }

    #[test]
    fn creation_reservation_refuses_a_replaced_new_root_before_copying() {
        let directory = tempfile::tempdir().unwrap();
        let personal = directory.path().join("personal");
        let displaced = directory.path().join("displaced-copy");
        std::fs::create_dir(&personal).unwrap();
        std::fs::write(personal.join("personal.txt"), b"Keep this personal folder").unwrap();

        let result = CopyDestination::create_after_root(directory.path(), "Copy", |root| {
            std::fs::rename(root, &displaced).unwrap();
            std::fs::rename(&personal, root).unwrap();
        });

        let refused = match result {
            Err(error) => {
                assert_eq!(error.code, ErrorCode::FileChanged);
                true
            }
            Ok(destination) => {
                destination.remove().unwrap();
                false
            }
        };
        let root = directory.path().join("copy");
        assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep this personal folder");
        assert!(refused, "a failed copy must never own a replacement introduced before its reservation");
        assert!(!root.join("inbox").exists(), "the replacement is refused before adding coordination files");
        assert!(displaced.is_dir());
    }

    #[test]
    fn published_projects_report_cleanup_failures_without_rejecting_the_project() {
        for existing in [None, Some(ExistingFiles::Edit), Some(ExistingFiles::StartFrom)] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("selected");
            std::fs::create_dir(&root).unwrap();
            if existing.is_some() {
                std::fs::write(root.join("personal.txt"), b"Keep my existing files").unwrap();
            }
            let mut retained = PathBuf::new();
            let mut ws = Workspace::create_at_with_publisher(&root, "Selected", "Test", TargetRef::None, existing, |ws, creation| {
                ws.publish_manifest_using(
                    creation,
                    write_atomic,
                    |cleanup, root| {
                        retained = root.join(&cleanup.name);
                        std::fs::write(retained.join("arrived.txt"), b"Keep this late arrival").unwrap();
                        cleanup.remove_empty(root)
                    },
                    |_| {},
                )
            })
            .unwrap();

            let warning = ws.creation_warning.as_deref().unwrap();
            assert!(warning.contains("manifest was published"));
            assert!(warning.contains(&retained.display().to_string()));
            assert_eq!(std::fs::read(retained.join("arrived.txt")).unwrap(), b"Keep this late arrival");
            assert_eq!(Workspace::open(&root).unwrap().manifest.generation, ws.manifest.generation);
            drop(crate::feed::lock_project(&root).unwrap());
            if existing.is_some() {
                assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep my existing files");
            }
            let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(MANIFEST)).unwrap()).unwrap();
            assert!(saved.get("creation_warning").is_none(), "cleanup guidance is not a manifest migration");
            ws.reload().unwrap();
            assert!(ws.creation_warning.is_none(), "the warning belongs to the successful creation result");
        }
    }

    #[test]
    fn a_published_copy_returns_its_warning_without_entering_failure_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let original = Workspace::create(directory.path(), "Original", "Test", TargetRef::None).unwrap();
        std::fs::create_dir(original.app_dir()).unwrap();
        std::fs::write(original.app_dir().join("index.html"), b"Keep this app").unwrap();
        let mut retained = PathBuf::new();
        let copy = original
            .duplicate_with_publisher(
                "Copy",
                original.lock_copy_source().unwrap(),
                |_| {},
                |_| panic!("a published copy must not enter failure cleanup"),
                |copy, creation| {
                    copy.publish_manifest_using(
                        creation,
                        write_atomic,
                        |cleanup, root| {
                            retained = root.join(&cleanup.name);
                            std::fs::write(retained.join("arrived.txt"), b"Keep the cleanup arrival").unwrap();
                            cleanup.remove_empty(root)
                        },
                        |_| {},
                    )
                },
            )
            .unwrap();

        assert!(copy.creation_warning.as_deref().unwrap().contains(&retained.display().to_string()));
        assert_eq!(Workspace::open(&copy.root).unwrap().manifest.generation, copy.manifest.generation);
        assert_ne!(copy.manifest.generation, original.manifest.generation);
        assert_eq!(std::fs::read(copy.app_dir().join("index.html")).unwrap(), b"Keep this app");
        assert_eq!(std::fs::read(original.app_dir().join("index.html")).unwrap(), b"Keep this app");
        assert_eq!(std::fs::read(retained.join("arrived.txt")).unwrap(), b"Keep the cleanup arrival");
        drop(crate::feed::lock_project(&copy.root).unwrap());
    }

    #[test]
    fn creation_rollback_reuses_reserved_space_when_new_cleanup_directories_cannot_be_created() {
        for existing in [None, Some(ExistingFiles::Edit), Some(ExistingFiles::StartFrom)] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("selected");
            std::fs::create_dir(&root).unwrap();
            if existing.is_some() {
                std::fs::write(root.join("personal.txt"), b"Keep this existing file").unwrap();
            }
            let storage_full = std::cell::Cell::new(false);
            let allocations = std::cell::Cell::new(0);
            let late_allocations = std::cell::Cell::new(0);
            let allocate = |parent: &Path| {
                if storage_full.get() {
                    late_allocations.set(late_allocations.get() + 1);
                    return Err(ToolError::io(parent.display(), std::io::Error::from(std::io::ErrorKind::StorageFull)));
                }
                allocations.set(allocations.get() + 1);
                CopyDestination::cleanup_directory(parent)
            };
            let error = Workspace::create_at_with_cleanup(
                &root,
                "Selected",
                "Test",
                TargetRef::None,
                existing,
                |ws, _| {
                    storage_full.set(true);
                    Err(ToolError::io(ws.root.join(MANIFEST).display(), std::io::Error::from(std::io::ErrorKind::StorageFull)))
                },
                &allocate,
            )
            .unwrap_err();

            assert_eq!(error.code, ErrorCode::FileError);
            assert_eq!(allocations.get(), 1, "rollback space is acquired before publication");
            assert_eq!(late_allocations.get(), 0, "failed publication must not depend on another directory allocation");
            assert!(!root.join(MANIFEST).exists());
            assert!(!root.join("inbox").exists(), "the failed attempt does not block a normal retry");
            assert_eq!(std::fs::read_dir(&root).unwrap().count(), usize::from(existing.is_some()));
            if existing.is_some() {
                assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep this existing file");
            }
            Workspace::create_at(&root, "Retry", "Test", TargetRef::None, existing).unwrap();
            assert!(!std::fs::read_dir(&root).unwrap().any(|entry| entry.unwrap().file_name().to_string_lossy().starts_with(".craft-copy-cleanup-")));
        }
    }

    #[test]
    fn cleanup_space_allocation_failure_precedes_inbox_reservation() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("selected");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("personal.txt"), b"Keep this existing file").unwrap();
        let published = std::cell::Cell::new(false);
        let result = Workspace::create_at_with_cleanup(
            &root,
            "Selected",
            "Test",
            TargetRef::None,
            Some(ExistingFiles::Edit),
            |_, _| {
                published.set(true);
                Ok(())
            },
            &|parent| Err(ToolError::io(parent.display(), std::io::Error::from(std::io::ErrorKind::StorageFull))),
        );

        assert_eq!(result.unwrap_err().code, ErrorCode::FileError);
        assert!(!published.get());
        assert!(!root.join(MANIFEST).exists());
        assert!(!root.join("inbox").exists());
        assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep this existing file");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        Workspace::create_at(&root, "Retry", "Test", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
    }

    #[test]
    fn creation_rollback_preserves_changed_or_occupied_cleanup_space_without_reallocating() {
        for change in ["occupied", "replaced", "missing"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("selected");
            std::fs::create_dir(&root).unwrap();
            let displaced = directory.path().join("displaced-cleanup");
            let reserved = std::cell::RefCell::new(PathBuf::new());
            let publication_failed = std::cell::Cell::new(false);
            let late_allocations = std::cell::Cell::new(0);
            let allocate = |parent: &Path| {
                if publication_failed.get() {
                    late_allocations.set(late_allocations.get() + 1);
                    return Err(ToolError::io(parent.display(), std::io::Error::from(std::io::ErrorKind::StorageFull)));
                }
                let private = CopyDestination::cleanup_directory(parent)?;
                reserved.replace(private.clone());
                Ok(private)
            };
            let error = Workspace::create_at_with_cleanup(
                &root,
                "Selected",
                "Test",
                TargetRef::None,
                None,
                |ws, _| {
                    let private = reserved.borrow();
                    if change == "missing" {
                        std::fs::remove_dir(&*private).unwrap();
                    } else {
                        if change == "replaced" {
                            std::fs::rename(&*private, &displaced).unwrap();
                            std::fs::create_dir(&*private).unwrap();
                        }
                        std::fs::write(private.join("personal.txt"), b"Keep these unexpected bytes").unwrap();
                    }
                    publication_failed.set(true);
                    Err(ToolError::io(ws.root.join(MANIFEST).display(), std::io::Error::from(std::io::ErrorKind::StorageFull)))
                },
                &allocate,
            )
            .unwrap_err();

            let private = reserved.borrow();
            assert_eq!(late_allocations.get(), 0);
            assert!(!root.join(MANIFEST).exists());
            assert!(root.join("inbox").is_dir());
            if change != "missing" {
                assert_eq!(std::fs::read(private.join("personal.txt")).unwrap(), b"Keep these unexpected bytes");
            }
            if change == "replaced" {
                assert!(displaced.is_dir());
            }
            assert!(error.for_people().contains(&private.display().to_string()));
        }
    }

    #[test]
    fn creation_reservation_is_released_after_definite_managed_publication_failure() {
        let directory = tempfile::tempdir().unwrap();
        let original = std::cell::RefCell::new(None);
        let failed = std::cell::RefCell::new(PathBuf::new());
        let error = Workspace::create_with_publisher(directory.path(), "Managed", "A website", TargetRef::None, |ws, _| {
            failed.replace(ws.root.clone());
            assert!(ws.inbox_dir().is_dir());
            let error = ToolError::io(ws.root.join(MANIFEST).display(), std::io::Error::from(std::io::ErrorKind::StorageFull))
                .with_remediation("Free storage before retrying.");
            original.replace(Some(error.clone()));
            Err(error)
        })
        .unwrap_err();

        let original = original.into_inner().unwrap();
        assert_eq!(error.code, original.code);
        assert_eq!(error.message, original.message);
        assert_eq!(error.remediation, original.remediation);
        let root = failed.into_inner();
        assert!(std::fs::read_dir(&root).unwrap().next().is_none(), "a failed managed creation must release its unpublished reservation and cleanup slot");
        let retried = Workspace::create_at(&root, "Retry", "A website", TargetRef::None, None).unwrap();
        assert_eq!(Workspace::open(&root).unwrap().manifest.generation, retried.manifest.generation);
    }

    #[test]
    fn managed_publication_failure_keeps_published_or_occupied_reservations_and_the_original_error() {
        for published in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let failed = std::cell::RefCell::new(PathBuf::new());
            let error = Workspace::create_with_publisher(directory.path(), "Managed", "A website", TargetRef::None, |ws, creation| {
                failed.replace(ws.root.clone());
                if published {
                    ws.publish_manifest(creation)?;
                } else {
                    std::fs::write(ws.inbox_dir().join("personal.txt"), b"Keep these files after the failed write").unwrap();
                }
                Err(ToolError::new(ErrorCode::FileError, "Injected managed publication failure.").with_remediation("Keep the original recovery instruction."))
            })
            .unwrap_err();

            let root = failed.into_inner();
            assert_eq!(error.code, ErrorCode::FileError);
            assert_eq!(error.message, "Injected managed publication failure.");
            let remediation = error.remediation.as_deref().unwrap();
            assert!(remediation.contains("Keep the original recovery instruction."));
            assert!(remediation.contains(&root.join(MANIFEST).display().to_string()));
            assert!(remediation.contains(&root.join("inbox").display().to_string()));
            assert!(root.join("inbox").is_dir());
            if published {
                assert_eq!(Workspace::open(&root).unwrap().manifest.name, "Managed");
                drop(crate::feed::lock_project(&root).unwrap());
            } else {
                assert!(!root.join(MANIFEST).exists());
                assert_eq!(std::fs::read(root.join("inbox/personal.txt")).unwrap(), b"Keep these files after the failed write");
            }
        }
    }

    #[test]
    fn creation_reservation_is_released_after_definite_picked_folder_publication_failure() {
        for existing in [None, Some(ExistingFiles::Edit), Some(ExistingFiles::StartFrom)] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("selected");
            std::fs::create_dir(&root).unwrap();
            if existing.is_some() {
                std::fs::write(root.join("personal.txt"), b"Keep this existing file").unwrap();
            }

            let error = Workspace::create_at_with_publisher(&root, "Selected", "A website", TargetRef::None, existing, |ws, _| {
                assert!(ws.inbox_dir().is_dir());
                Err(ToolError::io(ws.root.join(MANIFEST).display(), std::io::Error::from(std::io::ErrorKind::StorageFull)))
            })
            .unwrap_err();

            assert_eq!(error.code, ErrorCode::FileError);
            assert!(root.is_dir(), "failure never removes the person's selected folder");
            assert!(!root.join(MANIFEST).exists());
            assert!(!root.join("inbox").exists(), "an unpublished empty reservation must not block a retry");
            let retried = Workspace::create_at(&root, "Retry", "A website", TargetRef::None, existing).unwrap();
            assert_eq!(Workspace::open(&root).unwrap().manifest.generation, retried.manifest.generation);
            assert!(retried.inbox_dir().is_dir(), "a successful creation retains its coordination directory");
            if existing.is_some() {
                assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep this existing file");
            }
        }
    }

    #[test]
    fn creation_reservation_survives_a_picked_folder_error_after_manifest_publication() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("selected");
        let error = Workspace::create_at_with_publisher(&root, "Selected", "A website", TargetRef::None, None, |ws, creation| {
            ws.publish_manifest(creation)?;
            Err(ToolError::new(ErrorCode::FileError, "Injected error after publication."))
        })
        .unwrap_err();

        assert_eq!(error.message, "Injected error after publication.");
        assert_eq!(Workspace::open(&root).unwrap().manifest.name, "Selected");
        assert!(root.join("inbox").is_dir(), "an error after publication must not release project coordination");
        let remediation = error.remediation.as_deref().unwrap();
        assert!(remediation.contains(&root.display().to_string()));
        assert!(remediation.contains("Inspect"));
    }

    #[test]
    fn creation_reservation_preserves_a_nonempty_picked_folder_inbox_on_publication_failure() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("selected");
        let error = Workspace::create_at_with_publisher(&root, "Selected", "A website", TargetRef::None, None, |ws, _| {
            std::fs::write(ws.inbox_dir().join("personal.txt"), b"Keep the occupied inbox").unwrap();
            Err(ToolError::new(ErrorCode::FileError, "Injected publication failure."))
        })
        .unwrap_err();

        assert_eq!(error.message, "Injected publication failure.");
        assert!(!root.join(MANIFEST).exists());
        assert_eq!(std::fs::read(root.join("inbox/personal.txt")).unwrap(), b"Keep the occupied inbox");
        assert!(error.remediation.as_deref().unwrap().contains(&root.join("inbox").display().to_string()));
    }

    #[test]
    fn creation_reservation_preserves_replaced_picked_folder_entries_on_publication_failure() {
        for replace_root in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("selected");
            std::fs::create_dir(&root).unwrap();
            std::fs::write(root.join("personal.txt"), b"Keep the original file").unwrap();
            let displaced = directory.path().join("displaced");
            let error = Workspace::create_at_with_publisher(&root, "Selected", "A website", TargetRef::None, Some(ExistingFiles::Edit), |ws, _creation| {
                let entry = if replace_root { ws.root.clone() } else { ws.inbox_dir() };
                if replace_root {
                    rename_creation_root_for_replacement(&entry, &displaced);
                } else {
                    #[cfg(windows)]
                    detach_creation_lock_for_replacement(&ws.root, _creation, &directory.path().join("retained.lock"));
                    std::fs::rename(&entry, &displaced).unwrap();
                }
                std::fs::create_dir(&entry).unwrap();
                if replace_root {
                    std::fs::create_dir(ws.inbox_dir()).unwrap();
                    std::fs::write(ws.root.join("personal.txt"), b"Keep the replacement file").unwrap();
                }
                Err(ToolError::new(ErrorCode::FileError, "Injected publication failure."))
            })
            .unwrap_err();

            assert_eq!(error.message, "Injected publication failure.");
            assert!(root.join("inbox").is_dir(), "an empty replacement is not this attempt's reservation");
            assert!(displaced.is_dir(), "the displaced original also remains intact");
            if replace_root {
                assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep the replacement file");
                assert_eq!(std::fs::read(displaced.join("personal.txt")).unwrap(), b"Keep the original file");
                assert!(displaced.join("inbox").is_dir());
            } else {
                assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep the original file");
            }
            assert!(error.remediation.as_deref().unwrap().contains(&root.join("inbox").display().to_string()));
        }
    }

    #[test]
    fn creation_refuses_personal_contents_introduced_before_the_initial_directory_open() {
        let directory = tempfile::tempdir().unwrap();
        let displaced = directory.path().join("displaced-copy");
        let result = create_workspace_root_after_mkdir(directory.path(), "Copy", |root| {
            std::fs::rename(root, &displaced).unwrap();
            std::fs::create_dir(root).unwrap();
            std::fs::write(root.join("personal.txt"), b"Keep this early replacement").unwrap();
        });

        assert_eq!(result.unwrap_err().code, ErrorCode::FileChanged);
        let root = directory.path().join("copy");
        assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep this early replacement");
        assert!(!root.join("inbox").exists());
        assert!(displaced.is_dir());
    }

    #[test]
    fn copy_cleanup_preserves_files_added_to_its_unpublished_root() {
        let directory = tempfile::tempdir().unwrap();
        let destination = CopyDestination::create(directory.path(), "Copy").unwrap();
        std::fs::write(destination.root.join("personal.txt"), b"Keep this personal file").unwrap();

        let error = destination.remove().unwrap_err();

        let kept = std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("claimed"))
            .find(|path| path.join("personal.txt").is_file())
            .expect("holding the root's handle never authorizes deleting personal contents");
        assert_eq!(std::fs::read(kept.join("personal.txt")).unwrap(), b"Keep this personal file");
        assert!(error.message.contains(&kept.display().to_string()));
    }

    #[test]
    fn reservation_release_preserves_a_replacement_after_its_last_public_path_check() {
        let directory = tempfile::tempdir().unwrap();
        let (root, handle) = create_workspace_root(directory.path(), "New").unwrap();
        let creation = CreationReservation::claim(&root, handle).unwrap();
        let displaced = directory.path().join("displaced-inbox");

        let result = creation.release_empty_after_check(&root, || {
            std::fs::rename(root.join("inbox"), &displaced).unwrap();
            std::fs::create_dir(root.join("inbox")).unwrap();
        });

        let error = result.unwrap_err();
        #[cfg(windows)]
        let kept = root.join("inbox");
        #[cfg(not(windows))]
        let kept = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("reservation"))
            .find(|path| path.is_dir())
            .expect("the replacement inbox remains in the reported private folder");
        assert!(error.message.contains(&kept.display().to_string()));
        assert!(displaced.is_dir());
    }

    #[cfg(windows)]
    #[test]
    fn windows_retirement_preserves_a_source_replaced_after_its_last_identity_check() {
        let directory = tempfile::tempdir().unwrap();
        let (root, handle) = create_workspace_root(directory.path(), "New").unwrap();
        let creation = CreationReservation::claim(&root, handle).unwrap();
        let original = creation.coordination.as_ref().unwrap();
        let public = root.join("inbox/feed.lock");
        let private = root.join(&creation.cleanup.as_ref().unwrap().name);
        let retained = private.join("coordination-0");
        let displaced = directory.path().join("original.lock");
        let mut replacement = None;
        let error = ClaimedInbox::detach_entry(&public, &retained, original, || {
            std::fs::rename(&public, &displaced).unwrap();
            std::fs::write(&public, b"Keep this replacement entry").unwrap();
            replacement = Some(FileHandle::from_path(&public).unwrap());
        })
        .unwrap_err();
        assert!(!public.exists());
        assert_eq!(std::fs::read(&retained).unwrap(), b"Keep this replacement entry");
        assert_eq!(FileHandle::from_path(&retained).unwrap(), replacement.unwrap());
        assert_eq!(&FileHandle::from_path(&displaced).unwrap(), original);
        let other = std::fs::OpenOptions::new().read(true).write(true).open(displaced).unwrap();
        assert!(matches!(other.try_lock(), Err(std::fs::TryLockError::WouldBlock)));
        let expected_retained = std::fs::canonicalize(&retained).unwrap();
        let message = error.for_people();
        assert!(message.contains(&expected_retained.display().to_string()), "{message:?} should include {expected_retained:?}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_retirement_never_overwrites_a_private_target_created_after_its_source_check() {
        let directory = tempfile::tempdir().unwrap();
        let (root, handle) = create_workspace_root(directory.path(), "New").unwrap();
        let creation = CreationReservation::claim(&root, handle).unwrap();
        let original = creation.coordination.as_ref().unwrap();
        let public = root.join("inbox/feed.lock");
        let private = root.join(&creation.cleanup.as_ref().unwrap().name);
        let retained = private.join("coordination-0");
        let mut unexpected = None;
        let error = ClaimedInbox::detach_entry(&public, &retained, original, || {
            std::fs::write(&retained, b"Keep this private entry").unwrap();
            unexpected = Some(FileHandle::from_path(&retained).unwrap());
        })
        .unwrap_err();
        assert_eq!(&FileHandle::from_path(&public).unwrap(), original);
        assert_eq!(std::fs::metadata(&public).unwrap().len(), 0);
        assert_eq!(FileHandle::from_path(&retained).unwrap(), unexpected.unwrap());
        assert_eq!(std::fs::read(&retained).unwrap(), b"Keep this private entry");
        let other = std::fs::OpenOptions::new().read(true).write(true).open(public).unwrap();
        assert!(matches!(other.try_lock(), Err(std::fs::TryLockError::WouldBlock)));
        let expected_retained = std::fs::canonicalize(&retained).unwrap();
        let message = error.for_people();
        assert!(message.contains(&expected_retained.display().to_string()), "{message:?} should include {expected_retained:?}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_retirement_never_overwrites_a_closed_private_target_created_after_its_source_check() {
        let directory = tempfile::tempdir().unwrap();
        let (root, handle) = create_workspace_root(directory.path(), "New").unwrap();
        let creation = CreationReservation::claim(&root, handle).unwrap();
        let original = creation.coordination.as_ref().unwrap();
        let public = root.join("inbox/feed.lock");
        let private = root.join(&creation.cleanup.as_ref().unwrap().name);
        let retained = private.join("coordination-0");
        let witness = directory.path().join("unexpected-target-witness");
        let error = ClaimedInbox::detach_entry(&public, &retained, original, || {
            std::fs::write(&retained, b"Keep this closed private entry").unwrap();
            // The hard link preserves the inode witness without a target handle that
            // could itself prevent an overwrite. Open both identities only after the move.
            std::fs::hard_link(&retained, &witness).unwrap();
        })
        .unwrap_err();
        assert_eq!(&FileHandle::from_path(&public).unwrap(), original);
        assert_eq!(std::fs::metadata(&public).unwrap().len(), 0);
        assert_eq!(FileHandle::from_path(&retained).unwrap(), FileHandle::from_path(&witness).unwrap());
        assert_eq!(std::fs::read(&retained).unwrap(), b"Keep this closed private entry");
        assert_eq!(std::fs::read(&witness).unwrap(), b"Keep this closed private entry");
        let other = std::fs::OpenOptions::new().read(true).write(true).open(public).unwrap();
        assert!(matches!(other.try_lock(), Err(std::fs::TryLockError::WouldBlock)));
        let expected_retained = std::fs::canonicalize(&retained).unwrap();
        let message = error.for_people();
        assert!(message.contains(&expected_retained.display().to_string()), "{message:?} should include {expected_retained:?}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_retirement_cleans_projects_under_long_parent_paths() {
        let directory = tempfile::tempdir().unwrap();
        let mut parent = directory.path().to_path_buf();
        for _ in 0..10 {
            parent.push("a-long-selected-project-parent");
        }
        std::fs::create_dir_all(&parent).unwrap();
        let ws = Workspace::create(&parent, "Managed", "Test", TargetRef::None).unwrap();
        let root = ws.root.clone();
        std::fs::write(root.join("personal.txt"), b"Keep my files").unwrap();
        ws.delete().unwrap();
        assert!(!root.join(MANIFEST).exists());
        assert!(!root.join("inbox").exists());
        assert_eq!(std::fs::read(root.join("personal.txt")).unwrap(), b"Keep my files");
    }

    #[cfg(windows)]
    #[test]
    fn windows_retirement_preserves_a_late_initializers_new_lock_and_the_old_held_inode() {
        for keep_open in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let (root, handle) = create_workspace_root(directory.path(), "New").unwrap();
            let creation = CreationReservation::claim(&root, handle).unwrap();
            let private = root.join(&creation.cleanup.as_ref().unwrap().name);
            let original = FileHandle::from_path(root.join("inbox/feed.lock")).unwrap();
            let mut late = None;
            let mut late_identity = None;
            let result = creation.release_empty_after_check(&root, || {
                let retained = std::fs::read_dir(&private).unwrap().next().unwrap().unwrap().path();
                let other = std::fs::OpenOptions::new().read(true).write(true).open(retained).unwrap();
                assert!(matches!(other.try_lock(), Err(std::fs::TryLockError::WouldBlock)), "the old inode stays exclusively held after detachment");
                let file = craft_anything_platform::fs::open_child_file_for_update(creation.inbox.as_file(), Path::new("feed.lock"), true).unwrap();
                file.try_lock().unwrap();
                if keep_open {
                    late_identity = Some(FileHandle::from_path(root.join("inbox/feed.lock")).unwrap());
                    late = Some(file);
                }
            });
            let error = result.unwrap_err();
            let retained = std::fs::read_dir(&private)
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| path.is_file())
                .expect("the original locked inode is retained for inspection");
            assert_eq!(FileHandle::from_path(&retained).unwrap(), original);
            let new_path = if keep_open { root.join("inbox/feed.lock") } else { private.join("reservation/feed.lock") };
            if let Some(identity) = late_identity {
                assert_eq!(FileHandle::from_path(&new_path).unwrap(), identity);
            }
            assert_ne!(FileHandle::from_path(&new_path).unwrap(), original);
            assert_eq!(std::fs::metadata(&new_path).unwrap().len(), 0);
            assert!(error.for_people().contains(&private.display().to_string()));
            assert!(error.for_people().contains(&new_path.parent().unwrap().display().to_string()));
            assert!(!root.join(MANIFEST).exists());
            assert!(crate::feed::lock_project(&root).is_err(), "a new lock does not authorize a write to the unavailable project");
            drop(late);
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_retirement_keeps_both_locks_when_an_initializer_arrives_after_the_empty_check() {
        let directory = tempfile::tempdir().unwrap();
        let (root, handle) = create_workspace_root(directory.path(), "New").unwrap();
        let creation = CreationReservation::claim(&root, handle).unwrap();
        let original = creation.coordination.as_ref().unwrap();
        let claimed = ClaimedInbox::claim(&root, &creation.directory, &creation.inbox, creation.cleanup.as_ref().unwrap(), Some(original), || {}).unwrap();
        let retained = claimed.coordination_path(std::ffi::OsStr::new(crate::feed::PROJECT_LOCK_FILE));
        let mut late = None;
        let removed = std::cell::Cell::new(false);
        let result = claimed.release(
            || {
                let file = craft_anything_platform::fs::open_child_file_for_update(creation.inbox.as_file(), Path::new("feed.lock"), true).unwrap();
                file.try_lock().unwrap();
                late = Some(file);
            },
            || {
                removed.set(true);
                std::fs::remove_file(&retained).map_err(|error| ToolError::io(retained.display(), error))
            },
        );
        assert!(result.is_err());
        assert!(!removed.get(), "a newly nonempty claim stops before removing any old coordination link");
        assert_eq!(&FileHandle::from_path(&retained).unwrap(), original);
        let new_path = claimed.path.join("feed.lock");
        assert_ne!(&FileHandle::from_path(&new_path).unwrap(), original);
        assert_eq!(std::fs::metadata(new_path).unwrap().len(), 0);
        let other = std::fs::OpenOptions::new().read(true).write(true).open(retained).unwrap();
        assert!(matches!(other.try_lock(), Err(std::fs::TryLockError::WouldBlock)));
        drop(late);
    }

    #[cfg(windows)]
    #[test]
    fn windows_retirement_closes_the_captured_directory_before_removing_its_old_lock() {
        let directory = tempfile::tempdir().unwrap();
        let (root, handle) = create_workspace_root(directory.path(), "New").unwrap();
        let creation = CreationReservation::claim(&root, handle).unwrap();
        let original = creation.coordination.as_ref().unwrap();
        let claimed = ClaimedInbox::claim(&root, &creation.directory, &creation.inbox, creation.cleanup.as_ref().unwrap(), Some(original), || {}).unwrap();
        let retained = claimed.coordination_path(std::ffi::OsStr::new(crate::feed::PROJECT_LOCK_FILE));
        claimed
            .release(
                || {},
                || {
                    assert!(craft_anything_platform::fs::open_child_file_for_update(creation.inbox.as_file(), Path::new("feed.lock"), true).is_err());
                    assert_eq!(&FileHandle::from_path(&retained).unwrap(), original);
                    std::fs::remove_file(&retained).map_err(|error| ToolError::io(retained.display(), error))
                },
            )
            .unwrap();
        assert!(!claimed.path.exists());
        assert!(!retained.exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_retirement_preserves_new_manifests_and_replacement_inboxes() {
        for new_manifest in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let (root, handle) = create_workspace_root(directory.path(), "New").unwrap();
            let creation = CreationReservation::claim(&root, handle).unwrap();
            let private = root.join(&creation.cleanup.as_ref().unwrap().name);
            let original = FileHandle::from_path(root.join("inbox/feed.lock")).unwrap();
            let displaced = directory.path().join("old-inbox");
            let manifest = serde_json::to_vec(&Manifest::new("Replacement", "Keep this project", TargetRef::None).unwrap()).unwrap();
            let result = creation.release_empty_after_check(&root, || {
                if new_manifest {
                    std::fs::write(root.join(MANIFEST), &manifest).unwrap();
                } else {
                    std::fs::rename(root.join("inbox"), &displaced).unwrap();
                    std::fs::create_dir(root.join("inbox")).unwrap();
                    std::fs::write(root.join("inbox/personal.txt"), b"Keep the replacement inbox").unwrap();
                }
            });
            let error = result.unwrap_err();
            let retained = std::fs::read_dir(&private).unwrap().next().unwrap().unwrap().path();
            assert_eq!(FileHandle::from_path(retained).unwrap(), original);
            assert!(error.for_people().contains(&private.display().to_string()));
            assert!(!private.join("reservation").exists(), "the changed public state is refused before claiming it");
            if new_manifest {
                assert_eq!(std::fs::read(root.join(MANIFEST)).unwrap(), manifest);
            } else {
                assert_eq!(std::fs::read(root.join("inbox/personal.txt")).unwrap(), b"Keep the replacement inbox");
                assert_eq!(FileHandle::from_path(displaced).unwrap(), creation.inbox);
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_copy_cleanup_preserves_a_new_project_after_releasing_its_old_reservation() {
        let directory = tempfile::tempdir().unwrap();
        let destination = CopyDestination::create(directory.path(), "Copy").unwrap();
        let root = destination.root.clone();
        let mut replacement = None;
        let result = destination.remove_after_check(|| {
            let ws = Workspace::create_at(&root, "Replacement", "Keep this project", TargetRef::None, None).unwrap();
            std::fs::write(ws.inbox_dir().join("question.json"), b"Keep the next question").unwrap();
            replacement = Some((std::fs::read(root.join(MANIFEST)).unwrap(), FileHandle::from_path(ws.inbox_dir()).unwrap()));
        });
        assert!(result.is_err(), "a new project is not an empty failed copy");
        let (manifest, inbox) = replacement.unwrap();
        assert_eq!(std::fs::read(root.join(MANIFEST)).unwrap(), manifest);
        assert_eq!(FileHandle::from_path(root.join("inbox")).unwrap(), inbox);
        assert_eq!(std::fs::read(root.join("inbox/question.json")).unwrap(), b"Keep the next question");
    }

    #[test]
    fn failed_initialization_releases_only_its_own_empty_coordination_entries() {
        let directory = tempfile::tempdir().unwrap();
        let (root, handle) = create_workspace_root(directory.path(), "New").unwrap();
        std::fs::create_dir(root.join("inbox")).unwrap();
        let inbox = FileHandle::from_path(root.join("inbox")).unwrap();
        let lock = FileHandle::from_file(std::fs::File::create_new(root.join("inbox").join(crate::feed::PROJECT_LOCK_FILE)).unwrap()).unwrap();
        lock.as_file().try_lock().unwrap();

        release_created_inbox(&root, handle, inbox, Some(&lock), Some(&lock)).unwrap();

        assert!(!root.join("inbox").exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        Workspace::create_at(&root, "Retry", "New request", TargetRef::None, None).unwrap();
    }

    #[test]
    fn failed_initialization_preserves_foreign_or_changed_lock_files() {
        for owned in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let (root, handle) = create_workspace_root(directory.path(), "New").unwrap();
            std::fs::create_dir(root.join("inbox")).unwrap();
            let inbox = FileHandle::from_path(root.join("inbox")).unwrap();
            // Another stale initializer can win create_new after this attempt's mkdir.
            let path = root.join("inbox").join(crate::feed::PROJECT_LOCK_FILE);
            let lock = FileHandle::from_file(std::fs::File::create_new(&path).unwrap()).unwrap();
            let bytes = if owned { b"Keep these unexpected bytes".as_slice() } else { b"".as_slice() };
            std::fs::write(&path, bytes).unwrap();

            let error = release_created_inbox(&root, handle, inbox, owned.then_some(&lock), None).unwrap_err();

            if owned {
                assert!(error.message.contains(&path.display().to_string()));
                assert_eq!(lock, FileHandle::from_path(&path).unwrap());
                assert_eq!(std::fs::read(&path).unwrap(), bytes);
                assert!(Workspace::create_at(&root, "Retry", "New request", TargetRef::None, Some(ExistingFiles::Edit)).is_err());
                continue;
            }

            #[cfg(not(windows))]
            let kept = std::fs::read_dir(&root)
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.path().join("reservation"))
                .find(|path| path.join(crate::feed::PROJECT_LOCK_FILE).is_file())
                .expect("an unowned or changed lock is retained in the reported private folder");
            #[cfg(windows)]
            let kept = std::fs::read_dir(&root)
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| path.join("coordination-0").is_file())
                .expect("the unowned locked inode remains in the reported private folder");
            #[cfg(not(windows))]
            let retained = kept.join(crate::feed::PROJECT_LOCK_FILE);
            #[cfg(windows)]
            let retained = kept.join("coordination-0");
            assert!(error.message.contains(&kept.display().to_string()));
            assert_eq!(lock, FileHandle::from_path(&retained).unwrap());
            assert_eq!(std::fs::read(&retained).unwrap(), bytes);
            assert!(!root.join("inbox").exists());
            let retry = Workspace::create_at(&root, "Retry", "New request", TargetRef::None, Some(ExistingFiles::Edit)).unwrap_err();
            assert!(retry.for_people().contains("may contain retained project records"));
            assert!(!root.join(MANIFEST).exists());
            assert_eq!(std::fs::read(&retained).unwrap(), bytes);
            #[cfg(not(windows))]
            let private = kept.parent().unwrap();
            #[cfg(windows)]
            let private = kept.as_path();
            let relative = retained.strip_prefix(private).unwrap();
            let safe = directory.path().join("preserved-coordination");
            drop(lock);
            std::fs::rename(private, &safe).unwrap();
            assert_eq!(std::fs::read(safe.join(relative)).unwrap(), bytes);
            Workspace::create_at(&root, "Retry", "New request", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        }
    }

    #[test]
    fn failed_initialization_does_not_claim_a_replacement_inbox() {
        let directory = tempfile::tempdir().unwrap();
        let (root, handle) = create_workspace_root(directory.path(), "New").unwrap();
        std::fs::create_dir(root.join("inbox")).unwrap();
        let inbox = FileHandle::from_path(root.join("inbox")).unwrap();
        let displaced = directory.path().join("displaced-inbox");
        std::fs::rename(root.join("inbox"), &displaced).unwrap();
        std::fs::create_dir(root.join("inbox")).unwrap();
        std::fs::write(root.join("inbox/personal.txt"), b"Keep the replacement").unwrap();

        assert_eq!(release_created_inbox(&root, handle, inbox, None, None).unwrap_err().code, ErrorCode::FileChanged);

        assert_eq!(std::fs::read(root.join("inbox/personal.txt")).unwrap(), b"Keep the replacement");
        assert!(displaced.is_dir());
    }

    #[test]
    fn copy_cleanup_preserves_a_replacement_after_identity_validation() {
        let directory = tempfile::tempdir().unwrap();
        let destination = CopyDestination::create(directory.path(), "Copy").unwrap();
        let displaced = directory.path().join("displaced-copy");
        let root = destination.root.clone();
        let result = destination.remove_after_check(|| {
            std::fs::rename(&root, &displaced).unwrap();
            std::fs::create_dir(&root).unwrap();
            std::fs::write(root.join("personal.txt"), b"Keep this replacement").unwrap();
        });

        assert!(result.is_err(), "cleanup cannot remove a replacement using the earlier pathname check");
        assert!(displaced.is_dir());
        let kept = std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.path().join("personal.txt").is_file() || entry.path().join("claimed/personal.txt").is_file());
        assert!(kept, "the replacement survives at its original or explicitly reported private location");
    }

    #[test]
    fn a_replaced_copy_destination_is_preserved_when_cleanup_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("index.html"), "Keep my app").unwrap();
        let ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        let displaced = directory.path().join("displaced-copy");

        let result = ws.duplicate_with_hooks(
            "Copy",
            |_| std::fs::write(original.join("feed.json.broken"), b"pending recovery copy").unwrap(),
            |destination| {
                assert!(!destination.join(MANIFEST).exists(), "the manifest is not published before copying finishes");
                assert_eq!(Workspace::list(directory.path()).len(), 1);
                rename_creation_root_for_replacement(destination, &displaced);
                std::fs::create_dir(destination).unwrap();
                std::fs::write(destination.join("personal.txt"), b"Keep the replacement directory").unwrap();
            },
        );

        let error = result.unwrap_err();
        assert_eq!(error.code, ErrorCode::FileChanged);
        assert!(error.remediation.as_deref().unwrap().contains("left alone"));
        assert_eq!(std::fs::read(directory.path().join("copy/personal.txt")).unwrap(), b"Keep the replacement directory");
        assert!(displaced.is_dir(), "the displaced original destination is not removed by a stale pathname");
        assert_eq!(std::fs::read(original.join("index.html")).unwrap(), b"Keep my app");
        assert_eq!(Workspace::list(directory.path()).len(), 1);
    }

    #[test]
    fn duplication_keeps_a_distinct_case_sensitive_personal_backup_name() {
        use std::io::Write as _;
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("index.html"), "Keep my app").unwrap();
        let ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        let bytes = b"damaged conversation";
        std::fs::write(original.join(crate::feed::FEED_FILE), bytes).unwrap();
        crate::feed::save(&original, &mut crate::feed::load(&ws)).unwrap();
        let mut personal = match std::fs::OpenOptions::new().write(true).create_new(true).open(original.join("FEED.JSON.BROKEN")) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return, // This filesystem treats it as the reserved slot.
            Err(error) => panic!("could not create the independent personal file: {error}"),
        };
        personal.write_all(bytes).unwrap();
        drop(personal);

        let copy = ws.duplicate("Copy").unwrap();

        assert_eq!(std::fs::read(copy.app_dir().join("FEED.JSON.BROKEN")).unwrap(), bytes, "matching bytes and letters do not establish ownership");
        assert!(!copy.app_dir().join("feed.json.broken").exists());
    }

    #[test]
    fn duplication_classifies_case_renamed_backups_after_discovery() {
        for case in ["owned", "personal", "missing-proof", "replaced-proof", "unrecognized-receipt"] {
            let directory = tempfile::tempdir().unwrap();
            let original = directory.path().join("original");
            std::fs::create_dir(&original).unwrap();
            std::fs::write(original.join("index.html"), b"Keep my app").unwrap();
            let ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
            let bytes = b"damaged conversation";
            std::fs::write(original.join(crate::feed::FEED_FILE), bytes).unwrap();
            crate::feed::save(&original, &mut crate::feed::load(&ws)).unwrap();
            let reserved = original.join("feed.json.broken");
            let alias = original.join("FEED.JSON.BROKEN");
            if case == "personal" {
                use std::io::Write as _;
                match std::fs::File::create_new(&alias) {
                    Ok(mut personal) => personal.write_all(bytes).unwrap(),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue, // A distinct case variant needs a case-sensitive filesystem.
                    Err(error) => panic!("could not create the independent personal file: {error}"),
                }
            }
            let result = ws.duplicate_after_discovery("Copy", |selected| {
                assert_eq!(selected, std::slice::from_ref(&reserved));
                if case == "personal" {
                    std::fs::remove_file(&reserved).unwrap();
                } else {
                    let temporary = directory.path().join("renaming-backup");
                    std::fs::rename(&reserved, &temporary).unwrap();
                    std::fs::rename(&temporary, &alias).unwrap();
                }
                if case == "missing-proof" {
                    std::fs::remove_file(original.join("inbox/feed-backups/pending/feed.json.broken")).unwrap();
                } else if case == "replaced-proof" {
                    let staged = original.join("inbox/feed-backups/pending/feed.json.broken");
                    std::fs::rename(&staged, directory.path().join("retained-proof")).unwrap();
                    std::fs::write(&staged, bytes).unwrap();
                } else if case == "unrecognized-receipt" {
                    std::fs::write(original.join("inbox/feed-backups/feed.json.broken.json"), b"unrecognized record").unwrap();
                }
            });

            if matches!(case, "missing-proof" | "replaced-proof" | "unrecognized-receipt") {
                assert_eq!(result.unwrap_err().code, ErrorCode::FileChanged, "uncertain recovery content must not enter the app copy");
                assert_eq!(Workspace::list(directory.path()).len(), 1);
            } else {
                let copy = result.unwrap();
                if case == "personal" {
                    assert_eq!(std::fs::read(copy.app_dir().join("FEED.JSON.BROKEN")).unwrap(), bytes);
                } else {
                    assert!(!copy.app_dir().join("FEED.JSON.BROKEN").exists(), "renaming a verified recovery file does not turn it into personal app data");
                }
            }
            assert_eq!(std::fs::read(&alias).unwrap(), bytes, "source contents are always retained");
        }
    }

    #[cfg(windows)]
    #[test]
    fn duplication_excludes_a_case_alias_of_a_generated_backup() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("index.html"), "Keep my app").unwrap();
        let ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        std::fs::write(original.join(crate::feed::FEED_FILE), b"damaged conversation").unwrap();
        crate::feed::save(&original, &mut crate::feed::load(&ws)).unwrap();
        let reserved = original.join("feed.json.broken");
        let renamed = original.join("FEED.JSON.BROKEN");
        let temporary = original.join("renaming-backup");
        std::fs::rename(&reserved, &temporary).unwrap();
        std::fs::rename(&temporary, &renamed).unwrap();

        let copy = ws.duplicate("Copy").unwrap();

        assert!(!copy.app_dir().join("FEED.JSON.BROKEN").exists());
        assert_eq!(std::fs::read(&renamed).unwrap(), b"damaged conversation");
    }

    #[test]
    fn duplication_excludes_a_same_file_backup_alias() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("index.html"), "Keep my app").unwrap();
        let ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
        std::fs::write(original.join(crate::feed::FEED_FILE), b"damaged conversation").unwrap();
        crate::feed::save(&original, &mut crate::feed::load(&ws)).unwrap();
        match std::fs::hard_link(original.join("feed.json.broken"), original.join("FEED.JSON.BROKEN")) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return,
            Err(error) => panic!("could not create the same-file alias: {error}"),
        }

        let copy = ws.duplicate("Copy").unwrap();

        assert!(!copy.app_dir().join("FEED.JSON.BROKEN").exists(), "another spelling of the same generated file is not personal app content");
        assert!(!copy.app_dir().join("feed.json.broken").exists());
    }

    #[test]
    fn duplication_does_not_copy_a_known_backup_after_its_receipt_disappears() {
        for missing in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let original = directory.path().join("original");
            std::fs::create_dir(&original).unwrap();
            std::fs::write(original.join("index.html"), "Keep my app").unwrap();
            let ws = Workspace::create_at(&original, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).unwrap();
            std::fs::write(original.join(crate::feed::FEED_FILE), b"damaged conversation").unwrap();
            crate::feed::save(&original, &mut crate::feed::load(&ws)).unwrap();
            let backup = original.join("feed.json.broken");
            let receipt = original.join("inbox/feed-backups/feed.json.broken.json");
            let ownership = std::fs::read(&receipt).unwrap();

            let result = ws.duplicate_after_discovery("Copy", |selected| {
                assert_eq!(selected, std::slice::from_ref(&backup));
                if missing {
                    std::fs::remove_file(&receipt).unwrap();
                } else {
                    std::fs::write(&receipt, b"unrecognized record").unwrap();
                }
            });

            match result {
                Ok(copy) => {
                    assert!(!copy.app_dir().join("feed.json.broken").exists(), "losing a receipt must not turn known conversation bytes into app content")
                }
                Err(error) => assert_eq!(error.code, ErrorCode::FileChanged),
            }
            assert_eq!(std::fs::read(&backup).unwrap(), b"damaged conversation");
            std::fs::write(&receipt, ownership).unwrap();
            let copy = ws.duplicate("Retry").unwrap();
            assert!(!copy.app_dir().join("feed.json.broken").exists());
        }
    }

    #[test]
    fn duplication_rejects_a_backup_entry_replaced_after_opening() {
        let directory = tempfile::tempdir().unwrap();
        let backup = directory.path().join("feed.json.broken");
        std::fs::write(&backup, b"personal content").unwrap();
        let (_opened, metadata) = craft_anything_platform::fs::open_regular_file(&backup).unwrap();
        std::fs::rename(&backup, directory.path().join("personal.txt")).unwrap();
        std::fs::create_dir(&backup).unwrap();
        std::fs::write(backup.join("notes.txt"), b"Keep my notes").unwrap();

        assert_eq!(check_copy_entry(&backup, &metadata).unwrap_err().code, ErrorCode::FileChanged);
        assert_eq!(std::fs::read(backup.join("notes.txt")).unwrap(), b"Keep my notes");
        assert_eq!(std::fs::read(directory.path().join("personal.txt")).unwrap(), b"personal content");
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_reserved_link_is_a_clash_before_project_creation() {
        for name in crate::feed::DAMAGED_BACKUP_NAMES.iter().copied().chain([MANIFEST, crate::feed::FEED_FILE]) {
            for existing in [ExistingFiles::Edit, ExistingFiles::StartFrom] {
                let directory = tempfile::tempdir().unwrap();
                let link = directory.path().join(name);
                let target = Path::new("not-created");
                std::os::unix::fs::symlink(target, &link).unwrap();

                let result = Workspace::create_at(directory.path(), "Site", "A website", TargetRef::None, Some(existing));

                assert!(result.is_err(), "{name} is an existing directory entry even when its target is absent");
                assert_eq!(clashes(directory.path(), existing), [name]);
                assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
                assert_eq!(std::fs::read_link(&link).unwrap(), target);
                assert!(!directory.path().join(target).exists());
                assert!(!directory.path().join(MANIFEST).is_file());
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_unverifiable_reserved_entry_does_not_look_absent() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("not-a-folder");
        std::fs::write(&file, b"Keep this file").unwrap();

        assert!(checked_clashes(&file, ExistingFiles::Edit).is_err(), "metadata errors other than NotFound must propagate");
        assert!(!clashes(&file, ExistingFiles::Edit).is_empty(), "the folder chooser must not treat an error as an empty location");
        assert!(Workspace::create_at(&file, "Site", "A website", TargetRef::None, Some(ExistingFiles::Edit)).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"Keep this file");
    }

    #[test]
    fn normalized_resolves_dot_and_parent_components() {
        let root = std::env::current_dir().unwrap();
        assert_eq!(normalized(&root.join("a/../b/./c")), root.join("b/c"));
    }
}
