//! UI localisation: English strings in code are the lookup keys; a per-language
//! TSV catalog (`key<TAB>translation` per line, `#` comments) maps them to display text at render
//! time. A string with no translation is shown in English. Tool ids, the CLI and MCP never see
//! translated text.
//!
//! Each language's catalog is a folder, `i18n/<code>/`, whose `.tsv` files `build.rs` joins in
//! name order. Strings a change adds go in a file of its own in each folder (named for its
//! branch or roadmap item), so two changes that add strings don't conflict (#1465).
//!
//! Adding a language: put its strings in `i18n/xx/core.tsv` and add one row to [`LANGUAGES`].

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// One supported UI language.
pub struct LangInfo {
    /// BCP 47 code, lowercase. Also the `settings.language` value.
    pub code: &'static str,
    /// The language's name in itself.
    pub name: &'static str,
    /// Catalog contents (empty for the built-in English).
    pub source: &'static str,
}

/// A language's catalog, as `build.rs` joins it from `i18n/<code>/`.
macro_rules! catalog {
    ($code:literal) => {
        include_str!(concat!(env!("OUT_DIR"), "/i18n/", $code, ".tsv"))
    };
}

pub static LANGUAGES: &[LangInfo] = &[
    LangInfo { code: "en", name: "English", source: "" },
    LangInfo { code: "de", name: "Deutsch", source: catalog!("de") },
    LangInfo { code: "ka", name: "ქართული", source: catalog!("ka") },
    LangInfo { code: "fr", name: "Français", source: catalog!("fr") },
    LangInfo { code: "it", name: "Italiano", source: catalog!("it") },
    LangInfo { code: "ja", name: "日本語", source: catalog!("ja") },
    LangInfo { code: "zh-hans", name: "简体中文", source: catalog!("zh-hans") },
    LangInfo { code: "zh-hant", name: "繁體中文", source: catalog!("zh-hant") },
    LangInfo { code: "ru", name: "Русский", source: catalog!("ru") },
    LangInfo { code: "ko", name: "한국어", source: catalog!("ko") },
    LangInfo { code: "cs", name: "Čeština", source: catalog!("cs") },
    LangInfo { code: "es", name: "Español", source: catalog!("es") },
    LangInfo { code: "vi", name: "Tiếng Việt", source: catalog!("vi") },
    LangInfo { code: "pt-br", name: "Português (Brasil)", source: catalog!("pt-br") },
    LangInfo { code: "tr", name: "Türkçe", source: catalog!("tr") },
    LangInfo { code: "th", name: "ไทย", source: catalog!("th") },
    LangInfo { code: "et", name: "Eesti", source: catalog!("et") },
    LangInfo { code: "he", name: "עברית", source: catalog!("he") },
];

/// Match a locale against `langs`. An explicit script wins over the region, and legacy `iw`
/// normalizes to `he`.
pub fn language_from_tag_in(tag: &str, langs: &[LangInfo]) -> Option<&'static str> {
    let tag = tag.trim().split(['.', '@']).next().unwrap_or_default().replace('_', "-").to_ascii_lowercase();
    if let Some(lang) = langs.iter().find(|l| l.code == tag) {
        return Some(lang.code);
    }
    let mut parts = tag.split('-');
    let primary = match parts.next()? {
        "iw" => "he",
        other => other,
    };
    match primary {
        "zh" => {
            let parts: Vec<_> = parts.take_while(|p| p.len() != 1).collect();
            if parts.contains(&"hant") {
                Some("zh-hant")
            } else if parts.contains(&"hans") {
                Some("zh-hans")
            } else if parts.iter().any(|p| matches!(*p, "tw" | "hk" | "mo")) {
                Some("zh-hant")
            } else {
                Some("zh-hans")
            }
        }
        "pt" => Some("pt-br"),
        _ => langs.iter().find(|l| l.code == primary).map(|l| l.code),
    }
}

/// Match a locale to a registered UI language. An explicit script wins over the region.
pub fn language_from_tag(tag: &str) -> Option<&'static str> {
    language_from_tag_in(tag, LANGUAGES)
}

fn preferred_language(tags: impl IntoIterator<Item = impl AsRef<str>>) -> &'static str {
    tags.into_iter().find_map(|tag| language_from_tag(tag.as_ref())).unwrap_or("en")
}

fn system_language() -> &'static str {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static SYSTEM: OnceLock<&'static str> = OnceLock::new();
        SYSTEM.get_or_init(|| preferred_language(sys_locale::get_locales()))
    }
    #[cfg(target_arch = "wasm32")]
    {
        preferred_language(std::iter::empty::<&str>())
    }
}

