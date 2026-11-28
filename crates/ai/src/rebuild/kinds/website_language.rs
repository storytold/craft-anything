//! The website check's language policy, from explicit requests by the person. A declaration
//! is observable from outside the site; it does not prove that the content was translated.
//! Non-wildcard language-range matching follows RFC 4647, section 3.3.1:
//! <https://www.rfc-editor.org/rfc/rfc4647#section-3.3.1>. Tests use original synthetic captures.

use super::*;
use craft_anything_engine::workspace::{Claim, Manifest, WebsiteLanguage};
use icu_locale_core::Locale;
use regex::Regex;
use std::sync::LazyLock;

struct LanguageName {
    tag: &'static str,
    names: &'static [&'static str],
}

// Request vocabulary, not a language-tag registry. Explicit tags use ICU's existing parser.
const LANGUAGES: &[LanguageName] = &[
    LanguageName { tag: "en", names: &["english", "英语", "英語"] },
    LanguageName { tag: "fr", names: &["french", "français", "francais", "法语", "法語"] },
    LanguageName { tag: "de", names: &["german", "deutsch", "德语", "德語"] },
    LanguageName { tag: "es", names: &["spanish", "español", "espanol", "西班牙语"] },
    LanguageName { tag: "pt", names: &["portuguese", "português", "portugues", "葡萄牙语"] },
    LanguageName { tag: "pt-BR", names: &["brazilian portuguese", "português brasileiro"] },
    LanguageName { tag: "it", names: &["italian", "italiano", "意大利语"] },
    LanguageName { tag: "nl", names: &["dutch", "nederlands"] },
    LanguageName { tag: "sv", names: &["swedish", "svenska"] },
    LanguageName { tag: "da", names: &["danish", "dansk"] },
    LanguageName { tag: "fi", names: &["finnish", "suomi"] },
    LanguageName { tag: "no", names: &["norwegian", "norsk"] },
    LanguageName { tag: "et", names: &["estonian", "eesti"] },
    LanguageName { tag: "pl", names: &["polish", "polski"] },
    LanguageName { tag: "cs", names: &["czech", "čeština"] },
    LanguageName { tag: "uk", names: &["ukrainian", "українська"] },
    LanguageName { tag: "ru", names: &["russian", "русский", "俄语", "俄語"] },
    LanguageName { tag: "el", names: &["greek", "ελληνικά"] },
    LanguageName { tag: "tr", names: &["turkish", "türkçe"] },
    LanguageName { tag: "ar", names: &["arabic", "العربية", "阿拉伯语"] },
    LanguageName { tag: "he", names: &["hebrew", "עברית"] },
    LanguageName { tag: "fa", names: &["persian", "farsi", "فارسی"] },
    LanguageName { tag: "hi", names: &["hindi", "हिन्दी", "हिंदी"] },
    LanguageName { tag: "bn", names: &["bengali", "bangla", "বাংলা"] },
    LanguageName { tag: "id", names: &["indonesian", "bahasa indonesia"] },
    LanguageName { tag: "ms", names: &["malay", "bahasa melayu"] },
    LanguageName { tag: "vi", names: &["vietnamese", "tiếng việt"] },
    LanguageName { tag: "th", names: &["thai", "ไทย"] },
    LanguageName { tag: "ja", names: &["japanese", "日本語", "日语", "日語"] },
    LanguageName { tag: "ko", names: &["korean", "한국어", "韩语", "韓語"] },
    LanguageName { tag: "zh", names: &["chinese", "中文", "汉语", "漢語"] },
    LanguageName { tag: "zh-Hans", names: &["simplified chinese", "简体中文", "簡體中文"] },
    LanguageName { tag: "zh-Hant", names: &["traditional chinese", "繁体中文", "繁體中文"] },
];

