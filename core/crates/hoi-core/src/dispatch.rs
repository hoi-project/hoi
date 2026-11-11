//! One JSON entry point for hosts that speak JSON (the web worker, the Tauri shell):
//! `call("createPost", [draft])`. Method names and argument order match `HoiCore` in
//! packages/core-api. Bytes travel as base64url strings.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use hoi_types::uri::{b64, unb64};

use crate::api::{Published, R};
use crate::dto::*;
use crate::error::ApiError;
use crate::node::Node;

struct Args(Vec<Value>);

impl Args {
    fn get<T: DeserializeOwned>(&self, i: usize, name: &str) -> R<T> {
        let v = self.0.get(i).cloned().unwrap_or(Value::Null);
        serde_json::from_value(v).map_err(|e| ApiError::invalid(format!("argument {name}: {e}")))
    }
    fn str(&self, i: usize, name: &str) -> R<String> {
        self.get::<String>(i, name)
    }
    fn strs(&self, i: usize, name: &str) -> R<Vec<String>> {
        self.get::<Vec<String>>(i, name)
    }
    fn opt_str(&self, i: usize) -> Option<String> {
        self.0.get(i).and_then(|v| v.as_str()).map(String::from)
    }
    fn bytes(&self, i: usize, name: &str) -> R<Vec<u8>> {
        unb64(&self.str(i, name)?).map_err(|_| ApiError::invalid(format!("argument {name}: expected base64url")))
    }
    /// Optional bytes: absent or null is empty.
    fn opt_bytes(&self, i: usize, name: &str) -> R<Vec<u8>> {
        match self.0.get(i) {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(_) => self.bytes(i, name),
        }
    }
}