struct Catalog {
    code: String,
    rtl: bool,
    map: HashMap<String, String>,
}

fn current() -> &'static RwLock<Catalog> {
    static CUR: OnceLock<RwLock<Catalog>> = OnceLock::new();
    CUR.get_or_init(|| RwLock::new(Catalog { code: "en".into(), rtl: false, map: HashMap::new() }))
}

fn parse(source: &str) -> HashMap<String, String> {
    source
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .filter(|(k, v)| !k.is_empty() && !v.trim().is_empty())
        .map(|(k, v)| (k.replace("\\n", "\n"), v.trim_end().replace("\\n", "\n")))
        .collect()
}

/// Switch language. Empty means the system's preferred language; unsupported codes mean English.
pub fn set_language(code: &str) {
    let code = if code.trim().is_empty() { system_language() } else { language_from_tag(code).unwrap_or("en") };
    let lang = LANGUAGES.iter().find(|l| l.code == code);
    let resolved = lang.map(|l| l.code).unwrap_or("en");
    let map = lang.map(|l| parse(l.source)).unwrap_or_default();
    let rtl = is_rtl_code(resolved);
    if let Ok(mut g) = current().write() {
        *g = Catalog { code: resolved.to_string(), rtl, map };
    }
}

/// Activate a language code and TSV catalog directly without registering it in [`LANGUAGES`].
/// Used by RTL UI tests before a locale catalog is shipped.
pub(crate) fn set_custom_catalog(code: &str, source: &str) {
    let tag = code.trim().split(['.', '@']).next().unwrap_or_default().replace('_', "-").to_ascii_lowercase();
    let normalized = if tag == "iw" {
        "he".to_string()
    } else if let Some(rest) = tag.strip_prefix("iw-") {
        format!("he-{rest}")
    } else {
        tag
    };
    let rtl = is_rtl_code(&normalized);
    if let Ok(mut g) = current().write() {
        *g = Catalog { code: normalized, rtl, map: parse(source) };
    }
}

pub fn language() -> String {
    current().read().map(|g| g.code.clone()).unwrap_or_else(|_| "en".into())
}

/// Whether `code` names a right-to-left script or language (accepting legacy `iw` tags as Hebrew
/// and honoring explicit ISO 15924 script subtags before language defaults).
pub fn is_rtl_code(code: &str) -> bool {
    let tag = code.trim().split(['.', '@']).next().unwrap_or_default().replace('_', "-").to_ascii_lowercase();
    let mut parts = tag.split('-');
    let base = parts.next().unwrap_or("");
    if base == "x" {
        return false;
    }
    let next = parts.find(|p| !(p.len() == 3 && p.bytes().all(|b| b.is_ascii_alphabetic())));
    if let Some(script) = next.filter(|p| p.len() == 4 && p.bytes().all(|b| b.is_ascii_alphabetic())) {
        return matches!(script, "arab" | "hebr" | "nkoo" | "rohg" | "syrc" | "thaa" | "yezi");
    }
    matches!(base, "ar" | "ckb" | "dv" | "fa" | "he" | "iw" | "ps" | "sd" | "ug" | "ur" | "yi")
}

/// Whether the active UI language uses right-to-left script direction.
pub fn is_rtl() -> bool {
    current().read().map(|g| g.rtl).unwrap_or(false)
}

fn is_rtl_script(c: char) -> bool {
    matches!(
        u32::from(c),
        0x0590..=0x05FF | 0x0600..=0x065F | 0x066E..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFC | 0x1_0800..=0x1_0FFF | 0x1_E800..=0x1_EFFF
    )
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fn is_joiner(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}')
}

fn mirrored(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        other => other,
    }
}

