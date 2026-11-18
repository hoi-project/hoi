//! RFC-0009. A member archive holds the core's logical storage, inside the same
//! authenticated passphrase envelope as a key file. Local vault headers are host-owned.
use super::{Clock, Node};
use crate::{ApiError, api::R, dto::Me};
use hoi_crypto::keystore;
use hoi_store::{Change, Loaded, Persist};
use hoi_types::Wire;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

const MAGIC: &[u8] = b"hoi-member-snapshot\0\x01";
const ENVELOPE: usize = 70; // HOIK header plus authentication tag
const MAX_ROWS: usize = 131_072;

fn invalid() -> ApiError {
    ApiError::invalid("This file is not a supported member snapshot.")
}
fn too_large() -> ApiError {
    ApiError::invalid("Member snapshots support files up to 32 MiB and 131072 entries of either kind.")
}

fn push_field(out: &mut Vec<u8>, bytes: &[u8]) -> R<()> {
    if bytes.len().saturating_add(out.len()).saturating_add(4 + ENVELOPE) > Node::MAX_SNAPSHOT {
        return Err(too_large());
    }
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}
fn number(input: &mut &[u8]) -> R<usize> {
    let bytes = input.split_off(..4).ok_or_else(invalid)?;
    Ok(u32::from_le_bytes(bytes.try_into().map_err(|_| invalid())?) as usize)
}
fn field<'a>(input: &mut &'a [u8]) -> R<&'a [u8]> {
    let size = number(input)?;
    input.split_off(..size).ok_or_else(invalid)
}
fn entries(input: &mut &[u8]) -> R<usize> {
    let n = number(input)?;
    if n > MAX_ROWS || n > input.len() / 4 { Err(invalid()) } else { Ok(n) }
}

fn read_archive(bytes: &[u8]) -> R<Loaded> {
    let mut input = bytes.strip_prefix(MAGIC).ok_or_else(invalid)?;
    let mut data = Loaded::default();
    for _ in 0..entries(&mut input)? {
        data.records.push(field(&mut input)?.to_vec());
    }
    let mut keys = BTreeSet::new();
    for _ in 0..entries(&mut input)? {
        let key = std::str::from_utf8(field(&mut input)?).map_err(|_| invalid())?;
        if key.is_empty() || key.len() > 256 || !keys.insert(key) {
            return Err(invalid());
        }
        data.blobs.push((key.to_owned(), field(&mut input)?.to_vec()));
    }
    if !input.is_empty() || !["keys", "state", "format"].iter().all(|key| keys.contains(key)) {
        return Err(invalid());
    }
    Ok(data)
}

// Restore against an isolated storage session. The real vault sees one ordered
// transaction, only after replay, deletion catch-up preparation and cache disposal.
struct Staging {
    before: Option<Loaded>,
    writes: Arc<Mutex<Vec<Change>>>,
}
impl Persist for Staging {
    fn load(&mut self) -> Result<Loaded, String> {
        self.before.take().ok_or_else(|| "Snapshot staging was already opened.".into())
    }
    fn apply(&mut self, changes: &[Change]) -> Result<(), String> {
        self.writes.lock().map_err(|_| "Snapshot staging failed.")?.extend_from_slice(changes);
        Ok(())
    }
}
impl Node {
    fn check_snapshot_identity(&self) -> R<()> {
        let identity = self.need_me()?;
        if self.device_kp.as_ref().is_none_or(|key| key.public() != identity.device)
            || self.root_kp.as_ref().is_some_and(|key| key.public() != self.store.current_root(&identity.root))
            || identity.burned
        {
            return Err(ApiError::not_allowed("Only an active member device holding its signing key can use a member snapshot."));
        }
        Ok(())
    }

    pub const MAX_SNAPSHOT: usize = 32 * 1024 * 1024;

