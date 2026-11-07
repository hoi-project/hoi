//! Text that people read (PROTOCOL.md §18): names, titles, posts, comments, reasons.
//!
//! Records are immutable and signed, so nothing is cleaned up on arrival: a record either
//! passes or it is refused, on the device that writes it and on every device that receives it.
//! The rules refuse what has no business in a forum and a long history of abuse:
//!
//! - control characters, direction overrides ("Trojan Source"), invisible and zero-width
//!   characters, private-use and non-characters, hidden tag and selector runs that smuggle
//!   text past readers (and past language models summarising it), and stacked accents;
//! - raw HTML, embedded media, and link targets that are not plain `http(s)` addresses;
//! - addresses that hide where they lead: a username before the host, non-ASCII hosts that
//!   are not written in punycode, local and private network addresses, numeric tricks, and
//!   link text naming a different site than the link goes to.

use std::net::{Ipv4Addr, Ipv6Addr};

/// What kind of text a field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// A self-chosen display name: one line, plain spaces only, no direction marks.
    Name,
    /// A title, label or reason: one line, plain spaces only.
    Line,
    /// Markdown body text: several lines.
    Body,
}

pub const MAX_LINKS: usize = 8;

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fn is_direction_mark(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}')
}

fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{034F}'
            | '\u{115F}'
            | '\u{1160}'
            | '\u{17B4}'
            | '\u{17B5}'
            | '\u{180E}'
            | '\u{200B}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{2060}'..='\u{2064}'
            | '\u{206A}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{3164}'
            | '\u{FFA0}'
            | '\u{1D159}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0001}'
    )
}

fn is_private_or_nonchar(c: char) -> bool {
    let u = c as u32;
    matches!(u, 0xE000..=0xF8FF | 0xF0000..=0x10FFFF | 0xFDD0..=0xFDEF) || (u & 0xFFFE) == 0xFFFE
}

fn is_variation_selector(c: char) -> bool {
    matches!(c, '\u{FE00}'..='\u{FE0F}')
}

fn is_combining(c: char) -> bool {
    matches!(c, '\u{0300}'..='\u{036F}' | '\u{1AB0}'..='\u{1AFF}' | '\u{1DC0}'..='\u{1DFF}' | '\u{20D0}'..='\u{20FF}' | '\u{FE20}'..='\u{FE2F}')
}

fn is_joiner(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}')
}

/// Checks every character of `s`. `what` names the field in the error.
pub fn chars(s: &str, field: Field, what: &str) -> Result<(), String> {
    let multiline = field == Field::Body;
    let mut combining_run = 0;
    let mut prev_selector = false;
    let mut prev_joiner = false;
    // Inside a 🏴 tag sequence (subdivision flags): tag characters are allowed there only.
    let mut tags: Option<usize> = None;
    for c in s.chars() {
        if c.is_control() && !(multiline && (c == '\n' || c == '\t')) {
            return Err(format!("{what} has control characters"));
        }
        if is_bidi_control(c) {
            return Err(format!("{what} has direction overrides"));
        }
        if field != Field::Body && is_direction_mark(c) {
            return Err(format!("{what} has direction marks"));
        }
        if is_invisible(c) || (field != Field::Body && c == '\u{00AD}') {
            return Err(format!("{what} has invisible characters"));
        }
        if field != Field::Body && c.is_whitespace() && c != ' ' {
            return Err(format!("{what} has unusual spaces"));
        }
        if matches!(c, '\u{E0100}'..='\u{E01EF}') {
            return Err(format!("{what} has hidden selector characters"));
        }
        if is_private_or_nonchar(c) {
            return Err(format!("{what} has private-use characters"));
        }
        match c {
            '\u{1F3F4}' => tags = Some(0),
            '\u{E0020}'..='\u{E007E}' => match tags {
                Some(n) if n < 8 => tags = Some(n + 1),
                _ => return Err(format!("{what} has hidden tag characters")),
            },
            '\u{E007F}' => match tags {
                Some(n) if n > 0 => tags = None,
                _ => return Err(format!("{what} has hidden tag characters")),
            },
            _ => tags = None,
        }
        let sel = is_variation_selector(c);
        if sel && prev_selector {
            return Err(format!("{what} has stacked selector characters"));
        }
        prev_selector = sel;
        let j = is_joiner(c);
        if j && prev_joiner {
            return Err(format!("{what} has stacked joiners"));
        }
        prev_joiner = j;
        if is_combining(c) {
            combining_run += 1;
            if combining_run > 4 {
                return Err(format!("{what} has stacked accents"));
            }
        } else {
            combining_run = 0;
        }
    }
    Ok(())
}

