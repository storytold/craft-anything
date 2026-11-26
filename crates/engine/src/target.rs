//! Targets: what we're looking at, loaded once, identified by content hash.

use craft_anything_evidence::{ErrorCode, Subject, ToolError, sha256_hex};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Default cap, and the cap when `[limits] max_target_bytes` is omitted or zero.
/// A setting may raise or lower it. The result is the upper bound on every OS:
/// it does not depend on available memory, and it is never unlimited.
pub const MAX_TARGET_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// The cap for one session, in bytes. `None` and `Some(0)` are [`MAX_TARGET_BYTES`].
/// `Some(u64::MAX)` is the same: a setting cannot turn the cap off.
/// The value belongs to the settings a session was given.
pub fn effective_max_target_bytes(configured: Option<u64>) -> u64 {
    match configured {
        Some(n) if n > 0 && n < u64::MAX => n,
        _ => MAX_TARGET_BYTES,
    }
}

/// Bytes a reader may take. A reported length of zero is not an empty file and not
/// permission to ignore `limit`: a virtual file that keeps producing data still stops.
/// `u64::MAX` is not a limit, so it is the default cap.
pub(crate) fn read_cap(reported: u64, limit: u64) -> u64 {
    let limit = if limit == u64::MAX { MAX_TARGET_BYTES } else { limit };
    if reported == 0 { limit } else { reported.min(limit) }
}

/// Whether a file of `len` bytes may be opened as a target: under `limit`, on every OS,
/// and within `available` memory when that figure is known. A reported length of zero
/// has not been measured, so only the read cap bounds it. Unknown memory does not drop the cap.
pub fn admit(path: &Path, len: u64, limit: u64, available: Option<u64>) -> Result<(), ToolError> {
    let limit = if limit == u64::MAX { MAX_TARGET_BYTES } else { limit };
    if len > limit {
        return Err(ToolError::new(ErrorCode::OverLimit, format!("{} is {len} bytes; the limit is {limit}", path.display()))
            .with_remediation("Raise `max_target_bytes` under [limits] in config.toml."));
    }
    if len == 0 {
        return Ok(());
    }
    memory_fits(path, len, available)
}

/// Memory kept free beyond what loading needs.
const MEMORY_MARGIN: u64 = 512 * 1024 * 1024;

/// Loading a file of `len` bytes holds it twice for a moment (the read buffer, then the shared
/// copy every tool reads). Refuse, with a clear error, when this computer reports that it
/// doesn't have that much memory available. This is extra: the size cap applies on every OS,
/// including where available memory cannot be read.
pub fn check_memory_for(path: &Path, len: u64) -> Result<(), ToolError> {
    let available = std::fs::read_to_string("/proc/meminfo").ok().as_deref().and_then(mem_available);
    memory_fits(path, len, available)
}

fn memory_fits(path: &Path, len: u64, available: Option<u64>) -> Result<(), ToolError> {
    let Some(available) = available else { return Ok(()) };
    let needed = len.saturating_mul(2).saturating_add(MEMORY_MARGIN);
    if needed <= available {
        return Ok(());
    }
    let gib = |b: u64| b as f64 / (1024.0 * 1024.0 * 1024.0);
    Err(ToolError::new(
        ErrorCode::OverLimit,
        format!(
            "{} is {:.1} GiB; opening it needs about {:.1} GiB of free memory and {:.1} GiB is available",
            path.display(),
            gib(len),
            gib(needed),
            gib(available)
        ),
    )
    .with_remediation(
        "Free memory, or take out the part you need first (for a disk image: qemu-img convert -O raw, then extract one partition or its files), and open that.",
    ))
}

/// Read a regular file that [`admit`] allows for `limit` and the memory available now.
/// A path that is not a regular file is left to the reader, which reports that.
pub fn read_admitted(path: &Path, limit: u64) -> Result<Vec<u8>, ToolError> {
    let available = std::fs::read_to_string("/proc/meminfo").ok().as_deref().and_then(mem_available);
    read_admitted_with(path, limit, available)
}

