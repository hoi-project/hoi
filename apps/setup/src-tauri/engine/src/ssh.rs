//! One SSH session (russh): the server's host key is shown once and pinned from then on, a
//! changed key stops everything. Authentication by the agent, a key file (with its passphrase)
//! or a password typed for this session; nothing is written anywhere. Commands run with their
//! output streamed line by line; with a pty, sudo's password prompt is answered once.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Handle};
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate, load_secret_key};
use russh::{ChannelMsg, Disconnect};
use serde::{Deserialize, Serialize};

/// Where to connect.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Target {
    pub host: String,
    pub port: u16,
    pub user: String,
}

/// How to prove who we are. A password lives in memory for this session only.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Auth {
    Agent,
    Key { path: String, passphrase: Option<String> },
    Password { password: String },
}

/// The server's host key as people compare it: `ssh-ed25519` and `SHA256:…`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostKey {
    pub algorithm: String,
    pub fingerprint: String,
}

/// Which stream a line came from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    Out,
    Err,
}

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("could not reach {0}")]
    Unreachable(String),
    #[error("the server did not answer in time")]
    Timeout,
    #[error("the server's host key changed: expected {expected}, got {got} ({algorithm}). Stop here unless you know why (a reinstalled server, say), then forget this server and start again")]
    HostKeyChanged { expected: String, got: String, algorithm: String },
    #[error("the server sent no host key")]
    NoHostKey,
    #[error("{0}")]
    Auth(String),
    #[error("the key file: {0}")]
    Key(String),
    #[error("ssh: {0}")]
    Ssh(#[from] russh::Error),
    #[error("{0}")]
    Upload(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Why an RSA key file is refused. The engine is built without russh's `rsa` feature: the `rsa`
/// crate leaks private-key timing (RUSTSEC-2023-0071, the Marvin attack) and has no fix.
pub const RSA_UNSUPPORTED: &str =
    "RSA keys are not supported (the RSA implementation leaks timing, RUSTSEC-2023-0071). Use an Ed25519 key (ssh-keygen -t ed25519) or a password";

/// Loads a private key file, refusing RSA before anything is signed with it.
pub fn load_key(path: &str, passphrase: Option<&str>) -> Result<russh::keys::PrivateKey, SetupError> {
    checked_key(path, load_secret_key(path, passphrase))
}

fn checked_key(path: &str, loaded: Result<russh::keys::PrivateKey, russh::keys::Error>) -> Result<russh::keys::PrivateKey, SetupError> {
    match loaded {
        // An OpenSSH-format RSA file still loads; signing with it would fail later, less clearly.
        Ok(key) if key.algorithm().is_rsa() => Err(SetupError::Key(format!("{path}: {RSA_UNSUPPORTED}"))),
        Ok(key) => Ok(key),
        // A PEM-format RSA file ("BEGIN RSA PRIVATE KEY") is not understood at all.
        Err(russh::keys::Error::UnsupportedKeyType { key_type_string, .. }) if key_type_string.eq_ignore_ascii_case("rsa") => {
            Err(SetupError::Key(format!("{path}: {RSA_UNSUPPORTED}")))
        }
        Err(e) => Err(SetupError::Key(format!("{path}: {e}"))),
    }
}

/// A word for the shell: single quotes around it, a quote inside doubled the shell's way.
pub fn quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// Whether the output so far ends in a password prompt (sudo's, or a plain `Password:`).
pub fn ends_with_password_prompt(tail: &str) -> bool {
    let t = tail.trim_end_matches([' ', '\u{7}']);
    let lower = t.to_ascii_lowercase();
    lower.ends_with("password:") || (lower.contains("[sudo] password for") && lower.ends_with(':'))
}

enum Trust {
    /// Record the key, accept nothing: a first look at a server.
    Probe,
    /// Accept this fingerprint, nothing else.
    Pinned(String),
}

struct Handler {
    trust: Trust,
    seen: Arc<Mutex<Option<HostKey>>>,
}

fn host_key_of(key: &PublicKeyOrCertificate) -> HostKey {
    match key {
        PublicKeyOrCertificate::PublicKey { key, .. } => {
            HostKey { algorithm: key.algorithm().to_string(), fingerprint: key.fingerprint(HashAlg::Sha256).to_string() }
        }
        PublicKeyOrCertificate::Certificate(cert) => HostKey {
            algorithm: format!("{} (certificate)", cert.algorithm()),
            fingerprint: cert.public_key().fingerprint(HashAlg::Sha256).to_string(),
        },
    }
}

impl client::Handler for Handler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let seen = host_key_of(key);
        let ok = match &self.trust {
            Trust::Probe => false,
            Trust::Pinned(fingerprint) => *fingerprint == seen.fingerprint,
        };
        *self.seen.lock().expect("host key slot") = Some(seen);
        Ok(ok)
    }
}

