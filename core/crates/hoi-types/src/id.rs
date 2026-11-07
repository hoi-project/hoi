//! 32-byte identifiers: record ids (BLAKE3 of canonical bytes) and Ed25519 public keys.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::TypesError;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Id(pub [u8; 32]);

impl Id {
    pub const ZERO: Id = Id([0; 32]);

    pub fn hash(data: &[u8]) -> Id {
        Id(*blake3::hash(data).as_bytes())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn from_slice(b: &[u8]) -> Result<Id, TypesError> {
        let arr: [u8; 32] = b.try_into().map_err(|_| TypesError::Length { want: 32, got: b.len() })?;
        Ok(Id(arr))
    }

    pub fn hex(&self) -> String {
        hex_encode(&self.0)
    }

    pub fn from_hex(s: &str) -> Result<Id, TypesError> {
        let b = hex_decode(s)?;
        Id::from_slice(&b)
    }

    /// `a7f2·91c0`, how keys appear next to names.
    pub fn short_key(&self) -> String {
        let h = self.hex();
        format!("{}·{}", &h[0..4], &h[4..8])
    }

    /// `b3:9e1c…4f0a`, how record ids appear.
    pub fn short_id(&self) -> String {
        let h = self.hex();
        format!("b3:{}…{}", &h[0..4], &h[60..64])
    }
}

impl fmt::Debug for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id({})", &self.hex()[..16])
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}

impl FromStr for Id {
    type Err = TypesError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Id::from_hex(s)
    }
}

impl Serialize for Id {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}

impl<'de> Deserialize<'de> for Id {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Id::from_hex(&s).map_err(serde::de::Error::custom)
    }
}

pub fn hex_encode(b: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push(HEX[(x >> 4) as usize] as char);
        s.push(HEX[(x & 15) as usize] as char);
    }
    s
}

pub fn hex_decode(s: &str) -> Result<Vec<u8>, TypesError> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return Err(TypesError::Hex);
    }
    let nib = |c: u8| -> Result<u8, TypesError> {
        match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            _ => Err(TypesError::Hex),
        }
    };
    s.as_bytes().chunks(2).map(|p| Ok(nib(p[0])? << 4 | nib(p[1])?)).collect()
}

/// A 64-byte Ed25519 signature carried next to a record.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Sig(pub [u8; 64]);

impl Sig {
    pub fn from_slice(b: &[u8]) -> Result<Sig, TypesError> {
        let arr: [u8; 64] = b.try_into().map_err(|_| TypesError::Length { want: 64, got: b.len() })?;
        Ok(Sig(arr))
    }
}

impl fmt::Debug for Sig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sig({}…)", hex_encode(&self.0[..8]))
    }
}
