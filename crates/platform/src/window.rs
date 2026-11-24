//! Pictures of a running program's window, for studying how an app or game looks while it runs
//! (and comparing a rebuild with it). Uses the operating system's own capture through `xcap` on
//! Windows and macOS, and the X display on Linux; elsewhere it says it isn't available yet.

use craft_anything_evidence::{ErrorCode, ToolError};

#[cfg(target_os = "linux")]
mod x11;

/// Whether this system can picture a window: Windows, macOS and Linux with an X display (ROADMAP
/// R2.4). On Linux [`check_can_see_windows`] says whether there is one.
pub const SUPPORTED: bool = cfg!(any(windows, target_os = "macos", target_os = "linux"));

/// Why it can't, where it can't.
pub fn unsupported() -> ToolError {
    ToolError::new(ErrorCode::FeatureMissing, "Picturing a program's window works on Windows, macOS and Linux, not on this system yet.")
        .with_remediation("Add pictures of the original yourself (screenshots) instead.")
}

/// A window's picture: 8-bit RGBA, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowShot {
    pub title: String,
    pub app: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Whether this app may see other programs' windows. Without Screen Recording, macOS hides window
/// names and the capture library skips those windows, so a program that opened one looks as if it
/// hadn't. On Linux it needs an X display. Ask before starting a program: nothing can be pictured,
/// and starting it can have effects.
pub fn check_can_see_windows() -> Result<(), ToolError> {
    #[cfg(target_os = "macos")]
    return permission_result(objc2_core_graphics::CGPreflightScreenCaptureAccess());
    #[cfg(target_os = "linux")]
    return x11::connect().map(drop);
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    Ok(())
}

#[cfg(target_os = "macos")]
fn permission_result(allowed: bool) -> Result<(), ToolError> {
    if allowed {
        return Ok(());
    }
    Err(ToolError::new(ErrorCode::NoPermission, "The program wasn't started: this app can't see windows without Screen Recording permission.")
        .with_remediation(
            "Allow the app running Craft Anything in System Settings → Privacy & Security → Screen Recording, restart that app, then run this again.",
        ))
}

/// What choosing a window needs to know about it.
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
#[derive(Debug, Clone, Default)]
struct Candidate {
    pid: Option<u32>,
    /// The program's name as the system shows it.
    app: String,
    width: u32,
    height: u32,
    /// On screen: not minimized or unmapped.
    shown: bool,
}

/// The largest shown window that belongs to `pid`, or (when none does, as with a launcher that
/// starts the real program) to a program whose name matches `app_name`.
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn choose(windows: &[Candidate], pid: u32, app_name: Option<&str>) -> Option<usize> {
    let usable = || windows.iter().enumerate().filter(|(_, w)| w.shown && w.width > 32 && w.height > 32);
    let area = |(_, w): &(usize, &Candidate)| u64::from(w.width) * u64::from(w.height);
    let by_pid = usable().filter(|(_, w)| w.pid == Some(pid)).max_by_key(area);
    by_pid
        .or_else(|| {
            let want = app_name?.to_ascii_lowercase();
            usable().filter(|(_, w)| is_named(&w.app, &want)).max_by_key(area)
        })
        .map(|(i, _)| i)
}

/// No window to picture yet: the caller may look again or try the program's descendants.
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn no_window() -> ToolError {
    ToolError::new(ErrorCode::NotFound, "The program didn't show a window to picture.")
        .with_remediation("Give it longer to start (wait_secs), or check that it opens a window.")
}

