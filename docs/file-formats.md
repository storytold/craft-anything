# File formats

The table below lists the files that carry a Craft Anything format version and how older and newer
versions are handled. When a file has migration steps, Craft Anything parses it into raw JSON or
TOML, runs the steps up to the current version, then decodes it into Rust types. The step runner
is `crates/engine/src/versioned.rs`.

| File | Owner | Current version | Older files | Newer files |
|---|---|---|---|---|
| `<project>/craft.json` | `engine::workspace` (`Manifest`) | `MANIFEST_VERSION` = 8 (`version`; missing = 0; 3 adds `approvals`, 4 adds the saved builder session's mode `agent_session_deep`, 5 adds the current capture's mode `capture_deep`, 6 adds the toolchain a move to another language was confirmed with, `toolchain`, 7 adds the saved `website_language` choice, 8 records its `website_language_request_id`; language fields already written by early version-6/7 builds are preserved without inventing an absent request id; a version-3 session and a version-4 capture migrate as Deep) | brought forward, saved as current | opened read-only: saving is refused, also by a build that read the file before a newer one brought it forward (builds before version 6 don't check this, so close them before running a newer one on the same project) |
| `<project>/inbox/build-progress.json` | `engine::workspace::progress` (`BuildProgress`; written only by the rebuild run) | `BUILD_PROGRESS_VERSION` = 2 (`version`; missing = 0; 2 adds `checked`, the round kept while nothing it looked at changes) | brought forward; an absent file starts with no counted turns | the build stops and the record is left unchanged |
| `<project>/inbox/notes/<id>*.json` | `ai::rebuild::notes` (`NoteFile`, JSON Schema derived with `JsonSchema`) | 1 (`{version, id, text, left}`; `left` is the person's `{text, shown, pictures}` and optional original `added` time) | unversioned waiting notes keep their original words; legacy `.read.json` files remain excluded from replay | not claimed; restart stops with the notes directory unchanged |
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
| `<project>/feed.json` | `ai::rebuild::feed` (written by the run) | `FEED_VERSION` = 5 (`{version, items}`; a bare list = 0; 4 adds `PlanDismissed`, a folded plan card; 5 marks open steps stopped at a run's end) | read as before, saved as current | never written over; the conversation is rebuilt from the project for display |

A saved `User` message with pictures is `{text, pictures: [paths]}`, where the pictures are the
copies in the project's `attachments/`; one without pictures stays the plain string earlier
versions wrote.

Other JSON in a project (`assets.json`, `inbox/` except `build-progress.json`, `queued-request.json` and `notes/`, `checks/`,
evidence records) has no version yet; add one the first time its shape changes.

## Manifest updates

Writers of an existing project's `craft.json` share the operating-system lock in
`craft-manifest.lock`. Ordinary saves read and preserve the latest person-owned fields while
holding it. Person edits and website-language recording hold the same lock through their
comparison and atomic manifest replacement, so a delayed decision is checked against the
current language choice and request id before it is accepted. Automatic request ids are also
recomputed when that saved state changed. A rejected person-field proposal leaves the saved
manifest and the caller's handle unchanged.

The first manifest in a newly allocated or accepted folder is published before this
coordination file is needed. If that initial write fails, it does not leave a lock-file clash
that prevents creating the project in the folder again.

The sidecar remains in the project after unlocking; a new writer and one already waiting must
lock the same file. Opening it does not create a missing project directory. It is a Craft-owned
record, excluded from an app-in-place copy and removed when the project is deleted. This
coordinates manifest writers using this API; it does not make older builds or external editors
participate, or protect against concurrent project deletion or replacement of the directory or
lock file. A successful save keeps the existing manifest format and version.

## Waiting builder notes

A version-1 note is first saved as `<id>.json`. The builder's hook claims it with no-replace
publication to `<id>.read.json`; that proves hand-over, while recording request state such as
the website language can still fail. A hard link is preferred; when unavailable, the pinned
`tempfile` implementation attempts a no-clobber rename (native exclusive rename on supported
Windows, Linux and Apple filesystems). The full claim path is probed with disposable files on the
project's drive before a new note is accepted: if neither operation is supported, sending fails
with the person's draft retained. A successful exclusive rename moves the source, so a completed withdrawal cannot
be copied back into a new claim. An existing claim is never overwritten, and a failed publication
retains the waiting record. A failed claim-directory sync keeps both names after a hard link,
or the recoverable read name after an exclusive rename. Once request recording
succeeds, the worker syncs the read link, removes its matching waiting name and syncs that removal
before renaming the read file to `<id>.recorded.json`. A failure before that rename keeps the claim
recoverable; completion never leaves a waiting alias beside its marker. The completed marker is
synced before cleanup, and cleanup errors are reported instead of discarded. A completed record
left after a cleanup failure is excluded from replay.

Restart validates the full waiting/claimed/completed set before changing any record. Version-1
waiting and claimed notes with kept original words are recovered in id order. An active and a
completed record sharing an id stop startup even if the active record has no recovery words;
the live hook also refuses to claim that collision. A lone completed record does not replay.
Unreadable entries or newer formats stop startup with the directory unchanged. Legacy claimed
notes carry no evidence that request recording was unfinished, so they remain excluded.

The initial message carries each note's id and original request boundaries. Recovered notes are
merged between those parts in id order, before an unrelated new message. The language choice
and the id of its latest explicit request are saved together in `craft.json`. A delayed older
note can still reach the builder after a restart, but it cannot replace a newer saved language
choice. New request ids advance beyond that saved id and all occupied note filenames; exhaustion
returns an error. An older language choice without a recorded request id migrates without
inventing one; the next explicit request establishes its ordering record.

Known note times survive recovery and joined-message handoff unchanged, so a request from
before a Ghidra stop cannot clear it just because the app restarted. A genuine new message
keeps its own request time. Legacy notes with no saved time retain the existing treatment as
new requests when recovered; recovery does not rewrite that absent time in their raw records.

Recovery remains at-least-once: a builder can receive a previously claimed note again after an
interrupted recording. Carried initial notes and notes prepared for a follow-up prompt remain
claimed until their builder turn succeeds; a failed intervening progress save leaves them
recoverable. Claiming uses no-replace publication so an existing read record is preserved for
inspection. During a builder turn, only a continuous read prefix of the id-ordered queue is
recorded, so a claim arriving during a scan cannot apply a newer language before an earlier
request. Before the following prompt is sent, notes already claimed by the hook are confirmed
without including their text again. The worker remembers claims it reserved for an unsent
prompt separately: a partial preparation failure cannot mistake those claims for hook delivery.
This ownership is local to the worker; restart continues to use the saved records above.

A batch completion failure keeps only its unfinished notes in the next handoff. A completion
marker this writer published or fully validated is excluded from that handoff even if its final
directory sync fails; the persistence error is still reported. Successful builder turns save
their reply and check-only state before note cleanup, so a final cleanup sync failure cannot
make a message-free retry repeat a completed builder turn.

The queued request keeps the person's words, attachment paths and original request time, not a
builder choice, mode, run budget or queue position. The next manual Continue restores it; a new
message is refused while that saved request remains, so it cannot silently replace unread words.
The app keeps the refused message in its prompt box. Version 3 requires the original `asked` time,
including its subsecond precision. Versions 1 and 2 lacked it and conservatively use the Unix epoch, so restarting cannot
make an old request clear a later timestamped Ghidra stop. A new explicit message keeps its own
request time. The file is capped at 16 MiB; malformed, oversized, unreadable and newer records are
never treated as an absent request or overwritten. Reading does not consume it. Starting a
worker removes the matching saved record; a failed queued start or a pause leaves it stopped and
available through Continue after reopening, even when the saved feed predates the failure. This
backup does not provide exactly-once delivery across a crash after the worker has started.

A queued handoff keeps each complete request part through the same version-3 carried-note
records. Recovery reconstructs their order only when their combined words, shown words and
pictures account for the entire message. Older records whose carried notes describe only part
of a message retain that complete message and omit only exact matching waiting notes.

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
