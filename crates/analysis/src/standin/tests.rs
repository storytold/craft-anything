use super::*;
use serde_json::json;

/// One entry: method, URL, status, JSON or text body.
fn entry(method: &str, url: &str, status: i64, body: &str) -> Value {
    json!({
        "request": {"method": method, "url": url, "headers": []},
        "response": {"status": status, "headers": [], "content": {"mimeType": "application/json", "text": body}}
    })
}

fn har_of(entries: Vec<Value>) -> Vec<u8> {
    serde_json::to_vec(&json!({"log": {"version": "1.2", "entries": entries}})).unwrap()
}

fn built(entries: Vec<Value>) -> StandIn {
    build_from_bytes(&har_of(entries), &BuildOptions::default()).unwrap()
}

fn templates(s: &StandIn) -> Vec<String> {
    s.routes.iter().map(|r| format!("{} {}", r.method, r.template)).collect()
}

fn found(a: Answer) -> Found {
    match a {
        Answer::Found(f) => *f,
        Answer::NoMatch(n) => panic!("no match: {n:?}"),
    }
}

fn missing(a: Answer) -> NoMatch {
    match a {
        Answer::NoMatch(n) => n,
        Answer::Found(f) => panic!("found: {f:?}"),
    }
}

fn get(s: &StandIn, t: &mut Turns, path: &str, query: &str) -> Answer {
    answer(s, t, "GET", path, query, b"")
}

fn body(f: &Found) -> String {
    String::from_utf8_lossy(&f.body).into_owned()
}

const A: &str = "https://api.example.test";

// ---- templates ----

#[test]
fn numeric_ids_uuids_and_long_hex_become_id() {
    let s = built(vec![
        entry("GET", &format!("{A}/users/42"), 200, "{}"),
        entry("GET", &format!("{A}/users/43"), 200, "{}"),
        entry("GET", &format!("{A}/orders/550e8400-e29b-41d4-a716-446655440000/items"), 200, "{}"),
        entry("GET", &format!("{A}/blobs/0123456789abcdef0123"), 200, "{}"),
        entry("GET", &format!("{A}/files/a1b2c3d4e5f6g7h8i9j0k1"), 200, "{}"),
    ]);
    assert_eq!(templates(&s), ["GET /blobs/{id}", "GET /files/{id}", "GET /orders/{id}/items", "GET /users/{id}"]);
    assert_eq!(s.routes[3].requests, 2);
}

#[test]
fn id_shapes() {
    for id in ["1", "0042", "550e8400-e29b-41d4-a716-446655440000", "0123456789abcdef", "a1b2c3d4e5f6g7h8i9j0"] {
        assert!(is_id_segment(id), "{id}");
    }
    for word in ["", "users", "v1", "facade", "decade", "deadbeef", "abcdefabcdefabcdefabcdef", "user-profile-settings-page", "item.json", "2fa", "oauth2"] {
        assert!(!is_id_segment(word), "{word}");
    }
}

#[test]
fn different_collections_are_not_merged() {
    let shape = r#"[{"id":1}]"#;
    let s = built(vec![
        entry("GET", &format!("{A}/users"), 200, shape),
        entry("GET", &format!("{A}/posts"), 200, shape),
        entry("GET", &format!("{A}/tags"), 200, shape),
        entry("GET", &format!("{A}/api/users"), 200, shape),
        entry("GET", &format!("{A}/api/posts"), 200, shape),
        entry("GET", &format!("{A}/api/tags"), 200, shape),
        entry("GET", &format!("{A}/v1/users"), 200, shape),
        entry("GET", &format!("{A}/v1/posts"), 200, shape),
        entry("GET", &format!("{A}/v1/tags"), 200, shape),
        entry("GET", &format!("{A}/api/users/list"), 200, shape),
        entry("GET", &format!("{A}/api/posts/list"), 200, shape),
    ]);
    assert_eq!(s.routes.len(), 11, "{:?}", templates(&s));
    assert!(s.routes.iter().all(|r| !r.template.contains(ID)), "{:?}", templates(&s));
}