pub(crate) fn read_admitted_with(path: &Path, limit: u64, available: Option<u64>) -> Result<Vec<u8>, ToolError> {
    let (mut file, metadata) = craft_anything_platform::fs::open_regular_file(path)?;
    let reported = metadata.len();
    admit(path, reported, limit, available)?;
    // A reported size of zero can still be a readable virtual file. The cap stays finite either way,
    // and a known size is read only up to the length just admitted on this descriptor.
    let cap = read_cap(reported, limit);
    craft_anything_platform::fs::read_opened_capped(path, &mut file, reported, cap)
}

/// `MemAvailable` from `/proc/meminfo`, in bytes.
fn mem_available(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find(|l| l.starts_with("MemAvailable:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

/// SHA-256 of a file, read in pieces so a big file isn't held in memory a second time.
pub fn sha256_file(path: &Path) -> Result<String, ToolError> {
    use sha2::Digest as _;
    use std::io::Read as _;
    let (mut file, _) = craft_anything_platform::fs::open_regular_file(path)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).map_err(|e| ToolError::io(path.display(), e))?;
        if n == 0 {
            break;
        }
        hasher.update(buf.get(..n).unwrap_or_default());
    }
    Ok(hex::encode(hasher.finalize()))
}

pub type TargetId = String;

/// Metadata handed to an agent's MCP session. No file contents or caches cross the command
/// line; Deep mode reloads and verifies the file, while Clean room keeps only this metadata.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetContext {
    pub path: PathBuf,
    pub sha256: String,
    pub kind: TargetKind,
    pub raw: Option<RawSpec>,
}

impl TargetContext {
    pub fn from_target(target: &Target) -> Self {
        Self { path: target.path.clone(), sha256: target.sha256.clone(), kind: target.kind, raw: target.raw.clone() }
    }

    pub fn open(&self, clean_room: bool, limit: u64) -> Result<Target, ToolError> {
        if !self.path.is_absolute() || self.sha256.len() != 64 || !self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ToolError::invalid("A shared target needs an absolute path and a SHA-256 hash."));
        }
        let mut target = if clean_room {
            Target {
                id: format!("t_{}", self.sha256.get(..12).unwrap_or_default()),
                path: self.path.clone(),
                name: self.path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                sha256: self.sha256.clone(),
                kind: self.kind,
                bytes: Arc::from([]),
                raw: self.raw.clone(),
            }
        } else {
            let target = Target::load_limited(&self.path, limit)?;
            if target.sha256 != self.sha256 {
                return Err(ToolError::invalid("The shared target changed after it was opened.")
                    .with_remediation("Reopen the target in Craft Anything, then start a new conversation."));
            }
            target
        };
        if self.raw.is_some() || self.kind == TargetKind::DosCom {
            target.kind = self.kind;
            target.raw = self.raw.clone();
        }
        Ok(target)
    }
}

/// What kind of thing a target is. Detected from content, never from the extension alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TargetKind {
    Elf,
    Pe,
    MachO,
    /// A fat/universal Mach-O.
    MachOFat,
    /// A .NET assembly (PE with a CLI header).
    Managed,
    Wasm,
    Apk,
    Dex,
    Jar,
    /// A single Java `.class` file.
    JavaClass,
    Zip,
    Asar,
    /// A folder: an app bundle, an unpacked Electron app, a project.
    Directory,
    /// A JavaScript or HTML file.
    Script,
    /// A HAR or other network capture.
    NetworkCapture,
    Plist,
    /// EVM bytecode.
    Evm,
    /// A SQLite 3 database file.
    Sqlite,
    /// A DOS .COM program.
    DosCom,
    /// Raw bytes with a user-declared architecture (firmware, shellcode).
    RawBinary,
    /// Plain text, such as notes, source files and settings.
    Text,
    Unknown,
}

