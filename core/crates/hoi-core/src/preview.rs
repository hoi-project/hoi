//! Link previews (M3): a post's link shown with the linked page's title, description
//! and picture, when the person turned previews on. Only the native side fetches: a desktop app
//! or hoi-node fetches for itself and for the browsers paired with it (`pv` / `pvd` over the
//! sealed home link); a browser with no home node fetches nothing. The core decides what to
//! fetch and reads what came; the host does the network part under the caps below
//! (`hoi_net::preview`): https only, no cookies, addresses on the internet only, at most
//! [`PREVIEW_BYTES`] and [`PREVIEW_SECS`]. The picture goes through the same sanitizer as a
//! post's. A preview is kept on this device and never published.

use std::collections::{BTreeMap, BTreeSet};

use hoi_types::Id;
use hoi_types::cbor::Value;
use serde::{Deserialize, Serialize};

use crate::api::R;
use crate::dto::{Event, LinkPreview};
use crate::error::ApiError;
use crate::node::Node;
use hoi_store::Change;

/// The most a host reads of a page or a picture.
pub const PREVIEW_BYTES: usize = 512 * 1024;
/// The longest a host waits for a page or a picture, redirects included.
pub const PREVIEW_SECS: u64 = 5;
/// Redirects a host follows, each checked like the first address.
pub const PREVIEW_REDIRECTS: usize = 3;
/// A picture larger than this after sanitizing is left out (it must fit a `link` message).
const IMAGE_BYTES: usize = 384 * 1024;
/// Previews kept on this device.
const KEEP: usize = 64;
/// A preview is fetched again after a week; a failed one after an hour.
const FRESH: u64 = 7 * 86_400;
const RETRY: u64 = 3600;
/// A fetch that never came back is given up after this.
const INFLIGHT: u64 = 60;
/// Previews a browser asks its home node for in a minute (link messages are 12 a minute).
const ASKS_PER_MINUTE: u32 = 8;
/// Previews one paired device may ask its home node for in a minute.
const SERVE_PER_MINUTE: u32 = 12;
const TITLE: usize = 160;
const DESCRIPTION: usize = 300;
pub(crate) const BLOB: &str = "previews";

pub(crate) fn image_key(h: &Id) -> String {
    format!("pv:{}", h.hex())
}

/// What a host fetches for the core: a page, or the picture a page named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PreviewFetch {
    pub url: String,
    pub image: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Kept {
    pub title: Option<String>,
    pub description: Option<String>,
    pub site: String,
    pub image: Option<Id>,
    pub at: u64,
    pub failed: bool,
}

#[derive(Default)]
pub(crate) struct Previews {
    pub kept: BTreeMap<String, Kept>,
    pub images: BTreeMap<Id, Vec<u8>>,
    wanted: Vec<PreviewFetch>,
    /// url → (since, what the page said while its picture is on the way)
    inflight: BTreeMap<String, (u64, Option<Kept>)>,
    /// A picture on the way → the page it is for.
    pictures: BTreeMap<String, String>,
    /// url → the paired devices waiting for it: (peer, link secret)
    waiting: BTreeMap<String, BTreeSet<(String, Vec<u8>)>>,
    /// A browser: url → when it asked its home node.
    asked: BTreeMap<String, u64>,
    asks: (u64, u32),
    served: BTreeMap<String, (u64, u32)>,
}

/// What a page says about itself.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub title: Option<String>,
    pub description: Option<String>,
    pub site: Option<String>,
    pub image: Option<String>,
}