/// Isolate leading-punctuation LTR tokens (`.env`, `/tmp/demo.cany`, `~/x`, `-12.50`, `+3`)
/// with Unicode `LRI`..`PDI` before running UAX #9 so the leading separator stays attached to
/// the LTR token instead of migrating to the RTL side.
fn isolate_leading_ltr_tokens(line: &str) -> String {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut out = String::with_capacity(line.len().saturating_add(16));
    let mut i = 0usize;
    let mut isolate_depth = 0usize;
    while let Some(&(start, ch)) = chars.get(i) {
        if matches!(ch, '\u{2066}' | '\u{2067}' | '\u{2068}') {
            isolate_depth = isolate_depth.saturating_add(1);
            out.push(ch);
            i += 1;
            continue;
        }
        if ch == '\u{2069}' {
            isolate_depth = isolate_depth.saturating_sub(1);
            out.push(ch);
            i += 1;
            continue;
        }
        let at_token_boundary = i == 0 || chars.get(i - 1).is_some_and(|(_, p)| p.is_whitespace() || matches!(p, '(' | '[' | '{' | '"' | '\''));
        let is_ltr_prefix = isolate_depth == 0
            && at_token_boundary
            && ((matches!(ch, '.' | '/' | '~' | '\\') && chars.get(i + 1).is_some_and(|(_, next)| next.is_ascii_alphanumeric() || *next == '/'))
                || (matches!(ch, '-' | '+') && chars.get(i + 1).is_some_and(|(_, next)| next.is_ascii_digit())));
        if is_ltr_prefix {
            let mut j = i + 1;
            while let Some(&(_, c)) = chars.get(j) {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '/' | '-' | '+' | '_' | ':' | '~' | '\\') {
                    j += 1;
                } else {
                    break;
                }
            }
            while j > i + 1 && chars.get(j - 1).is_some_and(|(_, c)| matches!(c, '.' | ':' | '-' | '/')) {
                j -= 1;
            }
            let end = chars.get(j).map_or(line.len(), |(b, _)| *b);
            if let Some(tok) = line.get(start..end) {
                out.push('\u{2066}');
                out.push_str(tok);
                out.push('\u{2069}');
                i = j;
                continue;
            }
        }
        out.push(ch);
        i += 1;
    }
    out
}

fn push_visual_rtl_line(line: &str, out: &mut String) {
    let prepared = isolate_leading_ltr_tokens(line);
    let bidi = unicode_bidi::BidiInfo::new(&prepared, Some(unicode_bidi::Level::rtl()));
    for para in &bidi.paragraphs {
        let (levels, runs) = bidi.visual_runs(para, para.range.clone());
        for run in runs {
            let Some(slice) = prepared.get(run.clone()) else { continue };
            let rtl = levels.get(run.start).is_some_and(unicode_bidi::Level::is_rtl);
            if rtl {
                // Walk visually left-to-right (reverse of logical); keep each contiguous RTL-script
                // stretch in logical order so epaint's harfrust shapes that stretch RTL.
                let mut stretch: Vec<char> = Vec::new();
                for c in slice.chars().rev() {
                    if is_bidi_control(c) {
                        continue;
                    }
                    if is_rtl_script(c) || is_joiner(c) {
                        stretch.push(c);
                    } else {
                        for sc in stretch.drain(..).rev() {
                            out.push(sc);
                        }
                        out.push(mirrored(c));
                    }
                }
                for sc in stretch.drain(..).rev() {
                    out.push(sc);
                }
            } else {
                for c in slice.chars().filter(|c| !is_bidi_control(*c)) {
                    out.push(c);
                }
            }
        }
    }
}

/// Rearrange `text` into visual right-to-left order for egui 0.36 while keeping contiguous RTL
/// script stretches in logical order for harfrust shaping.
fn visual_rtl(text: &str) -> String {
    if !text.chars().any(is_rtl_script) {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    for (idx, raw_line) in text.split('\n').enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        let (line, cr) = raw_line.strip_suffix('\r').map_or((raw_line, false), |l| (l, true));
        if line.chars().any(is_rtl_script) {
            push_visual_rtl_line(line, &mut out);
        } else {
            out.push_str(line);
        }
        if cr {
            out.push('\r');
        }
    }
    out
}

/// Convert logical reading-order text into visual display order when the active language is RTL.
/// Left-to-right languages return `logical` borrowed without allocation.
pub fn display<'a>(logical: &'a str) -> std::borrow::Cow<'a, str> {
    if is_rtl() && logical.chars().any(is_rtl_script) { std::borrow::Cow::Owned(visual_rtl(logical)) } else { std::borrow::Cow::Borrowed(logical) }
}

