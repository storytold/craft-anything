//! How a backend is paid for and which account it uses (#170).
//!
//! People with both a coding-agent subscription and an API key, or with several accounts, couldn't
//! tell which one an AI in Settings used. [`billing_of`] answers from the saved settings alone, with
//! no I/O; a coding agent's own sign-in check ([`SignIn`]) can say more, and the account it names,
//! when it reports one. Nothing here reads, keeps or prints any part of a key.

use crate::{BackendConfig, BackendKind};
use serde::{Deserialize, Serialize};

/// How a backend is billed or authenticated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Billing {
    /// A coding agent signed in to a plan (Claude Pro or Max, ChatGPT…): its usage counts
    /// against the plan, not against a key.
    Subscription,
    /// An API key: each request is paid for by the key's account.
    ApiKey,
    /// A server the person runs (on this computer, over SSH or on their own network): no account.
    OwnServer,
    /// A coding agent that uses its own sign-in, which may be a plan or a key, and hasn't said
    /// which (its check can't tell, or hasn't run yet).
    AgentSignIn,
}

impl Billing {
    /// The words shown beside the AI.
    pub fn label(self) -> &'static str {
        match self {
            Billing::Subscription => "Subscription sign-in",
            Billing::ApiKey => "API key",
            Billing::OwnServer => "Your own server",
            Billing::AgentSignIn => "The app's own sign-in",
        }
    }

    /// One sentence on what that means for what it costs.
    pub fn explanation(self) -> &'static str {
        match self {
            Billing::Subscription => "Uses your plan's limits; requests aren't billed to an API key.",
            Billing::ApiKey => "Each request is billed to the key's account.",
            Billing::OwnServer => "Runs on a server you control; no account is billed.",
            Billing::AgentSignIn => "Uses whatever the app is signed in with: a plan or a key. Check it to find out which.",
        }
    }
}

/// What a coding agent's own status check says about its sign-in. Built from the agent's status
/// output; the output itself is never kept.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SignIn {
    pub signed_in: bool,
    /// How it's billed, when the agent says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing: Option<Billing>,
    /// The account it's signed in to (an email address or organization), when the agent names
    /// one in its own status. Never part of a key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

impl SignIn {
    /// Signed in or out, with nothing more known.
    pub fn bare(signed_in: bool) -> Self {
        Self { signed_in, billing: None, account: None }
    }
}

/// How `config` is billed, from its saved settings alone (no I/O, safe to call while drawing).
pub fn billing_of(config: &BackendConfig) -> Billing {
    match config.kind {
        BackendKind::Anthropic | BackendKind::Openai | BackendKind::Openrouter | BackendKind::Gemini => Billing::ApiKey,
        BackendKind::AgentCli => Billing::AgentSignIn,
        BackendKind::OpenaiCompatible => {
            let over_ssh = config.ssh_host.as_deref().is_some_and(|h| !h.trim().is_empty());
            // No address is the default local server (Ollama on this computer).
            let base = config.base_url.as_deref().map(str::trim).filter(|b| !b.is_empty());
            if over_ssh || base.is_none_or(|b| crate::http::is_this_computer(b) || on_own_network(b)) { Billing::OwnServer } else { Billing::ApiKey }
        }
    }
}

/// A private-network address (`192.168.x.x`, `10.x.x.x`, `172.16-31.x.x`, link-local, `fd…` or
/// `fe80…`) or a `.local`/`.lan`/`.home.arpa` name: a server the person runs, not a paid service.
fn on_own_network(url: &str) -> bool {
    let host = crate::http::host_of(url);
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        return ip.is_private() || ip.is_link_local();
    }
    if let Ok(ip) = host.parse::<std::net::Ipv6Addr>() {
        let first = ip.segments().first().copied().unwrap_or_default();
        return (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80;
    }
    [".local", ".lan", ".home.arpa", ".internal"].iter().any(|s| host.ends_with(s))
}