/// The `https://host` part of a URL, and its host, or `None` for anything but https.
fn origin(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("https://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = &rest[..end];
    if host.is_empty() || host.contains('@') || host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    Some((&url[..8 + end], host))
}

/// A link the core will ask a host to fetch: https, no credentials, a host, nothing odd.
pub fn fetchable(url: &str) -> bool {
    url.len() <= 2048 && !url.chars().any(|c| c.is_whitespace() || c.is_control() || c == '\\') && origin(url).is_some()
}

/// `image` (as a page wrote it) against the page's own address; https only.
pub fn resolve(page: &str, image: &str) -> Option<String> {
    let image = image.trim();
    let (base, _) = origin(page)?;
    let out = if image.starts_with("https://") {
        image.to_string()
    } else if let Some(rest) = image.strip_prefix("//") {
        format!("https://{rest}")
    } else if image.starts_with('/') {
        format!("{base}{image}")
    } else if image.contains("://") || image.starts_with("data:") || image.is_empty() {
        return None;
    } else {
        let path = &page[base.len()..];
        let path = &path[..path.find(['?', '#']).unwrap_or(path.len())];
        let dir = &path[..path.rfind('/').map_or(0, |i| i + 1)];
        let dir = if dir.is_empty() { "/" } else { dir };
        format!("{base}{dir}{image}")
    };
    fetchable(&out).then_some(out)
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let c = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32),
            n if n.starts_with('#') => n[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Text as a preview shows it: entities decoded, one line, no control or direction marks, short.
fn clean(s: &str, max: usize) -> Option<String> {
    let text: String = decode_entities(s)
        .chars()
        .filter(|c| !c.is_control() || c.is_whitespace())
        .filter(|c| !matches!(*c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}'))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        return None;
    }
    Some(match text.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", text[..i].trim_end()),
        None => text,
    })
}

/// The attributes of one tag, names lowercased.
fn attributes(tag: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let b = tag.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
            i += 1;
        }
        if start == i {
            i += 1;
            continue;
        }
        let name = tag[start..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() || b[i] != b'=' {
            out.entry(name).or_default();
            continue;
        }
        i += 1;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let value = if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
            let q = b[i];
            let s = i + 1;
            let e = b[s..].iter().position(|x| *x == q).map_or(b.len(), |p| s + p);
            i = (e + 1).min(b.len());
            &tag[s..e.min(tag.len())]
        } else {
            let s = i;
            while i < b.len() && !b[i].is_ascii_whitespace() {
                i += 1;
            }
            &tag[s..i]
        };
        out.entry(name).or_insert_with(|| value.to_string());
    }
    out
}

/// Reads what a page says about itself: its Open Graph title, description, site name and
/// picture, else its `<title>` and description. Nothing else is read.
pub fn parse_page(html: &[u8]) -> Page {
    let html = String::from_utf8_lossy(&html[..html.len().min(PREVIEW_BYTES)]);
    let lower = html.to_ascii_lowercase();
    let mut meta: BTreeMap<String, String> = BTreeMap::new();
    let mut at = 0;
    while let Some(i) = lower[at..].find("<meta") {
        let start = at + i + 5;
        let Some(len) = lower[start..].find('>') else { break };
        let a = attributes(&html[start..start + len]);
        if let (Some(key), Some(content)) = (a.get("property").or_else(|| a.get("name")), a.get("content")) {
            meta.entry(key.to_ascii_lowercase()).or_insert_with(|| content.clone());
        }
        at = start + len;
    }
    let title_tag = lower.find("<title").and_then(|i| {
        let open = i + lower[i..].find('>')? + 1;
        let close = open + lower[open..].find("</title")?;
        Some(html[open..close].to_string())
    });
    let get = |keys: &[&str]| keys.iter().find_map(|k| meta.get(*k).cloned());
    Page {
        title: get(&["og:title", "twitter:title"]).or(title_tag).and_then(|t| clean(&t, TITLE)),
        description: get(&["og:description", "twitter:description", "description"]).and_then(|d| clean(&d, DESCRIPTION)),
        site: get(&["og:site_name"]).and_then(|s| clean(&s, 60)),
        image: get(&["og:image:secure_url", "og:image", "twitter:image"]),
    }
}

/// A picture for a preview: sanitized like a post's, a still image, small enough to pass on.
fn picture(bytes: &[u8]) -> Option<Vec<u8>> {
    let (clean, info) = hoi_validate::media::sanitize(bytes).ok()?;
    (info.mime.starts_with("image/") && clean.len() <= IMAGE_BYTES).then_some(clean)
}

fn data_url(bytes: &[u8]) -> String {
    let mime = hoi_validate::media::sniff(bytes).unwrap_or("image/png");
    let mut b64 = hoi_types::uri::b64(bytes).replace('-', "+").replace('_', "/");
    while !b64.len().is_multiple_of(4) {
        b64.push('=');
    }
    format!("data:{mime};base64,{b64}")
}

