//! Exercise the P-curve migration against rcgen/ring-generated certificates.
use dimpl::crypto::{Buf, HashAlgorithm, SignatureAlgorithm, rust_crypto};

#[test]
fn all_dtls_key_exchange_groups_agree_and_reject_bad_points() {
    let provider = rust_crypto::default_provider();
    assert_eq!(provider.kx_groups.len(), 3);
    for group in provider.kx_groups {
        let left = group.start_exchange(Buf::new()).unwrap();
        let right = group.start_exchange(Buf::new()).unwrap();
        let left_public = left.pub_key().to_vec();
        let right_public = right.pub_key().to_vec();
        let mut left_secret = Buf::new();
        let mut right_secret = Buf::new();
        left.complete(&right_public, &mut left_secret).unwrap();
        right.complete(&left_public, &mut right_secret).unwrap();
        assert!(!left_secret.is_empty());
        assert_eq!(left_secret.as_ref(), right_secret.as_ref());
        let invalid = group.start_exchange(Buf::new()).unwrap();
        assert!(invalid.complete(&[0; 32], &mut Buf::new()).is_err());
    }
}

#[test]
fn dtls_signatures_interoperate_with_ring_certificates_and_reject_changes() {
    let provider = rust_crypto::default_provider();
    for (algorithm, hash) in [(&rcgen::PKCS_ECDSA_P256_SHA256, HashAlgorithm::SHA256), (&rcgen::PKCS_ECDSA_P384_SHA384, HashAlgorithm::SHA384)] {
        let key = rcgen::KeyPair::generate_for(algorithm).unwrap();
        let cert = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap().self_signed(&key).unwrap();
        let mut signing_key = provider.key_provider.load_private_key(&key.serialize_der()).unwrap();
        let mut signature = Buf::new();
        signing_key.sign(b"hoi DTLS transcript", hash, &mut signature).unwrap();
        provider.signature_verification.verify_signature(cert.der(), b"hoi DTLS transcript", &signature, hash, SignatureAlgorithm::ECDSA).unwrap();
        assert!(provider.signature_verification.verify_signature(cert.der(), b"tampered transcript", &signature, hash, SignatureAlgorithm::ECDSA).is_err());
        signature[0] ^= 1;
        assert!(provider.signature_verification.verify_signature(cert.der(), b"hoi DTLS transcript", &signature, hash, SignatureAlgorithm::ECDSA).is_err());
    }
}
