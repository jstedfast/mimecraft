// Adapted from MimeKit's Text/UrlScanner.cs and Text/TextConverter.cs.
// Test cases adapted from UnitTests/Text/UrlScannerTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use unicode_general_category::{GeneralCategory, get_general_category};

use crate::{
    idna,
    parse_utils::{is_ascii_atom, skip_ipv4, skip_ipv6},
};

// RFC 5321, 4.5.3.1.1-2 limits local parts to 64 octets and domains to 255.
// RFC 1035, 2.3.4 limits labels to 63 octets. Count UTF-8 bytes, not UTF-16 units.
const MAX_LOCAL_PART: usize = 64;
const MAX_DOMAIN: usize = 255;
const MAX_LABEL: usize = 63;

#[derive(Clone, Copy)]
enum Kind {
    Address,
    MailTo,
    File,
    Web,
}

type Pattern = (&'static str, &'static str, Kind);

const ADDRESS_PATTERNS: &[Pattern] = &[("@", "mailto:", Kind::Address)];
const HOST_PATTERNS: &[Pattern] = &[
    ("www.", "http://", Kind::Web),
    ("ftp.", "ftp://", Kind::Web),
];
// Earlier starts first, then longer patterns at the same start: leftmost-longest.
// The fixed source patterns replace the C# scanner's sorted registration lists.
const SCHEME_PATTERNS: &[Pattern] = &[
    ("telnet://", "", Kind::Web),
    ("webcal://", "", Kind::Web),
    ("mailto:", "", Kind::MailTo),
    ("callto:", "", Kind::Web),
    ("https://", "", Kind::Web),
    ("file://", "", Kind::File),
    ("sftp://", "", Kind::Web),
    ("http://", "", Kind::Web),
    ("news://", "", Kind::Web),
    ("nntp://", "", Kind::Web),
    ("h323:", "", Kind::Web),
    ("ftp://", "", Kind::Web),
    ("sip:", "", Kind::Web),
];

pub(super) struct UrlMatch {
    pub start: usize,
    pub end: usize,
    pub prefix: &'static str,
}

pub(super) fn scan(text: &str) -> Option<UrlMatch> {
    // Every pattern has exactly one anchor, and no earlier part of the pattern
    // contains an anchor. Earlier anchors therefore yield earlier matches.
    for (anchor_index, anchor) in text.bytes().enumerate() {
        let patterns = match anchor {
            b'@' => ADDRESS_PATTERNS,
            b':' => SCHEME_PATTERNS,
            b'.' => HOST_PATTERNS,
            _ => continue,
        };
        // A failed candidate (such as a lone '@') must not hide later links.
        // Local parts, domains and literals are bounded; userinfo lookahead
        // stops before the next scheme candidate.
        for &(pattern, prefix, kind) in patterns {
            let offset = pattern
                .find(char::from(anchor))
                .expect("pattern has its anchor");
            let Some(index) = anchor_index.checked_sub(offset) else {
                continue;
            };
            let content = index + pattern.len();
            // Fold only ASCII letters, independent of the user's locale.
            if !text
                .as_bytes()
                .get(index..content)
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(pattern.as_bytes()))
            {
                continue;
            }
            let (start, end) = match kind {
                Kind::Address => {
                    let Some(start) = address_start(text, index) else {
                        continue;
                    };
                    let Some(end) = address_domain(text, index + 1, true) else {
                        continue;
                    };
                    (start, end)
                }
                Kind::MailTo => {
                    let Some(end) = mailto_end(text, index, content) else {
                        continue;
                    };
                    (index, end)
                }
                Kind::File => {
                    let close = closing_brace(text, index);
                    let mut end = skip_url_suffix(text, content, close);
                    end = trim_trailing_punctuation(text, content, end);
                    if end == content {
                        continue;
                    }
                    (index, end)
                }
                Kind::Web => {
                    let Some(end) = web_end(text, index, content, pattern) else {
                        continue;
                    };
                    (index, end)
                }
            };
            return Some(UrlMatch { start, end, prefix });
        }
    }
    None
}

fn character(text: &str, index: usize) -> Option<char> {
    text[index..].chars().next()
}

fn closing_brace(text: &str, start: usize) -> Option<char> {
    match text[..start].chars().next_back()? {
        '(' => Some(')'),
        '{' => Some('}'),
        '[' => Some(']'),
        '<' => Some('>'),
        '|' => Some('|'),
        _ => None,
    }
}

fn is_non_ascii_url_safe(c: char) -> bool {
    // Allow IRIs and internationalized addresses, but not whitespace (including
    // NBSP and U+2028), controls, or invisible formatting characters such as
    // bidi overrides and zero-width spaces that can disguise the link target.
    !c.is_whitespace() && !c.is_control() && get_general_category(c) != GeneralCategory::Format
}

fn is_url_safe(c: char) -> bool {
    if !c.is_ascii() {
        return is_non_ascii_url_safe(c);
    }
    c.is_ascii_alphanumeric() || "$-_.+!*'(),{}|\\^~[]`#%\";/?:@&=".contains(c)
}

fn is_atom(c: char) -> bool {
    if c.is_ascii() {
        is_ascii_atom(c as u8)
    } else {
        is_non_ascii_url_safe(c)
    }
}

fn is_domain(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphanumeric() || c == '-'
    } else {
        is_non_ascii_url_safe(c)
    }
}

fn is_domain_start(c: char) -> bool {
    c != '-' && is_domain(c)
}

fn is_letter(c: char) -> bool {
    matches!(
        get_general_category(c),
        GeneralCategory::UppercaseLetter
            | GeneralCategory::LowercaseLetter
            | GeneralCategory::TitlecaseLetter
            | GeneralCategory::ModifierLetter
            | GeneralCategory::OtherLetter
    )
}