impl Node {
    /// The preview of a link, if this device has one; otherwise asks for it (an event says when
    /// it came) and answers `None`. Needs previews on, and a desktop app or hoi-node: this one,
    /// or for a browser its home node.
    pub fn link_preview(&mut self, url: &str) -> R<Option<LinkPreview>> {
        if !self.state.settings.link_previews {
            return Err(ApiError::not_allowed("Link previews are off. Turn them on in You → Privacy."));
        }
        if !fetchable(url) {
            return Ok(None);
        }
        let now = self.now();
        if let Some(k) = self.previews.kept.get(url) {
            let fresh = if k.failed { k.at + RETRY > now } else { k.at + FRESH > now };
            if fresh {
                return Ok((!k.failed).then(|| self.preview_view(url, k)));
            }
        }
        if self.can_relay() {
            self.want_preview(url, false, now);
        } else if self.peers.values().any(|p| p.home && p.said_hello) {
            self.ask_home_preview(url, now);
        } else {
            return Err(ApiError::not_allowed("Previews need a home node: a desktop app or hoi-node fetches them for this browser."));
        }
        Ok(None)
    }

    fn preview_view(&self, url: &str, k: &Kept) -> LinkPreview {
        LinkPreview {
            url: url.into(),
            title: k.title.clone(),
            description: k.description.clone(),
            site: k.site.clone(),
            image: k.image.and_then(|h| self.previews.images.get(&h)).map(|b| data_url(b)),
        }
    }

    fn want_preview(&mut self, url: &str, image: bool, now: u64) {
        let p = &mut self.previews;
        p.inflight.retain(|_, (at, _)| *at + INFLIGHT > now);
        let inflight = &p.inflight;
        p.pictures.retain(|_, page| inflight.contains_key(page));
        if !image && p.inflight.contains_key(url) {
            return;
        }
        if !image {
            p.inflight.insert(url.to_string(), (now, None));
        }
        p.wanted.push(PreviewFetch { url: url.to_string(), image });
    }

    fn ask_home_preview(&mut self, url: &str, now: u64) {
        let p = &mut self.previews;
        p.asked.retain(|_, at| *at + INFLIGHT > now);
        if p.asked.contains_key(url) {
            return;
        }
        if p.asks.0 != now / 60 {
            p.asks = (now / 60, 0);
        }
        if p.asks.1 >= ASKS_PER_MINUTE {
            return;
        }
        p.asks.1 += 1;
        p.asked.insert(url.to_string(), now);
        let plain = crate::link::body_of(vec![("t", Value::T("pv".into())), ("u", Value::T(url.into()))]);
        self.send_link_home(&plain);
    }

    /// For the host: what to fetch now (a desktop app or hoi-node).
    pub fn take_preview_fetches(&mut self) -> Vec<PreviewFetch> {
        std::mem::take(&mut self.previews.wanted)
    }

    /// For the host: what a fetch brought, or why it brought nothing. `url` is the page's or,
    /// with `image`, the picture's address as [`take_preview_fetches`](Self::take_preview_fetches) gave it.
    pub fn preview_fetched(&mut self, url: &str, image: bool, body: Result<Vec<u8>, String>) {
        let now = self.now();
        if !image {
            let Some(_) = self.previews.inflight.get(url) else { return };
            let Ok(html) = body else {
                self.previews.inflight.remove(url);
                return self.finish_preview(url, Kept { site: site_of(url), at: now, failed: true, ..Default::default() }, None);
            };
            let page = parse_page(&html);
            let kept =
                Kept { title: page.title, description: page.description, site: page.site.unwrap_or_else(|| site_of(url)), image: None, at: now, failed: false };
            match page.image.and_then(|i| resolve(url, &i)) {
                Some(img) => {
                    self.previews.inflight.insert(url.to_string(), (now, Some(kept)));
                    self.previews.pictures.insert(img.clone(), url.to_string());
                    self.want_preview(&img, true, now);
                }
                None => {
                    self.previews.inflight.remove(url);
                    self.finish_preview(url, kept, None);
                }
            }
            return;
        }
        // a picture: which page it was for
        let Some(page) = self.previews.pictures.remove(url) else { return };
        let Some((_, Some(kept))) = self.previews.inflight.remove(&page) else { return };
        let pic = body.ok().and_then(|b| picture(&b));
        self.finish_preview(&page, kept, pic);
    }

