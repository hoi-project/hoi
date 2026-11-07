//! The one HTTPS client this node uses to talk to registries (announcing to and mirroring an
//! index, mirroring another registry's names): the same ring-backed rustls the rest of the
//! node carries, with Mozilla's roots compiled in, so a container needs no certificate store.
//! Answers are bounded before they are read, so a misbehaving service cannot fill memory.

use std::net::IpAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Only through a VPN: every request leaves from its address, on its interface (`--vpn-only`).
static BOUND: OnceLock<(IpAddr, String)> = OnceLock::new();

/// From now on requests leave from `ip`, on `interface` where the system allows it.
pub fn bind_to(ip: IpAddr, interface: &str) {
    let _ = BOUND.set((ip, interface.to_string()));
}

/// The most a registry may send back for one request; its pages are a few kilobytes.
pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

pub fn client() -> Result<reqwest::Client, String> {
    client_with(REQUEST_TIMEOUT)
}

fn client_with(timeout: Duration) -> Result<reqwest::Client, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let mut builder = reqwest::Client::builder();
    if let Some((ip, _interface)) = BOUND.get() {
        builder = builder.local_address(*ip);
        #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "ios"))]
        {
            builder = builder.interface(_interface);
        }
    }
    builder
        .use_preconfigured_tls(tls)
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!("hoi-node/{}", hoi_core::VERSION))
        .build()
        .map_err(|e| e.to_string())
}

/// The body, or an error once it grows past what any registry answer could need.
pub async fn bounded(mut response: reqwest::Response) -> Result<Vec<u8>, String> {
    let too_big = || "the public index answered with far too much".to_string();
    if response.content_length().is_some_and(|n| n > MAX_RESPONSE_BYTES as u64) {
        return Err(too_big());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "the public index went away mid-answer".to_string())? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(too_big());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// `GET base + path` as JSON: `None` for 404, an error for anything else that isn't 200.
pub async fn get_json<T: serde::de::DeserializeOwned>(client: &reqwest::Client, base: &str, path: &str) -> Result<Option<T>, String> {
    let response = client.get(format!("{base}{path}")).send().await.map_err(|_| "the public index is unreachable".to_string())?;
    match response.status().as_u16() {
        200 => {
            let body = bounded(response).await?;
            serde_json::from_slice::<T>(&body).map(Some).map_err(|_| "the public index answered with something unexpected".to_string())
        }
        404 => Ok(None),
        status => Err(format!("the public index answered {status}")),
    }
}

/// A registry URL: HTTPS, or plain HTTP on this machine for tests and development. No
/// credentials, query or fragment, no trailing slash.
pub fn valid_index_url(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    let host = url.host_str()?;
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    let scheme_ok = url.scheme() == "https" || (url.scheme() == "http" && loopback);
    if !scheme_ok || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return None;
    }
    Some(value.trim_end_matches('/').to_string())
}

pub fn loopback_index(url: &str) -> bool {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| matches!(h, "localhost" | "127.0.0.1" | "[::1]"))).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_or_local_http_registries_without_secrets() {
        assert_eq!(valid_index_url("https://hoi.sh/registry/").as_deref(), Some("https://hoi.sh/registry"));
        assert_eq!(valid_index_url("http://127.0.0.1:7845").as_deref(), Some("http://127.0.0.1:7845"));
        for bad in ["http://hoi.sh/registry", "https://user:pw@hoi.sh/registry", "https://hoi.sh/registry?x=1", "https://hoi.sh/registry#x", "hoi.sh", ""] {
            assert!(valid_index_url(bad).is_none(), "{bad}");
        }
        assert!(loopback_index("http://localhost:7845"));
        assert!(!loopback_index("https://hoi.sh/registry"));
    }
}
