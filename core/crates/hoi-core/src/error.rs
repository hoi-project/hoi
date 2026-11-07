use serde::Serialize;

use hoi_validate::Reject;

/// What the UI sees when something fails: a code from `ErrorCode` in core-api and a sentence a
/// person can read.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(code: &str, message: impl Into<String>) -> ApiError {
        ApiError { code: code.to_string(), message: message.into() }
    }
    pub fn not_found(what: &str) -> ApiError {
        ApiError::new("not-found", format!("No {what} like that on this device."))
    }
    pub fn not_allowed(msg: impl Into<String>) -> ApiError {
        ApiError::new("not-allowed", msg)
    }
    pub fn invalid(msg: impl Into<String>) -> ApiError {
        ApiError::new("invalid", msg)
    }
    pub fn no_identity() -> ApiError {
        ApiError::new("no-identity", "Pick a name first. Your key is made on this device.")
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

impl From<Reject> for ApiError {
    fn from(r: Reject) -> ApiError {
        match r {
            Reject::RateLimited => ApiError::new("rate-limited", "Slow down a little. This community limits how often newcomers post."),
            Reject::Banned => ApiError::new("banned", "You're banned from this community."),
            Reject::TooBig => ApiError::new("too-big", "That's too long for one record."),
            Reject::Expired => ApiError::new("expired", "That has already faded."),
            Reject::Content(m) => ApiError::invalid(capitalize(&m)),
            Reject::UnknownCommunity => ApiError::not_found("community"),
            Reject::NotAllowed(m) => ApiError::not_allowed(capitalize(&m)),
            Reject::NoIdentity => ApiError::new("no-identity", "That key has no identity on this device yet."),
            Reject::WeakIdentity { have, need } => {
                ApiError::not_allowed(format!("This community asks for {need} bits of identity work; this key carries {have}."))
            }
            Reject::Burned => ApiError::not_allowed("That key was burned by its owner. Nothing it signs counts anymore."),
            other => ApiError::invalid(capitalize(&other.to_string())),
        }
    }
}

impl From<hoi_types::TypesError> for ApiError {
    fn from(e: hoi_types::TypesError) -> ApiError {
        ApiError::invalid(capitalize(&e.to_string()))
    }
}

impl From<hoi_types::cbor::CborError> for ApiError {
    fn from(_: hoi_types::cbor::CborError) -> ApiError {
        ApiError::invalid("That code or file is damaged.")
    }
}

impl From<hoi_crypto::CryptoError> for ApiError {
    fn from(e: hoi_crypto::CryptoError) -> ApiError {
        match e {
            hoi_crypto::CryptoError::Open => ApiError::new("bad-passphrase", "Wrong passphrase, or the file is damaged."),
            other => ApiError::new("internal", other.to_string()),
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str() + if s.ends_with('.') { "" } else { "." },
        None => String::new(),
    }
}
