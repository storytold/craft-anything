//! Splitting what Detect found into the two things a person is choosing between: an API provider
//! (a key, billed per token) and a harness (a coding agent already installed). Without the split,
//! eight more providers push the harnesses off the screen.

use craft_anything_ai::Detected;

/// Which half of the detected list is showing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DetectTab {
    #[default]
    Providers,
    Harnesses,
}

/// The tab a found backend belongs to: an agent program is a harness, everything else answers over
/// HTTP and is a provider (a local model server included).
pub fn tab_of(d: &Detected) -> DetectTab {
    if d.suggestion.kind.is_harness() { DetectTab::Harnesses } else { DetectTab::Providers }
}

/// Whether a detected backend accepts a pasted API key before it is added (#1358).
///
/// Allows pasting a custom key even when a key was already found in the environment (such as
/// `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, or a known hosted provider's variable), so a person can
/// override a stale environment key without adding the provider manually by address. Local servers
/// and coding agent harnesses do not take a pasted key here.
pub(crate) fn can_paste_key(d: &Detected) -> bool {
    d.needs_key
        || d.suggestion.kind.needs_key()
        || d.suggestion.base_url.as_deref().is_some_and(|b| craft_anything_ai::openai::is_opencode_go(b) || craft_anything_ai::providers::of_base(b).is_some())
}

/// How many of each were found, for the tab labels.
pub fn counts(detected: &[Detected]) -> (usize, usize) {
    detected.iter().fold((0, 0), |(p, h), d| match tab_of(d) {
        DetectTab::Providers => (p + 1, h),
        DetectTab::Harnesses => (p, h + 1),
    })
}

/// Which tab to show. A tab the person clicked stays, even when it is empty: that is how its "nothing
/// of this kind was found" line is read. Only the automatic first choice prefers the tab that has
/// something, so adding the only provider never opens on an empty screen.
pub fn resolve(chosen: Option<DetectTab>, providers: usize, harnesses: usize) -> DetectTab {
    match chosen {
        Some(tab) => tab,
        None if providers == 0 && harnesses > 0 => DetectTab::Harnesses,
        None => DetectTab::Providers,
    }
}

/// Draw the two tabs and return the one that is showing. An empty tab is dimmed, not disabled: it
/// still opens, and says what is missing.
pub fn bar(ui: &mut egui::Ui, chosen: &mut Option<DetectTab>, detected: &[Detected]) -> DetectTab {
    ui.horizontal_wrapped(|ui| buttons(ui, chosen, detected)).inner
}

/// Draw the tab buttons in the caller's layout, so adjacent actions share its spacing and wrapping.
pub fn buttons(ui: &mut egui::Ui, chosen: &mut Option<DetectTab>, detected: &[Detected]) -> DetectTab {
    let (providers, harnesses) = counts(detected);
    let showing = resolve(*chosen, providers, harnesses);
    for (tab, label, count) in [(DetectTab::Providers, "Providers", providers), (DetectTab::Harnesses, "Harnesses", harnesses)] {
        let mut text = egui::RichText::new(format!("{} ({count})", crate::i18n::tr(label)));
        if count == 0 {
            text = text.weak();
        }
        if ui.selectable_label(showing == tab, text).clicked() {
            *chosen = Some(tab);
        }
    }
    showing
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_anything_ai::{BackendConfig, BackendKind, Detected};

    fn detected(kind: BackendKind) -> Detected {
        let suggestion = BackendConfig { name: "X".into(), kind, model: "m".into(), base_url: None, ssh_host: None, enabled: true };
        Detected { suggestion, reason: String::new(), needs_key: false }
    }

    /// An agent app is a harness; an API backend and a local server are providers.
    #[test]
    fn the_two_kinds_land_on_their_own_tab() {
        assert_eq!(tab_of(&detected(BackendKind::AgentCli)), DetectTab::Harnesses);
        assert_eq!(tab_of(&detected(BackendKind::Openai)), DetectTab::Providers);
        assert_eq!(tab_of(&detected(BackendKind::OpenaiCompatible)), DetectTab::Providers);
        let all = [detected(BackendKind::AgentCli), detected(BackendKind::Anthropic), detected(BackendKind::Gemini)];
        assert_eq!(counts(&all), (2, 1));
    }

    /// The first choice prefers the tab with something; a click is always honoured, so an empty tab
    /// can show what it is missing.
    #[test]
    fn the_default_gives_way_but_a_click_does_not() {
        assert_eq!(resolve(None, 0, 3), DetectTab::Harnesses, "only harnesses were found");
        assert_eq!(resolve(None, 2, 0), DetectTab::Providers);
        assert_eq!(resolve(None, 0, 0), DetectTab::Providers, "nothing at all: the first tab");
        assert_eq!(resolve(Some(DetectTab::Harnesses), 2, 0), DetectTab::Harnesses, "a clicked empty tab stays");
        assert_eq!(resolve(Some(DetectTab::Providers), 0, 3), DetectTab::Providers, "a clicked empty tab stays");
        assert_eq!(resolve(Some(DetectTab::Harnesses), 2, 3), DetectTab::Harnesses);
    }

    /// #1358: hosted API providers accept a pasted key even when found in the environment, while
    /// local servers and agent harnesses do not.
    #[test]
    fn can_paste_key_covers_env_detected_providers_only() {
        let with_url = |kind, url: Option<&str>, needs_key| Detected {
            suggestion: BackendConfig { name: "X".into(), kind, model: "m".into(), base_url: url.map(str::to_string), ssh_host: None, enabled: true },
            reason: String::new(),
            needs_key,
        };
        for kind in [BackendKind::Openai, BackendKind::Anthropic, BackendKind::Openrouter, BackendKind::Gemini] {
            assert!(can_paste_key(&with_url(kind, None, false)), "{kind:?} with env key must be pastable");
            assert!(can_paste_key(&with_url(kind, None, true)), "{kind:?} without env key must be pastable");
        }
        assert!(can_paste_key(&with_url(BackendKind::OpenaiCompatible, None, true)), "explicit needs_key stays pastable");
        assert!(can_paste_key(&with_url(BackendKind::OpenaiCompatible, Some(craft_anything_ai::openai::OPENCODE_GO_BASE), false)));
        for provider in craft_anything_ai::providers::PROVIDERS {
            assert!(can_paste_key(&with_url(provider.kind, Some(provider.base_url), false)), "hosted provider {} must be pastable", provider.name);
        }
        assert!(!can_paste_key(&with_url(BackendKind::OpenaiCompatible, Some("http://127.0.0.1:11434/v1"), false)));
        assert!(!can_paste_key(&with_url(BackendKind::OpenaiCompatible, Some("http://192.168.1.20:11434/v1"), false)));
        assert!(!can_paste_key(&with_url(BackendKind::OpenaiCompatible, None, false)));
        assert!(!can_paste_key(&with_url(BackendKind::AgentCli, None, false)));
    }
}
