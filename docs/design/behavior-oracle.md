# Behaviour oracle: a design

Status: proposal · Scope: `crates/ai/src/rebuild/kinds/` + a new seam in `crates/engine` ·
Depends on: R2.4 (scripted input) for the native path; nothing new for the web and console paths.

**The console, web, game and native oracles are built, behind one trait.**
`crates/engine/src/oracle.rs` holds the seam (`Script`, `Probe`, `Observation`, `Difference`,
`compare`) and the trait (`BehaviorOracle`, `OracleCx`);
`crates/ai/src/rebuild/oracle_console.rs`, `oracle_web.rs`, `oracle_game.rs` and
`oracle_native.rs` hold the four oracles; `crates/ai/src/rebuild/oracles.rs` holds the registry
that holds them; `oracle_line.rs`, `oracle_web_line.rs`, `oracle_web_live.rs`,
`oracle_game_line.rs` and `oracle_native_line.rs` hold the end-to-end tests. Study records what
the original did, in `manifest.script`, and the check compares the build with it.

## The trait

```rust
pub trait BehaviorOracle: Send + Sync {
    fn id(&self) -> &'static str;
    fn supports(&self, target: &TargetRef) -> bool;
    fn record(&self, cx: &OracleCx) -> Result<Option<Script>, ToolError>;
    fn replay(&self, cx: &OracleCx, script: &Script) -> Result<Observation, ToolError>;
    fn compare(&self, script: &Script, build: &Observation) -> Vec<Difference> {
        compare(script, build)
    }
}
```

The recording and the replay are separate on purpose: the script must come from the original,
never from the build. A build that chooses its own test chooses its own oracle.

`OracleCx` is how an oracle reaches the engine:

```rust
pub struct OracleCx<'a> {
    pub call: &'a dyn Fn(&str, Value) -> Result<ToolOutput, ToolError>,
    pub target: &'a TargetRef,
    pub deep: bool,
    pub program: Option<&'a dyn RunsAProgram>,
    pub page: Option<&'a dyn DrivesAPage>,
    pub report: Option<&'a dyn ReadsAReport>,
}
```

One function reaches the engine, so an oracle drives the same tools the builder does and
inherits everything the engine guarantees: external programs run through the platform layer,
the session lock is held correctly, and a run is cancelled when the person pauses. **An oracle
never starts a process of its own.**

### What the build is, is a capability

The build can be a program to start, a page to drive, a report to read, or a program to watch.
Those are separate capabilities, each with its own trait, so an oracle declares only the ones it
has:

| Trait | The build is | Held by | Used by |
|---|---|---|---|
| `RunsAProgram` | a program to start | `TheProgram` | console |
| `DrivesAPage` | a page to drive | `ThePage` | web |
| `ReadsAReport` | a report to read | `TheReport` | game |
| `DrivesAnApp` | a program to start and watch | `TheApp` | native |

The set is open: `Needs` holds a list of `Capability` values, so a new kind of capability is a
new variant and nothing else has to change to make room for it.

The context holds them as trait objects, so the caller says what the build is and the oracle
asks for what it needs. A web oracle never sees a program; a console oracle never sees a page.

An oracle also says what it needs, so a caller that does not have it is told plainly rather
than the oracle finding out halfway through:

```rust
fn needs(&self) -> Needs { Needs::PROGRAM }
```

`Needs` is a small set of bits — `PROGRAM`, `PAGE`, `REPORT`, `APP` — and `met_by` says whether
a context has them. The registry refuses to judge with what is not there:

> "The web oracle needs something this build does not have."

rather than failing in the middle of a run.

### Sequences: one observation per step

The first version of the seam compared **one** observation — where the program ended up. That
misses the thing that matters most about a sequence: what each step *changed*.

So the script holds **one observation per step**:

```rust
pub struct Script {
    pub version: u32,
    pub oracle: String,
    pub steps: Vec<Value>,
    pub probes: Vec<Probe>,
    /// What the original was seen to do, one observation per step.
    pub original: Vec<Observation>,
}
```

and the comparison walks the steps:

- a step whose readings differ is a difference, and **names the step**;
- a step the build never reached is a difference of its own;
- a step neither side could read is an unknown, never a pass.

A sequence that ends where the original ended but went differently is caught — the end state
alone would have missed it. That is the whole point of the change.

Recording a sequence is the oracle's job, and each does it the way its subject is driven:

- **the web oracle** drives the page through its controls in one session and captures the
  accessibility tree after each action, so each step's tree is that step's reading;
