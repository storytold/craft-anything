//! Keep a recorded reply separate from proof that the inbox delivered it. Writers hold the
//! request's file lock across these transitions and the associated feed/answer writes.

use craft_anything_evidence::{ErrorCode, ToolError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const FORMAT: &str = "craft-anything-inbox-delivery";
const VERSION: u32 = 1;
const MAX_RECEIPT_BYTES: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeliveryState {
    /// Legacy answers, including temporary self-checks, do not have a receipt.
    Untracked,
    /// The conversation may contain the reply, but delivery has not been confirmed.
    Staged,
    /// The waiter consumed the answer; this survives removal of the request and answer.
    Delivered,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SavedState {
    Staged,
    Delivered,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    format: String,
    version: u32,
    id: String,
    state: SavedState,
}

fn path(dir: &Path, id: &str) -> Result<PathBuf, ToolError> {
    if !super::valid_id(id) {
        return Err(ToolError::invalid("This question has an invalid inbox id.").with_remediation("Ask the builder to send a new request."));
    }
    Ok(dir.join(format!("{id}.delivery.json")))
}

fn unreadable(error: ToolError) -> ToolError {
    error.with_remediation("Keep your reply and the delivery record unchanged. Restore access to this project's inbox, then retry.")
}

fn require_regular(path: &Path, metadata: &std::fs::Metadata) -> Result<(), ToolError> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(ToolError::new(ErrorCode::FileError, format!("{} isn't a regular delivery record.", path.display()))
            .with_remediation("Keep this entry unchanged and restore the project's delivery record as a regular file before retrying."));
    }
    Ok(())
}

/// Read only a recognized receipt for this exact question. Absence is the legacy case; an
/// unreadable, oversized, linked or unknown record must never become permission to overwrite it.
pub(super) fn state(dir: &Path, id: &str) -> Result<DeliveryState, ToolError> {
    let path = path(dir, id)?;
    let before = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(DeliveryState::Untracked),
        Err(error) => return Err(unreadable(ToolError::io(path.display(), error))),
    };
    require_regular(&path, &before)?;
    let bytes = craft_anything_platform::fs::read_file_capped(&path, MAX_RECEIPT_BYTES).map_err(unreadable)?;
    let after = std::fs::symlink_metadata(&path).map_err(|error| unreadable(ToolError::io(path.display(), error)))?;
    require_regular(&path, &after)?;
    let receipt: Receipt = serde_json::from_slice(&bytes).map_err(|_| {
        ToolError::new(ErrorCode::GarbledOutput, "This question's delivery record could not be read.")
            .with_remediation("Keep this record unchanged. Open the project with a compatible Craft Anything version, or ask the builder for a new question.")
    })?;
    if receipt.format != FORMAT || receipt.version != VERSION {
        return Err(ToolError::new(ErrorCode::NotSupported, "This question's delivery record has an unsupported format or version.")
            .with_remediation("Keep this record unchanged and open the project with a compatible Craft Anything version."));
    }
    if receipt.id != id {
        return Err(ToolError::new(ErrorCode::GarbledOutput, "The saved delivery record doesn't belong to this question.")
            .with_remediation("Keep this record unchanged and ask the builder to send a new request."));
    }
    Ok(match receipt.state {
        SavedState::Staged => DeliveryState::Staged,
        SavedState::Delivered => DeliveryState::Delivered,
    })
}

fn write(dir: &Path, id: &str, state: SavedState) -> Result<(), ToolError> {
    let path = path(dir, id)?;
    let receipt = Receipt { format: FORMAT.into(), version: VERSION, id: id.into(), state };
    let bytes = serde_json::to_vec(&receipt).map_err(|error| ToolError::internal(error.to_string()))?;
    craft_anything_platform::fs::write_atomic_private(&path, &bytes).map_err(unreadable)
}

/// Durably record the attempt before saving its reply. Repeated attempts remain staged and
/// retry the directory sync; an already delivered answer cannot authorize another write.
pub(super) fn stage(dir: &Path, id: &str) -> Result<(), ToolError> {
    stage_using(dir, id, crate::feed::sync_directory)
}

fn stage_using(dir: &Path, id: &str, sync: impl FnOnce(&Path) -> Result<(), ToolError>) -> Result<(), ToolError> {
    match state(dir, id)? {
        DeliveryState::Untracked => write(dir, id, SavedState::Staged)?,
        DeliveryState::Staged => {}
        DeliveryState::Delivered => {
            return Err(ToolError::new(ErrorCode::NotFound, "This question's answer has already been delivered.")
                .with_remediation("Answer a new request from the builder instead."));
        }
    }
    // Atomic replacement synced the receipt's contents, not its directory entry. Persisting
    // the conversation first could otherwise leave a saved reply looking like a legacy answer
    // after a crash. An existing staged receipt can be a previous attempt whose sync failed.
    sync(dir).map_err(unreadable)
}

