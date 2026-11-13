# Engine download proxies

This applies to the shared downloader used by Get Ghidra (including its JDK), pinned engine
installs and game-engine downloads. It does not change AI requests or other network clients.
Every download and redirect must remain HTTPS; a proxy is not an exception to that rule.

On Windows, the first valid proxy from ureq's environment selection wins: `ALL_PROXY` /
`all_proxy`, then `HTTPS_PROXY` / `https_proxy`, then `HTTP_PROXY` / `http_proxy`. Its authentication
and `NO_PROXY` behavior are unchanged. An environment proxy's loopback policy is not overridden.

If no valid environment proxy exists, the downloader reads the current user's enabled manual
Internet Settings proxy, without changing it. It accepts one HTTP proxy (`host:port`, optionally
`http://`, default port 80), or a semicolon list such as `http=one:8080;https=two:8081`. HTTPS
downloads choose the HTTPS entry; only its absence permits the HTTP entry. An explicitly invalid
HTTPS entry does not fall back to another entry. IPv6 proxy addresses need brackets.

For this system fallback:

- Localhost, IPv4 loopback (`127.*`) and the usual IPv6 loopback literals bypass the proxy.
- `NO_PROXY` / `no_proxy`, if set, replaces Windows `ProxyOverride`; an empty value means no
  Windows exceptions. Loopback still goes direct. The environment list uses ureq's syntax.
- Otherwise, `ProxyOverride` supports semicolon-separated exact domains/addresses, leading or
  trailing wildcard patterns (for example `*.example.com`, `10.*`) and `*`.
- Credential-bearing or non-HTTP system proxy addresses, invalid ports and malformed settings
  are ignored with a generic warning. New routing logs contain neither settings nor credentials.
  Authenticated environment proxies continue to work as before.

PAC/WPAD, SOCKS-only system settings, `<local>`, CIDR, port-qualified and complex interior-wildcard
Windows exceptions are not supported by this first implementation. Unsupported exceptions get a
generic warning; use explicit `NO_PROXY` hosts when needed. Settings over the parser's 4 KiB limit
are not used. There is no proxy configuration UI or macOS/Linux system-proxy discovery in this
change; those platforms retain their existing environment behavior.

Proxy discovery is repeated for each download, so changing a manual proxy does not require a
global cache reset. A working route does not guarantee that a large transfer will complete:
normal timeout, checksum and installation errors can still occur.

The regular tests use injected settings and local CONNECT/TLS fixtures, without registry writes.
The ignored `windows_manual_proxy_head_smoke` test is an explicitly gated live check against the
official Java download pin using HEAD only; it is not a complete Ghidra installation test.
