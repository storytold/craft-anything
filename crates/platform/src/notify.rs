//! System notifications, best effort: a toast on Windows (through PowerShell), Notification Centre
//! on macOS (`osascript`), and the desktop's notification service on Linux (`notify-send`, when
//! it's installed). A build that runs while Craft Anything is closed uses it to say it's done or
//! needs the person (#1970).
//!
//! What a notification says may come from untrusted input (a project's name, a build's error), so
//! it's always handed to the program as data, never as code: on Windows the script is fixed and
//! the text arrives in environment variables it reads; on macOS the AppleScript is fixed and the
//! text arrives as its `argv`; on Linux the text is `notify-send`'s arguments after `--`.
//!
//! Each program runs through [`crate::process`] with stdin closed or fed, a short deadline and
//! capped output. A missing program, a failure or a time-out is logged at debug level and
//! otherwise ignored: a notification never fails or holds up what sends it.

use crate::process::Process;
use craft_anything_evidence::ToolError;
use std::time::Duration;

/// How long a notification's program may take (PowerShell's start is the slow one).
pub const DEADLINE: Duration = Duration::from_secs(8);
/// The longest title and body shown; longer text is cut at a character boundary with "…".
pub const MOST_TITLE: usize = 64;
pub const MOST_BODY: usize = 240;

/// The environment variables the Windows script reads its text from.
pub const TITLE_VAR: &str = "CRAFT_ANYTHING_NOTIFY_TITLE";
pub const BODY_VAR: &str = "CRAFT_ANYTHING_NOTIFY_BODY";

/// Windows shows a toast only for an app it knows; PowerShell's own id is registered on every
/// Windows 10 and 11, so the toast says it came from Windows PowerShell.
const WINDOWS_APP_ID: &str = r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe";

/// The program a notification runs, as data, so each system's form can be tested on any system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub program: &'static str,
    pub args: Vec<String>,
    pub env: Vec<(&'static str, String)>,
    /// A script written to the program's stdin (then closed); `None` closes stdin at once.
    pub stdin: Option<String>,
}

impl Command {
    fn process(&self) -> Process {
        let mut p = Process::new(self.program).args(&self.args).timeout(DEADLINE).stdout_cap(16 * 1024).stderr_cap(16 * 1024).label("a notification");
        for (k, v) in &self.env {
            p = p.env(k, v);
        }
        if let Some(script) = &self.stdin {
            p = p.stdin_bytes(script.as_bytes().to_vec());
        }
        p
    }
}

