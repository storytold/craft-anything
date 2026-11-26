//! A stand-in backend, described from a recording of the original's network traffic. Someone
//! rebuilding a website's front end needs something for it to talk to; this says how the original
//! answered, route by route, so the same requests can be answered locally.
//!
//! This module only describes and decides. It has no sockets and no files: [`build`] turns a parsed
//! HAR into a versioned [`StandIn`], and [`answer`] picks what to send back for a request.
//!
//! Secrets don't go in the file. Request headers aren't kept at all. Cookie and credential response
//! headers, credential query values, secret-shaped values in bodies, and any value the caller names
//! are all dropped, and [`Dropped`] says how many. Bodies are otherwise kept: they're the answer.
//!
//! Replay follows the recording's timeline. Each answer keeps its place in the recording, and a
//! write a client replays moves later reads of its path (and of paths above or under it) on to the
//! answers recorded after that write, so a list read after a recorded "add" shows what the
//! original showed then. Nothing is computed from what the client sends: a write steps to the next
//! recorded write of its kind whatever it carries, and one the recording has no answer of its own
//! for (past the recorded ones of its kind) is refused and changes nothing.

use crate::web_har::{CREDENTIAL_HEADERS, Redactor, contains_ascii_ci, decode_query_name, is_credential_param, is_uuid, replace_needles_with, split_url};
use craft_anything_formats::har::{self, Har, HarEntry, HarError};
use craft_anything_formats::sourcemap::base64_decode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// The version of the file a [`StandIn`] is saved as (see docs/file-formats.md). 2 adds each
/// sample's place in the recording ([`Sample::order`]) and whether it answered a write ([`Sample::write`]).
pub const STANDIN_VERSION: u32 = 2;

/// Methods that read: anything else may change what later reads return.
const READ_METHODS: [&str; 4] = ["GET", "HEAD", "OPTIONS", "TRACE"];

/// Takes the place of a dropped secret.
const DROPPED: &str = "[dropped]";
/// A path segment that varies, in a template; also a secret-shaped segment in a kept path.
const ID: &str = "{id}";

const MAX_SEGMENTS: usize = 32;
const MAX_HEADERS_PER_SAMPLE: usize = 64;
const MAX_HEADER_VALUE: usize = 4096;
const MAX_LISTED: usize = 64;
const MAX_HOSTS: usize = 256;
const MAX_DROPPED_NAMES: usize = 32;
const MAX_NEAREST: usize = 5;
/// Longest request body [`request_key`] reads.
const MAX_KEY_BODY: usize = 1024 * 1024;
const MAX_KEY_NAMES: usize = 16;
/// Hex this long is an id, not a word (`facade`, `decade`).
const MIN_HEX_ID: usize = 16;
/// Mixed letters and digits this long is an id (`a1b2…`), which short names rarely are.
const MIN_TOKEN_ID: usize = 20;
/// How many different last segments, each answered in the same shape, make a word-like id.
const MIN_SIBLINGS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOptions {
    /// Values to drop wherever they appear, in every encoding. Only their count is recorded.
    pub hidden_values: Vec<String>,
    /// Only these hosts (`api.example.com` or with a port). Empty keeps every host.
    pub only_hosts: Vec<String>,
    /// Also keep pages, scripts, styles, pictures, fonts and media. Off: a rebuilt front end brings its own.
    pub include_static: bool,
    pub max_entries: usize,
    pub max_routes: usize,
    pub max_samples_per_route: usize,
    /// A body larger than this is left out whole, because half of a JSON document is no answer.
    pub max_body_bytes: usize,
    pub max_total_body_bytes: usize,
    /// The recording's file name (never its path) and digest, to say where this came from.
    pub source_name: Option<String>,
    pub source_sha256: Option<String>,
}

impl Default for BuildOptions {
    fn default() -> Self {
        BuildOptions {
            hidden_values: Vec::new(),
            only_hosts: Vec::new(),
            include_static: false,
            max_entries: 500_000,
            max_routes: 5_000,
            max_samples_per_route: 16,
            max_body_bytes: 4 * 1024 * 1024,
            max_total_body_bytes: 256 * 1024 * 1024,
            source_name: None,
            source_sha256: None,
        }
    }
}

/// Everything recorded about how the original answered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StandIn {
    pub version: u32,
    pub source: StandInSource,
    pub hosts: Vec<String>,
    /// Sorted by template, then method.
    pub routes: Vec<StandInRoute>,
    pub dropped: Dropped,
    pub skipped: Skipped,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StandInSource {
    pub file_name: Option<String>,
    pub sha256: Option<String>,
    pub entries_read: u64,
    pub entries_declared: u64,
    /// The recording had more entries than were read.
    pub cut_short: bool,
}

/// One method and path shape, and how it answered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StandInRoute {
    pub method: String,
    /// `/users/{id}/posts`.
    pub template: String,
    pub hosts: Vec<String>,
    pub statuses: Vec<u16>,
    pub content_types: Vec<String>,
    pub query_names: Vec<String>,
    /// Requests recorded for this route, whether or not a sample was kept.
    pub requests: u64,
    /// In recording order.
    pub samples: Vec<Sample>,
    pub samples_left_out: u64,
}

/// One recorded answer, and the request it answered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sample {
    pub path: String,
    /// `a=1&b=2`: sorted, with credential values dropped.
    pub query: String,
    /// What told this request apart from others to the same URL: see [`request_key`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_key: Option<String>,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    pub headers: Vec<(String, String)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// A body that isn't text, as the recording held it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base64: Option<String>,
    /// Why there's no body, when there isn't.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_left_out: Option<BodyLeftOut>,
    /// Recorded as 304 with the cached body; kept as a 200, since nothing here sends validators.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub was_not_modified: bool,
    /// Where in the recording this answer was given: its entry's index. Files before version 2
    /// have none: their answers replay in turn, and a write gets each one once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<u64>,
    /// It answered a write: a method that isn't a read, unless the body was GraphQL that only
    /// queries. Replaying one the original accepted (see [`accepted`]) moves later reads of related
    /// paths on to what was recorded after it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub write: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyLeftOut {
    /// The recording has no body for this response.
    NotRecorded,
    TooLarge,
    /// The whole file reached its size limit.
    FileLimit,
    /// It's binary or marked base64 but couldn't be decoded.
    Undecodable,
    /// It held a secret that can't be cut out of it.
    HeldSecret,
}

/// What was left out so that it can't reach the file. Counts and header names, never values.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Dropped {
    /// Credential or secret-looking headers, on requests and responses.
    pub headers: u64,
    pub header_names: Vec<String>,
    /// Entries in the recording's own cookie lists.
    pub cookies: u64,
    pub query_values: u64,
    pub body_values: u64,
    pub path_segments: u64,
    pub url_user_info: u64,
    /// Places a value the caller named was found.
    pub hidden_values: u64,
}

impl Dropped {
    pub fn total(&self) -> u64 {
        [self.headers, self.cookies, self.query_values, self.body_values, self.path_segments, self.url_user_info, self.hidden_values]
            .iter()
            .fold(0u64, |a, b| a.saturating_add(*b))
    }
}

/// Recorded requests that aren't in the stand-in, by why.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Skipped {
    /// `data:`, `blob:`, extension pages, WebSockets…
    pub not_web: u64,
    /// No usable response: the request failed or was cancelled.
    pub no_response: u64,
    pub static_files: u64,
    pub other_hosts: u64,
    pub beyond_route_limit: u64,
}

// ---------------------------------------------------------------------------------------------
// Secrets