fn wrap_paragraph_logical<'a>(ctx: &egui::Context, paragraph: &'a str, font_id: &egui::FontId, max_width: f32) -> Vec<&'a str> {
    if paragraph.is_empty() || !max_width.is_finite() {
        return vec![paragraph];
    }
    let max_w = max_width.max(1.0);
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut in_word = false;
    let mut word_start = 0usize;
    for (idx, ch) in paragraph.char_indices() {
        if ch.is_whitespace() {
            if in_word {
                spans.push((word_start, idx));
                in_word = false;
            }
        } else if !in_word {
            word_start = idx;
            in_word = true;
        }
    }
    if in_word {
        spans.push((word_start, paragraph.len()));
    }
    if spans.is_empty() {
        return vec![paragraph];
    }
    // Single-pass O(words) width measurement under one font-lock acquisition.
    let (space_w, word_widths): (f32, Vec<f32>) = ctx.fonts_mut(|f| {
        let space = f.layout_no_wrap(" ".to_owned(), font_id.clone(), Default::default()).rect.width();
        let widths = spans
            .iter()
            .map(|&(s, e)| f.layout_no_wrap(paragraph.get(s..e).unwrap_or_default().to_owned(), font_id.clone(), Default::default()).rect.width())
            .collect();
        (space, widths)
    });
    let mut rows = Vec::new();
    let mut iter = spans.into_iter().zip(word_widths);
    let Some(((mut line_start, mut line_end), mut line_w)) = iter.next() else {
        return vec![paragraph];
    };
    for ((span_start, span_end), w) in iter {
        if line_w + space_w + w <= max_w {
            line_end = span_end;
            line_w += space_w + w;
        } else {
            if let Some(row) = paragraph.get(line_start..line_end) {
                rows.push(row);
            }
            line_start = span_start;
            line_end = span_end;
            line_w = w;
        }
    }
    if let Some(row) = paragraph.get(line_start..line_end) {
        rows.push(row);
    }
    rows
}

/// Break `logical` text at `max_width` first using original logical substrings, then apply visual
/// RTL ordering to each display row containing RTL characters.
pub fn wrapped_visual_lines(ctx: &egui::Context, rtl: bool, logical: &str, font_id: egui::FontId, max_width: f32) -> String {
    if !rtl || !logical.chars().any(is_rtl_script) {
        return logical.to_owned();
    }
    let mut out = String::with_capacity(logical.len());
    let mut first_row = true;
    for paragraph in logical.split('\n') {
        for row_logical in wrap_paragraph_logical(ctx, paragraph, &font_id, max_width) {
            if !first_row {
                out.push('\n');
            }
            first_row = false;
            if row_logical.chars().any(is_rtl_script) {
                push_visual_rtl_line(row_logical, &mut out);
            } else {
                out.push_str(row_logical);
            }
        }
    }
    out
}

/// Render a single-line or default label from logical text, reordering only the painted string
/// when RTL is active while keeping the screen-reader label in logical reading order.
pub fn label(ui: &mut egui::Ui, logical: &str) -> egui::Response {
    rich_label(ui, logical, |r| r)
}

/// Render a styled label from logical text, reordering only the painted string when RTL is active
/// while keeping the screen-reader label in logical reading order.
pub fn rich_label(ui: &mut egui::Ui, logical: &str, style: impl FnOnce(egui::RichText) -> egui::RichText) -> egui::Response {
    let shown = display(logical);
    let reordered = matches!(shown, std::borrow::Cow::Owned(_)) && shown.as_ref() != logical;
    let resp = ui.label(style(egui::RichText::new(shown.as_ref())));
    if reordered {
        resp.ctx.accesskit_node_builder(resp.id, |b| {
            b.set_value(logical);
            b.set_label(logical);
        });
    }
    resp
}

/// Render a heading (`theme::semibold(size)`) from logical text, reordering only the painted
/// string when RTL is active while keeping the screen-reader label in logical reading order.
pub fn heading(ui: &mut egui::Ui, logical: &str, size: f32) -> egui::Response {
    rich_label(ui, logical, |r| r.font(crate::theme::semibold(size)))
}

/// Attach a translated tooltip from logical text, reordering for visual RTL display without
/// altering logical string values elsewhere.
pub fn hover_text(resp: egui::Response, logical: &str) -> egui::Response {
    if is_rtl() && logical.chars().any(is_rtl_script) {
        let logical = logical.to_owned();
        resp.on_hover_ui(move |ui| {
            wrapped_label(ui, &logical, egui::TextStyle::Body.resolve(ui.style()), |r| r);
        })
    } else {
        resp.on_hover_text(logical)
    }
}

