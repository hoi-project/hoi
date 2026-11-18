//! The native side of link previews: fetches what the core asks for, for this device
//! and the browsers paired with it, and hands back the bytes. Every hop, the first address and
//! each of at most three redirects, is checked the same way: https, no credentials, a name that
//! resolves only to addresses on the internet, and the connection made to exactly those
//! addresses (no second lookup an attacker's DNS could answer differently). No cookies, no
//! referrer, a generic user agent, [`PREVIEW_BYTES`] and [`PREVIEW_SECS`] at most. With "Only
//! through my VPN" every request leaves from the VPN's address, and none while it is down.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use hoi_core::preview::{PREVIEW_BYTES, PREVIEW_REDIRECTS, PREVIEW_SECS, fetchable};
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

use crate::Shared;
use crate::vpn::{NetPolicy, VpnState};

/// Fetches under way at once.
const AT_ONCE: usize = 4;
const POLL: Duration = Duration::from_millis(500);

pub struct PreviewHandle {
    task: JoinHandle<()>,
}

impl Drop for PreviewHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Which networks may be used, asked again for every fetch.
pub type Policy = Arc<dyn Fn() -> NetPolicy + Send + Sync>;

/// Fetches previews for the node until the handle is dropped.
pub fn start_previews(shared: Arc<Shared>, policy: Policy) -> PreviewHandle {
    let slots = Arc::new(Semaphore::new(AT_ONCE));
    let task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(POLL).await;
            let want = shared.with(|n| n.take_preview_fetches());
            for f in want {
                let (shared, policy, slots) = (shared.clone(), policy.clone(), slots.clone());
                tokio::spawn(async move {
                    let Ok(_slot) = slots.acquire_owned().await else { return };
                    let body = match tokio::time::timeout(Duration::from_secs(PREVIEW_SECS), fetch(&f.url, f.image, &policy())).await {
                        Ok(r) => r,
                        Err(_) => Err("the site took too long".into()),
                    };
                    shared.with(|n| n.preview_fetched(&f.url, f.image, body));
                });
            }
        }
    });
    PreviewHandle { task }
}

fn client(host: &str, addrs: &[SocketAddr], from: Option<&(IpAddr, String)>) -> Result<reqwest::Client, String> {
    let mut b = crate::public_http_client_builder().map_err(|e| e.to_string())?;
    if let Some((ip, _interface)) = from {
        b = b.local_address(*ip);
        #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "ios"))]
        {
            b = b.interface(_interface);
        }
    }
    b.resolve_to_addrs(host, addrs)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(PREVIEW_SECS))
        .user_agent("hoi link preview")
        .build()
        .map_err(|e| e.to_string())
}

/// What a fetch is for: a page, a picture, or a short text file (a handle's well-known file).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Want {
    Page,
    Image,
    Text,
}

/// One page or picture, by the rules above.
pub async fn fetch(url: &str, image: bool, policy: &NetPolicy) -> Result<Vec<u8>, String> {
    fetch_as(url, if image { Want::Image } else { Want::Page }, policy).await
}