fn skip_subdomain(text: &str, limit: usize, index: &mut usize) -> bool {
    // The caller has checked the first character. Scan one character beyond
    // the byte limit to detect overlong labels without splitting a UTF-8 scalar.
    let start = *index;
    *index += character(text, *index).expect("domain start").len_utf8();
    while *index < limit && *index - start <= MAX_LABEL {
        let c = character(text, *index).expect("within domain limit");
        if !is_domain(c) {
            break;
        }
        *index += c.len_utf8();
    }
    *index - start <= MAX_LABEL
}

fn skip_domain(text: &str, start: usize, mut index: usize) -> Option<usize> {
    // Include a consumed "www."/"ftp." prefix in the domain length. Look one
    // byte beyond the maximum so an overlong domain is rejected, not truncated.
    let limit = text.len().min(start.saturating_add(MAX_DOMAIN + 1));
    if index >= limit
        || !character(text, index).is_some_and(is_domain_start)
        || !skip_subdomain(text, limit, &mut index)
    {
        return None;
    }
    while index < limit && text.as_bytes()[index] == b'.' {
        let dot = index;
        index += 1;
        if index == limit {
            // Check whether the domain continues beyond the maximum length.
            if character(text, index).is_some_and(is_domain_start) {
                return None;
            }
            index = dot;
            break;
        }
        if !character(text, index).is_some_and(is_domain_start) {
            index = dot;
            break;
        }
        if !skip_subdomain(text, limit, &mut index) {
            return None;
        }
    }
    (index - start <= MAX_DOMAIN).then_some(index)
}

fn skip_word(text: &str, limit: usize, index: &mut usize) -> bool {
    if character(text, *index) == Some('"') {
        // Skip the opening quote, respecting escaped quotes and backslashes.
        *index += 1;
        let mut escaped = false;
        while *index < limit {
            let c = character(text, *index).expect("within local-part limit");
            *index += c.len_utf8();
            if c == '\\' {
                escaped = !escaped;
            } else if !escaped {
                if c == '"' {
                    return true;
                }
            } else {
                escaped = false;
            }
        }
        false
    } else {
        let start = *index;
        while *index < limit {
            let c = character(text, *index).expect("within local-part limit");
            if !is_atom(c) {
                break;
            }
            *index += c.len_utf8();
        }
        *index > start
    }
}

fn skip_word_backwards(text: &str, minimum: usize, mut index: usize) -> Option<usize> {
    if text[..index].ends_with('"') {
        // Skip the closing quote. Retain the source's backwards escaped-quote
        // rule, which considers a quote preceded by a backslash escaped.
        index -= 1;
        while index > minimum {
            let c = text[..index].chars().next_back()?;
            index -= c.len_utf8();
            if c == '"' && (index == minimum || text.as_bytes()[index - 1] != b'\\') {
                return Some(index);
            }
        }
        None
    } else {
        let end = index;
        while index > minimum {
            let c = text[..index].chars().next_back()?;
            if !is_atom(c) {
                break;
            }
            index -= c.len_utf8();
        }
        (index < end).then_some(index)
    }
}

fn address_start(text: &str, at: usize) -> Option<usize> {
    // Bound backwards scanning to one byte beyond the local-part limit.
    // Advancing by whole scalars keeps every returned slice boundary valid.
    let minimum = at.saturating_sub(MAX_LOCAL_PART + 1);
    let mut index = at;
    loop {
        index = skip_word_backwards(text, minimum, index)?;
        if index <= minimum || text.as_bytes()[index - 1] != b'.' {
            break;
        }
        index -= 1;
        if index <= minimum {
            return None;
        }
    }
    (at - index <= MAX_LOCAL_PART).then_some(index)
}

fn is_fully_qualified_domain(domain: &str) -> bool {
    // Require two labels and an alphabetic TLD or a validated IDNA A-label
    // to avoid linking bare addresses such as "foo@bar".
    let Some((_, tld)) = domain.rsplit_once('.') else {
        return false;
    };
    if tld.chars().count() < 2 {
        return false;
    }
    if tld.chars().all(is_letter) {
        return true;
    }
    if tld.len() <= 4 || tld.len() > MAX_LABEL || !tld.as_bytes()[..4].eq_ignore_ascii_case(b"xn--")
    {
        return false;
    }
    // Require decoding and a canonical round-trip: recognizing "xn--" alone
    // accepts fake A-labels. Only this bounded label needs IDNA allocations.
    // Unlike the C# provider, our existing IDNA helpers report invalid input.
    let Ok(unicode) = idna::to_unicode(tld) else {
        return false;
    };
    !unicode.eq_ignore_ascii_case(tld)
        && idna::to_ascii(&unicode).is_ok_and(|ascii| ascii.eq_ignore_ascii_case(tld))
}

fn address_domain(text: &str, mut index: usize, fully_qualified: bool) -> Option<usize> {
    if text.as_bytes().get(index) != Some(&b'[') {
        let start = index;
        index = skip_domain(text, start, index)?;
        if fully_qualified && !is_fully_qualified_domain(&text[start..index]) {
            return None;
        }
    } else {
        index += 1;
        // Both "1.2.3.4]" and "IPv6:::]" need exactly eight remaining bytes,
        // including ']'. The literal may end exactly at the scan boundary.
        if text.len() - index < 8 {
            return None;
        }
        if text.as_bytes()[index..index + 5].eq_ignore_ascii_case(b"IPv6:") {
            index += 5;
            if !skip_ipv6(text, &mut index, true) {
                return None;
            }
        } else if !skip_ipv4(text, &mut index, true) {
            return None;
        }
        if text.as_bytes().get(index) != Some(&b']') {
            return None;
        }
        index += 1;
    }
    Some(index)
}

