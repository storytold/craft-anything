//! One-use consent for a signed-in website study. No approval is kept across runs.
//! Only local project files are previewed; neither previews nor page contents leave this module
//! as tool results. Digests bind the request to the complete capture, target and saved account.
use crate::tools::{
    site_sign_in::{self, SavedSignIn},
    web_common,
    website_tools::shown_url,
};
use crate::workspace::{TargetRef, Workspace};
use craft_anything_evidence::{ErrorCode, ToolError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};

const MAX_FILE: u64 = 32 << 20;
const MAX_CAPTURE: u64 = 512 << 20;
const MAX_FILES: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Screenshot {
    /// Relative to the project's capture folder; never a URI or an absolute path.
    pub file: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureReview {
    /// Address without query or fragment. This is not an assertion of authentication.
    pub website: String,
    pub capture_sha256: String,
    pub screenshots: Vec<Screenshot>,
}

fn problem(message: &str) -> ToolError {
    ToolError::new(ErrorCode::NoPermission, message)
        .with_remediation("Use web_site_sign_in to sign in again if needed, then start a new study. No captured account content was sent to the builder.")
}

/// Resolve a capture member without following links, including a link at capture/ itself.
fn member(root: &Path, relative: &Path) -> Result<PathBuf, ToolError> {
    let mut path = root.join("capture");
    if !std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
        return Err(problem("The study's capture folder is missing or is a link."));
    }
    for part in relative.components() {
        let Component::Normal(part) = part else { return Err(problem("A screenshot path is outside this project's capture folder.")) };
        path.push(part);
        let meta = std::fs::symlink_metadata(&path).map_err(|_| problem("A captured file is missing or cannot be read."))?;
        if meta.file_type().is_symlink() || (!meta.is_dir() && !meta.is_file()) {
            return Err(problem("A captured file is a link or is not a regular file."));
        }
    }
    Ok(path)
}

fn read(root: &Path, relative: &Path) -> Result<Vec<u8>, ToolError> {
    craft_anything_platform::fs::read_file_capped(&member(root, relative)?, MAX_FILE)
        .map_err(|_| problem("A captured file cannot be read within the preview limits."))
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Capture paths and bytes both contribute to the digest; links and non-files are refused.
fn capture_digest(root: &Path, target: &str, revision: &str) -> Result<String, ToolError> {
    let mut pending = vec![PathBuf::new()];
    let mut files = Vec::new();
    let mut entries = 0usize;
    while let Some(dir) = pending.pop() {
        let path = member(root, &dir)?;
        let children = std::fs::read_dir(path).map_err(|_| problem("The captured files cannot be listed."))?;
        for child in children {
            let child = child.map_err(|_| problem("A captured file cannot be listed."))?;
            entries += 1;
            if entries > MAX_FILES {
                return Err(problem("This capture has too many files to confirm safely."));
            }
            let relative = dir.join(child.file_name());
            let path = member(root, &relative)?;
            if path.is_dir() {
                pending.push(relative);
            } else {
                files.push(relative);
            }
        }
    }
    files.sort();
    let mut hash = Sha256::new();
    hash.update(target.as_bytes());
    hash.update([0]);
    hash.update(revision.as_bytes());
    hash.update([0]);
    let mut total = 0u64;
    for file in files {
        let bytes = read(root, &file)?;
        total = total.saturating_add(bytes.len() as u64);
        if total > MAX_CAPTURE {
            return Err(problem("This capture is too large to confirm safely."));
        }
        let name = file.to_str().ok_or_else(|| problem("A capture file name cannot be confirmed safely."))?;
        hash.update(name.as_bytes());
        hash.update([0]);
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(&bytes);
    }
    Ok(hex::encode(hash.finalize()))
}

/// Prepare the approval at the Study → Build boundary. Saved-profile existence is never taken
/// as proof of login. Old signed-in captures without account identity require a fresh study.
pub fn prepare(ws: &Workspace, saved: Option<&SavedSignIn>) -> Result<Option<CaptureReview>, ToolError> {
    let TargetRef::Website { url } = &ws.manifest.target else { return Ok(None) };
    let bytes = read(&ws.root, Path::new("capture.json"))?;
    let metadata: Value = serde_json::from_slice(&bytes).map_err(|_| problem("The study's capture record cannot be read."))?;
    if metadata.get("signed_in").and_then(Value::as_bool) == Some(false) {
        return Ok(None);
    }
    if metadata.get("signed_in").and_then(Value::as_bool) != Some(true) {
        return Err(problem("The study did not record whether it used a saved sign-in."));
    }
    let key = site_sign_in::site_key(url)?;
    let captured_url = metadata.get("url").and_then(Value::as_str).ok_or_else(|| problem("The captured website is missing."))?;
    if metadata.get("signed_in_site").and_then(Value::as_str) != Some(key.as_str()) || site_sign_in::site_key(captured_url)? != key {
        return Err(problem("The capture belongs to a different website."));
    }
    let saved = saved.ok_or_else(|| problem("The sign-in used by this capture is no longer saved."))?;
    if saved.key != key {
        return Err(problem("The saved sign-in belongs to a different website."));
    }
    if metadata.get("sign_in_revision").and_then(Value::as_str) != Some(saved.revision.as_str()) {
        return Err(problem("The saved account changed, or this capture has no account identity."));
    }
    if metadata.get("sign_in_page").and_then(Value::as_bool) != Some(false) {
        return Err(problem("The capture still shows a sign-in page, or its sign-in state could not be checked."));
    }
    let shots = metadata
        .get("screenshots")
        .and_then(Value::as_array)
        .filter(|s| !s.is_empty() && s.len() <= 4)
        .ok_or_else(|| problem("The study has no usable first screenshots."))?;
    let mut screenshots = Vec::new();
    for shot in shots {
        let file = shot.get("file").and_then(Value::as_str).ok_or_else(|| problem("A first screenshot has no file name."))?;
        // Only the first page's screenshots, never an arbitrary path supplied in a record.
        if !file.starts_with("screenshot-") || !file.ends_with(".png") || file.contains(['/', '\\', ':']) {
            return Err(problem("A first screenshot has an invalid local file name."));
        }
        let bytes = read(&ws.root, Path::new(file))?;
        web_common::decode_png(&bytes).map_err(|_| problem("A first screenshot cannot be decoded. Please capture the website again."))?;
        screenshots.push(Screenshot { file: file.to_string(), sha256: digest(&bytes) });
    }
    Ok(Some(CaptureReview { website: shown_url(url), capture_sha256: capture_digest(&ws.root, url, &saved.revision)?, screenshots }))
}

impl CaptureReview {
    /// Called on a UI worker. A request cannot make the image loader read outside capture/ or
    /// display content that changed since the request. Nothing is fetched from the website.
    pub fn preview_bytes(&self, root: &Path, screenshot: &Screenshot) -> Result<Vec<u8>, ToolError> {
        if !self.screenshots.contains(screenshot)
            || !screenshot.file.starts_with("screenshot-")
            || !screenshot.file.ends_with(".png")
            || screenshot.file.contains(['/', '\\', ':'])
        {
            return Err(problem("The screenshot is not part of this confirmation."));
        }
        let bytes = read(root, Path::new(&screenshot.file))?;
        if digest(&bytes) != screenshot.sha256 {
            return Err(problem("The screenshot changed while confirmation was waiting."));
        }
        Ok(bytes)
    }
}
