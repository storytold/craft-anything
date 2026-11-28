# File formats

The table below lists the files that carry a Craft Anything format version and how older and newer
versions are handled. When a file has migration steps, Craft Anything parses it into raw JSON or
TOML, runs the steps up to the current version, then decodes it into Rust types. The step runner
is `crates/engine/src/versioned.rs`.

| File | Owner | Current version | Older files | Newer files |
|---|---|---|---|---|
| `<project>/craft.json` | `engine::workspace` (`Manifest`) | `MANIFEST_VERSION` = 7 (`version`; missing = 0; 3 adds `approvals`, 4 adds the saved builder session's mode `agent_session_deep`, 5 adds the current capture's mode `capture_deep`, 6 adds the toolchain a move to another language was confirmed with, `toolchain`, 7 adds project `generation`; a version-3 session and a version-4 capture migrate as Deep; older generations stay absent) | brought forward, saved as current | opened read-only: saving is refused, also by a build that read the file before a newer one brought it forward (builds before version 6 don't check this, so close them before running a newer one on the same project) |
| `<project>/inbox/build-progress.json` | `engine::workspace::progress` (`BuildProgress`; written only by the rebuild run) | `BUILD_PROGRESS_VERSION` = 2 (`version`; missing = 0; 2 adds `checked`, the round kept while nothing it looked at changes) | brought forward; an absent file starts with no counted turns | the build stops and the record is left unchanged |
| `<project>/inbox/queued-request.json` | `ai::rebuild::queued_request` (`QueuedRequest`; written before a run joins the queue) | 3 (`version`, required; `message` and original request time `asked`, optional `shown`, `shown_pictures` and `carried_notes` defaulting to empty lists; carried notes record their id and original text, shown text, pictures and known time; schema derived from the record type) | version 1 has no carried-note identities, so separately waiting notes are preserved; versions 1 and 2 have no request time and migrate with the Unix epoch; an absent file means no queued request | the start stops and the record is left unchanged |
| `config.toml` (`settings::config_dir()`) | `engine::settings` (`Settings`) | `SETTINGS_VERSION` = 2 (`version`; missing = 0; 2 moved `[engines.ghidra] enabled` to `deep_mode`) | brought forward | loaded, never saved over |
| `<name>.cany/project.toml` | `engine::project` (`ProjectMeta`) | `FORMAT_VERSION` = 1 (`format_version`, required) | brought forward | refused, with "update Craft Anything" |
| `<project>/transcript.json` | `ai::context` (the API builder's conversation) | `TRANSCRIPT_VERSION` = 2 (`{version, deep, history}`; a version-1 transcript counts as Deep) | read as before; one without `deep` is resumed only in Deep | skipped when resuming (see [Newer files](#newer-files)) |
| `<project>/app/mechanics.json` | `engine::mechanics` (written by the builder, read by `MechanicsCheck`) | `MECHANICS_VERSION` = 1 (`version`; missing = 1) | — | the check fails and says so |
| `<project>/checks/round-N/self-test.json` | `engine::self_test` (written by the built game's `--self-test`, read by `SelfTestCheck`) | 1 (`version`; missing = 1) | — | the check fails and says so |
| `<project>/usage.json` | `engine::workspace` (`ProjectUsage`; written by builds and questions) | `USAGE_VERSION` = 1 (`version`; missing = 0) | read as before, saved as current; absent `tokens_incomplete` means no missing token counts were recorded | read as far as its fields are known, never written over |
| `<project>/questions.json` | `engine::questions` (`AskedQuestion`, written by `rebuild_question_ask`) | `QUESTIONS_VERSION` = 1 (`{version, items}`) | read as before | read as far as its fields are known |
| `<project>/pinned/<n>/pin.json` | `engine::pins` (`Pin`) | `PIN_VERSION` = 2 (`version`; missing = 1, any other value that is not a whole number is refused; 2 records the selected program for a copied direct launch) | brought forward | read as far as its fields are known, never written over (renaming is refused) |
| `<folder>/standin.json` | `engine::tools::standin_tools` (`analysis::standin::StandIn`; written only by `web_standin_build`) | `STANDIN_VERSION` = 2 (`version`, required; 2 adds each sample's place in the recording, `order`, and `write` for an answer to a write; a version-1 file has neither: its answers replay in turn, and a write gets each one once) | brought forward | refused, with "update Craft Anything", and never written over |
| `<folder>/craft-pseudocode-export.json` | `engine::tools::program_facts_tools` (`ExportState`; written by `native_source_folder_write`) | `EXPORT_STATE_VERSION` = 2 (`version`, required; 2 records the source of each function) | version 1 records migrate as built-in code, preserving completed files | refused and never written over |
| `<project>/feed.json` | `ai::rebuild::feed` (written by the run) | `FEED_VERSION` = 6 (`{version, items, recovery?}`; a bare list = 0; 4 adds `PlanDismissed`; 5 marks open steps stopped at a run's end; 6 retains the manifest projection used during recovery) | read as before, saved as current | never written over; the conversation is rebuilt from the project for display |
| `<project>/inbox/<id>.delivery.json` | `engine::inbox::delivery` | 1 (`format`, `version`, `id`, `state`; staged or delivered) | absent records leave existing feed answers compatible | unrecognized records are preserved and block a new recorded submission |
| `<project>/inbox/feed-backups/<backup-name>.json` | `engine::feed::backups` | 2 (`format`, `version`, `backup`, `length`, `sha256`; requires retained private file identity) | version-1 final receipts retain historical length/hash proof; absent or unrecognized records establish no ownership | unrecognized records are preserved during recovery and never authorize backup deletion; deleting a project clears its inbox records but keeps unverified backup-shaped entries |
| `<project>/inbox/feed-backups/pending/<backup-name>` and `<backup-name>.json` | `engine::feed::backups` | raw source bytes and a version-2 ownership receipt; the raw identity link remains after publication | version-1 interrupted records retain their same-file check, including a truncated old final receipt | interrupted and completed version-2 publication require the receipt and the same opened file as the private copy; equal bytes in a separate file remain personal |
| `<project>/inbox/feed-recovery/conversation-<millis>-<pid>-<counter>.recovery.json` | `engine::feed::recovery`, requested by the run worker | 1 (`version`, folded `items`, `metadata` with format, source identity and recovery details) | no earlier format | unknown or invalid records are retained without automatic import or removal |

A saved `User` message with pictures is `{text, pictures: [paths]}`, where the pictures are the
copies in the project's `attachments/`; one without pictures stays the plain string earlier
versions wrote.

Other JSON in a project (`assets.json`, `inbox/` except the versioned records listed above, `checks/`,
evidence records) has no version yet; add one the first time its shape changes.

The queued request keeps the person's words, attachment paths and original request time, not a
builder choice, mode, run budget or queue position. The next manual start restores it when no
new message was supplied. Version 3 requires the original `asked` time, including its subsecond
precision. Versions 1 and 2 lacked it and conservatively use the Unix epoch, so restarting cannot
make an old request clear a later timestamped Ghidra stop. A new explicit message keeps its own
request time. The file is capped at 16 MiB; malformed, oversized, unreadable and newer records are
never treated as an absent request or overwritten. Reading does not consume it. Starting a
worker removes the matching saved record; a failed start or a pause leaves it for retry. This
backup does not provide exactly-once delivery across a crash after the worker has started.

## Stand-in cached responses

Version 1 stand-in files retain a recorded 304 when no usable cached body was saved. Those samples
remain in the file for diagnostics, but replay skips them within the best matching path, query and
operation group. If that group has no usable answer, it reports `cached_body_unavailable` instead
of sending a bare 304 to a fresh client. A usable cached body is still stored and replayed as a 200
with `was_not_modified: true`. This interpretation also applies to existing v1 files; the persisted
shape and version are unchanged.

## Resumable pseudo-C exports

Version 2 records each function's `source`: its decompiler (`engine`), the returned function's
`entry` in the built-in address space, and `engine_entry` when Ghidra loaded the program elsewhere.
The engine version and selected language are kept when known. The map key and file name use the
requested entry, so a returned containing function cannot change the resume key. `structured` is
null when the decompiler does not report that property.

Version 1 used only the built-in lifter. Its migration records that source for each function and
keeps its `structured` flag. Existing code files are left alone: later runs can finish the same
folder with Ghidra, with each old and new file still attributed to the engine that wrote it.
The state records what was written; completion and source counts check which files still exist.
A missing file counts as unfinished even when a call's writing budget is zero.

The state must be a regular file of at most 256 MiB. Broken, unrecognised and newer states are
refused before files are written. The generated schema is
[`pseudocode-export.schema.json`](schemas/pseudocode-export.schema.json).

## Newer files

Craft Anything does not migrate newer files backward when reading them. What happens when saving
depends on the format. `craft.json` and `config.toml` are still read as far as their fields are known,
so the person can see their project or keep their settings; every save returns a `ToolError` that
says to update Craft Anything. A newer `craft.json` whose fields can't be read at all is refused the
same way, and a newer `config.toml` that can't be read gives defaults without touching the file.
A newer `.cany` is refused outright: its evidence layout may have changed in ways we can't
see.

Before saving `feed.json`, a filesystem read failure also stops the save and keeps the existing path.
Only a path confirmed absent without following symlinks can be created without inspecting an old
version. A fallback remembers that the original wasn't loaded, even if the file becomes readable
later. A live writer may load an actually decoded replacement and replay its unsaved events; a missing, unreadable, damaged or newer replacement cannot clear the original guard. Every application feed save compares its loaded source while holding a project-scoped operating-system lock through replacement. An originally absent path must still be absent. A mismatch retains the old writer and unsaved events for a fresh load; ordinary answer transactions load and apply their reply under the same lock. Version 6 records the damaged source's SHA-256 and the exact manifest request, checks, reply and failure note that produced a recovery prefix. Only a writer that observed that same damaged source can consume its projection; a temporary read failure cannot authorize removing old history. Later manifest updates cannot change the recorded origin. Older feeds have no recovery provenance; matching text alone never establishes it. Readable bytes that cannot be parsed are copied
exactly to an unused `feed.json.broken` (then `.broken.1` through `.broken.15`) before replacement.
Existing backups are never replaced. Recovery completes and syncs raw bytes and a version-2 receipt inside private `inbox/feed-backups/pending/` before publishing a root-level hard link. It publishes the already-synced receipt by an exclusive hard link, then retains the private raw link as file-identity proof. The directory name `pending` therefore also holds completed backups' identity anchors. Version-2 ownership requires that identity as well as matching length and digest, even after final publication. Version-1 final receipts keep their historical length/hash behavior; past identity evidence cannot be reconstructed. A complete version-1 pending receipt can still prove an interrupted old publication whose final JSON was truncated, while an unknown complete final version never falls back to pending evidence. Before public publication, a failed attempt cleans only its own unchanged staging entries and retains the original error if cleanup also fails. Replacements and already-published proof are preserved. If feed replacement fails after a complete backup, a retry reuses that copy only when its ownership proof and current bytes both match the damaged source. When freeing a slot, move the backup and all associated records, including its retained raw link and private receipt, together. Project deletion first moves each selected backup into a fresh `.craft-backup-quarantine-*` folder beside the project records, then verifies the claimed regular file against its ownership proof before unlinking it. A changed entry is restored without replacing another path, or retained in that folder with an error explaining where it is. That folder is outside the records deleted with the project and is never cleaned recursively. Legacy personal files or directories with backup-shaped names survive deletion, are included when duplicating an in-place project and contribute to its app fingerprint; verified generated copies are excluded. Ownership scans skip absent candidates before reading unrelated receipts, keep I/O errors for present uncertain candidates, and check cancellation between 64 KiB hash chunks. A failed backup or receipt stops the save and leaves the source intact;
the recovered conversation names the backup. A newer format is left in place without recovery.

The empty `inbox/feed.lock` coordination file remains in place after each transaction. Lock waits
are bounded to five seconds; request locks precede this feed lock, never the other way around.
Deletion holds the same lock and first removes inbox payloads while `craft.json` remains discoverable.
Every entry identifying the held lock inode is retained, including alternate filesystem spellings.
A payload-cleanup failure can therefore be retried after access is repaired. Deletion also reserves
its private cleanup directory and retains its identity before removing `craft.json`; a permission or
space failure at that allocation leaves the manifest and public inbox available for retry. If manifest
removal fails, only the verified empty cleanup reservation is released. After the manifest is removed,
the still-locked inbox is claimed into that reserved directory before its empty lock entries are removed.
Failure to claim reports the public inbox path; no replacement cleanup directory is allocated.
Once that public name is released, deletion never cleans it again, so a new project can reserve it
without losing its inbox to the old deletion. The guard remains held through final cleanup; unexpected
contents are retained at the reported private path. Later filesystem failures can still interrupt
that cleanup. Writers open the inbox relative to the retained project directory. Both writers and
initial creators open or create `feed.lock` relative to their captured inbox handle, so reuse of the
public pathname cannot redirect a late lock creation into another creator's empty reservation.
Existing-file opens use the same retained directory; Unix opens reject links and cannot block on a
substituted FIFO. Writers check that the project exists before creating coordination
entries and recheck the root directory identity, manifest creation time and generation after acquiring
the lock. New projects and copies receive an operating-system-random 128-bit generation, saved as
32 hexadecimal characters in `craft.json`. Ordinary manifest updates acquire the project lock,
validate the caller's loaded identity, merge the current person-owned fields and replace the manifest
without releasing that guard. They cannot adopt another project's generation or recreate a deleted
manifest. Only initial creation and successful duplication publish a new manifest without an existing
project guard.
Older manifests retain a recorded generation, or keep it absent if none was recorded; opening one
does not write it or invent a per-open identity. A later project created in the same directory receives a new
generation even if its creation timestamp repeats. Loaded writers retain that identity; replay and separate
recovery archives reject a different project later created at the same path. Saved-answer merging
checks that identity before adopting any replacement rows. Deletion also compares
its loaded workspace with the project under the lock before removing anything. They do not write
conversation, backup, archive or recorded-answer payloads after deletion. Earlier builds could publish
a project with a generation but no inbox, so a generation alone cannot prove a persistent reservation.
An existing project may initialize a missing inbox through the same guarded protocol. Initial creators hold the
same exclusive coordination inode from reservation through publication or failed-creation cleanup.
A failed acquisition retains the original newly created lock handle. Rollback can use it only after
confirming the original root/inbox, an absent manifest and exclusive ownership of that same empty
inode; an already held lock is not acquired again. Contention, continued I/O errors or changed
identity preserve the reservation and its inspection guidance. Opening an existing unlocked file
does not grant cleanup ownership.
After its first capture of a newly made inbox, an initializer must confirm the original project before
opening a coordination file or gaining cleanup authority. If that confirmation fails, it retains the
current inbox and reports its inspection path without creating or acquiring a lock: deletion and a new
creator may have reused the name between mkdir and capture. Even an empty retained inbox can belong
to that creator. This uncertain case can require manual inspection before retry; automatic cleanup is
not promised. After a successful first confirmation, the initializer still rechecks the project under
the exclusive lock. If deletion then wins and no manifest exists, it can privately claim its confirmed
inbox only after checking the held root/inbox identities and exclusively held lock again. A lock created by another
initializer can be released under this same protocol; an initializer that did not create the inbox
never removes it. A busy creator or deleter keeps the public reservation until it finishes.
Cleanup removes only a verified empty coordination entry and empty directories. Existing
reservations, a published replacement and changed entries are preserved. Unrecognized contents
remain at the reported private path for inspection; final cleanup is not a filesystem-atomic lifecycle
protocol. Ordinary inbox request posting uses its separate protocol.
Duplication checks the loaded source identity under the same lock in both app layouts and holds
it through destination manifest publication. In-place backup discovery also stays under that lock,
so application recovery cannot change ownership during the copy. After actual write-access denial,
an existing regular, empty coordination file can instead be opened read-only and held with a shared
lock that excludes feed writers. It checks both the original inbox directory identity and held lock
identity again before publication. This guard cannot be supplied to a feed-writing API. After actual
write-access denial, a legacy source with an existing inbox directory but no coordination file can
use an uncoordinated copy. It checks the original inbox directory identity and the absent lock both
before copying and before destination publication. An existing invalid or unusable lock file, a
lock timeout, unsupported locking or contention does not permit this fallback.
A readable legacy source with no
inbox can be copied without creating one when the attempted lock creation is denied, including a
read-only filesystem; owner write bits do not establish the current caller's access. Other I/O errors
still fail the copy. This fallback checks the same directory and project
identity, and an absent inbox, both before copying and before publishing the destination manifest.
Application publishers retain their inbox, so an inbox appearing during the copy requires a retry
even after that publisher releases its lock. Manifest and backup classification checks still apply. Each backup-shaped file
is classified through the opened handle that will be copied. Equivalent case spellings count as
the reserved slot only when they identify the same file; a distinct personal file is retained.
A new or changed unverified entry requires a retry. Losing a receipt also cannot turn a previously
verified recovery copy into personal app content during the same copy. Every initial creator,
including creation in a person-selected folder, first reserves a private cleanup directory with a
held identity, then atomically reserves the existing `inbox/` name before publishing a manifest. If
that initial cleanup-space allocation fails, no inbox is created. A selected directory link is resolved before validation; the returned workspace
and the desktop's remembered project use that real path. The reservation prevents another creator
from adopting a copy still in progress. After successful publication it remains until project deletion,
so releasing it cannot race later feed writers. A creation that loses the
reservation leaves the winner's directory alone. A fresh destination captures its root identity immediately
after creation, refuses a nonempty root and checks that same identity before claiming its inbox.
Directory creation and the first open are separate operations, so captured identity never authorizes
deleting directory contents. A failed initial publication in a selected folder reuses the already
reserved cleanup directory, without allocating another one after a storage-full error. It privately
claims this attempt's empty inbox, rechecks both directory identities and manifest absence, and removes only the
held empty coordination file and any same-file aliases, then the empty claimed directory. A visible
manifest, changed identity or unrecognized inbox content is preserved with inspection
guidance; a missing, changed or occupied cleanup reservation never falls back to a new allocation.
The selected folder is never removed by that cleanup. Successful manifest publication removes only
the verified empty cleanup reservation. If that cleanup fails while the root, inbox, coordination
file and published manifest identity remain unchanged, creation and duplication still return the
published workspace with transient cleanup guidance and log the retained path. Creation and copy
callers also show the guidance through the UI, CLI or tool result; reopening a workspace clears the
transient warning without changing the manifest. Identity changes and initial write errors remain failures. Preallocation does not guarantee that rename, unlink or directory removal can succeed on
an exhausted filesystem. Legacy-initializer cleanup and the outer private root claim for failed copies
can still need allocation. The destination manifest is published after copying
succeeds, while the root, inbox and coordination file still match their held identities and no manifest
has appeared. Copying exclusively creates each destination directory and file; an existing or
newly arrived destination entry is refused instead of overwritten. Ordinary files keep their source
permissions through the opened output handle.
A failed copy checks the same reservation, claims its destination into a fresh private
`.craft-copy-cleanup-*` folder beside it, then checks that reservation again. Cleanup removes only an
empty reservation and empty root; unfinished copied files and unexpected contents are preserved at
the reported claimed path for inspection. Failure reporting keeps the original recovery advice and
the complete nested cleanup guidance, including the retained path. A failed copy is never recursively removed. This coordination does not make a directory copy an atomic filesystem snapshot
against external programs that do not take the lock. App fingerprinting also uses the same opened
regular file for ownership classification and content hashing, so recovery cannot substitute a newly
generated backup between those two reads.

If a run's final feed save fails, its worker attempts a separate recovery copy before sending the
`Finished` event. Version 1 keeps the complete folded conversation, the loaded source's state and
SHA-256 where available, and details containing the pending-event count, original manifest history
and save error. Raw tool-event parameters and results are not serialized. Each copy is exclusively
created in private `inbox/feed-recovery/pending/`, uses mode `0600` on Unix, and is completely serialized
and synced before an exclusive hard link publishes its final name. Partial staging files remain
inspectable but do not enter completed-archive discovery or its file/byte budgets. Unix also syncs its
directory and each parent entry, including on a retry after a failed parent sync. A record is limited to 64 MiB and name reservation to 64 attempts. An error keeping the
copy is reported without claiming the complete unsaved conversation reached disk. Retained partial
paths are reported for inspection before moving them aside, and changed entries are marked unverified.
Repeating the same over-limit snapshot can leave another partial, so retry alone is not a cleanup
strategy. A durable final archive remains a usable recovery copy if private staging cleanup fails;
the report keeps its path and a separate cleanup warning. Polling and dropping a run do not retry
persistence or archive writes.

Display loading discovers at most 256 recovery-directory entries, reads at most 64 MiB in total
and adds at most 16 historical notices for recognized records. A live run caches these notices
when it opens. Writer loads, saved conversations and later recovery snapshots exclude them;
legacy saved notices are removed only when their full generated Error shape and archive path match.
Each notice names its exact path and is deduplicated against that path. These notices do not claim the current feed is unsaved and
do not explain a new run's failure. Unknown versions, invalid or oversized files and links are
left alone. Recovery copies are never automatically replayed, overwritten or cleaned up; inspect
their conversation before deciding what to restore, since it may overlap the current feed.

Within a live run, replay also retains the position and saved context of its active streamed message.
An intervening saved answer and the display-only clearing of streaming flags do not split its next
text delta into another message. A replaced or separately appended external message does not become
that stream merely because its text is equal. This context stays in memory and is committed with the
replayed feed when replacement becomes visible; it adds no persisted feed fields. A newly introduced
question remains at its original event boundary during replay, even if loading already appended
its pending row or another client answered it. Repeated Ask events do not move the existing row
or end an active stream when that row exists. If repaired history no longer contains the question,
replay restores it at the event boundary even if its inbox request has expired; the request is not
reopened. The pending journal retains only checklist steps, tool names and result flags from tool
events, not their unused raw input or summaries. The desktop installs one writer display snapshot
per polled batch, so live text never folds through recovery or persistence notices. It still handles
every event's workflow effects. A queued-start failure has no run snapshot and always displays its
own reason, even after an earlier error or notice. A successful answer retry refreshes only that
card without consuming queued workflow events.

Answers are saved in the conversation before they are delivered through the inbox. On Unix, the
feed's containing directory is synced after atomic replacement and before delivery; non-Unix
platforms retain the existing directory-sync no-op. If replacement is visible but directory sync
fails, the loaded source, replayed state and cleared journal adopt those visible bytes, while the
operation still reports the durability error and withholds answer delivery. A later pre-write failure
cannot inherit the earlier visible-write flag. A final run with unconfirmed durability still attempts
a separate recovery archive, including when its event journal is already empty. A save failure
leaves the question pending. A delivery failure also leaves it available for
retry; display views show the pending question and saved reply while writers retain the recorded answer. Submission reserves the request first, acquires and validates the loaded project's guard, rechecks the request file's identity, then stages a versioned receipt, persists the reply and publishes the answer while retaining that guard. Deletion cannot complete between these steps, and a submission prepared after deletion cannot recreate an old delivery record. The waiter marks the receipt delivered before removing an answer. If receipt persistence fails at consumption, the answer file remains as durable delivery evidence. An existing answer's I/O error stops another submission before its persistence callback. A confirmed staged reply whose request expires is shown as undelivered, with its text available for a new request. If delivery evidence cannot be read, the display keeps the words and says delivery is unknown, without suggesting another submission. Display snapshots retain the opened feed identity, length and modification time and recheck them after reading receipt state. A changed snapshot shows its earlier reply as unconfirmed rather than attributing a later retry's delivery to it; refreshing loads the latest reply. This check covers application atomic replacement, not arbitrary external in-place writes that preserve metadata. Ordinary temporary inbox answers without feed persistence do not create receipts.

A confirmed pending reply keeps its retry card through unrelated feed writes in the same project;
it never gains delivery confirmation from the stale snapshot. A different project at the same path
cannot reuse that card. The display recheck opens only regular files without blocking on a FIFO.
After consumption, Unix directory sync confirms the delivered receipt before either request or
answer is unlinked. A failed receipt write or sync retains both files while returning the actual
accepted answer; discovery does not offer that answer for resubmission.

A complete version-2 raw/receipt pair left in private backup staging can resume publication when
both records match the damaged source and their opened identities remain unchanged. This can use
the last available backup slot without creating another private copy. Unverifiable pairs remain in
place with both exact paths in the error, and version-1 ownership compatibility is unchanged.

Timeout and cancellation withdraw under the same request-file lock as submission. If unlinking
the request fails, the waiter truncates and syncs its locked file as an empty withdrawal marker.
Pending discovery ignores that marker and submission rejects it even after directory access is
restored. A checked wait reports an I/O error if withdrawal cannot be confirmed; it does not report
that failure as a person's refusal or a completed timeout. The compatibility `wait` API logs such
an error and returns no answer; production callers use the checked path.

The API builder skips newer transcripts when resuming and leaves the file in place. If a fresh
build later saves a transcript to the same path, it replaces the existing file.

A file that can't be parsed is not "newer": `config.toml` is set aside as `config.toml.broken` and
defaults are used; an unreadable `craft.json` gives a plain-English error when opened and is left
out of the project list.

## Adding a migration step

1. Bump the version constant (`MANIFEST_VERSION`, `SETTINGS_VERSION` or `FORMAT_VERSION`).
2. Append one function to the matching steps list (`MANIFEST_STEPS`, `SETTINGS_STEPS`,
   `FORMAT_STEPS`). It takes the raw document from version n to n + 1: rename, move or fill keys.
   Return `Err` with a plain-English reason when it can't. A compile-time assertion fails if the
   list and the constant disagree.
3. Add a test that reads a literal sample of the old file (keep the existing samples; old files
   must keep loading forever).
4. For `craft.json`, `inbox/build-progress.json`, `mechanics.json`, `self-test.json` and `craft-pseudocode-export.json`, run `cargo xtask schemas` and commit `docs/schemas/`.
   `cargo xtask schemas --check` (and an engine test) fail when the published schema is stale.