fn skip_address(text: &str, start: usize) -> Option<usize> {
    // Limit forward local-part scanning too, with one extra byte to detect
    // overlong candidates instead of accepting a truncated prefix.
    let limit = text.len().min(start.saturating_add(MAX_LOCAL_PART + 1));
    let mut index = start;
    if !skip_word(text, limit, &mut index) || index >= limit {
        return None;
    }
    while text.as_bytes()[index] == b'.' {
        index += 1;
        if index >= limit || !skip_word(text, limit, &mut index) || index >= limit {
            return None;
        }
    }
    if index - start > MAX_LOCAL_PART || text.as_bytes().get(index) != Some(&b'@') {
        return None;
    }
    address_domain(text, index + 1, false)
}

fn mailto_end(text: &str, start: usize, content: usize) -> Option<usize> {
    let close = closing_brace(text, start);
    let mut index = skip_address(text, content).unwrap_or(content);
    if text.as_bytes().get(index) == Some(&b'?') {
        let query = index;
        index = skip_url_suffix(text, index, close);
        index = trim_trailing_punctuation(text, query, index);
    }
    (index > content).then_some(index)
}

fn is_enclosing_delimiter(c: char, close: Option<char>, depth: &mut usize) -> bool {
    if Some(c) == close {
        if *depth == 0 {
            return true;
        }
        *depth -= 1;
    } else if matches!(
        (c, close),
        ('(', Some(')')) | ('[', Some(']')) | ('{', Some('}'))
    ) {
        *depth += 1;
    }
    false
}

fn skip_url_suffix(text: &str, mut index: usize, close: Option<char>) -> usize {
    let mut depth = 0;
    // A matching inner pair belongs to the URL, not its enclosing prose.
    // Only the outer delimiter needs a depth counter; trailing unmatched
    // closers are trimmed separately.
    while let Some(c) = character(text, index) {
        if !is_url_safe(c) || is_enclosing_delimiter(c, close, &mut depth) {
            break;
        }
        index += c.len_utf8();
    }
    index
}

fn skip_user_info(text: &str, start: usize, close: Option<char>) -> usize {
    let mut index = start;
    let mut depth = 0;
    // RFC 3986, 3.2.1: userinfo = *(unreserved / pct-encoded / sub-delims / ":").
    // Only called for "://" patterns: '/' in the next such candidate stops
    // lookahead. Never rescan unbounded runs of implied hosts or "sip:" prefixes.
    while let Some(c) = character(text, index) {
        if c == '@' {
            return index + 1;
        }
        if is_enclosing_delimiter(c, close, &mut depth) {
            break;
        }
        if c == '%' {
            if !text
                .as_bytes()
                .get(index + 1)
                .is_some_and(u8::is_ascii_hexdigit)
                || !text
                    .as_bytes()
                    .get(index + 2)
                    .is_some_and(u8::is_ascii_hexdigit)
            {
                break;
            }
            index += 3;
        } else if (!c.is_ascii() && is_non_ascii_url_safe(c))
            || c.is_ascii_alphanumeric()
            || "-._~!$&'()*+,;=:".contains(c)
        {
            index += c.len_utf8();
        } else {
            break;
        }
    }
    start
}

fn web_end(text: &str, start: usize, mut index: usize, pattern: &str) -> Option<usize> {
    let close = closing_brace(text, start);
    let authority = pattern.ends_with("://");
    if authority {
        index = skip_user_info(text, index, close);
    }
    // For "www." and "ftp.", the pattern is part of the hostname. Otherwise,
    // measure the host after userinfo so its length does not count as host data.
    let host = if pattern.ends_with('.') { start } else { index };
    if authority && text.as_bytes().get(index) == Some(&b'[') {
        index += 1;
        if !skip_ipv6(text, &mut index, false) || text.as_bytes().get(index) != Some(&b']') {
            return None;
        }
        index += 1;
    } else {
        index = skip_domain(text, host, index)?;
    }
    // Check for a port; retain the source's digit-only, unbounded port spelling.
    if text.as_bytes().get(index) == Some(&b':')
        && text
            .as_bytes()
            .get(index + 1)
            .is_some_and(u8::is_ascii_digit)
    {
        index += 2;
        while text.as_bytes().get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
    }
    // Query and fragment data use pchar / "/" / "?" (RFC 3986, 3.4-3.5),
    // not letter/digit-prefixed parameters. Keep an empty '#' but trim prose.
    if matches!(text.as_bytes().get(index), Some(b'/' | b'?' | b'#')) {
        let path = index;
        index = skip_url_suffix(text, index, close);
        index = trim_trailing_punctuation(text, path, index);
    }
    Some(index)
}