- **the native oracle** starts the program, reads its window, acts on one control, reads again,
  and keeps what it saw after each action;
- **the console oracle** runs the program once per input, and each run is a step;
- **the game oracle** reads the build's own report, which is one reading.

The web oracle no longer needs its own step-by-step comparison: the generic one does it, and
the special case is gone.

### Chains: the fields are filled before the buttons are pressed

A probe-style session — act on each control on its own — misses the behaviour that matters most
in a real program: **a control that depends on another**. A sign-in page's button does nothing
useful unless the field beside it was filled first. Acted on separately, both look fine.

So a session is a **chain**, not a list:

1. open the page
2. fill every field, with the same plain text on both sides
3. press every button

The chain is what shows the dependency. Measured on a page whose button reports the field's
value:

| session | what the page showed at the end |
|---|---|
| fill, then click | `Welcome craft-anything` |
| click only | `Welcome` |

**A rebuild that only presses the button is caught**, because the chain shows the value that
should have been there. A probe-style session could not have told them apart.

The fill text is the same on both sides (`craft-anything`), so the two runs differ only in what
they do with it. The native oracle does the same thing: it sets each field's value, then presses
each button, and compares what the window showed after each step.

Setting a field's value needed one thing that did not exist: the accessibility walkthrough could
press a control but had no way to put text into it. `AXSetValue` is not a standard accessibility
action — a value is an attribute — so the helper now writes the attribute directly, and a step
carries the value with it. The Windows path does the same through the value pattern.

The write was checked against a real accessibility element: a window with a field of its own,
whose value went from `before` to `after` (`crates/engine/src/tools/apple_runtime/ax_value_selfcheck.swift`,
which anyone with the Accessibility permission can run). The project's own test
(`the_helper_puts_text_into_a_field`) does the same through the tool, and is ignored by default
because it needs the permission.

### Groups: a control that shows other controls

A chain is still not enough. A page's controls are not all on the page at once: a choice shows a
part of the form, and that part has fields of its own. Filling every field first and pressing
every button after would fill fields that are not on the page yet, and would press a button
before the choice that makes it meaningful.

So the session is a chain of **groups**:

1. the fields that are here now are filled, and the buttons that are here now are pressed;
2. pressing one of them may show more controls, and those are filled and pressed next.

The groups are found by **observation, not by guessing**. A control is acted on on its own, the
page is read again, and what appeared is that control's group. Measured on a checkout page whose
choice shows a card number and a CVC:

```
the first group:  fill, click, click, click
the second group: select, fill, fill     ← the choice, then the fields it showed
```

A build that shows the choice but never the fields it reveals is caught, because the second
group's readings differ.

#### The markup is needed, not only the accessibility tree

Finding the groups raised a question the accessibility tree cannot answer on its own: **how is a
control addressed, and what value does a choice take?**

- A control with no accessible name cannot be found by role and name at all — the walkthrough
  refuses an empty name, and a role query with an empty name matches the wrong node.
- A choice is chosen by the **value** of one of its options (`card`), not by its label (`Card`).
  Choosing the label silently does nothing, and the step still reports itself as fine.

Both answers are in the page's own markup: the `id` says how to find a control, and the
`option`'s `value` says what a choice takes. So the markup is read beside the accessibility
tree, and the selector is preferred when there is one.

This is also a finding about the tool itself: the role-and-name locator silently matched the
wrong node for a `<select>`, reporting success while nothing happened. **The oracle would have
believed it.** The markup is what makes it addressable.

The locator is now fixed, for everyone who uses it: the query is a *search*, not a filter, so the
role of each answer is checked and an answer of another role is skipped. The test
`a_role_lookup_skips_an_answer_of_another_role` locks it in — it fails with the old code and
passes with the new.

#### The native groups

A native window is grouped the same way, by the container a control sits in. The accessibility
tree says how deep each control is, and the tree lists a container's children together, so a
control at a different depth has entered a container of its own and starts a group. Within a
group the fields are filled before the buttons are pressed.

#### A dependency between two choices

A choice can decide what *another* choice offers: a country decides which regions are offered.
The dependency is invisible to a method that looks for controls that **appeared**, because the
region choice was on the page all along — only what it *offers* changed.

So what is compared is not which controls appeared but **what each control offers, before and
after**. A control whose options changed depends on the control that was pressed, and it is
brought into that control's group.

Two things make this work:

- **The options come from the markup, not the accessibility tree.** The tree says what a control
  is called; the markup says what it offers. A page whose inline script mentions option labels
  puts those labels into the tree as if they were options, so the tree is not to be trusted
  here.