fn config() -> Arc<client::Config> {
    Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(600)),
        keepalive_interval: Some(Duration::from_secs(15)),
        keepalive_max: 4,
        ..Default::default()
    })
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(25);

/// What an upload writes at a time, so the window can show how far it is.
const UPLOAD_PIECE: usize = 256 * 1024;

async fn dial(target: &Target, trust: Trust) -> (Result<Handle<Handler>, SetupError>, Option<HostKey>) {
    let seen = Arc::new(Mutex::new(None));
    let handler = Handler { trust, seen: seen.clone() };
    let result = match tokio::time::timeout(CONNECT_TIMEOUT, client::connect(config(), (target.host.as_str(), target.port), handler)).await {
        Err(_) => Err(SetupError::Timeout),
        Ok(Err(russh::Error::IO(e))) => Err(SetupError::Unreachable(format!("{}:{} ({e})", target.host, target.port))),
        Ok(Err(e)) => Err(SetupError::Ssh(e)),
        Ok(Ok(handle)) => Ok(handle),
    };
    let seen = seen.lock().expect("host key slot").clone();
    (result, seen)
}

/// A first look at a server: its host key, for the person to compare and then pin. Nothing is
/// trusted and nobody is authenticated.
pub async fn probe_host_key(target: &Target) -> Result<HostKey, SetupError> {
    let (result, seen) = dial(target, Trust::Probe).await;
    if let Some(key) = seen {
        if let Ok(handle) = result {
            let _ = handle.disconnect(Disconnect::ByApplication, "", "English").await;
        }
        return Ok(key);
    }
    match result {
        Err(e) => Err(e),
        Ok(_) => Err(SetupError::NoHostKey),
    }
}

#[cfg(windows)]
async fn agent() -> Result<AgentClient<Box<dyn russh::keys::agent::client::AgentStream + Send + Unpin>>, SetupError> {
    AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent")
        .await
        .map(|a| a.dynamic())
        .map_err(|e| SetupError::Auth(format!("no ssh-agent on this computer (Windows' OpenSSH Authentication Agent service): {e}")))
}

#[cfg(not(windows))]
async fn agent() -> Result<AgentClient<Box<dyn russh::keys::agent::client::AgentStream + Send + Unpin>>, SetupError> {
    AgentClient::connect_env().await.map(|a| a.dynamic()).map_err(|e| SetupError::Auth(format!("no ssh-agent on this computer (SSH_AUTH_SOCK): {e}")))
}

/// A connection to one server, authenticated, with its host key pinned.
pub struct Session {
    handle: Handle<Handler>,
    pub target: Target,
}