/// Confirm consumption before removing the answer. The caller retains the answer file if this
/// write fails, so it still has durable evidence of delivery. Legacy answers create no receipt.
pub(super) fn mark_delivered(dir: &Path, id: &str) -> Result<(), ToolError> {
    match state(dir, id)? {
        DeliveryState::Staged => write(dir, id, SavedState::Delivered),
        DeliveryState::Untracked | DeliveryState::Delivered => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "question-1";

    #[test]
    fn legacy_answers_do_not_create_delivery_records() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(state(dir.path(), ID).unwrap(), DeliveryState::Untracked);
        mark_delivered(dir.path(), ID).unwrap();
        assert!(!path(dir.path(), ID).unwrap().exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn staged_and_delivered_states_survive_reopening() {
        let dir = tempfile::tempdir().unwrap();
        stage(dir.path(), ID).unwrap();
        let reopened = dir.path().to_path_buf();
        assert_eq!(state(&reopened, ID).unwrap(), DeliveryState::Staged);
        let record_path = path(&reopened, ID).unwrap();
        let staged = std::fs::read(&record_path).unwrap();
        let record: serde_json::Value = serde_json::from_slice(&staged).unwrap();
        assert_eq!(record["format"], FORMAT);
        assert_eq!(record["version"], VERSION);
        assert_eq!(record["id"], ID);
        assert_eq!(record["state"], "staged");
        stage(&reopened, ID).unwrap();
        assert_eq!(std::fs::read(&record_path).unwrap(), staged, "retrying does not replace a staged receipt");

        mark_delivered(&reopened, ID).unwrap();
        assert_eq!(state(dir.path(), ID).unwrap(), DeliveryState::Delivered);
        let delivered = std::fs::read(&record_path).unwrap();
        mark_delivered(dir.path(), ID).unwrap();
        assert_eq!(std::fs::read(&record_path).unwrap(), delivered);
    }

    #[test]
    fn new_and_existing_staged_receipts_require_a_directory_sync_before_persistence() {
        for already_staged in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            if already_staged {
                write(dir.path(), ID, SavedState::Staged).unwrap();
            }
            let record_path = path(dir.path(), ID).unwrap();
            let mut syncs = 0;
            let mut persisted = false;
            let error = stage_using(dir.path(), ID, |synced| {
                syncs += 1;
                assert_eq!(synced, dir.path());
                assert_eq!(state(dir.path(), ID).unwrap(), DeliveryState::Staged);
                Err(ToolError::new(ErrorCode::FileError, "injected staged receipt directory sync failure"))
            })
            .map(|()| persisted = true)
            .expect_err("staged receipt directory sync must run before persistence");
            assert_eq!(syncs, 1);
            assert!(!persisted, "a failed receipt sync must stop the following conversation write");
            assert!(error.message.contains("injected staged receipt directory sync failure"));
            assert_eq!(state(dir.path(), ID).unwrap(), DeliveryState::Staged);
            let staged = std::fs::read(&record_path).unwrap();

            stage_using(dir.path(), ID, |synced| {
                syncs += 1;
                assert_eq!(synced, dir.path());
                assert_eq!(std::fs::read(&record_path).unwrap(), staged);
                crate::feed::sync_directory(synced)
            })
            .map(|()| persisted = true)
            .unwrap();
            assert_eq!(syncs, 2, "retry must sync even when the receipt already says staged");
            assert!(persisted);
            assert_eq!(std::fs::read(&record_path).unwrap(), staged);
        }
    }

    #[test]
    fn a_delivered_receipt_cannot_be_downgraded() {
        let dir = tempfile::tempdir().unwrap();
        stage(dir.path(), ID).unwrap();
        mark_delivered(dir.path(), ID).unwrap();
        let record_path = path(dir.path(), ID).unwrap();
        let before = std::fs::read(&record_path).unwrap();

        assert_eq!(stage_using(dir.path(), ID, |_| panic!("a delivered receipt cannot authorize persistence")).unwrap_err().code, ErrorCode::NotFound);

        assert_eq!(state(dir.path(), ID).unwrap(), DeliveryState::Delivered);
        assert_eq!(std::fs::read(&record_path).unwrap(), before);
    }

    #[test]
    fn unknown_newer_and_corrupt_receipts_are_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        stage(dir.path(), ID).unwrap();
        let record_path = path(dir.path(), ID).unwrap();
        let original: serde_json::Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        let cases = [
            ("version", serde_json::json!(VERSION + 1)),
            ("version", serde_json::json!(0)),
            ("format", serde_json::json!("personal-data")),
            ("state", serde_json::json!("unknown")),
            ("id", serde_json::json!("another-question")),
            ("future", serde_json::json!(true)),
        ];
        for (field, value) in cases {
            let mut changed = original.clone();
            changed[field] = value;
            let bytes = serde_json::to_vec(&changed).unwrap();
            std::fs::write(&record_path, &bytes).unwrap();
            assert!(state(dir.path(), ID).is_err(), "{field}");
            assert!(stage(dir.path(), ID).is_err(), "{field}");
            assert!(mark_delivered(dir.path(), ID).is_err(), "{field}");
            assert_eq!(std::fs::read(&record_path).unwrap(), bytes, "{field}");
        }
        for bytes in [b"{not json".as_slice(), b"\xff\x00".as_slice()] {
            std::fs::write(&record_path, bytes).unwrap();
            assert_eq!(state(dir.path(), ID).unwrap_err().code, ErrorCode::GarbledOutput);
            assert!(stage(dir.path(), ID).is_err());
            assert!(mark_delivered(dir.path(), ID).is_err());
            assert_eq!(std::fs::read(&record_path).unwrap(), bytes);
        }
    }

    #[test]
    fn oversized_receipts_are_not_read_or_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let record_path = path(dir.path(), ID).unwrap();
        let bytes = vec![b' '; MAX_RECEIPT_BYTES as usize + 1];
        std::fs::write(&record_path, &bytes).unwrap();
        assert_eq!(state(dir.path(), ID).unwrap_err().code, ErrorCode::OverLimit);
        assert_eq!(stage(dir.path(), ID).unwrap_err().code, ErrorCode::OverLimit);
        assert_eq!(mark_delivered(dir.path(), ID).unwrap_err().code, ErrorCode::OverLimit);
        assert_eq!(std::fs::read(&record_path).unwrap(), bytes);
    }

    #[test]
    fn non_regular_receipts_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let record_path = path(dir.path(), ID).unwrap();
        std::fs::create_dir(&record_path).unwrap();
        std::fs::write(record_path.join("notes.txt"), "Keep this directory").unwrap();
        assert!(state(dir.path(), ID).is_err());
        assert!(stage(dir.path(), ID).is_err());
        assert!(mark_delivered(dir.path(), ID).is_err());
        assert_eq!(std::fs::read(record_path.join("notes.txt")).unwrap(), b"Keep this directory");
    }

    #[test]
    fn invalid_ids_are_rejected_before_accessing_files() {
        let dir = tempfile::tempdir().unwrap();
        for id in ["", "../outside", "/outside", "a/b", "a\\b", "UPPER", "question.json"] {
            assert_eq!(state(dir.path(), id).unwrap_err().code, ErrorCode::BadInput, "{id}");
            assert_eq!(stage(dir.path(), id).unwrap_err().code, ErrorCode::BadInput, "{id}");
            assert_eq!(mark_delivered(dir.path(), id).unwrap_err().code, ErrorCode::BadInput, "{id}");
        }
        assert_eq!(stage(dir.path(), &"a".repeat(65)).unwrap_err().code, ErrorCode::BadInput);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        let longest = "a".repeat(64);
        stage(dir.path(), &longest).unwrap();
        assert_eq!(state(dir.path(), &longest).unwrap(), DeliveryState::Staged);
    }

    #[cfg(unix)]
    #[test]
    fn linked_receipts_are_never_followed_or_replaced() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        stage(dir.path(), ID).unwrap();
        let record_path = path(dir.path(), ID).unwrap();
        let saved = dir.path().join("saved.json");
        std::fs::rename(&record_path, &saved).unwrap();
        let bytes = std::fs::read(&saved).unwrap();
        symlink(&saved, &record_path).unwrap();
        assert!(state(dir.path(), ID).is_err());
        assert!(stage(dir.path(), ID).is_err());
        assert!(mark_delivered(dir.path(), ID).is_err());
        assert_eq!(std::fs::read(&saved).unwrap(), bytes);
        assert!(std::fs::symlink_metadata(&record_path).unwrap().file_type().is_symlink());

        std::fs::remove_file(&saved).unwrap();
        assert!(state(dir.path(), ID).is_err(), "a dangling link is not an absent legacy record");
        assert!(stage(dir.path(), ID).is_err());
        assert!(mark_delivered(dir.path(), ID).is_err());
        assert!(!saved.exists());
        assert!(std::fs::symlink_metadata(&record_path).unwrap().file_type().is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn receipts_are_owner_only_when_staged_and_delivered() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        stage(dir.path(), ID).unwrap();
        let record_path = path(dir.path(), ID).unwrap();
        assert_eq!(std::fs::metadata(&record_path).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::set_permissions(&record_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        mark_delivered(dir.path(), ID).unwrap();
        assert_eq!(std::fs::metadata(&record_path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_receipt_is_not_treated_as_legacy() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        stage(dir.path(), ID).unwrap();
        let record_path = path(dir.path(), ID).unwrap();
        let before = std::fs::read(&record_path).unwrap();
        std::fs::set_permissions(&record_path, std::fs::Permissions::from_mode(0o200)).unwrap();
        if std::fs::read(&record_path).is_ok() {
            std::fs::set_permissions(&record_path, std::fs::Permissions::from_mode(0o600)).unwrap();
            return; // Privileged test processes can bypass ordinary Unix permissions.
        }
        let read = state(dir.path(), ID);
        let staged = stage(dir.path(), ID);
        let delivered = mark_delivered(dir.path(), ID);
        std::fs::set_permissions(&record_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read.unwrap_err().code, ErrorCode::NoPermission);
        assert_eq!(staged.unwrap_err().code, ErrorCode::NoPermission);
        assert_eq!(delivered.unwrap_err().code, ErrorCode::NoPermission);
        assert_eq!(std::fs::read(&record_path).unwrap(), before);
    }
}
