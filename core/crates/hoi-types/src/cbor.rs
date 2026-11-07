//! Canonical DAG-CBOR, restricted to what hoi records use (PROTOCOL.md §1).
//!
//! Encoding is deterministic: shortest-form integers, definite lengths, map keys are text and
//! sorted length-first then bytewise, no floats, no tags. The decoder is strict and rejects
//! anything the encoder would not produce, so `encode(decode(b)) == b` for every accepted `b`.
//! That is what makes record ids stable and signatures non-malleable.

use std::cmp::Ordering;

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// Unsigned integer.
    U(u64),
    /// Negative integer, stored as `-1 - n`.
    N(u64),
    B(Vec<u8>),
    T(String),
    A(Vec<Value>),
    /// Text-keyed map. Keep it canonical by building it with [`Value::map`].
    M(Vec<(String, Value)>),
    Bool(bool),
    Null,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CborError {
    #[error("unexpected end of input")]
    Eof,
    #[error("trailing bytes after the value")]
    Trailing,
    #[error("integer not in shortest form")]
    NotShortest,
    #[error("indefinite lengths are not allowed")]
    Indefinite,
    #[error("floats are not allowed")]
    Float,
    #[error("tags are not allowed")]
    Tag,
    #[error("map keys must be text")]
    KeyNotText,
    #[error("map keys not in canonical order or duplicated")]
    KeyOrder,
    #[error("invalid utf-8 in text")]
    Utf8,
    #[error("unsupported simple value")]
    Simple,
    #[error("nesting too deep")]
    Depth,
    #[error("length too large")]
    TooLong,
}

pub const MAX_DEPTH: usize = 16;

/// DAG-CBOR key order: shorter keys first, then bytewise.
pub fn key_cmp(a: &str, b: &str) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.as_bytes().cmp(b.as_bytes()))
}

impl Value {
    /// Builds a canonical map from entries in any order. Entries with `Value::Null` are kept;
    /// callers drop optional fields themselves so absent and null stay distinct.
    pub fn map<K: Into<String>>(entries: impl IntoIterator<Item = (K, Value)>) -> Value {
        let mut v: Vec<(String, Value)> = entries.into_iter().map(|(k, v)| (k.into(), v)).collect();
        v.sort_by(|a, b| key_cmp(&a.0, &b.0));
        Value::M(v)
    }

    pub fn int(i: i64) -> Value {
        if i >= 0 { Value::U(i as u64) } else { Value::N((-(i + 1)) as u64) }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(128);
        encode_into(self, &mut out);
        out
    }
}

pub fn head(major: u8, n: u64, out: &mut Vec<u8>) {
    let m = major << 5;
    if n < 24 {
        out.push(m | n as u8);
    } else if n <= u8::MAX as u64 {
        out.push(m | 24);
        out.push(n as u8);
    } else if n <= u16::MAX as u64 {
        out.push(m | 25);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else if n <= u32::MAX as u64 {
        out.push(m | 26);
        out.extend_from_slice(&(n as u32).to_be_bytes());
    } else {
        out.push(m | 27);
        out.extend_from_slice(&n.to_be_bytes());
    }
}

pub fn encode_into(v: &Value, out: &mut Vec<u8>) {
    match v {
        Value::U(n) => head(0, *n, out),
        Value::N(n) => head(1, *n, out),
        Value::B(b) => {
            head(2, b.len() as u64, out);
            out.extend_from_slice(b);
        }
        Value::T(s) => encode_text(s, out),
        Value::A(items) => {
            head(4, items.len() as u64, out);
            for i in items {
                encode_into(i, out);
            }
        }
        Value::M(entries) => {
            debug_assert!(entries.windows(2).all(|w| key_cmp(&w[0].0, &w[1].0) == Ordering::Less), "map not canonical");
            head(5, entries.len() as u64, out);
            for (k, v) in entries {
                encode_text(k, out);
                encode_into(v, out);
            }
        }
        Value::Bool(false) => out.push(0xf4),
        Value::Bool(true) => out.push(0xf5),
        Value::Null => out.push(0xf6),
    }
}

pub fn encode_text(s: &str, out: &mut Vec<u8>) {
    head(3, s.len() as u64, out);
    out.extend_from_slice(s.as_bytes());
}

/// Strictly decodes exactly one canonical value spanning all of `bytes`.
pub fn decode(bytes: &[u8]) -> Result<Value, CborError> {
    let mut d = Decoder { b: bytes, pos: 0 };
    let v = d.value(0)?;
    if d.pos != bytes.len() {
        return Err(CborError::Trailing);
    }
    Ok(v)
}

struct Decoder<'a> {
    b: &'a [u8],
    pos: usize,
}