/// One line of plain text: characters, length, not blank when `required`.
pub fn line(s: &str, what: &str, max_chars: usize, required: bool) -> Result<(), String> {
    if s.chars().count() > max_chars {
        return Err(format!("{what} is too long"));
    }
    if required && s.trim().is_empty() {
        return Err(format!("{what} is empty"));
    }
    chars(s, Field::Line, what)
}

/// A self-chosen display name (≤ 32 characters). Anyone can claim any name, so names may not
/// dress up as keys either.
pub fn name(s: &str) -> Result<(), String> {
    if s.chars().count() > 32 {
        return Err("name is too long".into());
    }
    if s.trim().is_empty() {
        return Err("name is empty".into());
    }
    if s.trim() != s {
        return Err("name starts or ends with a space".into());
    }
    if s.contains("  ") {
        return Err("name has runs of spaces".into());
    }
    if s.contains('·') || s.to_ascii_lowercase().starts_with("did:") {
        return Err("name looks like a key".into());
    }
    chars(s, Field::Name, "name")
}

// ------------------------------------------------------------------ addresses

/// The host of a URL that already passed [`url`], lowercase, without `www.`.
pub fn host_of(u: &str) -> Option<String> {
    let rest = u.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = if let Some(v6) = authority.strip_prefix('[') { v6.split(']').next()? } else { authority.rsplit_once(':').map_or(authority, |(h, _)| h) };
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    Some(h.strip_prefix("www.").map(String::from).unwrap_or(h))
}

fn v4_is_local(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast() || ip.octets()[0] == 0 || ip.octets()[0] >= 224
        // 100.64.0.0/10, carrier-grade NAT
        || (ip.octets()[0] == 100 && (ip.octets()[1] & 0xC0) == 64)
}

fn v6_is_local(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    ip.is_loopback() || ip.is_unspecified() || (s[0] & 0xFE00) == 0xFC00 || (s[0] & 0xFFC0) == 0xFE80 || ip.to_ipv4_mapped().is_some_and(v4_is_local)
}

/// An `http(s)` address that doesn't hide where it leads (PROTOCOL.md §18.2).
pub fn url(s: &str) -> Result<(), String> {
    if s.len() > 2048 {
        return Err("address is too long".into());
    }
    if s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("address has spaces or control characters".into());
    }
    let lower = s.to_ascii_lowercase();
    let rest = if lower.starts_with("https://") || lower.starts_with("http://") {
        &s[s.find("://").map_or(0, |i| i + 3)..]
    } else {
        return Err("address must start with http:// or https://".into());
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return Err("address hides its site behind a username".into());
    }
    if authority.contains('\\') {
        return Err("address has a backslash".into());
    }
    let host = if let Some(v6) = authority.strip_prefix('[') {
        let (h, after) = v6.split_once(']').ok_or("address has a broken IPv6 host")?;
        if !(after.is_empty() || after.strip_prefix(':').is_some_and(|p| p.parse::<u16>().is_ok_and(|n| n > 0))) {
            return Err("address has a bad port".into());
        }
        let ip: Ipv6Addr = h.parse().map_err(|_| "address has a broken IPv6 host")?;
        if v6_is_local(ip) {
            return Err("address points into a private network".into());
        }
        return Ok(());
    } else {
        match authority.rsplit_once(':') {
            Some((h, port)) => {
                if !port.parse::<u16>().is_ok_and(|n| n > 0) {
                    return Err("address has a bad port".into());
                }
                h
            }
            None => authority,
        }
    };
    if host.is_empty() {
        return Err("address has no site".into());
    }
    if !host.is_ascii() {
        return Err("international site names must be written in punycode (xn--)".into());
    }
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.len() > 253 || !host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.') {
        return Err("address has an unusual site name".into());
    }
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        if v4_is_local(ip) {
            return Err("address points into a private network".into());
        }
        return Ok(());
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.iter().any(|l| l.is_empty() || l.len() > 63 || l.starts_with('-') || l.ends_with('-')) {
        return Err("address has an unusual site name".into());
    }
    let tld = labels.last().copied().unwrap_or("");
    // Top-level domains are never numeric: this catches 2130706433, 0x7f.1 and friends.
    if !tld.bytes().any(|b| b.is_ascii_alphabetic()) || tld.starts_with("0x") || labels.len() < 2 && host != "localhost" {
        return Err("address has an unusual site name".into());
    }
    if host == "localhost" || ["localhost", "local", "internal", "home", "lan", "localdomain", "home.arpa"].iter().any(|s| host.ends_with(&format!(".{s}"))) {
        return Err("address points into a private network".into());
    }
    Ok(())
}

