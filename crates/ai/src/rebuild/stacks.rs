//! The stacks a build can use, from the `stacks.toml` table: what the builder is told for each,
//! and the phrases that ask for one by name. Kinds name their default stack; the table says how
//! to build with it. The table also lists libraries asked for on top of a stack (shadcn/ui,
//! Tailwind CSS), whose steps are added to the stack's.

use std::sync::OnceLock;

/// One stack ("React + TypeScript (Vite)", "Unity"…).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Stack {
    /// Shown to the person and stored in the project.
    pub name: String,
    /// The game engine it builds with (`unity`, `unreal`, `godot`, `rust`), if any.
    #[serde(default)]
    pub engine: Option<String>,
    /// Phrases in a request that ask for this stack (`" in python"`), matched on the lower-case
    /// request padded with spaces.
    #[serde(default)]
    pub asked_by: Vec<String>,
    /// What the builder is told; `{editor}`, `{player}` and `{ext}` are filled in. `None` gets
    /// the generic instruction.
    #[serde(default)]
    pub steps: Option<String>,
    /// package.json dependencies that show a build uses this stack (`next`, `vite`); in a
    /// person's own project, Cargo.toml's too (`eframe`).
    #[serde(default)]
    pub uses: Vec<String>,
    /// What a person's own project must depend on as well to count as this stack (`react`).
    #[serde(default)]
    pub needs: Vec<String>,
    /// Files, relative to a folder, that show a person's own project is built with this stack
    /// (`project.godot`); a leading `*` matches by the name's ending (`*.uproject`).
    #[serde(default)]
    pub found_by: Vec<String>,
    /// Its build is a website: a project built with it and nothing to recreate is one.
    #[serde(default)]
    pub site: bool,
    /// The program its build can't run without (`cargo`), and how to get it.
    #[serde(default)]
    pub program: Option<String>,
    #[serde(default)]
    pub program_from: Option<String>,
    /// The toolchains its build can be checked with (`Rust`; `Maven`, `Gradle`), by name in
    /// toolchains.toml, the first whose files are there.
    #[serde(default)]
    pub toolchains: Vec<String>,
}

pub use craft_anything_engine::toolchains::{Toolchain, found_in as toolchain_in};

/// The stack that leaves the choice to the builder (a website's default); [`used_in`] then
/// records the one it used.
pub const BUILDERS_CHOICE: &str = "Builder's choice";

/// A component or styling library asked for on top of a stack ("shadcn/ui", "Tailwind CSS").
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Library {
    pub name: String,
    /// Phrases in a request that ask for it, matched like [`Stack::asked_by`].
    #[serde(default)]
    pub asked_by: Vec<String>,
    /// The stacks it works with, by name; empty = any.
    #[serde(default)]
    pub stacks: Vec<String>,
    /// What the builder is told, after the stack's steps.
    pub steps: String,
}

#[derive(Debug, Default, serde::Deserialize)]
struct Table {
    #[serde(default)]
    stack: Vec<Stack>,
    #[serde(default)]
    library: Vec<Library>,
}

/// The built-in table. A table that doesn't parse (a broken edit) leaves it empty, so every
/// stack gets the generic instruction instead of the app failing; a test keeps it parsing.
fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| parse(include_str!("stacks.toml")).unwrap_or_default())
}

pub fn catalogue() -> &'static [Stack] {
    &table().stack
}

fn parse(text: &str) -> Result<Table, toml::de::Error> {
    toml::from_str::<Table>(text)
}

/// A request as `asked_by` phrases are matched: lower-case words, punctuation turned into
/// spaces, padded with spaces (`"Use Next.js, shadcn/ui"` → `" use next js shadcn ui "`). "C#"
/// reads "csharp", since without its `#` it would be C, and ".NET" or "ASP.NET" as a word of its
/// own (in quotes, brackets or backticks too) reads "dotnet", since without its dot it would be
/// "net revenue"'s net. An address (`https://asp.net`, `www.…`) is read as it is.
pub fn words(request: &str) -> String {
    let lower = request.to_lowercase();
    let read: Vec<String> = lower
        .split_whitespace()
        .map(|raw| {
            if raw.contains("://") || raw.contains("www.") {
                return raw.to_string();
            }
            // ".NET/WinForms" is two words.
            let parts = raw.split('/').map(|part| {
                let word = part.trim_matches(|c: char| !c.is_alphanumeric() && !matches!(c, '.' | '#')).trim_end_matches('.');
                match word {
                    ".net" => "dotnet".into(),
                    "asp.net" => "asp dotnet".into(),
                    _ => part.replace("c#", "csharp"),
                }
            });
            parts.collect::<Vec<String>>().join(" ")
        })
        .collect();
    let read = read.join(" ");
    format!(" {} ", read.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" "))
}

pub fn get(name: &str) -> Option<&'static Stack> {
    catalogue().iter().find(|s| s.name == name)
}