fn ok<T: Serialize>(v: T) -> R<Value> {
    serde_json::to_value(v).map_err(|e| ApiError::new("internal", e.to_string()))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FeedQuery {
    community: Option<String>,
    sort: Option<String>,
    limit: Option<usize>,
}

impl Node {
    /// Calls one API method with positional JSON arguments, then persists.
    pub fn call(&mut self, method: &str, args: Value) -> R<Value> {
        let a = Args(match args {
            Value::Array(v) => v,
            Value::Null => Vec::new(),
            other => vec![other],
        });
        let out = self.call_inner(method, &a);
        self.commit()?;
        out
    }

    fn call_inner(&mut self, method: &str, a: &Args) -> R<Value> {
        match method {
            "boot" => ok(self.boot()),
            "me" => ok(self.me()),
            "createIdentity" => ok(self.create_identity_stirred(&a.str(0, "name")?, &a.opt_bytes(1, "stir")?)?),
            "setName" => ok(self.set_name(&a.str(0, "name")?)?),
            "setAvatar" => ok(self.set_avatar(&a.opt_bytes(0, "image")?)?),
            "setAvatarColor" => ok(self.set_avatar_color(a.get::<Option<String>>(0, "color")?.as_deref())?),
            "exportIdentity" => ok(b64(&self.export_identity(&a.str(0, "passphrase")?)?)),
            "importIdentity" => ok(self.import_identity(&a.bytes(0, "data")?, &a.str(1, "passphrase")?)?),
            "recoveryKey" => ok(self.recovery_key()?),
            "restoreIdentity" => ok(self.restore_identity(&a.str(0, "recoveryKey")?, &a.str(1, "name")?)?),
            "forgetRecoveryKey" => ok(self.forget_recovery_key()?),
            "recoveryStatus" => ok(self.recovery_status()?),
            "giveShares" => ok(self.give_shares(&a.str(0, "community")?, &a.strs(1, "friends")?)?),
            "heldShares" => ok(self.held_shares()?),
            "shareCandidates" => ok(self.share_candidates(&a.str(0, "community")?)?),
            "askFriends" => ok(self.ask_friends()?),
            "requestDigits" => ok(self.request_digits(&a.str(0, "code")?)?),
            "answerRequest" => ok(self.answer_request(&a.str(0, "person")?, &a.str(1, "code")?)?),
            "takeAnswer" => {
                let (answers, me) = self.take_answer(&a.str(0, "code")?, &a.str(1, "name")?)?;
                ok(serde_json::json!({ "answers": answers, "me": me }))
            }
            "linkRequest" => ok(self.link_request(&a.str(0, "label")?)?),
            "linkApprove" => ok(self.link_approve(&a.str(0, "request")?)?),
            "linkComplete" => ok(self.link_complete(&a.str(0, "approval")?)?),
            "linkHome" => ok(self.link_home(&a.str(0, "label")?)?),
            "linkCancel" => {
                self.link_cancel();
                Ok(Value::Null)
            }
            "linkAsks" => ok(self.link_asks()),
            "linkAnswer" => ok(self.link_answer(a.get(0, "id")?, a.get(1, "yes")?)?),
            "revokeDevice" => ok(self.revoke_device(&a.str(0, "key")?)?),
            "setDeviceLabel" => ok(self.set_device_label(&a.str(0, "key")?, &a.str(1, "label")?)?),
            "strengthenIdentity" => match a.get::<Option<u64>>(1, "work")? {
                Some(work) => ok(self.strengthen_identity_with(a.get(0, "bits")?, work)?),
                None => ok(self.strengthen_identity(a.get(0, "bits")?)?),
            },
            "burnIdentity" => ok(self.burn_identity(&a.str(0, "confirm")?)?),

            "communities" => ok(self.communities()),
            "discoverCommunities" => ok(self.discover_communities()),
            "openNodeLink" => ok(self.open_node_link(&a.str(0, "link")?)?),
            "previewNodeLink" => ok(self.preview_node_link(&a.str(0, "link")?)?),
            "nodeNotices" => ok(self.node_notices()),
            "acceptNotice" => ok(self.accept_notice(&a.str(0, "node")?, &a.str(1, "hash")?)?),
            "declineNotice" => ok(self.decline_notice(&a.str(0, "node")?)?),
            "forgetNotice" => ok(self.forget_notice(&a.str(0, "node")?)?),
            "publicJoin" => ok(self.public_join(&a.str(0, "community")?, &a.get::<Vec<String>>(1, "peers")?, a.opt_str(2).as_deref())?),
            "publicJoinStatus" => ok(self.public_join_status(&a.str(0, "community")?)?),
            "communityRegistration" => ok(self.community_registration(&a.str(0, "id")?)?),
            "communityPublication" => ok(self.community_publication(&a.str(0, "id")?)?),
            "community" => ok(self.community(&a.str(0, "id")?)?),
            "createCommunity" => ok(self.create_community(a.get(0, "draft")?)?),
            "createPrivateCommunity" => ok(self.create_private_community(a.get(0, "draft")?)?),
            "privateGroups" => ok(self.private_groups()?),
            "privateGroup" => ok(self.private_group(&a.str(0, "community")?)?),
            "privateJoinRequest" => ok(self.private_join_request()?),
            "privatePreviewRequest" => ok(self.private_preview_request(&a.str(0, "request")?)?),
            "privateInvite" => ok(self.private_invite(&a.str(0, "community")?, &a.str(1, "request")?)?),
            "privateJoin" => ok(self.private_join(&a.str(0, "invite")?)?),
            "privateRemove" => ok(self.private_remove(&a.str(0, "community")?, &a.str(1, "device")?)?),
            "privatePassRequest" => ok(self.private_pass_request(&a.str(0, "community")?, &a.str(1, "request")?)?),
            "privateCopy" => ok(self.private_copy(&a.str(0, "community")?)?),
            "privateOutbox" => ok(self.private_outbox(&a.str(0, "community")?)?),
            "privateReceive" => ok(self.private_receive(&a.str(0, "community")?, &a.str(1, "body")?)?),
            "privateAck" => ok(self.private_ack(&a.str(0, "community")?, &a.get::<Vec<String>>(1, "ids")?)?),
            "privateRead" => ok(self.private_read(&a.str(0, "community")?, &a.get::<Vec<String>>(1, "targets")?)?),
            "previewInvite" => ok(self.preview_invite(&a.str(0, "link")?)?),
            "join" => ok(self.join(&a.str(0, "link")?)?),
            "leave" => ok(self.leave(&a.str(0, "id")?)?),
            "setPinning" => ok(self.set_pinning(&a.str(0, "id")?, a.get(1, "on")?)?),
            "invite" => {
                // Sharing an invite asks the public nodes in reach to keep the community, so
                // the link works while every member is away.
                let id = a.str(0, "id")?;
                self.carry_for_invite(&id)?;
                ok(self.invite(&id)?)
            }
            "exportBundle" => ok(b64(&self.export_bundle(&a.str(0, "id")?)?)),
            "importBundle" => ok(self.import_bundle(&a.bytes(0, "data")?)?),

            "setLimits" => ok(self.set_limits(&a.str(0, "id")?, &a.get::<LimitsDto>(1, "limits")?)?),
            "setRules" => ok(self.set_rules(&a.str(0, "id")?, &a.str(1, "text")?)?),
            "addMod" => ok(self.add_mod(&a.str(0, "id")?, &a.str(1, "key")?)?),
            "removeMod" => ok(self.remove_mod(&a.str(0, "id")?, &a.str(1, "key")?)?),
            "retire" => ok(self.retire(&a.str(0, "id")?, &a.str(1, "key")?)?),
            "looked" => {
                // a look the host took in a worker of its own (crate::look)
                let id = crate::api::parse_id(&a.str(0, "hash")?)?;
                let look: crate::look::LookDto = a.get(1, "look")?;
                self.looked(id, look.parse());
                ok(())
            }
            "cosign" => ok(self.cosign(&a.str(0, "proposal")?)?),

            "ledger" => ok(self.ledger(&a.str(0, "id")?)?),
            "remove" => ok(self.remove(&a.str(0, "target")?, &a.str(1, "reason")?)?),
            "pin" => ok(self.pin(&a.str(0, "target")?, a.get(1, "on")?)?),
            "label" => ok(self.label(&a.str(0, "target")?, &a.str(1, "label")?, a.opt_str(2).as_deref())?),
            "ban" => ok(self.ban(&a.str(0, "community")?, &a.str(1, "key")?, &a.opt_str(2).unwrap_or_default())?),
            "unban" => ok(self.unban(&a.str(0, "community")?, &a.str(1, "key")?, &a.opt_str(2).unwrap_or_default())?),
            "admit" => ok(self.admit(&a.str(0, "community")?, &a.str(1, "key")?)?),

            "standing" => ok(self.standing(&a.str(0, "community")?, a.opt_str(1).as_deref())?),
            "vouch" => ok(self.vouch(&a.str(0, "community")?, &a.str(1, "key")?, a.get(2, "value")?, a.opt_str(3).as_deref())?),
            "review" => ok(self.review(&a.str(0, "target")?, &a.str(1, "verdict")?, a.opt_str(2).as_deref())?),
            "reviewQueue" => ok(self.review_queue(&a.str(0, "community")?)?),
            "media" => ok(self.media(&a.str(0, "hash")?)?),

            "feed" => {
                let q: FeedQuery = a.get(0, "query")?;
                ok(self.feed(q.community.as_deref(), q.sort.as_deref().unwrap_or("hot"), q.limit)?)
            }
            "post" => ok(self.post(&a.str(0, "id")?)?),
            "createPost" => ok(self.create_post(&a.get(0, "draft")?)?),
            "reply" => ok(self.reply(&a.str(0, "parent")?, &a.str(1, "text")?)?),
            "edit" => ok(self.edit(&a.str(0, "id")?, &a.str(1, "text")?, a.opt_str(2).as_deref())?),
            "deleteOwn" => ok(self.delete_own(&a.str(0, "id")?)?),
            "deletionProgress" => {
                self.commit()?;
                ok(self.deletion_progress())
            }
            "previewBulkDeletion" => ok(self.preview_bulk_deletion()?),
            "confirmBulkDeletion" => ok(self.confirm_bulk_deletion(&a.str(0, "snapshot")?)?),
            "cancelBulkDeletion" => ok(self.cancel_bulk_deletion(&a.str(0, "snapshot")?)?),
            "bulkDeletions" => {
                self.commit()?;
                ok(self.bulk_deletions())
            }
            // RFC-0003: readiness only from state storage accepted.
            "cleanupReadiness" => {
                self.commit()?;
                ok(self.cleanup_readiness())
            }
            "vote" => ok(self.vote(&a.str(0, "target")?, a.get(1, "value")?)?),
            "flag" => ok(self.flag(&a.str(0, "target")?, &a.str(1, "reason")?)?),
            "save" => ok(self.save(&a.str(0, "id")?, a.get(1, "on")?)?),
            "saved" => ok(self.saved()),
            "search" => ok(self.search(&a.str(0, "q")?, a.opt_str(1).as_deref())?),

            "person" => ok(self.person_view(&a.str(0, "key")?, a.opt_str(1).as_deref())?),
            "setTrust" => ok(self.set_trust(&a.str(0, "key")?, a.get(1, "friend")?)?),
            "trustSharing" => ok(self.trust_sharing(&a.str(0, "community")?)?),
            "shareTrust" => ok(self.share_trust(&a.str(0, "community")?, a.get(1, "on")?)?),
            "labelers" => ok(self.labelers()),
            "followLabeler" => ok(self.follow_labeler(&a.str(0, "key")?, a.get(1, "on")?)?),
            "blocklistKeepers" => ok(self.blocklist_keepers()),
            "followBlocklist" => ok(self.follow_blocklist(&a.str(0, "key")?, a.get(1, "on")?)?),
            "publishBlocklist" => ok(self.publish_blocklist(&a.str(0, "community")?, &a.str(1, "text")?)?),
            "aggregators" => ok(self.aggregators()),
            "followAggregator" => ok(self.follow_aggregator(&a.str(0, "key")?, a.get(1, "on")?)?),
            "aggregating" => ok(self.aggregating(&a.str(0, "community")?)?),
            "setAggregating" => ok(self.set_aggregating(&a.str(0, "community")?, a.get(1, "on")?)?),
            "ageChallenge" => ok(self.age_challenge(&a.str(0, "community")?)?),
            "ageProofCode" => ok(self.age_proof_code(&a.str(0, "community")?, &a.str(1, "presentation")?)?),
            "admitByAge" => ok(self.admit_by_age(&a.str(0, "code")?)?),
            "wikiPages" => ok(self.wiki_pages(&a.str(0, "community")?)?),
            "wikiPage" => ok(self.wiki_page(&a.str(0, "community")?, &a.str(1, "page")?)?),
            "editWiki" => ok(self.edit_wiki(&a.str(0, "community")?, &a.str(1, "page")?, &a.str(2, "text")?)?),
            "linkPreview" => ok(self.link_preview(&a.str(0, "url")?)?),
            "publishLabel" => ok(self.publish_label(&a.str(0, "target")?, &a.str(1, "label")?, a.opt_str(2).as_deref())?),
            "setMuted" => ok(self.set_muted(&a.str(0, "key")?, a.get(1, "muted")?)?),
            "setPetname" => ok(self.set_petname(&a.str(0, "key")?, a.opt_str(1).as_deref())?),

            "stats" => ok(self.stats()),
            "settings" => ok(self.settings()),
            "setSettings" => ok(self.set_settings(&a.get::<SettingsPatch>(0, "patch")?)?),
            "net" => ok(self.net_status()),
            "traffic" => ok(self.traffic()),
            "warmth" => ok(self.warmth()),
            "communityWarmth" => ok(self.community_warmth(&a.str(0, "community")?)?),
            "requestCarry" => ok(self.request_carry(&a.str(0, "community")?)?),
            "pair" => ok(self.pair(&a.str(0, "link")?)?),
            "previewPair" => ok(self.preview_pair(&a.str(0, "link")?)?),
            "unpair" => ok(self.unpair()),
            "rotatePairing" => ok(self.rotate_pairing()?),
            "wipe" => ok(self.wipe()?),
            "clearAfterDeletion" => ok(self.clear_after_deletion(&a.str(0, "inventory")?, &a.str(1, "deletions")?)?),
            "importBlocklist" => ok(self.import_blocklist(&a.str(0, "text")?)?),
            "sweep" => ok(self.sweep()),

            // draft → mine elsewhere → publish (web workers, native threads)
            "draftPost" => ok(self.draft_post(&a.get(0, "draft")?)?),
            "draftReply" => ok(self.draft_reply(&a.str(0, "parent")?, &a.str(1, "text")?)?),
            "draftEdit" => ok(self.draft_edit(&a.str(0, "id")?, &a.str(1, "text")?, a.opt_str(2).as_deref())?),
            "publishDraft" => match self.publish_draft(a.get(0, "token")?, a.get(1, "nonce")?)? {
                Published::Post(p) => ok(p),
                Published::Comment(c) => ok(c),
                Published::Nothing => Ok(Value::Null),
                Published::Signing(id) => ok(serde_json::json!({ "signing": id })),
            },
            "published" => {
                let id = crate::api::parse_id(&a.str(0, "id")?)?;
                let kind = self.store.get(&id).map(|s| s.record.kind()).ok_or_else(|| ApiError::not_found("record"))?;
                match self.published(kind, &id)? {
                    Published::Post(p) => ok(p),
                    Published::Comment(c) => ok(c),
                    Published::Nothing | Published::Signing(_) => Ok(Value::Null),
                }
            }
            "signAsks" => ok(self.sign_asks()),
            "signAnswer" => ok(self.sign_answer(&a.str(0, "id")?, a.get(1, "yes")?)?),
            "linkHomeRemote" => ok(self.link_home_remote(&a.str(0, "label")?)?),
            "draftIdentity" => ok(self.draft_identity(&a.str(0, "name")?, &a.opt_bytes(1, "stir")?)?),
            "publishIdentity" => ok(self.publish_identity(&a.str(0, "rootKey")?, a.get(1, "work")?)?),
            #[cfg(not(target_arch = "wasm32"))]
            "mine" => {
                let p = hoi_crypto::pow::Puzzle { prefix: a.bytes(0, "prefix")?, suffix: a.bytes(1, "suffix")?, bits: a.get(2, "bits")? };
                ok(mine_puzzle(&p))
            }
            #[cfg(not(target_arch = "wasm32"))]
            "mineIdentity" => {
                let root = crate::api::parse_id(&a.str(0, "rootKey")?)?;
                let bits: u32 = a.get(1, "bits")?;
                if !(hoi_crypto::identity::MIN_BITS..=hoi_crypto::identity::MAX_BITS).contains(&bits) {
                    return Err(ApiError::invalid(format!("Identity work is {} to {} bits.", hoi_crypto::identity::MIN_BITS, hoi_crypto::identity::MAX_BITS)));
                }
                ok(hoi_crypto::identity::mine(&root, bits))
            }
            #[cfg(not(target_arch = "wasm32"))]
            "benchmark" => ok(
                json!({ "hashesPerSecond": hoi_crypto::pow::benchmark(200_000) * std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64 }),
            ),
            other => Err(ApiError::invalid(format!("unknown method {other}"))),
        }
    }
}

/// Mines a puzzle on every core.
#[cfg(not(target_arch = "wasm32"))]
pub fn mine_puzzle(p: &hoi_crypto::pow::Puzzle) -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(16) as u64;
    let found = AtomicU64::new(u64::MAX);
    std::thread::scope(|s| {
        for t in 0..threads {
            let found = &found;
            s.spawn(move || {
                let mut start = t;
                loop {
                    if found.load(Ordering::Relaxed) != u64::MAX {
                        return;
                    }
                    if let Some(n) = p.search(start, threads, 4096) {
                        found.fetch_min(n, Ordering::Relaxed);
                        return;
                    }
                    start += threads * 4096;
                }
            });
        }
    });
    found.load(Ordering::Relaxed)
}

#[allow(dead_code)]
fn unused() -> Value {
    json!(null)
}
