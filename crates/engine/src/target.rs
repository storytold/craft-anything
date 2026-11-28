//! Targets: what we're looking at, loaded once, identified by content hash.

use craft_anything_evidence::{ErrorCode, Subject, ToolError, sha256_hex};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Biggest file we load into memory. Bigger inputs get a clear error rather than an OOM.
pub const MAX_TARGET_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Fixed chunk size used when streaming a file through SHA-256 to avoid buffering the whole file.
const HASH_CHUNK_BYTES: usize = 64 * 1024;

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

    pub fn open(&self, clean_room: bool) -> Result<Target, ToolError> {
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
            let target = Target::load(&self.path)?;
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
    /// Load a file or directory.
    pub fn load(path: &Path) -> Result<Target, ToolError> {
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
        let bytes = craft_anything_platform::fs::read_file_capped(path, MAX_TARGET_BYTES)?;
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
        self.check_unchanged_with_cap(MAX_TARGET_BYTES)
    }

    /// Fail if the file on disk no longer matches what we loaded, refusing files larger than `cap`.
    fn check_unchanged_with_cap(&self, cap: u64) -> Result<(), ToolError> {
        if self.kind == TargetKind::Directory {
            return Ok(());
        }
        let digest = hash_file_capped(&self.path, cap)?;
        if digest == self.sha256 {
            Ok(())
        } else {
            Err(ToolError::new(ErrorCode::FileChanged, format!("{} changed on disk since it was opened", self.path.display()))
                .with_remediation("Open it again to analyse the new version."))
        }
    }
}

/// Build the `OverLimit` error returned when a streamed hash reads past `cap`.
fn hash_read_over_limit(path: &Path, cap: u64) -> ToolError {
    ToolError::new(ErrorCode::OverLimit, format!("{} contains more than {cap} bytes; the read limit was reached", path.display()))
        .with_remediation("Raise the size limit parameter if you really want to read it.")
}

/// Stream up to `cap` bytes from `reader` through SHA-256 and return the lowercase hex digest.
/// Probes one byte past `cap` without overflowing `cap`, returning `OverLimit` if more bytes exist.
fn hash_reader_capped<R: std::io::Read>(reader: &mut R, path: &Path, cap: u64) -> Result<String, ToolError> {
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; HASH_CHUNK_BYTES];
    let mut total: u64 = 0;
    loop {
        let remaining = cap.saturating_sub(total);
        let to_read = if remaining == 0 { 1 } else { usize::try_from(remaining).unwrap_or(chunk.len()).min(chunk.len()) };
        let Some(buf) = chunk.get_mut(..to_read) else {
            return Err(ToolError::io(path.display(), std::io::Error::other("hash chunk slice out of bounds")));
        };
        match reader.read(buf) {
            Ok(0) => return Ok(hex::encode(hasher.finalize())),
            Ok(n) => {
                let Some(slice) = chunk.get(..to_read).and_then(|window| window.get(..n)) else {
                    return Err(ToolError::io(path.display(), std::io::Error::other("reader returned more bytes than requested")));
                };
                if remaining == 0 {
                    return Err(hash_read_over_limit(path, cap));
                }
                let next_total = total.checked_add(n as u64).ok_or_else(|| hash_read_over_limit(path, cap))?;
                if next_total > cap {
                    return Err(hash_read_over_limit(path, cap));
                }
                hasher.update(slice);
                total = next_total;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(ToolError::io(path.display(), error)),
        }
    }
}