/// Whether a window's owner is the program called `want` (lowercase): the name on its own or as
/// whole words, so `win` never finds "Window Server". The system's own windows are never the
/// program's, whatever it's called.
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn is_named(owner: &str, want: &str) -> bool {
    let owner = owner.to_ascii_lowercase();
    if want.is_empty() || owner == "window server" {
        return false;
    }
    owner.match_indices(want).any(|(at, _)| {
        let before = owner[..at].chars().next_back();
        let after = owner[at + want.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// Picture the largest visible window that belongs to `pid`, or (when none does, as with a
/// launcher that starts the real program) to a process whose name matches `app_name`.
#[cfg(any(windows, target_os = "macos"))]
pub fn capture_window(pid: u32, app_name: Option<&str>) -> Result<WindowShot, ToolError> {
    let windows = xcap::Window::all().map_err(|e| ToolError::new(ErrorCode::FeatureMissing, format!("Can't list windows on this computer: {e}")))?;
    let candidates: Vec<Candidate> = windows
        .iter()
        .map(|w| Candidate {
            pid: w.pid().ok(),
            app: w.app_name().unwrap_or_default(),
            width: w.width().unwrap_or(0),
            height: w.height().unwrap_or(0),
            shown: !w.is_minimized().unwrap_or(true),
        })
        .collect();
    let w = choose(&candidates, pid, app_name).and_then(|i| windows.get(i)).ok_or_else(no_window)?;
    let img = w.capture_image().map_err(|e| ToolError::new(ErrorCode::ProgramFailed, format!("Couldn't picture the window: {e}")))?;
    Ok(WindowShot {
        title: w.title().unwrap_or_default(),
        app: w.app_name().unwrap_or_default(),
        width: img.width(),
        height: img.height(),
        rgba: img.into_raw(),
    })
}

/// Linux: a window on the X display (an X11 session, or XWayland under Wayland).
#[cfg(target_os = "linux")]
pub fn capture_window(pid: u32, app_name: Option<&str>) -> Result<WindowShot, ToolError> {
    x11::capture(pid, app_name)
}

/// The BSDs and others: not built in yet (ROADMAP R2.4).
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub fn capture_window(_pid: u32, _app_name: Option<&str>) -> Result<WindowShot, ToolError> {
    Err(unsupported())
}

#[cfg(all(test, any(windows, target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;

    #[test]
    fn a_name_matches_whole_words_and_never_the_system() {
        assert!(is_named("Game", "game"));
        assert!(is_named("Game Launcher", "game"));
        assert!(is_named("game.exe", "game"));
        assert!(is_named("My Game Client", "my game"));
        assert!(!is_named("Window Server", "win"), "the menu bar was pictured as a program called win");
        assert!(!is_named("Window Server", "window"));
        assert!(!is_named("Trunk", "run"));
        assert!(!is_named("Game", ""));
    }

    fn window(pid: u32, app: &str, width: u32, height: u32) -> Candidate {
        Candidate { pid: Some(pid), app: app.into(), width, height, shown: true }
    }

    #[test]
    fn the_programs_largest_shown_window_is_chosen_before_any_by_name() {
        let windows = [
            window(7, "Game", 4000, 3000),
            Candidate { shown: false, ..window(42, "Game", 2000, 1500) },
            window(42, "Game", 20, 20),
            window(42, "Game", 640, 480),
            window(42, "Game", 800, 600),
        ];
        assert_eq!(choose(&windows, 42, Some("game")), Some(4), "its own, shown, big enough and the largest");
        let launched = [window(7, "Launcher", 900, 700), window(8, "Game", 640, 480), window(9, "Gamepad Tool", 1000, 800)];
        assert_eq!(choose(&launched, 42, Some("game")), Some(1), "a launcher's program is found by name, by whole words");
        assert_eq!(choose(&launched, 42, None), None);
        assert_eq!(choose(&[Candidate { pid: None, ..window(0, "Window Server", 3000, 2000) }], 42, Some("win")), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_missing_permission_says_where_to_allow_it() {
        assert!(permission_result(true).is_ok());
        let e = permission_result(false).unwrap_err();
        assert_eq!(e.code, ErrorCode::NoPermission);
        assert!(e.message.contains("wasn't started"));
        assert!(e.remediation.as_deref().is_some_and(|r| r.contains("Screen Recording")), "{e:?}");
    }
}
