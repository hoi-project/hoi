//! hoi-sim: a harness for pentesting the hoi protocol and showing how it holds.
//!
//! Three ways in:
//!   - [`scenario::siege`] runs a whole battle in one process over accelerated time and scores the
//!     protocol's invariants — no network, no Docker, good for CI and a quick look.
//!   - [`forge`] builds the raw adversarial records ("bad signature", "backdated vouch", …) that
//!     both the battle and the live agent use to attack.
//!   - [`scatter`] places generations of pieces with the core's rules among places a party runs a
//!     share of, and sets how often it could rebuild or withhold against the exact arithmetic.
//!   - [`wire`] is a one-shot WebSocket client: an agent (honest or hostile) dials a running hub,
//!     syncs, does one thing, syncs again, and leaves. This is what drives the Docker sandbox,
//!     where many independent sessions each run their own agent.
//!
//! Nothing here is part of a release build; it depends on `hoi-core` like any other host.

pub mod address;
pub mod forge;
pub mod ladder;
pub mod life;
pub mod load;
pub mod report;
pub mod scatter;
pub mod scenario;
pub mod wire;

/// The web app address a node reports in links. Not used over the wire; any value is fine here.
pub const WEB_BASE: &str = "https://hoi.sh/app/";