impl TargetKind {
    /// Plain-English description for Beginner mode.
    pub fn plain(self) -> &'static str {
        match self {
            TargetKind::Elf => "A Linux (or Unix) program",
            TargetKind::Pe => "A Windows program",
            TargetKind::MachO | TargetKind::MachOFat => "A Mac or iPhone program",
            TargetKind::Managed => "A .NET program (C#, F# or VB)",
            TargetKind::Wasm => "A WebAssembly module (code that runs in a browser)",
            TargetKind::Apk => "An Android app",
            TargetKind::Dex => "Android app code (DEX)",
            TargetKind::Jar => "A Java program",
            TargetKind::JavaClass => "Java program code (one class)",
            TargetKind::Zip => "A ZIP archive",
            TargetKind::Asar => "The packed contents of an Electron app (a website inside a desktop app)",
            TargetKind::Directory => "A folder",
            TargetKind::Script => "A script or web page",
            TargetKind::NetworkCapture => "A recording of network traffic",
            TargetKind::Plist => "An Apple settings file (plist)",
            TargetKind::Evm => "A smart contract (EVM bytecode)",
            TargetKind::Sqlite => "A SQLite database (how an app stores its data)",
            TargetKind::DosCom => "A DOS program",
            TargetKind::RawBinary => "Raw machine code",
            TargetKind::Text => "A text file (notes, source code or settings)",
            TargetKind::Unknown => "A file we don't recognise yet",
        }
    }

    pub fn id(self) -> String {
        serde_json::to_value(self).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| "unknown".into())
    }
}

/// Whether an otherwise unknown file looks like text. Inspect at most 8 KiB:
/// a byte-order mark counts as text, otherwise require valid UTF-8, no NULs
/// and few control bytes. Empty input is not text.
pub fn looks_like_text(bytes: &[u8]) -> bool {
    const SNIFF: usize = 8 * 1024;
    if bytes.starts_with(b"\xEF\xBB\xBF") || bytes.starts_with(b"\xFF\xFE") || bytes.starts_with(b"\xFE\xFF") {
        return true;
    }
    let head = bytes.get(..bytes.len().min(SNIFF)).unwrap_or_default();
    if head.is_empty() || head.contains(&0) {
        return false;
    }
    let cut_at_end = bytes.len() > SNIFF;
    let mut chunks = head.utf8_chunks().peekable();
    while let Some(chunk) = chunks.next() {
        // The window may end inside a character; only that last sequence may be incomplete.
        let last = chunks.peek().is_none();
        if !chunk.invalid().is_empty() && !(last && cut_at_end && chunk.invalid().len() < 4) {
            return false;
        }
    }
    let control = head.iter().filter(|b| b.is_ascii_control() && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0c | 0x1b)).count();
    control.saturating_mul(100) <= head.len()
}

/// Sniff the kind from the first bytes (and the name, only to break ties).
///
/// Signatures come from the shared `craft_sniff` table; the tie-breaks that need our own parsers
/// (managed PE, Java class or universal Mach-O, APK by its manifest) and the kinds with no
/// signature (ASAR, XML plists, captures, scripts, text) stay here.
pub fn sniff(bytes: &[u8], name: &str) -> TargetKind {
    use craft_sniff::Kind;
    let lower = name.to_ascii_lowercase();
    match craft_sniff::identify(bytes) {
        Some(Kind::Sqlite) => TargetKind::Sqlite,
        Some(Kind::Elf) => TargetKind::Elf,
        Some(Kind::Pe | Kind::DosExecutable) => {
            if craft_anything_formats::pe_is_managed(bytes) {
                TargetKind::Managed
            } else {
                TargetKind::Pe
            }
        }
        Some(Kind::MachO) => TargetKind::MachO,
        // Both a Java class and a universal Mach-O start 0xCAFEBABE. A class has a plausible
        // major version where a fat binary has a small big-endian architecture count, so prefer
        // the .class extension, then the version check.
        Some(Kind::MachOFat | Kind::JavaClass) => {
            if lower.ends_with(".class") || craft_anything_formats::java_class::is_class(bytes) {
                TargetKind::JavaClass
            } else {
                TargetKind::MachOFat
            }
        }
        Some(Kind::Wasm) => TargetKind::Wasm,
        Some(Kind::Dex) => TargetKind::Dex,
        Some(Kind::Zip) => sniff_zip(bytes, &lower),
        Some(Kind::BinaryPlist) => TargetKind::Plist,
        Some(_) => TargetKind::Unknown,
        None => sniff_unsigned(bytes, &lower),
    }
}

/// A ZIP is an APK or a JAR by its name, then an APK by its manifest.
fn sniff_zip(bytes: &[u8], lower: &str) -> TargetKind {
    if lower.ends_with(".apk") || lower.ends_with(".aab") || lower.ends_with(".xapk") {
        TargetKind::Apk
    } else if lower.ends_with(".jar") || lower.ends_with(".war") {
        TargetKind::Jar
    } else if craft_anything_formats::zip_has_entry(bytes, "AndroidManifest.xml") {
        TargetKind::Apk
    } else {
        TargetKind::Zip
    }
}

