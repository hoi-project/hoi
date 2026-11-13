//! hoi-types: records, ids, canonical encoding and URIs. Pure, no I/O (CONTRIBUTING.md).

pub mod cbor;
pub mod declaration;
pub mod did;
pub mod id;
pub mod notice;
pub mod record;
pub mod scatter;
pub mod uri;

pub use declaration::NodeDeclaration;
pub use did::{did_key, parse_did_key, parse_key};
pub use id::{Id, Sig, hex_decode, hex_encode};
pub use notice::NodeNotice;
pub use record::*;
pub use uri::{Invite, NodeLink, Pair};

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TypesError {
    #[error("cbor: {0}")]
    Cbor(#[from] cbor::CborError),
    #[error("expected {want} bytes, got {got}")]
    Length { want: usize, got: usize },
    #[error("bad hex")]
    Hex,
    #[error("bad did:key")]
    Did,
    #[error("{0}")]
    Field(String),
    #[error("unsupported protocol version {0}")]
    Version(u64),
    #[error("{0}")]
    Uri(String),
}
