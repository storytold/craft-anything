# File formats

The table below lists the files that carry a Craft Anything format version and how older and newer
versions are handled. When a file has migration steps, Craft Anything parses it into raw JSON or
TOML, runs the steps up to the current version, then decodes it into Rust types. The step runner
is `crates/engine/src/versioned.rs`.

| File | Owner | Current version | Older files | Newer files |
|---|---|---|---|---|
| `<project>/craft.json` | `engine::workspace` (`Manifest`) | `MANIFEST_VERSION` = 6 (`version`; missing = 0; 3 adds `approvals`, 4 adds the saved builder session's mode `agent_session_deep`, 5 adds the current capture's mode `capture_deep`, 6 adds the toolchain a move to another language was confirmed with, `toolchain`; a version-3 session and a version-4 capture migrate as Deep) | brought forward, saved as current | opened read-only: saving is refused, also by a build that read the file before a newer one brought it forward (builds before version 6 don't check this, so close them before running a newer one on the same project) |
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
| `<project>/feed.json` | `ai::rebuild::feed` (written by the run) | `FEED_VERSION` = 5 (`{version, items}`; a bare list = 0; 4 adds `PlanDismissed`, a folded plan card; 5 marks open steps stopped at a run's end) | read as before, saved as current | never written over; the conversation is rebuilt from the project for display |

A saved `User` message with pictures is `{text, pictures: [paths]}`, where the pictures are the
copies in the project's `attachments/`; one without pictures stays the plain string earlier
versions wrote.

Other JSON in a project (`assets.json`, `inbox/` except `build-progress.json` and `queued-request.json`, `checks/`,
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
