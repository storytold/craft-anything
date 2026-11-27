//! `web_site_open` with the person's own running browser (`cdp_endpoint`), against a stand-in
//! DevTools endpoint on 127.0.0.1 that already has one tab of the person's open. No real browser.

use craft_anything_platform::sync::Mutex;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

/// What the stand-in browser saw: commands on its own connection and the tabs connected to.
#[derive(Default)]
struct Seen {
    browser_calls: Vec<String>,
    tabs_connected: Vec<String>,
    created: bool,
    closed: Vec<String>,
    /// Refuse to close tabs, as a browser that stopped answering would.
    refuse_close: bool,
}

fn own_browser() -> (String, Arc<Mutex<Seen>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let shared = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let seen = shared.clone();
            std::thread::spawn(move || serve(stream, port, &seen));
        }
    });
    (format!("http://127.0.0.1:{port}"), seen)
}

fn serve(mut stream: TcpStream, port: u16, seen: &Mutex<Seen>) {
    let (path, key) = read_request(&mut stream);
    let ws = |p: &str| format!("ws://127.0.0.1:{port}{p}");
    let body = match path.as_str() {
        "/json/version" => json!({"Browser": "Chrome/150", "Protocol-Version": "1.3", "webSocketDebuggerUrl": ws("/devtools/browser/B")}),
        "/json/list" => {
            let s = seen.lock().unwrap();
            let mut tabs = vec![
                json!({"id": "PAGE1", "type": "page", "url": "https://mail.test/inbox", "title": "Inbox", "webSocketDebuggerUrl": ws("/devtools/page/PAGE1")}),
            ];
            if s.created && !s.closed.iter().any(|c| c == "NEW1") {
                tabs.push(json!({"id": "NEW1", "type": "page", "url": "about:blank", "title": "", "webSocketDebuggerUrl": ws("/devtools/page/NEW1")}));
            }
            json!(tabs)
        }
        _ => {
            let Some(key) = key else { return };
            let accept = crate::tools::browser_tools::tests::ws_accept(&key);
            let _ = write!(stream, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n");
            if let Some(tab) = path.strip_prefix("/devtools/page/") {
                seen.lock().unwrap().tabs_connected.push(tab.to_string());
            }
            serve_ws(&mut stream, &path, seen);
            return;
        }
    };
    let body = body.to_string();
    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
}

/// The request path and, for a WebSocket upgrade, its key.
fn read_request(stream: &mut TcpStream) -> (String, Option<String>) {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n == 1) {
        buf.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&buf).into_owned();
    let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
    let key = text.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("sec-websocket-key")).map(|(_, v)| v.trim().to_string()));
    (path, key)
}

fn serve_ws(stream: &mut TcpStream, path: &str, seen: &Mutex<Seen>) {
    while let Some(msg) = ws_read(stream) {
        let Ok(req) = serde_json::from_str::<Value>(&msg) else { continue };
        let method = req["method"].as_str().unwrap_or_default().to_string();
        if path.starts_with("/devtools/browser/") {
            let mut s = seen.lock().unwrap();
            s.browser_calls.push(method.clone());
            match method.as_str() {
                "Target.createTarget" => s.created = true,
                "Target.closeTarget" if !s.refuse_close => s.closed.push(req["params"]["targetId"].as_str().unwrap_or_default().to_string()),
                _ => {}
            }
        }
        if method == "Target.closeTarget" && seen.lock().unwrap().refuse_close {
            ws_write(stream, &json!({"id": req["id"], "error": {"code": -32000, "message": "busy"}}).to_string());
            continue;
        }
        let result = match method.as_str() {
            "Target.createTarget" => json!({"targetId": "NEW1"}),
            "Target.closeTarget" => json!({"success": true}),
            "Runtime.evaluate" => json!({"result": {"value": {"url": "https://app.test/home?code=abc", "title": "Home", "width": 1440, "height": 900}}}),
            _ => json!({}),
        };
        ws_write(stream, &json!({"id": req["id"], "result": result}).to_string());
        if method == "Page.navigate" {
            ws_write(stream, &json!({"method": "Page.loadEventFired", "params": {}}).to_string());
        }
    }
}

fn ws_read(stream: &mut TcpStream) -> Option<String> {
    let mut hdr = [0u8; 2];
    stream.read_exact(&mut hdr).ok()?;
    let mut len = (hdr[1] & 0x7f) as usize;
    if len == 126 {
        let mut e = [0u8; 2];
        stream.read_exact(&mut e).ok()?;
        len = u16::from_be_bytes(e) as usize;
    } else if len == 127 {
        let mut e = [0u8; 8];
        stream.read_exact(&mut e).ok()?;
        len = u64::from_be_bytes(e) as usize;
    }
    let mut mask = [0u8; 4];
    stream.read_exact(&mut mask).ok()?;
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).ok()?;
    for (i, b) in payload.iter_mut().enumerate() {
        *b ^= mask[i & 3];
    }
    (hdr[0] & 0x0f != 0x8).then(|| String::from_utf8_lossy(&payload).into_owned())
}

fn ws_write(stream: &mut TcpStream, text: &str) {
    let bytes = text.as_bytes();
    let mut frame = vec![0x81u8];
    if bytes.len() < 126 {
        frame.push(bytes.len() as u8);
    } else {
        frame.push(126);
        frame.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    }
    frame.extend_from_slice(bytes);
    let _ = stream.write_all(&frame);
}