fn trim_trailing_punctuation(text: &str, start: usize, mut end: usize) -> usize {
    // Prose punctuation usually belongs outside the URL. Strip closing brackets
    // only when unbalanced, preserving paths such as "/wiki/Foo_(bar)".
    let mut balance = [0isize; 3];
    for byte in text.as_bytes()[start..end].iter().copied() {
        match byte {
            b'(' => balance[0] += 1,
            b')' => balance[0] -= 1,
            b'[' => balance[1] += 1,
            b']' => balance[1] -= 1,
            b'{' => balance[2] += 1,
            b'}' => balance[2] -= 1,
            _ => {}
        }
    }
    while end > start {
        let byte = text.as_bytes()[end - 1];
        if b".,:;!?'\"*".contains(&byte) {
            end -= 1;
        } else {
            let slot = match byte {
                b')' => 0,
                b']' => 1,
                b'}' => 2,
                _ => break,
            };
            if balance[slot] >= 0 {
                break;
            }
            balance[slot] += 1;
            end -= 1;
        }
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(input: &str, expected: Option<&str>) {
        assert_eq!(
            scan(input).map(|found| &input[found.start..found.end]),
            expected,
            "{input:?}"
        );
    }

    fn assert_match(input: &str, expected: Option<&str>, prefix: &str) {
        // Exercise physical EOF and a bounded UTF-8 slice whose surrounding
        // text must not affect the match or its offsets.
        let padding = "\u{1f600}skip ";
        let surrounded = format!("{padding}{input}outside http://ignored.example");
        for text in [
            input,
            &surrounded[padding.len()..padding.len() + input.len()],
        ] {
            let found = scan(text);
            if let Some(expected) = expected {
                let found = found.unwrap_or_else(|| panic!("missing match in {input:?}"));
                assert_eq!(found.start, input.find(expected).unwrap(), "{input:?}");
                assert_eq!(found.end, found.start + expected.len(), "{input:?}");
                assert_eq!(found.prefix, prefix, "{input:?}");
                assert_eq!(&text[found.start..found.end], expected, "{input:?}");
            } else {
                assert!(found.is_none(), "unexpected match in {input:?}");
            }
        }
    }

    #[test]
    fn source_patterns_and_addresses() {
        for (input, expected) in [
            ("nothing to match...", None),
            (
                "simple.addrspec@example.com.",
                Some("simple.addrspec@example.com"),
            ),
            (
                "\"quoted local part\"@example.com",
                Some("\"quoted local part\"@example.com"),
            ),
            (
                "\"quoted \\\"local\\\" part\"@example.com",
                Some("\"quoted \\\"local\\\" part\"@example.com"),
            ),
            ("ipv4@[127.0.0.1]", Some("ipv4@[127.0.0.1]")),
            ("ipv6@[IPv6:::1]", Some("ipv6@[IPv6:::1]")),
            (
                "ipv6@[IPv6:::ffff:10.0.0.1]",
                Some("ipv6@[IPv6:::ffff:10.0.0.1]"),
            ),
            (
                "ipv6@[IPv6:FE80:0000:0000:0000:0202:B3FF:FE1E:8329]",
                Some("ipv6@[IPv6:FE80:0000:0000:0000:0202:B3FF:FE1E:8329]"),
            ),
            (
                "mailto:simple.addrspec@example.com",
                Some("mailto:simple.addrspec@example.com"),
            ),
            (
                "mailto:\"quoted local part\"@example.com",
                Some("mailto:\"quoted local part\"@example.com"),
            ),
            (
                "mailto:\"quoted \\\"local\\\" part\"@example.com",
                Some("mailto:\"quoted \\\"local\\\" part\"@example.com"),
            ),
            ("mailto:ipv4@[127.0.0.1]", Some("mailto:ipv4@[127.0.0.1]")),
            ("mailto:ipv6@[IPv6:::1]", Some("mailto:ipv6@[IPv6:::1]")),
            (
                "mailto:?subject=Shake%20it%20off,%20shake%20it%20off",
                Some("mailto:?subject=Shake%20it%20off,%20shake%20it%20off"),
            ),
            (
                "mailto:taylor.swift@mtv.com?subject=Shake%20it%20off,%20shake%20it%20off",
                Some("mailto:taylor.swift@mtv.com?subject=Shake%20it%20off,%20shake%20it%20off"),
            ),
            (
                "file:///path/to/some/filename.txt",
                Some("file:///path/to/some/filename.txt"),
            ),
            ("http://www.xamarin.com", Some("http://www.xamarin.com")),
            (
                "http://www.xamarin.com:80/logo.png",
                Some("http://www.xamarin.com:80/logo.png"),
            ),
            (
                "http://www.xamarin.com:80?query",
                Some("http://www.xamarin.com:80?query"),
            ),
            (
                "http://www.xamarin.com?query",
                Some("http://www.xamarin.com?query"),
            ),
            (
                "http://www.xamarin.com?? Wow!",
                Some("http://www.xamarin.com"),
            ),
            ("https://23andme.com??", Some("https://23andme.com")),
            ("ftp.", None),
            ("wwww.example.com", Some("www.example.com")),
            ("sftp.example.com", Some("ftp.example.com")),
            ("hhttp://example.com", Some("http://example.com")),
            ("SFTP://example.com", Some("SFTP://example.com")),
            ("HtTpS://example.com", Some("HtTpS://example.com")),
            ("WWW.example.com", Some("WWW.example.com")),
            ("ftp:example.com", None),
            ("http:/example.com", None),
            ("http//example.com", None),
            ("http:", None),
            ("file:/", None),
            ("www", None),
            ("ww.example.com", None),
            ("a.b@example.com", Some("a.b@example.com")),
            ("a.b.c@example.com", Some("a.b.c@example.com")),
            (".b@example.com", None),
            ("FILE://server/share", Some("FILE://server/share")),
            ("MAILTO:user@example.com", Some("MAILTO:user@example.com")),
            ("f\u{130}le://server/share", None),
            ("f\u{131}le://server/share", None),
            (
                "Email me @ home or visit http://www.example.com",
                Some("http://www.example.com"),
            ),
            (
                "Visit ftp. or www. or http:// or mailto: or file:// or http://example.com",
                Some("http://example.com"),
            ),
        ] {
            check(input, expected);
        }
    }

    #[test]
    fn source_prose_punctuation_and_enclosing_brackets() {
        for suffix in [
            ".", ", then", "; then", ": it", "!", "?!", "...", ").", "]", "}",
        ] {
            check(
                &format!("See http://example.com/path{suffix}"),
                Some("http://example.com/path"),
            );
        }
        for (open, close) in [
            ("(", ")"),
            ("{", "}"),
            ("<", ">"),
            ("[", "]"),
            ("|", "|"),
            ("'", "'"),
            ("\"", "\""),
            ("*", "*"),
        ] {
            check(
                &format!("See {open}http://example.com/path{close}"),
                Some("http://example.com/path"),
            );
        }
        for (input, expected) in [
            ("http://example.com/.", Some("http://example.com/")),
            (
                "http://example.com/path?q=1.",
                Some("http://example.com/path?q=1"),
            ),
            (
                "http://example.com/a.b/c.html.",
                Some("http://example.com/a.b/c.html"),
            ),
            (
                "http://en.wikipedia.org/wiki/Foo_(bar)).",
                Some("http://en.wikipedia.org/wiki/Foo_(bar)"),
            ),
            ("http://example.com/a[0]", Some("http://example.com/a[0]")),
            ("http://example.com/a{0}", Some("http://example.com/a{0}")),
            ("www.example.com/path.", Some("www.example.com/path")),
            (
                "file:///path/to/file.txt, ok",
                Some("file:///path/to/file.txt"),
            ),
            (
                "mailto:user@example.com?subject=hi.",
                Some("mailto:user@example.com?subject=hi"),
            ),
            ("mailto:user@example.com?", Some("mailto:user@example.com")),
            ("mailto:?subject=hi!", Some("mailto:?subject=hi")),
            ("mailto:?.", None),
        ] {
            check(input, expected);
        }
    }

    #[test]
    fn unicode_boundaries_and_letter_categories() {
        for c in [
            '\u{a0}',
            '\u{2028}',
            '\u{3000}',
            '\u{202e}',
            '\u{200b}',
            '\u{2066}',
            '\u{feff}',
            '\u{85}',
            '\u{ad}',
            '\u{e0001}',
        ] {
            check(
                &format!("http://example.com/{c}hidden"),
                Some("http://example.com/"),
            );
            check(&format!("user{c}@example.com"), None);
        }
        for (input, expected) in [
            ("http://ex\u{202e}ample.com", Some("http://ex")),
            ("user@example.com\u{a0}x", Some("user@example.com")),
            (
                "http://b\u{fc}cher.example/stra\u{df}e",
                Some("http://b\u{fc}cher.example/stra\u{df}e"),
            ),
            (
                "jos\u{e9}@b\u{fc}cher.example",
                Some("jos\u{e9}@b\u{fc}cher.example"),
            ),
            ("user@localhost", None),
            ("user@example", None),
            ("user@example.c", None),
            ("user@example.c0m", None),
            ("user@example.123", None),
            ("user@example.co", Some("user@example.co")),
            ("user@mail.example.co.uk", Some("user@mail.example.co.uk")),
            (
                "user@\u{43f}\u{440}\u{438}\u{43c}\u{435}\u{440}.\u{440}\u{444}",
                Some("user@\u{43f}\u{440}\u{438}\u{43c}\u{435}\u{440}.\u{440}\u{444}"),
            ),
            ("mailto:user@localhost", Some("mailto:user@localhost")),
            ("user@example.\u{e9}", None),
            (
                "user@example.\u{e9}\u{e9}",
                Some("user@example.\u{e9}\u{e9}"),
            ),
            // Rust's is_alphabetic includes some marks and letter numbers;
            // MimeKit's IsLetter uses only Unicode letter general categories.
            ("user@example.a\u{345}", None),
            ("user@example.a\u{2160}", None),
            (
                "http://example.com/?\u{661}=1",
                Some("http://example.com/?\u{661}=1"),
            ),
            (
                "http://example.com/?\u{b2}=1",
                Some("http://example.com/?\u{b2}=1"),
            ),
        ] {
            check(input, expected);
        }
    }

    #[test]
    fn bounded_local_parts_labels_and_domains() {
        for local in [
            "a".repeat(64),
            format!("{}.{}", "a".repeat(32), "b".repeat(31)),
            format!("\"{}\"", "a".repeat(62)),
            "\u{e9}".repeat(32),
        ] {
            for prefix in ["", "mailto:"] {
                let input = format!("{prefix}{local}@example.com");
                check(&input, Some(&input));
            }
        }
        for local in [
            "a".repeat(65),
            format!("{}.{}", "a".repeat(32), "b".repeat(32)),
            format!("\"{}\"", "a".repeat(63)),
            "\u{e9}".repeat(33),
        ] {
            check(&format!("x {local}@example.com"), None);
            check(&format!("mailto:{local}@example.com"), None);
        }
        for label in ["a".repeat(63), format!("{}a", "\u{e9}".repeat(31))] {
            for prefix in ["user@", "mailto:user@", "http://", "www."] {
                let input = format!("{prefix}{label}.com");
                check(&input, Some(&input));
            }
        }
        for label in ["a".repeat(64), "\u{e9}".repeat(32)] {
            for prefix in [
                "user@",
                "mailto:user@",
                "http://",
                "http://example.",
                "www.",
            ] {
                check(&format!("{prefix}{label}.com"), None);
            }
        }
        let label = "a".repeat(63);
        let maximum = format!("{label}.{label}.{label}.{}.com", "a".repeat(59));
        let overlong = format!("{label}.{label}.{label}.{}.com", "a".repeat(60));
        assert_eq!(maximum.len(), 255);
        assert_eq!(overlong.len(), 256);
        for prefix in ["user@", "mailto:user@", "http://", "https://"] {
            let input = format!("{prefix}{maximum}");
            check(&input, Some(&input));
            check(&format!("{prefix}{overlong}"), None);
        }
        let input = format!("www.{}", &maximum[4..]);
        check(&input, Some(&input));
        check(&format!("www.{}", &overlong[4..]), None);
        let input = format!("user@{maximum}");
        check(&format!("{input}."), Some(&input));
        check(&format!("user@{label}.{label}.{label}.{label}.com"), None);
        // The maximum-domain boundary can fall inside a UTF-8 scalar.
        check(
            &format!("http://{label}.{label}.{label}.{}\u{1f600}", "a".repeat(62)),
            None,
        );
    }

    #[test]
    fn source_literal_recovery_and_heuristics() {
        for input in [
            "a@[99999999999999999999.1.1.1]",
            "a@[1.2.3.4444]",
            "a@[IPv6:fffff::1]",
            "a@[IPv6:1::2::3]",
            "a@[IPv6:1:2:3:4:5:6:7:8:9]",
            "a@[IPv6::::1]",
        ] {
            check(input, None);
            check(&format!("mailto:{input}"), None);
        }
        check(&format!("a@[IPv6:{}]", "f".repeat(100_000)), None);
        check("a@[1.2.3.4]", Some("a@[1.2.3.4]"));
        check("a@[1.2.3.4] ", Some("a@[1.2.3.4]"));
        check("http://host/#fragment", Some("http://host/#fragment"));
        check("http://host#fragment", Some("http://host#fragment"));
        check("http://user@example.com", Some("http://user@example.com"));
        check("http://[::1]/", Some("http://[::1]/"));
        check("(http://host/Foo_(bar))", Some("http://host/Foo_(bar)"));
        check("http://host:999999", Some("http://host:999999"));
    }

    #[test]
    fn ranges_prefixes_and_fixed_pattern_order() {
        let input = "http://example.com and www.example.org";
        check(&input[2..], Some("www.example.org"));
        check(&input[..6], None);
        check("ww.example.org", None);
        for patterns in [ADDRESS_PATTERNS, HOST_PATTERNS, SCHEME_PATTERNS] {
            let mut previous = (usize::MAX, usize::MAX);
            for &(pattern, prefix, kind) in patterns {
                let anchors: Vec<_> = pattern.match_indices(['@', ':', '.']).collect();
                assert_eq!(anchors.len(), 1);
                let key = (anchors[0].0, pattern.len());
                assert!(key <= previous, "{pattern}");
                previous = key;
                let input = match kind {
                    Kind::Address => "me@example.com".to_owned(),
                    Kind::MailTo => format!("{pattern}me@example.com"),
                    _ => format!("{pattern}example.com"),
                };
                let found = scan(&input).unwrap();
                assert_eq!(found.prefix, prefix);
                assert_eq!(&input[found.start..found.end], input);
            }
        }
    }

    #[test]
    fn utf8_slicing_and_length_boundaries() {
        for c in [
            '\0',
            '\u{80}',
            '\u{e9}',
            '\u{202e}',
            '\u{ffff}',
            '\u{1f600}',
            '\u{10ffff}',
        ] {
            for prefix in ["http://", "mailto:", "www."] {
                let target = if prefix == "mailto:" {
                    "a@example.com"
                } else {
                    "example.com"
                };
                let link = format!("{prefix}{target}");
                check(&format!("{c}{link}"), Some(&link));
            }
        }
        for local in ["\u{e9}".repeat(32), "\u{1f600}".repeat(16)] {
            let link = format!("{local}@example.com");
            check(&format!("\u{1f600} ({link})"), Some(&link));
            check(&format!("\u{1f600}{link}"), None);
        }
        let label = "a".repeat(63);
        let prefix = format!("{label}.{label}.{label}.");
        let host = format!("{prefix}{}a.com", "\u{e9}".repeat(29));
        assert_eq!(host.len(), MAX_DOMAIN);
        let link = format!("http://{host}");
        check(&link, Some(&link));
        // The domain limit, rather than the label limit, splits this scalar.
        let host = format!("{prefix}{}.a\u{1f600}", "a".repeat(59));
        assert_eq!(host.len(), MAX_DOMAIN + 2);
        check(&format!("http://{host}"), None);
    }

    #[test]
    fn ipv4_literal_boundaries() {
        for (literal, valid) in [
            ("1.2.3.4", true),
            ("0.0.0.0", true),
            ("255.255.255.255", true),
            ("001.002.003.004", true),
            ("256.1.2.3", false),
            ("1.2.3", false),
            ("1.2.3.4.5", false),
            ("1..2.3", false),
            ("1.2.3.", false),
            ("1.2.3.4444", false),
            ("0x1.2.3.4", false),
            ("1.2.3.-4", false),
        ] {
            for (scheme, prefix) in [("", "mailto:"), ("mailto:", "")] {
                let address = format!("{scheme}a@[{literal}]");
                let expected = valid.then_some(address.as_str());
                assert_match(&address, expected, prefix);
                assert_match(&format!("See {address}, then."), expected, prefix);
            }
        }
    }

    #[test]
    fn ipv6_literal_boundaries_distinguish_smtp_and_uri_grammar() {
        for (literal, smtp, uri) in [
            ("::", true, true),
            ("::1", true, true),
            ("1::", true, true),
            ("2001:db8::1", true, true),
            ("1:2:3:4:5:6:7:8", true, true),
            ("FE80:0000:0000:0000:0202:B3FF:FE1E:8329", true, true),
            ("1:2:3:4:5:6::", true, true),
            ("1:2:3:4:5:6::8", false, true),
            ("::2:3:4:5:6:7:8", false, true),
            ("1:2:3:4:5:6:7::", false, true),
            ("::192.0.2.1", true, true),
            ("::ffff:192.0.2.1", true, true),
            ("1:2:3:4:5:6:192.0.2.1", true, true),
            ("1:2:3:4::192.0.2.1", true, true),
            ("1:2:3:4:5::192.0.2.1", false, true),
            ("::ffff:192.000.002.1", true, false),
            ("", false, false),
            (":", false, false),
            ("1:2:3:4:5:6:7:", false, false),
            (":1:2:3:4:5:6:7", false, false),
            ("1:2:3:4:5:6:7", false, false),
            ("1:2:3:4:5:6:7:8:9", false, false),
            ("1:2:3:4:5:6:7:8::", false, false),
            ("1::2::3", false, false),
            (":::1", false, false),
            ("1:::", false, false),
            ("fffff::1", false, false),
            ("::g", false, false),
            ("192.0.2.1", false, false),
            ("::ffff:192.0.2", false, false),
            ("::ffff:192.0.2.", false, false),
            ("::ffff:192.0.2.256", false, false),
            ("::ffff:192.0.2.1:2", false, false),
            ("::ffff:192.0.2.1.2", false, false),
            ("::ffff:192.0.2.1234", false, false),
            ("::ffff:0xc0.0.2.1", false, false),
            ("1:2:3:4:5:6:7:192.0.2.1", false, false),
            ("fe80::1%25eth0", false, false),
        ] {
            for (scheme, prefix) in [("", "mailto:"), ("mailto:", "")] {
                let address = format!("{scheme}a@[IPv6:{literal}]");
                let expected = smtp.then_some(address.as_str());
                assert_match(&address, expected, prefix);
                assert_match(&format!("See {address}, then."), expected, prefix);
            }
            let url = format!("http://[{literal}]/");
            assert_match(&url, uri.then_some(url.as_str()), "");
        }
    }

    #[test]
    fn truncated_literals_never_read_beyond_the_slice() {
        for input in [
            "a@[1.2.3.4]",
            "mailto:a@[1.2.3.4]",
            "a@[IPv6:::]",
            "mailto:a@[IPv6:::1]",
            "http://[::1]",
        ] {
            for length in 0..input.len() {
                assert_match(&input[..length], None, "");
            }
        }
    }

    #[test]
    fn balanced_enclosed_url_suffixes() {
        for (open, close) in [("(", ")"), ("[", "]"), ("{", "}"), ("<", ">"), ("|", "|")] {
            for prefix in [
                "http://example.com/",
                "file:///path/",
                "mailto:a@example.com?subject=",
                "mailto:?subject=",
            ] {
                let url = format!("{prefix}a(b(c))[d[e]]{{f{{g}}}}");
                assert_match(&format!("{open}{url}{close})., next"), Some(&url), "");
            }
        }
    }

    #[test]
    fn query_fragment_and_prose_boundaries() {
        for (input, expected, prefix) in [
            (
                "(http://example.com/a(bar))).",
                "http://example.com/a(bar)",
                "",
            ),
            (
                "[http://example.com/a([b]{c})].",
                "http://example.com/a([b]{c})",
                "",
            ),
            (
                "{http://example.com/a({b}[c])}.",
                "http://example.com/a({b}[c])",
                "",
            ),
            (
                "http://example.com#section.",
                "http://example.com#section",
                "",
            ),
            (
                "http://example.com:00099999#section",
                "http://example.com:00099999#section",
                "",
            ),
            (
                "http://example.com/path#section",
                "http://example.com/path#section",
                "",
            ),
            (
                "http://example.com?x=1#section",
                "http://example.com?x=1#section",
                "",
            ),
            (
                "(www.example.com#part_(one)).",
                "www.example.com#part_(one)",
                "http://",
            ),
            (
                "ftp.example.com#section",
                "ftp.example.com#section",
                "ftp://",
            ),
            (
                "http://example.com#\u{440}\u{430}\u{437}\u{434}\u{435}\u{43b}",
                "http://example.com#\u{440}\u{430}\u{437}\u{434}\u{435}\u{43b}",
                "",
            ),
            ("http://example.com#", "http://example.com#", ""),
            ("(http://example.com#).", "http://example.com#", ""),
            ("http://example.com?x=#", "http://example.com?x=#", ""),
            ("http://example.com?_=1", "http://example.com?_=1", ""),
            ("http://example.com?%78=1", "http://example.com?%78=1", ""),
            (
                "http://example.com?x=1&_=2",
                "http://example.com?x=1&_=2",
                "",
            ),
            ("http://example.com?-x=1", "http://example.com?-x=1", ""),
            (
                "http://example.com?=value&x=&_=2#frag",
                "http://example.com?=value&x=&_=2#frag",
                "",
            ),
            (
                "http://example.com?&x=1&&y=",
                "http://example.com?&x=1&&y=",
                "",
            ),
            (
                "http://example.com?x=a?b&y=/c?d",
                "http://example.com?x=a?b&y=/c?d",
                "",
            ),
            ("http://example.com??x=1", "http://example.com??x=1", ""),
            ("http://example.com?!", "http://example.com", ""),
            ("http://example.com?x=1?!", "http://example.com?x=1", ""),
            (
                "http://example.com?_=1\u{a0}next",
                "http://example.com?_=1",
                "",
            ),
            (
                "http://example.com?_=1\u{202e}next",
                "http://example.com?_=1",
                "",
            ),
            (
                "http://example.com#section\u{200b}next",
                "http://example.com#section",
                "",
            ),
            (
                "http://example.com#section\0next",
                "http://example.com#section",
                "",
            ),
            (
                "http://example.com#section\u{2028}next",
                "http://example.com#section",
                "",
            ),
        ] {
            assert_match(input, Some(expected), prefix);
        }
    }

    #[test]
    fn validated_idna_top_level_domains() {
        for (domain, valid) in [
            ("example.xn--p1ai", true),
            ("EXAMPLE.XN--P1AI", true),
            ("example.\u{440}\u{444}", true),
            ("example.xn--fiqs8s", true),
            ("example.\u{4e2d}\u{56fd}", true),
            ("example.xn--", false),
            ("example.xn--a", false),
            ("example.xn---a", false),
            ("example.xn--0", false),
            ("example.xn--abc-", false),
            ("example.xn--p1ai-", false),
            ("xn--p1ai", false),
            ("example", false),
            ("example.123", false),
            ("example.c", false),
            ("example.c0m", false),
            ("example.a-b", false),
        ] {
            let address = format!("user@{domain}");
            assert_match(&address, valid.then_some(address.as_str()), "mailto:");
            // An explicit mailto does not require the bare-address TLD heuristic.
            let url = format!("mailto:{address}");
            assert_match(&url, Some(&url), "");
        }
    }

    #[test]
    fn web_authority_boundaries() {
        for (input, expected) in [
            ("http://[::]", Some("http://[::]")),
            (
                "HTTP://[2001:DB8::1]:00099999#f",
                Some("HTTP://[2001:DB8::1]:00099999#f"),
            ),
            ("[http://[::1]/a[0]]", Some("http://[::1]/a[0]")),
            (
                "http://user@example.com:080/path?_=1#f",
                Some("http://user@example.com:080/path?_=1#f"),
            ),
            (
                "http://demo:example@[::1]:080/path?x=1&_=2#f",
                Some("http://demo:example@[::1]:080/path?x=1&_=2#f"),
            ),
            (
                "http://demo%40mail:ex%3Aample@example.com/",
                Some("http://demo%40mail:ex%3Aample@example.com/"),
            ),
            (
                "http://demo:ex!$&'()*+,;=ample@example.com/",
                Some("http://demo:ex!$&'()*+,;=ample@example.com/"),
            ),
            (
                "(http://demo:ex(a)mple@example.com/)",
                Some("http://demo:ex(a)mple@example.com/"),
            ),
            ("http://@example.com/", Some("http://@example.com/")),
            (
                "http://:example@example.com/",
                Some("http://:example@example.com/"),
            ),
            (
                "http://us\u{e9}r@b\u{fc}cher.example/",
                Some("http://us\u{e9}r@b\u{fc}cher.example/"),
            ),
            ("http://user%xx@example.com/", Some("http://user")),
            ("http://user%2@example.com/", Some("http://user")),
            ("http://user%40", Some("http://user")),
            ("http://user\u{202e}@example.com/", Some("http://user")),
            (
                "http://example.com/path@elsewhere.test",
                Some("http://example.com/path@elsewhere.test"),
            ),
            (
                "http://example.com?x=user@elsewhere.test",
                Some("http://example.com?x=user@elsewhere.test"),
            ),
            (
                "http://example.com#user@elsewhere.test",
                Some("http://example.com#user@elsewhere.test"),
            ),
            ("http://user@", None),
            ("http://user@/", None),
            ("http://user@-host/", None),
            ("http://user@.example/", None),
            ("http://user@@example.com/", None),
            ("http://[::1", None),
            ("http://user@[::1", None),
            ("http://[]/", None),
            ("http://[IPv6:::1]/", None),
            ("http://[127.0.0.1]/", None),
            ("http://[v1.future]/", None),
            ("http://[fe80::1%25eth0]/", None),
        ] {
            assert_match(input, expected, "");
        }
        // Userinfo lookahead is intentionally limited to "://" patterns.
        assert_match("sip:user@example.com", Some("sip:user"), "");
        assert_match("www.user@example.com", Some("www.user"), "http://");
    }

    #[test]
    fn new_components_preserve_utf8_length_limits() {
        let unicode = "\u{e9}".repeat(57);
        let alabel = idna::to_ascii(&unicode).unwrap();
        assert_eq!(alabel.len(), MAX_LABEL);
        let address = format!("user@example.{alabel}");
        assert_match(&address, Some(&address), "mailto:");
        assert_match(&format!("{address}a"), None, "");
        // Unlike the C# UTF-16 count, the Rust scanner counts UTF-8 bytes.
        assert_match(&format!("user@example.{unicode}"), None, "");
        let address = format!("user@example.{}", "\u{e9}".repeat(31));
        assert_match(&address, Some(&address), "mailto:");
        let label = "a".repeat(64);
        assert_match(&format!("user@example.xn--{label}"), None, "");
        assert_match(&format!("http://user@{label}.com"), None, "");
        let label = "a".repeat(63);
        let host = format!("{label}.{label}.{label}.{}.com", "a".repeat(59));
        assert_eq!(host.len(), MAX_DOMAIN);
        let url = format!("http://{}@{host}:00099999#f", "u".repeat(10_000));
        assert_match(&url, Some(&url), "");
        assert_match(&format!("http://user@{host}a"), None, "");
    }

    #[test]
    fn adversarial_candidates_do_not_hide_a_later_link() {
        use std::time::{Duration, Instant};

        fn time_scan(generate: fn(usize) -> String, n: usize) -> Duration {
            let input = format!("{} http://example.com", generate(n));
            let mut remaining = input.as_str();
            let mut last = None;
            let start = Instant::now();
            while let Some(found) = scan(remaining) {
                assert!(found.end > found.start);
                last = Some(&remaining[found.start..found.end]);
                remaining = &remaining[found.end..];
            }
            assert_eq!(last, Some("http://example.com"));
            start.elapsed()
        }

        let cases: &[fn(usize) -> String] = &[
            |n| format!("\"{}", "\\\"@".repeat(n)),
            |n| "a@-".repeat(n),
            |n| format!("{}{}", "a".repeat(n), "@-".repeat(n)),
            |n| "mailto:\"x\\\"".repeat(n),
            |n| format!("mailto:{}{}", "a".repeat(n), "mailto:".repeat(n)),
            |n| format!("{}{}", "a.".repeat(n), "@-".repeat(n)),
            |n| "www.".repeat(n),
            |n| "http:// ".repeat(n),
            |n| format!("(http://example.com/{}{})", "(".repeat(n), ")".repeat(n)),
            |n| format!("(http://example.com/{}", "(".repeat(n)),
            |n| format!("http://example.com/{}", ")".repeat(n)),
            |n| "http://[1:2:3:4:5:6:7:]/ ".repeat(n),
            |n| "http://demo:example@- ".repeat(n),
            |n| format!("http://{}@", "user:".repeat(n)),
            |n| format!("http://{}", "www.".repeat(n)),
            |n| format!("http://{}", "sip:".repeat(n)),
            |n| "sip:".repeat(n),
            |n| "a@[1111.2.3.4] ".repeat(n),
            |n| "a@[IPv6:fffff::1] ".repeat(n),
        ];
        for (case, &generate) in cases.iter().enumerate() {
            let elapsed = time_scan(generate, 100_000);
            if elapsed < Duration::from_secs(10) {
                continue;
            }
            // As upstream, allow slow/instrumented builds to use scaling:
            // 4x input should cost roughly 4x time, not the 16x of quadratic work.
            let quarter = time_scan(generate, 25_000);
            let ratio = elapsed.as_secs_f64() / quarter.as_secs_f64().max(0.001);
            assert!(
                ratio < 8.0,
                "case {case}: {elapsed:?} vs {quarter:?} (ratio {ratio})"
            );
        }
    }
}