/// Whether link text reads like a site name of its own ("paypal.com", "https://x.y/z").
fn looks_like_site(text: &str) -> Option<String> {
    let t = text.trim();
    if t.len() < 4 || t.contains(char::is_whitespace) || !t.contains('.') {
        return None;
    }
    let with_scheme =
        if t.to_ascii_lowercase().starts_with("http://") || t.to_ascii_lowercase().starts_with("https://") { t.to_string() } else { format!("https://{t}") };
    let h = host_of(&with_scheme)?;
    let tld = h.rsplit('.').next()?;
    (tld.len() >= 2 && tld.bytes().all(|b| b.is_ascii_alphabetic())).then_some(h)
}

fn same_site(a: &str, b: &str) -> bool {
    a == b || a.ends_with(&format!(".{b}")) || b.ends_with(&format!(".{a}"))
}

/// Markdown link targets in `s`: `[text](target)` and `[ref]: target` lines. Returns
/// `(text, target)`; reference definitions have no text.
fn link_targets(s: &str) -> Vec<(Option<&str>, &str)> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while let Some(off) = s[i..].find("](") {
        let close = i + off;
        let start = close + 2;
        let end = s[start..].find(|c: char| c == ')' || c.is_whitespace()).map_or(s.len(), |e| start + e);
        // the matching '[' on the same line
        let line_start = s[..close].rfind('\n').map_or(0, |n| n + 1);
        let text = s[line_start..close].rfind('[').map(|o| &s[line_start + o + 1..close]);
        out.push((text, &s[start..end]));
        i = start.max(close + 1);
        if i >= b.len() {
            break;
        }
    }
    for l in s.lines() {
        let t = l.trim_start_matches(' ');
        if l.len() - t.len() <= 3
            && t.starts_with('[')
            && let Some(rb) = t.find("]:")
        {
            let target = t[rb + 2..].split_whitespace().next().unwrap_or("");
            out.push((None, target));
        }
    }
    out
}

/// Bare `http(s)://` addresses in `s`, as renderers would auto-link them.
fn bare_urls(s: &str) -> Vec<&str> {
    let lower = s.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = lower[i..].find("http") {
        let at = i + off;
        let rest = &lower[at..];
        if rest.starts_with("http://") || rest.starts_with("https://") {
            let end = s[at..].find(|c: char| c.is_whitespace() || matches!(c, ')' | '>' | '"' | '\'' | '<' | ']')).map_or(s.len(), |e| at + e);
            out.push(s[at..end].trim_end_matches(['.', ',', ';', ':', '!', '?']));
            i = end.max(at + 1);
        } else {
            i = at + 4;
        }
        if i >= s.len() {
            break;
        }
    }
    out
}

/// Whether `s` carries any link a renderer would follow (newcomers share text only).
pub fn has_links(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("www.")
        || link_targets(s).iter().any(|(_, t)| !t.is_empty() && !t.starts_with('#'))
}