/// A stand-in browser path gets past the engine check on any machine; nothing is started.
fn engine() -> crate::Engine {
    let mut s = crate::testing::session();
    s.settings.engines.tools.chromium = Some(std::env::current_exe().unwrap());
    crate::Engine::new(s)
}

fn call(e: &crate::Engine, id: &str, p: Value) -> Result<Value, craft_anything_evidence::ToolError> {
    e.call(id, p, crate::CallOptions::new(crate::Caller::Mcp).with_consent(crate::Consent::Granted)).map(|o| o.to_json())
}

/// The study opens and uses a tab of its own in the person's browser, never reads or closes their
/// tab, lists only its own tab to the other browser tools, and closes its tab at the end.
#[test]
fn a_study_in_the_persons_browser_uses_and_closes_its_own_tab() {
    let (endpoint, seen) = own_browser();
    let e = engine();
    let port = endpoint.rsplit(':').next().unwrap().to_string();
    let out = call(&e, "web_site_open", json!({"url": "https://app.test/home", "cdp_endpoint": port, "settle_ms": 0, "timeout_secs": 20})).unwrap();
    let r = &out["result"];
    assert_eq!(r["own_browser"], true, "{out}");
    assert_eq!(r["url"], "https://app.test/home", "the address loses its query, as signed in: {out}");
    assert!(out.to_string().contains("Opened in a new tab of your own browser"), "the limitation says whose browser it was: {out}");
    let listed = call(&e, "web_pages_list", json!({})).unwrap();
    let ids: Vec<&str> = listed["result"]["targets"].as_array().unwrap().iter().filter_map(|t| t["id"].as_str()).collect();
    assert_eq!(ids, ["NEW1"], "{listed}");
    assert_eq!(call(&e, "web_site_close", json!({})).unwrap()["result"]["closed"], true);
    let s = seen.lock().unwrap();
    assert!(s.browser_calls.contains(&"Target.createTarget".to_string()), "{:?}", s.browser_calls);
    assert_eq!(s.closed, ["NEW1"], "only our tab is closed");
    assert!(!s.tabs_connected.is_empty() && s.tabs_connected.iter().all(|t| t == "NEW1"), "the person's tab was read: {:?}", s.tabs_connected);
}

/// In the person's browser no tool reaches their other tabs, even when named, and nothing is set
/// that would reach them (blocking downloads holds for the whole browser).
#[test]
fn the_persons_other_tabs_are_out_of_reach() {
    let (endpoint, seen) = own_browser();
    let e = engine();
    call(&e, "web_site_open", json!({"url": "https://app.test/home", "cdp_endpoint": endpoint, "settle_ms": 0, "timeout_secs": 20})).unwrap();
    for tool in ["web_page_screenshot_take", "web_page_show", "web_coverage_measure"] {
        let err = call(&e, tool, json!({"target_id": "PAGE1"})).unwrap_err();
        assert!(err.message.contains("another tab of your own browser"), "{tool}: {err:?}");
    }
    let err = call(&e, "web_walkthrough_record", json!({"actions": [{"kind": "goto", "url": "https://app.test/"}], "capture": {"events": ["downloads"]}}))
        .unwrap_err();
    assert!(err.message.contains("downloads"), "{err:?}");
    let s = seen.lock().unwrap();
    assert!(s.tabs_connected.iter().all(|t| t == "NEW1"), "the person's tab was read: {:?}", s.tabs_connected);
}

/// A close the browser refuses leaves the study open, so trying again closes the tab.
#[test]
fn a_study_tab_that_wont_close_can_be_closed_again() {
    let (endpoint, seen) = own_browser();
    let e = engine();
    call(&e, "web_site_open", json!({"url": "https://app.test/home", "cdp_endpoint": endpoint, "settle_ms": 0, "timeout_secs": 20})).unwrap();
    seen.lock().unwrap().refuse_close = true;
    assert!(call(&e, "web_site_close", json!({})).is_err(), "a failed close isn't reported as closed");
    seen.lock().unwrap().refuse_close = false;
    assert_eq!(call(&e, "web_site_close", json!({})).unwrap()["result"]["closed"], true);
    assert_eq!(seen.lock().unwrap().closed, ["NEW1"]);
}

/// Only a loopback endpoint the person gave is used: another computer or a name is refused
/// before anything is reached, and a saved sign-in can't be asked for at the same time.
#[test]
fn the_persons_browser_must_be_on_this_computer() {
    let e = engine();
    for ep in ["http://192.168.1.5:9222", "http://example.com:9222", "ws://10.0.0.2:9222/devtools/browser/x", "http://127.0.0.1"] {
        let err = call(&e, "web_site_open", json!({"url": "https://app.test/", "cdp_endpoint": ep, "timeout_secs": 5})).unwrap_err();
        assert_eq!(err.code, craft_anything_evidence::ErrorCode::BadSettings, "{ep}: {err:?}");
    }
    let err = call(&e, "web_site_open", json!({"url": "https://app.test/", "cdp_endpoint": "9222", "signed_in": true})).unwrap_err();
    assert!(err.message.contains("cdp_endpoint"), "{err:?}");
}
