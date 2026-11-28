//! Versions found in folder and file names, compared the way people mean them: `12.10.0` is newer
//! than `12.2` even though it sorts before it as text. Names without a version are never guessed
//! at: [`version_order`] ranks them below the names that carry a version and falls back to plain
//! text among themselves, and [`parse`] returns `None`.

/// A version read out of a name: `ghidra_12.2_PUBLIC` is `[12, 2]`, `jdk-25.0.5.1+2` is
/// `[25, 0, 5, 1]` (it stops before the build number after `+` or `_`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub Vec<u32>);

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let parts = self.0.iter().map(u32::to_string).collect::<Vec<_>>().join(".");
        f.write_str(&parts)
    }
}

/// The first run of `digits(.digits)*` in `name`: `ghidra_12.2_PUBLIC` → `12.2`,
/// `jdk-25.0.5.1+2` → `25.0.5.1`, `ghidra-custom` → `None`. A run ends at any other character
/// (or at a `.` not followed by a digit), so `12..3` and `12.` both give `12`. Never panics on
/// odd names; a part too large for a `u32` is skipped.
pub fn parse(name: &str) -> Option<Version> {
    let b = name.as_bytes();
    let start = b.iter().position(u8::is_ascii_digit)?;
    let mut end = start;
    while end < b.len() {
        if b[end].is_ascii_digit() || (b[end] == b'.' && end + 1 < b.len() && b[end + 1].is_ascii_digit()) {
            end += 1;
        } else {
            break;
        }
    }
    let parts: Vec<u32> = name[start..end].split('.').filter_map(|p| p.parse().ok()).collect();
    (!parts.is_empty()).then_some(Version(parts))
}

/// Folder and file names in version order, newest last: the versions [`parse`] reads compare as
/// numbers first (`java-8` before `java-17`, `12.1.4` before `12.10.0`, `12.2` before `12.2.1`
/// however the rest of the name reads), and a name that carries a version outranks one that
/// carries none (`ghidra_custom` before `ghidra_12.2_PUBLIC`), because a name without a version
/// can't be told from an older release. Only names that name the same version (or none) are
/// compared as text. Searches sort by it and take the last match, so the newest and the versioned
/// win.
pub fn version_order(a: &str, b: &str) -> std::cmp::Ordering {
    match parse(a).cmp(&parse(b)) {
        std::cmp::Ordering::Equal => {}
        o => return o,
    }
    fn parts(s: &str) -> Vec<(bool, &str)> {
        let mut out = Vec::new();
        let mut start = 0;
        let mut digit = None;
        for (i, c) in s.char_indices() {
            let d = c.is_ascii_digit();
            if digit.is_some_and(|was| was != d) {
                out.push((digit == Some(true), &s[start..i]));
                start = i;
            }
            digit = Some(d);
        }
        if start < s.len() {
            out.push((digit == Some(true), &s[start..]));
        }
        out
    }
    let (pa, pb) = (parts(a), parts(b));
    for ((da, sa), (db, sb)) in pa.iter().zip(&pb) {
        let o = if *da && *db {
            let (ta, tb) = (sa.trim_start_matches('0'), sb.trim_start_matches('0'));
            ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb))
        } else {
            sa.cmp(sb)
        };
        if o.is_ne() {
            return o;
        }
    }
    pa.len().cmp(&pb.len()).then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering::*;

    #[test]
    fn versions_read_out_of_names_compare_numerically() {
        assert!(parse("ghidra_12.10.0_PUBLIC") > parse("ghidra_12.2_PUBLIC"));
        assert!(parse("ghidra_12.2_PUBLIC") > parse("ghidra_12.1.4_PUBLIC"));
        assert_eq!(parse("jdk-25.0.5.1+2"), Some(Version(vec![25, 0, 5, 1])));
        assert_eq!(parse("ghidra_12_PUBLIC"), Some(Version(vec![12])));
        // Trailing separators and empty runs end the version, they don't grow it.
        assert_eq!(parse("12..3"), Some(Version(vec![12])));
        assert_eq!(parse("12."), Some(Version(vec![12])));
        // Leading zeros are the number they name.
        assert_eq!(parse("jdk-021"), Some(Version(vec![21])));
    }

    #[test]
    fn names_without_a_version_parse_to_none() {
        assert_eq!(parse("ghidra-custom"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("bin"), None);
    }

    #[test]
    fn odd_names_never_panic() {
        let long = format!("ghidra_{}.0_PUBLIC", "9".repeat(64));
        assert!(parse(&long).is_some(), "an overlong part is skipped, not a panic");
        for odd in ["ghidra_12.2.\u{0}", "12.2.999999999999999999999", "…12.2"] {
            let _ = parse(odd);
            let _ = version_order(odd, odd);
            assert_eq!(version_order(odd, odd), Equal);
        }
    }

    #[test]
    fn version_order_keeps_its_text_fallback() {
        assert_eq!(version_order("java-8", "java-17"), Less);
        assert_eq!(version_order("a", "a"), Equal);
        assert_eq!(version_order("jdk-021", "jdk-21"), Less);
        assert_eq!(version_order("z", "y"), Greater);
        // A name without a version never outranks one that says its version, however it reads.
        assert_eq!(version_order("ghidra_custom", "ghidra_12.2_PUBLIC"), Less);
        assert_eq!(version_order("ghidra_12.2_PUBLIC", "ghidra_custom"), Greater);
        assert_eq!(version_order("jdk-25.0.5.1+2", "Home"), Greater);
        // The version decides before the rest of the name does: `_PUBLIC` sorts after `.`, which
        // put 12.2 above 12.2.1 when only the text was compared.
        assert_eq!(version_order("ghidra_12.2_PUBLIC", "ghidra_12.2.1_PUBLIC"), Less);
    }
}