/// Markdown body text (PROTOCOL.md §18.1).
pub fn markdown(s: &str, what: &str, max_chars: usize, max_lines: usize) -> Result<(), String> {
    if s.chars().count() > max_chars {
        return Err(format!("{what} is too long"));
    }
    if s.matches('\n').count() > max_lines {
        return Err(format!("{what} has too many lines"));
    }
    chars(s, Field::Body, what)?;
    let b = s.as_bytes();
    for i in 0..b.len().saturating_sub(1) {
        if b[i] == b'<' && (b[i + 1].is_ascii_alphabetic() || matches!(b[i + 1], b'/' | b'!' | b'?')) {
            return Err(format!("{what} contains raw HTML"));
        }
    }
    if s.contains("![") {
        return Err(format!("{what} embeds media"));
    }
    let bare = bare_urls(s);
    if bare.len() > MAX_LINKS {
        return Err(format!("{what} has more than {MAX_LINKS} links"));
    }
    for u in bare {
        url(u).map_err(|e| format!("{what}: {e}"))?;
    }
    for (text, target) in link_targets(s) {
        let target = target.trim_matches(['<', '>']);
        if target.is_empty() || target.starts_with('#') {
            continue;
        }
        url(target).map_err(|e| format!("{what}: link {e}"))?;
        if let (Some(t), Some(h)) = (text.and_then(looks_like_site), host_of(target))
            && !same_site(&t, &h)
        {
            return Err(format!("{what}: link text names a different site than it leads to"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn smuggling_is_refused() {
        let body = |s: &str| chars(s, Field::Body, "text");
        assert!(body("plain words, emoji 👩‍💻 ❤️ and 🏴󠁧󠁢󠁳󠁣󠁴󠁿").is_ok());
        assert!(body("right-to-left: שלום עולם, ZWNJ in می‌خواهم").is_ok());
        assert!(body("evil\u{202E}txt.exe").is_err(), "direction override");
        assert!(body("zero\u{200B}width").is_err());
        assert!(body("a\u{E0041}\u{E0042}").is_err(), "tag smuggling outside a flag");
        assert!(body("x\u{E0100}\u{E0101}").is_err(), "selector smuggling");
        assert!(body("x\u{FE0F}\u{FE0F}").is_err(), "stacked selectors");
        assert!(body("z\u{0301}\u{0302}\u{0303}\u{0304}\u{0305}").is_err(), "zalgo");
        assert!(body("é\u{0301}").is_ok());
        assert!(body("pua \u{E000}").is_err());
        assert!(body("bell \u{7}").is_err());
        assert!(body("two\nlines\tand a tab").is_ok());
        assert!(chars("two\nlines", Field::Line, "title").is_err());
        assert!(chars("nbsp\u{00A0}here", Field::Line, "title").is_err());
        assert!(chars("mark\u{200F}", Field::Name, "name").is_err());
    }

    #[test]
    fn names() {
        assert!(name("lena").is_ok());
        assert!(name("Ana María").is_ok());
        assert!(name(" lena").is_err());
        assert!(name("a7f2·91c0").is_err());
        assert!(name("did:key:z6Mk").is_err());
        assert!(name("le  na").is_err());
        assert!(name(&"x".repeat(33)).is_err());
    }

    #[test]
    fn addresses() {
        for ok in
            ["https://hoi.sh", "https://xn--bcher-kva.example/path?q=1#x", "http://example.org:8080/a", "https://[2001:db8::1]/", "https://93.184.216.34/"]
        {
            assert!(url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "javascript:alert(1)",
            "data:text/html,hi",
            "https://bank.example@evil.example/",
            "https://bücher.example/",
            "http://localhost:8080/",
            "http://192.168.1.1/admin",
            "http://10.0.0.1",
            "http://2130706433/",
            "http://0x7f.0x1/",
            "https://[::1]/",
            "https://router.local/",
            "https://exa mple.com",
            "https://example.com:0/",
            "https://example",
        ] {
            assert!(url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn markdown_links() {
        let md = |s: &str| markdown(s, "text", 40_000, 2000);
        assert!(md("see [the docs](https://hoi.sh/protocol) and https://example.org.").is_ok());
        assert!(md("[click](javascript:alert(1))").is_err());
        assert!(md("[click](JAVASCRIPT:alert(1))").is_err());
        assert!(md("[ref]: javascript:alert(1)\n\n[x][ref]").is_err());
        assert!(md("[paypal.com](https://evil.example/login)").is_err(), "deceptive link text");
        assert!(md("[https://hoi.sh](https://www.hoi.sh/)").is_ok());
        assert!(md("[read more](https://news.example/a)").is_ok());
        assert!(md("[jump](#section)").is_ok());
        assert!(md("<script>alert(1)</script>").is_err());
        assert!(md("2 < 3 and 5 > 4").is_ok());
        assert!(md("![cat](https://x.example/cat.png)").is_err());
        assert!(md("visit https://user@evil.example now").is_err());
        assert!(md(&"https://a.example ".repeat(9)).is_err());
        assert!(has_links("go to www.example.com") && has_links("[x](https://a.example)") && !has_links("just words. no links"));
    }

    proptest! {
        #[test]
        fn never_panics(s in ".{0,200}") {
            let _ = markdown(&s, "text", 40_000, 2000);
            let _ = name(&s);
            let _ = url(&s);
            let _ = has_links(&s);
        }
    }
}
