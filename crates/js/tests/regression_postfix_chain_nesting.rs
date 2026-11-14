//! Each link of `f()()`, `a[0][1]` or a tagged template chain, and each label in `a: b: c:`, wraps
//! what came before in one more syntax-tree node. The nesting estimate has to count them, or a
//! source deeper than the parser's stack is admitted and the whole process stops on a stack overflow.

use craft_anything_js::lex::{nesting_estimate, tokenize};
use craft_anything_js::{Limits, ParseMode, admit, analyze};

/// Links (or labels) in each chain: more than the native nesting limit of 4096.
const LINKS: usize = 5_000;

/// The nesting estimate for `src`.
fn estimate(src: &str) -> usize {
    let (tokens, truncated) = tokenize(src, 1_000_000);
    assert!(!truncated);
    nesting_estimate(&tokens)
}

/// `LINKS` labels in a row, then one statement.
fn label_run(links: usize) -> String {
    let mut src: String = (0..links).map(|i| format!("l{i}: ")).collect();
    src.push_str("debugger;");
    src
}

/// Calls, index reads and tagged templates that follow one another each add a level.
#[test]
fn postfix_chains_count_every_link() {
    for src in [
        format!("f{};", "()".repeat(LINKS)),
        format!("a{};", "[0]".repeat(LINKS)),
        format!("f{};", "``".repeat(LINKS)),
        format!("f{};", "`a${b}c`".repeat(LINKS)),
    ] {
        // The first link follows a name, not another link.
        let depth = estimate(&src);
        assert!(depth + 1 >= LINKS, "{depth} for {}", &src[..12]);
        assert!(admit(&src, &Limits::default()).is_err(), "{} must be refused", &src[..12]);
    }
}

/// A comment between two links does not end the chain.
#[test]
fn comments_do_not_break_a_chain() {
    let depth = estimate(&format!("f{};", "/* gap */()".repeat(LINKS)));
    assert!(depth >= LINKS, "{depth}");
}

/// JavaScript whitespace outside ASCII (no-break space, byte-order mark, the other space
/// separators) and the vertical tab separate tokens, so they can't end a chain or hide a label.
#[test]
fn every_javascript_space_separates_links() {
    for space in ["\u{b}", "\u{a0}", "\u{feff}", "\u{1680}", "\u{2000}", "\u{200a}", "\u{202f}", "\u{205f}", "\u{3000}"] {
        let calls = estimate(&format!("f{};", format!("(){space}").repeat(LINKS)));
        let index = estimate(&format!("a{};", format!("[0]{space}").repeat(LINKS)));
        let labels: String = (0..LINKS).map(|i| format!("l{i}:{space}")).collect();
        let labels = estimate(&format!("{labels}debugger;"));
        assert!(calls >= LINKS && index >= LINKS && labels + 1 >= LINKS, "{space:?}: {calls} {index} {labels}");
    }
}

/// Those spaces are not part of a name: `typeof` and `if` on either side of one still count.
#[test]
fn javascript_spaces_end_names() {
    let (tokens, _) = tokenize("typeof\u{a0}x\u{feff}if\u{3000}(a)", 100);
    assert_eq!(tokens.iter().map(|token| token.text).collect::<Vec<_>>(), ["typeof", "x", "if", "(", "a", ")"]);
    assert!(estimate(&format!("{}0;", "typeof\u{a0}".repeat(LINKS))) >= LINKS);
}

/// Each label after another wraps the following statement in one more node.
#[test]
fn consecutive_labels_count_every_label() {
    let src = label_run(LINKS);
    let depth = estimate(&src);
    assert!(depth + 1 >= LINKS, "{depth}");
    assert!(admit(&src, &Limits::default()).is_err());
}

/// Statements, case clauses, object keys and conditionals keep a small estimate.
#[test]
fn ordinary_code_stays_shallow() {
    let declarations = "function a() {}\nfunction b() {}\n".repeat(1_000);
    let cases: String = (0..1_000).map(|i| format!("case {i}: case 'k{i}': ")).collect();
    let switch = format!("switch (x) {{ {cases} default: y(); }}");
    let objects = "x = { a: 1, b: f(), c: [0][0] };\n".repeat(1_000);
    let conditionals = "x = a ? b : c ? d : e;\n".repeat(1_000);
    let statements = "f(a)(b); g[0][1]; tag`x`; label: for (;;) break label;\n".repeat(1_000);
    for src in [declarations, switch, objects, conditionals, statements] {
        let depth = estimate(&src);
        assert!(depth < 16, "{depth} for {}", &src[..24]);
    }
}

/// A two-million-link call chain stopped the whole process with a stack overflow; it now takes
/// the lexer fallback.
#[test]
fn deep_call_chain_takes_the_lexer_fallback() {
    let src = format!("f{};", "()".repeat(2_000_000));
    let analysis = analyze("chain.js", &src, &Limits::default());
    assert_eq!(analysis.facts.mode, ParseMode::Lexer);
    assert!(analysis.facts.mode_reason.as_deref().is_some_and(|reason| reason.contains("nests about")), "{:?}", analysis.facts.mode_reason);
}

/// The same with a no-break space after every call, which used to end the chain for the estimate.
#[test]
fn deep_spaced_call_chain_takes_the_lexer_fallback() {
    let src = format!("f{};", "()\u{a0}".repeat(2_000_000));
    assert_eq!(analyze("spaced.js", &src, &Limits::default()).facts.mode, ParseMode::Lexer);
}

/// The same for two million labels in a row.
#[test]
fn deep_label_run_takes_the_lexer_fallback() {
    let analysis = analyze("labels.js", &label_run(2_000_000), &Limits::default());
    assert_eq!(analysis.facts.mode, ParseMode::Lexer);
}