static REQUEST: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?ix)^(?:please\s+)?(?:(?:can|could|would)\s+you\s+)?(?:",
        r"\b(?:translate|rebuild|recreate|clone|copy|build|make|render|rewrite|create)\s+",
        r"(?:(?:a|an|the|this|my|our|whole|entire|new)\s+)*",
        r"(?:(?:website|site|web\s+page|page|content|text|copy|it|this)\s+)?(?:in|into|to)\s+",
        r"|\b(?:set|change)\s+(?:(?:the|this|website|site|page)\s+)*language\s+to\s+",
        r"|(?:请|請)?(?:(?:将|把|將)(?:这个|這個|这|這|整个|整個)?(?:网站|網站|网页|網頁|页面|頁面))?(?:翻译成|翻譯成)",
        r"|(?:网站语言改为|網站語言改為))(?P<language>.+)$",
    ))
    .ok()
});
static USE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:please\s+)?use\s+(?P<language>.+?)\s+for\s+(?:(?:the|this|my|our)\s+)?(?:website|site|page)$").ok());
static ADJECTIVE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:please\s+)?(?:create|build|make|rebuild|recreate|clone|copy)\s+(?:(?:a|an|the|this|my|our|new)\s+)*(?P<language>.+?)\s+(?:website|site|page)$").ok()
});
static HTML: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"(?i)^(?:please\s+)?set\s+(?:the\s+)?<html\s+lang\s*=\s*["'](?P<language>[^"']+)["']\s*>$"#).ok());

fn named(text: &str, allow_tag: bool) -> Option<WebsiteLanguage> {
    let text = text.trim();
    if matches!(text, "original language" | "the original language" | "原文") {
        return Some(WebsiteLanguage::Original);
    }
    if let Some(language) = LANGUAGES.iter().find(|l| l.names.contains(&text)) {
        return Some(WebsiteLanguage::Requested { tag: language.tag.into() });
    }
    if (allow_tag || text.contains('-')) && text.len() <= 63 && !text.contains('_') {
        let primary = text.split('-').next();
        let known_language = LANGUAGES.iter().any(|language| language.tag.split('-').next() == primary);
        if allow_tag || known_language {
            return Locale::try_from_str(text).ok().map(|l| WebsiteLanguage::Requested { tag: l.to_string() });
        }
    }
    None
}

/// Conservative, explicit instructions only. A language mentioned in an example, a negative
/// instruction, quoted source text or a programming-stack request doesn't choose the page's
/// language. Unrecognised wording keeps the current choice; it never guesses from built content.
fn requested(text: &str) -> Option<WebsiteLanguage> {
    if text.len() > 64 * 1024 {
        return None;
    }
    let text = text.to_lowercase();
    if let Some(capture) = HTML.as_ref().and_then(|r| r.captures(text.trim().trim_end_matches('.'))).and_then(|c| c.name("language")) {
        return named(capture.as_str(), true);
    }
    // Do not turn quoted examples or fenced source into instructions when splitting sentences.
    // Rich/ambiguous requests keep the saved choice rather than guessing at their meaning.
    if text.contains(['"', '\'', '`', '“', '”', '‘', '’', '「', '」'])
        || text.contains("~~~")
        || ["for example", "this example", "an example", "example:"].iter().any(|example| text.contains(example))
    {
        return None;
    }
    let mut choice = None;
    for clause in text.split(['.', ';', '\n', '!', '?', '。', '；', '！', '？']) {
        let clause = clause.trim();
        let words = format!(" {} ", clause.replace(['’', ','], " "));
        if [" not ", " don't ", " don t ", " never ", " avoid ", " example ", " without ", "不要", "不需要"].iter().any(|no| words.contains(no))
            || clause.starts_with(['>', '`'])
        {
            continue;
        }
        if ["restore the original language", "keep the original language", "use the original language", "恢复原文", "恢復原文"]
            .iter()
            .any(|reset| clause == *reset || clause.strip_prefix("please ") == Some(*reset))
        {
            choice = Some(WebsiteLanguage::Original);
            continue;
        }
        for pattern in [&*REQUEST, &*USE, &*ADJECTIVE].into_iter().flatten() {
            if let Some(capture) = pattern.captures(clause).and_then(|c| c.name("language")) {
                let allow_tag = clause.contains("language") || clause.contains("translate") || clause.contains("<html");
                if let Some(language) = named(capture.as_str(), allow_tag) {
                    choice = Some(language);
                }
            }
        }
    }
    choice
}

pub(super) fn record(ws: &mut Workspace, requests: &[&str]) -> Result<(), ToolError> {
    record_after_snapshot(ws, requests, || {})
}

fn record_after_snapshot(ws: &mut Workspace, requests: &[&str], after_snapshot: impl FnOnce()) -> Result<(), ToolError> {
    let original_request = ws.manifest.request.clone();
    record_choice(
        ws,
        |current| {
            let saved = current.website_language_request_id.unwrap_or(0);
            let requests = requests
                .iter()
                .enumerate()
                .map(|(offset, text)| {
                    let id = saved
                        .checked_add(offset as u64)
                        .and_then(|id| id.checked_add(1))
                        .ok_or_else(|| ToolError::invalid("The website-language request ids have no space for a later request."))?;
                    Ok((id, *text))
                })
                .collect::<Result<Vec<_>, ToolError>>()?;
            Ok(select(&original_request, current, &requests))
        },
        after_snapshot,
    )
}

pub(super) fn record_ordered(ws: &mut Workspace, requests: &[(u64, &str)]) -> Result<(), ToolError> {
    record_ordered_after_snapshot(ws, requests, || {})
}

fn record_ordered_after_snapshot(ws: &mut Workspace, requests: &[(u64, &str)], after_snapshot: impl FnOnce()) -> Result<(), ToolError> {
    let original_request = ws.manifest.request.clone();
    record_choice(ws, |current| Ok(select(&original_request, current, requests)), after_snapshot)
}

type Choice = (Option<WebsiteLanguage>, Option<u64>);

fn select(original_request: &str, current: &Manifest, requests: &[(u64, &str)]) -> Choice {
    let order = current.website_language_request_id;
    let initial = current.website_language.clone().or_else(|| requested(original_request));
    let latest = requests
        .iter()
        .filter(|(id, _)| order.is_none_or(|saved| *id > saved))
        .filter_map(|(id, text)| requested(text).map(|choice| (*id, choice)))
        .max_by_key(|(id, _)| *id);
    latest.map_or((initial, order), |(id, choice)| (Some(choice), Some(id)))
}

fn record_choice(ws: &mut Workspace, select: impl Fn(&Manifest) -> Result<Choice, ToolError>, after_snapshot: impl FnOnce()) -> Result<(), ToolError> {
    let current = Workspace::open(&ws.root)?.manifest;
    let mut next = select(&current)?;
    after_snapshot();
    ws.update_person(|m| {
        // Parsing can happen outside the lock, but accepting an ordering decision cannot. A
        // different handle may have recorded a later request since the snapshot was read.
        if m.website_language != current.website_language || m.website_language_request_id != current.website_language_request_id {
            next = select(m)?;
        }
        let changed = m.website_language != next.0 || m.website_language_request_id != next.1;
        (m.website_language, m.website_language_request_id) = next;
        Ok(changed)
    })
}

pub(super) fn brief(ws: &Workspace) -> Vec<String> {
    match ws.manifest.website_language.as_ref() {
        Some(WebsiteLanguage::Requested { tag }) => vec![format!(
            "The person requested the website in {tag}. Translate its content and declare that language on html. The check compares that declaration, not the original's words or headings. A matching declaration alone cannot verify translation quality."
        )],
        Some(WebsiteLanguage::Original) => vec!["The person asked to restore the original language; keep its words and headings again.".into()],
        None => Vec::new(),
    }
}

#[derive(Default)]
pub(super) struct ContentCheck {
    pub note: Option<String>,
    pub claim: Option<Claim>,
    pub unknown: Option<String>,
}

pub(super) fn state_claim(name: &str, width: u64, paired: bool, language: Option<&WebsiteLanguage>) -> Result<Claim, String> {
    if !paired && matches!(language, Some(WebsiteLanguage::Requested { .. })) {
        return Err(format!("{name} at {width} px couldn't be paired by label after the language change; whether its translated control opens is unknown"));
    }
    Ok(Claim::new(format!("{name} opens in the build at {width} px"), paired, "no such control opens"))
}

pub(super) fn content_check(page: &str, language: Option<&WebsiteLanguage>, original: Option<&Value>, built: Option<&Value>) -> ContentCheck {
    let Some(WebsiteLanguage::Requested { tag }) = language else {
        return ContentCheck { note: original.zip(built).and_then(|(o, b)| super::website::text_likeness(o, b)).map(|t| t.note()), ..ContentCheck::default() };
    };
    let Some(declared) = built.and_then(|b| b.pointer("/a11y/lang")).and_then(Value::as_str) else {
        return ContentCheck { unknown: Some(format!("{page}'s declared language wasn't measured (requested {tag})")), ..ContentCheck::default() };
    };
    // Basic language-range matching is case-insensitive and ends on a subtag boundary:
    // fr allows fr-CA, while a request for fr-CA does not accept plain fr or fr-FR.
    let valid = !declared.contains('_') && Locale::try_from_str(declared).is_ok();
    let matches = valid
        && (declared.eq_ignore_ascii_case(tag)
            || declared.get(..tag.len()).is_some_and(|prefix| prefix.eq_ignore_ascii_case(tag))
                && declared.get(tag.len()..).is_some_and(|suffix| suffix.starts_with('-')));
    let detail = if declared.is_empty() { "the page has no language declaration".into() } else { format!("the page declares {declared}") };
    ContentCheck {
        note: Some(format!("requested language {tag}: {detail}; translated content is not verified by the declaration")),
        claim: Some(Claim::new(format!("{page} declares the requested language {tag}"), matches, detail)),
        unknown: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn french() -> WebsiteLanguage {
        WebsiteLanguage::Requested { tag: "fr".into() }
    }

    #[test]
    fn explicit_website_language_requests_and_controls() {
        for (text, tag) in [
            ("Rebuild this website in French.", "fr"),
            ("Build a website in French.", "fr"),
            ("Create a French website.", "fr"),
            ("Translate the site into Brazilian Portuguese.", "pt-BR"),
            ("Please translate it to zh-Hant-TW.", "zh-Hant-TW"),
            ("Set the page language to FR-ca.", "fr-CA"),
            ("Use Deutsch for the website", "de"),
            ("将网站翻译成中文。", "zh"),
            ("Set <html lang=\"fr-CA\">", "fr-CA"),
        ] {
            assert_eq!(requested(text), Some(WebsiteLanguage::Requested { tag: tag.into() }), "{text}");
        }
        for text in [
            "Do not translate this website into French.",
            "Don't translate it to French",
            "不要将网站翻译成中文",
            "The French site is an example",
            "For example, rebuild this website in French",
            "> Rebuild this website in French",
            "Add a French flag",
            "Translate the English error message in docs",
            "Build this website in Rust",
            "Build this site in CSS",
            "Make this website in light-gray",
            "Use a French image for the site",
            "Create a French restaurant website",
            "Set the page language to fr_CA",
            "Add the caption \"Rebuild this website in French.\"",
            "Explain how to translate the site into French",
            "Add this example: first do the layout. Rebuild this website in French.",
            "```text\nRebuild this website in French.\n```",
        ] {
            assert_eq!(requested(text), None, "{text}");
        }
    }

    #[test]
    fn translated_menu_labels_are_unknown_instead_of_missing_controls() {
        let original = json!({"states": [{"kind": "menu", "label": "About", "file": "state-1.png"}]});
        let built = json!({"states": [{"kind": "menu", "label": "À propos", "file": "state-1.png"}]});
        let pairs = super::super::website::pair_states(&original, &built);
        assert_eq!(pairs.len(), 1);
        assert!(pairs[0].1.is_none());
        assert!(state_claim("About", 390, false, Some(&french())).unwrap_err().contains("unknown"));
        assert!(state_claim("About", 390, true, Some(&french())).unwrap().passed);
        assert!(!state_claim("About", 390, false, None).unwrap().passed);
        assert!(!state_claim("About", 390, false, Some(&WebsiteLanguage::Original)).unwrap().passed);
    }

    #[test]
    fn requested_website_language_replaces_original_word_and_heading_feedback() {
        let original = json!({"text": "Welcome home home", "headings": [{"text": "Welcome"}], "a11y": {"lang": "en"}});
        let built = json!({"text": "Bienvenue chez nous", "headings": [{"text": "Bienvenue"}], "a11y": {"lang": "FR-ca"}});
        let check = content_check("Start", Some(&french()), Some(&original), Some(&built));
        assert!(check.claim.as_ref().is_some_and(|c| c.passed), "{:?}", check.note);
        let note = check.note.unwrap();
        assert!(!note.contains("missing") && !note.contains("Welcome") && !note.contains("home"), "{note}");
        assert!(note.contains("not verified"), "{note}");
        let result = CheckResult { similarity: Some(90.0), claims: check.claim.into_iter().collect(), ..CheckResult::new("web-visual", 1, true, note) };
        let refine = super::super::WebsiteKind.refine(&result, false);
        assert!(!refine.contains("Welcome") && !refine.contains("home"));
    }

    #[test]
    fn requested_website_language_missing_or_wrong_declaration_does_not_pass() {
        for tag in ["", "en", "french", "fr_CA", "fr-", "france"] {
            let built = json!({"a11y": {"lang": tag}});
            let checked = content_check("About", Some(&french()), None, Some(&built));
            assert!(checked.claim.is_some_and(|c| !c.passed), "{tag}");
        }
        for built in [None, Some(json!({})), Some(json!({"a11y": {"lang": null}}))] {
            let checked = content_check("About", Some(&french()), None, built.as_ref());
            assert!(checked.claim.is_none() && checked.unknown.is_some());
        }
        let regional = WebsiteLanguage::Requested { tag: "fr-CA".into() };
        assert!(!content_check("Start", Some(&regional), None, Some(&json!({"a11y": {"lang":"fr"}}))).claim.unwrap().passed);
    }

    #[test]
    fn ordinary_and_restored_website_language_keep_text_checks() {
        let original = json!({"text": "Welcome home", "headings": []});
        let built = json!({"text": "Bienvenue", "headings": []});
        for intent in [None, Some(&WebsiteLanguage::Original)] {
            let checked = content_check("Start", intent, Some(&original), Some(&built));
            assert!(checked.note.unwrap().contains("missing words"));
            assert!(checked.claim.is_none() && checked.unknown.is_none());
        }
    }

    #[test]
    fn saved_request_order_survives_equal_choices_old_writers_and_original_reset() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(dir.path(), "site", "Rebuild this website in English", TargetRef::None).unwrap();
        let stale = Workspace::open(&ws.root).unwrap();
        record_ordered(&mut ws, &[(71, "Build the website in French")]).unwrap();
        record_ordered(&mut ws, &[(73, "Build the website in French")]).unwrap();
        stale.save().unwrap();
        let mut restarted = Workspace::open(&ws.root).unwrap();
        record_ordered(&mut restarted, &[(72, "Build the website in German")]).unwrap();
        assert_eq!(restarted.manifest.website_language, Some(french()));
        assert_eq!(restarted.manifest.website_language_request_id, Some(73));
        record_ordered(&mut restarted, &[(74, "Restore the original language")]).unwrap();
        record_ordered(&mut restarted, &[(73, "Build the website in French")]).unwrap();
        assert_eq!(Workspace::open(&ws.root).unwrap().manifest.website_language, Some(WebsiteLanguage::Original));
        assert_eq!(Workspace::open(&ws.root).unwrap().manifest.website_language_request_id, Some(74));
    }

    #[test]
    fn an_older_language_request_rechecks_a_concurrent_equal_choice() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(dir.path(), "site", "Rebuild this website in English", TargetRef::None).unwrap();
        record_ordered(&mut ws, &[(71, "Build the website in French")]).unwrap();
        let mut newer = Workspace::open(&ws.root).unwrap();
        record_ordered_after_snapshot(&mut ws, &[(72, "Build the website in German")], || {
            record_ordered(&mut newer, &[(73, "Build the website in French")]).unwrap();
            newer.set_person(|m| m.name = "The person's name".into()).unwrap();
        })
        .unwrap();
        let disk = Workspace::open(&ws.root).unwrap().manifest;
        assert_eq!(disk.website_language, Some(french()));
        assert_eq!(disk.website_language_request_id, newer.manifest.website_language_request_id);
        assert_eq!(ws.manifest.website_language, disk.website_language);
        assert_eq!(ws.manifest.website_language_request_id, disk.website_language_request_id);
        assert_eq!(disk.name, newer.manifest.name, "recording also keeps unrelated person edits");
    }

    #[test]
    fn an_unordered_language_request_advances_past_a_concurrent_request() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(dir.path(), "site", "Rebuild this website in English", TargetRef::None).unwrap();
        record_ordered(&mut ws, &[(71, "Build the website in French")]).unwrap();
        let mut newer = Workspace::open(&ws.root).unwrap();
        record_after_snapshot(&mut ws, &["Build the website in German"], || {
            record_ordered(&mut newer, &[(80, "Restore the original language")]).unwrap();
        })
        .unwrap();
        let disk = Workspace::open(&ws.root).unwrap().manifest;
        assert_eq!(disk.website_language, Some(WebsiteLanguage::Requested { tag: "de".into() }));
        assert!(disk.website_language_request_id > newer.manifest.website_language_request_id, "new automatic ids advance from the guarded state");
        assert_eq!(ws.manifest.website_language_request_id, disk.website_language_request_id);
    }

    #[test]
    fn a_failed_guarded_language_selection_keeps_the_saved_choice_and_handle() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(dir.path(), "site", "Rebuild this website in English", TargetRef::None).unwrap();
        record_ordered(&mut ws, &[(u64::MAX - 1, "Build the website in French")]).unwrap();
        let original = ws.manifest.clone();
        let mut newer = Workspace::open(&ws.root).unwrap();
        let mut saved = Vec::new();
        let error = record_after_snapshot(&mut ws, &["Build the website in German"], || {
            record_ordered(&mut newer, &[(u64::MAX, "Restore the original language")]).unwrap();
            saved = std::fs::read(newer.root.join(craft_anything_engine::workspace::MANIFEST)).unwrap();
        })
        .unwrap_err();
        assert!(error.message.contains("no space for a later request"));
        assert_eq!(std::fs::read(ws.root.join(craft_anything_engine::workspace::MANIFEST)).unwrap(), saved);
        assert_eq!(ws.manifest, original, "a rejected proposal does not become this handle's state");
    }

    #[test]
    fn website_language_survives_followups_old_writers_and_original_reset() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(dir.path(), "site", "Rebuild this website in French", TargetRef::None).unwrap();
        record(&mut ws, &[]).unwrap();
        assert_eq!(ws.manifest.website_language, Some(french()));
        let stale = Workspace::open(&ws.root).unwrap();
        record(&mut ws, &["Set the page language to de"]).unwrap();
        stale.save().unwrap();
        ws.reload().unwrap();
        assert_eq!(ws.manifest.website_language, Some(WebsiteLanguage::Requested { tag: "de".into() }));
        record(&mut ws, &["Make the header blue"]).unwrap();
        record(&mut ws, &["Restore the original language"]).unwrap();
        ws.reload().unwrap();
        record(&mut ws, &[]).unwrap();
        assert_eq!(ws.manifest.website_language, Some(WebsiteLanguage::Original));
        let mut old = serde_json::to_value(&ws.manifest).unwrap();
        old["version"] = 5.into();
        old.as_object_mut().unwrap().remove("website_language");
        old.as_object_mut().unwrap().remove("website_language_request_id");
        let upgraded = craft_anything_engine::workspace::read_manifest(&serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(upgraded.version, craft_anything_engine::workspace::MANIFEST_VERSION);
        assert_eq!(upgraded.website_language, None);
    }
}
