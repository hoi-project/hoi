//! hoi-core: the one crate that does all the thinking (TECHNICAL.md §2). UIs call [`Node`]
//! (directly, through [`Node::call`] JSON dispatch, through hoi-wasm or hoi-ffi); they never
//! touch keys, storage or the network themselves.

pub mod age;
pub mod api;
pub mod assurance;
pub mod bind;
pub mod blobs;
mod blocklists;
mod bulk;
pub mod carry;
mod cleanup;
mod deletion;
mod directory;
pub mod dispatch;
mod drop_share;
pub mod dto;
pub mod error;
pub mod find;
mod forward;
pub mod handle;
mod home_scatter;
mod inject;
mod keep;
mod labelers;
pub mod lan;
pub mod link;
pub mod look;
mod mailbox;
pub mod net;
pub mod node;
mod nodes;
pub mod notice;
mod open;
mod pickup;
pub mod preview;
pub mod private;
mod profile;
mod profile_color;
mod public_join;
mod rebuild;
pub mod recover;
mod registration;
pub mod remote;
mod scatter;
pub mod signal;
pub mod state;
pub mod suppress;
mod tally;
pub mod traffic;
mod trust;
mod views;
mod warmth;
mod wiki;
mod work;

#[cfg(test)]
mod tests;

// The DTOs become UniFFI records for hoi-ffi (Swift, Kotlin).
#[cfg(feature = "ffi")]
uniffi::setup_scaffolding!();

pub use api::{Published, VERSION};
pub use carry::CarryPolicy;
pub use error::ApiError;
pub use net::{PeerMeta, Role};
pub use node::{Clock, Node, Source};
pub use registration::{
    community_publication_statement, community_registration_name, community_registration_statement, node_announcement_statement, registry_origin_ok,
    release_manifest, release_versions,
};
pub use scatter::ScatterOutcome;
pub use work::IdentityWork;

/// Wall clock in Unix seconds for native hosts.
#[cfg(not(target_arch = "wasm32"))]
pub fn system_clock() -> Clock {
    Box::new(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
}