/// The stack's own program (`cargo`) when `have` says this computer doesn't have it, and how to get it.
pub fn missing_program(stack: &str, have: impl Fn(&str) -> bool) -> Option<(&'static str, &'static str)> {
    let s = get(stack)?;
    let program = s.program.as_deref().filter(|p| !have(p))?;
    Some((program, s.program_from.as_deref().unwrap_or_default()))
}

/// A stack picked by name: its full name or its engine id, ignoring case (`unity`, `Godot`,
/// `React + TypeScript (Vite)`).
pub fn resolve(name: &str) -> Option<&'static Stack> {
    let n = name.trim();
    catalogue().iter().find(|s| s.name.eq_ignore_ascii_case(n) || s.engine.as_deref().is_some_and(|e| e.eq_ignore_ascii_case(n)))
}

/// The first stack whose phrases appear in `words` (a request through [`words`]).
pub fn asked_for(words: &str) -> Option<&'static Stack> {
    // A port names its source too ("from csharp to java"): its destination's phrase goes first.
    let find = |ports: bool| catalogue().iter().find(|s| s.asked_by.iter().any(|p| (!ports || p.contains(" to ")) && words.contains(p.as_str())));
    find(true).or_else(|| find(false))
}

/// The stack a build in `app` uses, from its package.json dependencies (the first stack in table
/// order whose `uses` it lists); None when it has none or names no stack in the table.
pub fn used_in(app: &std::path::Path) -> Option<&'static Stack> {
    let deps = craft_anything_engine::outputs::npm_dependencies(app);
    catalogue().iter().find(|s| s.uses.iter().any(|d| deps.contains(d)))
}

/// The toolchains `stack` can be written with whose files are in `app` (a manifest, or a launch
/// file such as `main.py` for a script with none), in the table's order.
fn own_toolchains(stack: &str, app: &std::path::Path) -> Vec<&'static Toolchain> {
    let tools = craft_anything_engine::toolchains::catalogue();
    let here = |t: &&Toolchain| t.is_in(app) || t.has_entrypoint(app);
    let names = get(stack).map(|s| s.toolchains.as_slice()).unwrap_or_default();
    names.iter().filter_map(|name| tools.iter().find(|t| t.name == *name).filter(here)).collect()
}

/// The toolchain `stack` is written in: of those there, `chosen` (the one a move was confirmed
/// with), else the one whose project file was written last (a Java port that added
/// build.gradle.kts beside an old pom.xml is Gradle), else the table's first.
fn own_toolchain(stack: &str, chosen: Option<&str>, app: &std::path::Path) -> Option<&'static Toolchain> {
    let found = own_toolchains(stack, app);
    if let Some(t) = found.iter().find(|t| Some(t.name.as_str()) == chosen) {
        return Some(t);
    }
    let mut best = *found.first()?;
    let mut newest = best.newest_marker(app);
    for &t in &found[1..] {
        let time = t.newest_marker(app);
        if time > newest {
            (best, newest) = (t, time);
        }
    }
    Some(best)
}

/// Whether `stack` is a language other than Node.js whose files are still in `app`: a
/// package.json left from before a move then says nothing about what the build uses.
pub fn holds_its_language(stack: &str, app: &std::path::Path) -> bool {
    own_toolchain(stack, None, app).is_some_and(|t| t.name != "Node.js")
}

/// The toolchain `app` is built with: the recorded stack's (`chosen` first, the one its move was
/// confirmed with) when its files are there, since a project moved to another language can still
/// hold the old one's manifest (#1837); else the first found.
pub fn toolchain_for(stack: &str, chosen: Option<&str>, app: &std::path::Path) -> Option<&'static Toolchain> {
    own_toolchain(stack, chosen, app).or_else(|| toolchain_in(app))
}

