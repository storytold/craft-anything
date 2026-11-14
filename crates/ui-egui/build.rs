//! Optional craft-fonts (fallbacks for CJK, Arabic, Thai, Hebrew, Devanagari…), following
//! craft-fonts' `docs/integration.md`. Releases and CI set it.
//!
//! Unset `CRAFT_FONTS_DIR` (the default) embeds nothing and the app uses its own DM Sans/JetBrains
//! Mono. Set it to an absolute path of a craft-fonts checkout to embed every font its manifest
//! lists. A bad path is a warning unless `CRAFT_FONTS_REQUIRED=1`.
//!
//! With `CRAFT_FONTS_REQUIRED` set (to anything but nothing or `0`) the build fails unless the
//! fonts are embedded: for a bad path, and for no `CRAFT_FONTS_DIR` at all. A build that was asked
//! for the fonts never quietly ships without them.
//!
//! `tests/craft_fonts_build.rs` reads this file as a module to test [`plan`].

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_REQUIRED");
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32");
    let plan = plan(std::env::var_os("CRAFT_FONTS_DIR"), std::env::var_os("CRAFT_FONTS_REQUIRED"), wasm);
    for input in &plan.inputs {
        println!("cargo::rerun-if-changed={}", input.display());
    }
    if let Some(warning) = &plan.warning {
        println!("cargo::warning={warning}");
    }
    // Cargo fails the build on this line; the table is still written so that is the only error.
    if let Some(error) = &plan.error {
        println!("cargo::error={error}");
    }
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default());
    let out = out_dir.join("craft_fonts.rs");
    if let Err(e) = std::fs::write(&out, &plan.source) {
        println!("cargo::error=writing {}: {e}", out.display());
    }
    join_catalogs(&PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default()).join("src/i18n"), &out_dir.join("i18n"));
}

/// Joins each language's catalog files (`src/i18n/<code>/*.tsv`, in name order) into
/// `<out>/<code>.tsv`, which `i18n.rs` embeds. A PR adds its strings in a file of its own, so two
/// PRs that add strings don't touch the same lines (#1465).
pub fn join_catalogs(src: &Path, out: &Path) {
    println!("cargo::rerun-if-changed={}", src.display());
    let dirs = match entries(src) {
        Ok(paths) => paths.into_iter().filter(|p| p.is_dir()),
        Err(e) => return println!("cargo::error=reading {}: {e}", src.display()),
    };
    // Start from nothing, so a language whose folder was removed doesn't keep its old catalog.
    if let Err(e) = std::fs::remove_dir_all(out).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }) {
        return println!("cargo::error=clearing {}: {e}", out.display());
    }
    if let Err(e) = std::fs::create_dir_all(out) {
        return println!("cargo::error=making {}: {e}", out.display());
    }
    for dir in dirs {
        let files = match entries(&dir) {
            Ok(paths) => paths.into_iter().filter(|p| p.extension().is_some_and(|x| x == "tsv")),
            Err(e) => {
                println!("cargo::error=reading {}: {e}", dir.display());
                continue;
            }
        };
        let mut joined = String::new();
        for file in files {
            // The folder's scan sees a new or changed file; a listed file that's gone sees a removal.
            println!("cargo::rerun-if-changed={}", file.display());
            match std::fs::read_to_string(&file) {
                Ok(text) => {
                    joined.push_str(&text);
                    if !text.ends_with('\n') {
                        joined.push('\n');
                    }
                }
                Err(e) => println!("cargo::error=reading {}: {e}", file.display()),
            }
        }
        let Some(code) = dir.file_name() else { continue };
        let target = out.join(code).with_extension("tsv");
        if let Err(e) = std::fs::write(&target, joined) {
            println!("cargo::error=writing {}: {e}", target.display());
        }
    }
}

/// A folder's entries in name order; an entry that can't be read is an error, not a gap.
fn entries(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut paths = std::fs::read_dir(dir)?.map(|e| e.map(|e| e.path())).collect::<std::io::Result<Vec<_>>>()?;
    paths.sort();
    Ok(paths)
}

/// What a build does about craft-fonts, decided from its inputs alone.
#[derive(Debug, Default)]
pub struct Plan {
    /// The `CRAFT_FONTS` table `theme.rs` includes; empty when no fonts are embedded.
    pub source: String,
    /// Files the table was made from: the build runs again when one changes.
    pub inputs: Vec<PathBuf>,
    /// Fails the build: the fonts were required and aren't embedded.
    pub error: Option<String>,
    /// The fonts were asked for, not required, and aren't embedded.
    pub warning: Option<String>,
}

/// `dir` and `required` are `CRAFT_FONTS_DIR` and `CRAFT_FONTS_REQUIRED` as the environment has
/// them (set to nothing counts as unset); `wasm` is whether the build is for the browser.
pub fn plan(dir: Option<OsString>, required: Option<OsString>, wasm: bool) -> Plan {
    let required = required.is_some_and(|v| !v.is_empty() && v != "0");
    let dir = dir.filter(|d| !d.is_empty()).map(PathBuf::from);
    let mut plan = Plan::default();
    let mut entries = String::new();
    match dir {
        None if required => {
            plan.error = Some(
                "CRAFT_FONTS_REQUIRED is set but CRAFT_FONTS_DIR is not, so no craft-fonts would be embedded: set CRAFT_FONTS_DIR to the absolute path of a \
                 craft-fonts checkout (docs/localization.md), or unset CRAFT_FONTS_REQUIRED to build without them"
                    .into(),
            );
        }
        None => {}
        Some(dir) => match craft_fonts(&dir, wasm, &mut plan.inputs) {
            Ok(found) if required && found.is_empty() => {
                plan.error = Some(format!("CRAFT_FONTS_DIR={}: no fonts remain after filtering for this target", dir.display()));
            }
            Ok(found) => entries = found,
            Err(e) if required => plan.error = Some(format!("CRAFT_FONTS_DIR={}: {e}", dir.display())),
            Err(e) => plan.warning = Some(format!("building without craft-fonts: CRAFT_FONTS_DIR={}: {e}", dir.display())),
        },
    }
    plan.source = format!("pub static CRAFT_FONTS: &[CraftFont] = &[\n{entries}];\n");
    plan
}

/// One `CraftFont { .. }` initialiser per manifest line; the files read are added to `inputs`.
fn craft_fonts(dir: &Path, wasm: bool, inputs: &mut Vec<PathBuf>) -> Result<String, String> {
    let manifest = dir.join("fonts/manifest.txt");
    inputs.push(manifest.clone());
    let text = std::fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    const WEB_FONTS: &[(&str, &str)] = &[("BIZ UDPGothic", "Regular")];
    let mut out = String::new();
    let mut listed = 0usize;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let f: Vec<&str> = line.split(" | ").map(str::trim).collect();
        let [family, style, file, scripts, ..] = f.as_slice() else {
            return Err(format!("malformed manifest line: {line}"));
        };
        listed += 1;
        if wasm && !WEB_FONTS.contains(&(*family, *style)) {
            continue;
        }
        let path = dir.join(file).canonicalize().map_err(|e| format!("{file}: {e}"))?;
        inputs.push(path.clone());
        let scripts: Vec<String> = scripts.split(',').map(|s| format!("{:?}", s.trim())).collect();
        let _ = writeln!(
            out,
            "    CraftFont {{ family: {family:?}, style: {style:?}, scripts: &[{}], bytes: include_bytes!({:?}) }},",
            scripts.join(", "),
            path.display().to_string(),
        );
    }
    if listed == 0 {
        return Err(format!("{} lists no fonts", manifest.display()));
    }
    Ok(out)
}
