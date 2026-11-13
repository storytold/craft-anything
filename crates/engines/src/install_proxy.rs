//! Windows manual-proxy fallback for engine downloads only. No global ureq features or OS writes.
use ureq::{Proxy, ProxyProtocol};

// Deliberately not Debug: a rejected registry value can contain credentials.
pub(super) struct ManualSettings {
    pub enabled: bool,
    pub server: String,
    pub overrides: String,
}

#[cfg(windows)]
pub(super) fn for_download() -> Option<Proxy> {
    // Delegate existing environment priority, authentication and NO_PROXY semantics to ureq.
    let environment = Proxy::try_from_env();
    if environment.is_some() {
        return environment;
    }
    let manual = read_manual_settings()?;
    let bypass = std::env::var("NO_PROXY").or_else(|_| std::env::var("no_proxy")).ok();
    if bypass.is_none() && manual.overrides.split(';').map(str::trim).any(|p| !p.is_empty() && !exception(p)) {
        log::warn!("Some Windows proxy exceptions are unsupported for engine downloads; use NO_PROXY for explicit hosts.");
    }
    let selected = select_proxy(None, Some(&manual), bypass.as_deref());
    if manual.enabled {
        if selected.is_some() {
            log::info!("Windows manual HTTP proxy configured for engine downloads; bypass rules apply.");
        } else {
            log::warn!("Windows manual proxy setting is invalid or unsupported for engine downloads; connecting directly.");
        }
    }
    selected
}

#[cfg(windows)]
fn read_manual_settings() -> Option<ManualSettings> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings", KEY_READ).ok()?;
    Some(ManualSettings {
        enabled: key.get_value::<u32, _>("ProxyEnable").ok()? == 1,
        server: key.get_value("ProxyServer").ok()?,
        overrides: key.get_value("ProxyOverride").unwrap_or_default(),
    })
}

pub(super) fn select_proxy(environment: Option<Proxy>, manual: Option<&ManualSettings>, no_proxy: Option<&str>) -> Option<Proxy> {
    if environment.is_some() {
        return environment;
    }
    let manual = manual.filter(|m| m.enabled && (no_proxy.is_some() || m.overrides.len() <= 4096))?;
    let (host, port) = endpoint(&manual.server)?;
    let mut proxy = Proxy::builder(ProxyProtocol::Http).host(host).port(port);
    // Only the system fallback adds these exceptions; an environment proxy is left unchanged.
    for pattern in ["localhost", "127.*", "[::1]", "::1", "[0:0:0:0:0:0:0:1]"] {
        proxy = proxy.no_proxy(pattern);
    }
    if let Some(no_proxy) = no_proxy {
        // Explicitly empty means no OS exceptions, just as an empty NO_PROXY does for ureq.
        for pattern in no_proxy.split(',').map(str::trim) {
            proxy = proxy.no_proxy(pattern);
        }
    } else {
        for pattern in manual.overrides.split(';').map(str::trim).filter(|p| exception(p)) {
            proxy = proxy.no_proxy(pattern);
            let address = pattern.strip_prefix('[').and_then(|p| p.strip_suffix(']')).unwrap_or(pattern);
            if let Ok(ip) = address.parse::<std::net::Ipv6Addr>() {
                proxy = proxy.no_proxy(&format!("[{address}]")).no_proxy(&format!("[{ip}]"));
            }
        }
    }
    proxy.build().ok()
}