/// The kinds the shared table has no signature for, told apart by name and content.
fn sniff_unsigned(bytes: &[u8], lower: &str) -> TargetKind {
    if lower.ends_with(".asar") || craft_anything_formats::looks_like_asar(bytes) {
        TargetKind::Asar
    } else if lower.ends_with(".plist") && bytes.windows(6).take(512).any(|w| w == b"<plist") {
        TargetKind::Plist
    } else if lower.ends_with(".har") {
        TargetKind::NetworkCapture
    } else if lower.ends_with(".com") && bytes.len() <= 0xff00 {
        TargetKind::DosCom
    } else if [".js", ".mjs", ".cjs", ".html", ".htm", ".ts"].iter().any(|e| lower.ends_with(e)) {
        TargetKind::Script
    } else if looks_like_text(bytes) {
        TargetKind::Text
    } else {
        TargetKind::Unknown
    }
}

/// A loaded target.
#[derive(Debug, Clone)]
pub struct Target {
    /// `t_` + first 12 hex of the sha256 (+ a suffix if the same file is opened twice).
    pub id: TargetId,
    pub path: PathBuf,
    pub name: String,
    pub sha256: String,
    pub kind: TargetKind,
    /// File contents (empty for directories).
    pub bytes: Arc<[u8]>,
    /// For raw binaries: declared architecture and entry point.
    pub raw: Option<RawSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RawSpec {
    /// `x86`, `x86_64`, `arm`, `arm64`.
    pub architecture: String,
    /// Load and entry address.
    pub entry_point: u64,
}

impl Target {
    /// Load a file or directory with the default cap ([`MAX_TARGET_BYTES`]).
    pub fn load(path: &Path) -> Result<Target, ToolError> {
        Self::load_limited(path, effective_max_target_bytes(None))
    }

    /// Load a file or directory, refusing one bigger than `limit` or too large for free memory.
    pub fn load_limited(path: &Path, limit: u64) -> Result<Target, ToolError> {
        let meta = std::fs::metadata(path).map_err(|e| ToolError::io(path.display(), e))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
        if meta.is_dir() {
            let digest = sha256_hex(path.to_string_lossy().as_bytes());
            return Ok(Target {
                id: format!("t_{}", digest.get(..12).unwrap_or(&digest)),
                path: path.to_path_buf(),
                name,
                sha256: digest,
                kind: TargetKind::Directory,
                bytes: Arc::from(Vec::new()),
                raw: None,
            });
        }
        let bytes = read_admitted(path, limit)?;
        Ok(Target::from_bytes(path.to_path_buf(), name, bytes))
    }

    pub fn from_bytes(path: PathBuf, name: String, bytes: Vec<u8>) -> Target {
        let sha256 = sha256_hex(&bytes);
        let kind = sniff(&bytes, &name);
        Target { id: format!("t_{}", sha256.get(..12).unwrap_or(&sha256)), path, name, sha256, kind, bytes: Arc::from(bytes), raw: None }
    }

    pub fn subject(&self) -> Subject {
        Subject { kind: self.kind.id(), sha256: Some(self.sha256.clone()), name: Some(self.name.clone()) }
    }