    fn finish_preview(&mut self, url: &str, mut kept: Kept, image: Option<Vec<u8>>) {
        if let Some(bytes) = image {
            let h = Id(*blake3::hash(&bytes).as_bytes());
            self.changes.push(Change::Blob { key: image_key(&h), value: Some(bytes.clone()) });
            self.previews.images.insert(h, bytes);
            kept.image = Some(h);
        }
        if let Some(old) = self.previews.kept.insert(url.to_string(), kept.clone()) {
            self.forget_preview_image(old.image);
        }
        while self.previews.kept.len() > KEEP {
            let Some(oldest) = self.previews.kept.iter().min_by_key(|(_, k)| k.at).map(|(u, _)| u.clone()) else { break };
            if let Some(k) = self.previews.kept.remove(&oldest) {
                self.forget_preview_image(k.image);
            }
        }
        self.save_previews();
        self.events.push(Event::Preview { url: url.to_string() });
        // the paired devices that asked for it
        let waiting = self.previews.waiting.remove(url).unwrap_or_default();
        self.send_preview(url, &kept, waiting);
    }

    fn send_preview(&mut self, url: &str, kept: &Kept, to: BTreeSet<(String, Vec<u8>)>) {
        if !to.is_empty() {
            let mut fields = vec![("t", Value::T("pvd".into())), ("u", Value::T(url.into())), ("s", Value::T(kept.site.clone()))];
            if kept.failed {
                fields.push(("x", Value::Bool(true)));
            }
            if let Some(t) = &kept.title {
                fields.push(("ti", Value::T(t.clone())));
            }
            if let Some(d) = &kept.description {
                fields.push(("d", Value::T(d.clone())));
            }
            if let Some(b) = kept.image.and_then(|h| self.previews.images.get(&h)) {
                fields.push(("i", Value::B(b.clone())));
            }
            let plain = crate::link::body_of(fields);
            for (peer, secret) in to {
                self.send_link_to(&peer, &secret, &plain);
            }
        }
    }

    fn forget_preview_image(&mut self, image: Option<Id>) {
        if let Some(h) = image
            && !self.previews.kept.values().any(|k| k.image == Some(h))
            && self.previews.images.remove(&h).is_some()
        {
            self.changes.push(Change::Blob { key: image_key(&h), value: None });
        }
    }

    fn save_previews(&mut self) {
        let value = serde_json::to_vec(&self.previews.kept).ok();
        self.changes.push(Change::Blob { key: BLOB.into(), value });
    }

    /// At load: the kept previews and their pictures.
    pub(crate) fn load_previews(&mut self, index: Option<&[u8]>, images: Vec<(Id, Vec<u8>)>) {
        self.previews.kept = index.and_then(|v| serde_json::from_slice(v).ok()).unwrap_or_default();
        for (h, b) in images {
            if blake3::hash(&b).as_bytes() == h.as_bytes() && self.previews.kept.values().any(|k| k.image == Some(h)) {
                self.previews.images.insert(h, b);
            } else {
                self.changes.push(Change::Blob { key: image_key(&h), value: None });
            }
        }
    }

    /// On a home node: a paired device asks for a preview (`pv`).
    pub(crate) fn on_preview_ask(&mut self, peer: &str, secret: &[u8], url: &str) {
        if !self.can_relay() || !self.peers.get(peer).is_some_and(|p| p.client) || !fetchable(url) {
            return;
        }
        let now = self.now();
        let served = self.previews.served.entry(peer.to_string()).or_insert((now / 60, 0));
        if served.0 != now / 60 {
            *served = (now / 60, 0);
        }
        if served.1 >= SERVE_PER_MINUTE {
            return;
        }
        served.1 += 1;
        let who = BTreeSet::from([(peer.to_string(), secret.to_vec())]);
        if let Some(k) = self.previews.kept.get(url).cloned()
            && k.at + if k.failed { RETRY } else { FRESH } > now
        {
            // answers at once from what this node keeps
            return self.send_preview(url, &k, who);
        }
        self.previews.waiting.entry(url.to_string()).or_default().extend(who);
        self.want_preview(url, false, now);
    }