/// Render a wrapping label from logical text for an explicit `rtl` flag, deferring RTL visual
/// reordering until after line breaks while keeping the screen-reader label in logical order.
pub fn wrapped_label_for(
    ui: &mut egui::Ui,
    rtl: bool,
    logical: &str,
    font_id: egui::FontId,
    style: impl FnOnce(egui::RichText) -> egui::RichText,
) -> egui::Response {
    let needs_rtl_wrap = rtl && logical.chars().any(is_rtl_script);
    let shown = if needs_rtl_wrap { wrapped_visual_lines(ui.ctx(), rtl, logical, font_id.clone(), ui.available_width()) } else { logical.to_owned() };
    let rich = style(egui::RichText::new(&shown).font(font_id));
    let resp = if needs_rtl_wrap { ui.add(egui::Label::new(rich).extend()) } else { ui.add(egui::Label::new(rich).wrap()) };
    if needs_rtl_wrap && shown != logical {
        resp.ctx.accesskit_node_builder(resp.id, |b| {
            b.set_value(logical);
            b.set_label(logical);
        });
    }
    resp
}

/// Create [`egui::RichText`] in visual display order for widgets (headers, windows, custom rows)
/// that accept a `RichText` directly. Pair with [`keep_logical_a11y`] on interactive headers.
pub fn rich_text(logical: &str) -> egui::RichText {
    egui::RichText::new(display(logical).as_ref())
}

/// Restore `logical` reading order on a widget's AccessKit label/value when its painted text was
/// reordered for visual RTL display.
pub fn keep_logical_a11y(resp: &egui::Response, logical: &str) {
    if is_rtl() && logical.chars().any(is_rtl_script) {
        resp.ctx.accesskit_node_builder(resp.id, |b| {
            b.set_value(logical);
            b.set_label(logical);
        });
    }
}

/// Render a styled wrapping body label from logical text, deferring RTL visual reordering until
/// after line breaks while keeping the screen-reader label in logical reading order.
pub fn wrapped_rich_label(ui: &mut egui::Ui, logical: &str, style: impl FnOnce(egui::RichText) -> egui::RichText) -> egui::Response {
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    wrapped_label_for(ui, is_rtl(), logical, font_id, style)
}

/// Render a single-line truncating label from logical text, reordering only the painted string
/// when RTL is active while keeping the screen-reader label in logical reading order.
pub fn truncated_label(ui: &mut egui::Ui, logical: &str, style: impl FnOnce(egui::RichText) -> egui::RichText) -> egui::Response {
    let shown = display(logical);
    let reordered = matches!(shown, std::borrow::Cow::Owned(_)) && shown.as_ref() != logical;
    let resp = ui.add(egui::Label::new(style(egui::RichText::new(shown.as_ref()))).truncate());
    if reordered {
        resp.ctx.accesskit_node_builder(resp.id, |b| {
            b.set_value(logical);
            b.set_label(logical);
        });
    }
    resp
}

/// Render a wrapping label from logical text, deferring RTL visual reordering until after egui
/// determines line breaks.
pub fn wrapped_label(ui: &mut egui::Ui, logical: &str, font_id: egui::FontId, style: impl FnOnce(egui::RichText) -> egui::RichText) -> egui::Response {
    wrapped_label_for(ui, is_rtl(), logical, font_id, style)
}

/// Translate a UI string in logical reading order.
pub fn tr(s: &str) -> String {
    match current().read() {
        Ok(g) => g.map.get(s).cloned().unwrap_or_else(|| s.to_string()),
        Err(_) => s.to_string(),
    }
}

/// Translate, then fill `{name}` placeholders in one pass (`platform::text::fill`), in logical
/// reading order so inserted values stay literal even when they look like placeholders.
pub fn trf(s: &str, args: &[(&str, &str)]) -> String {
    let template = tr(s);
    if template.contains('{') { craft_anything_platform::text::fill(&template, args) } else { template }
}

#[cfg(test)]
pub(crate) const TEST_HE_CATALOG: &str = "\
Apply\tהחלה
Command palette\tלוח פקודות
Command palette (Ctrl/⌘ K)\tלוח פקודות (Ctrl/⌘ K)
Export a report (Markdown)\tייצוא דוח (Markdown)
More memory makes Ghidra's first look at a big program faster; leave some for the rest of the computer. Used from the next time Ghidra starts.\tיותר זיכרון הופך את הבדיקה הראשונה של Ghidra בתוכנית גדולה למהירה יותר; חשוב להשאיר קצת לשאר המחשב. נכנס לתוקף בהפעלה הבאה של Ghidra.
New project\tפרויקט חדש
Open {name}\tפתיחת {name}
";

