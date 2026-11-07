//! Shamir's secret sharing over GF(2⁸), byte by byte: `k` of `n` shares rebuild
//! the secret, fewer say nothing about it. Social recovery splits a person's recovery line into
//! five shares, any three of which bring it back.
//!
//! The field is the AES one (x⁸ + x⁴ + x³ + x + 1), the same as the first-run rehearsal in the app
//! (`apps/web/src/lib/sayhoi/shamir.ts`), so `core/vectors/shamir.json` holds both. Unlike
//! scattered delivery's Reed–Solomon code, which only ever sees ciphertext, this runs on secrets:
//! multiplication and inversion take the same steps whatever the bytes, with no tables and no
//! branches on secret values.

use zeroize::Zeroizing;

use crate::CryptoError;

/// a · b in GF(2⁸), in constant time.
fn mul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0u8;
    for _ in 0..8 {
        p ^= a & 0u8.wrapping_sub(b & 1);
        let carry = 0u8.wrapping_sub(a >> 7);
        a = (a << 1) ^ (0x1b & carry);
        b >>= 1;
    }
    p
}

/// a⁻¹ = a²⁵⁴ in GF(2⁸) (0 for 0), in constant time.
fn inv(a: u8) -> u8 {
    let a2 = mul(a, a);
    let a4 = mul(a2, a2);
    let a8 = mul(a4, a4);
    let a16 = mul(a8, a8);
    let a32 = mul(a16, a16);
    let a64 = mul(a32, a32);
    let a128 = mul(a64, a64);
    mul(a128, mul(a64, mul(a32, mul(a16, mul(a8, mul(a4, a2))))))
}

/// One share: its point `x` (1 to 255) and the secret's polynomials at `x`.
#[derive(Clone)]
pub struct Share {
    pub x: u8,
    pub y: Zeroizing<Vec<u8>>,
}

impl std::fmt::Debug for Share {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Share(x = {}, {} bytes)", self.x, self.y.len())
    }
}