fn endpoint(setting: &str) -> Option<(&str, u16)> {
    if setting.len() > 4096 || setting.chars().any(char::is_control) {
        return None;
    }
    let setting = setting.trim();
    let address = if setting.contains('=') {
        let find = |protocol: &str| {
            setting.split(';').find_map(|part| {
                let (name, address) = part.split_once('=')?;
                name.trim().eq_ignore_ascii_case(protocol).then_some(address.trim())
            })
        };
        find("https").or_else(|| find("http"))?
    } else {
        setting
    };
    let address = match address.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => rest,
        Some(_) => return None,
        None => address,
    };
    let address = address.strip_suffix('/').unwrap_or(address);
    if address.is_empty() || address.chars().any(|c| c.is_whitespace() || c.is_control() || "@/?#\\".contains(c)) {
        return None;
    }
    let (host, port) = if address.starts_with('[') {
        let end = address.find(']')?;
        let host = address.get(..=end)?;
        host.strip_prefix('[')?.strip_suffix(']')?.parse::<std::net::Ipv6Addr>().ok()?;
        let rest = address.get(end + 1..)?;
        let port = if rest.is_empty() { 80 } else { rest.strip_prefix(':')?.parse::<u16>().ok()? };
        (host, port)
    } else {
        let (host, port) = match address.split_once(':') {
            Some((host, port)) => (host, port.parse::<u16>().ok()?),
            None => (address, 80),
        };
        if !domain(host) {
            return None;
        }
        (host, port)
    };
    (port != 0).then_some((host, port))
}

fn domain(host: &str) -> bool {
    let host = host.strip_suffix('.').unwrap_or(host);
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| !label.is_empty() && label.len() <= 63 && label.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'))
        && (!host.bytes().all(|c| c.is_ascii_digit() || c == b'.') || host.parse::<std::net::Ipv4Addr>().is_ok())
}

