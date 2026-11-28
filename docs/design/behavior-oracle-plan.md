# Behaviour oracle: what was asked for, what was built, what is left

Status: working document · Scope: the behaviour oracle work · Roadmap: R2.5 (behaviour checks for
non-web builds) · Pull requests: [#2387](https://github.com/storytold/craft-anything/pull/2387)
(the tool-layer fix), [#2388](https://github.com/storytold/craft-anything/pull/2388) (the oracle
seam and the four oracles)

This is the working record of one piece of work: what it was for, how it is put together, what
was measured, and what is still missing. [behavior-oracle.md](behavior-oracle.md) is the design
of the thing itself; this is the record of the decisions behind it and the state of the work.

## Why it was asked for

The question behind the whole thing was one sentence: **can this thing replicate any software,
identically?**

The answer is no, and the project's own design already says so — the mechanic's sheet is
explicit that an unverified value stays unverified, and the clean-room boundary says outright
that the original's insides are off limits. What was missing was the machinery that turns that
admission into something a build can be *judged* against, rather than merely believed about.

Three gaps in what existed:

| Kind of build | What was compared | What was missing |
|---|---|---|
| Website | The original's and the build's pictures | What the page *does*, not only how it looks |
| Console program | The words both printed | The behaviour behind the words |
| Game | Nothing — the build's own `--self-test` report | The original as the judge |

A game was the worst of the three: the build wrote the report that said the game worked.

## The shape of the answer

A **behaviour oracle** runs the same scripted experiment against the original and against the
build, and compares what the two saw. The rule that makes it worth anything:

> **The script must come from the original, never from the build.**
> A build that chooses its own test chooses its own oracle.

That is the mistake the game's `--self-test` makes today, and it is why the game's own report
cannot be the verdict.

### Three rules, the same for every oracle

| Difference | Meaning | Effect |
|---|---|---|
| `Value` | the two sides read a different value | a failed claim |
| `Missing` | one side could read it and the other could not | a failed claim |
| `Unreadable` | neither side could read it | an unknown, never a pass |

The last rule is the one that matters. A missing reading is not a reading of nothing, so it
never counts as alike. This is the rule the mechanic's sheet already followed, generalised to
every kind of build.

### What the build is, is a capability

A program to start, a page to drive, a report to read, a program to start *and watch* — each its
own trait, so an oracle declares only the ones it has, and a caller that does not have them is
told plainly instead of halfway through:

> "The web oracle needs something this build does not have."

The set is open on purpose. A fifth capability is a fifth trait and one more `Capability`
variant; nothing else has to change. (This replaced an earlier `Needs` bitmask, which would have
forced an edit in every other place for every new capability.)

### An oracle drives the engine, it does not open its own channel

`OracleCx` reaches the engine through one function:

```rust
pub struct OracleCx<'a> {
    pub call: &'a dyn Fn(&str, Value) -> Result<ToolOutput, ToolError>,
    pub target: &'a TargetRef,
    pub deep: bool,
    pub program: Option<&'a dyn RunsAProgram>,
    pub app: Option<&'a dyn DrivesAnApp>,
    pub page: Option<&'a dyn DrivesAPage>,
    pub report: Option<&'a dyn ReadsAReport>,
    pub sheet: Option<&'a dyn NamesValues>,
}
```

So an oracle drives the same tools the builder does and inherits everything the engine
guarantees: external programs through the platform layer, the session lock held correctly, a run
cancelled when the person pauses. **An oracle never starts a process of its own.**

## The code that was built

### The seam — `crates/engine/src/oracle.rs`

| Type | What it is |
|---|---|
| `Script` | the steps, the probes, and what the original was seen to do — one observation per step |
| `Observation` | one reading per probe, in the script's order; a probe that could not be read is `None` |
| `Reading` | a value and the engine that produced it |
| `Probe`, `ProbeKind` | what to read: exact, words, picture, fields |
| `Difference`, `DifferenceKind` | `Value`, `Missing`, `Unreadable` |
| `BehaviorOracle` | the trait: `id`, `supports`, `needs`, `record`, `replay`, `compare` |
| `OracleCx` | the context: the engine call, the target, the capabilities |
| `TheProgram`, `TheApp`, `ThePage`, `TheReport` | the capability holders |

`compare()` walks the original's observations and the build's side by side, step by step, and
names the step it is talking about. A value that differs reads:

> at step 0 the original read {"move":["3.1"]} but the build read {"move":["9.9"]}

and one side reading and the other not reads:

> at step 3 (sign-in) the original read the page's text but the build could not

### The four oracles — `crates/ai/src/rebuild/`

| File | Oracle | Needs | Reads the original's inside? |
|---|---|---|---|
| `oracle_console.rs` | `console` | `Program` | No — runs it and reads what it printed |
| `oracle_web.rs` | `web` | `Page` | Yes — it reads the page's markup, so it is recorded in Deep mode |
| `oracle_game.rs` | `game-mechanics` | `Report` + `NamesValues` | Yes — the original's own constants, so it is recorded in Deep mode and refuses without it |
| `oracle_native.rs` | `native` | `App` | No — starts the program and watches its window from outside |

The console and native oracles work in Clean room: watching a program from outside is a
visitor's view. The web and game oracles read the original's inside, so they are recorded in
Deep mode; the game oracle says so plainly and records nothing without it.

`oracles.rs` holds the registry: `built_in()`, `by_id()`, `for_target()`, `record()`, `judge()`,
`can_judge()`.

### The engine's side — `crates/engine/src/tools/`

- `native_session.rs` — three tools (`native_program_session_begin` / `_show` / `_end`) so a
  program can be started and watched. Whitelisted in `clean_room.rs`: starting a program and
  watching it from outside is a visitor's view.
- `apple_runtime/native_ui_helper.swift` — a step carries a value, and `AXSetValue` writes the
  attribute directly, because a value is an attribute and not an action.
- `apple_runtime/native_ui_windows.ps1` — the same through the value pattern.

### Where a script is stored

Beside the capture, in `capture/script.json`. `craft.json` gains one optional field naming it.
`MANIFEST_VERSION` went from 5 to 6 for it; a project made before this keeps the checks it had.

## What was measured

Every claim below was measured, not argued.

### The web chain

A page whose button reports the field's value:

| session | what the page showed at the end |
|---|---|
| fill, then click | `Welcome craft-anything` |
| click only | `Welcome` |

A rebuild that only presses the button is caught.

### The web groups

A choice that shows more of the form. The groups are found by observation, not by guessing: the
control is pressed on its own, the page is read again, and what appeared is that control's
group.

```
the first group:  fill, click, click, click
the second group: select, fill, fill     ← the choice, then the fields it showed
```

### The web dependencies

A country decides which regions are offered. The region choice was on the page all along, so
nothing *appeared* when the country was chosen — only what the region *offers* changed. So what
is compared is not which controls appeared but **what each control offers, before and after**:

```
page as it is:            region's options = []
after choosing Germany:   region's options = ['de-b', 'de-h']
after choosing the US:    region's options = ['us-n', 'us-c']
```

### The game sheet

The sheet is the vocabulary both sides speak. The original's code has numbers and function
names; the build's report has numbers and step names; the sheet says which number is which. The
difference is reported in the sheet's terms — a build that walked 9.9 metres where the original
walked 4.5 is caught, even though its own self-test says it passed.

### The self-test is no longer the verdict

It is still read, and still matters: a step the build says **failed** is a real failure. What it
can no longer do is **pass**. With a comparison recorded, its passing steps are corroboration —
reported, but not holding the check up.

## The three bugs this work found

All three were found by measuring, and all three were silent: they reported success while doing
nothing.

### 1. A role lookup could answer with the wrong node

`queryAXTree` is a *search*, not a filter: it ranks what it finds by how well it matches and
answers with the best few, and the first of them need not have the role that was asked for. A
search for a combo box named "Method" could answer with the group around it. Acting on that does
nothing, while the step reports that it worked.

```rust
.filter(|n| match n.pointer("/role/value").and_then(Value::as_str) {
    Some(r) => r.eq_ignore_ascii_case(&wanted),
    None => true,
})
```

An answer that names no role is still taken, because the search was asked for that role anyway.
Fixed in [#2387](https://github.com/storytold/craft-anything/pull/2387) — it affects every user
of the walkthrough, not only the oracle. The test
`a_role_lookup_skips_an_answer_of_another_role` fails before the fix and passes after it.

### 2. A choice must be given the option's value, not its label

The accessibility tree says "Card"; the markup says `value="card"`. Choosing the label silently
did nothing.

### 3. A choice with no accessible name cannot be found by role and name

The walkthrough refuses an empty name, and a role query with an empty name matches the wrong
node. The markup's `id` is the way in — the same answer the web path uses for the option's
value, and the one the native path uses for a control with no title.

## The decisions that shaped the code

### The markup is read beside the accessibility tree

The tree says what a control is *called*; the markup says what it *is*. Both are needed, and
the tree cannot be trusted for options: a page whose inline script mentions option labels puts
those labels into the tree as if they were options.

### A page with two controls of the same kind is told apart by order

The tree and the markup list the controls in the same order, so the *n*-th control of a role in
the tree is the *n*-th of that role in the markup. Pairing them by role alone gave both the
first one's selector.

### Every option of a choice is tried

A choice offers several options and only some of them change the page. The option that changed
the most is kept. The default is tried too: a page's default may hide what it shows until the
choice is made, so the default can be the option that matters.

### The session has a budget

Each control tried on its own costs a browser session, and every action runs twice. At most
twelve controls are tried on their own, and at most forty actions in the whole session. A page
with a hundred choices cannot make a study take a hundred browser sessions.

### The native oracle's groups come from the tree's depth

The accessibility tree says how deep each control sits, and lists a container's children
together, so a control at a different depth has entered a container of its own. Within a group
the fields are filled before the buttons are pressed.

### A control with no title is named by its identifier

An `NSTextField` has no accessible title, and a field the walkthrough cannot find is a field the
oracle cannot drive.

## How the work was verified

| Check | Result |
|---|---|
| `cargo test --workspace --lib -- --test-threads=1` | 3128 passed, 0 failed |
| `cargo clippy --workspace --lib --tests` | 0 warnings (4 pre-existing manifest warnings) |
| `cargo fmt --check` | ok |
| `cargo xtask layers` / `locks` / `secrets` / `schemas` | all green |
| live web tests (real Chrome) | 7 passed |
| live native tests (real window) | 2 passed |

The one failure in the suite is `ui-egui`'s
`a_non_text_folder_is_refused_before_a_look_starts` ("Illegal byte sequence", an invalid-UTF-8
filename). It was verified on the unmodified tree with `git stash`, so it is not this work's.

The native live checks run with:

```sh
cargo xtask native-oracle        # the native oracle's live checks
cargo xtask native-oracle --all  # and the accessibility helper's own check
```

They drive a real window and read it back, so they need the Accessibility permission. The
checks say so plainly when it is missing.

## What is left

### The real gaps

1. **The groups are found by acting on each control once, not by searching over paths.** A
   dependency that only shows along a particular sequence of choices is not found.

### What was measured, and then built

The value dependency — a choice that changes *what a field holds*, rather than what appears or
what another choice offers — was measured before it was built, because comparing values is prone
to false positives.

**The measurement.** Each control on a page was acted on alone, and the page read again; a
control whose value changed, and which was not the control that was acted on, is a dependency.
Measured on three real pages (a form with eleven fields, a to-do page, an encyclopedia) and on a
page built for the purpose:

| page | controls | value dependencies |
|---|---|---|
| a form with eleven fields | 11 | 0 |
| a to-do page | 1 | 0 |
| an encyclopedia | 2 | 0 |
| a page with a plan that rewrites a name | 3 | **1** |

**The false positive the measurement found.** On one page, two fields looked like dependencies —
until the values were read: they were the page's own `value="John"` and `value="Doe"`, and the
fields had changed because *the probe typed into them*. A dependency is another control changing,
never the one that was acted on. Without that rule, the measurement reports the probe's own
action as a dependency.

**So it was built.** A field's value is now readable, and a control whose value changed is
brought into the session — and the field is *read* rather than filled, because the choice decides
what it holds and typing into it would not be the same behaviour.

Two things came with it:

- **The accessibility tree keeps the value now**, where before it kept only the role and the name.
  The native helper already did this; the web one did not.
- **A password field's value is never read.** Chrome reports a password field as an ordinary text
  box with no mark in the tree to tell it apart, and reports its value as one bullet per
  character — which is the password's *length*. The bullets are therefore never kept, and a read
  of a password field is refused outright, by the element's own `type`.

### The gaps that remain

2. **The native session's budget** was added with the web one: sixteen groups and sixty actions.
3. **A dependency whose effect is a control's *value* is found, but one whose effect is neither
   that, nor what appears, nor what is offered** — a choice that changes a field's *value* in a
   way the accessibility tree does not report, say — is still not found.
4. **The groups are still found by acting on each control once**, not by searching over paths.

### The work around it

5. **Windows' `setvalue` path is unverified.** The code is written (`ValuePattern::SetValue`),
   but there is no Windows host to run it on.
6. **The game's mechanics oracle has no live test.** Its numbers are compared against the
   original's decompiled constants, which is verified by unit test; a game that runs is not.
7. **The native live tests cannot run on this machine.** The accessibility tree reports
   `AXApplication` for every element whose role it cannot determine — including a text field's
   own element, and every window of every app, Finder included. The structure is right (a menu
   bar and a scroll area come through with their own roles), but the roles and the values do not.
   This was confirmed to be **not** this work: the pristine upstream helper, with none of these
   changes, returns the same empty tree. It is a condition of macOS 27.0.1 on this machine, and
   the native live checks are `#[ignore]`d, so the suite stays green.

   Two real bugs were found while chasing it, and both are fixed:

   - **A program started by the session tool died with the tool call.** The tool's process
     supervised the program and killed the group when it exited, so the window was gone before
     anyone could read it. The program is now launched fire-and-forget, the way a program that is
     meant to outlive its launcher is.
   - **The oracle read the window before it drew.** It now waits for the window, so a program
     that takes a moment to draw is still read.

### Housekeeping

6. **`GIT_CONFIG_*` environment variables** — found and fixed. The Harness's shared subprocess
   scrub drops any variable whose *name* looks like a credential, and `GIT_CONFIG_KEY_0` does.
   The count and the values survived, so git was handed a count naming pairs that were not there
   and refused every command with `missing config key GIT_CONFIG_KEY_0`. The count now goes with
   the keys it counts.

   This was not only this project's problem: the plugin manager's GitHub connection check runs
   `git ls-remote` through that scrub, so on a machine whose parent environment carries that
   configuration it failed outright and reported the connection as broken. The same call was
   verified to fail before the fix and to work after it.

   The Harness does not accept external pull requests, so the fix is posted as a discussion with
   the branch ready to apply:
   https://github.com/deepseek-ai/deepseek-harness/discussions/14307

## The pull requests

Two, because GitHub would not take a fork branch as a base and the upstream's convention is
flat pull requests against `main`.

| Pull request | What it holds | Size |
|---|---|---|
| [#2387](https://github.com/storytold/craft-anything/pull/2387) | the role lookup fix | 2 files, +96 |
| [#2388](https://github.com/storytold/craft-anything/pull/2388) | the seam and the four oracles | 37 files, +5656 |

The two were separated because the role lookup fix is worth landing on its own: it affects
every user of the walkthrough, it is small, and it carries no dependency on the oracle work.

A third pull request was opened and closed: the web, native and game oracles were first split
from the console oracle and the seam, but they sit on that seam and could not be reviewed
separately against `main`.

### A note on the continuous integration

Both pull requests sit at `action_required`: the gate workflow has not run. This is not
particular to them — every open pull request in the repository is in the same state, including
other contributors', and the shared workflow comes from another repository and needs a
maintainer's approval before it runs. CodeRabbit has run and passed.