impl Share {
    /// `x ‖ y`.
    pub fn to_bytes(&self) -> Zeroizing<Vec<u8>> {
        let mut out = Zeroizing::new(Vec::with_capacity(1 + self.y.len()));
        out.push(self.x);
        out.extend_from_slice(&self.y);
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Share, CryptoError> {
        match bytes.split_first() {
            Some((&x, y)) if x != 0 && !y.is_empty() => Ok(Share { x, y: Zeroizing::new(y.to_vec()) }),
            _ => Err(CryptoError::Format),
        }
    }
}

/// Splits `secret` into `n` shares, any `k` of which rebuild it, with coefficients from the
/// system's randomness.
pub fn split(secret: &[u8], n: u8, k: u8) -> Result<Vec<Share>, CryptoError> {
    let mut failed = false;
    let shares = split_with(secret, n, k, |coeffs| match getrandom::fill(coeffs) {
        Ok(()) => {}
        Err(_) => failed = true,
    })?;
    if failed { Err(CryptoError::Rng) } else { Ok(shares) }
}

/// [`split`] with the coefficients for each byte from `random` (`k - 1` of them, c₁ first), for
/// test vectors.
pub fn split_with(secret: &[u8], n: u8, k: u8, mut random: impl FnMut(&mut [u8])) -> Result<Vec<Share>, CryptoError> {
    if k < 2 || k > n || secret.is_empty() {
        return Err(CryptoError::Format);
    }
    let mut shares: Vec<Share> = (1..=n).map(|x| Share { x, y: Zeroizing::new(vec![0u8; secret.len()]) }).collect();
    let mut coeffs = Zeroizing::new(vec![0u8; k as usize - 1]);
    for (i, &byte) in secret.iter().enumerate() {
        random(&mut coeffs);
        for s in shares.iter_mut() {
            // Horner: secret + c₁·x + c₂·x² + …
            let mut y = 0u8;
            for &c in coeffs.iter().rev() {
                y = mul(y ^ c, s.x);
            }
            s.y[i] = y ^ byte;
        }
    }
    Ok(shares)
}

/// Lagrange interpolation at 0. The shares must have distinct points and equal lengths; with
/// fewer shares than the threshold the answer is noise, which the caller's own check catches
/// (the recovery line carries one).
pub fn combine(shares: &[Share]) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    let len = shares.first().map(|s| s.y.len()).ok_or(CryptoError::Format)?;
    if shares.len() < 2 || shares.iter().any(|s| s.x == 0 || s.y.len() != len) {
        return Err(CryptoError::Format);
    }
    for (i, a) in shares.iter().enumerate() {
        if shares[i + 1..].iter().any(|b| b.x == a.x) {
            return Err(CryptoError::Format);
        }
    }
    let mut out = Zeroizing::new(vec![0u8; len]);
    for (i, si) in shares.iter().enumerate() {
        // the basis polynomial for share i at x = 0: ∏ xⱼ / (xⱼ − xᵢ), and minus is xor here
        let mut l = 1u8;
        for (j, sj) in shares.iter().enumerate() {
            if i != j {
                l = mul(l, mul(sj.x, inv(sj.x ^ si.x)));
            }
        }
        for (o, &y) in out.iter_mut().zip(si.y.iter()) {
            *o ^= mul(y, l);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_field_is_the_aes_field() {
        // FIPS 197 §4.2: {57} · {83} = {c1}, and every element but 0 has an inverse
        assert_eq!(mul(0x57, 0x83), 0xc1);
        assert_eq!(mul(0x57, 0x13), 0xfe);
        for a in 1..=255u8 {
            assert_eq!(mul(a, inv(a)), 1, "{a}");
        }
        assert_eq!(inv(0), 0);
    }

    #[test]
    fn any_three_of_five_rebuild_and_two_do_not() {
        let secret = crate::random_bytes::<32>().unwrap();
        let shares = split(&secret, 5, 3).unwrap();
        for a in 0..5 {
            for b in a + 1..5 {
                for c in b + 1..5 {
                    let picked = [shares[a].clone(), shares[b].clone(), shares[c].clone()];
                    assert_eq!(combine(&picked).unwrap().as_slice(), secret, "{a}{b}{c}");
                }
                assert_ne!(combine(&[shares[a].clone(), shares[b].clone()]).unwrap().as_slice(), secret);
            }
        }
        assert_eq!(combine(&shares).unwrap().as_slice(), secret, "more than enough is fine");
        let round = Share::from_bytes(&shares[0].to_bytes()).unwrap();
        assert_eq!((round.x, round.y.as_slice()), (shares[0].x, shares[0].y.as_slice()));
    }

    #[test]
    fn malformed_sets_are_refused() {
        let shares = split(b"secret", 3, 2).unwrap();
        assert!(combine(&[shares[0].clone()]).is_err());
        assert!(combine(&[shares[0].clone(), shares[0].clone()]).is_err());
        let short = Share { x: 9, y: Zeroizing::new(vec![1]) };
        assert!(combine(&[shares[0].clone(), short]).is_err());
        assert!(split(b"x", 2, 3).is_err() && split(b"x", 2, 1).is_err() && split(b"", 3, 2).is_err());
        assert!(Share::from_bytes(&[0, 1]).is_err() && Share::from_bytes(&[1]).is_err());
    }

    /// `core/vectors/shamir.json`: a fixed secret, fixed coefficients, the five shares, and the
    /// secret again from each set of three.
    #[test]
    fn the_shared_vectors() {
        let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../vectors/shamir.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let hex = |s: &serde_json::Value| hoi_types::hex_decode(s.as_str().unwrap()).unwrap();
        let secret = hex(&v["secret"]);
        let coeffs = hex(&v["coefficients"]);
        let k = v["k"].as_u64().unwrap() as u8;
        let mut at = 0;
        let shares = split_with(&secret, v["n"].as_u64().unwrap() as u8, k, |c| {
            c.copy_from_slice(&coeffs[at..at + c.len()]);
            at += c.len();
        })
        .unwrap();
        for (s, want) in shares.iter().zip(v["shares"].as_array().unwrap()) {
            assert_eq!(s.to_bytes().as_slice(), hex(want).as_slice());
        }
        assert_eq!(combine(&shares[1..4]).unwrap().as_slice(), secret.as_slice());
    }
}