/// Hash a regular file in fixed-size chunks, refusing files larger than `cap` bytes either by
/// metadata size or by bytes streamed.
fn hash_file_capped(path: &Path, cap: u64) -> Result<String, ToolError> {
    let (mut file, metadata) = craft_anything_platform::fs::open_regular_file(path)?;
    if metadata.len() > cap {
        return Err(ToolError::new(ErrorCode::OverLimit, format!("{} is {} bytes; the limit is {cap}", path.display(), metadata.len()))
            .with_remediation("Raise the size limit parameter if you really want to read it."));
    }
    hash_reader_capped(&mut file, path, cap)
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

    /// Guards that chunked target rechecking matches `sha256_hex` across multiple chunks, detects
    /// a rewritten file as `FileChanged`, and still accepts directory targets.
    #[test]
    fn recheck_hashes_multi_chunk_file_and_detects_rewrites() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sample.bin");
        let mut bytes: Vec<u8> = (0..(HASH_CHUNK_BYTES * 2 + 333)).map(|i| ((i * 31) % 256) as u8).collect();
        std::fs::write(&file, &bytes).unwrap();

        assert_eq!(hash_file_capped(&file, bytes.len() as u64).unwrap(), sha256_hex(&bytes));
        let target = Target::load(&file).unwrap();
        assert!(target.check_unchanged().is_ok());
        assert!(target.check_unchanged_with_cap(bytes.len() as u64).is_ok());

        bytes[HASH_CHUNK_BYTES + 17] ^= 0xff;
        std::fs::write(&file, &bytes).unwrap();
        let changed = target.check_unchanged().unwrap_err();
        assert_eq!(changed.code, ErrorCode::FileChanged);
        assert!(changed.message.contains("changed on disk"), "unexpected message: {}", changed.message);
        assert!(changed.remediation.as_deref().is_some_and(|r| r.contains("Open it again")));

        let dir_target = Target::load(dir.path()).unwrap();
        assert!(dir_target.check_unchanged().is_ok());
    }

    /// Guards that target rechecking enforces the byte cap both from file metadata and while
    /// streaming past the cap, while accepting files at the exact cap.
    #[test]
    fn recheck_enforces_exact_cap_and_rejects_metadata_and_streaming_overflow() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("capped.bin");
        let payload = vec![0x5au8; HASH_CHUNK_BYTES + 129];
        std::fs::write(&file, &payload).unwrap();

        let target = Target::load(&file).unwrap();
        assert!(target.check_unchanged_with_cap(payload.len() as u64).is_ok());

        let meta_over = target.check_unchanged_with_cap((payload.len() - 1) as u64).unwrap_err();
        assert_eq!(meta_over.code, ErrorCode::OverLimit);
        assert!(meta_over.message.contains("capped.bin"), "unexpected message: {}", meta_over.message);
        assert!(meta_over.message.contains("bytes; the limit is"), "expected metadata pre-check before streaming: {}", meta_over.message);

        let mut exact_stream = std::io::Cursor::new(payload.as_slice());
        assert_eq!(hash_reader_capped(&mut exact_stream, &file, payload.len() as u64).unwrap(), sha256_hex(&payload));

        let mut overflow_stream = std::io::Cursor::new(payload.as_slice());
        let stream_over = hash_reader_capped(&mut overflow_stream, &file, (payload.len() - 1) as u64).unwrap_err();
        assert_eq!(stream_over.code, ErrorCode::OverLimit);
        assert!(stream_over.message.contains("capped.bin"), "unexpected message: {}", stream_over.message);
        assert!(stream_over.message.contains("contains more than"), "expected streaming cap error: {}", stream_over.message);

        let mut empty_stream = std::io::Cursor::new(b"".as_slice());
        assert_eq!(hash_reader_capped(&mut empty_stream, &file, 0).unwrap(), sha256_hex(b""));

        let mut one_byte_stream = std::io::Cursor::new(b"x".as_slice());
        assert_eq!(hash_reader_capped(&mut one_byte_stream, &file, 0).unwrap_err().code, ErrorCode::OverLimit);
    }

    /// Guards that rechecking fails when the opened file is deleted or replaced by a directory.
    #[test]
    fn recheck_reports_missing_and_non_regular_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("gone.bin");
        std::fs::write(&file, b"present").unwrap();
        let target = Target::load(&file).unwrap();

        std::fs::remove_file(&file).unwrap();
        assert_eq!(target.check_unchanged().unwrap_err().code, ErrorCode::NotFound);

        std::fs::create_dir(&file).unwrap();
        assert_eq!(target.check_unchanged().unwrap_err().code, ErrorCode::BadInput);
    }

    /// Streaming reader stub that records requested buffer sizes, injects one `Interrupted` error,
    /// yields short reads, and can fail with a non-`Interrupted` I/O error.
    struct ChunkedTestReader {
        data: Vec<u8>,
        pos: usize,
        max_step: usize,
        interrupted_once: bool,
        fail_after: Option<usize>,
        max_requested_buf: usize,
    }

    impl std::io::Read for ChunkedTestReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.max_requested_buf = self.max_requested_buf.max(buf.len());
            if self.interrupted_once {
                self.interrupted_once = false;
                return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "simulated interrupt"));
            }
            if self.fail_after.is_some_and(|limit| self.pos >= limit) {
                return Err(std::io::Error::other("simulated stream read error"));
            }
            let remaining = self.data.len().saturating_sub(self.pos);
            let n = remaining.min(buf.len()).min(self.max_step);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    /// Guards chunk-bounded reads (`buf.len() <= HASH_CHUNK_BYTES`), `u64::MAX` caps without
    /// overflow, `Interrupted` retries, non-`Interrupted` I/O errors, and stopping at `cap + 1`.
    #[test]
    fn streaming_hash_reads_in_bounded_chunks_retries_interrupts_and_propagates_io_errors() {
        let payload: Vec<u8> = (0..(HASH_CHUNK_BYTES + 257)).map(|i| ((i * 13) % 251) as u8).collect();
        let path = Path::new("stream.bin");

        let mut reader = ChunkedTestReader { data: payload.clone(), pos: 0, max_step: 1024, interrupted_once: true, fail_after: None, max_requested_buf: 0 };
        assert_eq!(hash_reader_capped(&mut reader, path, u64::MAX).unwrap(), sha256_hex(&payload));
        assert!(reader.max_requested_buf <= HASH_CHUNK_BYTES, "read buffer exceeded chunk cap: {}", reader.max_requested_buf);

        let cap = 2000u64;
        let mut over = ChunkedTestReader { data: payload.clone(), pos: 0, max_step: 4096, interrupted_once: false, fail_after: None, max_requested_buf: 0 };
        assert_eq!(hash_reader_capped(&mut over, path, cap).unwrap_err().code, ErrorCode::OverLimit);
        assert_eq!(over.pos, (cap + 1) as usize, "streaming hash must stop immediately after the one-byte overflow probe");

        let mut broken = ChunkedTestReader { data: payload, pos: 0, max_step: 512, interrupted_once: false, fail_after: Some(512), max_requested_buf: 0 };
        let err = hash_reader_capped(&mut broken, path, u64::MAX).unwrap_err();
        assert_eq!(err.code, ErrorCode::FileError);
        assert!(err.message.contains("stream.bin"), "unexpected message: {}", err.message);
    }
}