    /// Browser workers supply the decrypted logical database from their existing vault;
    /// the UI receives only the resulting encrypted file.
    pub fn seal_member_snapshot(&self, loaded: &Loaded, password: &str) -> R<Vec<u8>> {
        self.check_snapshot_identity()?;
        if password.chars().count() < 15 || password.len() > 1024 {
            return Err(ApiError::invalid("Use a backup passphrase of at least 15 characters, up to 1024 bytes."));
        }
        if loaded.records.len() > MAX_ROWS || loaded.blobs.len() > MAX_ROWS {
            return Err(too_large());
        }
        if !["keys", "state", "format"].iter().all(|key| loaded.blobs.iter().any(|(name, _)| name == key)) {
            return Err(ApiError::invalid("Member storage is unavailable for export."));
        }
        let mut plain = Zeroizing::new(MAGIC.to_vec());
        plain.extend_from_slice(&(loaded.records.len() as u32).to_le_bytes());
        for wire in &loaded.records {
            push_field(&mut plain, wire)?;
        }
        plain.extend_from_slice(&(loaded.blobs.len() as u32).to_le_bytes());
        for (key, value) in &loaded.blobs {
            push_field(&mut plain, key.as_bytes())?;
            push_field(&mut plain, value)?;
        }
        Ok(keystore::seal(&plain, password, keystore::Cost::default())?)
    }
    pub fn export_snapshot(&mut self, password: &str) -> R<Vec<u8>> {
        self.commit()?;
        let loaded = self.persist.load().map_err(|_| ApiError::new("internal", "Member storage could not be read."))?;
        self.seal_member_snapshot(&loaded, password)
    }
    pub fn import_snapshot(&mut self, file: &[u8], password: &str) -> R<Me> {
        if self.state.me.is_some() || !self.state.joined.is_empty() || self.store.all().next().is_some() {
            return Err(ApiError::not_allowed("Restore a member snapshot on a fresh installation."));
        }
        if file.len() > Self::MAX_SNAPSHOT {
            return Err(too_large());
        }
        if password.len() > 1024 {
            return Err(invalid());
        }
        let loaded = read_archive(&Zeroizing::new(keystore::open(file, password)?))?;
        let mut transaction = vec![Change::Wipe];
        let mut ids = BTreeSet::new();
        for wire in &loaded.records {
            let (record, id) = Wire::decode(wire).map_err(|_| invalid())?;
            if !ids.insert(id) {
                return Err(invalid());
            }
            transaction.push(Change::Put { id, community: record.record.community, expires_at: record.record.expires_at, wire: wire.clone() });
        }
        transaction.extend(loaded.blobs.iter().map(|(key, value)| Change::Blob { key: key.clone(), value: Some(value.clone()) }));
        let writes = Arc::new(Mutex::new(Vec::new()));
        let store = Staging { before: Some(loaded), writes: writes.clone() };
        let now = self.now();
        let clock: Clock = Box::new(move || now);
        let mut staged = Self::open_restored(Box::new(store), &self.platform, &self.web_base, clock)?;
        staged.check_snapshot_identity()?;

        // Pairing credentials, transport identities and network permissions belong to
        // the installation. Bringing an archive to a phone must not enable a public node.
        staged.state.home = None;
        staged.state.paired.clear();
        staged.state.link_key = None;
        staged.state.serve_token = None;
        staged.state.node_id = self.state.node_id;
        staged.state.notices.clear();
        staged.remote.keys.clear();
        staged.transport_kp = self.transport_kp.as_ref().map(|k| hoi_crypto::Keypair::from_secret(&k.secret()));
        staged.rtc_cert.clone_from(&self.rtc_cert);
        staged.state.settings.relay = self.state.settings.relay;
        staged.state.settings.public_node = false;
        staged.state.settings.hold_pieces_gb = 0.;
        staged.state.settings.direct_links = false;
        staged.state.settings.share_holders = false;
        staged.state.settings.vpn.clone_from(&self.state.settings.vpn);
        staged.state.settings.home_network = self.state.settings.home_network;
        staged.state.settings.relay_notice_seen = self.state.settings.relay_notice_seen;
        staged.mark_state();
        staged.mark_keys();
        staged.commit()?;
        let me = staged.me().ok_or_else(invalid)?;
        transaction.append(&mut *writes.lock().map_err(|_| invalid())?);
        self.persist.apply(&transaction).map_err(|_| ApiError::new("internal", "The member snapshot could not be saved."))?;
        std::mem::swap(&mut staged.persist, &mut self.persist);
        std::mem::swap(&mut staged.clock, &mut self.clock);
        staged.consent_gate = self.consent_gate;
        staged.look_elsewhere = self.look_elsewhere;
        staged.identity_bits = self.identity_bits;
        staged.node_name = self.node_name.take();
        staged.carry_policy = self.carry_policy;
        staged.carry_limit = self.carry_limit;
        staged.carry_bytes = self.carry_bytes;
        staged.scatter.pieces = self.scatter.pieces.take();
        staged.software = self.software.take();
        staged.operator_notice = self.operator_notice.take();
        staged.operator_declaration = self.operator_declaration.take();
        *self = staged;
        Ok(me)
    }
}