- **Every option is tried, not just the first.** A choice offers several options and only some
  of them change the page. The option that changed the most is kept: a page's default may hide
  what it shows until the choice is made, so the default can be the option that matters.

A page with two choices of the same kind gives each its own selector by pairing the tree's
controls with the markup's elements **in order**, the order both list them in.

A choice can also change what a *field holds*. That is found too: a control whose value changed
is brought into the group of the control that changed it, and it is **read** rather than filled,
because the choice decides what it holds and typing into it would not be the same behaviour.

Two things make this safe:

- **The control that was acted on is left out.** Its value changed because it was acted on, not
  because it depends on anything. Without that rule every probe looks like a dependency.
- **A password field's value is never read.** Chrome reports a password field as an ordinary text
  box, with no mark in the tree to tell it apart, and reports its value as one bullet per
  character — which is the password's length. A read of a password field is refused outright, by
  the element's own `type`.

What this still does not find: a dependency between two controls whose effect is in none of
these — a choice that changes something the accessibility tree does not report, say. The groups
are found by acting on each control once, not by searching over paths.

#### The session is bounded

Each control a session tries on its own costs a browser session, and every action is run twice
(the original, then the build). So the session has a budget:

- at most twelve controls are tried on their own, to see what they show;
- at most forty actions in the whole session.

A group that does not fit is left out, and the groups before it are kept. A page with a hundred
choices cannot make a study take a hundred browser sessions.

### The native oracle: a window, watched from outside

A windowed program has no page and no report. What it has is a window, and a window can be read
and acted on from outside — which is a visitor's view, so this oracle works in Clean room.

It needed one thing that did not exist: **a program that stays running while it is watched.**
The picture tool starts a program, waits, pictures it and closes it; a walkthrough needs it up.
So there are three new engine tools:

- `native_program_session_begin` — start a program and leave it running
- `native_program_session_show` — report the open sessions
- `native_program_session_end` — close one

They use the platform's process layer, so the program inherits everything that layer guarantees:
a deadline, a cancelled run killed with its process group, and output that never fills a pipe.

The oracle then starts the original, reads its window's controls, closes it, and does the same to
the build. The controls are read as labelled values — role and title, and a field's value — so
two programs showing the same controls in a different order behave the same way.

On macOS the window read needs Accessibility permission, and the picture needs Screen Recording.
Without them the tools say so with a typed reason (`accessibility_not_granted`) rather than
returning an empty tree, so a missing permission is never mistaken for a program with no
controls.

The native oracle's live checks are run with `cargo xtask native-oracle` (and `--all` for the
helper's own check). They drive a real window and read it back, so they are the proof that the
whole native path works: the session is recorded, the controls are grouped by the container they
are in, and the build is compared with the original.

A control with no accessible name is named by its identifier, the way the web path names a
control by its id: an `NSTextField` has no title, and a field the walkthrough cannot find is a
field the oracle cannot drive.

The registry is a trait plus a list, the way the project's other seams work: adding an oracle is
a file and a registration line.

One thing the registry is not: it does not pick an oracle from the target alone. A game and a
console program are both a program on this computer, and which one it is comes from the request
and the stack, not from the file. So the caller — which knows the kind — asks for the oracle it
wants by id, and the registry holds them all.

## The problem this solves

The website check compares the build against the original's own pictures: the original is
photographed in Study, the build is photographed in Check, and the two are compared with SSIM.
The original is the oracle. A difference in pixels is a difference in behaviour, measured against
something that did not come from the build.

The other two kinds have no such oracle:

| Kind | What is compared today | Where the comparison comes from |
|---|---|---|
| Website | Screenshots of the original vs. screenshots of the build | The original — independent |
| Console app | The original's runs vs. the build's runs, over three inputs | The original — independent |
| Game | The build's own `--self-test` report | **The build** — not independent |

A game whose builder writes a self-test that always says `passed: true` passes the check. The
mechanics sheet is the same shape: the build fills in `app/mechanics.json`, and the check records
that its behaviour is unverified, because nothing measured the build against it.

The fix is one missing part: **a way to run the same scripted experiment against the original and
against the build, and compare what came out.** The comparison already exists for two kinds; what
is missing is the scripting for the third, and the wiring so that every kind uses one comparison.

## What already exists, and what is missing

Present, and usable as-is:

- `runtime_walkthrough_record` — runs a program under a terminal with a scripted timeline of
  typed input, resizes and signals; records the terminal screen, the exit, the dispatched events
  and (opt-in) which files changed. Values that change every run — process ids, ports, 10 ms time
  buckets — are normalised so two runs compare. It takes an absolute executable path, so it runs
  the original as readily as the build.
