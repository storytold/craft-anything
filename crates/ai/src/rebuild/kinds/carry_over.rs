//! What plain files can't carry over from a captured website, said after its build: the forms,
//! sign-ins, payments and bookings the capture saw, the ones it didn't (which isn't proof the site
//! has none), and the pictures the owner has to supply, since a capture keeps no picture files.

use super::website::routes;
use super::*;

/// Pictures named one by one; the rest are counted.
const MAX_PICTURES: usize = 8;

#[derive(Clone, Copy, PartialEq)]
enum Part {
    SignIn,
    Payment,
    Booking,
}

impl Part {
    const ALL: [Part; 3] = [Part::SignIn, Part::Payment, Part::Booking];

    fn name(self) -> &'static str {
        match self {
            Part::SignIn => "sign-ins",
            Part::Payment => "payments",
            Part::Booking => "bookings",
        }
    }

    /// Words in a link, a form's address or a field that point at this part.
    fn words(self) -> &'static [&'static str] {
        match self {
            Part::SignIn => &["login", "signin", "logon", "password", "account"],
            Part::Payment => &["checkout", "payment", "pay", "cart", "basket", "buy", "donate", "card", "cvc", "cvv"],
            Part::Booking => &["book", "booking", "reserve", "reservation", "appointment"],
        }
    }

    fn in_text(self, text: &str) -> bool {
        let lower = text.to_lowercase();
        // "Sign in" and "log in" as one word, so they match like "signin" and "login".
        let joined = lower.replace("sign in", "signin").replace("sign-in", "signin").replace("log in", "login").replace("log-in", "login");
        joined.split(|c: char| !c.is_alphanumeric()).any(|w| self.words().contains(&w))
    }

    fn in_field(self, field: &Value) -> bool {
        let s = |k: &str| field.get(k).and_then(Value::as_str).unwrap_or_default();
        match (self, s("type")) {
            (Part::SignIn, "password") => true,
            (Part::Booking, "date" | "time" | "datetime-local") => true,
            _ => ["name", "label", "placeholder"].iter().any(|k| self.in_text(s(k))),
        }
    }
}

/// A captured page: where it is on the site and its `capture.json`.
pub struct Page {
    pub path: String,
    pub capture: Value,
}

/// The start page and the other captured pages of a project, as far as their captures can be read.
pub fn pages(ws: &Workspace) -> Vec<Page> {
    let read = |dir: &Path| std::fs::read(dir.join("capture.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let capture = ws.capture_dir();
    let start_path = match &ws.manifest.target {
        TargetRef::Website { url } => path_of(url),
        _ => "/".into(),
    };
    let start = read(&capture).map(|c| Page { path: start_path, capture: c });
    let others = routes(ws).into_iter().filter_map(|r| read(&capture.join("routes").join(&r.slug)).map(|c| Page { path: r.path, capture: c }));
    start.into_iter().chain(others).collect()
}

/// The path of a web address (`https://a.com/shop?x=1` is `/shop`), `/` for the site's root.
fn path_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    rest.find('/').map_or_else(|| "/".into(), |at| rest[at..].to_string())
}

fn list<'a>(v: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    v.get(key).and_then(Value::as_array).into_iter().flatten()
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// A picture's address without its query or fragment, which can carry ids.
fn bare(src: &str) -> &str {
    if src.starts_with("data:") { "an inline picture" } else { src.split(['?', '#']).next().unwrap_or_default() }
}

/// What the build can't carry over from `pages`, in plain lines for the person; None when nothing
/// was captured.
pub fn carry_over(pages: &[Page]) -> Option<String> {
    if pages.is_empty() {
        return None;
    }
    let mut seen: Vec<String> = Vec::new();
    let mut unseen: Vec<&str> = Vec::new();
    let forms: Vec<(&str, &Value)> = pages.iter().flat_map(|p| list(&p.capture, "forms").map(move |f| (p.path.as_str(), f))).collect();
    let in_form = |part: Part, f: &Value| part.in_text(text(f, "action")) || list(f, "fields").any(|x| part.in_field(x));
    for part in Part::ALL {
        let in_forms: Vec<&str> = forms.iter().filter(|(_, f)| in_form(part, f)).map(|(p, _)| *p).collect();
        let links: Vec<&str> = pages.iter().flat_map(|p| list(&p.capture, "links")).map(|l| text(l, "text")).filter(|t| part.in_text(t)).collect();
        // A captured sign-in page is one even when its form wasn't read.
        let sign_in_page = part == Part::SignIn && pages.iter().any(|p| p.capture.get("sign_in_page") == Some(&Value::Bool(true)));
        if in_forms.is_empty() && links.is_empty() && !sign_in_page {
            unseen.push(part.name());
            continue;
        }
        let mut found_in: Vec<String> = Vec::new();
        if !in_forms.is_empty() {
            found_in.push(format!("{} form(s) on {}", in_forms.len(), dedup(&in_forms).join(", ")));
        } else if sign_in_page {
            found_in.push("a page that asks for a password".into());
        }
        if let Some(first) = links.iter().find(|t| !t.trim().is_empty()) {
            found_in.push(format!("{} link(s) such as \"{}\"", links.len(), first.trim()));
        }
        seen.push(format!("{} ({})", part.name(), found_in.join("; ")));
    }
    let other: Vec<&str> = forms.iter().filter(|(_, f)| !Part::ALL.iter().any(|p| in_form(*p, f))).map(|(p, _)| *p).collect();
    if !other.is_empty() {
        seen.push(format!("{} other form(s) on {}", other.len(), dedup(&other).join(", ")));
    }

    let n = pages.len();
    let mut s = format!("What a build made of plain files can't carry over, from the capture of {n} page(s):\n");
    if seen.is_empty() {
        s.push_str("- Seen: no forms, sign-ins, payments or bookings.\n");
    } else {
        s.push_str(&format!(
            "- Seen, not carried over as working features: {}. The build copies how they look; copied form markup isn't a working form, so each needs a service of its own to send, sign in, pay or book.\n",
            seen.join(", ")
        ));
    }
    if !unseen.is_empty() {
        s.push_str(&format!(
            "- Not seen: {}. The capture looked at {n} page(s) in one state, without using them, so that isn't proof the site has none.\n",
            unseen.join(", ")
        ));
    }
    s.push_str(&pictures(pages));
    Some(s)
}

fn dedup<'a>(paths: &[&'a str]) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    for p in paths {
        if !out.contains(p) {
            out.push(p);
        }
    }
    out
}