impl Session {
    /// Connects to `target` with the host key the person pinned, and authenticates with `auth`.
    pub async fn connect(target: Target, auth: Auth, pinned: &str) -> Result<Session, SetupError> {
        let (result, seen) = dial(&target, Trust::Pinned(pinned.to_string())).await;
        let mut handle = match result {
            Ok(handle) => handle,
            Err(SetupError::Ssh(russh::Error::UnknownKey)) => {
                let got = seen.unwrap_or(HostKey { algorithm: "?".into(), fingerprint: "?".into() });
                return Err(SetupError::HostKeyChanged { expected: pinned.to_string(), got: got.fingerprint, algorithm: got.algorithm });
            }
            Err(e) => return Err(e),
        };
        let user = target.user.clone();
        let ok = match auth {
            Auth::Password { password } => handle.authenticate_password(user, password).await?.success(),
            Auth::Key { path, passphrase } => {
                let key = load_key(&path, passphrase.as_deref())?;
                let hash = handle.best_supported_rsa_hash().await?.flatten();
                handle.authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash)).await?.success()
            }
            Auth::Agent => {
                let mut agent = agent().await?;
                let identities = agent.request_identities().await.map_err(|e| SetupError::Auth(format!("the agent lists no keys: {e}")))?;
                let hash = handle.best_supported_rsa_hash().await?.flatten();
                let mut ok = false;
                for identity in identities {
                    if let AgentIdentity::PublicKey { key, .. } = identity
                        && handle.authenticate_publickey_with(user.clone(), key, hash, &mut agent).await.map(|r| r.success()).unwrap_or(false)
                    {
                        ok = true;
                        break;
                    }
                }
                ok
            }
        };
        if !ok {
            let _ = handle.disconnect(Disconnect::ByApplication, "", "English").await;
            return Err(SetupError::Auth(format!("{}@{} did not accept that login", target.user, target.host)));
        }
        Ok(Session { handle, target })
    }

    /// Runs `command` and streams its lines; returns the exit status. With `pty` the command may
    /// ask for a password (sudo), and `answer` is typed once when it does; a second prompt means
    /// the answer was wrong and ends the run.
    pub async fn exec(&self, command: &str, pty: bool, stdin: Option<Vec<u8>>, answer: Option<&str>, on_line: impl FnMut(Stream, String)) -> Result<u32, SetupError> {
        self.exec_with(command, pty, stdin.as_deref(), answer, on_line, |_, _| {}).await
    }

    /// `exec`, with `stdin` written in pieces and `on_sent(sent, total)` called after each.
    async fn exec_with(
        &self,
        command: &str,
        pty: bool,
        stdin: Option<&[u8]>,
        answer: Option<&str>,
        mut on_line: impl FnMut(Stream, String),
        mut on_sent: impl FnMut(u64, u64),
    ) -> Result<u32, SetupError> {
        let mut channel = self.handle.channel_open_session().await?;
        if pty {
            channel.request_pty(false, "xterm", 160, 48, 0, 0, &[]).await?;
        }
        channel.exec(true, command).await?;
        if let Some(bytes) = stdin {
            let total = bytes.len() as u64;
            let mut sent = 0u64;
            for piece in bytes.chunks(UPLOAD_PIECE) {
                channel.data(piece).await?;
                sent += piece.len() as u64;
                on_sent(sent, total);
            }
            channel.eof().await?;
        }
        let mut out = String::new();
        let mut err = String::new();
        let mut answered = false;
        let mut code = None;
        loop {
            let Some(msg) = channel.wait().await else { break };
            match msg {
                ChannelMsg::Data { data } => {
                    out.push_str(&String::from_utf8_lossy(&data));
                    drain(&mut out, Stream::Out, &mut on_line);
                    if pty && ends_with_password_prompt(&out) {
                        match answer {
                            Some(password) if !answered => {
                                answered = true;
                                channel.data(format!("{password}\n").as_bytes()).await?;
                                out.push('\n');
                                drain(&mut out, Stream::Out, &mut on_line);
                            }
                            _ => {
                                on_line(Stream::Err, if answered { "the password was not accepted".into() } else { "a password is asked and none was given".into() });
                                let _ = channel.eof().await;
                                let _ = channel.close().await;
                                return Ok(255);
                            }
                        }
                    }
                }
                ChannelMsg::ExtendedData { data, .. } => {
                    err.push_str(&String::from_utf8_lossy(&data));
                    drain(&mut err, Stream::Err, &mut on_line);
                }
                ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
                ChannelMsg::Eof | ChannelMsg::Close if code.is_some() => break,
                _ => {}
            }
        }
        if !out.trim().is_empty() {
            on_line(Stream::Out, std::mem::take(&mut out));
        }
        if !err.trim().is_empty() {
            on_line(Stream::Err, std::mem::take(&mut err));
        }
        Ok(code.unwrap_or(255))
    }

    /// Runs `command` and returns everything it printed, for reading a file or a status.
    pub async fn read(&self, command: &str) -> Result<(u32, String), SetupError> {
        let mut text = String::new();
        let code = self.exec(command, false, None, None, |_, line| {
            text.push_str(&line);
            text.push('\n');
        })
        .await?;
        Ok((code, text))
    }

    /// Writes `bytes` to `path` on the server through this session, readable by its owner only
    /// until `mode` says otherwise. No scp, no sftp: `cat` behind the same login.
    pub async fn upload(&self, path: &str, bytes: &[u8], mode: &str) -> Result<(), SetupError> {
        let command = format!("umask 077 && cat > {p} && chmod {m} {p}", p = quote(path), m = quote(mode));
        let mut said = String::new();
        let code = self.exec(&command, false, Some(bytes.to_vec()), None, |_, line| {
            said.push_str(&line);
            said.push('\n');
        })
        .await?;
        if code != 0 {
            return Err(SetupError::Auth(format!("could not write {path} on the server: {}", said.trim())));
        }
        Ok(())
    }

    /// Writes a file from this computer to `path` on the server (`upload`, in pieces, with
    /// `progress(sent, total)`), then has the server hash what arrived and compares it with this
    /// computer's hash. Returns that SHA-256.
    pub async fn upload_verified(&self, path: &str, bytes: &[u8], mode: &str, progress: impl FnMut(u64, u64)) -> Result<String, SetupError> {
        let command = format!("umask 077 && cat > {p} && chmod {m} {p}", p = quote(path), m = quote(mode));
        let mut said = String::new();
        let code = self
            .exec_with(
                &command,
                false,
                Some(bytes),
                None,
                |_, line| {
                    said.push_str(&line);
                    said.push('\n');
                },
                progress,
            )
            .await?;
        if code != 0 {
            return Err(SetupError::Upload(format!("could not write {path} on the server: {}", said.trim())));
        }
        let want = crate::node_files::sha256_hex(bytes);
        let (code, text) = self.read(&format!("sha256sum -- {}", quote(path))).await?;
        let got = text.split_whitespace().next().unwrap_or_default();
        if code != 0 {
            return Err(SetupError::Upload(format!("could not check {path} on the server (sha256sum): {}", text.trim())));
        }
        if got != want {
            return Err(SetupError::Upload(format!("{path} arrived damaged (SHA-256 {got}, sent {want}); upload it again")));
        }
        Ok(want)
    }

    pub async fn close(self) {
        let _ = self.handle.disconnect(Disconnect::ByApplication, "", "English").await;
    }
}