- `runtime_captures_compare` — compares two such records dimension by dimension (terminal, exit,
  filesystem, events) and reports what changed.
- `runtime_ui_walkthrough_record` — drives a running app's screen by pid with accessibility
  actions, recording the UI tree before and after each step.
- `runtime_ui_read` — reads a running app's UI tree.
- `native_window_capture` — pictures a program's window.
- `web_walkthrough_record` — runs a scripted browser session (goto/click/fill/press/select/wait)
  over CDP, capturing per-step screenshots, the DOM, the accessibility tree, network outlines and
  console events, under a fixed environment so two runs of the same steps see the same page.

Missing:

- A way to **record the experiment against the original** and keep it. Study photographs the
  original for websites; it does not script it.
- A way to **generate the script**. Nothing today decides which buttons to press or which inputs
  to type.
- A comparison for the native UI tree and for the web accessibility tree.
- One place where "this build behaves like the original" is decided, shared by all kinds.

## The design

### One seam: `BehaviorOracle`

A new trait in `crates/engine/src/oracle.rs` (L4: it is a capability, not orchestration):

```rust
/// A way to run the same scripted experiment against a target and against a build, so the two
/// can be compared. The original's side is recorded in Study; the build's side runs in Check.
pub trait BehaviorOracle: Send + Sync {
    /// The id of this oracle, stored with the script so a build is compared the same way.
    fn id(&self) -> &'static str;

    /// Whether this oracle can produce an oracle for this target. False when the target kind
    /// is not one it drives, or when the platform it needs is not this one.
    fn supports(&self, target: &TargetRef) -> bool;

    /// Record the experiment against the original. Called once, in Study. The returned script
    /// is stored with the project and replayed, unchanged, against the build.
    fn record(&self, cx: &OracleCx) -> Result<Script, ToolError>;

    /// Run the recorded script against the build and return what it saw.
    fn replay(&self, cx: &OracleCx, script: &Script) -> Result<Observation, ToolError>;

    /// Compare the original's observation with the build's. Returns the differences; a
    /// difference is a failed claim, not a failed check.
    fn compare(&self, original: &Observation, build: &Observation) -> Vec<Difference>;
}
```

`record` and `replay` are separate on purpose: the script must come from the original, never
from the build. A build that chooses its own test chooses its own oracle, which is the mistake
`SelfTestCheck` makes today.

### What a script is

```rust
/// A scripted experiment: the steps, and how to read what came out. Stored with the project
/// so the same steps run against the original and against the build.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Script {
    /// Which oracle recorded it, so a build is compared the same way.
    pub oracle: String,
    /// The version of that oracle's script format. A script from an older version is replayed
    /// by that version's reader or refused, never guessed at.
    pub format_version: u32,
    /// The steps, in the oracle's own vocabulary (a terminal timeline, a list of UI actions, a
    /// list of browser actions).
    pub steps: Vec<Step>,
    /// What the oracle read back after each step, and how it is compared. This is the part that
    /// makes two runs comparable: the same fields, the same normalisation, the same tolerances.
    pub probes: Vec<Probe>,
    /// What the original was seen to do. Recorded once, in Study, and replayed against the
    /// build: this is the oracle.
    pub original: Observation,
}
```

`Step` and `Probe` are oracle-specific. What they share is the contract: a step is something
done to the program, a probe is something read back.

### Where a script is stored

Not in `craft.json`. The manifest is the project's index — its status, its stack, its approvals —
and a script with its steps, probes and a recorded observation is bulk. It belongs beside the
capture, in the project's own folder:

```
capture/
  script.json          the script and the original's observation
```

`craft.json` gains one optional field, `script_id: Option<String>`, naming that file — the same
way it already names a capture. Because the field is optional and the manifest is versioned
(`MANIFEST_VERSION` is 5, with one migration step per version), adding it is one migration step
that does nothing to older projects: a project made before this has no script and stays a
website-only check.

This follows the file-format rule the project already states: every file we write carries a
version, has a schema, and has a migration path (`crates/engine/src/versioned.rs`). `script.json`
carries its own `version` and its own migration steps, separate from `craft.json`'s.

### The script format is versioned per oracle

`Script.format_version` is the oracle's own script format, not the project's. Each oracle owns
its format and its steps, because a terminal timeline and a browser action list have nothing in
common. What they share is the trait above: record, replay, compare.

