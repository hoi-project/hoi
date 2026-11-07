//! Compatibility and tamper checks for the vendored provider dependency migration.
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::RustCrypto;
use openmls_traits::{crypto::OpenMlsCrypto, signatures::Signer, types::*};

#[test]
fn provider_signatures_interoperate_with_basic_credentials() {
    let crypto = RustCrypto::default();
    for scheme in [SignatureScheme::ED25519, SignatureScheme::ECDSA_SECP256R1_SHA256, SignatureScheme::ECDSA_SECP384R1_SHA384] {
        let key = SignatureKeyPair::new(scheme).unwrap();
        let signature = key.sign(b"hoi signed message").unwrap();
        crypto.verify_signature(scheme, b"hoi signed message", key.public(), &signature).unwrap();
        assert!(crypto.verify_signature(scheme, b"modified message", key.public(), &signature).is_err());
        let (secret, public) = crypto.signature_key_gen(scheme).unwrap();
        let signature = crypto.sign(scheme, b"provider generated", &secret).unwrap();
        crypto.verify_signature(scheme, b"provider generated", &public, &signature).unwrap();
        assert!(crypto.verify_signature(scheme, b"provider generated", key.public(), &signature).is_err());
    }
}

#[test]
fn all_supported_mls_suites_authenticate_hpke_and_exports() {
    let crypto = RustCrypto::default();
    for suite in crypto.supported_ciphersuites() {
        let config = || suite.hpke_config();
        let recipient = crypto.derive_hpke_keypair(config(), &[7; 48]).unwrap();
        let ciphertext = crypto.hpke_seal(config(), &recipient.public, b"hoi context", b"room", b"private message").unwrap();
        assert_eq!(crypto.hpke_open(config(), &ciphertext, recipient.private.as_ref(), b"hoi context", b"room").unwrap(), b"private message");
        assert!(crypto.hpke_open(config(), &ciphertext, recipient.private.as_ref(), b"wrong context", b"room").is_err());
        assert!(crypto.hpke_open(config(), &ciphertext, recipient.private.as_ref(), b"hoi context", b"wrong room").is_err());
        let other = crypto.derive_hpke_keypair(config(), &[8; 48]).unwrap();
        assert!(crypto.hpke_open(config(), &ciphertext, other.private.as_ref(), b"hoi context", b"room").is_err());
        let (enc, sender) = crypto.hpke_setup_sender_and_export(config(), &recipient.public, b"hoi", b"export", 32).unwrap();
        let receiver = crypto.hpke_setup_receiver_and_export(config(), &enc, recipient.private.as_ref(), b"hoi", b"export", 32).unwrap();
        assert_eq!(&*sender, &*receiver);
    }
}

#[test]
fn aead_checks_nonce_lengths_and_rejects_tampering() {
    let crypto = RustCrypto::default();
    for algorithm in [AeadType::Aes128Gcm, AeadType::Aes256Gcm, AeadType::ChaCha20Poly1305] {
        let key = vec![3; algorithm.key_size()];
        let nonce = vec![4; algorithm.nonce_size()];
        let mut ciphertext = crypto.aead_encrypt(algorithm, &key, b"secret", &nonce, b"context").unwrap();
        assert_eq!(crypto.aead_decrypt(algorithm, &key, &ciphertext, &nonce, b"context").unwrap(), b"secret");
        assert!(crypto.aead_decrypt(algorithm, &key, &ciphertext, &nonce, b"wrong context").is_err());
        ciphertext[0] ^= 1;
        assert!(crypto.aead_decrypt(algorithm, &key, &ciphertext, &nonce, b"context").is_err());
        assert!(crypto.aead_encrypt(algorithm, &key, b"secret", &nonce[..nonce.len() - 1], b"context").is_err());
    }
}
