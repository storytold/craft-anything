//! Splitting what Detect found into the two things a person is choosing between: a coding agent
//! already installed and signed in (it runs on the person's own subscription, no key), and an API
//! provider (a key, billed per token, or a model server). Without the split, eight more providers
//! push the coding agents off the screen. Coding agents come first: most people asked for a key
//! they never needed (#186).

use craft_anything_ai::Detected;

/// Which half of the detected list is showing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DetectTab {
    #[default]
    Providers,
    Harnesses,
}

impl DetectTab {
    /// The tab's name on screen. "Harness" is jargon; a person knows a coding agent.
    pub fn label(self) -> &'static str {
        match self {
            DetectTab::Harnesses => "Coding agents",
            DetectTab::Providers => "Providers",
        }
    }

    /// One line under the tabs saying what the shown kind costs and needs, so nobody goes looking
    /// for an API key they don't need (#186).
    pub fn blurb(self) -> &'static str {
        match self {
            DetectTab::Harnesses => "Uses the subscription you already have once you're signed in. No API key needed.",
            DetectTab::Providers => "An API key, billed per use, or a model server such as Ollama.",
        }
    }
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
/// of this kind was found" line is read. The automatic first choice is a coding agent whenever one
/// was found, since it needs no key (#186), and otherwise the tab that has something, so adding the
/// only provider never opens on an empty screen.
pub fn resolve(chosen: Option<DetectTab>, providers: usize, harnesses: usize) -> DetectTab {
    match chosen {
        Some(tab) => tab,
        None if harnesses > 0 => DetectTab::Harnesses,
        None if providers > 0 => DetectTab::Providers,
        None => DetectTab::Harnesses,
    }
}

/// Draw the two tabs and return the one that is showing. An empty tab is dimmed, not disabled: it
/// still opens, and says what is missing.
pub fn bar(ui: &mut egui::Ui, chosen: &mut Option<DetectTab>, detected: &[Detected]) -> DetectTab {
    let tab = ui.horizontal_wrapped(|ui| buttons(ui, chosen, detected)).inner;
    blurb(ui, tab);
    tab
}

/// The shown tab's line on what it needs: no key for a coding agent, a key or a server otherwise.
pub fn blurb(ui: &mut egui::Ui, tab: DetectTab) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.add(egui::Label::new(egui::RichText::new(crate::i18n::tr(tab.blurb())).color(t.text_dim).size(crate::theme::SMALL)).wrap());
}

/// Draw the tab buttons in the caller's layout, so adjacent actions share its spacing and wrapping.
pub fn buttons(ui: &mut egui::Ui, chosen: &mut Option<DetectTab>, detected: &[Detected]) -> DetectTab {
    let (providers, harnesses) = counts(detected);
    let showing = resolve(*chosen, providers, harnesses);
    // Coding agents first: they need no key, and a key is what people thought they had to find.
    for (tab, count) in [(DetectTab::Harnesses, harnesses), (DetectTab::Providers, providers)] {
        let mut text = egui::RichText::new(format!("{} ({count})", crate::i18n::tr(tab.label())));
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

    /// The first choice is a coding agent whenever one was found, then the tab with something; a
    /// click is always honoured, so an empty tab can show what it is missing.
    #[test]
    fn the_default_gives_way_but_a_click_does_not() {
        assert_eq!(resolve(None, 0, 3), DetectTab::Harnesses, "only harnesses were found");
        assert_eq!(resolve(None, 2, 0), DetectTab::Providers);
        assert_eq!(resolve(None, 5, 1), DetectTab::Harnesses, "a coding agent needs no key, so it comes first (#186)");
        assert_eq!(resolve(None, 0, 0), DetectTab::Harnesses, "nothing at all: the first tab, coding agents");
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