This is the "data over code for catalogues" rule applied to scripts: the format is a version
number and a table, not a branch on the oracle's name.

### What an observation is

```rust
/// What an oracle saw when it replayed a script.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Observation {
    /// Which oracle produced this, and the script version it replayed.
    pub oracle: String,
    pub format_version: u32,
    /// One entry per probe, in the script's order. A probe that could not be read is `None`,
    /// not a default value: a missing reading is not a reading of nothing.
    pub readings: Vec<Option<Reading>>,
    /// Why some probes could not be read, when any could not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<String>,
}
```

`Option<Reading>` rather than a default matters. If the original's screen could not be read at
step 2, and the build's could, then the comparison at step 2 is **unknown**, not "the build showed
nothing". A missing reading never counts as alike.

### The three comparison rules

Every oracle reduces its observations to the same three kinds of difference. This is the part
that makes kinds comparable with each other, and it is deliberately small:

| Difference | Meaning | Effect |
|---|---|---|
| `Value` | The same probe read a different value | A failed claim |
| `Missing` | The original had a reading and the build did not, or the reverse | A failed claim |
| `Unreadable` | Neither side could be read | An unknown, never a pass |

The rule that matters most is the last one. An unreadable probe produces an unknown, and an
unknown keeps the check from being good enough. That is the same rule the mechanics sheet already
follows (`crates/ai/src/rebuild/kinds/mechanics.rs`), generalised.

### Where it plugs in

Three existing pieces move:

1. **`RunsCheck` becomes `OracleCheck`.** It already does the right thing — original runs vs.
   build runs — but its inputs are three fixed cases from `CASES`, and it compares word likeness.
   It becomes a `BehaviorOracle` whose script is those cases (plus more), and whose probes are
   exit code and normalised output.

2. **`SelfTestCheck` stops being a check.** The build's own self-test is evidence about the
   build, not a comparison. It becomes one probe among several, and it is reported as the build's
   own word. It is never the thing that decides.

3. **A new `BehaviorCheck` implements `Check`.** It reads the script from the project, replays it
   against the build, compares, and turns each difference into a `Claim`:

```rust
impl Check for BehaviorCheck {
    fn id(&self) -> &'static str { "behaviour" }

    fn applies(&self, cx: &CheckCx) -> bool {
        cx.ws.manifest.script_id.is_some()
    }

    fn can_refine(&self, _result: &CheckResult) -> bool {
        true
    }

    fn run(&self, cx: &mut CheckCx) -> Result<CheckResult, ToolError> {
        let script = read_script(cx.ws)?;
        let build = self.oracle.replay(cx, &script)?;
        let differences = self.oracle.compare(&script.original, &build);
        let claims = differences.iter().map(|d| Claim::new(d.title(), !d.is_failure(), d.detail())).collect();
        let unknowns = differences.iter().filter(|d| d.kind == DifferenceKind::Unreadable).map(|d| d.detail().to_string()).collect();
        Ok(CheckResult { claims, unknowns, ..CheckResult::new(self.id(), cx.round, true, differences.note()) })
    }
}
```

`CheckResult::new(..., true, ...)` — the check passes when it ran. What decides good enough is
`CheckResult::good`, which already requires every claim to hold and no unknowns to remain. No
change to `good()` is needed; that is the point of putting the judgement in claims.

### An oracle drives the engine, it does not open its own channel

The oracle does not spawn processes itself. It goes through `CheckCx::call(tool, params)` — the
same entry point the builder uses — so it inherits everything the engine already guarantees:

- external programs run through `craft_anything_platform::process` (closed stdin, deadlines,
  killed process groups, capped output);
- the session lock is held correctly and a stuck holder is an error, not a hang;
- the round's time limit and the person's Pause stop the oracle's own runs
  (`CheckCx::stop` returns the `CheckStop` that carries both);
- the run is journalled like any other tool call.

This matters for the rule that every external program goes through the platform layer. An oracle
that spawned Chrome itself would break that rule and would need its own timeout, cancellation and
process-group handling. An oracle that calls `web_walkthrough_record` gets all of it.

```rust
/// The context an oracle runs in. It is a `CheckCx`, so an oracle drives the engine through
/// `call` and stops through `stop`, and never opens a process of its own.
pub struct OracleCx<'a> {
    pub cx: &'a CheckCx<'a>,
}
```

For the Study side, where there is no `CheckCx`, the oracle is called through the same engine
handle the rest of Study uses. The trait's two entry points therefore take the narrowest thing
that works:

```rust
fn record(&self, cx: &mut OracleCx) -> Result<Script, ToolError>;
fn replay(&self, cx: &mut OracleCx, script: &Script) -> Result<Observation, ToolError>;
```

with `OracleCx` holding an `&Engine` plus an optional `&CheckCx`, so the same oracle serves both
phases without two traits.

### Generating the script

This is the hard part, and it is the part with the most risk. The rule it has to satisfy:

> The script must be derived from the original, and must be replayable against the build.

Two viable approaches, in order of preference.

**Probes, not paths.** Do not try to find "the path through the app that reaches feature X".
Enumerate the original's interactive surface and script one interaction per element:

- From the original's UI tree (or the web accessibility tree), list every element that can be
  acted on.
- For each, one script that acts on it and reads the screen back.
- The script is a **vector of independent one-step experiments**, not a sequence.

This is deterministic, replayable, and cheap. It does not cover behaviour that needs a specific
*sequence* of interactions, which is a real gap, but it covers a large fraction of what a person
would check by hand, and it needs no intelligence to produce.

**A second pass for sequences.** Once single-step probes work, add short scripted sequences
between elements (type into a field, then press the button it belongs to). This is where the
value is, because most real behaviour is "do this, then do that". It needs a way to relate
elements to each other, which the UI tree partly gives (roles, labels, containment).

Both approaches read the original's tree. Neither needs to read the original's code, so both work
in Clean room for the native path — the observation is from outside, exactly as a visitor or user
would see it.

### Environment normalisation

Before this can produce trustworthy results, the two runs must differ only in the program. What
already exists:

- `runtime_walkthrough_record` normalises process ids, ports, the working directory, the temp
  folder and 10 ms time buckets, and takes literal replacement patterns for anything else.
- `web_walkthrough_record` fixes the environment: viewport, locale, time zone, colour scheme,
  reduced motion, service workers, device scale factor.

What is missing, and is the first thing to build:

- **The native UI tree** has no normalisation at all. It contains values that change every run
  (timestamps in a clock, a counter, a random id, a window size). A probe over the whole tree
  would report those as differences on every run.
  The fix is to make probes **selective**: a probe names what to read (a labelled element's value,
  the enabled state of a button, the text of a heading), not "the whole tree". Then a changing
  value is simply not part of any probe.
- **Screenshots** compare pixel-by-pixel in the web path and by SSIM in the website path. A
  cursor blink, a caret, a hover state and an animation frame are all differences that are not
  behaviour. The website path already handles this by photographing the original three times and
  excluding what changes by itself (`noise-<width>.png`); the new path needs the same treatment.

### What each kind gets

| Kind | Oracle | Script source | Probes |
|---|---|---|---|
| Website | `web_walkthrough_record` | The page's own accessible elements | Accessibility tree leaves; screenshot per step (SSIM) |
| Console app | `runtime_walkthrough_record` | The program's own usage text and a fixed case list | Exit code; normalised terminal output |
| Native app | `runtime_ui_walkthrough_record` | The original's UI tree, one action per element | The affected element's value, role and enabled state |
| Game | `runtime_ui_walkthrough_record` + `native_window_capture` | The original's window and its input bindings | The screen after each scripted input; the mechanics values, measured |

The game case is the one that changes the most. The mechanics sheet stops being a document the
builder fills in and becomes a set of **values measured from the build and compared with the
original's measured values** — which is what the check already says it cannot do
(`crates/ai/src/rebuild/kinds/mechanics.rs`).

## What this does not do

- **It does not prove equivalence.** It compares a finite, chosen set of behaviours. The
  original's behaviour outside the probes is unknown, and stays unknown.
- **It does not replace the build's self-test.** The self-test remains useful: it can see the
  game's *internal* state (a speed, a score, a hit count) that no screen shows. It becomes
  corroborating evidence, never the verdict.
- **It does not work in Deep mode only.** The native observation path is outside the original, so
  it works in Clean room. The web path needs the page's DOM, so it stays Deep-only — which is
  already how the website check is arranged.
- **It does not scale to every program.** Some programs need a licence, a server, a device, or
  credentials. An oracle for those is a separate piece of work, and where it cannot exist, the
  check stays what it is today: unverified, and saying so.

## What was measured before this was designed

The design above was checked against the built engine, not reasoned about. A two-program
calculator (`original.py` and `rebuild.py`, differing only in that the rebuild adds one instead
of subtracting, and prints floats without the trailing `.0`) and a two-page web app were run
through the tools above.

**The console path works, and is exact.** One script, two programs:

```json
{"actions": [],
 "events": [{"at_ms": 300, "type": "input", "data": "3 + 4\n"},
            {"at_ms": 600, "type": "input", "data": "10 - 2\n"},
            {"at_ms": 900, "type": "input", "data": "quit\n"}],
 "executable": "/tmp/oracle-test/original.py",
 "terminal": {"columns": 80, "rows": 24},
 "normalization": {"time_step_ms": 10}}
```

`runtime_walkthrough_record` on the original read back the terminal screen:

```
calc> 3 + 4
3.0 + 4.0 = 7.0
calc> 10 - 2
10.0 - 2.0 = 8.0
calc> quit
bye
```

and on the rebuild, with the same script:

```
calc> 3 + 4
3.0 + 4.0 = 7
calc> 10 - 2
10.0 - 2.0 = 12
```

`runtime_captures_compare` reported `overall: changed`, with the terminal dimension showing
exactly two differing lines — `3.0 + 4.0 = 7.0` against `3.0 + 4.0 = 7`, and
`10.0 - 2.0 = 8.0` against `10.0 - 2.0 = 12`. The exit code and the event dispatch were
`unchanged`. The comparison finds the bug and the cosmetic difference, and does not report the
rest of the screen as different.

**The console path is deterministic.** The same script run twice against the original compared
`overall: unknown` with the terminal dimension `unchanged` — the only dimension that moved was
`filesystem`, which neither run observed. Process ids, ports and timing were already normalised
away. Two runs of one program compare equal.

**The web path works, and is semantically exact.** One browser script — `goto`, `fill #n` with
`41`, `click #add`, `wait` — run against the original and against the rebuild. The per-step
screenshots were byte-identical for the first two steps and diverged at the click, which is
where the two pages differ:

```
step 0 goto   SAME        b67b86a930a8 vs b67b86a930a8
step 1 fill   SAME        280df861c92e vs 280df861c92e
step 2 click  DIFFERENT   8c9e7bd12548 vs 0d1227a5c61d
step 3 wait   DIFFERENT   8c9e7bd12548 vs 0d1227a5c61d
```

The accessibility tree gives the difference as text, which is what a probe should read:

```
only in orig:    ['Result: 42']
only in rebuild: ['Result: none']
```

**The web path is deterministic.** The same script run twice against the same page produced
byte-identical screenshots at every step and an identical accessibility tree. The fixed
environment in `web_walkthrough_record` (viewport, locale, time zone, colour scheme, reduced
motion, service workers, device scale factor) is what buys this.

**Two things this also showed.**

*Clean room refuses the web path, correctly.* `web_walkthrough_record` is not in the clean-room
table, because reading a page's DOM is reading inside it. The console path
(`runtime_walkthrough_record`) and the native path (`runtime_ui_walkthrough_record`,
`runtime_ui_read`, `native_window_capture`) **are** in it, because they watch from outside. So
the native oracle can work in Clean room and the web oracle cannot — which matches how the
website check is already arranged.

*The native path needs permissions the CLI does not have.* `runtime_ui_read` returned
`no_permission` / `accessibility_not_granted` and `native_window_capture` returned
`no_permission` for Screen Recording. The implementation is complete — `native_ui_helper.swift`
checks `AXIsProcessTrusted()` and reports a typed reason rather than an empty tree — and the
desktop app has these permissions. The CLI does not, so the native oracle cannot be verified
from the command line on this machine. This is the reason the work below starts with the console
and web paths.

## Risks

| Risk | Why it matters | Mitigation |
|---|---|---|
| **False failures from environment noise** | The builder spends its rounds fixing things that are not broken | Selective probes, not whole-tree comparison; reproduce-before-comparing (run the original twice, discard probes that differ between its own runs) |
| **Script generation is brittle** | A selector that stops matching silently stops testing | Every script records the elements it found; a probe that matched nothing in the original is never compared, and says so |
| **A build games the probe** | If the builder can see the probe list, it can make the probe pass | The probe reads the original's state, not the build's; a probe's expected value comes from the original's observation, recorded in Study |
| **Cost** | Every probe is a program run | Probe budget per kind; run the original once, replay once; reuse the original's observation across rounds |
| **macOS permissions** | The native path needs Accessibility and Screen Recording | Out of scope for the CLI; the desktop app has them. The console and web paths need none, so they land first |

## Order of work

1. **Normalisation and selective probes.** Before any of this is trustworthy, two runs of the
   *same* program must compare equal. This is the gate on everything else.
2. **`BehaviorOracle` trait and the console oracle.** The console path needs no permissions and
   no new observation primitive; it is the cheapest end-to-end proof that the shape is right.