/// Text fit to show: no control characters but line breaks (a NUL can't be passed to a program at
/// all), and no longer than `most` characters.
pub fn clean(text: &str, most: usize) -> String {
    let flat: String = text.chars().map(|c| if c == '\n' || !c.is_control() { c } else { ' ' }).collect();
    let flat = flat.trim();
    if flat.chars().count() <= most {
        return flat.to_string();
    }
    let mut cut: String = flat.chars().take(most.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// The Windows form: a fixed PowerShell script on stdin that reads its text from [`TITLE_VAR`]
/// and [`BODY_VAR`], and puts it in the toast as text nodes (never as markup).
pub fn windows(title: &str, body: &str) -> Command {
    // One line: PowerShell reads a script on stdin line by line, and a `try` spread over lines
    // needs a blank line to end it.
    let script = [
        "$ErrorActionPreference = 'Stop'",
        "try {",
        "$null = [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime];",
        "$null = [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime];",
        "$xml = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02);",
        "$text = $xml.GetElementsByTagName('text');",
        &format!("$null = $text.Item(0).AppendChild($xml.CreateTextNode($env:{TITLE_VAR}));"),
        &format!("$null = $text.Item(1).AppendChild($xml.CreateTextNode($env:{BODY_VAR}));"),
        "$toast = [Windows.UI.Notifications.ToastNotification]::new($xml);",
        &format!("[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{WINDOWS_APP_ID}').Show($toast)"),
        "} catch { [Console]::Error.WriteLine($_); exit 1 }",
    ];
    let script = format!("{}; {}\n", script[0], script[1..].join(" "));
    Command {
        program: "powershell",
        args: ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", "-"].map(String::from).to_vec(),
        env: vec![(TITLE_VAR, clean(title, MOST_TITLE)), (BODY_VAR, clean(body, MOST_BODY))],
        stdin: Some(script),
    }
}

/// The macOS form: a fixed AppleScript that shows the last two of its arguments. (Read from the
/// end, so whether `osascript` passes the `--` on or not, they're the title and the body.)
pub fn macos(title: &str, body: &str) -> Command {
    Command {
        program: "osascript",
        args: vec![
            "-e".into(),
            "on run argv".into(),
            "-e".into(),
            "display notification (item -1 of argv) with title (item -2 of argv)".into(),
            "-e".into(),
            "end run".into(),
            "--".into(),
            clean(title, MOST_TITLE),
            clean(body, MOST_BODY),
        ],
        env: Vec::new(),
        stdin: None,
    }
}

/// The Linux form: `notify-send`, with the text after `--` so none of it is read as an option.
/// Notification services may read the body as simple markup, so `&`, `<` and `>` are escaped.
pub fn linux(title: &str, body: &str) -> Command {
    let escaped = clean(body, MOST_BODY).replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    Command {
        program: "notify-send",
        args: vec!["--app-name=Craft Anything".into(), "--".into(), clean(title, MOST_TITLE), escaped],
        env: Vec::new(),
        stdin: None,
    }
}

/// This system's form, or `None` where there's no way to notify.
pub fn for_this_system(title: &str, body: &str) -> Option<Command> {
    if cfg!(windows) {
        Some(windows(title, body))
    } else if cfg!(target_os = "macos") {
        Some(macos(title, body))
    } else if cfg!(unix) && !crate::in_flatpak() {
        Some(linux(title, body))
    } else {
        None
    }
}

/// Show a notification and say whether its program reported it shown. Blocks for up to
/// [`DEADLINE`]; most callers want [`notify`].
pub fn try_notify(title: &str, body: &str) -> Result<(), ToolError> {
    let Some(command) = for_this_system(title, body) else {
        return Err(ToolError::new(craft_anything_evidence::ErrorCode::NotSupported, "This system has no way to show a notification."));
    };
    let out = command.process().run()?;
    if out.success { Ok(()) } else { Err(crate::process::nonzero_error("a notification", &out)) }
}

/// Show a notification, best effort: whatever goes wrong is logged at debug level only.
pub fn notify(title: &str, body: &str) {
    if let Err(e) = try_notify(title, body) {
        log::debug!("no notification ({}): {}", body, e.message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOSTILE: &[&str] = &[
        "Notes\"; calc.exe",
        "it's $(whoami) and `id` and ${env:PATH}",
        "line one\nline two\r\n\"'; Start-Process calc; '",
        "</text><text>injected</text>",
        "-e do shell script \"touch /tmp/x\"",
        "a & b < c > d",
        "nul\0and\u{7}bell",
    ];

    #[test]
    fn text_is_cleaned_and_cut() {
        assert_eq!(clean("  a\tb\0c\nd  ", 100), "a b c\nd");
        assert_eq!(clean("abcdef", 4), "abc…");
        assert_eq!(clean("ééééé", 3).chars().count(), 3, "cut at a character, not a byte");
        assert_eq!(clean("", 4), "");
    }

    #[test]
    fn windows_keeps_the_text_out_of_the_script() {
        let base = windows("Craft Anything", "x");
        for text in HOSTILE {
            let c = windows(text, text);
            assert_eq!(c.stdin, base.stdin, "the script is the same whatever it says: {text:?}");
            assert_eq!(c.args, base.args, "and so are the arguments");
            assert_eq!(c.args.last().map(String::as_str), Some("-"), "the script is read from stdin");
            let shown: Vec<&str> = c.env.iter().map(|(_, v)| v.as_str()).collect();
            assert_eq!(shown, [clean(text, MOST_TITLE), clean(text, MOST_BODY)], "the text is only data: {text:?}");
            assert!(c.env.iter().all(|(_, v)| !v.contains('\0')), "nothing that can't be passed");
        }
        let script = base.stdin.unwrap_or_default();
        assert!(script.contains(&format!("CreateTextNode($env:{TITLE_VAR})")) && script.contains(&format!("CreateTextNode($env:{BODY_VAR})")), "{script}");
        assert_eq!(script.lines().count(), 1, "one line, so PowerShell runs it whole: {script}");
    }

    #[test]
    fn macos_passes_the_text_as_arguments_to_a_fixed_script() {
        let base = macos("Craft Anything", "x");
        for text in HOSTILE {
            let c = macos("Craft Anything", text);
            let (script, rest) = c.args.split_at(7);
            assert_eq!(script, &base.args[..7], "the script is the same whatever it says: {text:?}");
            assert_eq!(script.last().map(String::as_str), Some("--"));
            assert_eq!(rest, ["Craft Anything".to_string(), clean(text, MOST_BODY)], "{text:?}");
            assert_eq!(c.stdin, None);
        }
        assert!(base.args.iter().any(|a| a == "on run argv"));
    }

    #[test]
    fn linux_passes_the_text_after_the_options() {
        for text in HOSTILE {
            let c = linux(text, text);
            let end = c.args.iter().position(|a| a == "--").unwrap_or(usize::MAX);
            assert_eq!(end, 1, "the text comes after `--`: {:?}", c.args);
            assert_eq!(c.args.len(), 4);
            assert_eq!(c.args.get(2), Some(&clean(text, MOST_TITLE)));
            let body = c.args.get(3).cloned().unwrap_or_default();
            assert!(!body.contains('<') && !body.contains('>'), "no markup: {body}");
        }
        assert_eq!(linux("t", "a & b").args.get(3).map(String::as_str), Some("a &amp; b"));
    }

    /// Shows one real notification on this computer. Run by hand:
    /// `cargo test -p craft-anything-platform notify::tests::shows_a_real_notification -- --ignored --nocapture`.
    #[test]
    #[ignore = "shows a real notification"]
    fn shows_a_real_notification() {
        let result = try_notify("Craft Anything", "Notes \"$(x)\" `y`: the build is ready.");
        println!("notification: {result:?}");
        assert!(result.is_ok(), "{result:?}");
    }
}