fn exception(value: &str) -> bool {
    if value == "*" {
        return true;
    }
    let core = value.strip_prefix('*').unwrap_or(value);
    let core = core.strip_suffix('*').unwrap_or(core);
    let core = core.trim_matches('.');
    !core.is_empty()
        && !core.contains('*')
        && (domain(core)
            || (value.ends_with('*') && core.split('.').all(|p| p.parse::<u8>().is_ok()))
            || core.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(core).parse::<std::net::Ipv6Addr>().is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(server: &str, overrides: &str) -> ManualSettings {
        ManualSettings { enabled: true, server: server.into(), overrides: overrides.into() }
    }

    #[test]
    fn single_addresses_and_protocol_lists() {
        for (value, host, port) in [
            ("proxy.example:8080", "proxy.example", 8080),
            ("proxy.example", "proxy.example", 80),
            ("HTTP://127.0.0.1:7897/", "127.0.0.1", 7897),
            ("[::1]:8080", "[::1]", 8080),
            ("http=one:80; HTTPS = two:8080 ;socks=ignored:1080", "two", 8080),
            ("socks=ignored:1080;http=one:81", "one", 81),
        ] {
            assert_eq!(endpoint(value), Some((host, port)), "{value}");
        }
    }

    #[test]
    fn unsupported_settings_are_not_partially_reinterpreted() {
        for value in [
            "",
            "https=;http=valid:80",
            "https=socks5://bad:1080;http=valid:80",
            "socks=only:1080",
            "https://proxy:443",
            "http://sk-test:sk-test@proxy:80",
            "proxy/path",
            "proxy?token=sk-test",
            "proxy#fragment",
            "bad:0",
            "bad:65536",
            "bad:abc",
            "[invalid]:80",
            "::1:80",
            "bad host:80",
            "999.999.999.999:80",
            "proxy\n:80",
            "http=one:80 https=two:81",
        ] {
            assert!(endpoint(value).is_none(), "{value}");
        }
        assert!(endpoint(&"x".repeat(4097)).is_none());
    }

    #[test]
    fn no_manual_setting_preserves_direct_and_environment_behavior() {
        assert!(select_proxy(None, None, None).is_none());
        let mut manual = settings("proxy.example:8080", "");
        manual.enabled = false;
        assert!(select_proxy(None, Some(&manual), None).is_none());
        manual.enabled = true;
        let environment = Proxy::builder(ProxyProtocol::Http).host("env.example").port(1234).no_proxy("*.environment.example").build().unwrap();
        let selected = select_proxy(Some(environment), Some(&manual), None).unwrap();
        assert_eq!(selected.host(), "env.example");
        assert!(selected.is_no_proxy(&"https://one.environment.example/".parse().unwrap()));
        assert!(!selected.is_no_proxy(&"https://localhost/".parse().unwrap()), "do not change existing environment bypass policy");
    }

    #[test]
    fn system_fallback_honors_supported_exceptions_and_loopback() {
        let manual = settings("proxy.example:8080", "*.corp.example;EXACT.example;10.*;[2001:db8::1];2001:db8::2;<local>;192.0.2.0/24;bad@pattern");
        let proxy = select_proxy(None, Some(&manual), None).unwrap();
        for host in ["api.corp.example", "exact.example", "10.2.3.4", "[2001:db8::1]", "[2001:db8::2]", "localhost", "127.0.0.2", "[::1]"] {
            assert!(proxy.is_no_proxy(&format!("https://{host}/").parse().unwrap()), "{host}");
        }
        assert!(!proxy.is_no_proxy(&"https://github.com/".parse().unwrap()));
        assert!(!proxy.is_no_proxy(&"https://singlelabel/".parse().unwrap()), "<local> is outside this version's supported rules");
        let replaced = select_proxy(None, Some(&manual), Some("only.example")).unwrap();
        assert!(replaced.is_no_proxy(&"https://only.example/".parse().unwrap()));
        assert!(!replaced.is_no_proxy(&"https://exact.example/".parse().unwrap()));
        let empty = select_proxy(None, Some(&manual), Some("")).unwrap();
        assert!(!empty.is_no_proxy(&"https://exact.example/".parse().unwrap()));
        assert!(empty.is_no_proxy(&"https://127.0.0.3/".parse().unwrap()));
        assert!(select_proxy(None, Some(&manual), Some("*")).unwrap().is_no_proxy(&"https://github.com/".parse().unwrap()));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "subprocess helper; run only through environment_precedence_in_child_processes"]
    fn environment_child() {
        let Ok(expected) = std::env::var("CRAFT_ANYTHING_TEST_PROXY_HOST") else { return };
        let proxy = for_download().unwrap();
        assert_eq!(proxy.host(), expected);
        assert!(proxy.is_no_proxy(&"https://skip.example/".parse().unwrap()));
        assert!(!proxy.is_no_proxy(&"https://localhost/".parse().unwrap()), "environment proxies keep their own bypass rules");
    }

    #[cfg(windows)]
    #[test]
    fn environment_precedence_in_child_processes() {
        let exe = std::env::current_exe().unwrap();
        for (all, https, http, expected) in [
            ("http://all.invalid:8001", "http://https.invalid:8002", "http://http.invalid:8003", "all.invalid"),
            ("", "http://https.invalid:8002", "http://http.invalid:8003", "https.invalid"),
            ("", "", "http://http.invalid:8003", "http.invalid"),
            ("not a proxy", "http://https.invalid:8002", "", "https.invalid"),
            ("", "http://sk-test:sk-test@https.invalid:8002", "", "https.invalid"),
        ] {
            let mut child = craft_anything_platform::Process::new(&exe)
                .args(["--ignored", "--exact", "install::download_proxy::tests::environment_child"])
                .timeout(std::time::Duration::from_secs(10));
            for key in ["ALL_PROXY", "all_proxy", "HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "NO_PROXY", "no_proxy"] {
                child = child.env_remove(key);
            }
            let output = child
                .env("ALL_PROXY", all)
                .env("HTTPS_PROXY", https)
                .env("HTTP_PROXY", http)
                .env("NO_PROXY", "skip.example")
                .env("CRAFT_ANYTHING_TEST_PROXY_HOST", expected)
                .run()
                .unwrap();
            assert!(output.success, "{}", output.stdout_text());
        }
    }
}