/// A word or token that is a credential by its form alone, wherever it turns up.
pub fn looks_secret(t: &str) -> bool {
    let has_digit = t.bytes().any(|b| b.is_ascii_digit());
    (t.starts_with("eyJ") && t.len() >= 20 && t.bytes().filter(|b| *b == b'.').count() == 2)
        || (t.starts_with("sk-") && (t.len() >= 12 || (has_digit && t.len() >= 8)))
        || [
            "sk_live_",
            "sk_test_",
            "rk_live_",
            "rk_test_",
            "whsec_",
            "ghp_",
            "gho_",
            "ghs_",
            "ghu_",
            "github_pat_",
            "xoxb-",
            "xoxp-",
            "xoxa-",
            "xoxs-",
            "glpat-",
        ]
        .iter()
        .any(|p| t.starts_with(p) && t.len() > p.len() + 3)
        || ((t.starts_with("AKIA") || t.starts_with("ASIA")) && t.len() >= 16 && t.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()))
        || (t.starts_with("AIza") && t.len() >= 30)
        || (t.starts_with("npm_") && t.len() >= 20)
}

fn is_secret_header(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    CREDENTIAL_HEADERS.contains(&l.as_str())
        || ["cookie", "auth", "token", "secret", "password", "credential", "csrf", "xsrf", "session", "signature", "api-key", "apikey", "-key"]
            .iter()
            .any(|w| l.contains(w))
}

/// Headers about the connection or the original's own security setup, which mean nothing (or the wrong
/// thing) coming from a stand-in on this computer. They're not secrets, so they aren't counted.
fn is_transport_header(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    matches!(
        l.as_str(),
        "connection"
            | "keep-alive"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "content-length"
            | "content-encoding"
            | "content-type"
            | "host"
            | "date"
            | "alt-svc"
            | "strict-transport-security"
            | "content-security-policy"
            | "content-security-policy-report-only"
            | "expect-ct"
            | "nel"
            | "report-to"
            | "reporting-endpoints"
            | "public-key-pins"
            | "server-timing"
            | "via"
    ) || l.starts_with("proxy-")
        || l.starts_with("access-control-")
        || l.starts_with(':')
}

/// A JSON key whose text value is a credential. Narrower than for query strings, because `key`,
/// `code` and `session` are everyday names in a response body; those still get the shape check.
fn is_secret_json_key(k: &str) -> bool {
    let l = k.to_ascii_lowercase().replace('-', "_");
    l == "authorization"
        || l == "cookie"
        || l == "set_cookie"
        || l == "api_key"
        || l == "apikey"
        || l == "private_key"
        || l.ends_with("token")
        || l.contains("password")
        || l.contains("passwd")
        || l.contains("secret")
        || l.contains("credential")
}

struct Scrub {
    /// Every spelling of every value the caller named.
    needles: Vec<String>,
    dropped: Dropped,
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '~' | '+' | '/' | '=')
}

