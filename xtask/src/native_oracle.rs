//! The native behaviour oracle's live checks.
//!
//! These drive a real window and read it through the accessibility APIs, which macOS only allows
//! for a program the person has given the Accessibility permission to. The desktop app has it; a
//! plain terminal does not, and the checks say so plainly when it is missing.

/// Run the native oracle's live checks, and the accessibility helper's own check with `--all`.
pub fn run(all: bool) -> bool {
    let mut ok = true;
    println!("== the native oracle's live checks ==");
    ok &= test("craft-anything-ai", "oracle_native_live");
    if all {
        println!();
        println!("== the accessibility helper's own check ==");
        ok &= test("craft-anything-engine", "the_helper_puts_text_into_a_field");
    }
    println!();
    println!("The Accessibility permission is given in System Settings → Privacy & Security → Accessibility.");
    println!("Without it the checks report 'this app does not have Accessibility permission'.");
    ok
}

/// One package's ignored test, by name.
fn test(package: &str, name: &str) -> bool {
    let args = ["test", "-p", package, "--lib", name, "--", "--ignored", "--nocapture"];
    crate::run("cargo", &args)
}