    /// Fail if the file on disk no longer matches what we loaded.
    pub fn check_unchanged(&self) -> Result<(), ToolError> {
        if self.kind == TargetKind::Directory {
            return Ok(());
        }
        // Hashed in pieces: re-reading a big target whole would hold it in memory twice more.
        match sha256_file(&self.path) {
            Ok(h) if h == self.sha256 => Ok(()),
            Ok(_) => Err(ToolError::new(ErrorCode::FileChanged, format!("{} changed on disk since it was opened", self.path.display()))
                .with_remediation("Open it again to analyse the new version.")),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_common_magics() {
        assert_eq!(sniff(b"\x7fELF\x02\x01", "a"), TargetKind::Elf);
        assert_eq!(sniff(b"\0asm\x01\0\0\0", "m"), TargetKind::Wasm);
        assert_eq!(sniff(b"dex\n035\0", "classes.dex"), TargetKind::Dex);
        assert_eq!(sniff(b"PK\x03\x04", "app.apk"), TargetKind::Apk);
        assert_eq!(sniff(b"bplist00", "Info.plist"), TargetKind::Plist);
        assert_eq!(sniff(b"SQLite format 3\0\x10\x00", "app.db"), TargetKind::Sqlite);
        assert_eq!(sniff(b"\xcf\xfa\xed\xfe\x07\0\0\x01", "tool"), TargetKind::MachO);
        assert_eq!(sniff(b"\xca\xfe\xba\xbe\0\0\0\x02", "tool"), TargetKind::MachOFat);
        assert_eq!(sniff(b"\xca\xfe\xba\xbf\0\0\0\x02", "tool"), TargetKind::MachOFat);
        assert_eq!(sniff(b"\xca\xfe\xba\xbe\0\0\0\x41", "Main"), TargetKind::JavaClass);
        assert_eq!(sniff(b"\xca\xfe\xba\xbe\0\0\0\x02", "Main.class"), TargetKind::JavaClass);
        assert_eq!(sniff(b"MZ\x90\0", "setup.exe"), TargetKind::Pe);
        assert_eq!(sniff(b"PK\x03\x04", "lib.jar"), TargetKind::Jar);
        assert_eq!(sniff(b"PK\x05\x06\0\0\0\0", "empty.zip"), TargetKind::Zip);
        assert_eq!(sniff(b"<?xml?><plist version=\"1.0\"/>", "Info.plist"), TargetKind::Plist);
        // Media and archives the shared table names are not targets we inspect by kind.
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR", "icon.png"), TargetKind::Unknown);
        assert_eq!(sniff(b"070701", "archive.cpio"), TargetKind::Unknown);
        assert_eq!(sniff(b"", "x"), TargetKind::Unknown);
        assert_eq!(sniff(b"console.log(1)", "main.js"), TargetKind::Script);
        assert_eq!(sniff(b"# Notes\n", "notes.md"), TargetKind::Text);
        assert_eq!(sniff(b"key = true\n", "config.toml"), TargetKind::Text);
        assert_eq!(sniff(b"\xEF\xBB\xBFhello", "notes.txt"), TargetKind::Text);
        assert_eq!(sniff(b"\xFF\xFEh\0i\0", "notes.txt"), TargetKind::Text);
        assert_eq!(sniff(b"\0\xFF\x02", "notes.md"), TargetKind::Unknown);
        // The bytes decide, not the name: a gzip stream called `.txt` is not text.
        assert_eq!(sniff(b"\x1f\x8b\x08\x00\x00\x00", "notes.txt"), TargetKind::Unknown);
        assert_eq!(sniff(b"", "empty.txt"), TargetKind::Unknown);
        assert_eq!(TargetKind::Text.id(), "text");
        assert!(TargetKind::Text.plain().contains("text file"));
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn the_default_cap_holds_on_every_system_and_a_setting_can_move_it() {
        assert_eq!(effective_max_target_bytes(None), MAX_TARGET_BYTES);
        assert_eq!(effective_max_target_bytes(Some(0)), MAX_TARGET_BYTES);
        assert_eq!(effective_max_target_bytes(Some(u64::MAX)), MAX_TARGET_BYTES);
        assert!(effective_max_target_bytes(Some(8)) < MAX_TARGET_BYTES);
        let raised = MAX_TARGET_BYTES + GIB;
        assert!(effective_max_target_bytes(Some(raised)) > MAX_TARGET_BYTES);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("disk.img");
        let over = MAX_TARGET_BYTES + 1;
        std::fs::File::create(&path).unwrap().set_len(over).unwrap();
        let unset = admit(&path, over, effective_max_target_bytes(None), None).unwrap_err();
        assert_eq!(unset.code, ErrorCode::OverLimit);
        assert!(unset.remediation.as_deref().unwrap_or_default().contains("max_target_bytes"), "{unset:?}");
        // Plenty of memory, and unknown memory, both still stop at the default cap.
        assert_eq!(admit(&path, over, u64::MAX, Some(64 * GIB)).unwrap_err().code, ErrorCode::OverLimit);
        assert_eq!(admit(&path, over, u64::MAX, None).unwrap_err().code, ErrorCode::OverLimit);
        admit(&path, over, raised, None).expect("a raised setting is the bound when memory cannot be read");
        admit(&path, over, raised, Some(64 * GIB)).expect("a raised setting admits the file when memory allows");
    }

    #[test]
    fn a_reported_length_of_zero_does_not_drop_the_cap() {
        assert_eq!(read_cap(0, u64::MAX), MAX_TARGET_BYTES);
        assert_eq!(read_cap(0, 0), 0, "an explicit empty budget stays empty");
        assert_eq!(read_cap(0, 64), 64);
        assert!(read_cap(100, 64) <= 64);
        assert_eq!(read_cap(100, MAX_TARGET_BYTES), 100);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_virtual_file_that_reports_zero_bytes_stops_at_the_cap() {
        let path = Path::new("/proc/self/status");
        let meta = std::fs::metadata(path).expect("status");
        assert_eq!(meta.len(), 0);
        assert!(meta.is_file());
        let err = read_admitted_with(path, 16, None).unwrap_err();
        assert_eq!(err.code, ErrorCode::OverLimit);
        let bytes = read_admitted_with(path, u64::MAX, None).unwrap();
        assert!(bytes.len() > 16);
        assert!((bytes.len() as u64) < MAX_TARGET_BYTES);
    }

    #[test]
    fn a_file_this_computer_cannot_hold_is_refused_with_a_clear_error_not_a_crash() {
        let e = admit(Path::new("/x/disk.qcow2"), 10 * GIB, 16 * GIB, Some(8 * GIB)).unwrap_err();
        assert_eq!(e.code, ErrorCode::OverLimit);
        assert!(e.message.contains("free memory"), "{}", e.message);
        assert!(e.remediation.as_deref().unwrap_or_default().contains("qemu-img"), "{e:?}");
    }

    #[test]
    fn mem_available_is_read_from_proc_meminfo() {
        let text = "MemTotal:       16000000 kB\nMemFree:  1 kB\nMemAvailable:   11534336 kB\n";
        assert_eq!(mem_available(text), Some(11 * GIB));
        assert_eq!(mem_available("MemTotal: 1 kB\n"), None);
    }

    #[test]
    fn each_session_applies_its_own_limit() {
        use crate::session::Session;
        use crate::settings::{LimitSettings, Settings};
        let capped = Settings { limits: LimitSettings { max_target_bytes: Some(8) }, ..Settings::default() };
        let open = Session::new(Settings::default());
        let tight = Session::new(capped);
        assert_eq!(open.max_target_bytes(), MAX_TARGET_BYTES);
        assert_eq!(tight.max_target_bytes(), 8);
        // Building the default session does not loosen the capped one.
        assert_eq!(tight.max_target_bytes(), 8);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.bin");
        std::fs::write(&path, b"0123456789").unwrap();
        assert_eq!(Target::load_limited(&path, 8).unwrap_err().code, ErrorCode::OverLimit);
        assert_eq!(Target::load_limited(&path, 1024).unwrap().bytes.len(), 10);
    }

    #[test]
    fn a_raised_limit_still_refuses_a_file_memory_cannot_hold() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        // Logical size only: admission must fail before the file is read into memory.
        std::fs::File::create(&path).unwrap().set_len(3 * GIB).unwrap();
        let err = read_admitted_with(&path, 8 * GIB, Some(4 * GIB)).unwrap_err();
        assert_eq!(err.code, ErrorCode::OverLimit);
        assert!(err.message.contains("free memory"), "{}", err.message);
        let by_cap = read_admitted_with(&path, MAX_TARGET_BYTES, Some(64 * GIB)).unwrap_err();
        assert_eq!(by_cap.code, ErrorCode::OverLimit);
        assert!(!by_cap.message.contains("free memory"), "{}", by_cap.message);
        assert_eq!(read_admitted_with(&path, u64::MAX, None).unwrap_err().code, ErrorCode::OverLimit);
        let small = dir.path().join("small.bin");
        std::fs::write(&small, b"ok").unwrap();
        assert_eq!(read_admitted_with(&small, MAX_TARGET_BYTES, Some(4 * GIB)).unwrap(), b"ok");
    }

    #[test]
    fn a_file_is_hashed_in_pieces_the_same_as_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.bin");
        let bytes: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(sha256_file(&path).unwrap(), sha256_hex(&bytes));
    }
}