/// What follows `Bearer` or `Basic` is a credential, unless it's an ordinary word ("Basic settings").
fn is_scheme_credential(token: &str, basic: bool) -> bool {
    let mut letters = token.chars();
    let first_is_any_case = letters.next().is_some_and(|c| c.is_ascii_alphabetic());
    let rest = letters.as_str();
    let word = first_is_any_case && (rest.bytes().all(|b| b.is_ascii_lowercase()) || (token.bytes().all(|b| b.is_ascii_uppercase())));
    token.len() >= 8 && !word && (!basic || token.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=')))
}

/// `token` with any secret-shaped part after a `/` or `=` replaced, when it has one.
fn without_secret_parts(token: &str) -> Option<String> {
    if !token.split(['/', '=']).any(looks_secret) {
        return None;
    }
    let part = |p: &str| if looks_secret(p) { DROPPED.to_string() } else { p.to_string() };
    let segment = |s: &str| s.split('=').map(part).collect::<Vec<_>>().join("=");
    Some(token.split('/').map(segment).collect::<Vec<_>>().join("/"))
}

/// Where a URL's user-info ends, given what follows its `//`.
fn user_info_end(after_slashes: &str) -> Option<usize> {
    let end = after_slashes.find(|c: char| matches!(c, '/' | '?' | '#' | '"' | '\'' | '<' | '>' | '\\') || c.is_whitespace()).unwrap_or(after_slashes.len());
    after_slashes.get(..end)?.rfind('@').map(|at| at + 1)
}

impl Scrub {
    fn new(values: &[String]) -> Scrub {
        Scrub { needles: Redactor::new(values).needles, dropped: Dropped::default() }
    }

    fn holds_hidden(&self, s: &str) -> bool {
        self.needles.iter().any(|n| contains_ascii_ci(s, n))
    }

    fn hide(&mut self, s: String) -> String {
        if !self.holds_hidden(&s) {
            return s;
        }
        let (out, found) = replace_needles_with(&s, &self.needles, DROPPED);
        self.dropped.hidden_values = self.dropped.hidden_values.saturating_add(found);
        out
    }

    /// `s` with the `user:password@` of every URL in it taken out.
    fn user_info(&mut self, s: &str) -> String {
        if !s.contains("://") || !s.contains('@') {
            return s.to_string();
        }
        let mut out = String::with_capacity(s.len());
        let mut rest = s;
        while let Some(i) = rest.find("://") {
            let (head, tail) = rest.split_at(i + 3);
            out.push_str(head);
            rest = match user_info_end(tail) {
                Some(end) => {
                    self.dropped.url_user_info = self.dropped.url_user_info.saturating_add(1);
                    tail.get(end..).unwrap_or_default()
                }
                None => tail,
            };
        }
        out.push_str(rest);
        out
    }

    /// Replace secret-shaped tokens in free text, `name=value` credentials, and whatever follows
    /// `Bearer` or `Basic`.
    fn tokens(&mut self, s: &str) -> String {
        self.scan(s, true)
    }

    /// The same for a script, a style sheet or a page: `x.token=y` is code there, and so is any word
    /// after `Basic`, so only forms that are credentials anywhere go, and `name=value` only in a URL.
    fn code(&mut self, s: &str) -> String {
        self.scan(s, false)
    }

    fn scan(&mut self, s: &str, free_text: bool) -> String {
        let s = self.user_info(s);
        let mut out = String::with_capacity(s.len());
        let mut rest = s.as_str();
        let mut scheme: Option<bool> = None;
        while let Some(start) = rest.find(is_token_char) {
            out.push_str(rest.get(..start).unwrap_or_default());
            let tail = rest.get(start..).unwrap_or_default();
            let end = tail.find(|c: char| !is_token_char(c)).unwrap_or(tail.len());
            let token = tail.get(..end).unwrap_or_default();
            let in_url = out.ends_with('?') || (out.ends_with('&') && !out.ends_with("&&")) || out.ends_with("&amp;");
            let named = (free_text || in_url) && token.split_once('=').is_some_and(|(k, v)| !v.is_empty() && (is_secret_json_key(k) || is_credential_param(k)));
            let after_scheme = free_text && scheme.is_some_and(|basic| is_scheme_credential(token, basic));
            if named || looks_secret(token) || after_scheme {
                self.dropped.body_values = self.dropped.body_values.saturating_add(1);
                out.push_str(DROPPED);
            } else if let Some(clean) = without_secret_parts(token) {
                self.dropped.body_values = self.dropped.body_values.saturating_add(1);
                out.push_str(&clean);
            } else {
                out.push_str(token);
            }
            scheme = if token.eq_ignore_ascii_case("bearer") {
                Some(false)
            } else if token.eq_ignore_ascii_case("basic") {
                Some(true)
            } else {
                None
            };
            rest = tail.get(end..).unwrap_or_default();
        }
        out.push_str(rest);
        out
    }

    fn json(&mut self, v: &mut Value) {
        match v {
            Value::String(s) => {
                let clean = self.tokens(s);
                if clean != *s {
                    *s = clean;
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| self.json(x)),
            Value::Object(o) => {
                let before = o.len();
                o.retain(|k, _| !looks_secret(k));
                self.dropped.body_values = self.dropped.body_values.saturating_add((before - o.len()) as u64);
                for (k, x) in o.iter_mut() {
                    match x {
                        Value::String(s) if !s.is_empty() && is_secret_json_key(k) => {
                            self.dropped.body_values = self.dropped.body_values.saturating_add(1);
                            *x = Value::String(DROPPED.into());
                        }
                        _ => self.json(x),
                    }
                }
            }
            _ => {}
        }
    }

    /// A path with secret-shaped or caller-named segments replaced by [`ID`].
    fn path(&mut self, path: &str, count: bool) -> String {
        let segs: Vec<String> = path
            .split('/')
            .map(|seg| {
                if self.holds_hidden(seg) {
                    if count {
                        self.dropped.hidden_values = self.dropped.hidden_values.saturating_add(1);
                    }
                    ID.to_string()
                } else if looks_secret(seg) {
                    if count {
                        self.dropped.path_segments = self.dropped.path_segments.saturating_add(1);
                    }
                    ID.to_string()
                } else {
                    seg.to_string()
                }
            })
            .collect();
        segs.join("/")
    }

    /// A query's names and canonical text: credential values dropped, pairs sorted.
    fn query(&mut self, pairs: &[(String, String)], count: bool) -> (Vec<String>, String) {
        let mut names = Vec::new();
        let mut kept = Vec::new();
        for (k, v) in pairs {
            if self.holds_hidden(k) || self.holds_hidden(&decode_query_name(k)) {
                if count {
                    self.dropped.hidden_values = self.dropped.hidden_values.saturating_add(1);
                }
                continue;
            }
            names.push(k.clone());
            let hidden_val = self.holds_hidden(v) || self.holds_hidden(&decode_query_name(v));
            if v.is_empty() {
                kept.push(k.clone());
            } else if is_credential_param(k) || is_secret_json_key(k) || looks_secret(v) || hidden_val {
                if count {
                    if hidden_val {
                        self.dropped.hidden_values = self.dropped.hidden_values.saturating_add(1);
                    } else {
                        self.dropped.query_values = self.dropped.query_values.saturating_add(1);
                    }
                }
                kept.push(format!("{k}={DROPPED}"));
            } else {
                kept.push(format!("{k}={v}"));
            }
        }
        kept.sort();
        names.sort();
        names.dedup();
        (names, kept.join("&"))
    }

    /// Apply current rules to a request or a saved query. Raw requests ignore empty segments,
    /// but canonical saved queries use empty slots for explicit empty key/value pairs (`=`).
    fn query_text(&mut self, query: &str, preserve_empty_pairs: bool) -> String {
        let pairs: Vec<(String, String)> = query
            .split('&')
            .filter(|p| preserve_empty_pairs || !p.is_empty())
            .take(200)
            .map(|p| p.split_once('=').map_or((p.to_string(), String::new()), |(k, v)| (k.to_string(), v.to_string())))
            .collect();
        self.query(&pairs, false).1
    }

    fn note_header(&mut self, name: &str) {
        self.dropped.headers = self.dropped.headers.saturating_add(1);
        let l = name.to_ascii_lowercase();
        if l.len() <= 64
            && !self.holds_hidden(&l)
            && !looks_secret(&l)
            && self.dropped.header_names.len() < MAX_DROPPED_NAMES
            && !self.dropped.header_names.contains(&l)
        {
            self.dropped.header_names.push(l);
        }
    }

    /// The response headers worth sending again, none of them secret.
    fn headers(&mut self, headers: &[(String, String)]) -> Vec<(String, String)> {
        let mut kept = Vec::new();
        for (k, v) in headers {
            if is_secret_header(k) {
                self.note_header(k);
                continue;
            }
            if is_transport_header(k)
                || kept.len() >= MAX_HEADERS_PER_SAMPLE
                || v.len() > MAX_HEADER_VALUE
                || k.bytes().any(|b| b.is_ascii_control())
                || v.bytes().any(|b| b.is_ascii_control())
            {
                continue;
            }
            // A redirect is still a redirect without its credentials; any other header that held
            // a secret-shaped value is left out whole.
            let is_location = k.eq_ignore_ascii_case("location");
            let before = self.dropped.body_values;
            let value = if is_location { self.location(v) } else { self.tokens(v) };
            let held_secret = self.dropped.body_values != before && !is_location;
            if held_secret || self.holds_hidden(&value) || self.holds_hidden(k) {
                // Counted as a header, not a body value.
                self.dropped.body_values = before;
                self.note_header(k);
                continue;
            }
            kept.push((k.clone(), value));
        }
        kept
    }

    /// Where a redirect goes: its query's credentials dropped, and any user-info with them.
    fn location(&mut self, url: &str) -> String {
        // `//user:password@host/…`: the one form the scan for `://` doesn't see.
        let relative = url.strip_prefix("//").and_then(|r| {
            let end = user_info_end(r)?;
            Some(format!("//{}", r.get(end..)?))
        });
        if relative.is_some() {
            self.dropped.url_user_info = self.dropped.url_user_info.saturating_add(1);
        }
        let url = relative.as_deref().unwrap_or(url);
        let Some((base, query)) = url.split_once('?') else { return self.tokens(url) };
        let pairs: Vec<(String, String)> = query
            .split('&')
            .take(200)
            .filter(|p| !p.is_empty())
            .map(|p| p.split_once('=').map_or((p.to_string(), String::new()), |(k, v)| (k.to_string(), v.to_string())))
            .collect();
        let (_, clean) = self.query(&pairs, true);
        format!("{}?{clean}", self.tokens(base))
    }
}

/// A request's path with secret-shaped segments replaced, fit to keep in a log of what was asked for.
pub fn safe_path(path: &str) -> String {
    Scrub::new(&[]).path(path, false)
}

// ---------------------------------------------------------------------------------------------
// What tells requests to one URL apart

/// What tells this request apart from others to the same URL, without keeping any of its values: the
/// GraphQL operations it names (`op:GetUser`), or the sorted names in a JSON body (`keys:a,b`).
pub fn request_key(body: &[u8]) -> Option<String> {
    key_of(&json_body(body)?)
}

/// A request body as JSON, when it is JSON and small enough to read.
fn json_body(body: &[u8]) -> Option<Value> {
    if body.is_empty() || body.len() > MAX_KEY_BODY {
        return None;
    }
    serde_json::from_slice(body).ok()
}

/// Whether a request body is GraphQL that only reads: every operation it runs is a query. A POST
/// carrying one is a read. A body naming its operation without the text (a persisted query) can't
/// be told apart from a mutation, so it isn't.
fn graphql_reads(v: &Value) -> bool {
    match v {
        Value::Object(o) => only_queries(o),
        Value::Array(a) => !a.is_empty() && a.len() <= MAX_KEY_NAMES && a.iter().all(|x| x.as_object().is_some_and(only_queries)),
        _ => false,
    }
}

/// One GraphQL request whose own text shows it only queries.
fn only_queries(o: &serde_json::Map<String, Value>) -> bool {
    let Some(doc) = own_document(o) else { return false };
    let name = o.get("operationName").and_then(Value::as_str).filter(|n| !n.is_empty());
    // A name that isn't among the operations read (past the reader's cap, say) could be anything.
    let runs: Vec<_> = doc.operations.iter().filter(|d| name.is_none() || d.name.as_deref() == name).collect();
    !doc.open_ended && !runs.is_empty() && runs.iter().all(|d| d.kind == "query")
}

fn own_document(o: &serde_json::Map<String, Value>) -> Option<craft_anything_js::api_ops::Document> {
    o.get("query").and_then(Value::as_str).and_then(|q| craft_anything_js::api_ops::read_document(q, true))
}

fn key_of(v: &Value) -> Option<String> {
    let one = |o: &serde_json::Map<String, Value>| -> Option<String> {
        if let Some(n) = o.get("operationName").and_then(Value::as_str).filter(|n| !n.is_empty()) {
            return Some(n.to_string());
        }
        let doc = craft_anything_js::api_ops::read_document(o.get("query")?.as_str()?, true)?;
        doc.operations.into_iter().find_map(|d| d.name)
    };
    match v {
        Value::Object(o) => match one(o) {
            Some(n) => Some(format!("op:{n}")),
            None if o.is_empty() => None,
            None => {
                // serde_json's maps are sorted, so the same names always give the same key.
                let names: Vec<&str> = o.keys().take(MAX_KEY_NAMES).map(String::as_str).filter(|k| !looks_secret(k)).collect();
                Some(format!("keys:{}", names.join(",")))
            }
        },
        // A batch of GraphQL operations.
        Value::Array(a) => {
            let names: Vec<String> = a.iter().take(MAX_KEY_NAMES).filter_map(|x| x.as_object().and_then(one)).collect();
            (!names.is_empty()).then(|| format!("op:{}", names.join(",")))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Path templates

/// A path segment that's an id by its form alone: a number, UUID, long hex, or a long mix of letters
/// and digits.
pub fn is_id_segment(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let (mut digits, mut letters, mut other) = (false, false, false);
    for b in s.bytes() {
        match b {
            b'0'..=b'9' => digits = true,
            b'a'..=b'z' | b'A'..=b'Z' => letters = true,
            b'-' | b'_' => {}
            _ => other = true,
        }
    }
    if other {
        return false;
    }
    (digits && !letters && s.bytes().all(|b| b.is_ascii_digit()))
        || is_uuid(s)
        || (s.len() >= MIN_HEX_ID && digits && s.bytes().all(|b| b.is_ascii_hexdigit()))
        || (s.len() >= MIN_TOKEN_ID && digits && letters)
}

/// Words a path has by design, under which a different word is a different resource.
fn is_namespace(seg: &str) -> bool {
    let l = seg.to_ascii_lowercase();
    matches!(
        l.as_str(),
        "" | "api"
            | "apis"
            | "rest"
            | "rpc"
            | "graphql"
            | "gql"
            | "trpc"
            | "public"
            | "internal"
            | "services"
            | "service"
            | "latest"
            | "stable"
            | "beta"
            | "alpha"
            | "{id}"
    ) || (l.len() >= 2 && l.starts_with('v') && l.bytes().skip(1).all(|b| b.is_ascii_digit() || b == b'.'))
}

/// The JSON shape of a body, for telling whether siblings answer alike: the names at its top, or in its
/// first item.
fn body_shape(text: Option<&str>) -> Option<String> {
    let v: Value = serde_json::from_str(text?).ok()?;
    let names = |o: &serde_json::Map<String, Value>| o.keys().take(64).map(String::as_str).collect::<Vec<_>>().join(",");
    match &v {
        Value::Object(o) if !o.is_empty() => Some(format!("o:{}", names(o))),
        Value::Array(a) => a.first().and_then(Value::as_object).filter(|o| !o.is_empty()).map(|o| format!("a:{}", names(o))),
        _ => None,
    }
}

/// One recorded request on its way to a route.
struct Seen<'a> {
    entry: &'a HarEntry,
    method: String,
    host: String,
    /// The path as kept (secret segments replaced).
    path: String,
    /// The path's segments, ids replaced: becomes the template.
    segs: Vec<String>,
    query_names: Vec<String>,
    query: String,
    /// Decoded body text and its shape.
    shape: Option<String>,
    /// The entry's index in the recording.
    order: u64,
    /// The request was a write.
    write: bool,
    /// What told the request apart: see [`request_key`].
    request_key: Option<String>,
}

/// Replace segments that differ between otherwise identical paths when the recording shows they're
/// values, not names. Two different values with the same text after them (`/users/a/posts`,
/// `/users/b/posts`) are enough; as the last segment it takes [`MIN_SIBLINGS`] values that all answer
/// in the same shape. The first segment, one under a namespace like `/api`, and anything with a dot
/// in it stay as they are, so `/users` and `/posts`, or `/api/users` and `/api/posts`, stay apart.
fn merge_siblings(seen: &mut [Seen<'_>]) {
    let longest = seen.iter().map(|s| s.segs.len()).max().unwrap_or(0).min(MAX_SEGMENTS);
    for pos in (1..longest).rev() {
        let mut groups: BTreeMap<(String, usize, String), Vec<usize>> = BTreeMap::new();
        for (i, s) in seen.iter().enumerate() {
            if s.segs.len() <= pos || s.segs.len() > MAX_SEGMENTS {
                continue;
            }
            let mut key = String::new();
            for (j, seg) in s.segs.iter().enumerate() {
                if j != pos {
                    key.push_str(seg);
                }
                key.push('/');
            }
            groups.entry((s.method.clone(), s.segs.len(), key)).or_default().push(i);
        }
        for ((_, len, _), members) in groups {
            let Some(first) = members.first().and_then(|i| seen.get(*i)) else { continue };
            if first.segs.get(pos - 1).is_none_or(|p| is_namespace(p)) {
                continue;
            }
            let values: BTreeSet<&str> = members.iter().filter_map(|i| seen.get(*i)?.segs.get(pos)).map(String::as_str).filter(|v| *v != ID).collect();
            if values.iter().any(|v| v.is_empty() || v.contains('.')) {
                continue;
            }
            let has_suffix = pos + 1 < len;
            let same_shape = || {
                let mut shapes = members.iter().filter_map(|i| seen.get(*i)).map(|s| s.shape.as_deref());
                let first = shapes.next().flatten();
                first.is_some() && shapes.all(|x| x == first)
            };
            let merge = if has_suffix { values.len() >= 2 } else { values.len() >= MIN_SIBLINGS && same_shape() };
            if merge {
                for i in &members {
                    if let Some(seg) = seen.get_mut(*i).and_then(|s| s.segs.get_mut(pos)) {
                        *seg = ID.to_string();
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Building

fn is_static(e: &HarEntry) -> bool {
    if let Some(t) = e.resource_type.as_deref().map(str::to_ascii_lowercase) {
        return matches!(t.as_str(), "document" | "script" | "stylesheet" | "image" | "font" | "media" | "manifest" | "texttrack" | "other-static");
    }
    let mime = e.response.mime.as_deref().unwrap_or_default().to_ascii_lowercase();
    ["text/html", "text/css", "javascript", "ecmascript", "font/", "image/", "audio/", "video/", "application/wasm"].iter().any(|m| mime.contains(m))
}

fn host_wanted(host: &str, only: &[String]) -> bool {
    only.is_empty() || only.iter().any(|h| h.eq_ignore_ascii_case(host) || host.split(':').next().is_some_and(|name| h.eq_ignore_ascii_case(name)))
}

/// Parse a HAR and describe it. A file that isn't a HAR is an error; odd entries in one are skipped.
pub fn build_from_bytes(json: &[u8], opts: &BuildOptions) -> Result<StandIn, HarError> {
    Ok(build(&har::parse(json, opts.max_entries)?, opts))
}

/// Describe a parsed recording.
pub fn build(har: &Har, opts: &BuildOptions) -> StandIn {
    let mut scrub = Scrub::new(&opts.hidden_values);
    let mut skipped = Skipped::default();
    let mut seen: Vec<Seen<'_>> = Vec::new();
    for (order, e) in (0u64..).zip(&har.entries) {
        // What's dropped from a request counts even if the request goes no further.
        for (k, _) in &e.request.headers {
            if is_secret_header(k) {
                scrub.note_header(k);
            }
        }
        scrub.dropped.cookies = scrub.dropped.cookies.saturating_add(e.request.cookie_names.len() as u64).saturating_add(e.response.cookie_names.len() as u64);
        let (scheme, host, path, pairs, user_info) = split_url(&e.request.url);
        if !matches!(scheme.as_str(), "http" | "https") || host.is_empty() {
            skipped.not_web = skipped.not_web.saturating_add(1);
            continue;
        }
        if !(200..=599).contains(&e.response.status) {
            skipped.no_response = skipped.no_response.saturating_add(1);
            continue;
        }
        if !host_wanted(&host, &opts.only_hosts) {
            skipped.other_hosts = skipped.other_hosts.saturating_add(1);
            continue;
        }
        if !opts.include_static && is_static(e) {
            skipped.static_files = skipped.static_files.saturating_add(1);
            continue;
        }
        let method = e.request.method.to_ascii_uppercase();
        if method.is_empty() || method.len() > 32 || !method.bytes().all(|b| b.is_ascii_alphabetic()) {
            skipped.no_response = skipped.no_response.saturating_add(1);
            continue;
        }
        if user_info {
            scrub.dropped.url_user_info = scrub.dropped.url_user_info.saturating_add(1);
        }
        let host = if scrub.holds_hidden(&host) { DROPPED.to_string() } else { host };
        let path = scrub.path(&path, true);
        let (query_names, query) = scrub.query(&pairs, true);
        let segs: Vec<String> = path.split('/').map(|s| if is_id_segment(s) { ID.to_string() } else { s.to_string() }).collect();
        let shape = if (200..300).contains(&e.response.status)
            && e.response.encoding.is_none()
            && e.response.text.as_ref().is_some_and(|t| t.len() <= opts.max_body_bytes)
        {
            body_shape(e.response.text.as_deref())
        } else {
            None
        };
        // The request body is read once, for both what tells it apart and whether it writes.
        let json = e.request.post.as_ref().and_then(|p| p.text.as_deref()).and_then(|t| json_body(t.as_bytes()));
        let write = is_write(&method, json.as_ref());
        let request_key = json.as_ref().and_then(key_of).filter(|k| !scrub.holds_hidden(k));
        seen.push(Seen { entry: e, method, host, path, segs, query_names, query, shape, order, write, request_key });
    }
    merge_siblings(&mut seen);

    let mut routes: BTreeMap<(String, String), StandInRoute> = BTreeMap::new();
    let mut hosts: BTreeSet<String> = BTreeSet::new();
    let mut body_bytes = 0usize;
    // The place of the last accepted write kept for each template: a read answered the same either
    // side of a related one is kept twice, since the two say what was there before and after.
    let mut kept_writes: BTreeMap<String, u64> = BTreeMap::new();
    // The latest of them, so a read repeated long after every write needn't look through them all.
    let mut latest_kept_write: Option<u64> = None;
    for s in &seen {
        let template = s.segs.join("/");
        let key = (template.clone(), s.method.clone());
        if !routes.contains_key(&key) && routes.len() >= opts.max_routes {
            skipped.beyond_route_limit = skipped.beyond_route_limit.saturating_add(1);
            continue;
        }
        if hosts.len() < MAX_HOSTS {
            hosts.insert(s.host.clone());
        }
        let route = routes.entry(key).or_insert_with(|| StandInRoute { method: s.method.clone(), template, ..Default::default() });
        route.requests = route.requests.saturating_add(1);
        push_unique_sorted(&mut route.hosts, &s.host);
        for n in &s.query_names {
            push_unique_sorted(&mut route.query_names, n);
        }
        let full = route.samples.len() >= opts.max_samples_per_route;
        let replace = if full {
            route
                .samples
                .iter()
                .position(|sample| sample.status == 304 && sample.path == s.path && sample.query == s.query && sample.request_key == s.request_key)
        } else {
            None
        };
        if full && replace.is_none() {
            route.samples_left_out = route.samples_left_out.saturating_add(1);
            // Still read its headers, so what's dropped is counted the same however many are kept.
            let _ = scrub.headers(&s.entry.response.headers);
            note_status(route, u16::try_from(s.entry.response.status).unwrap_or(500), s.entry.response.mime.as_deref(), &mut scrub);
            continue;
        }
        let mut sample = make_sample(s, opts, &mut scrub);
        // A request polled over and over gives the same answer many times; one is enough. Each write
        // is its own step through the recording, so writes are all kept.
        let repeats_last = |sample: &Sample| {
            route.samples.last().is_some_and(|last| {
                let written_since = || {
                    latest_kept_write.is_some_and(|w| after(w, last.order))
                        && kept_writes.iter().any(|(t, o)| after(*o, last.order) && related(t, &route.template))
                };
                !last.write && same_sample(last, sample) && !written_since()
            })
        };
        let mut repeat = repeats_last(&sample);
        if !repeat {
            let size = sample.text.as_ref().map_or(0, String::len).saturating_add(sample.base64.as_ref().map_or(0, String::len));
            if let Some(total) = body_bytes.checked_add(size).filter(|total| *total <= opts.max_total_body_bytes) {
                body_bytes = total;
            } else {
                sample.text = None;
                sample.base64 = None;
                sample.body_left_out = Some(BodyLeftOut::FileLimit);
                // A 304 without its cached body must remain unavailable to a fresh client.
                if sample.was_not_modified {
                    sample.status = 304;
                    sample.was_not_modified = false;
                }
                // Leaving a body out can make it identical to the previous omitted answer.
                repeat = repeats_last(&sample);
            }
        }
        note_status(route, sample.status, sample.content_type.as_deref(), &mut scrub);
        if replace.is_some() && (sample.status == 304 || sample.body_left_out.is_some_and(|reason| reason != BodyLeftOut::NotRecorded)) {
            // A missing cached body or a refused body cannot improve the stored evidence. Neither was charged.
            route.samples_left_out = route.samples_left_out.saturating_add(1);
            continue;
        }
        if !repeat {
            if let Some(at) = replace {
                // The position came from this unchanged sample list. Remove only an unavailable
                // answer to this exact request; append the replacement in recording order.
                route.samples.remove(at);
                route.samples_left_out = route.samples_left_out.saturating_add(1);
            }
            if accepted(&sample) {
                kept_writes.insert(route.template.clone(), s.order);
                latest_kept_write = latest_kept_write.max(Some(s.order));
            }
            route.samples.push(sample);
        }
    }

    StandIn {
        version: STANDIN_VERSION,
        source: StandInSource {
            file_name: opts.source_name.clone().filter(|n| !scrub.holds_hidden(n)),
            sha256: opts.source_sha256.clone().filter(|d| !opts.hidden_values.contains(d)),
            entries_read: har.entries.len() as u64,
            entries_declared: har.declared_entries as u64,
            cut_short: har.truncated,
        },
        hosts: hosts.into_iter().collect(),
        routes: routes.into_values().collect(),
        dropped: scrub.dropped,
        skipped,
    }
}

fn same_sample(left: &Sample, right: &Sample) -> bool {
    // Keep this exhaustive so a new response field cannot silently disappear from repeat checks.
    // Where each was recorded doesn't make them different answers.
    let Sample { path, query, request_key, status, content_type, headers, text, base64, body_left_out, was_not_modified, order: _, write } = left;
    path == &right.path
        && query == &right.query
        && request_key == &right.request_key
        && status == &right.status
        && content_type == &right.content_type
        && text == &right.text
        && base64 == &right.base64
        && body_left_out == &right.body_left_out
        && was_not_modified == &right.was_not_modified
        && write == &right.write
        && same_headers(headers, &right.headers)
}

fn same_headers(left: &[(String, String)], right: &[(String, String)]) -> bool {
    // Header names are case-insensitive; different names may move, but values for one name
    // must keep their order. Capture caps this list, and the stored representation stays intact.
    left == right
        || (left.len() == right.len()
            && left.iter().all(|(name, _)| {
                left.iter()
                    .filter(|(key, _)| key.eq_ignore_ascii_case(name))
                    .map(|(_, value)| value)
                    .eq(right.iter().filter(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value))
            }))
}

fn push_unique_sorted(list: &mut Vec<String>, value: &str) {
    if list.len() < MAX_LISTED
        && let Err(i) = list.binary_search_by(|x| x.as_str().cmp(value))
    {
        list.insert(i, value.to_string());
    }
}

fn clean_mime(mime: Option<&str>, scrub: &mut Scrub) -> Option<String> {
    let m = mime?.trim();
    if m.is_empty() || m.len() > 200 || m.bytes().any(|b| b.is_ascii_control()) || scrub.holds_hidden(m) {
        return None;
    }
    Some(m.to_string())
}

fn note_status(route: &mut StandInRoute, status: u16, mime: Option<&str>, scrub: &mut Scrub) {
    if let Err(i) = route.statuses.binary_search(&status)
        && route.statuses.len() < MAX_LISTED
    {
        route.statuses.insert(i, status);
    }
    if let Some(m) = clean_mime(mime, scrub) {
        push_unique_sorted(&mut route.content_types, &m);
    }
}

fn make_sample(s: &Seen<'_>, opts: &BuildOptions, scrub: &mut Scrub) -> Sample {
    let r = &s.entry.response;
    let mut sample = Sample {
        path: s.path.clone(),
        query: s.query.clone(),
        request_key: s.request_key.clone(),
        status: u16::try_from(r.status).unwrap_or(500),
        content_type: clean_mime(r.mime.as_deref(), scrub),
        headers: scrub.headers(&r.headers),
        order: Some(s.order),
        write: s.write,
        ..Default::default()
    };
    if let Some(url) = r.redirect_url.as_deref()
        && !sample.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("location"))
        && !url.bytes().any(|b| b.is_ascii_control())
    {
        let clean = scrub.location(url);
        if !scrub.holds_hidden(&clean) {
            sample.headers.push(("Location".into(), clean));
        }
    }
    let Some(raw) = r.text.as_deref().filter(|t| !t.is_empty()) else {
        sample.body_left_out = Some(BodyLeftOut::NotRecorded);
        return sample;
    };
    if raw.len() > opts.max_body_bytes.saturating_mul(2) {
        sample.body_left_out = Some(BodyLeftOut::TooLarge);
        return sample;
    }
    let is_base64 = r.encoding.as_deref().is_some_and(|e| e.eq_ignore_ascii_case("base64"));
    let (text, binary): (Option<String>, Option<&str>) = if is_base64 {
        let Some(bytes) = base64_decode(raw) else {
            sample.body_left_out = Some(BodyLeftOut::Undecodable);
            return sample;
        };
        if bytes.len() > opts.max_body_bytes {
            sample.body_left_out = Some(BodyLeftOut::TooLarge);
            return sample;
        }
        match String::from_utf8(bytes) {
            Ok(t) => (Some(t), None),
            Err(e) => {
                // Can't be edited in place, so a named secret or a key in it costs the whole body.
                let bytes = e.as_bytes();
                let contains_ci = |needle: &[u8]| !needle.is_empty() && bytes.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle));
                let has_pem = bytes.windows(b"PRIVATE KEY-----".len()).any(|w| w == b"PRIVATE KEY-----");
                if scrub.needles.iter().any(|n| contains_ci(n.as_bytes())) || has_pem {
                    scrub.dropped.body_values = scrub.dropped.body_values.saturating_add(1);
                    sample.body_left_out = Some(BodyLeftOut::HeldSecret);
                    return sample;
                }
                (None, Some(raw))
            }
        }
    } else {
        (Some(raw.to_string()), None)
    };
    if text.as_ref().is_some_and(|t| t.len() > opts.max_body_bytes) {
        sample.body_left_out = Some(BodyLeftOut::TooLarge);
        return sample;
    }
    if let Some(t) = text {
        if t.contains("PRIVATE KEY-----") {
            scrub.dropped.body_values = scrub.dropped.body_values.saturating_add(1);
            sample.body_left_out = Some(BodyLeftOut::HeldSecret);
            return sample;
        }
        let clean = clean_text(t, sample.content_type.as_deref(), scrub);
        if clean.len() > opts.max_body_bytes {
            sample.body_left_out = Some(BodyLeftOut::TooLarge);
            return sample;
        }
        sample.text = Some(clean);
    } else {
        sample.base64 = binary.map(String::from);
    }
    // The answer a 304 stands for: the body the browser had cached.
    if sample.status == 304 {
        sample.status = 200;
        sample.was_not_modified = true;
    }
    sample
}

/// A response body with its secrets out. JSON is cleaned by key and value and only rewritten if
/// something changed, so an untouched body keeps its exact bytes.
fn clean_text(text: String, mime: Option<&str>, scrub: &mut Scrub) -> String {
    let looks_json = mime.is_some_and(|m| m.to_ascii_lowercase().contains("json")) || text.trim_start().starts_with(['{', '[']);
    if looks_json && let Ok(mut v) = serde_json::from_str::<Value>(&text) {
        let before = v.clone();
        scrub.json(&mut v);
        let out = if v == before { text } else { serde_json::to_string(&v).unwrap_or_default() };
        return scrub.hide(out);
    }
    let lower = mime.map(str::to_ascii_lowercase).unwrap_or_default();
    let is_code = ["javascript", "ecmascript", "text/css", "text/html", "xhtml"].iter().any(|m| lower.contains(m));
    let cleaned = if is_code { scrub.code(&text) } else { scrub.tokens(&text) };
    scrub.hide(cleaned)
}

// ---------------------------------------------------------------------------------------------
// Answering

/// How a request was matched, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    PathAndQuery,
    Path,
    Template,
}

/// A recorded answer picked for a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub route: String,
    pub how: MatchKind,
    pub status: u16,
    pub content_type: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub body_left_out: Option<BodyLeftOut>,
    /// Which of the answers that fit this was (1-based), and how many there are.
    pub turn: usize,
    pub of: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoMatchReason {
    /// Nothing with this path shape was recorded.
    PathNotRecorded,
    /// The path was recorded, but only with other methods.
    MethodNotRecorded,
    /// The URL was recorded, but not this GraphQL operation.
    OperationNotRecorded,
    /// The matching responses need a cached body that is unavailable in the recording.
    CachedBodyUnavailable,
    /// A write the recording has no answer of its own for: past the recorded writes of its kind,
    /// a named operation where only another one's (its name hidden) was recorded, or a mutation
    /// sent as text where only reads were.
    WriteNotRecorded,
}

/// A route close to what was asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NearbyRoute {
    pub method: String,
    pub template: String,
}

/// Nothing recorded answers this request, and what comes closest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoMatch {
    pub reason: NoMatchReason,
    pub nearest: Vec<NearbyRoute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Found(Box<Found>),
    NoMatch(NoMatch),
}

/// One client's way through a stand-in: how far each set of matching answers has got, so a
/// repeated request goes through them in order, and where in the recording its writes have taken
/// it. Grows only with the samples in the stand-in, never with the requests made.
#[derive(Debug, Clone, Default)]
pub struct Turns {
    turns: BTreeMap<(usize, usize, MatchKind), usize>,
    /// By route and recorded request key: the place of the last write of that kind replayed,
    /// accepted or not. The next one answers from after it.
    written: BTreeMap<(usize, Option<String>), u64>,
    /// By route: the place of the last accepted write replayed. Reads of related paths answer from
    /// after it.
    accepted: BTreeMap<usize, u64>,
    /// In a file without places: how many writes each set of answers has given, apart from the
    /// reads' turns, so polling a read doesn't use up a write's answers.
    untimed_writes: BTreeMap<(usize, usize, MatchKind), usize>,
}

impl Turns {
    /// How far the writes replayed so far have taken this client: the entry index of the latest
    /// accepted one. `None` until one is replayed, and always for a file without places.
    pub fn position(&self) -> Option<u64> {
        self.accepted.values().max().copied()
    }
}

/// A write the original accepted: answered without an error. A 304 still kept as one has no body
/// to replay, so no client can get past it.
fn accepted(s: &Sample) -> bool {
    s.write && (200..400).contains(&s.status) && s.status != 304
}

/// Whether a write to one path template can change what a read of the other returns: one is the
/// other, or lies under it (`/notes` and `/notes/{id}`, `/cart` and `/cart/items`). Templates are
/// worked out for each method apart, so `{id}` stands for any segment (`/notes/b` and `/notes/{id}`).
fn related(a: &str, b: &str) -> bool {
    template_segments(a).zip(template_segments(b)).all(|(x, y)| x == y || x == ID || y == ID)
}

/// A template's segments without the slashes at either end. One inside stays, as an empty segment:
/// `/api//notes` is answered apart from `/api/notes`.
fn template_segments(t: &str) -> impl Iterator<Item = &str> {
    let t = t.strip_prefix('/').unwrap_or(t);
    let t = t.strip_suffix('/').unwrap_or(t);
    (!t.is_empty()).then(|| t.split('/')).into_iter().flatten()
}

/// A request that may change what later reads return: a method that isn't a read, unless it's a
/// POST whose JSON body is GraphQL that only queries. `method` is upper case.
fn is_write(method: &str, json: Option<&Value>) -> bool {
    !READ_METHODS.contains(&method) && !(method == "POST" && json.is_some_and(graphql_reads))
}

/// Whether [`answer`] takes this request as a write, so a server can count the ones it refuses.
/// A POST too large to read could be a query, so it's counted as a request with no answer.
pub fn is_write_request(method: &str, body: &[u8]) -> bool {
    let method = method.to_ascii_uppercase();
    method != "HEAD" && !(method == "POST" && body.len() > MAX_KEY_BODY) && is_write(&method, json_body(body).as_ref())
}

/// Whether a body carries GraphQL text of its own that doesn't only query: then it's known to be
/// a write, and a recorded read isn't its answer. Each request of a batch is judged by its own
/// text, so one named only by its operation doesn't make the batch a write.
fn writes_by_its_own_text(v: &Value) -> bool {
    let one = |o: &serde_json::Map<String, Value>| own_document(o).is_some() && !only_queries(o);
    match v {
        Value::Object(o) => one(o),
        Value::Array(a) => a.iter().any(|x| x.as_object().is_some_and(one)),
        _ => false,
    }
}

/// `order` comes after `place`, where no place is before the start.
fn after(order: u64, place: Option<u64>) -> bool {
    place.is_none_or(|p| order > p)
}

/// Narrow the fitting answers of route `ri` to the ones that apply at this client's place in the
/// recording, and say whether they're writes to step through. A write request takes the first
/// recorded write after the last one replayed of that write's own kind; past them, or for a named
/// GraphQL operation none of them recorded, there's none (`None`): a stand-in never answers a write
/// nobody recorded as if it worked. Otherwise the
/// recorded reads apply: those between the last accepted write replayed on a related path and the
/// next related one recorded, or else the earliest from after that place (which shows the client's
/// own write), or else the newest from before it.
/// When any answer has no place (a file from before version 2), all of them apply, and [`answer`]
/// gives a write each of them once.
fn on_timeline(
    standin: &StandIn,
    ri: usize,
    route: &StandInRoute,
    fit: Vec<usize>,
    turns: &Turns,
    write: bool,
    key: Option<&str>,
) -> Option<(Vec<usize>, bool)> {
    let placed: Option<Vec<(usize, u64, bool)>> = fit.iter().map(|i| route.samples.get(*i).and_then(|s| Some((*i, s.order?, s.write)))).collect();
    let Some(placed) = placed.filter(|p| !p.is_empty()) else { return Some((fit, false)) };
    let first_by = |pool: &[(usize, u64, bool)], keep: &dyn Fn(u64) -> bool| pool.iter().filter(|x| keep(x.1)).min_by_key(|x| x.1).map(|x| x.0);
    let last_by = |pool: &[(usize, u64, bool)], keep: &dyn Fn(u64) -> bool| pool.iter().filter(|x| keep(x.1)).max_by_key(|x| x.1).map(|x| x.0);
    let (writes, reads): (Vec<_>, Vec<_>) = placed.iter().partition(|x| x.2);
    if write && !writes.is_empty() {
        let kind = |x: &(usize, u64, bool)| route.samples.get(x.0).and_then(|s| s.request_key.clone());
        // Another operation's answer (its name hidden from the file, say) isn't this one's.
        if key.is_some_and(|k| k.starts_with("op:")) && !writes.iter().any(|x| kind(x).as_deref() == key) {
            return None;
        }
        let next = writes.iter().filter(|x| after(x.1, turns.written.get(&(ri, kind(x))).copied())).min_by_key(|x| x.1).map(|x| x.0);
        return next.map(|i| (vec![i], true));
    }
    // A request named only by its operation can't be told from a write, but the recording can.
    let pool = if reads.is_empty() { placed } else { reads };
    let near: Vec<(usize, &StandInRoute)> = standin.routes.iter().enumerate().filter(|(_, r)| related(&r.template, &route.template)).collect();
    let clock = near.iter().filter_map(|(j, _)| turns.accepted.get(j)).max().copied();
    let next_write = near.iter().flat_map(|(_, r)| &r.samples).filter(|s| accepted(s)).filter_map(|s| s.order).filter(|o| after(*o, clock)).min();
    let before_next = |o: u64| next_write.is_none_or(|n| o < n);
    let mut now: Vec<(usize, u64, bool)> = pool.iter().copied().filter(|x| after(x.1, clock) && before_next(x.1)).collect();
    if now.is_empty() {
        let fallback = first_by(&pool, &|o| after(o, clock)).or_else(|| last_by(&pool, &|o| !after(o, clock)));
        return Some((fallback.into_iter().collect(), false));
    }
    now.sort_by_key(|x| x.1);
    Some((now.into_iter().map(|x| x.0).collect(), false))
}

fn template_fits(template: &str, path: &str) -> bool {
    let mut t = template.split('/');
    let mut p = path.split('/');
    loop {
        match (t.next(), p.next()) {
            (None, None) => return true,
            (Some(a), Some(b)) => {
                if a == ID {
                    if b.is_empty() {
                        return false;
                    }
                } else if a != b {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

/// More literal segments is more specific: `/users/search` before `/users/{id}`.
fn literal_segments(template: &str) -> usize {
    template.split('/').filter(|s| *s != ID).count()
}

/// Which samples fit this request's body. A named GraphQL operation must be one that was recorded if
/// the URL recorded any, since another operation's answer would be the wrong data. Otherwise the
/// same key is preferred, and any sample will do.
fn fitting(all: Vec<usize>, route: &StandInRoute, key: Option<&str>) -> Vec<usize> {
    let key_of = |i: &usize| route.samples.get(*i).and_then(|s| s.request_key.as_deref());
    let same: Vec<usize> = all.iter().copied().filter(|i| key.is_some() && key_of(i) == key).collect();
    if !same.is_empty() {
        return same;
    }
    if key.is_some_and(|k| k.starts_with("op:")) && all.iter().any(|i| key_of(i).is_some_and(|k| k.starts_with("op:"))) {
        return Vec::new();
    }
    all
}

/// The recorded answer for a request, trying the exact path and query, then the exact path, then the
/// path template. Of the answers that fit, those that apply at the client's place in the recording
/// are used (see [`on_timeline`]); a repeated request goes through them in recording order, then
/// starts again. A replayed write the original accepted moves `turns` on to its place, so later reads
/// of related paths answer from after it. This never changes the stand-in. `path` and `query` are as the client sent them.
pub fn answer(standin: &StandIn, turns: &mut Turns, method: &str, path: &str, query: &str, body: &[u8]) -> Answer {
    let method = method.to_ascii_uppercase();
    // HEAD asks what GET would say; the server leaves out the body.
    let look_for = if method == "HEAD" { "GET" } else { method.as_str() };
    // A request is brought to the same form as the recording, so a dropped value still compares equal.
    let mut scrub = Scrub::new(&[]);
    let clean_path = scrub.path(path.split(['?', '#']).next().unwrap_or_default(), false);
    let clean_query = scrub.query_text(query, false);
    let json = json_body(body);
    let key = json.as_ref().and_then(key_of);
    let key = key.as_deref();
    let write = is_write(look_for, json.as_ref());
    // A request named only by its GraphQL operation may be a read the recording knows.
    let known_write = write && (!key.is_some_and(|k| k.starts_with("op:")) || json.as_ref().is_some_and(writes_by_its_own_text));

    let mut operation_missing = false;
    let mut consider = |route: &StandInRoute, candidates: Vec<usize>| -> Option<Vec<usize>> {
        if candidates.is_empty() {
            return None;
        }
        let fit = fitting(candidates, route, key);
        if fit.is_empty() {
            operation_missing = true;
            return None;
        }
        Some(fit)
    };

    let same_method = || standin.routes.iter().enumerate().filter(|(_, r)| r.method == look_for);
    let mut picked: Option<(usize, MatchKind, Vec<usize>)> = None;
    for (ri, route) in same_method() {
        let exact: Vec<usize> = route
            .samples
            .iter()
            .enumerate()
            .filter(|(_, s)| s.path == clean_path && (s.query == clean_query || scrub.query_text(&s.query, true) == clean_query))
            .map(|(i, _)| i)
            .collect();
        if let Some(fit) = consider(route, exact) {
            picked = Some((ri, MatchKind::PathAndQuery, fit));
            break;
        }
    }
    if picked.is_none() {
        for (ri, route) in same_method() {
            let on_path: Vec<usize> = route.samples.iter().enumerate().filter(|(_, s)| s.path == clean_path).map(|(i, _)| i).collect();
            if let Some(fit) = consider(route, on_path) {
                picked = Some((ri, MatchKind::Path, fit));
                break;
            }
        }
    }
    if picked.is_none() {
        let mut fits: Vec<(usize, &StandInRoute)> = same_method().filter(|(_, r)| template_fits(&r.template, &clean_path)).collect();
        fits.sort_by_key(|(_, r)| std::cmp::Reverse(literal_segments(&r.template)));
        for (ri, route) in fits {
            if let Some(fit) = consider(route, (0..route.samples.len()).collect()) {
                picked = Some((ri, MatchKind::Template, fit));
                break;
            }
        }
    }

    let Some((ri, how, mut fit)) = picked else {
        let path_known = standin.routes.iter().any(|r| template_fits(&r.template, &clean_path) || r.samples.iter().any(|s| s.path == clean_path));
        let reason = if operation_missing {
            NoMatchReason::OperationNotRecorded
        } else if path_known {
            NoMatchReason::MethodNotRecorded
        } else {
            NoMatchReason::PathNotRecorded
        };
        return Answer::NoMatch(NoMatch { reason, nearest: nearest(standin, look_for, &clean_path) });
    };
    let Some(route) = standin.routes.get(ri) else {
        return Answer::NoMatch(NoMatch { reason: NoMatchReason::PathNotRecorded, nearest: Vec::new() });
    };
    // Usable cached 304 bodies become 200 at build time. Keep unavailable samples in the file
    // as evidence, but a fresh client cannot use them. Filter only after choosing the best match:
    // another query, resource or GraphQL operation is not a substitute for its missing body.
    fit.retain(|i| route.samples.get(*i).is_some_and(|s| s.status != 304));
    if fit.is_empty() {
        return Answer::NoMatch(NoMatch { reason: NoMatchReason::CachedBodyUnavailable, nearest: nearest(standin, look_for, &clean_path) });
    }
    // On a timeline a lone answer takes no turn, so a set that starts with it later starts at the
    // beginning; without positions every answer takes one, as before positions were recorded.
    let timed = fit.iter().all(|i| route.samples.get(*i).is_some_and(|s| s.order.is_some()));
    // A mutation sent as text where only reads were recorded: none of them is its answer.
    if timed && write && json.as_ref().is_some_and(writes_by_its_own_text) && fit.iter().all(|i| route.samples.get(*i).is_some_and(|s| !s.write)) {
        return Answer::NoMatch(NoMatch { reason: NoMatchReason::WriteNotRecorded, nearest: Vec::new() });
    }
    let Some((fit, stepping)) = on_timeline(standin, ri, route, fit, turns, write, key) else {
        return Answer::NoMatch(NoMatch { reason: NoMatchReason::WriteNotRecorded, nearest: Vec::new() });
    };
    let at = match fit.first() {
        Some(first) if fit.len() > 1 || !timed => {
            // A file from before version 2 can't tell its writes from its reads: a write takes each
            // recorded answer once, then gets none, like a write past the recorded ones on a timeline.
            let untimed_write = !timed && known_write;
            let counter = if untimed_write { &mut turns.untimed_writes } else { &mut turns.turns }.entry((ri, *first, how)).or_insert(0);
            if untimed_write && *counter >= fit.len() {
                return Answer::NoMatch(NoMatch { reason: NoMatchReason::WriteNotRecorded, nearest: Vec::new() });
            }
            let at = counter.checked_rem(fit.len()).unwrap_or(0);
            *counter = counter.saturating_add(1);
            at
        }
        _ => 0,
    };
    let Some(sample) = fit.get(at).and_then(|i| route.samples.get(*i)) else {
        return Answer::NoMatch(NoMatch { reason: NoMatchReason::PathNotRecorded, nearest: Vec::new() });
    };
    if let (true, Some(order)) = (stepping, sample.order) {
        let written = turns.written.entry((ri, sample.request_key.clone())).or_insert(order);
        *written = (*written).max(order);
        if accepted(sample) {
            let at = turns.accepted.entry(ri).or_insert(order);
            *at = (*at).max(order);
        }
    }
    let body = match (&sample.text, &sample.base64) {
        (Some(t), _) => t.clone().into_bytes(),
        (None, Some(b)) => base64_decode(b).unwrap_or_default(),
        (None, None) => Vec::new(),
    };
    Answer::Found(Box::new(Found {
        route: format!("{} {}", route.method, route.template),
        how,
        status: sample.status,
        content_type: sample.content_type.clone(),
        headers: sample.headers.clone(),
        body,
        body_left_out: sample.body_left_out,
        turn: at + 1,
        of: fit.len(),
    }))
}

/// The routes closest to a path: most segments the same in the same place, same method first.
pub fn nearest(standin: &StandIn, method: &str, path: &str) -> Vec<NearbyRoute> {
    let wanted: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let mut scored: Vec<(i64, &StandInRoute)> = standin
        .routes
        .iter()
        .map(|r| {
            let segs: Vec<&str> = r.template.split('/').filter(|s| !s.is_empty()).collect();
            let same = segs.iter().zip(&wanted).filter(|(a, b)| a == b).count() as i64;
            let id_fit = segs.iter().zip(&wanted).filter(|(a, _)| **a == ID).count() as i64;
            let last_same = i64::from(segs.last().is_some_and(|l| Some(l) == wanted.last()));
            let apart = (segs.len() as i64 - wanted.len() as i64).abs();
            (same * 4 + id_fit + last_same * 2 - apart + i64::from(r.method == method) * 3, r)
        })
        .collect();
    scored.sort_by(|(a, x), (b, y)| b.cmp(a).then_with(|| x.template.cmp(&y.template)).then_with(|| x.method.cmp(&y.method)));
    scored.into_iter().take(MAX_NEAREST).map(|(_, r)| NearbyRoute { method: r.method.clone(), template: r.template.clone() }).collect()
}

#[cfg(test)]
mod tests;