#[cfg(test)]
pub(crate) fn tr_for(code: &str, s: &str) -> String {
    let source = LANGUAGES.iter().find(|l| l.code == code).map(|l| l.source).or_else(|| (code == "he").then_some(TEST_HE_CATALOG)).unwrap_or_default();
    let map = parse(source);
    map.get(s).cloned().unwrap_or_else(|| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    static LANG_LOCK: craft_anything_platform::sync::Mutex<()> = craft_anything_platform::sync::Mutex::new(());

    fn display_for(code: &str, s: &str) -> String {
        let raw = tr_for(code, s);
        if is_rtl_code(code) { visual_rtl(&raw) } else { raw }
    }

    fn display_trf_for(code: &str, s: &str, args: &[(&str, &str)]) -> String {
        let template = tr_for(code, s);
        let filled = if template.contains('{') { craft_anything_platform::text::fill(&template, args) } else { template };
        if is_rtl_code(code) { visual_rtl(&filled) } else { filled }
    }

    #[test]
    fn catalog_parse_and_fallback() {
        let _guard = LANG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_language("en");
        let m = parse("# comment\nHello\tHola\nEmpty\t \nbad line\n");
        assert_eq!(m.get("Hello").map(String::as_str), Some("Hola"));
        assert!(!m.contains_key("Empty"));
        assert_eq!(trf("Open {name}", &[("name", "a.exe")]), "Open a.exe");
        assert_eq!(trf("{a} {b}", &[("a", "{b}"), ("b", "x")]), "{b} x");
        assert_eq!(tr("Untranslated"), "Untranslated");
        assert_eq!(trf("Untranslated", &[("a", "1")]), "Untranslated");
    }

    #[test]
    fn hebrew_tr_stays_logical_and_display_reorders_visual_rtl() {
        let _guard = LANG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(is_rtl_code("he"));
        // tr/trf keep logical reading order for accessibility, search, menus, and wrapping.
        assert_eq!(tr_for("he", "New project"), "פרויקט חדש");
        assert_eq!(tr_for("he", "Export a report (Markdown)"), "ייצוא דוח (Markdown)");
        set_custom_catalog("he", TEST_HE_CATALOG);
        assert!(is_rtl());
        assert_eq!(tr("Command palette"), "לוח פקודות");
        assert_eq!(trf("Open {name}", &[("name", ".env")]), "פתיחת .env");
        assert_eq!(display(&tr("Command palette")).as_ref(), "פקודות לוח");
        set_language("en");
        assert!(!is_rtl());

        // Single Hebrew word stays logical (epaint/harfrust shapes the single RTL run).
        assert_eq!(display_for("he", "Apply"), "החלה");
        // Multi-word Hebrew reorders words for visual right-to-left display.
        assert_eq!(display_for("he", "Command palette"), "פקודות לוח");
        // Mixed Hebrew + Latin + numbers + parentheses keep LTR segments intact and mirror RTL brackets.
        assert_eq!(display_for("he", "Export a report (Markdown)"), "(Markdown) דוח ייצוא");
        assert_eq!(display_for("he", "Command palette (Ctrl/⌘ K)"), "(Ctrl/⌘ K) פקודות לוח");
        assert_eq!(visual_rtl("תמונה 1 מתוך 3"), "3 מתוך 1 תמונה");
        assert_eq!(visual_rtl("(תמונה 1 מצורפת)"), "(מצורפת 1 תמונה)");
        // Placeholder fill keeps inserted values literal before visual RTL reordering.
        assert_eq!(display_trf_for("he", "Open {name}", &[("name", "demo.exe")]), "demo.exe פתיחת");
        assert_eq!(display_trf_for("he", "Open {name}", &[("name", ".env")]), ".env פתיחת");
        assert_eq!(display_trf_for("he", "Open {name}", &[("name", "/tmp/demo.cany")]), "/tmp/demo.cany פתיחת");
        assert_eq!(display_trf_for("he", "Open {name}", &[("name", "\u{2066}foo - אב - bar\u{2069}")]), "foo - אב - bar פתיחת");
        // LTR-only lines (including indentation) stay untouched in multi-line visual reordering.
        assert_eq!(visual_rtl("כותרת בעברית\n  cargo test\nעוד שורה"), "בעברית כותרת\n  cargo test\nשורה עוד");
        // Wrapping reorders each display row after line breaking, so opening words stay on row 0
        // and characters inside Hebrew words stay in logical order even with a Hebrew font loaded.
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx, crate::theme::ThemeKind::Studio);
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        let logical = tr_for(
            "he",
            "More memory makes Ghidra's first look at a big program faster; leave some for the rest of the computer. Used from the next time Ghidra starts.",
        );
        let wrapped = wrapped_visual_lines(&ctx, true, &logical, egui::FontId::proportional(crate::theme::CAPTION), 220.0);
        let lines: Vec<&str> = wrapped.lines().collect();
        assert!(lines.len() >= 2, "expected wrapped lines, got {wrapped:?}");
        assert!(lines[0].contains("זיכרון"), "first row should begin with opening Hebrew words: {lines:?}");
        assert!(!lines[0].contains("נורכיז"), "Hebrew word letters must remain logical: {lines:?}");
        assert!(!lines[0].contains("לתוקף"), "first row should not start with paragraph end: {lines:?}");

        let english_long = "This is a long English-only fallback error message that should still wrap inside narrow dialogs.";
        let mut english_width = 0.0f32;
        ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.allocate_ui(egui::vec2(140.0, 200.0), |ui| {
                    let resp = wrapped_label_for(ui, true, english_long, egui::FontId::proportional(crate::theme::CAPTION), |r| r);
                    english_width = resp.rect.width();
                });
            });
        })
        .textures_delta
        .clear();
        assert!(english_width <= 150.0, "English fallback under RTL must still wrap, got width {english_width}");
    }

    /// Script and region matching: an explicit script wins over a region, a registered language
    /// resolves from any of its regions, and a locale with no catalogue falls back to English.
    #[test]
    fn chinese_scripts_regions_and_fallback() {
        for tag in ["zh", "zh-CN", "zh_SG.UTF-8", "ZH-Hans", "zh-Hans-TW"] {
            assert_eq!(language_from_tag(tag), Some("zh-hans"), "{tag}");
        }
        for tag in ["zh-TW", "zh-HK", "zh-MO", "zh-Hant", "zh-Hant-CN"] {
            assert_eq!(language_from_tag(tag), Some("zh-hant"), "{tag}");
        }
        assert_eq!(language_from_tag("en-US"), Some("en"));
        assert_eq!(language_from_tag("ja-JP"), Some("ja"));
        assert_eq!(language_from_tag("fr-CA"), Some("fr"));
        assert_eq!(language_from_tag("de-DE"), Some("de"));
        assert_eq!(language_from_tag("de_AT.UTF-8"), Some("de"));
        assert_eq!(language_from_tag("es-ES"), Some("es"));
        assert_eq!(language_from_tag("es-MX"), Some("es"));
        assert_eq!(language_from_tag("sv-SE"), None);
        assert_eq!(preferred_language(["sv-SE", "zh-HK", "en-US"]), "zh-hant");
        assert_eq!(language_from_tag("vi-VN"), Some("vi"));
        assert_eq!(language_from_tag("vi_VN.UTF-8"), Some("vi"));
        assert_eq!(language_from_tag("pt_BR.UTF-8"), Some("pt-br"));
        assert_eq!(language_from_tag("pt-PT"), Some("pt-br"));
        assert_eq!(language_from_tag("tr-TR"), Some("tr"));
        assert_eq!(language_from_tag("th-TH"), Some("th"));
        assert_eq!(language_from_tag("th_TH.UTF-8"), Some("th"));
        assert_eq!(language_from_tag("he"), Some("he"));
        assert_eq!(language_from_tag("he-IL"), Some("he"));
        assert_eq!(language_from_tag("iw"), Some("he"));
        assert_eq!(language_from_tag("iw-IL"), Some("he"));
        assert_eq!(language_from_tag("iw_IL.UTF-8"), Some("he"));
        assert!(is_rtl_code("he"));
        assert!(is_rtl_code("iw"));
        assert!(is_rtl_code("iw-IL"));
        assert!(is_rtl_code("he-Hebr-IL"));
        assert!(is_rtl_code("he-u-nu-latn"));
        assert!(is_rtl_code("ckb"));
        assert!(is_rtl_code("ku-Arab-IQ"));
        assert!(!is_rtl_code("ku"));
        assert!(!is_rtl_code("ku-Latn-TR"));
        assert!(!is_rtl_code("sd-Deva-IN"));
        assert!(!is_rtl_code("en-x-hebr"));
        assert!(!is_rtl_code("en-u-nu-arab"));
        assert!(!is_rtl_code("x-Arab"));
        assert!(!is_rtl_code("x-Hebr"));
        assert!(!is_rtl_code("en"));
        assert_eq!(preferred_language(["de-DE", "zh-HK"]), "de");
        assert_eq!(preferred_language(["sv-SE", "vi-VN", "en-US"]), "vi");
        assert_eq!(preferred_language(["en-US", "zh-CN"]), "en");
        assert_eq!(preferred_language(["sv-SE"]), "en");
        assert_eq!(preferred_language(std::iter::empty::<&str>()), "en");
    }

    /// Every registered catalogue parses, and all of them carry the same key set: a language that
    /// missed keys would fall back to English mid-interface.
    #[test]
    fn catalogs_are_valid_and_have_matching_keys() {
        fn placeholders(s: &str) -> Vec<&str> {
            let mut out: Vec<_> = s.split('{').skip(1).filter_map(|s| s.split_once('}').map(|(p, _)| p)).collect();
            out.sort_unstable();
            out
        }
        let mut catalogs = Vec::new();
        for lang in LANGUAGES
            .iter()
            .filter(|l| matches!(l.code, "zh-hans" | "zh-hant" | "ru" | "ko" | "cs" | "de" | "ka" | "es" | "it" | "vi" | "pt-br" | "tr" | "th" | "et" | "he"))
        {
            let mut keys = std::collections::BTreeSet::new();
            for (i, line) in lang.source.lines().enumerate() {
                if line.trim().is_empty() || line.starts_with('#') {
                    continue;
                }
                let (key, value) = line.split_once('\t').unwrap_or_else(|| panic!("{}:{}: missing tab", lang.code, i + 1));
                assert!(!key.is_empty() && !value.trim().is_empty(), "{}:{}: empty column", lang.code, i + 1);
                assert!(!value.contains('\t'), "{}:{}: extra column", lang.code, i + 1);
                assert!(keys.insert(key.to_string()), "{}:{}: duplicate {key}", lang.code, i + 1);
                assert_eq!(placeholders(key), placeholders(value), "{}: {key}", lang.code);
                assert_eq!(key.ends_with('…'), value.ends_with('…'), "{}: {key}", lang.code);
            }
            let parsed = parse(lang.source);
            assert_eq!(parsed.len(), keys.len());
            assert!(keys.len() > 500, "{}: incomplete catalog", lang.code);
            catalogs.push(keys);
        }
        assert!(catalogs.len() >= 2);
        for c in catalogs.iter().skip(1) {
            assert_eq!(catalogs.first(), Some(c));
        }
    }

    /// Every translation keeps its key's `{placeholders}`, or `trf` would leave them unfilled.
    #[test]
    fn catalogs_keep_placeholders() {
        let holes = |s: &str| {
            let mut v: Vec<String> = s.split('{').skip(1).filter_map(|p| p.split_once('}')).map(|(h, _)| h.to_string()).collect();
            v.sort();
            v
        };
        for l in LANGUAGES {
            let m = parse(l.source);
            assert!(l.source.is_empty() || m.len() > 100, "{} catalog barely parsed", l.code);
            for (k, v) in &m {
                assert_eq!(holes(k), holes(v), "{}: {k:?}", l.code);
            }
        }
    }

    /// Prevent new bare `ui.label(tr(...))`, `RichText::new(tr(...))`, etc. calls in `ui-egui`
    /// that would bypass `i18n::display()` / RTL helpers and paint logical reading order in RTL.
    #[test]
    fn no_bare_tr_in_ui_egui_widgets() {
        let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let forbidden = [
            "ui.label(tr(",
            "ui.label(trf(",
            "ui.label(&tr(",
            "ui.label(&trf(",
            "ui.label(crate::i18n::tr(",
            "ui.label(crate::i18n::trf(",
            "RichText::new(tr(",
            "RichText::new(trf(",
            "RichText::new(&tr(",
            "RichText::new(&trf(",
            "RichText::new(crate::i18n::tr(",
            "RichText::new(crate::i18n::trf(",
            "on_hover_text(tr(",
            "on_hover_text(trf(",
            "on_hover_text(crate::i18n::tr(",
            "on_hover_text(crate::i18n::trf(",
            "Window::new(tr(",
            "Window::new(trf(",
            "label_text(&tr(",
            "label_text(&trf(",
        ];
        let mut violations = Vec::new();
        let mut stack = vec![src_dir];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") || path.file_name().and_then(|n| n.to_str()) == Some("i18n.rs") {
                    continue;
                }
                let raw = std::fs::read_to_string(&path).unwrap();
                let prod = raw.split("\n#[cfg(test)]").next().unwrap_or(&raw);
                let compact: String = prod.chars().filter(|c| !c.is_whitespace()).collect();
                for pat in forbidden {
                    if compact.contains(pat) {
                        violations.push(format!("{}: contains bare `{pat}` — route through i18n::label / rich_label / rich_text / hover_text", path.display()));
                    }
                }
            }
        }
        assert!(violations.is_empty(), "bare i18n widget calls:\n{}", violations.join("\n"));
    }
}