/// The account and billing in Claude Code's `auth status --json` answer. Its known fields:
/// `loggedIn`, `authMethod` (`claude.ai` for a plan's sign-in; a key's method otherwise),
/// `apiProvider` (`firstParty`, or a cloud such as Bedrock or Vertex), `subscriptionType`,
/// `email` and `orgName`. Anything else is ignored, and an unknown method leaves billing unknown.
pub(crate) fn claude_sign_in(status: &serde_json::Value) -> Option<SignIn> {
    let text = |k: &str| status.get(k).and_then(serde_json::Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let signed_in = status.get("loggedIn").and_then(serde_json::Value::as_bool)?;
    if !signed_in {
        return Some(SignIn::bare(false));
    }
    let method = text("authMethod").map(str::to_ascii_lowercase);
    let cloud = text("apiProvider").is_some_and(|p| !p.eq_ignore_ascii_case("firstParty"));
    let billing = if cloud || method.as_deref().is_some_and(|m| m.contains("key") || m.contains("console")) {
        Some(Billing::ApiKey)
    } else if text("subscriptionType").is_some() || method.as_deref().is_some_and(|m| m == "claude.ai" || m.contains("oauth")) {
        Some(Billing::Subscription)
    } else {
        None
    };
    let account = text("email").or_else(|| text("orgName")).map(|a| a.chars().take(120).collect());
    Some(SignIn { signed_in, billing, account })
}

/// Billing in `codex login status`'s answer: "Logged in using ChatGPT" for a plan, "Logged in
/// using an API key - …" for a key (whose shown part is never read past these words).
pub(crate) fn codex_billing(text: &str) -> Option<Billing> {
    let text = text.to_ascii_lowercase();
    let using = text.split("logged in using").nth(1)?;
    if using.trim_start().starts_with("chatgpt") {
        Some(Billing::Subscription)
    } else if using.contains("api key") {
        Some(Billing::ApiKey)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn backend(kind: BackendKind, base: Option<&str>, ssh: Option<&str>) -> BackendConfig {
        BackendConfig { name: "x".into(), kind, model: "m".into(), base_url: base.map(str::to_string), ssh_host: ssh.map(str::to_string), enabled: true }
    }

    #[test]
    fn each_kind_of_backend_has_a_billing_kind() {
        for kind in [BackendKind::Anthropic, BackendKind::Openai, BackendKind::Openrouter, BackendKind::Gemini] {
            assert_eq!(billing_of(&backend(kind, None, None)), Billing::ApiKey, "{kind:?}");
        }
        assert_eq!(billing_of(&backend(BackendKind::AgentCli, None, None)), Billing::AgentSignIn);
        let compatible = |base, ssh| billing_of(&backend(BackendKind::OpenaiCompatible, base, ssh));
        for own in [
            "http://127.0.0.1:11434/v1",
            "http://localhost:1234/v1",
            "http://192.168.1.20:11434/v1",
            "http://10.0.0.5/v1",
            "http://[fd00::1]:8080",
            "http://box.local:11434",
        ] {
            assert_eq!(compatible(Some(own), None), Billing::OwnServer, "{own}");
        }
        assert_eq!(compatible(None, None), Billing::OwnServer, "the default address is this computer");
        assert_eq!(compatible(Some("  "), None), Billing::OwnServer);
        assert_eq!(compatible(Some("https://api.x.ai/v1"), Some("studio")), Billing::OwnServer, "a server reached over SSH");
        for paid in ["https://api.x.ai/v1", "https://opencode.ai/zen/go/openai/v1", "http://localhost.example.com/v1", "http://8.8.8.8/v1"] {
            assert_eq!(compatible(Some(paid), None), Billing::ApiKey, "{paid}");
        }
    }

    #[test]
    fn every_billing_kind_has_words() {
        for b in [Billing::Subscription, Billing::ApiKey, Billing::OwnServer, Billing::AgentSignIn] {
            assert!(!b.label().is_empty() && b.explanation().ends_with('.'), "{b:?}");
        }
    }

    #[test]
    fn claude_code_status_names_its_plan_or_key_and_account() {
        let plan = json!({"loggedIn": true, "authMethod": "claude.ai", "apiProvider": "firstParty", "email": "someone@example.com", "orgName": "Org", "subscriptionType": "max"});
        assert_eq!(claude_sign_in(&plan), Some(SignIn { signed_in: true, billing: Some(Billing::Subscription), account: Some("someone@example.com".into()) }));
        let key = json!({"loggedIn": true, "authMethod": "api_key", "apiProvider": "firstParty", "orgName": "Org"});
        assert_eq!(claude_sign_in(&key), Some(SignIn { signed_in: true, billing: Some(Billing::ApiKey), account: Some("Org".into()) }));
        let cloud = json!({"loggedIn": true, "apiProvider": "bedrock"});
        assert_eq!(claude_sign_in(&cloud).and_then(|s| s.billing), Some(Billing::ApiKey));
        let unknown = json!({"loggedIn": true, "authMethod": "something-new", "email": ""});
        assert_eq!(claude_sign_in(&unknown), Some(SignIn::bare(true)), "an unknown method and an empty account say nothing");
        let out = json!({"loggedIn": false, "email": "someone@example.com"});
        assert_eq!(claude_sign_in(&out), Some(SignIn::bare(false)), "a signed-out status names no account");
        for none in [json!({}), json!({"loggedIn": "yes"}), json!([1])] {
            assert_eq!(claude_sign_in(&none), None);
        }
        let long = json!({"loggedIn": true, "email": "a".repeat(10_000)});
        assert_eq!(claude_sign_in(&long).and_then(|s| s.account).map(|a| a.chars().count()), Some(120));
    }

    #[test]
    fn codex_status_names_its_plan_or_key() {
        assert_eq!(codex_billing("Logged in using ChatGPT\n"), Some(Billing::Subscription));
        assert_eq!(codex_billing("Logged in using an API key - sk-test***"), Some(Billing::ApiKey));
        assert_eq!(codex_billing("Logged in"), None);
        assert_eq!(codex_billing("Not logged in"), None);
    }
}