/// Hands complete lines to `on_line`, keeping a partial last line (a prompt) in `buffer`.
fn drain(buffer: &mut String, stream: Stream, on_line: &mut impl FnMut(Stream, String)) {
    while let Some(i) = buffer.find('\n') {
        let line: String = buffer.drain(..=i).collect();
        on_line(stream, line.trim_end_matches(['\n', '\r']).to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_quoted_for_the_shell() {
        assert_eq!(quote("plain"), "'plain'");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote("$(touch x); `y`"), "'$(touch x); `y`'");
    }

    #[test]
    fn password_prompts_are_seen_at_the_end_of_the_output() {
        assert!(ends_with_password_prompt("[sudo] password for lena: "));
        assert!(ends_with_password_prompt("Checking…\nPassword:"));
        assert!(ends_with_password_prompt("[sudo] password for root:\u{7}"));
        assert!(!ends_with_password_prompt("password: was typed\n"));
        assert!(!ends_with_password_prompt("  ok     rights      sudo (the install asks for your password)\n"));
    }

    #[test]
    fn lines_are_handed_over_whole_and_prompts_kept() {
        let mut got = Vec::new();
        let mut buffer = String::from("one\r\ntwo\n[sudo] password for lena: ");
        drain(&mut buffer, Stream::Out, &mut |s, l| got.push((s, l)));
        assert_eq!(got, vec![(Stream::Out, "one".to_string()), (Stream::Out, "two".to_string())]);
        assert_eq!(buffer, "[sudo] password for lena: ");
    }

    #[test]
    fn rsa_key_files_are_refused_with_advice_and_others_load() {
        let ed25519 = russh::keys::PrivateKey::from(russh::keys::ssh_key::private::Ed25519Keypair::from_seed(&[7; 32]));
        assert!(checked_key("id_ed25519", Ok(ed25519)).is_ok());
        let pem = russh::keys::Error::UnsupportedKeyType { key_type_string: "RSA".into(), key_type_raw: Vec::new() };
        let err = checked_key("id_rsa", Err(pem)).unwrap_err().to_string();
        assert!(err.contains("id_rsa") && err.contains("RSA keys are not supported") && err.contains("ssh-keygen -t ed25519"), "{err}");
        let other = russh::keys::Error::UnsupportedKeyType { key_type_string: "DSA".into(), key_type_raw: Vec::new() };
        assert!(!checked_key("id_dsa", Err(other)).unwrap_err().to_string().contains("RSA keys"));
    }

    #[tokio::test]
    async fn a_closed_port_is_unreachable_and_says_so() {
        let target = Target { host: "127.0.0.1".into(), port: 9, user: "root".into() };
        let err = probe_host_key(&target).await.unwrap_err();
        assert!(matches!(err, SetupError::Unreachable(_) | SetupError::Timeout | SetupError::Ssh(_)), "{err}");
    }
}