/// One fetch of `want`, by the rules above.
pub async fn fetch_as(url: &str, want: Want, policy: &NetPolicy) -> Result<Vec<u8>, String> {
    let image = want == Want::Image;
    let from = match policy.state() {
        VpnState::Off => None,
        VpnState::Up(_) => {
            let state = policy.state();
            let ip = state.ipv4().map(IpAddr::V4).or_else(|| state.addresses().first().copied()).ok_or("the VPN has no address")?;
            Some((ip, policy.vpn.clone().unwrap_or_default()))
        }
        VpnState::Down(why) => return Err(format!("only through the VPN, and it {}", why.words())),
    };
    let mut url = url.to_string();
    for _ in 0..=PREVIEW_REDIRECTS {
        if !fetchable(&url) {
            return Err("previews follow https links only".into());
        }
        let parsed = reqwest::Url::parse(&url).map_err(|_| "not a link".to_string())?;
        let host = parsed.host_str().ok_or("a link without a host")?.trim_start_matches('[').trim_end_matches(']').to_string();
        let port = parsed.port_or_known_default().unwrap_or(443);
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port)).await.map_err(|_| "no such site".to_string())?.collect();
        if addrs.is_empty() || addrs.iter().any(|a| !crate::public_ip(a.ip())) {
            return Err("that site is not on the internet (a private or local address)".into());
        }
        let accept = match want {
            Want::Image => "image/png,image/jpeg,image/webp,image/gif",
            Want::Page => "text/html,application/xhtml+xml",
            Want::Text => "text/plain",
        };
        let response = client(&host, &addrs, from.as_ref())?
            .get(parsed.clone())
            .header(reqwest::header::ACCEPT, accept)
            .send()
            .await
            .map_err(|_| "the site didn't answer".to_string())?;
        if response.status().is_redirection() {
            let to = response.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()).ok_or("a redirect to nowhere")?;
            url = parsed.join(to).map_err(|_| "a redirect to nowhere".to_string())?.to_string();
            continue;
        }
        if !response.status().is_success() {
            return Err(format!("the site answered {}", response.status().as_u16()));
        }
        let kind = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
        let fits = match want {
            Want::Image => kind.starts_with("image/"),
            Want::Page => kind.starts_with("text/html") || kind.starts_with("application/xhtml+xml"),
            Want::Text => kind.starts_with("text/plain") || kind.is_empty() || kind.starts_with("application/octet-stream"),
        };
        if !fits {
            return Err("not a page or a picture".into());
        }
        return read_bounded(response, image).await;
    }
    Err("too many redirects".into())
}

/// The body up to [`PREVIEW_BYTES`]: a page is cut there (what a preview reads is near the top),
/// a picture that doesn't fit is none.
async fn read_bounded(mut response: reqwest::Response, image: bool) -> Result<Vec<u8>, String> {
    if image && response.content_length().is_some_and(|n| n > PREVIEW_BYTES as u64) {
        return Err("the picture is too large".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "the site went away mid-answer".to_string())? {
        if body.len() + chunk.len() > PREVIEW_BYTES {
            if image {
                return Err("the picture is too large".into());
            }
            body.extend_from_slice(&chunk[..PREVIEW_BYTES - body.len()]);
            break;
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn preview_client_builds_before_a_peer_connection() {
        client("preview.invalid", &["127.0.0.1:443".parse().unwrap()], None).expect("build with the selected preview endpoint");
    }

    #[tokio::test]
    async fn the_shared_http_client_starts_tls_without_a_peer_connection() {
        use tokio::io::AsyncReadExt;

        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let url = format!("https://{}/", listener.local_addr().unwrap());
        // This loopback fixture must be reachable on machines with an HTTPS proxy.
        let client = crate::public_http_client_builder().unwrap().no_proxy().https_only(true).build().unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            let request = client.get(url).send();
            tokio::pin!(request);
            let receive = async {
                let (mut socket, _) = listener.accept().await.unwrap();
                assert_eq!(socket.read_u8().await.unwrap(), 0x16, "expected a TLS handshake");
            };
            tokio::select! {
                _ = receive => assert!(request.await.unwrap_err().is_connect(), "an interrupted handshake must fail the request"),
                result = &mut request => panic!("request finished before the loopback listener received TLS: {result:?}"),
            }
        })
        .await
        .expect("the HTTP client must reach the loopback listener");
    }

    #[tokio::test]
    async fn local_and_plain_addresses_are_never_fetched() {
        let open = NetPolicy::default();
        for url in ["http://example.com/", "https://127.0.0.1/", "https://localhost/", "https://10.0.0.1/x", "https://[::1]/", "https://user@example.com/"] {
            assert!(fetch(url, false, &open).await.is_err(), "{url}");
        }
    }

    #[tokio::test]
    async fn nothing_leaves_while_the_vpn_is_down() {
        let vpn = NetPolicy { vpn: Some("hoi-test-no-such-interface".into()), home_network: false };
        let e = fetch("https://example.com/", false, &vpn).await.unwrap_err();
        assert!(e.contains("VPN"), "{e}");
    }
}
