//! Link previews (M3): fetched by a desktop app or hoi-node, for itself and for the
//! browsers paired with it; kept on the device, never published.
use super::*;
use crate::preview::PreviewFetch;

const PAGE: &str = "https://news.example/story";

fn previews_on(n: &mut Node) {
    n.set_settings(&SettingsPatch { link_previews: Some(true), ..Default::default() }).unwrap();
}

fn html() -> Vec<u8> {
    br#"<html><head><title>Fallback</title><meta property="og:title" content="The audit, in short">
        <meta property="og:description" content="What changed &amp; why"><meta property="og:image" content="/card.png"></head></html>"#
        .to_vec()
}

/// What a host does: fetch what the core wants (here, from a fixed answer).
fn serve(n: &mut Node, png: &[u8]) -> Vec<PreviewFetch> {
    let mut asked = Vec::new();
    for _ in 0..4 {
        let want = n.take_preview_fetches();
        if want.is_empty() {
            break;
        }
        for f in want {
            let body = match (f.image, f.url.as_str()) {
                (false, PAGE) => Ok(html()),
                (true, "https://news.example/card.png") => Ok(png.to_vec()),
                _ => Err("not here".to_string()),
            };
            n.preview_fetched(&f.url, f.image, body);
            asked.push(f);
        }
    }
    n.commit().unwrap();
    asked
}

#[test]
fn a_desktop_fetches_a_preview_for_itself_and_keeps_it() {
    let t = Arc::new(AtomicU64::new(T0));
    let mem = Mem::default();
    let mut n = node("desktop", &t, &mem);
    n.create_identity("mira").unwrap();
    let records = n.store.len();
    assert_eq!(n.link_preview(PAGE).unwrap_err().code, "not-allowed", "off until the person turns it on");
    previews_on(&mut n);
    assert!(n.link_preview(PAGE).unwrap().is_none(), "asked for, not there yet");
    assert!(n.link_preview(PAGE).unwrap().is_none());
    let png = picture(48, image::ImageFormat::Png);
    let asked = serve(&mut n, &png);
    assert_eq!(asked.len(), 2, "the page once, then its picture: {asked:?}");
    assert!(n.take_events().iter().any(|e| matches!(e, crate::dto::Event::Preview { url } if url == PAGE)));
    let p = n.link_preview(PAGE).unwrap().expect("kept");
    assert_eq!(p.title.as_deref(), Some("The audit, in short"));
    assert_eq!(p.description.as_deref(), Some("What changed & why"));
    assert_eq!(p.site, "news.example");
    assert!(p.image.as_deref().is_some_and(|i| i.starts_with("data:image/png;base64,")));
    // kept across a restart, and nothing about it is a record
    drop(n);
    let mut again = node("desktop", &t, &mem);
    assert!(again.link_preview(PAGE).unwrap().is_some_and(|p| p.image.is_some()));
    assert!(again.take_preview_fetches().is_empty());
    assert_eq!(again.store.len(), records, "a preview is no record: nothing was published");
    // a page that can't be fetched is tried again only after a while
    assert!(again.link_preview("https://gone.example/").unwrap().is_none());
    serve(&mut again, &png);
    assert!(again.link_preview("https://gone.example/").unwrap().is_none());
    assert!(again.take_preview_fetches().is_empty());
    // only https
    assert!(again.link_preview("http://news.example/").unwrap().is_none());
    assert!(again.take_preview_fetches().is_empty());
}

#[test]
fn a_browser_asks_its_home_node_and_one_without_is_told_why() {
    let t = Arc::new(AtomicU64::new(T0));
    let mut home = node("desktop", &t, &Mem::default());
    let mut web = node("web", &t, &Mem::default());
    home.create_identity("mira").unwrap();
    web.create_identity("mira's browser").unwrap();
    previews_on(&mut web);
    assert_eq!(web.link_preview(PAGE).unwrap_err().code, "not-allowed", "no home node to fetch it");
    assert!(web.take_preview_fetches().is_empty(), "a browser never fetches itself");
    let c = home.create_community(draft("garden")).unwrap();
    pair(&mut home, &mut web, &c.id);
    assert!(web.link_preview(PAGE).unwrap().is_none());
    pump(&mut home, &mut web);
    let png = picture(48, image::ImageFormat::Png);
    // the home node fetches, though its own person never turned previews on there
    assert_eq!(serve(&mut home, &png).len(), 2);
    pump(&mut home, &mut web);
    let p = web.link_preview(PAGE).unwrap().expect("the home node's answer");
    assert_eq!(p.title.as_deref(), Some("The audit, in short"));
    assert!(p.image.is_some(), "the picture came, checked again here");
    assert!(web.take_preview_fetches().is_empty());
}
