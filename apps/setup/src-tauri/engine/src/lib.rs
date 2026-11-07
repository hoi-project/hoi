//! The engine of the hoi node setup app: one SSH session per server with the host
//! key pinned, files uploaded through that session (a hoi-node built on this computer too,
//! checked on arrival), commands run with their output streamed and sudo's password prompt
//! answered, servers remembered without their passwords, and the same steps on this computer.
//! Nothing here knows the wizard; the app's window drives it.

pub mod local;
pub mod node_files;
pub mod profiles;
pub mod ssh;

pub use local::local_exec;
pub use node_files::NodeFile;
pub use profiles::{AuthKind, Profile, Profiles};
pub use ssh::{Auth, HostKey, Session, SetupError, Stream, Target, probe_host_key, quote};