#[test]
fn word_like_values_merge_when_the_recording_shows_they_vary() {
    let user = |name: &str| entry("GET", &format!("{A}/users/{name}"), 200, r#"{"name":"x","bio":"y"}"#);
    let s = built(vec![user("alice"), user("bob"), user("carol")]);
    assert_eq!(templates(&s), ["GET /users/{id}"]);
    // Text after them is stronger evidence: two are enough.
    let s = built(vec![entry("GET", &format!("{A}/users/alice/posts"), 200, "[]"), entry("GET", &format!("{A}/users/bob/posts"), 200, "[]")]);
    assert_eq!(templates(&s), ["GET /users/{id}/posts"]);
}

#[test]
fn siblings_that_answer_differently_stay_apart() {
    let s = built(vec![
        entry("GET", &format!("{A}/shop/cart"), 200, r#"{"items":[]}"#),
        entry("GET", &format!("{A}/shop/orders"), 200, r#"{"orders":[]}"#),
        entry("GET", &format!("{A}/shop/account"), 200, r#"{"email":"x"}"#),
    ]);
    assert_eq!(s.routes.len(), 3, "{:?}", templates(&s));
    // Two values aren't enough without text after them, and no JSON says nothing about shape.
    let s = built(vec![entry("GET", &format!("{A}/shop/a"), 200, r#"{"x":1}"#), entry("GET", &format!("{A}/shop/b"), 200, r#"{"x":1}"#)]);
    assert_eq!(s.routes.len(), 2);
    let s = built(vec![entry("GET", &format!("{A}/p/a"), 200, "x"), entry("GET", &format!("{A}/p/b"), 200, "x"), entry("GET", &format!("{A}/p/c"), 200, "x")]);
    assert_eq!(s.routes.len(), 3);
}

#[test]
fn procedure_names_and_file_names_are_not_ids() {
    let s = built(vec![
        entry("GET", &format!("{A}/trpc/post.list"), 200, r#"{"a":1}"#),
        entry("GET", &format!("{A}/trpc/post.get"), 200, r#"{"a":1}"#),
        entry("GET", &format!("{A}/trpc/user.get"), 200, r#"{"a":1}"#),
        entry("GET", &format!("{A}/data/a.json"), 200, r#"{"a":1}"#),
        entry("GET", &format!("{A}/data/b.json"), 200, r#"{"a":1}"#),
        entry("GET", &format!("{A}/data/c.json"), 200, r#"{"a":1}"#),
    ]);
    assert_eq!(s.routes.len(), 6, "{:?}", templates(&s));
}

#[test]
fn a_literal_sibling_of_an_id_route_stays_literal() {
    let s = built(vec![entry("GET", &format!("{A}/users/1"), 200, r#"{"id":1}"#), entry("GET", &format!("{A}/users/search"), 200, r#"{"results":[]}"#)]);
    assert_eq!(templates(&s), ["GET /users/search", "GET /users/{id}"]);
    let mut t = Turns::default();
    assert!(body(&found(get(&s, &mut t, "/users/search", ""))).contains("results"));
    assert!(body(&found(get(&s, &mut t, "/users/9", ""))).contains("id"));
}

#[test]
fn methods_are_separate_routes() {
    let s = built(vec![entry("GET", &format!("{A}/items/1"), 200, r#"{"a":1}"#), entry("delete", &format!("{A}/items/1"), 204, "")]);
    assert_eq!(templates(&s), ["DELETE /items/{id}", "GET /items/{id}"]);
}

#[test]
fn routes_note_statuses_types_query_names_and_hosts() {
    let mut other = entry("GET", "https://b.example.test/items/2?page=1&sort=asc", 404, "{}");
    other["response"]["content"]["mimeType"] = json!("application/problem+json");
    let s = built(vec![entry("GET", &format!("{A}/items/1?page=2"), 200, "{}"), other]);
    assert_eq!(s.routes.len(), 1);
    let r = &s.routes[0];
    assert_eq!(r.statuses, [200, 404]);
    assert_eq!(r.content_types, ["application/json", "application/problem+json"]);
    assert_eq!(r.query_names, ["page", "sort"]);
    assert_eq!(r.hosts, ["api.example.test", "b.example.test"]);
    assert_eq!(s.hosts, r.hosts);
    assert_eq!(s.version, STANDIN_VERSION);
}

// ---- secrets ----

/// A recording with secrets in every place one can be. Everything in it is an obvious fake.
fn secret_recording() -> Vec<u8> {
    har_of(vec![json!({
        "request": {
            "method": "POST",
            "url": "https://user:hunter2-fake@api.example.test/session/eyJhbGciOiJub25lIn0.eyJzdWIiOiJ4In0.c2ln?token=query-token-fake&page=1&access_token=query-access-fake&note=sk-test-in-query-1234",
            "headers": [
                {"name": "Cookie", "value": "sid=request-cookie-fake"},
                {"name": "Authorization", "value": "Bearer request-bearer-fake"},
                {"name": "X-Api-Key", "value": "sk-test"},
                {"name": "Accept", "value": "application/json"}
            ],
            "cookies": [{"name": "sid", "value": "request-cookie-fake"}],
            "postData": {"mimeType": "application/json", "text": "{\"password\":\"request-password-fake\",\"email\":\"person@example.test\"}"}
        },
        "response": {
            "status": 200,
            "headers": [
                {"name": "Set-Cookie", "value": "sid=response-cookie-fake; HttpOnly"},
                {"name": "Authorization", "value": "Bearer response-bearer-fake"},
                {"name": "X-CSRF-Token", "value": "csrf-fake"},
                {"name": "X-Session-Thing", "value": "session-thing-fake"},
                {"name": "X-Custom", "value": "carries sk-test-header-value-1234"},
                {"name": "Location", "value": "/next?token=location-token-fake&ok=1"},
                {"name": "X-Total-Count", "value": "12"}
            ],
            "cookies": [{"name": "sid", "value": "response-cookie-fake"}],
            "content": {"mimeType": "application/json", "text": "{\"access_token\":\"body-token-fake\",\"refreshToken\":\"body-refresh-fake\",\"profile\":{\"password\":\"body-password-fake\",\"name\":\"Pat\",\"note\":\"Bearer body-bearer-value-fake\",\"jwt\":\"eyJhbGciOiJub25lIn0.eyJzdWIiOiJ5In0.c2ln\",\"gh\":\"ghp_fakefakefakefake\"},\"sk-test-as-a-key-1234\":1,\"key\":\"react-key\",\"code\":\"GB\",\"max_tokens\":5}"}
        }
    })])
}

const FAKES: &[&str] = &[
    "hunter2-fake",
    "eyJhbGciOiJub25lIn0",
    "query-token-fake",
    "query-access-fake",
    "sk-test",
    "request-cookie-fake",
    "request-bearer-fake",
    "request-password-fake",
    "person@example.test",
    "response-cookie-fake",
    "response-bearer-fake",
    "csrf-fake",
    "session-thing-fake",
    "location-token-fake",
    "body-token-fake",
    "body-refresh-fake",
    "body-password-fake",
    "body-bearer-value-fake",
    "ghp_fakefakefakefake",
];

#[test]
fn no_secret_reaches_the_file() {
    let s = build_from_bytes(&secret_recording(), &BuildOptions::default()).unwrap();
    let text = serde_json::to_string(&s).unwrap();
    for fake in FAKES {
        assert!(!text.contains(fake), "{fake} is in the file: {text}");
    }
    let kept: Vec<String> = s.routes.iter().flat_map(|r| &r.samples).flat_map(|x| &x.headers).map(|(k, _)| k.to_ascii_lowercase()).collect();
    assert_eq!(kept, ["location", "x-total-count"], "{text}");
    let d = &s.dropped;
    assert_eq!(d.headers, 8, "{d:?}");
    assert_eq!(d.cookies, 2);
    assert_eq!(d.url_user_info, 1);
    assert_eq!(d.path_segments, 1);
    assert_eq!(d.query_values, 4, "three in the URL and one in the redirect: {d:?}");
    assert!(d.body_values >= 6, "{d:?}");
    assert!(d.total() > 15);
    for name in ["cookie", "authorization", "x-api-key", "set-cookie", "x-csrf-token", "x-custom"] {
        assert!(d.header_names.iter().any(|n| n == name), "{name}: {:?}", d.header_names);
    }
}

#[test]
fn everyday_values_survive_the_secret_checks() {
    let s = build_from_bytes(&secret_recording(), &BuildOptions::default()).unwrap();
    let sample = &s.routes[0].samples[0];
    let v: Value = serde_json::from_str(sample.text.as_deref().unwrap()).unwrap();
    assert_eq!(v["profile"]["name"], "Pat");
    assert_eq!(v["key"], "react-key", "`key` and `code` are everyday names in a body");
    assert_eq!(v["code"], "GB");
    assert_eq!(v["max_tokens"], 5);
    assert_eq!(v["access_token"], DROPPED);
    assert!(sample.headers.iter().any(|(k, v)| k == "X-Total-Count" && v == "12"));
    let location = sample.headers.iter().find(|(k, _)| k == "Location").unwrap();
    assert!(location.1.contains("ok=1") && location.1.contains(DROPPED), "{location:?}");
}

#[test]
fn a_recorded_secret_query_still_matches_a_request_with_any_value_there() {
    let s = built(vec![entry("GET", &format!("{A}/feed?token=recorded-fake&page=1"), 200, r#"{"a":1}"#)]);
    let mut t = Turns::default();
    let f = found(get(&s, &mut t, "/feed", "page=1&token=something-else"));
    assert_eq!(f.how, MatchKind::PathAndQuery);
}

#[test]
fn named_values_are_dropped_in_every_spelling() {
    let secret = "my secret/value";
    let mut e = entry("GET", &format!("{A}/a/my%20secret%2Fvalue/b?x=my+secret/value"), 200, "");
    e["response"]["content"]["text"] = json!(format!("{{\"say\":\"{secret}\"}}"));
    e["response"]["headers"] = json!([{"name": "X-Thing", "value": secret}]);
    let opts = BuildOptions { hidden_values: vec![secret.into()], ..Default::default() };
    let s = build_from_bytes(&har_of(vec![e]), &opts).unwrap();
    let text = serde_json::to_string(&s).unwrap();
    for spelling in [secret, "my%20secret", "my+secret"] {
        assert!(!text.contains(spelling), "{spelling}: {text}");
    }
    assert!(s.dropped.hidden_values >= 3, "{:?}", s.dropped);
}

#[test]
fn a_named_value_inside_a_binary_body_costs_the_body() {
    // "\xff" keeps it from being text; the rest is the named value.
    let encoded = "/3N0b3AtcGxlYXNl";
    assert_eq!(base64_decode(encoded).unwrap(), b"\xffstop-please");
    let mut e = entry("GET", &format!("{A}/bin"), 200, "");
    e["response"]["content"] = json!({"mimeType": "application/octet-stream", "encoding": "base64", "text": encoded});
    let har = har_of(vec![e]);
    let opts = BuildOptions { hidden_values: vec!["stop-please".into()], ..Default::default() };
    let s = build_from_bytes(&har, &opts).unwrap();
    assert_eq!(s.routes[0].samples[0].body_left_out, Some(BodyLeftOut::HeldSecret));
    assert!(s.routes[0].samples[0].base64.is_none());
    let plain = build_from_bytes(&har, &BuildOptions::default()).unwrap();
    assert_eq!(plain.routes[0].samples[0].base64.as_deref(), Some(encoded));
    assert_eq!(found(get(&plain, &mut Turns::default(), "/bin", "")).body, b"\xffstop-please");
}

#[test]
fn private_keys_cost_the_whole_body() {
    // Put together here so that no key-shaped text sits in the repository.
    let (begin, finish) = (format!("-----BEGIN RSA {} KEY-----", "PRIVATE"), format!("-----END RSA {} KEY-----", "PRIVATE"));
    let s = built(vec![entry("GET", &format!("{A}/k"), 200, &format!("{begin}\nfake\n{finish}"))]);
    assert_eq!(s.routes[0].samples[0].body_left_out, Some(BodyLeftOut::HeldSecret));
    assert!(!serde_json::to_string(&s).unwrap().contains("KEY-----"));
}

/// The first recorded answer's text for a body of this type.
fn cleaned(mime: &str, body: &str) -> (String, u64) {
    let mut e = entry("GET", &format!("{A}/page"), 200, body);
    e["response"]["content"]["mimeType"] = json!(mime);
    let opts = BuildOptions { include_static: true, ..BuildOptions::default() };
    let s = build_from_bytes(&har_of(vec![e]), &opts).unwrap();
    (s.routes[0].samples[0].text.clone().unwrap(), s.dropped.body_values)
}

#[test]
fn secrets_inside_urls_and_pairs_go_and_the_rest_stays() {
    let (text, n) =
        cleaned("application/json", r#"{"next":"https://x.example/cb?ref=ghp_fakefakefakefake","hook":"https://x.example/sk_live_fakefake1234/run"}"#);
    assert!(!text.contains("ghp_") && !text.contains("sk_live_"), "{text}");
    assert!(text.contains("ref=[dropped]") && text.contains("//x.example/[dropped]/run"), "{text}");
    assert_eq!(n, 2);
}

#[test]
fn a_url_keeps_its_host_and_loses_its_user_info() {
    for (mime, body) in
        [("text/plain", "see https://bob:hunter2@internal.example/app for it"), ("application/json", r#"{"u":"https://bob:hunter2@internal.example/app"}"#)]
    {
        let mut e = entry("GET", &format!("{A}/page"), 200, body);
        e["response"]["content"]["mimeType"] = json!(mime);
        let s = built(vec![e]);
        let text = s.routes[0].samples[0].text.as_deref().unwrap();
        assert!(!text.contains("hunter2") && !text.contains("bob") && text.contains("https://internal.example/app"), "{text}");
        assert_eq!(s.dropped.url_user_info, 1);
    }
    let (kept, _) = cleaned("text/plain", "mail pat@example.test or see https://example.test/a@b ok");
    assert_eq!(kept, "mail pat@example.test or see https://example.test/a@b ok");
}

#[test]
fn a_redirect_keeps_going_there_without_user_info() {
    for (location, want) in [
        ("https://bob:hunter2@internal.example/app?x=1", "https://internal.example/app?x=1"),
        ("https://bob:hunter2@internal.example/app", "https://internal.example/app"),
        ("//bob:hunter2@internal.example/app?x=1", "//internal.example/app?x=1"),
    ] {
        let mut e = entry("GET", &format!("{A}/old"), 302, "");
        e["response"]["headers"] = json!([{"name": "Location", "value": location}]);
        let s = built(vec![e]);
        let headers = &s.routes[0].samples[0].headers;
        assert!(headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("location") && v == want), "{location}: {headers:?}");
        assert_eq!(s.dropped.url_user_info, 1, "{location}");
        assert!(!serde_json::to_string(&s).unwrap().contains("hunter2"));
    }
}

#[test]
fn everyday_words_after_basic_or_bearer_stay() {
    let body = r#"{"plan":"Basic Membership","title":"Basic settings","kind":"Basic self-service","note":"bearer authentication","loud":"BASIC MEMBERSHIP"}"#;
    let (text, n) = cleaned("application/json", body);
    assert_eq!((text.as_str(), n), (body, 0));
    for secret in ["Basic dXNlcjpwYXNz", "Basic Zm9vOmJhcg==", "Bearer body-bearer-value-fake", "bearer abcdefgh12345678"] {
        let (text, n) = cleaned("text/plain", secret);
        assert!(text.ends_with(" [dropped]") && n == 1, "{secret}: {text}");
    }
}

#[test]
fn code_stays_code() {
    for mime in ["application/javascript", "text/javascript; charset=utf-8", "text/css", "text/html"] {
        let code = "this.token=t;e.password=n;a&&token=b;var p=\"Basic settings\";";
        let (text, n) = cleaned(mime, code);
        assert_eq!((text.as_str(), n), (code, 0), "{mime}");
    }
    let (text, n) = cleaned("text/html", r#"<a href="/x?token=real-fake&amp;api_key=other-fake&ok=1">sk-test-free-text-1234</a>"#);
    assert_eq!(n, 3, "{text}");
    assert!(!text.contains("real-fake") && !text.contains("other-fake") && !text.contains("sk-test") && text.contains("ok=1"), "{text}");
    let (plain, _) = cleaned("text/plain", "this.token=t");
    assert_eq!(plain, "[dropped]", "free text is still read as text");
}

#[test]
fn free_text_bodies_are_scanned_too() {
    let mut e =
        entry("GET", &format!("{A}/page"), 200, "Your key is sk-test-free-text-1234 and access_token=plain-fake; Authorization: Bearer abcdefgh12345678 ok");
    e["response"]["content"]["mimeType"] = json!("text/plain");
    let s = built(vec![e]);
    let text = s.routes[0].samples[0].text.as_deref().unwrap();
    assert!(!text.contains("sk-test") && !text.contains("plain-fake") && !text.contains("abcdefgh12345678"), "{text}");
    assert!(text.starts_with("Your key is") && text.ends_with("ok"), "{text}");
}

#[test]
fn untouched_json_keeps_its_exact_bytes() {
    let original = "{ \"b\" : 1,\n  \"a\": [1, 2] }";
    let s = built(vec![entry("GET", &format!("{A}/x"), 200, original)]);
    assert_eq!(s.routes[0].samples[0].text.as_deref(), Some(original));
}

#[test]
fn request_bodies_are_never_kept_only_their_key() {
    assert_eq!(request_key(br#"{"operationName":"GetUser","query":"query GetUser { a }"}"#).as_deref(), Some("op:GetUser"));
    assert_eq!(request_key(br#"{"query":"query Named { a }"}"#).as_deref(), Some("op:Named"));
    assert_eq!(request_key(br#"[{"operationName":"A"},{"operationName":"B"}]"#).as_deref(), Some("op:A,B"));
    assert_eq!(request_key(br#"{"password":"nope-fake","email":"x"}"#).as_deref(), Some("keys:email,password"));
    assert_eq!(request_key(br#"{"query":"{ a }"}"#).as_deref(), Some("keys:query"), "an anonymous document names nothing");
    for none in [&b""[..], b"not json", b"{}", b"[]", b"[1,2]", b"42", b"\xff\xfe"] {
        assert_eq!(request_key(none), None, "{none:?}");
    }
    assert_eq!(request_key(&vec![b' '; MAX_KEY_BODY + 1]), None, "too large to read");
}

// ---- what's skipped ----

#[test]
fn entries_that_cannot_be_answered_are_counted_not_kept() {
    let mut page = entry("GET", &format!("{A}/index"), 200, "<html>");
    page["_resourceType"] = json!("document");
    let mut pic = entry("GET", &format!("{A}/a.png"), 200, "");
    pic["response"]["content"]["mimeType"] = json!("image/png");
    let har = har_of(vec![
        entry("GET", "data:text/plain,hi", 200, ""),
        entry("GET", "blob:https://x/1", 200, ""),
        entry("GET", "wss://x.test/socket", 101, ""),
        entry("GET", "chrome-extension://abc/x", 200, ""),
        entry("GET", &format!("{A}/failed"), 0, ""),
        entry("GET", &format!("{A}/switching"), 101, ""),
        page,
        pic,
        entry("BAD METHOD", &format!("{A}/x"), 200, ""),
        entry("GET", "https://other.example.test/x", 200, "{}"),
        entry("GET", &format!("{A}/kept"), 200, "{}"),
    ]);
    let opts = BuildOptions { only_hosts: vec!["api.example.test".into()], ..Default::default() };
    let s = build_from_bytes(&har, &opts).unwrap();
    assert_eq!(templates(&s), ["GET /kept"]);
    assert_eq!(s.skipped, Skipped { not_web: 4, no_response: 3, static_files: 2, other_hosts: 1, beyond_route_limit: 0 });
    let all = BuildOptions { include_static: true, ..Default::default() };
    let s = build_from_bytes(&har, &all).unwrap();
    assert!(templates(&s).contains(&"GET /index".to_string()) && templates(&s).contains(&"GET /a.png".to_string()));
}

#[test]
fn a_port_in_the_recording_counts_as_the_same_host_when_asked_by_name() {
    let har = har_of(vec![entry("GET", "http://localhost:3000/x", 200, "{}")]);
    let by_name = BuildOptions { only_hosts: vec!["LOCALHOST".into()], ..Default::default() };
    assert_eq!(build_from_bytes(&har, &by_name).unwrap().routes.len(), 1);
    let other = BuildOptions { only_hosts: vec!["localhost:4000".into()], ..Default::default() };
    assert_eq!(build_from_bytes(&har, &other).unwrap().routes.len(), 0);
}

// ---- malformed and huge ----

#[test]
fn malformed_recordings_are_errors_or_skipped_never_a_crash() {
    for bad in [&b""[..], b"nope", b"{}", b"[]", b"{\"log\":{}}", b"{\"log\":{\"entries\":3}}"] {
        assert!(build_from_bytes(bad, &BuildOptions::default()).is_err(), "{bad:?}");
    }
    let odd = br#"{"log":{"entries":[1,null,"x",{},{"request":3,"response":[]},
        {"request":{"method":"GET","url":"https://x.test/ok"},"response":{"status":99999999999,"content":{"text":3}}},
        {"request":{"method":"GET","url":"https://x.test/neg"},"response":{"status":-1}},
        {"request":{"method":"GET","url":""},"response":{"status":200}},
        {"request":{"method":"GET","url":"https://"},"response":{"status":200}},
        {"request":{"method":"GET","url":"https://x.test/n"},"response":{"status":200,"headers":[{"name":"X\r\nInjected","value":"a"},{"name":"X-Bad","value":"a\r\nb: c"},{"value":"no name"},7]}},
        {"request":{"method":"GET","url":"https://x.test/b"},"response":{"status":200,"content":{"encoding":"base64","text":"@@not base64@@"}}}
    ]}}"#;
    let s = build_from_bytes(odd, &BuildOptions::default()).unwrap();
    assert_eq!(templates(&s), ["GET /b", "GET /n"], "{s:?}");
    assert_eq!(s.routes[0].samples[0].body_left_out, Some(BodyLeftOut::Undecodable));
    let n = &s.routes[1].samples[0];
    assert!(n.headers.is_empty(), "no header that could start a new line: {:?}", n.headers);
    assert_eq!(n.body_left_out, Some(BodyLeftOut::NotRecorded));
}

#[test]
fn caps_hold_on_huge_inputs() {
    let opts = BuildOptions { max_entries: 50, max_routes: 10, max_samples_per_route: 3, max_body_bytes: 64, max_total_body_bytes: 200, ..Default::default() };
    let mut entries: Vec<Value> = (0..30).map(|i| entry("GET", &format!("{A}/same/{i}"), 200, &format!("{{\"i\":{i}}}"))).collect();
    entries.extend((0..40).map(|i| entry("GET", &format!("{A}/r{i}x"), 200, "{}")));
    let s = build_from_bytes(&har_of(entries), &opts).unwrap();
    assert!(s.source.cut_short);
    assert_eq!((s.source.entries_read, s.source.entries_declared), (50, 70));
    assert_eq!(s.routes.len(), 10);
    assert_eq!(s.skipped.beyond_route_limit, 11);
    let same = s.routes.iter().find(|r| r.template == "/same/{id}").unwrap();
    assert_eq!((same.requests, same.samples.len(), same.samples_left_out), (30, 3, 27));

    let big = "x".repeat(65);
    let s = build_from_bytes(&har_of(vec![entry("GET", &format!("{A}/big"), 200, &big)]), &opts).unwrap();
    assert_eq!(s.routes[0].samples[0].body_left_out, Some(BodyLeftOut::TooLarge));
    assert!(s.routes[0].samples[0].text.is_none(), "never half a body");

    let files: Vec<Value> = (0..5).map(|i| entry("GET", &format!("{A}/f{i}y"), 200, &"y".repeat(60))).collect();
    let s = build_from_bytes(&har_of(files), &opts).unwrap();
    let kept: usize = s.routes.iter().flat_map(|r| &r.samples).filter_map(|x| x.text.as_ref()).map(String::len).sum();
    assert!(kept <= 200, "{kept}");
    assert_eq!(s.routes.iter().flat_map(|r| &r.samples).filter(|x| x.body_left_out == Some(BodyLeftOut::FileLimit)).count(), 2);
}

#[test]
fn hostile_paths_and_queries_are_bounded() {
    let deep = format!("{A}/{}", "a/".repeat(20_000));
    let wide: String = (0..5_000).map(|i| format!("k{i}=v&")).collect();
    let s = built(vec![
        entry("GET", &deep, 200, "{}"),
        entry("GET", &format!("{A}/q?{wide}"), 200, "{}"),
        entry("GET", &format!("{A}/{}", "é/".repeat(50)), 200, "{}"),
    ]);
    assert_eq!(s.routes.len(), 3);
    let wide_route = s.routes.iter().find(|r| r.template == "/q").unwrap();
    assert!(wide_route.query_names.len() <= MAX_LISTED);
    let mut t = Turns::default();
    for (path, query) in [(deep.trim_start_matches(A), ""), ("/", ""), ("", ""), ("//", "&&&==&"), ("/é", "é=é")] {
        let _ = get(&s, &mut t, path, query);
    }
    let _ = answer(&s, &mut t, "", "/", "", &[0xff; 64]);
}

#[test]
fn deeply_nested_json_is_kept_or_refused_without_a_crash() {
    let deep = format!("{}1{}", "[".repeat(100_000), "]".repeat(100_000));
    let s = built(vec![entry("GET", &format!("{A}/deep"), 200, &deep)]);
    assert_eq!(s.routes.len(), 1);
    assert!(request_key(deep.as_bytes()).is_none());
}

// ---- answering ----

#[test]
fn the_best_match_wins_in_order() {
    let s = built(vec![
        entry("GET", &format!("{A}/items/1"), 200, r#"{"from":"path-no-query"}"#),
        entry("GET", &format!("{A}/items/1?a=1&b=2"), 200, r#"{"from":"path-and-query"}"#),
        entry("GET", &format!("{A}/items/2"), 200, r#"{"from":"other-item"}"#),
    ]);
    let mut t = Turns::default();
    let f = found(get(&s, &mut t, "/items/1", "b=2&a=1"));
    assert_eq!(f.how, MatchKind::PathAndQuery, "the order of a query doesn't matter");
    assert!(body(&f).contains("path-and-query"));
    let f = found(get(&s, &mut t, "/items/2", "z=9"));
    assert_eq!(f.how, MatchKind::Path);
    assert!(body(&f).contains("other-item"));
    let f = found(get(&s, &mut Turns::default(), "/items/999", ""));
    assert_eq!(f.how, MatchKind::Template);
    assert!(body(&f).contains("path-no-query"), "the first recorded answer");
    assert_eq!(f.route, "GET /items/{id}");
}

#[test]
fn repeated_requests_cycle_through_recorded_answers_in_order() {
    let s = built(vec![
        entry("GET", &format!("{A}/poll"), 200, r#"{"n":1}"#),
        entry("GET", &format!("{A}/poll"), 200, r#"{"n":2}"#),
        entry("GET", &format!("{A}/poll"), 503, r#"{"n":3}"#),
    ]);
    let mut t = Turns::default();
    let seen: Vec<(u16, String, usize, usize)> = (0..7)
        .map(|_| {
            let f = found(get(&s, &mut t, "/poll", ""));
            (f.status, body(&f), f.turn, f.of)
        })
        .collect();
    assert_eq!(seen[0], (200, r#"{"n":1}"#.to_string(), 1, 3));
    assert_eq!(seen[1], (200, r#"{"n":2}"#.to_string(), 2, 3));
    assert_eq!(seen[2], (503, r#"{"n":3}"#.to_string(), 3, 3));
    assert_eq!(seen[3], seen[0], "and round again");
    assert_eq!(seen[6], seen[0]);
    // Each caller's turns are its own, and a different request doesn't move them.
    assert_eq!(found(get(&s, &mut Turns::default(), "/poll", "")).turn, 1);
}

#[test]
fn identical_answers_in_a_row_collapse_into_one() {
    let s = built(vec![
        entry("GET", &format!("{A}/same"), 200, "{}"),
        entry("GET", &format!("{A}/same"), 200, "{}"),
        entry("GET", &format!("{A}/same"), 200, r#"{"a":1}"#),
    ]);
    assert_eq!(s.routes[0].requests, 3);
    assert_eq!(s.routes[0].samples.len(), 2);
}

#[test]
fn turns_do_not_grow_with_requests() {
    let s = built(vec![entry("GET", &format!("{A}/users/1"), 200, "{}"), entry("GET", &format!("{A}/users/2"), 200, "{}")]);
    let mut t = Turns::default();
    for i in 0..2_000 {
        let _ = get(&s, &mut t, &format!("/users/{i}"), &format!("x={i}"));
        let _ = get(&s, &mut t, &format!("/nothing/{i}"), "");
    }
    assert!(t.turns.len() <= 3, "{}", t.turns.len());
    // Moving through a recording doesn't grow them past what it holds either.
    let s = notes_recording();
    let mut t = Turns::default();
    for _ in 0..500 {
        let _ = answer(&s, &mut t, "POST", "/notes", "", b"");
        let _ = answer(&s, &mut t, "DELETE", "/notes/7", "", b"");
        let _ = get(&s, &mut t, "/notes", "");
    }
    assert!(t.turns.len() <= 7 * 3, "{}", t.turns.len());
}

/// A to-do list recorded while two notes were added and one taken away.
fn notes_recording() -> StandIn {
    built(vec![
        entry("GET", &format!("{A}/notes"), 200, r#"{"notes":[]}"#),
        entry("POST", &format!("{A}/notes"), 201, r#"{"id":7}"#),
        entry("GET", &format!("{A}/notes"), 200, r#"{"notes":[7]}"#),
        entry("POST", &format!("{A}/notes"), 201, r#"{"id":8}"#),
        entry("GET", &format!("{A}/notes"), 200, r#"{"notes":[7,8]}"#),
        entry("DELETE", &format!("{A}/notes/7"), 204, ""),
        entry("GET", &format!("{A}/notes"), 200, r#"{"notes":[8]}"#),
    ])
}

fn read_notes(s: &StandIn, t: &mut Turns) -> String {
    body(&found(get(s, t, "/notes", "")))
}

#[test]
fn a_write_moves_later_reads_on_to_what_was_recorded_after_it() {
    let s = notes_recording();
    let before = s.clone();
    let mut t = Turns::default();
    assert_eq!(read_notes(&s, &mut t), r#"{"notes":[]}"#);
    assert_eq!(read_notes(&s, &mut t), r#"{"notes":[]}"#, "reads alone don't move on");
    let created = found(answer(&s, &mut t, "POST", "/notes", "", br#"{"text":"anything"}"#));
    assert_eq!((created.status, body(&created).as_str()), (201, r#"{"id":7}"#));
    assert_eq!(read_notes(&s, &mut t), r#"{"notes":[7]}"#);
    assert_eq!(read_notes(&s, &mut t), r#"{"notes":[7]}"#);
    assert_eq!(body(&found(answer(&s, &mut t, "post", "/notes", "", b"{}"))), r#"{"id":8}"#, "the next write is the next one recorded");
    assert_eq!(read_notes(&s, &mut t), r#"{"notes":[7,8]}"#);
    assert_eq!(found(answer(&s, &mut t, "DELETE", "/notes/7", "", b"")).status, 204);
    assert_eq!(read_notes(&s, &mut t), r#"{"notes":[8]}"#);
    assert_eq!(t.position(), Some(5));
    assert_eq!(s, before, "the stand-in itself never changes");
    // Another client starts at the beginning.
    assert_eq!(read_notes(&s, &mut Turns::default()), r#"{"notes":[]}"#);
}

#[test]
fn a_write_past_the_recorded_ones_is_refused_and_time_never_goes_back() {
    let s = notes_recording();
    let mut t = Turns::default();
    for _ in 0..2 {
        let _ = found(answer(&s, &mut t, "POST", "/notes", "", b""));
    }
    // No third add was recorded: answering it with a recorded success would say it worked.
    let third = missing(answer(&s, &mut t, "POST", "/notes", "", b""));
    assert_eq!(third.reason, NoMatchReason::WriteNotRecorded);
    assert_eq!(read_notes(&s, &mut t), r#"{"notes":[7,8]}"#, "and it changes nothing");
    // The delete was recorded later than both posts; posting again doesn't take the reads back.
    let _ = found(answer(&s, &mut t, "DELETE", "/notes/7", "", b""));
    assert_eq!(missing(answer(&s, &mut t, "DELETE", "/notes/7", "", b"")).reason, NoMatchReason::WriteNotRecorded, "nor a second delete");
    assert_eq!(missing(answer(&s, &mut t, "POST", "/notes", "", b"")).reason, NoMatchReason::WriteNotRecorded);
    assert_eq!(read_notes(&s, &mut t), r#"{"notes":[8]}"#);
    assert_eq!(t.position(), Some(5));
}

#[test]
fn a_write_the_client_skips_still_ends_the_reads_before_it() {
    let s = notes_recording();
    let mut t = Turns::default();
    // Deleting first jumps over both posts: the reads after the delete are the ones that apply.
    let _ = answer(&s, &mut t, "DELETE", "/notes/7", "", b"");
    assert_eq!(read_notes(&s, &mut t), r#"{"notes":[8]}"#);
    // Posts step through their own recorded answers, and reads don't go back before the delete.
    assert_eq!(body(&found(answer(&s, &mut t, "POST", "/notes", "", b""))), r#"{"id":7}"#);
    assert_eq!(read_notes(&s, &mut t), r#"{"notes":[8]}"#);
}

#[test]
fn a_refused_write_changes_nothing() {
    let s = built(vec![
        entry("GET", &format!("{A}/cart"), 200, r#"{"items":0}"#),
        entry("POST", &format!("{A}/cart"), 422, r#"{"error":"out of stock"}"#),
        entry("GET", &format!("{A}/cart"), 200, r#"{"items":0,"seen":2}"#),
        entry("POST", &format!("{A}/cart"), 200, r#"{"items":1}"#),
        entry("GET", &format!("{A}/cart"), 200, r#"{"items":1}"#),
    ]);
    let mut t = Turns::default();
    let refused = found(answer(&s, &mut t, "POST", "/cart", "", b"{}"));
    assert_eq!(refused.status, 422);
    assert_eq!(t.position(), None, "the original refused it, so later reads stay where they were");
    // Both reads before the accepted write still apply, in turn.
    let reads: Vec<String> = (0..3).map(|_| body(&found(get(&s, &mut t, "/cart", "")))).collect();
    assert_eq!(reads, [r#"{"items":0}"#, r#"{"items":0,"seen":2}"#, r#"{"items":0}"#]);
    assert_eq!(found(answer(&s, &mut t, "POST", "/cart", "", b"{}")).status, 200);
    assert_eq!(body(&found(get(&s, &mut t, "/cart", ""))), r#"{"items":1}"#);
}

#[test]
fn graphql_queries_sent_by_post_are_reads_and_mutations_are_writes() {
    let gql = |text: &str, resp: &str| {
        let mut e = entry("POST", &format!("{A}/graphql"), 200, resp);
        e["request"]["postData"] = json!({"mimeType": "application/json", "text": text});
        e
    };
    let list = r#"{"query":"query Todos { todos { id } }"}"#;
    let add = r#"{"query":"mutation Add($t: String) { add(text: $t) { id } }","variables":{"t":"x"}}"#;
    let s = built(vec![gql(list, r#"{"data":{"todos":[]}}"#), gql(add, r#"{"data":{"add":{"id":1}}}"#), gql(list, r#"{"data":{"todos":[{"id":1}]}}"#)]);
    let samples = &s.routes[0].samples;
    assert_eq!(samples.iter().map(|x| x.write).collect::<Vec<_>>(), [false, true, false]);
    let mut t = Turns::default();
    let ask = |t: &mut Turns, text: &str| body(&found(answer(&s, t, "POST", "/graphql", "", text.as_bytes())));
    assert_eq!(ask(&mut t, list), r#"{"data":{"todos":[]}}"#);
    assert_eq!(ask(&mut t, list), r#"{"data":{"todos":[]}}"#, "a query moves nothing on");
    assert_eq!(ask(&mut t, add), r#"{"data":{"add":{"id":1}}}"#);
    assert_eq!(ask(&mut t, list), r#"{"data":{"todos":[{"id":1}]}}"#);
    // A query sent only by name, or one we can't read, can't be told from a write: it goes as one.
    let only_reads = |b: &[u8]| json_body(b).is_some_and(|v| graphql_reads(&v));
    assert!(!only_reads(br#"{"operationName":"Todos","extensions":{"persistedQuery":{"sha256Hash":"ab"}}}"#));
    assert!(!only_reads(br#"[{"query":"query A { a }"},{"query":"mutation B { b }"}]"#));
    assert!(only_reads(br#"[{"query":"query A { a }"},{"query":"{ b }"}]"#));
    assert!(!only_reads(b"not json") && !only_reads(b""));
}

#[test]
fn an_unrelated_write_moves_nothing() {
    let track = |n: u32| entry("POST", &format!("{A}/track"), 204, &n.to_string());
    let s = built(vec![
        entry("GET", &format!("{A}/notes"), 200, r#"{"notes":[]}"#),
        track(1),
        entry("POST", &format!("{A}/notes"), 201, r#"{"id":7}"#),
        entry("GET", &format!("{A}/notes"), 200, r#"{"notes":[7]}"#),
        track(2),
        entry("GET", &format!("{A}/notes"), 200, r#"{"notes":[7]}"#),
    ]);
    let notes = s.routes.iter().find(|r| r.template == "/notes" && r.method == "GET").unwrap();
    assert_eq!(notes.samples.len(), 2, "a beacon between two equal reads doesn't keep both");
    assert_eq!(s.routes.iter().find(|r| r.template == "/track").unwrap().samples.len(), 2, "writes are each kept");
    let mut t = Turns::default();
    for _ in 0..3 {
        let _ = answer(&s, &mut t, "POST", "/track", "", b"");
    }
    assert_eq!(read_notes(&s, &mut t), r#"{"notes":[]}"#, "no note was added");
    assert_eq!(body(&found(answer(&s, &mut t, "POST", "/notes", "", b""))), r#"{"id":7}"#, "its own first post");
    assert_eq!(read_notes(&s, &mut t), r#"{"notes":[7]}"#);
    for (a, b, yes) in [
        ("/notes", "/notes/{id}", true),
        ("/cart/items", "/cart", true),
        ("/notes", "/notes", true),
        ("/notes", "/notesx", false),
        ("/track", "/notes", false),
        ("/", "/a", true),
    ] {
        assert_eq!((related(a, b), related(b, a)), (yes, yes), "{a} {b}");
    }
}

#[test]
fn a_query_named_only_by_its_operation_reads_like_the_recording_did() {
    let mut e = entry("POST", &format!("{A}/graphql"), 200, r#"{"data":{"n":1}}"#);
    e["request"]["postData"] = json!({"text": r#"{"query":"query Count { n }"}"#});
    let mut e2 = e.clone();
    e2["response"]["content"]["text"] = json!(r#"{"data":{"n":2}}"#);
    let s = built(vec![e, e2]);
    let mut t = Turns::default();
    // A persisted query sends a name and no document; the recorded ones were queries, so it goes round.
    let persisted = br#"{"operationName":"Count","extensions":{"persistedQuery":{"sha256Hash":"ab"}}}"#;
    let seen: Vec<String> = (0..3).map(|_| body(&found(answer(&s, &mut t, "POST", "/graphql", "", persisted)))).collect();
    assert_eq!(seen, [r#"{"data":{"n":1}}"#, r#"{"data":{"n":2}}"#, r#"{"data":{"n":1}}"#]);
    assert_eq!(t.position(), None);
}

#[test]
fn polling_between_two_writes_still_goes_round() {
    let s = built(vec![
        entry("GET", &format!("{A}/job"), 200, r#"{"state":"idle"}"#),
        entry("POST", &format!("{A}/job"), 202, r#"{"started":true}"#),
        entry("GET", &format!("{A}/job"), 200, r#"{"state":"running"}"#),
        entry("GET", &format!("{A}/job"), 200, r#"{"state":"done"}"#),
    ]);
    let mut t = Turns::default();
    assert_eq!(body(&found(get(&s, &mut t, "/job", ""))), r#"{"state":"idle"}"#);
    let _ = answer(&s, &mut t, "POST", "/job", "", b"");
    let seen: Vec<(String, usize, usize)> = (0..3)
        .map(|_| {
            let f = found(get(&s, &mut t, "/job", ""));
            (body(&f), f.turn, f.of)
        })
        .collect();
    assert_eq!(seen[0], (r#"{"state":"running"}"#.to_string(), 1, 2));
    assert_eq!(seen[1], (r#"{"state":"done"}"#.to_string(), 2, 2));
    assert_eq!(seen[2], seen[0]);
}

#[test]
fn a_read_with_nothing_recorded_since_the_last_write_says_what_it_last_said() {
    let s = built(vec![
        entry("GET", &format!("{A}/me"), 200, r#"{"name":"Pat"}"#),
        entry("PUT", &format!("{A}/me"), 200, "{}"),
        entry("PUT", &format!("{A}/me"), 200, r#"{"again":true}"#),
        entry("GET", &format!("{A}/me/avatar"), 200, r#"{"x":1}"#),
    ]);
    let mut t = Turns::default();
    let _ = answer(&s, &mut t, "PUT", "/me", "", b"");
    assert_eq!(body(&found(get(&s, &mut t, "/me", ""))), r#"{"name":"Pat"}"#, "the newest answer from before");
    // Asked before its time, a read gets the earliest answer recorded for it.
    assert_eq!(body(&found(get(&s, &mut Turns::default(), "/me/avatar", ""))), r#"{"x":1}"#);
}

#[test]
fn the_same_answer_either_side_of_a_write_is_kept_twice() {
    let s = built(vec![
        entry("GET", &format!("{A}/feed"), 200, r#"{"n":0}"#),
        entry("GET", &format!("{A}/feed"), 200, r#"{"n":0}"#),
        entry("POST", &format!("{A}/feed/like"), 200, "{}"),
        entry("GET", &format!("{A}/feed"), 200, r#"{"n":0}"#),
        entry("GET", &format!("{A}/feed"), 200, r#"{"n":1}"#),
    ]);
    let feed = s.routes.iter().find(|r| r.template == "/feed").unwrap();
    assert_eq!(feed.samples.iter().map(|x| x.order).collect::<Vec<_>>(), [Some(0), Some(3), Some(4)], "the poll before the write collapses, not across it");
    let mut t = Turns::default();
    let _ = answer(&s, &mut t, "POST", "/feed/like", "", b"");
    let after: Vec<String> = (0..2).map(|_| body(&found(get(&s, &mut t, "/feed", "")))).collect();
    assert_eq!(after, [r#"{"n":0}"#, r#"{"n":1}"#], "the recording's order after the write");
}

#[test]
fn a_file_without_positions_replays_as_before() {
    let v = json!({"version": 1, "routes": [
        {"method": "GET", "template": "/a", "samples": [{"path": "/a", "status": 200, "text": "1"}, {"path": "/a", "status": 200, "text": "2"}]},
        {"method": "POST", "template": "/a", "samples": [{"path": "/a", "status": 201, "text": "made"}]},
    ]});
    let s: StandIn = serde_json::from_value(v).unwrap();
    let mut t = Turns::default();
    assert_eq!(body(&found(get(&s, &mut t, "/a", ""))), "1");
    assert_eq!(body(&found(answer(&s, &mut t, "POST", "/a", "", b""))), "made");
    assert_eq!(t.position(), None);
    assert_eq!(body(&found(get(&s, &mut t, "/a", ""))), "2", "round in recorded order, whatever was written");
    // A hand-edited mix of placed and unplaced answers is read the old way too.
    let mixed = json!({"version": 2, "routes": [
        {"method": "GET", "template": "/b", "samples": [{"path": "/b", "status": 200, "text": "1", "order": 9}, {"path": "/b", "status": 200, "text": "2"}]},
        {"method": "POST", "template": "/b", "samples": [{"path": "/b", "status": 200, "text": "w", "order": 3, "write": true}]},
    ]});
    let s: StandIn = serde_json::from_value(mixed).unwrap();
    let mut t = Turns::default();
    let _ = answer(&s, &mut t, "POST", "/b", "", b"");
    assert_eq!([body(&found(get(&s, &mut t, "/b", ""))), body(&found(get(&s, &mut t, "/b", "")))], ["1", "2"]);
}

#[test]
fn a_file_without_positions_counts_a_lone_answer_as_a_turn() {
    let v = json!({"version": 1, "routes": [
        {"method": "POST", "template": "/search", "samples": [
            {"path": "/search", "status": 200, "text": "by q", "request_key": "keys:q"},
            {"path": "/search", "status": 200, "text": "by tag", "request_key": "keys:tag"},
        ]},
    ]});
    let s: StandIn = serde_json::from_value(v).unwrap();
    let mut t = Turns::default();
    assert_eq!(body(&found(answer(&s, &mut t, "POST", "/search", "", br#"{"q":"a"}"#))), "by q");
    assert_eq!(body(&found(answer(&s, &mut t, "POST", "/search", "", br#"{"other":1}"#))), "by tag", "the old replay: the first answer took its turn");
}

#[test]
fn hostile_positions_cannot_crash_or_loop() {
    let v = json!({"version": 2, "routes": [
        {"method": "GET", "template": "/a", "samples": [
            {"path": "/a", "status": 200, "text": "max", "order": u64::MAX},
            {"path": "/a", "status": 200, "text": "zero", "order": 0},
            {"path": "/a", "status": 200, "text": "dup", "order": 0},
        ]},
        {"method": "POST", "template": "/a", "samples": [
            {"path": "/a", "status": 200, "text": "w", "order": u64::MAX, "write": true},
            {"path": "/a", "status": 200, "text": "v", "order": 0, "write": true},
        ]},
    ]});
    let s: StandIn = serde_json::from_value(v).unwrap();
    let mut t = Turns::default();
    for _ in 0..10 {
        let _ = get(&s, &mut t, "/a", "");
        let _ = answer(&s, &mut t, "POST", "/a", "", b"");
    }
    assert_eq!(t.position(), Some(u64::MAX));
    assert_eq!(body(&found(get(&s, &mut t, "/a", ""))), "max");
}

#[test]
fn head_is_answered_as_get_and_methods_do_not_cross() {
    let s = built(vec![entry("GET", &format!("{A}/a"), 200, r#"{"x":1}"#)]);
    let mut t = Turns::default();
    assert_eq!(found(answer(&s, &mut t, "head", "/a", "", b"")).status, 200);
    let n = missing(answer(&s, &mut t, "POST", "/a", "", b""));
    assert_eq!(n.reason, NoMatchReason::MethodNotRecorded);
    assert_eq!(n.nearest[0], NearbyRoute { method: "GET".into(), template: "/a".into() });
}

#[test]
fn graphql_operations_are_told_apart_at_one_url() {
    let post = |op: &str, resp: &str| {
        let mut e = entry("POST", &format!("{A}/graphql"), 200, resp);
        e["request"]["postData"] = json!({"mimeType": "application/json", "text": json!({"operationName": op, "query": format!("query {op} {{ a }}"), "variables": {"secret": "variable-fake"}}).to_string()});
        e
    };
    let s = built(vec![post("GetUser", r#"{"data":{"user":1}}"#), post("GetFeed", r#"{"data":{"feed":2}}"#)]);
    assert!(!serde_json::to_string(&s).unwrap().contains("variable-fake"), "request bodies aren't kept");
    let ask = |t: &mut Turns, op: &str| answer(&s, t, "POST", "/graphql", "", json!({"operationName": op}).to_string().as_bytes());
    let mut t = Turns::default();
    for _ in 0..3 {
        assert!(body(&found(ask(&mut t, "GetFeed"))).contains("feed"));
        assert!(body(&found(ask(&mut t, "GetUser"))).contains("user"));
    }
    // Another operation's data would be the wrong answer, not a close one.
    let n = missing(ask(&mut t, "SaveUser"));
    assert_eq!(n.reason, NoMatchReason::OperationNotRecorded);
    assert_eq!(n.nearest[0].template, "/graphql");
    // A body that names no operation takes what there is: the recording shows these were queries.
    assert_eq!(found(answer(&s, &mut t, "POST", "/graphql", "", b"not json")).of, 2);
}

#[test]
fn json_bodies_prefer_the_same_names_but_accept_others() {
    let post = |body: &str, resp: &str| {
        let mut e = entry("POST", &format!("{A}/search"), 200, resp);
        e["request"]["postData"] = json!({"text": body});
        e
    };
    let s = built(vec![post(r#"{"q":"a"}"#, r#"{"by":"q"}"#), post(r#"{"tag":"b"}"#, r#"{"by":"tag"}"#)]);
    let mut t = Turns::default();
    assert!(body(&found(answer(&s, &mut t, "POST", "/search", "", br#"{"tag":"other"}"#))).contains("tag"));
    // Any other body may be answered by either. As a write it steps through them, and the `tag`
    // write before it doesn't move the `q` kind on; once both were replayed there's none left.
    assert_eq!(body(&found(answer(&s, &mut t, "POST", "/search", "", br#"{"new":1}"#))), r#"{"by":"q"}"#);
    assert_eq!(missing(answer(&s, &mut t, "POST", "/search", "", br#"{"new":1}"#)).reason, NoMatchReason::WriteNotRecorded);
}

#[test]
fn no_match_names_the_nearest_routes() {
    let s = built(vec![
        entry("GET", &format!("{A}/api/users/1"), 200, "{}"),
        entry("GET", &format!("{A}/api/users/1/posts"), 200, "{}"),
        entry("POST", &format!("{A}/api/users"), 201, "{}"),
        entry("GET", &format!("{A}/health"), 200, "{}"),
    ]);
    let mut t = Turns::default();
    let n = missing(get(&s, &mut t, "/api/users/1/comments", ""));
    assert_eq!(n.reason, NoMatchReason::PathNotRecorded);
    assert_eq!(n.nearest[0], NearbyRoute { method: "GET".into(), template: "/api/users/{id}/posts".into() });
    assert!(n.nearest.len() <= MAX_NEAREST);
    assert_eq!(n.nearest.last().unwrap().template, "/health");
    assert_eq!(missing(get(&StandIn::default(), &mut t, "/x", "")).nearest, []);
}

#[test]
fn a_template_id_needs_something_there() {
    let s = built(vec![entry("GET", &format!("{A}/users/1"), 200, "{}")]);
    let mut t = Turns::default();
    assert!(matches!(get(&s, &mut t, "/users/", ""), Answer::NoMatch(_)));
    assert!(matches!(get(&s, &mut t, "/users", ""), Answer::NoMatch(_)));
    assert!(matches!(get(&s, &mut t, "/users/1/2", ""), Answer::NoMatch(_)));
}

#[test]
fn a_304_with_its_cached_body_answers_as_200() {
    let s = built(vec![entry("GET", &format!("{A}/c"), 304, r#"{"cached":true}"#), entry("GET", &format!("{A}/e"), 304, "")]);
    let c = &s.routes[0].samples[0];
    assert_eq!((c.status, c.was_not_modified), (200, true));
    let e = &s.routes[1].samples[0];
    assert_eq!((e.status, e.was_not_modified), (304, false), "nothing to stand in for it");
}

#[test]
fn a_304_without_its_cached_body_has_no_replayable_answer() {
    for recorded in [false, true] {
        let mut e = entry("GET", &format!("{A}/cached"), 304, "");
        if !recorded {
            e["response"]["content"].as_object_mut().unwrap().remove("text");
        }
        let s = built(vec![e]);
        let no = missing(get(&s, &mut Turns::default(), "/cached", ""));
        assert_eq!(serde_json::to_value(no.reason).unwrap(), "cached_body_unavailable");
        assert_eq!(no.nearest, [NearbyRoute { method: "GET".into(), template: "/cached".into() }]);
    }
}

#[test]
fn polling_skips_304_samples_without_a_cached_body() {
    let s = built(vec![
        entry("GET", &format!("{A}/poll"), 304, ""),
        entry("GET", &format!("{A}/poll"), 200, r#"{"n":1}"#),
        entry("GET", &format!("{A}/poll"), 304, ""),
        entry("GET", &format!("{A}/poll"), 200, r#"{"n":2}"#),
    ]);
    let mut turns = Turns::default();
    for expected in [1, 2, 1, 2] {
        let f = found(get(&s, &mut turns, "/poll", ""));
        assert_eq!((f.status, f.of), (200, 2));
        assert_eq!(body(&f), format!("{{\"n\":{expected}}}"));
    }
}

#[test]
fn a_missing_cached_answer_does_not_fall_back_to_a_different_query_or_id() {
    let s = built(vec![
        entry("GET", &format!("{A}/users/1?view=summary"), 304, ""),
        entry("GET", &format!("{A}/users/1?view=details"), 200, r#"{"details":true}"#),
        entry("GET", &format!("{A}/users/2?view=summary"), 200, r#"{"id":2}"#),
    ]);
    let no = missing(get(&s, &mut Turns::default(), "/users/1", "view=summary"));
    assert_eq!(serde_json::to_value(no.reason).unwrap(), "cached_body_unavailable");
    assert_eq!(body(&found(get(&s, &mut Turns::default(), "/users/2", "view=summary"))), r#"{"id":2}"#);
}

#[test]
fn a_missing_cached_graphql_answer_keeps_its_operation() {
    let mut cached = entry("POST", &format!("{A}/graphql"), 304, "");
    cached["request"]["postData"] = json!({"text": r#"{"operationName":"GetUser"}"#});
    let mut other = entry("POST", &format!("{A}/graphql"), 200, r#"{"feed":[]}"#);
    other["request"]["postData"] = json!({"text": r#"{"operationName":"GetFeed"}"#});
    let s = built(vec![cached, other]);
    let no = missing(answer(&s, &mut Turns::default(), "POST", "/graphql", "", br#"{"operationName":"GetUser"}"#));
    assert_eq!(serde_json::to_value(no.reason).unwrap(), "cached_body_unavailable");
    assert_eq!(body(&found(answer(&s, &mut Turns::default(), "POST", "/graphql", "", br#"{"operationName":"GetFeed"}"#))), r#"{"feed":[]}"#);
}

#[test]
fn a_304_whose_cached_body_was_left_out_is_unavailable() {
    let mut invalid = entry("GET", &format!("{A}/cached"), 304, "@@@");
    invalid["response"]["content"]["encoding"] = json!("base64");
    let fake_pem_header = ["-----BEGIN ", "PRIVATE KEY-----"].concat();
    let cases = [
        (entry("GET", &format!("{A}/cached"), 304, "large"), BuildOptions { max_body_bytes: 1, ..Default::default() }, BodyLeftOut::TooLarge),
        (entry("GET", &format!("{A}/cached"), 304, "{}"), BuildOptions { max_total_body_bytes: 1, ..Default::default() }, BodyLeftOut::FileLimit),
        (invalid, BuildOptions::default(), BodyLeftOut::Undecodable),
        (entry("GET", &format!("{A}/cached"), 304, &fake_pem_header), BuildOptions::default(), BodyLeftOut::HeldSecret),
    ];
    for (e, options, reason) in cases {
        let s = build_from_bytes(&har_of(vec![e]), &options).unwrap();
        assert_eq!(s.routes[0].samples[0].body_left_out, Some(reason));
        let no = missing(get(&s, &mut Turns::default(), "/cached", ""));
        assert_eq!(serde_json::to_value(no.reason).unwrap(), "cached_body_unavailable");
    }
}

#[test]
fn cached_text_and_binary_304_bodies_still_replay_as_200() {
    let mut binary = entry("GET", &format!("{A}/binary"), 304, "//4=");
    binary["response"]["content"]["mimeType"] = json!("application/octet-stream");
    binary["response"]["content"]["encoding"] = json!("base64");
    let s = built(vec![entry("GET", &format!("{A}/text"), 304, r#"{"cached":true}"#), binary]);
    let text = found(get(&s, &mut Turns::default(), "/text", ""));
    assert_eq!((text.status, body(&text).as_str()), (200, r#"{"cached":true}"#));
    let binary = found(get(&s, &mut Turns::default(), "/binary", ""));
    assert_eq!((binary.status, binary.body.as_slice()), (200, &b"\xff\xfe"[..]));
}

#[test]
fn an_existing_v1_file_does_not_replay_a_bare_304() {
    let s: StandIn = serde_json::from_str(
        r#"{"version":1,"routes":[{"method":"GET","template":"/cached","samples":[{"path":"/cached","status":304,"body_left_out":"not_recorded"}]}]}"#,
    )
    .unwrap();
    let no = missing(get(&s, &mut Turns::default(), "/cached", ""));
    assert_eq!(serde_json::to_value(no.reason).unwrap(), "cached_body_unavailable");
    let controls = built(vec![entry("GET", &format!("{A}/empty"), 200, ""), entry("GET", &format!("{A}/no-content"), 204, "")]);
    assert_eq!(found(get(&controls, &mut Turns::default(), "/empty", "")).status, 200);
    assert_eq!(found(get(&controls, &mut Turns::default(), "/no-content", "")).status, 204);
}

#[test]
fn a_saved_stand_in_reads_back_the_same() {
    let opts = BuildOptions { source_name: Some("site.har".into()), source_sha256: Some("abc".into()), ..Default::default() };
    let s = build_from_bytes(&secret_recording(), &opts).unwrap();
    let text = serde_json::to_string(&s).unwrap();
    let back: StandIn = serde_json::from_str(&text).unwrap();
    assert_eq!(back, s);
    assert_eq!(back.version, STANDIN_VERSION);
    assert_eq!(back.source.file_name.as_deref(), Some("site.har"));
}

#[test]
fn a_hand_edited_file_cannot_crash_the_matcher() {
    let v = json!({"version": 1, "routes": [
        {"method": "GET", "template": "/a", "samples": []},
        {"method": "GET", "template": "/b", "samples": [{"path": "/b", "status": 200, "base64": "@@@"}]},
    ]});
    let s: StandIn = serde_json::from_value(v).unwrap();
    let mut t = Turns::default();
    assert!(matches!(get(&s, &mut t, "/a", ""), Answer::NoMatch(_)), "a route with no samples has nothing to say");
    assert_eq!(found(get(&s, &mut t, "/b", "")).body, b"");
}

#[test]
fn a_read_after_the_clients_write_skips_to_what_followed_it() {
    let s = built(vec![
        entry("GET", &format!("{A}/notes"), 200, r#"{"notes":[]}"#),
        entry("POST", &format!("{A}/notes"), 201, r#"{"id":7}"#),
        entry("PATCH", &format!("{A}/notes/7"), 200, "{}"),
        entry("GET", &format!("{A}/notes"), 200, r#"{"notes":[7]}"#),
    ]);
    let mut t = Turns::default();
    let _ = answer(&s, &mut t, "POST", "/notes", "", b"");
    // Nothing was read between the post and the autosave after it: the next read shows the note.
    assert_eq!(read_notes(&s, &mut t), r#"{"notes":[7]}"#);
}

#[test]
fn a_write_reaches_reads_whose_template_was_worked_out_apart() {
    let s = built(vec![
        entry("GET", &format!("{A}/notes/a"), 200, r#"{"note":"a"}"#),
        entry("GET", &format!("{A}/notes/b"), 200, r#"{"note":"b"}"#),
        entry("GET", &format!("{A}/notes/c"), 200, r#"{"note":"c"}"#),
        entry("DELETE", &format!("{A}/notes/b"), 204, ""),
        entry("GET", &format!("{A}/notes/b"), 200, r#"{"note":"b, edited elsewhere"}"#),
    ]);
    assert!(templates(&s).contains(&"GET /notes/{id}".to_string()) && templates(&s).contains(&"DELETE /notes/b".to_string()), "{:?}", templates(&s));
    let mut t = Turns::default();
    for _ in 0..2 {
        assert_eq!(body(&found(get(&s, &mut t, "/notes/b", ""))), r#"{"note":"b"}"#, "the delete hasn't happened yet");
    }
    let _ = answer(&s, &mut t, "DELETE", "/notes/b", "", b"");
    assert_eq!(body(&found(get(&s, &mut t, "/notes/b", ""))), r#"{"note":"b, edited elsewhere"}"#);
}

#[test]
fn a_named_operation_that_was_not_read_is_not_taken_for_a_query() {
    let many: String = (0..80).map(|i| format!("query Q{i} {{ a }} ")).collect();
    let doc = format!("{many} mutation AddNote {{ add }}");
    let v = json!({"query": doc, "operationName": "AddNote"});
    assert!(!graphql_reads(&v), "the mutation may be past what was read");
    assert!(graphql_reads(&json!({"query": doc, "operationName": "Q3"})));
}

#[test]
fn a_write_kept_as_an_unavailable_304_ends_no_reads() {
    let s = built(vec![
        entry("GET", &format!("{A}/notes"), 200, r#"{"notes":[]}"#),
        entry("PUT", &format!("{A}/notes"), 304, ""),
        entry("GET", &format!("{A}/notes"), 200, r#"{"notes":[7]}"#),
    ]);
    let put = s.routes.iter().find(|r| r.method == "PUT").unwrap();
    assert_eq!(put.samples[0].status, 304, "kept as evidence");
    assert!(matches!(answer(&s, &mut Turns::default(), "PUT", "/notes", "", b""), Answer::NoMatch(_)), "no client can replay it");
    let mut t = Turns::default();
    let reads: Vec<String> = (0..3).map(|_| read_notes(&s, &mut t)).collect();
    assert_eq!(reads, [r#"{"notes":[]}"#, r#"{"notes":[7]}"#, r#"{"notes":[]}"#], "both reads go round as if it weren't there");
    // A replayable write between the same reads still ends the first one.
    let s = built(vec![
        entry("GET", &format!("{A}/notes"), 200, r#"{"notes":[]}"#),
        entry("PUT", &format!("{A}/notes"), 200, "{}"),
        entry("GET", &format!("{A}/notes"), 200, r#"{"notes":[7]}"#),
    ]);
    let mut t = Turns::default();
    assert_eq!([read_notes(&s, &mut t), read_notes(&s, &mut t)], [r#"{"notes":[]}"#, r#"{"notes":[]}"#]);
}

#[test]
fn a_write_to_a_path_with_an_empty_segment_moves_only_its_own_reads() {
    let s = built(vec![
        entry("GET", &format!("{A}/api/notes"), 200, r#"{"notes":[]}"#),
        entry("POST", &format!("{A}/api//notes"), 201, "{}"),
        entry("GET", &format!("{A}/api/notes"), 200, r#"{"notes":[7]}"#),
    ]);
    assert!(templates(&s).contains(&"POST /api//notes".to_string()), "{:?}", templates(&s));
    let mut t = Turns::default();
    let _ = answer(&s, &mut t, "POST", "/api//notes", "", b"");
    assert_eq!(body(&found(get(&s, &mut t, "/api/notes", ""))), r#"{"notes":[]}"#, "replay answers the two apart, so the write isn't this read's");
    for (a, b, yes) in [
        ("/api//notes", "/api/notes", false),
        ("/api//notes", "/api//notes/{id}", true),
        ("/api//notes", "/api", true),
        ("/notes/", "/notes/{id}", true),
        ("/", "/", true),
        ("", "/a", true),
    ] {
        assert_eq!((related(a, b), related(b, a)), (yes, yes), "{a} {b}");
    }
}

#[test]
fn only_a_post_carrying_a_graphql_query_counts_as_a_read() {
    let patch = |status: i64| {
        let mut e = entry("PATCH", &format!("{A}/items/7"), status, "{}");
        e["request"]["postData"] = json!({"mimeType": "application/json", "text": r#"{"query":"{ title }"}"#});
        e
    };
    let s = built(vec![
        entry("GET", &format!("{A}/items/7"), 200, r#"{"title":"old"}"#),
        patch(200),
        entry("GET", &format!("{A}/items/7"), 200, r#"{"title":"new"}"#),
    ]);
    let patched = s.routes.iter().find(|r| r.method == "PATCH").unwrap();
    assert!(patched.samples[0].write, "a REST field named query is still a write");
    let mut t = Turns::default();
    let _ = answer(&s, &mut t, "PATCH", "/items/7", "", br#"{"query":"{ title }"}"#);
    assert_eq!(body(&found(get(&s, &mut t, "/items/7", ""))), r#"{"title":"new"}"#);
    let query = json!({"query": "{ title }"});
    assert!(is_write("PATCH", Some(&query)) && is_write("PUT", Some(&query)) && is_write("DELETE", Some(&query)));
    assert!(!is_write("POST", Some(&query)) && !is_write("GET", Some(&query)));
}

#[test]
fn a_named_operation_answered_by_another_ones_recording_moves_nothing() {
    let mut add = entry("POST", &format!("{A}/graphql"), 200, r#"{"data":{"added":1}}"#);
    add["request"]["postData"] = json!({"text": r#"{"query":"mutation PrivateAdd { add }","operationName":"PrivateAdd"}"#});
    let entries = vec![entry("GET", &format!("{A}/graphql"), 200, r#"{"n":0}"#), add, entry("GET", &format!("{A}/graphql"), 200, r#"{"n":1}"#)];
    let opts = BuildOptions { hidden_values: vec!["PrivateAdd".into()], ..Default::default() };
    let s = build_from_bytes(&har_of(entries.clone()), &opts).unwrap();
    let post = s.routes.iter().find(|r| r.method == "POST").unwrap();
    assert_eq!(post.samples[0].request_key, None, "the hidden name stays out of the file");
    let mut t = Turns::default();
    let clear = br#"{"query":"mutation Clear { clear }","operationName":"Clear"}"#;
    // The recording can't say the hidden write was this one: no match, no answer.
    assert_eq!(missing(answer(&s, &mut t, "POST", "/graphql", "", clear)).reason, NoMatchReason::WriteNotRecorded);
    assert_eq!(t.position(), None);
    assert_eq!(body(&found(get(&s, &mut t, "/graphql", ""))), r#"{"n":0}"#, "later reads stay where they were");
    // Recorded by name, the same operation still steps.
    let s = built(entries);
    let mut t = Turns::default();
    let private = br#"{"query":"mutation PrivateAdd { add }","operationName":"PrivateAdd"}"#;
    let _ = answer(&s, &mut t, "POST", "/graphql", "", private);
    assert_eq!(t.position(), Some(1));
    assert_eq!(body(&found(get(&s, &mut t, "/graphql", ""))), r#"{"n":1}"#);
}

#[test]
fn a_mutation_sent_as_text_is_not_answered_by_a_recorded_read() {
    let mut list = entry("POST", &format!("{A}/graphql"), 200, r#"{"data":{"list":[1]}}"#);
    list["request"]["postData"] = json!({"text": r#"{"query":"query HiddenList { list }","operationName":"HiddenList"}"#});
    let opts = BuildOptions { hidden_values: vec!["HiddenList".into()], ..Default::default() };
    let s = build_from_bytes(&har_of(vec![list]), &opts).unwrap();
    let mut t = Turns::default();
    let clear = br#"{"query":"mutation Clear { clear }"}"#;
    assert_eq!(missing(answer(&s, &mut t, "POST", "/graphql", "", clear)).reason, NoMatchReason::WriteNotRecorded);
    assert_eq!(t.position(), None, "and it moves nothing");
    // Named only, a request can't be told from a read, so the recorded read still answers it.
    assert_eq!(body(&found(answer(&s, &mut t, "POST", "/graphql", "", br#"{"operationName":"HiddenList"}"#))), r#"{"data":{"list":[1]}}"#);
    let query = br#"{"query":"query Other { list }"}"#;
    assert_eq!(body(&found(answer(&s, &mut t, "POST", "/graphql", "", query))), r#"{"data":{"list":[1]}}"#, "nor is a query sent as text refused");
}

#[test]
fn only_writes_are_counted_as_writes() {
    assert!(is_write_request("post", br#"{"query":"mutation Clear { clear }"}"#));
    assert!(is_write_request("DELETE", b""));
    assert!(is_write_request("POST", br#"{"operationName":"List"}"#), "a name alone could be a write");
    assert!(!is_write_request("POST", br#"{"query":"query List { list }"}"#));
    assert!(!is_write_request("GET", b""));
    assert!(!is_write_request("HEAD", b""));
    let big = format!(r#"{{"query":"query List {{ list }}","pad":"{}"}}"#, "x".repeat(MAX_KEY_BODY));
    assert!(!is_write_request("POST", big.as_bytes()), "too large to read, it could be a query: unanswered, not a refused write");
    assert!(is_write_request("PUT", big.as_bytes()));
}

#[test]
fn a_batch_naming_a_read_beside_a_query_sent_as_text_gets_the_recorded_read() {
    let mut batch = entry("POST", &format!("{A}/graphql"), 200, r#"[{"data":{"a":1}},{"data":{"b":2}}]"#);
    batch["request"]["postData"] = json!({"text": r#"[{"query":"query A { a }","operationName":"A"},{"query":"query B { b }","operationName":"B"}]"#});
    let s = built(vec![batch]);
    let mut t = Turns::default();
    let mixed = br#"[{"operationName":"A"},{"operationName":"B","query":"query B { b }"}]"#;
    assert_eq!(body(&found(answer(&s, &mut t, "POST", "/graphql", "", mixed))), r#"[{"data":{"a":1}},{"data":{"b":2}}]"#);
    assert_eq!(t.position(), None);
    // A mutation's own text in the batch still makes it a write nobody recorded.
    let with_write = br#"[{"operationName":"A"},{"operationName":"B","query":"mutation B { b }"}]"#;
    assert_eq!(missing(answer(&s, &mut t, "POST", "/graphql", "", with_write)).reason, NoMatchReason::WriteNotRecorded);
}

#[test]
fn a_file_from_before_version_2_gives_a_write_each_recorded_answer_once() {
    let mut list = entry("POST", &format!("{A}/graphql"), 200, r#"{"data":{"list":[]}}"#);
    list["request"]["postData"] = json!({"text": r#"{"operationName":"List"}"#});
    let mut s = built(vec![
        entry("GET", &format!("{A}/todos"), 200, "[]"),
        entry("POST", &format!("{A}/todos"), 201, r#"{"id":1}"#),
        entry("POST", &format!("{A}/todos"), 201, r#"{"id":2}"#),
        list,
    ]);
    // What a version-1 file migrates to: no places, nothing marked as a write.
    for sample in s.routes.iter_mut().flat_map(|r| r.samples.iter_mut()) {
        (sample.order, sample.write) = (None, false);
    }
    let mut t = Turns::default();
    let adds: Vec<String> = (0..2).map(|_| body(&found(answer(&s, &mut t, "POST", "/todos", "", b"{}")))).collect();
    assert_eq!(adds, [r#"{"id":1}"#, r#"{"id":2}"#]);
    for _ in 0..2 {
        assert_eq!(missing(answer(&s, &mut t, "POST", "/todos", "", b"{}")).reason, NoMatchReason::WriteNotRecorded, "past them, none");
    }
    assert_eq!(body(&found(get(&s, &mut t, "/todos", ""))), "[]", "reads still answer");
    // Reads polled first don't use up a write's answers.
    let mut ask = entry("POST", &format!("{A}/gql"), 200, r#"{"data":{"n":0}}"#);
    ask["request"]["postData"] = json!({"text": r#"{"query":"query { n }"}"#});
    let mut bump = entry("POST", &format!("{A}/gql"), 200, r#"{"data":{"bumped":true}}"#);
    bump["request"]["postData"] = json!({"text": r#"{"query":"mutation { bump }"}"#});
    let mut anon = built(vec![ask, bump]);
    for sample in anon.routes.iter_mut().flat_map(|r| r.samples.iter_mut()) {
        (sample.order, sample.write) = (None, false);
    }
    let mut t2 = Turns::default();
    for _ in 0..3 {
        let _ = found(answer(&anon, &mut t2, "POST", "/gql", "", br#"{"query":"query { n }"}"#));
    }
    let _ = found(answer(&anon, &mut t2, "POST", "/gql", "", br#"{"query":"mutation { bump }"}"#));
    // Named only, a request may be a read, so it still takes its turns.
    for _ in 0..3 {
        assert_eq!(body(&found(answer(&s, &mut t, "POST", "/graphql", "", br#"{"operationName":"List"}"#))), r#"{"data":{"list":[]}}"#);
    }
}

#[test]
fn a_body_of_another_kind_steps_each_recorded_kind_on_its_own() {
    let post = |req: &str, resp: &str| {
        let mut e = entry("POST", &format!("{A}/search"), 200, resp);
        e["request"]["postData"] = json!({"mimeType": "application/json", "text": req});
        e
    };
    let s = built(vec![post(r#"{"q":"a"}"#, r#"{"by":"q"}"#), post(r#"{"tag":"b"}"#, r#"{"by":"tag 1"}"#), post(r#"{"tag":"c"}"#, r#"{"by":"tag 2"}"#)]);
    let mut t = Turns::default();
    let other: Vec<String> = (0..3).map(|_| body(&found(answer(&s, &mut t, "POST", "/search", "", br#"{"new":1}"#)))).collect();
    assert_eq!(other, [r#"{"by":"q"}"#, r#"{"by":"tag 1"}"#, r#"{"by":"tag 2"}"#]);
    assert_eq!(missing(answer(&s, &mut t, "POST", "/search", "", br#"{"new":1}"#)).reason, NoMatchReason::WriteNotRecorded, "past them all, none");
    // A body of a recorded kind still takes only its own.
    let mut t = Turns::default();
    let tags: Vec<String> = (0..2).map(|_| body(&found(answer(&s, &mut t, "POST", "/search", "", br#"{"tag":"x"}"#)))).collect();
    assert_eq!(tags, [r#"{"by":"tag 1"}"#, r#"{"by":"tag 2"}"#]);
    assert_eq!(missing(answer(&s, &mut t, "POST", "/search", "", br#"{"tag":"x"}"#)).reason, NoMatchReason::WriteNotRecorded);
}

#[test]
fn a_read_answered_before_its_time_takes_no_turn_from_the_reads_after_the_write() {
    let s = built(vec![
        entry("POST", &format!("{A}/job"), 202, r#"{"started":true}"#),
        entry("GET", &format!("{A}/job"), 200, r#"{"state":"running"}"#),
        entry("GET", &format!("{A}/job"), 200, r#"{"state":"done"}"#),
    ]);
    let mut t = Turns::default();
    let early = found(get(&s, &mut t, "/job", ""));
    assert_eq!((body(&early).as_str(), early.turn, early.of), (r#"{"state":"running"}"#, 1, 1));
    let _ = answer(&s, &mut t, "POST", "/job", "", b"");
    let seen: Vec<String> = (0..3).map(|_| body(&found(get(&s, &mut t, "/job", "")))).collect();
    assert_eq!(seen, [r#"{"state":"running"}"#, r#"{"state":"done"}"#, r#"{"state":"running"}"#]);
}
