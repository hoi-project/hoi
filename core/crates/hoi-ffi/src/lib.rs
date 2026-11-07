//! hoi-ffi: hoi-core for the native phone apps (SwiftUI on iOS, Jetpack Compose on Android),
//! through UniFFI. See apps/mobile/README.md for how the pieces fit.
//!
//! - [`HoiNode`]: one per app process. Opens the node in the app's data directory (SQLite at
//!   `<data_dir>/hoi.db`), owns a small tokio runtime for the home-node link and the expiry
//!   sweeper, and exposes the whole `HoiCore` API (packages/core-api) as typed methods.
//! - [`KeyStore`]: implemented by the app (Keychain on iOS, Android Keystore on Android). A
//!   random data key goes there; SQLite holds only encrypted record and secret-key payloads.
//! - [`EventListener`]: implemented by the app. Gets every core event as JSON, shaped like
//!   `CoreEvent` in packages/core-api: `changed`, `notice`, `net` and `stamp`.
//!
//! Every method blocks the calling thread: storage, mining and validation run in place. Call
//! from a background thread (Swift: a detached `Task`; Kotlin: `Dispatchers.IO`).

uniffi::setup_scaffolding!();

mod events;
mod keys;
mod link;
mod node;

#[cfg(test)]
mod tests;

pub use node::HoiNode;

/// Why a call failed.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum HoiError {
    /// `code` is one of core-api's `ErrorCode`s: `no-identity`, `not-found`, `not-allowed`,
    /// `rate-limited`, `invalid`, `bad-invite`, `bad-passphrase`, `expired`, `banned`,
    /// `too-big`, `offline`, `internal`. `message` is a sentence for people: show it as is.
    /// (Kotlin calls it `reason`, because exceptions already have a `message`.)
    Api { code: String, message: String },
}

impl HoiError {
    pub(crate) fn new(code: &str, message: impl Into<String>) -> HoiError {
        HoiError::Api { code: code.to_string(), message: message.into() }
    }

    pub fn code(&self) -> &str {
        match self {
            HoiError::Api { code, .. } => code,
        }
    }
}

impl std::fmt::Display for HoiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HoiError::Api { message, .. } => f.write_str(message),
        }
    }
}

impl std::error::Error for HoiError {}

impl From<hoi_core::ApiError> for HoiError {
    fn from(e: hoi_core::ApiError) -> HoiError {
        HoiError::Api { code: e.code, message: e.message }
    }
}

/// Protects the key that encrypts all local database payloads.
///
/// The core stores a 32-byte random database key under `"vault-key-v1"`; legacy `"keys"`
/// entries are migrated. Identity, MLS and record payloads are encrypted in SQLite.
/// iOS: a Keychain generic password with
/// `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`, so background refresh can read it and it
/// never leaves the device in backups. Android: the blob encrypted with an AES-GCM key held by
/// the Android Keystore, stored in the app's private files.
///
/// Called on background threads, sometimes while a call is in flight: implementations must be
/// thread-safe and must not call back into [`HoiNode`].
#[uniffi::export(with_foreign)]
pub trait KeyStore: Send + Sync {
    /// The stored value, or nothing if there is none (or it can't be read).
    fn get(&self, key: String) -> Option<Vec<u8>>;
    /// Stores `value`, or deletes the entry when it is nothing.
    fn set(&self, key: String, value: Option<Vec<u8>>);
}

/// Receives core events as JSON strings, one per call:
///
/// - `{"type":"changed","communities":["<hex>",…]}`: records arrived, expired or were removed.
///   Refresh what's on screen for those communities.
/// - `{"type":"notice","text":"…"}`: something worth a toast.
/// - `{"type":"net","status":{…NetStatus…}}`: the home-node link changed.
/// - `{"type":"stamp","phase":"start","bits":N,"estimateMs":N}` and
///   `{"type":"stamp","phase":"done","bits":N,"estimateMs":N,"tookMs":N}`: proof-of-work around
///   `create_post`, `reply` and `edit`, for the work bar.
///
/// Called on background threads. Hop to the main thread asynchronously (Swift:
/// `Task { @MainActor in … }`, Kotlin: a `MutableSharedFlow` or `Dispatchers.Main`) and never
/// block waiting for the main thread here.
#[uniffi::export(with_foreign)]
pub trait EventListener: Send + Sync {
    fn on_event(&self, json: String);
}