3. **The web oracle.** Reuse `web_walkthrough_record`; add the accessibility-tree comparison. The
   website kind is already the reference implementation of the idea.
4. **The native oracle.** Needs the permissions, and the UI tree's normalisation.
5. **Script generation.** Probes first, sequences second.
6. **The game kind.** Mechanics measured, self-test demoted to corroboration.

Steps 1–3 need nothing that does not exist. Step 4 needs a desktop environment. Step 5 is where
the difficulty is.

### What is done

- **Step 1, for the console path.** `runtime_walkthrough_record` already normalises process ids,
  ports, the working directory, the temp folder and time buckets. The oracle reads a run through
  two probes — the exit code, and the terminal as one piece of text — so a changing value is
  simply not part of any probe. Two runs of one program compare equal; a test says so.
- **Step 2, in substance.** The seam is `crates/engine/src/oracle.rs`; the console oracle is
  `crates/ai/src/rebuild/oracle_console.rs`. The `BehaviorOracle` trait itself is not written:
  it needs one context type that serves both Study (no check context) and Check (one), and
  shaping that is the remaining piece of this step.
- **The line is connected.** Study records the original's behaviour into `manifest.script`
  (in the same pass that captures the runs, so the original is not run twice); the
  `console-behaviour` check is registered for the app kind and compares the build with it.
- **Step 3, the web oracle.** `crates/ai/src/rebuild/oracle_web.rs` drives the original's page
  through its own controls — one action per control the accessibility tree shows — and keeps the
  run. The `web-behaviour` check drives the build through the same session and compares:
  the page's accessible values as labelled pairs, and the two runs step by step through the
  engine's own `web_snapshots_compare`, which names the step and the field that changed.

  The web path needed one piece the console path did not: the original's address and the
  build's address differ, so the comparison is given a normalisation rule that replaces the
  original's address on both sides. Without it every step reads as changed, for the address
  alone.

One flaw was found and fixed while building it: the word comparison stripped a leading sign, so
`42` and `-42` compared as identical. A sign is part of a value, and a comparison that cannot
tell them apart calls a wrong answer right.

### The game: one vocabulary, or none

A game was the hardest case, because its check was the one that could not judge itself. The
build's `--self-test` report said the core loop worked, and the build wrote that report.

The oracle gives the sheet something to be measured against:

- **the original's numbers** come from its own code, in Deep mode: the program is decompiled and
  the numeric constants in its functions are read;
- **the build's numbers** come from its `--self-test` report, where each step says what it
  measured ("walked 3.1 m forward").

The self-test is still read, and still matters: a step the build says failed is a real failure
(a build that says it cannot move has not been rebuilt). What it can no longer do is pass. With a
comparison recorded, its passing steps are corroboration; they are reported, and they do not hold
the check up.

#### The two sides do not speak the same language

The first version of this compared the original's `{function: [constants]}` with the build's
`{step: [numbers]}` as if the keys were the same. They are not: the original's keys are
function names (`sub_1000`) and the build's are step names (`move`). **Every function was
reported as missing from the build**, so every game would have failed.

The fix is the **mechanics sheet**: it names the values ("walk speed", "rifle damage"), and it
is the one vocabulary both sides can speak. So:

- the sheet gives the values their names and their numbers — the vocabulary;
- the original's constants are what its own code says, kept beside them, **not** the comparison;
- the build's report answers in the sheet's names, which is why the self-test contract now says
  to name a step after the value it measures.

A whole number is written without its fraction on both sides (`7`, not `7.0`), so a sheet value
and a measured detail compare as the same number.

A build that measures the sheet's numbers is alike; one that measures another number is caught,
and the difference names the value:

> at walk speed the original read "4.5" but the build read "9.9"

In Clean room there is no code to read, so the original's numbers are not established and the
sheet stays unverified — as it says it is.

Two flaws were found and fixed while building it:

- the constant reader used a look-behind, which the `regex` crate does not have, so it found
  nothing;
- it read every constant with `f64::parse`, which does not accept hex, so `0x68` was silently
  dropped while `4.5` was kept.

## Verification

Each step needs a test that fails before it and passes after:

- Two runs of the same program compare equal, and the comparison says why when they don't.
- A program that differs in one observable way is reported as differing in that way and no other.
- A probe that could not be read produces an unknown, never a pass.
- A build that lies in its self-test but behaves differently is caught by the oracle.
- The original's observation is recorded once and reused; a second recording of the same original
  is not needed between rounds.
- A script recorded against one program is refused against a program of a different kind.