/// Each language's files in `app` ([`Toolchain::marker_files`]) with their size and time, to tell
/// what a builder's turn wrote from what was there before it; None for a language whose folder
/// was too big to read whole.
pub type LanguageFiles = Vec<(&'static str, Option<Vec<(std::path::PathBuf, u64, Option<std::time::SystemTime>)>>)>;

pub fn language_files(app: &std::path::Path) -> LanguageFiles {
    let stat = |p: std::path::PathBuf| {
        let meta = std::fs::metadata(&p).ok();
        let (len, time) = (meta.as_ref().map_or(0, std::fs::Metadata::len), meta.and_then(|m| m.modified().ok()));
        (p, len, time)
    };
    craft_anything_engine::toolchains::catalogue().iter().map(|t| (t.name.as_str(), t.marker_files(app).map(|f| f.into_iter().map(stat).collect()))).collect()
}

/// `asked`, the stack a follow-up request names, when the build moved to it: the turn wrote one of
/// its language's files in `app` (new or changed since `before`, its manifest or its code; a
/// deleted file or an old `main.py` left as it was isn't a move), with its own dependencies when
/// it has some (a package.json for Next.js isn't Electron), and it is written in another language
/// than `stack`. Within one language (a Vite site that mentions Electron) the recorded stack
/// stays. A folder too big to read whole can't show a move. Returns the stack and the toolchain
/// the turn wrote, to check the build with.
pub fn moved_to(stack: &str, asked: &'static str, app: &std::path::Path, before: &LanguageFiles) -> Option<(&'static str, &'static str)> {
    let now = language_files(app);
    let files = |all: &LanguageFiles, name: &str| all.iter().find(|(n, _)| *n == name).map_or(Some(Vec::new()), |(_, f)| f.clone());
    let wrote = |t: &&Toolchain| match (files(&now, &t.name), files(before, &t.name)) {
        (Some(after), Some(was)) => after.iter().any(|f| !was.contains(f)),
        _ => false,
    };
    // Of its toolchains, the one the turn wrote (Gradle's file, not an untouched pom.xml).
    let new = own_toolchains(asked, app).into_iter().find(wrote)?;
    let s = get(asked)?;
    if !s.uses.is_empty() {
        // Only the new language's own file counts: a Cargo key named `phaser` isn't npm's Phaser.
        let deps = match new.name.as_str() {
            "Node.js" => craft_anything_engine::outputs::npm_dependencies(app),
            "Rust" => craft_anything_engine::outputs::cargo_dependencies(app),
            _ => Default::default(),
        };
        if !s.uses.iter().any(|d| deps.contains(d)) || !s.needs.iter().all(|d| deps.contains(d)) {
            return None;
        }
    }
    (!get(stack).is_some_and(|s| s.toolchains.contains(&new.name))).then_some((asked, new.name.as_str()))
}

/// The stack a folder of the person's own files is built with: an engine's project file says
/// most, then the dependencies, then the other files that name a stack. None when nothing does.
pub fn found_in(dir: &std::path::Path) -> Option<&'static Stack> {
    let by_file = |engine: bool| catalogue().iter().filter(|s| s.engine.is_some() == engine).find(|s| s.found_by.iter().any(|f| has_file(dir, f)));
    let mut deps = craft_anything_engine::outputs::npm_dependencies(dir);
    deps.extend(craft_anything_engine::outputs::cargo_dependencies(dir));
    let by_deps = || catalogue().iter().find(|s| s.uses.iter().any(|d| deps.contains(d)) && s.needs.iter().all(|d| deps.contains(d)));
    by_file(true).or_else(by_deps).or_else(|| by_file(false))
}

fn has_file(dir: &std::path::Path, name: &str) -> bool {
    match name.strip_prefix('*') {
        Some(ending) => std::fs::read_dir(dir).into_iter().flatten().flatten().any(|e| e.file_name().to_string_lossy().ends_with(ending)),
        None => dir.join(name).is_file(),
    }
}

/// The libraries `request` asks for that work with `stack`, in table order.
pub fn libraries_for(request: &str, stack: &str) -> Vec<&'static Library> {
    let w = words(request);
    table()
        .library
        .iter()
        .filter(|l| l.asked_by.iter().any(|p| w.contains(p.as_str())) && (l.stacks.is_empty() || l.stacks.iter().any(|s| s == stack)))
        .collect()
}