    /// On a browser: the home node's answer (`pvd`). Its picture is checked again here.
    pub(crate) fn on_preview_answer(&mut self, peer: &str, msg: &BTreeMap<String, Value>) {
        if !self.peers.get(peer).is_some_and(|p| p.home) {
            return;
        }
        let text = |k: &str| match msg.get(k) {
            Some(Value::T(t)) => Some(t.clone()),
            _ => None,
        };
        let Some(url) = text("u").filter(|u| self.previews.asked.remove(u).is_some()) else { return };
        let kept = Kept {
            title: text("ti").and_then(|t| clean(&t, TITLE)),
            description: text("d").and_then(|d| clean(&d, DESCRIPTION)),
            site: text("s").and_then(|s| clean(&s, 60)).unwrap_or_else(|| site_of(&url)),
            image: None,
            at: self.now(),
            failed: matches!(msg.get("x"), Some(Value::Bool(true))),
        };
        let pic = match msg.get("i") {
            Some(Value::B(b)) => picture(b),
            _ => None,
        };
        self.finish_preview(&url, kept, pic);
    }
}

/// The host of a URL, as a preview names its site when the page doesn't.
fn site_of(url: &str) -> String {
    origin(url).map(|(_, h)| h.trim_start_matches("www.").to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_says_what_it_is_and_nothing_else_is_read() {
        let html = br#"<!doctype html><html><head><title>Fallback &amp; more</title>
            <meta property="og:title" content="Signal&#39;s new   audit">
            <meta name=description content='A short &quot;summary&quot;'>
            <meta property="og:image" content="/img/card.png">
            <meta property="og:site_name" content="Example News">
            <script>document.title = "never read"</script></head><body>...</body></html>"#;
        let p = parse_page(html);
        assert_eq!(p.title.as_deref(), Some("Signal's new audit"));
        assert_eq!(p.description.as_deref(), Some("A short \"summary\""));
        assert_eq!(p.site.as_deref(), Some("Example News"));
        assert_eq!(p.image.as_deref(), Some("/img/card.png"));
        let bare = parse_page("<html><head><title>\n  Just a title\u{202e} </title></head></html>".as_bytes());
        assert_eq!(bare.title.as_deref(), Some("Just a title"));
        assert_eq!(bare.description, None);
    }

    #[test]
    fn pictures_resolve_against_the_page_and_stay_on_https() {
        let page = "https://news.example/2026/story.html?x=1";
        assert_eq!(resolve(page, "/img/a.png").as_deref(), Some("https://news.example/img/a.png"));
        assert_eq!(resolve(page, "a.png").as_deref(), Some("https://news.example/2026/a.png"));
        assert_eq!(resolve(page, "//cdn.example/a.png").as_deref(), Some("https://cdn.example/a.png"));
        assert_eq!(resolve(page, "https://cdn.example/a.png").as_deref(), Some("https://cdn.example/a.png"));
        assert_eq!(resolve(page, "http://cdn.example/a.png"), None);
        assert_eq!(resolve(page, "data:image/png;base64,AAAA"), None);
        assert!(!fetchable("http://news.example/"));
        assert!(!fetchable("https://user@news.example/"));
        assert!(!fetchable("https:// news.example/"));
        assert!(fetchable("https://news.example"));
    }

    #[test]
    fn long_text_is_cut_and_odd_characters_go() {
        let long = "word ".repeat(200);
        let t = clean(&long, TITLE).unwrap();
        assert!(t.chars().count() <= TITLE + 1 && t.ends_with('…'));
        assert_eq!(clean("a\u{200b}b\tc", 10).as_deref(), Some("ab c"));
        assert_eq!(clean("   ", 10), None);
        assert_eq!(decode_entities("&#x41;&#66;&bogus;&"), "AB&bogus;&");
    }
}