impl Decoder<'_> {
    fn byte(&mut self) -> Result<u8, CborError> {
        let x = *self.b.get(self.pos).ok_or(CborError::Eof)?;
        self.pos += 1;
        Ok(x)
    }

    fn take(&mut self, n: u64) -> Result<&[u8], CborError> {
        let n = usize::try_from(n).map_err(|_| CborError::TooLong)?;
        if self.b.len() - self.pos < n {
            return Err(CborError::Eof);
        }
        let s = &self.b[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn arg(&mut self, info: u8) -> Result<u64, CborError> {
        let n = match info {
            0..=23 => return Ok(info as u64),
            24 => {
                let n = self.byte()? as u64;
                if n < 24 {
                    return Err(CborError::NotShortest);
                }
                n
            }
            25 => {
                let s = self.take(2)?;
                let n = u16::from_be_bytes([s[0], s[1]]) as u64;
                if n <= u8::MAX as u64 {
                    return Err(CborError::NotShortest);
                }
                n
            }
            26 => {
                let s = self.take(4)?;
                let n = u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as u64;
                if n <= u16::MAX as u64 {
                    return Err(CborError::NotShortest);
                }
                n
            }
            27 => {
                let s = self.take(8)?;
                let n = u64::from_be_bytes(s.try_into().expect("8 bytes"));
                if n <= u32::MAX as u64 {
                    return Err(CborError::NotShortest);
                }
                n
            }
            31 => return Err(CborError::Indefinite),
            _ => return Err(CborError::Simple),
        };
        Ok(n)
    }

    fn text(&mut self, n: u64) -> Result<String, CborError> {
        let s = self.take(n)?;
        String::from_utf8(s.to_vec()).map_err(|_| CborError::Utf8)
    }

    fn value(&mut self, depth: usize) -> Result<Value, CborError> {
        if depth > MAX_DEPTH {
            return Err(CborError::Depth);
        }
        let ib = self.byte()?;
        let major = ib >> 5;
        let info = ib & 0x1f;
        match major {
            0 => Ok(Value::U(self.arg(info)?)),
            1 => Ok(Value::N(self.arg(info)?)),
            2 => {
                let n = self.arg(info)?;
                Ok(Value::B(self.take(n)?.to_vec()))
            }
            3 => {
                let n = self.arg(info)?;
                Ok(Value::T(self.text(n)?))
            }
            4 => {
                let n = self.arg(info)?;
                // Every item needs at least one byte; reject absurd lengths before allocating.
                if n > (self.b.len() - self.pos) as u64 {
                    return Err(CborError::TooLong);
                }
                let mut items = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    items.push(self.value(depth + 1)?);
                }
                Ok(Value::A(items))
            }
            5 => {
                let n = self.arg(info)?;
                if n.saturating_mul(2) > (self.b.len() - self.pos) as u64 {
                    return Err(CborError::TooLong);
                }
                let mut entries: Vec<(String, Value)> = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    let kb = self.byte()?;
                    if kb >> 5 != 3 {
                        return Err(CborError::KeyNotText);
                    }
                    let kn = self.arg(kb & 0x1f)?;
                    let k = self.text(kn)?;
                    if let Some((prev, _)) = entries.last()
                        && key_cmp(prev, &k) != Ordering::Less
                    {
                        return Err(CborError::KeyOrder);
                    }
                    let v = self.value(depth + 1)?;
                    entries.push((k, v));
                }
                Ok(Value::M(entries))
            }
            6 => Err(CborError::Tag),
            7 => match info {
                20 => Ok(Value::Bool(false)),
                21 => Ok(Value::Bool(true)),
                22 => Ok(Value::Null),
                25..=27 => Err(CborError::Float),
                31 => Err(CborError::Indefinite),
                _ => Err(CborError::Simple),
            },
            _ => unreachable!("major type is 3 bits"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn known_encodings() {
        assert_eq!(Value::U(0).encode(), [0x00]);
        assert_eq!(Value::U(23).encode(), [0x17]);
        assert_eq!(Value::U(24).encode(), [0x18, 0x18]);
        assert_eq!(Value::U(1000).encode(), [0x19, 0x03, 0xe8]);
        assert_eq!(Value::int(-1).encode(), [0x20]);
        assert_eq!(Value::T("a".into()).encode(), [0x61, 0x61]);
        // length-first key order: "v" < "kind" < "nonce"
        let m = Value::map([("nonce", Value::U(1)), ("v", Value::U(1)), ("kind", Value::T("x".into()))]);
        let Value::M(e) = &m else { unreachable!() };
        assert_eq!(e.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["v", "kind", "nonce"]);
    }

    #[test]
    fn rejects_non_canonical() {
        assert_eq!(decode(&[0x18, 0x05]), Err(CborError::NotShortest));
        assert_eq!(decode(&[0x9f, 0xff]), Err(CborError::Indefinite));
        assert_eq!(decode(&[0xfb, 0, 0, 0, 0, 0, 0, 0, 0]), Err(CborError::Float));
        assert_eq!(decode(&[0xc1, 0x00]), Err(CborError::Tag));
        // {"b":1,"a":2} is out of order
        assert_eq!(decode(&[0xa2, 0x61, b'b', 0x01, 0x61, b'a', 0x02]), Err(CborError::KeyOrder));
        assert_eq!(decode(&[0x00, 0x00]), Err(CborError::Trailing));
        assert_eq!(decode(&[0xa1, 0x01, 0x01]), Err(CborError::KeyNotText));
    }

    fn arb_value() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            any::<u64>().prop_map(Value::U),
            any::<u64>().prop_map(Value::N),
            proptest::collection::vec(any::<u8>(), 0..40).prop_map(Value::B),
            ".{0,20}".prop_map(Value::T),
            any::<bool>().prop_map(Value::Bool),
            Just(Value::Null),
        ];
        leaf.prop_recursive(4, 64, 8, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 0..6).prop_map(Value::A),
                proptest::collection::btree_map("[a-z_]{1,10}", inner, 0..6).prop_map(Value::map),
            ]
        })
    }

    proptest! {
        #[test]
        fn roundtrip(v in arb_value()) {
            let b = v.encode();
            prop_assert_eq!(decode(&b).unwrap(), v);
        }

        #[test]
        fn decode_never_panics(b in proptest::collection::vec(any::<u8>(), 0..256)) {
            if let Ok(v) = decode(&b) {
                // Anything accepted must re-encode to the exact same bytes.
                prop_assert_eq!(v.encode(), b);
            }
        }
    }
}
