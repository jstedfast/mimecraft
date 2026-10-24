// Adapted from MimeKit's Text/RtfInterpreter.cs (ParseHyperlink).
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

/// Extracts a permitted external target from a decoded RTF field instruction.
///
/// Pass the text collected from a `\fldinst` destination, not raw RTF source.
/// This helper does not tokenize RTF, decode escapes, interpret nested groups,
/// or collect instructions. The result borrows the original string without
/// allocating; scanning is linear and uses constant additional storage.
///
/// `HYPERLINK` and the permitted schemes (`http`, `https`, `mailto`, `ftp`, and
/// `tel`) are matched ignoring ASCII case. Whitespace is Unicode whitespace.
/// Switches are case-sensitive: `\l` before the target rejects a local bookmark,
/// `\o` and `\t` consume an argument, and other switches consume only themselves.
///
/// Recovery follows MimeKit: no separator is required after `HYPERLINK`,
/// switch recognition uses their first letter, and an unterminated quoted
/// target extends to the end of the input. Quotes are not backslash-unescaped.
/// Parsing stops at the first target, ignoring any following switches or text.
///
/// Returns `None` for another field type, a missing target, an encountered
/// local-bookmark switch, a disallowed/missing scheme, or control characters
/// remaining in the trimmed target. Rejected targets do not cause a search for
/// a later one. A permitted scheme alone (such as `https:`) is accepted.
///
/// This is **not** full URL validation or HTML sanitization. The returned
/// target is not normalized or encoded; use [`super::html_attribute_encode`]
/// when placing it in an HTML attribute and apply any application URL policy.
/// Unlike the interpreter's bounded field collector, this standalone helper
/// neither truncates nor imposes a length limit on its already-borrowed input.
///
/// ```
/// use mimecraft::text::parse_rtf_hyperlink;
///
/// assert_eq!(
///     parse_rtf_hyperlink(r#"HYPERLINK \o "tooltip" "https://example.com/""#),
///     Some("https://example.com/"),
/// );
/// assert_eq!(parse_rtf_hyperlink(r#"HYPERLINK \l "bookmark""#), None);
/// assert_eq!(parse_rtf_hyperlink(r#"HYPERLINK "file:///tmp/document""#), None);
/// ```
pub fn parse_rtf_hyperlink(instruction: &str) -> Option<&str> {
    const HYPERLINK: &str = "HYPERLINK";
    let mut remaining = instruction.trim_start();
    if !remaining
        .get(..HYPERLINK.len())?
        .eq_ignore_ascii_case(HYPERLINK)
    {
        return None;
    }
    remaining = &remaining[HYPERLINK.len()..];

    // RTF 1.9.1, "Fields", defers to ECMA-376 Part 1, 17.16.5.25:
    // HYPERLINK [switches] "url" [switches]. Switches include \l (bookmark),
    // \m, \n, \o "tooltip", and \t "target frame".
    loop {
        remaining = remaining.trim_start();
        if remaining.is_empty() {
            return None;
        }

        if let Some(switch) = remaining.strip_prefix('\\') {
            // A local bookmark encountered before the target is not an
            // external link. Preserve the source's first-letter matching.
            if switch.starts_with('l') {
                return None;
            }

            // Only \o and \t take arguments; \m and \n do not, so the text
            // following them may be the URL itself.
            let has_argument = switch.starts_with('o') || switch.starts_with('t');
            remaining = split_word(switch).1;
            if has_argument {
                remaining = split_argument(remaining.trim_start()).1;
            }
            continue;
        }

        let url = split_argument(remaining).0.trim();

        // Browsers may strip controls (e.g. "java\tscript:") and thereby
        // defeat a scheme check. Reject controls remaining after trimming.
        if url.chars().any(char::is_control) {
            return None;
        }

        // Retain the source's scheme allowlist, excluding javascript:,
        // vbscript:, data:, and file: from targets intended for an <a href>.
        let (scheme, _) = url.split_once(':')?;
        return ["http", "https", "mailto", "ftp", "tel"]
            .iter()
            .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
            .then_some(url);
    }
}

fn split_word(text: &str) -> (&str, &str) {
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    text.split_at(end)
}

fn split_argument(text: &str) -> (&str, &str) {
    if let Some(quoted) = text.strip_prefix('"') {
        // Accept a missing closing quote by taking the remainder, just as the
        // source does for targets and skipped switch arguments.
        quoted.split_once('"').unwrap_or((quoted, ""))
    } else {
        split_word(text)
    }
}