/// The pictures the owner supplies: each one's address, size and alt text from the capture. A
/// clean-room capture keeps only sizes in `assets.images`, so its outline gives the addresses.
fn pictures(pages: &[Page]) -> String {
    // Each picture's full address (the key, as a query can tell two pictures apart), then the
    // address shown, its size and alt text.
    let mut seen: Vec<(String, String, String, String)> = Vec::new();
    for p in pages {
        let assets: Vec<&Value> =
            p.capture.pointer("/assets/images").and_then(Value::as_array).into_iter().flatten().filter(|i| !text(i, "src").is_empty()).collect();
        let found: Vec<(String, String, String, String)> = if assets.is_empty() {
            list(&p.capture, "outline")
                .filter(|i| text(i, "tag") == "img" && !text(i, "src").is_empty())
                .map(|i| {
                    let size = |k: &str| i.pointer(&format!("/box/{k}")).and_then(Value::as_u64).unwrap_or(0);
                    let src = text(i, "src");
                    (src.to_string(), bare(src).to_string(), format!("shown at {}×{}", size("w"), size("h")), text(i, "alt").to_string())
                })
                .collect()
        } else {
            assets
                .iter()
                .map(|i| {
                    let size = |k: &str| i.get(k).and_then(Value::as_u64).unwrap_or(0);
                    let src = text(i, "src");
                    (src.to_string(), bare(src).to_string(), format!("{}×{}", size("w"), size("h")), text(i, "alt").to_string())
                })
                .collect()
        };
        for f in found {
            if !seen.iter().any(|s| s.0 == f.0) {
                seen.push(f);
            }
        }
    }
    if seen.is_empty() {
        return "- Pictures: none seen in the captured pages.".into();
    }
    let named: Vec<String> = seen
        .iter()
        .take(MAX_PICTURES)
        .map(|(_, src, size, alt)| if alt.is_empty() { format!("{src} ({size}, no alt text)") } else { format!("{src} ({size}, alt \"{alt}\")") })
        .collect();
    let more = seen.len().saturating_sub(MAX_PICTURES);
    let more = if more == 0 { String::new() } else { format!("; and {more} more in the captured pages' capture.json files") };
    format!(
        "- Pictures the owner supplies: {} seen. The capture saved no picture files, only their addresses, sizes and alt text, so the build uses placeholders for photos and logos unless you confirm you own them; fetching or publishing each one is a separate yes. {}{more}.",
        seen.len(),
        named.join("; ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(path: &str, capture: Value) -> Page {
        Page { path: path.into(), capture }
    }

    #[test]
    fn what_the_capture_saw_is_kept_apart_from_what_it_did_not() {
        let pages = [
            page(
                "/",
                json!({
                    "forms": [{"action": "/subscribe", "fields": [{"type": "email", "name": "email"}]}],
                    "links": [{"text": "Log in", "href": "https://a.com/login"}, {"text": "About", "href": "https://a.com/about"}],
                    "assets": {"images": [{"src": "https://a.com/hero.jpg?v=3", "w": 1600, "h": 900, "alt": "Our team"}]}
                }),
            ),
            page("/shop", json!({"forms": [{"action": "/checkout", "fields": [{"type": "text", "name": "card-number"}]}], "links": []})),
        ];
        let s = carry_over(&pages).unwrap();
        let (seen, rest) = s.split_once("- Not seen").expect(&s);
        for part in ["sign-ins", "payments", "1 other form(s) on /", "on /shop"] {
            assert!(seen.contains(part), "{part} seen in {s}");
        }
        assert!(rest.contains("bookings") && rest.contains("isn't proof"), "{s}");
        assert!(!rest.contains("payments") && !rest.contains("sign-ins"), "{s}");
        let pictures = s.lines().find(|l| l.contains("hero.jpg")).expect(&s);
        assert!(pictures.contains("1600×900") && pictures.contains("Our team"), "address, size and alt text: {s}");
        assert!(!s.contains("v=3"), "no query string: {s}");
        assert!(s.contains("placeholders") && s.contains("separate yes"), "{s}");
    }

    #[test]
    fn a_capture_without_forms_or_pictures_proves_nothing_missing() {
        let s = carry_over(&[page("/", json!({"links": [{"text": "About"}], "forms": []}))]).unwrap();
        assert!(s.contains("Seen: no forms"), "{s}");
        let not_seen = s.lines().find(|l| l.starts_with("- Not seen")).expect(&s);
        for part in ["sign-ins", "payments", "bookings", "isn't proof"] {
            assert!(not_seen.contains(part), "{part} in {s}");
        }
        assert!(s.contains("Pictures: none seen"), "{s}");
        assert!(carry_over(&[]).is_none(), "no capture says nothing");
    }

    /// A clean-room capture keeps picture sizes only in `assets.images`; the addresses and alt
    /// text are in its outline.
    #[test]
    fn a_clean_room_capture_names_its_pictures_from_the_outline() {
        let c = json!({
            "assets": {"images": [{"w": 200, "h": 80}]},
            "outline": [{"tag": "img", "src": "https://a.com/logo.png", "alt": "Logo", "box": {"x": 0, "y": 0, "w": 100, "h": 40}}],
            "sign_in_page": true
        });
        let s = carry_over(&[page("/", c)]).unwrap();
        let pictures = s.lines().find(|l| l.contains("logo.png")).expect(&s);
        assert!(pictures.contains("100×40") && pictures.contains("Logo"), "{s}");
        let seen = s.lines().find(|l| l.starts_with("- Seen")).expect(&s);
        assert!(seen.contains("sign-ins"), "a page asking for a password is a sign-in: {s}");
    }

    /// The start page is where the target points, not always the site's root.
    #[test]
    fn the_start_page_is_named_by_its_own_path() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Shop", "Clone it", TargetRef::Website { url: "https://a.com/shop?ref=ad".into() }).unwrap();
        std::fs::create_dir_all(ws.capture_dir()).unwrap();
        std::fs::write(ws.capture_dir().join("capture.json"), r#"{"forms": [{"action": "/checkout"}]}"#).unwrap();
        let s = carry_over(&pages(&ws)).unwrap();
        assert!(s.contains("1 form(s) on /shop)"), "{s}");
        assert_eq!(path_of("https://a.com"), "/");
    }

    /// Addresses are shown without their query, but two pictures told apart only by it are both
    /// listed, and those past the named ones are looked for in every captured page.
    #[test]
    fn pictures_told_apart_by_their_query_are_each_counted() {
        let img = |id: &str| json!({"src": format!("https://a.com/image?id={id}"), "w": 10, "h": 10});
        let start = page("/", json!({"assets": {"images": [img("hero"), img("logo"), img("hero")]}}));
        let s = carry_over(&[start]).unwrap();
        assert!(s.contains("Pictures the owner supplies: 2 seen"), "{s}");
        assert!(!s.contains("id="), "{s}");
        let many: Vec<Value> = (0..MAX_PICTURES + 1).map(|i| img(&i.to_string())).collect();
        let s = carry_over(&[page("/", json!({})), page("/shop", json!({"assets": {"images": many}}))]).unwrap();
        assert!(s.contains("1 more in the captured pages'"), "{s}");
    }

    #[test]
    fn a_word_inside_another_is_not_a_match() {
        assert!(!Part::Booking.in_text("Facebook"));
        assert!(!Part::Payment.in_text("Display"));
        assert!(Part::SignIn.in_text("Sign in"));
        assert!(Part::Booking.in_text("Book a table"));
    }

    #[test]
    fn a_project_reads_its_start_page_and_other_pages() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::create(d.path(), "Site", "Clone it", TargetRef::Website { url: "https://a.com".into() }).unwrap();
        assert!(pages(&ws).is_empty(), "nothing captured yet");
        let capture = ws.capture_dir();
        std::fs::create_dir_all(capture.join("routes/book")).unwrap();
        std::fs::write(capture.join("capture.json"), "{}").unwrap();
        std::fs::write(capture.join("routes/book/capture.json"), r#"{"forms": [{"fields": [{"type": "date"}]}]}"#).unwrap();
        std::fs::write(capture.join(super::super::website::ROUTES_FILE), r#"[{"path": "/book", "slug": "book", "url": "https://a.com/book"}]"#).unwrap();
        let s = carry_over(&pages(&ws)).unwrap();
        let seen = s.lines().find(|l| l.starts_with("- Seen")).expect(&s);
        assert!(seen.contains("bookings") && seen.contains("/book"), "the other page's form is read: {s}");
    }
}
