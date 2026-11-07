//! The WebRTC crypto provider: str0m's RustCrypto backend (DTLS via dimpl, SRTP, STUN HMAC,
//! SHA-256), vendored so the certificate can be made with `rcgen` on `ring` (already in the tree
//! through iroh's `tls-ring`) instead of pulling `aws-lc-rs`, a C and CMake build, into every
//! desktop, phone and node build. The four files next to this one are str0m-rust-crypto 0.6.0
//! (MIT OR Apache-2.0, https://github.com/algesten/str0m), unchanged but for their headers, the
//! certificate maker in dtls.rs, which calls [`certificate`], and DTLS server cookies, which
//! dtls.rs turns back on (dimpl's default): ICE consent proves nothing here (rtc.rs).

mod dtls;
mod sha1;
mod sha256;
mod srtp;

use str0m::config::{CryptoProvider, DtlsCert};

pub(crate) fn provider() -> CryptoProvider {
    static SRTP: srtp::RustCryptoSrtpProvider = srtp::RustCryptoSrtpProvider;
    static SHA1_HMAC: sha1::RustCryptoSha1HmacProvider = sha1::RustCryptoSha1HmacProvider;
    static SHA256: sha256::RustCryptoSha256Provider = sha256::RustCryptoSha256Provider;
    static DTLS: dtls::RustCryptoDtlsProvider = dtls::RustCryptoDtlsProvider;
    CryptoProvider { srtp_provider: &SRTP, sha1_hmac_provider: &SHA1_HMAC, sha256_provider: &SHA256, dtls_provider: &DTLS }
}

/// A self-signed ECDSA P-256 certificate for DTLS. Browsers check its fingerprint, never its
/// dates or names, so it can live as long as the node: routes in invites keep working.
pub(crate) fn certificate() -> Option<DtlsCert> {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).ok()?;
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).ok()?;
    params.distinguished_name.push(rcgen::DnType::CommonName, "hoi node");
    params.is_ca = rcgen::IsCa::NoCa;
    params.not_before = rcgen::date_time_ymd(2026, 1, 1);
    params.not_after = rcgen::date_time_ymd(2099, 12, 31);
    // Unique serials: Firefox refuses two certificates with the same issuer and serial.
    let serial: [u8; 16] = hoi_crypto::random_bytes().ok()?;
    params.serial_number = Some(serial.to_vec().into());
    let cert = params.self_signed(&key).ok()?;
    Some(DtlsCert { certificate: cert.der().to_vec(), private_key: key.serialize_der() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use str0m_proto::crypto::DtlsVersion;
    use str0m_proto::crypto::dtls::{DtlsInstance, DtlsOutput};

    /// The random of a HelloRetryRequest (RFC 8446 §4.1.3).
    const HRR: [u8; 32] = [
        0xcf, 0x21, 0xad, 0x74, 0xe5, 0x9a, 0x61, 0x11, 0xbe, 0x1d, 0x8c, 0x02, 0x1e, 0x65, 0xb8, 0x91, 0xc2, 0xa2, 0x11, 0x16, 0x7a, 0xbb, 0x8c, 0x5e, 0x07,
        0x9e, 0x09, 0xe2, 0xc8, 0xa8, 0x33, 0x9c,
    ];

    /// What one side sends now, a datagram per packet, and whether it is connected.
    fn flight(d: &mut dyn DtlsInstance, connected: &mut bool) -> Vec<Vec<u8>> {
        let mut buf = vec![0u8; 4096];
        let mut out = Vec::new();
        loop {
            match d.poll_output(&mut buf) {
                DtlsOutput::Packet(p) => out.push(p.to_vec()),
                DtlsOutput::Connected => *connected = true,
                DtlsOutput::Timeout(_) => return out,
                _ => {}
            }
        }
    }

    #[test]
    fn the_node_asks_for_a_dtls_cookie_before_any_key_exchange() {
        let p = provider();
        // A DTLS 1.2 client (HelloVerifyRequest) and one offering 1.3 too (HelloRetryRequest).
        for version in [DtlsVersion::Dtls12, DtlsVersion::Auto] {
            let now = Instant::now();
            let node = certificate().unwrap();
            let mut server = p.dtls_provider.new_dtls(&node, now, DtlsVersion::Auto, None).unwrap();
            server.set_active(false);
            server.handle_timeout(now).unwrap();
            let mut client = p.dtls_provider.new_dtls(&certificate().unwrap(), now, version, None).unwrap();
            client.set_active(true);
            client.handle_timeout(now).unwrap();
            let (mut up, mut down) = (false, false);
            for d in flight(client.as_mut(), &mut down) {
                server.handle_packet(&d).unwrap();
            }
            let answer = flight(server.as_mut(), &mut up);
            assert_eq!(answer.len(), 1, "{version}: one datagram");
            let a = &answer[0];
            // Record header (13 bytes), handshake header (12), then the message.
            let asks = a[0] == 22 && (a[13] == 3 || (a[13] == 2 && a[13 + 12 + 2..13 + 12 + 2 + 32] == HRR));
            assert!(asks, "{version}: the first answer asks for a cookie");
            assert!(a.len() < 256 && !a.windows(64).any(|w| w == &node.certificate[..64]), "{version}: no certificate before the cookie");
            // With the cookie echoed, the handshake completes.
            let mut wire = answer;
            let mut t = now;
            for _ in 0..16 {
                for d in &wire {
                    client.handle_packet(d).unwrap();
                }
                for d in flight(client.as_mut(), &mut down) {
                    server.handle_packet(&d).unwrap();
                }
                wire = flight(server.as_mut(), &mut up);
                if up && down {
                    break;
                }
                t += Duration::from_millis(10);
                client.handle_timeout(t).unwrap();
                server.handle_timeout(t).unwrap();
            }
            assert!(up && down, "{version}: connected after the cookie");
        }
    }
}