/// What the builder is told for `stack`. `editor`: the game engine's editor for engine stacks.
pub fn steps(stack: &str, editor: Option<&std::path::Path>) -> String {
    let Some(text) = get(stack).and_then(|s| s.steps.as_deref()) else {
        return format!("Use {stack}. Create the project in app/ and make sure it builds or runs without errors.");
    };
    let (player, ext) = if cfg!(windows) {
        ("StandaloneWindows64", "exe")
    } else if cfg!(target_os = "macos") {
        ("StandaloneOSX", "app")
    } else {
        ("StandaloneLinux64", "x86_64")
    };
    let editor = editor.map(|e| e.display().to_string()).unwrap_or_default();
    craft_anything_platform::text::fill(text, &[("editor", &editor), ("player", player), ("ext", ext)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_says_what_it_is_built_with() {
        // Each case in a folder of its own.
        let found = |files: &[(&str, &str)]| {
            let d = tempfile::tempdir().unwrap();
            for (name, text) in files {
                let path = d.path().join(name);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text).unwrap();
            }
            found_in(d.path()).map(|s| s.name.as_str())
        };
        let vite = r#"{"dependencies":{"react":"19"},"devDependencies":{"vite":"7"}}"#;
        assert_eq!(found(&[("project.godot", "")]), Some("Godot"));
        assert_eq!(found(&[("ProjectSettings/ProjectVersion.txt", "")]), Some("Unity"));
        assert_eq!(found(&[("Game.uproject", "{}")]), Some("Unreal Engine"));
        assert_eq!(found(&[("project.godot", ""), ("package.json", vite)]), Some("Godot"), "an engine's file says most");
        assert_eq!(found(&[("package.json", vite), ("index.html", "")]), Some("React + TypeScript (Vite)"));
        assert_eq!(
            found(&[("package.json", r#"{"dependencies":{"vue":"3"},"devDependencies":{"vite":"7"}}"#)]),
            Some(BUILDERS_CHOICE),
            "Vite alone isn't React"
        );
        assert_eq!(found(&[("package.json", r#"{"dependencies":{"svelte":"5"}}"#), ("index.html", "")]), Some(BUILDERS_CHOICE));
        assert_eq!(found(&[("Cargo.toml", "[dependencies]\neframe = \"0.30\"\n")]), Some("Rust + egui (eframe)"));
        assert_eq!(found(&[("Cargo.toml", "[dependencies]\nmacroquad = \"0.4\"\n")]), Some("Rust + macroquad"));
        assert_eq!(found(&[("Cargo.toml", "[dependencies]\nmacroquad = \"0.4\"\negui = \"0.30\"\n")]), Some("Rust + macroquad"), "a game with an egui overlay");
        assert_eq!(found(&[("Cargo.toml", "[dependencies]\niced = \"0.13\"\n")]), Some(BUILDERS_CHOICE));
        assert_eq!(found(&[("Cargo.toml", "not [ toml")]), Some(BUILDERS_CHOICE));
        assert_eq!(found(&[("pyproject.toml", "")]), Some("Python"));
        assert_eq!(found(&[("requirements.txt", "")]), Some("Python"));
        assert_eq!(found(&[("go.mod", "")]), Some("Go"));
        assert_eq!(found(&[("App.csproj", "")]), Some("C# (.NET)"));
        assert_eq!(found(&[("Gemfile", "")]), Some(BUILDERS_CHOICE), "a language with no stack");
        assert_eq!(found(&[("index.html", "")]), Some("Plain HTML and CSS"));
        assert_eq!(found(&[("notes.md", "")]), None);
    }

    /// Verify file-based toolchain detection and that every shared row has build/test commands.
    #[test]
    fn a_folder_says_how_it_builds_and_tests_itself() {
        let tool = |file: &str| {
            let d = tempfile::tempdir().unwrap();
            std::fs::write(d.path().join(file), "").unwrap();
            toolchain_in(d.path()).map(|t| t.name.as_str())
        };
        assert_eq!(tool("Cargo.toml"), Some("Rust"));
        assert_eq!(tool("package.json"), Some("Node.js"));
        assert_eq!(tool("requirements.txt"), Some("Python"));
        assert_eq!(tool("go.mod"), Some("Go"));
        assert_eq!(tool("App.csproj"), Some(".NET"));
        assert_eq!(tool("Shop.sln"), Some(".NET"));
        assert_eq!(tool("pom.xml"), Some("Maven"));
        assert_eq!(tool("build.gradle.kts"), Some("Gradle"));
        assert_eq!(tool("Makefile"), Some("Make"));
        assert_eq!(tool("notes.md"), None);
        for t in craft_anything_engine::toolchains::catalogue() {
            assert!(!t.found_by.is_empty() && !t.build.is_empty() && !t.test.is_empty(), "{}", t.name);
        }
    }

    #[test]
    fn the_table_parses_and_covers_every_engine() {
        let names: Vec<&str> = catalogue().iter().map(|s| s.name.as_str()).collect();
        assert!(names.len() >= 7, "{names:?}");
        for b in craft_anything_engines::game::Builder::ALL {
            assert!(get(b.stack()).is_some(), "no stack in stacks.toml for {}", b.stack());
        }
        assert_eq!(get("Unity").and_then(|s| s.engine.as_deref()), Some("unity"));
        let tools = craft_anything_engine::toolchains::catalogue();
        for s in catalogue() {
            assert!(s.toolchains.iter().all(|t| tools.iter().any(|c| c.name == *t)), "{} names a toolchain not in toolchains.toml", s.name);
            assert_eq!(s.program.is_some(), s.toolchains == ["Rust"], "{}: every Rust stack, and only those, needs cargo", s.name);
        }
        assert!(parse("[[stack]]\nname = 3").is_err(), "a broken table is an error, not a panic");
        for l in &table().library {
            assert!(!l.steps.is_empty() && !l.asked_by.is_empty(), "{l:?}");
            assert!(l.stacks.iter().all(|s| get(s).is_some()), "{} names a stack that isn't in the table", l.name);
        }
    }

    #[test]
    fn a_build_left_to_the_builder_is_recorded_under_the_stack_it_used() {
        let d = tempfile::tempdir().unwrap();
        assert!(
            get(BUILDERS_CHOICE)
                .is_some_and(|s| s.steps.as_deref().is_some_and(|t| t.starts_with("Use what their request names; otherwise choose the stack yourself")))
        );
        assert!(used_in(d.path()).is_none(), "no package.json");
        std::fs::write(d.path().join("Cargo.toml"), "[dependencies]\neframe = \"0.30\"\n").unwrap();
        assert!(used_in(d.path()).is_none(), "Cargo.toml never replaces a stack the request named");
        std::fs::write(d.path().join("package.json"), "not json").unwrap();
        assert!(used_in(d.path()).is_none(), "a broken package.json is no stack, not a crash");
        std::fs::write(d.path().join("package.json"), r#"{"dependencies": {"next": "16", "react": "19"}, "devDependencies": {"vite": "7"}}"#).unwrap();
        assert_eq!(used_in(d.path()).map(|s| s.name.as_str()), Some("Next.js + TypeScript"), "table order decides");
        std::fs::write(d.path().join("package.json"), r#"{"devDependencies": {"vite": "7"}}"#).unwrap();
        assert_eq!(used_in(d.path()).map(|s| s.name.as_str()), Some("React + TypeScript (Vite)"));
        std::fs::write(d.path().join("package.json"), r#"{"dependencies": {"astro": "5"}}"#).unwrap();
        assert!(used_in(d.path()).is_none(), "a stack the table doesn't know stays the builder's choice");
        assert_eq!(libraries_for("use shadcn", BUILDERS_CHOICE).len(), 1, "a library works with the builder's choice too");
    }

    /// #1837: a Rust project rewritten in Python still holds its Cargo.toml until it's moved out.
    /// The build is checked in the language of the stack recorded, which a follow-up asking for
    /// another language moves once the new one's files are there.
    #[test]
    fn a_build_moved_to_another_language_is_checked_in_the_new_one() {
        let d = tempfile::tempdir().unwrap();
        let tool = |stack: &str| toolchain_for(stack, None, d.path()).map(|t| t.name.as_str());
        std::fs::write(d.path().join("Cargo.toml"), "[package]\nname = \"game\"\n").unwrap();
        assert_eq!(moved_to("Rust + macroquad", "Python", d.path(), &Vec::new()), None, "no Python files yet: the build hasn't moved");
        std::fs::write(d.path().join("pyproject.toml"), "[project]\nname = \"game\"\n").unwrap();
        assert_eq!(tool("Rust + macroquad"), Some("Rust"));
        assert_eq!(tool("Python"), Some("Python"), "not the Cargo.toml left behind");
        assert_eq!(tool("Unity"), Some("Rust"), "a stack with no toolchain takes the first found, as before");
        assert_eq!(moved_to("Rust + macroquad", "Python", d.path(), &Vec::new()).map(|m| m.0), Some("Python"));
        assert_eq!(moved_to("Python", "Python", d.path(), &Vec::new()), None, "already recorded");
        // Electron's own dependency is there, so only the language guard can keep the stack.
        std::fs::write(d.path().join("package.json"), r#"{"devDependencies": {"electron": "39"}}"#).unwrap();
        assert_eq!(moved_to("React + TypeScript (Vite)", "Electron", d.path(), &Vec::new()), None, "one language: the stack stays");
    }

    /// Go, C# and Java have stacks too, so a build moved to one of them is recorded and checked
    /// in it; Java by whichever of Maven and Gradle the builder used.
    #[test]
    fn a_build_moved_to_go_csharp_or_java_is_checked_in_it() {
        for (request, stack, file, tool) in [
            ("now rewrite it in golang", "Go", "go.mod", "Go"),
            ("Rewrite it in Go.", "Go", "go.mod", "Go"),
            ("rewrite it in .NET", "C# (.NET)", "Game.csproj", ".NET"),
            ("make it a C sharp app", "C# (.NET)", "Game.sln", ".NET"),
            ("Rewrite it in C#", "C# (.NET)", "Game.csproj", ".NET"),
            ("redo this in Java", "Java", "pom.xml", "Maven"),
            ("redo this in Java", "Java", "build.gradle.kts", "Gradle"),
        ] {
            let d = tempfile::tempdir().unwrap();
            std::fs::write(d.path().join("Cargo.toml"), "[package]\nname = \"game\"\n").unwrap();
            let asked = asked_for(&words(request)).map(|s| s.name.as_str());
            assert_eq!(asked, Some(stack), "{request}");
            assert_eq!(moved_to("Rust + macroquad", stack, d.path(), &Vec::new()), None, "{stack}: nothing of it yet");
            std::fs::write(d.path().join(file), "").unwrap();
            assert_eq!(moved_to("Rust + macroquad", stack, d.path(), &Vec::new()).map(|m| m.0), Some(stack), "{file}");
            assert_eq!(toolchain_for(stack, None, d.path()).map(|t| t.name.as_str()), Some(tool), "{file}: not the Cargo.toml left behind");
        }
        assert_eq!(asked_for(&words("sign in, go to settings")), None, "the verb \"go\" isn't Go");
        assert_eq!(asked_for(&words("written in javascript")).map(|s| s.name.as_str()), Some("React + TypeScript (Vite)"), "JavaScript isn't Java");
        assert_eq!(asked_for(&words("a tool in C")), None, "C isn't C#");
        for r in ["Rewrite this from Python to Go.", "port it from Rust to Go", "rewrite this in Go"] {
            assert_eq!(asked_for(&words(r)).map(|s| s.name.as_str()), Some("Go"), "{r}");
        }
        assert_eq!(asked_for(&words("we're ready to go")), None);
        for r in ["Add a table with net revenue", "use the money in net terms", "link to example.net"] {
            assert_eq!(asked_for(&words(r)), None, "{r}");
        }
        for r in [
            "rewrite it in .NET",
            "an ASP.NET site",
            "C# (.NET) please",
            ".NET 9",
            "Rewrite it in `.NET`",
            "in \".NET\"",
            "port it to .NET.",
            "Rewrite it in .NET/WinForms",
        ] {
            assert_eq!(asked_for(&words(r)).map(|s| s.name.as_str()), Some("C# (.NET)"), "{r}");
        }
        // Addresses are read as they are, and "<language> to go" is a port only after "from".
        for r in ["Add a link to https://asp.net", "see www.asp.net for more", "Use Python to go through the CSV files"] {
            assert_eq!(asked_for(&words(r)), None, "{r}");
        }
        for r in ["Port this from Rust to Java", "port it to Java", "rewrite this from Python to Java"] {
            assert_eq!(asked_for(&words(r)).map(|s| s.name.as_str()), Some("Java"), "{r}");
        }
        assert_eq!(asked_for(&words("port this to Go")).map(|s| s.name.as_str()), Some("Go"));
    }

    /// A folder of the person's own Go, C# or Java project is that stack, not the builder's choice.
    #[test]
    fn an_own_go_csharp_or_java_folder_is_found_as_that_stack() {
        for (file, stack) in [("go.mod", "Go"), ("Shop.csproj", "C# (.NET)"), ("Shop.sln", "C# (.NET)"), ("pom.xml", "Java"), ("build.gradle.kts", "Java")] {
            let d = tempfile::tempdir().unwrap();
            std::fs::write(d.path().join(file), "").unwrap();
            assert_eq!(found_in(d.path()).map(|s| s.name.as_str()), Some(stack), "{file}");
        }
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("Gemfile"), "").unwrap();
        assert_eq!(found_in(d.path()).map(|s| s.name.as_str()), Some(BUILDERS_CHOICE), "a language with no stack");
    }

    /// Review of #2099: a move shows in files the turn wrote, not in ones it left as they were.
    #[test]
    fn a_move_needs_files_the_turn_wrote() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("go.mod"), "module app\n").unwrap();
        std::fs::write(d.path().join("main.py"), "print(1)\n").unwrap();
        let before = language_files(d.path());
        assert_eq!(moved_to("Go", "Python", d.path(), &before), None, "an old main.py the turn didn't touch");
        std::fs::write(d.path().join("main.py"), "print('rewritten')\n").unwrap();
        assert_eq!(moved_to("Go", "Python", d.path(), &before).map(|m| m.0), Some("Python"), "rewritten");
        let before = language_files(d.path());
        std::fs::write(d.path().join("pyproject.toml"), "[project]\nname = \"app\"\n").unwrap();
        assert_eq!(moved_to("Go", "Python", d.path(), &before).map(|m| m.0), Some("Python"), "a new manifest");
    }

    /// Review of #2099: a port names its source too, and the destination is what's asked for.
    #[test]
    fn a_port_asks_for_its_destination() {
        for (r, stack) in [("Port this from C# to Java", "Java"), ("rewrite this from Java to Go", "Go"), ("a C# app", "C# (.NET)")] {
            assert_eq!(asked_for(&words(r)).map(|s| s.name.as_str()), Some(stack), "{r}");
        }
    }

    /// Review of #2099: a move needs a file the turn wrote, of any of the language's project
    /// files or its code, and the build is checked with the toolchain that changed.
    #[test]
    fn a_move_shows_in_what_the_turn_wrote_not_what_it_deleted() {
        // Deleting an old main.py beside a Python manifest isn't a move to Python.
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("go.mod"), "module app\n").unwrap();
        std::fs::write(d.path().join("pyproject.toml"), "[project]\nname = \"app\"\n").unwrap();
        std::fs::write(d.path().join("main.py"), "print(1)\n").unwrap();
        let before = language_files(d.path());
        std::fs::remove_file(d.path().join("main.py")).unwrap();
        assert_eq!(moved_to("Go", "Python", d.path(), &before), None);

        // A second .csproj the turn changed counts, not only the first one found.
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("Cargo.toml"), "[package]\nname = \"game\"\n").unwrap();
        std::fs::write(d.path().join("Game.csproj"), "<Project/>").unwrap();
        std::fs::write(d.path().join("Tools.csproj"), "<Project/>").unwrap();
        let before = language_files(d.path());
        assert_eq!(moved_to("Rust + macroquad", "C# (.NET)", d.path(), &before), None, "untouched");
        std::fs::write(d.path().join("Tools.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\"/>").unwrap();
        assert_eq!(moved_to("Rust + macroquad", "C# (.NET)", d.path(), &before).map(|m| m.0), Some("C# (.NET)"));

        // A Bevy port that keeps an existing Cargo.toml shows in its code.
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("package.json"), r#"{"devDependencies": {"electron": "39"}}"#).unwrap();
        std::fs::write(d.path().join("Cargo.toml"), "[dependencies]\nbevy = \"0.17\"\n").unwrap();
        std::fs::create_dir(d.path().join("src")).unwrap();
        std::fs::write(d.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        let before = language_files(d.path());
        assert_eq!(moved_to("Electron", "Bevy", d.path(), &before), None, "an untouched Cargo project beside the app");
        std::fs::write(d.path().join("src/main.rs"), "fn main() { /* bevy */ }\n").unwrap();
        assert_eq!(moved_to("Electron", "Bevy", d.path(), &before).map(|m| m.0), Some("Bevy"));

        // A Java port that adds Gradle beside an old pom.xml is checked with Gradle.
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("Cargo.toml"), "[package]\nname = \"game\"\n").unwrap();
        let pom = d.path().join("pom.xml");
        std::fs::write(&pom, "<project/>").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options().write(true).open(&pom).unwrap().set_modified(old).unwrap();
        let before = language_files(d.path());
        std::fs::write(d.path().join("build.gradle.kts"), "plugins { java }\n").unwrap();
        assert_eq!(moved_to("Rust + macroquad", "Java", d.path(), &before), Some(("Java", "Gradle")));
        assert_eq!(toolchain_for("Java", None, d.path()).map(|t| t.name.as_str()), Some("Gradle"), "not the old pom.xml");
        // With equal times only the toolchain the move recorded tells them apart.
        let gradle = d.path().join("build.gradle.kts");
        std::fs::File::options().write(true).open(&gradle).unwrap().set_modified(old).unwrap();
        assert_eq!(toolchain_for("Java", None, d.path()).map(|t| t.name.as_str()), Some("Maven"), "a tie falls back to the table's order");
        assert_eq!(toolchain_for("Java", Some("Gradle"), d.path()).map(|t| t.name.as_str()), Some("Gradle"), "the recorded one wins");
        assert_eq!(toolchain_for("Rust + macroquad", Some("Gradle"), d.path()).map(|t| t.name.as_str()), Some("Rust"), "not another stack's");
    }

    /// Review of #2099: what shows a move, and what mustn't undo one.
    #[test]
    fn what_shows_a_move_and_what_keeps_it() {
        // A Python script with no manifest is Python too.
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("Cargo.toml"), "[package]\nname = \"game\"\n").unwrap();
        std::fs::write(d.path().join("main.py"), "print(1)\n").unwrap();
        assert_eq!(moved_to("Rust + macroquad", "Python", d.path(), &Vec::new()).map(|m| m.0), Some("Python"));
        assert_eq!(toolchain_for("Python", None, d.path()).map(|t| t.name.as_str()), Some("Python"));
        // A package.json for Next.js is no move to Electron; one that lists electron is.
        std::fs::write(d.path().join("package.json"), r#"{"dependencies": {"next": "16"}}"#).unwrap();
        assert_eq!(moved_to("Rust + macroquad", "Electron", d.path(), &Vec::new()), None);
        std::fs::write(d.path().join("package.json"), r#"{"devDependencies": {"electron": "39"}}"#).unwrap();
        assert_eq!(moved_to("Rust + macroquad", "Electron", d.path(), &Vec::new()).map(|m| m.0), Some("Electron"));
        // React needs React as well as Vite: a Vue app built with Vite isn't one.
        std::fs::write(d.path().join("package.json"), r#"{"dependencies": {"vue": "3"}, "devDependencies": {"vite": "7"}}"#).unwrap();
        assert_eq!(moved_to("Rust + macroquad", "React + TypeScript (Vite)", d.path(), &Vec::new()), None);
        std::fs::write(d.path().join("package.json"), r#"{"dependencies": {"react": "19"}, "devDependencies": {"vite": "7"}}"#).unwrap();
        assert_eq!(moved_to("Rust + macroquad", "React + TypeScript (Vite)", d.path(), &Vec::new()).map(|m| m.0), Some("React + TypeScript (Vite)"));
        // Only the new language's own file counts: a Cargo key named phaser isn't npm's Phaser.
        let cargo = tempfile::tempdir().unwrap();
        std::fs::write(cargo.path().join("Cargo.toml"), "[dependencies]\nphaser = \"1\"\n").unwrap();
        std::fs::write(cargo.path().join("package.json"), r#"{"dependencies": {"lodash": "4"}}"#).unwrap();
        assert_eq!(moved_to("Rust + macroquad", "Phaser", cargo.path(), &Vec::new()), None);
        // A renamed crate counts under its package: `engine = { package = "bevy" }` is Bevy.
        let bevy = tempfile::tempdir().unwrap();
        std::fs::write(bevy.path().join("package.json"), r#"{"dependencies": {"electron": "39"}}"#).unwrap();
        std::fs::write(bevy.path().join("Cargo.toml"), "[dependencies]\nengine = { package = \"bevy\", version = \"0.17\" }\n").unwrap();
        assert_eq!(moved_to("Electron", "Bevy", bevy.path(), &Vec::new()).map(|m| m.0), Some("Bevy"));
        // A Python build keeps its stack beside an old Vite package.json; a Vite one doesn't hold.
        std::fs::write(d.path().join("package.json"), r#"{"devDependencies": {"vite": "7"}}"#).unwrap();
        assert!(holds_its_language("Python", d.path()));
        assert!(!holds_its_language("React + TypeScript (Vite)", d.path()), "Node.js is what package.json says");
        assert!(!holds_its_language("Go", d.path()), "no Go files");
        // A project file is found in a folder of more than 256 entries.
        let big = tempfile::tempdir().unwrap();
        std::fs::write(big.path().join("Game.csproj"), "").unwrap();
        for i in 0..300 {
            std::fs::write(big.path().join(format!("asset{i}.txt")), "").unwrap();
        }
        assert_eq!(toolchain_for("C# (.NET)", None, big.path()).map(|t| t.name.as_str()), Some(".NET"));
    }

    #[test]
    fn spelled_variants_of_a_stack_and_its_libraries() {
        let next = Some("Next.js + TypeScript");
        for r in ["use nextjs", "Use Next.js", "built with next js", "in Next.JS please", "(Next.js) with React", "use next and shadcn", "a site in next"] {
            assert_eq!(asked_for(&words(r)).map(|s| s.name.as_str()), next, "{r}");
        }
        assert_eq!(asked_for(&words("what to do next")), None, "\"next\" alone isn't a stack");
        assert_eq!(asked_for(&words("build it with Vite")).map(|s| s.name.as_str()), Some("React + TypeScript (Vite)"));
        let names = |r: &str, stack: &str| libraries_for(r, stack).iter().map(|l| l.name.as_str()).collect::<Vec<_>>();
        for r in ["use shadcn", "with shadcn/ui", "shadcn-ui components", "ShadCN"] {
            assert_eq!(names(r, "Next.js + TypeScript"), ["shadcn/ui"], "{r}");
        }
        assert_eq!(names("shadcn and tailwind", "React + TypeScript (Vite)"), ["shadcn/ui", "Tailwind CSS"]);
        assert_eq!(names("TailwindCSS", "Next.js + TypeScript"), ["Tailwind CSS"]);
        assert!(names("shadcn", "Python").is_empty(), "a library only joins the stacks it works with");
        assert!(names("a plain page", "Next.js + TypeScript").is_empty());
    }

    #[test]
    fn named_game_frameworks_do_not_select_an_editor() {
        for (request, name) in [("Make a game in Phaser with Vite", "Phaser"), ("Make a game using Bevy", "Bevy")] {
            let stack = asked_for(&words(request)).unwrap();
            assert_eq!(stack.name, name);
            assert!(stack.engine.is_none());
            assert_eq!(resolve(name), Some(stack));
        }
        assert!(asked_for(&words("a game with a phaser cannon")).is_none());
    }

    #[test]
    fn named_stacks_and_placeholders() {
        assert_eq!(asked_for(&words("A tool in Python.")).map(|s| s.name.as_str()), Some("Python"));
        assert_eq!(asked_for(&words("a page with react")).map(|s| s.name.as_str()), Some("React + TypeScript (Vite)"));
        assert_eq!(asked_for(&words("a notes app")), None);
        assert_eq!(resolve("godot").map(|s| s.name.as_str()), Some("Godot"));
        assert_eq!(resolve(" unreal ").map(|s| s.name.as_str()), Some("Unreal Engine"));
        assert_eq!(resolve("rust + EGUI (eframe)").map(|s| s.name.as_str()), Some("Rust + egui (eframe)"));
        assert!(resolve("cobol").is_none());
        let unity = steps("Unity", Some(std::path::Path::new("/ed/Unity")));
        assert!(unity.contains("the editor is /ed/Unity") && !unity.contains('{'), "{unity}");
        assert_eq!(steps("Elm", None), "Use Elm. Create the project in app/ and make sure it builds or runs without errors.");
    }

    /// An editor path is inserted as it is, even when a folder in it is named like a later
    /// placeholder (`{player}`, `{ext}`), which chained replacements would fill in turn.
    #[test]
    fn an_editor_path_stays_literal() {
        for path in ["/ed/{ext}/Unity", "/ed/{player}/Unity", "/ed/{player}{ext}/Un{ity"] {
            let unity = steps("Unity", Some(std::path::Path::new(path)));
            assert!(unity.contains(&format!("the editor is {path})")), "{unity}");
            assert!(unity.contains(&format!("`\"{path}\" -batchmode")), "{unity}");
            assert!(!unity.contains("{player} player") && !unity.contains("Game.{ext}"), "{unity}");
        }
    }
}
