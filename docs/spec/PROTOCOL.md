# hoi protocol specification

Version 0.3 (draft) · October 2026 · Wire-level companion to `TECHNICAL.md`. Supersedes 0.2.

This document is normative where it says MUST. Everything is a signed, content-addressed **record**; identities are keys that did memory-hard work once; communities are keys; standing in a community is earned from the community's own signed records; decisions weigh voices, and voices thin out when someone vouches in many people at once; owners propose rules and limits and the community decides them; sync is set reconciliation; moderation is a signed ledger that the community can overturn; deletion is expiry.

> **Pre-release.** 0.2 changed the delegation format and added required fields (`Limits.ladder`, identity records); 0.3 changes no bytes on the wire, only what records mean (§6.1, §15.4, §15.5). Nothing was deployed, so the envelope stays `v: 1` until the first release. Stored records a device can no longer decode are deleted at startup and counted, so the app can say so.

## 1. Primitives

| Thing | Choice |
|---|---|
| Signature | Ed25519 (strict verification; small-order keys refused) |
| Hash / id | BLAKE3-256 |
| Encoding | DAG-CBOR, canonical (sorted map keys, shortest ints, no floats, no tags, definite lengths) |
| Proof-of-work stamp | BLAKE3 hashcash: leading zero bits of `BLAKE3(id ‖ u64be(nonce))` |
| Identity work | Argon2id v1.3, m = 8192 KiB, t = 1, p = 1, 32-byte output (§4.1) |
| Time | Unix seconds, UTC |
| Identity | `did:key:z6Mk…` (Ed25519 public key) |
| Transport | iroh (QUIC); ALPNs `hoi/gossip/1`, `hoi/sync/1`, `hoi/blob/1` (reserved); WebSocket to a home node today (§14) |

## 2. Record envelope

```
Record = {
  v:          1,
  kind:       "post" | "comment" | "vote" | "edit" | "delete" | "delegation" | "revoke" | "identity" |
              "community" | "ledger" | "label" | "report" | "vouch" | "review",
  community:  bytes32,      // absent for delegation, revoke, identity; optional for label
  author:     bytes32,      // signing key: root, device, or (genesis) community key
  parent:     bytes32?,     // comment → post/comment; vote/edit/delete/label/review → record;
                            // report/vouch → a person's root key
  created_at: uint,
  expires_at: uint,
  body:       map,          // kind-specific, §3
  nonce:      uint,         // stamp nonce
}
id  = BLAKE3(canonical_cbor(Record))
sig = Ed25519.sign(author_sk, id)
Wire = { r: Record, sig: bytes64 }
```

Rules:
- Receivers MUST decode strictly (unknown or missing fields, non-canonical bytes and unknown kinds are refused), recompute `id` from the canonical bytes and verify `sig` over it.
- `author` MAY be a device key; then the receiver MUST hold a valid, unexpired, unrevoked `delegation` for it whose scope covers the kind and community (§3).
- Records are immutable. Optional fields are omitted when absent; an empty `media` array is not canonical and MUST be refused.
- Max encoded wire size: 64 KiB (post, edit, ledger, community), 16 KiB (comment), 2 KiB (delegation, report), 1 KiB (review), 512 B (vote, label, delete, revoke, identity, vouch).
- `expires_at` MUST be ≥ `created_at`. It MUST be ≤ `created_at + community.max_retention` (the limits of the record's day, §4), except for records **exempt from retention**: genesis, delegation, revoke, identity, vouch, ledger entries kept until superseded (rules, limits, mod-add, mod-remove) and reviews answering a proposal (§6.1), which MUST be ≤ `created_at + 100 years`.
- Freshness: `created_at` MUST NOT be more than 10 min in the future. Records arriving live on gossip MUST also be at most 10 min old (threshold proposals excepted, §6); catch-up sync and imports accept older records inside the retention window, never expired ones.

## 3. Kinds

**post**: `{ title: str, text: str (markdown subset), link: str?, name: str?, media: [MediaRef]? }` — `name` is the author's self-chosen display name (≤ 32 chars, §18.3); anyone can claim any name, so clients MUST show the key next to it. `media`: 1–4 items, §17.
**comment**: `{ text: str, name: str? }`, `parent` required (a post or comment of the same community).
**vote**: `{ value: 1 | -1 | 0, seq: uint }` — latest `seq` per `(author root, parent)` wins; `0` retracts. `parent` is a post or comment of the same community.
**edit**: `{ text: str, title: str? }`, `parent` = original, same author root and community required.
**delete**: `{}`, `parent` = record to delete, same author root required. Compliant nodes delete the body and refuse re-ingestion.
**delegation** (signed by the **device**): `{ root: bytes32, scope: [str], expires_at: uint, root_sig: bytes64 }`.
- The record signature is the device's consent; `root_sig = Ed25519.sign(root_sk, delegation_statement)` is the root's blessing, where `delegation_statement = BLAKE3(canonical_cbor({ t: "hoi/delegation/1", root, device, scope, expires_at }))` and `device` is the record's `author`.
- `root ≠ author`; `body.expires_at = record.expires_at ≤ created_at + 400 days`; neither key may be weak.
- Scope strings: `post`, `comment`, `vote`, `moderate:<community hex>`, `*`. `edit`/`delete` need `post` or `comment`; `vote`, `label`, `report`, `vouch` and `review` need `vote`; `ledger` needs `moderate:<community hex>`. Identity, revoke, delegation and genesis records are signed by the root (or community) key itself.
- A device belongs to the **first root** that blessed it: a delegation for a known device from another root MUST be refused; a later delegation from the same root with a later expiry renews it. A key that has an identity record MUST NOT be accepted as a device, and a known device MUST NOT be accepted as an identity. A root has at most **16 live devices** (unexpired, unrevoked): receivers refuse a delegation for a new device beyond that until one is revoked.
**revoke** (signed by a root): `{ device: bytes32 }`. Only the root that blessed a device may revoke it; revocations that arrive before the delegation are kept and applied when it arrives. Records the device created at or after the revocation are refused. **`device = author` burns the identity** (§4.2).
**identity** (signed by a root): `{ work: uint, rc: bytes32?, of: bytes32? }` — `work` is the nonce of the root's identity work (§4.1); `rc` commits to the recovery key that may replace this root, `BLAKE3-derive-key("hoi 2026 recovery commitment v1", recovery key)`; `of` names the person a later root continues (§4.3). Every identity record of one root MUST carry the same `rc` and `of`; a root seen with two is treated as burned (§4.2).
**rotate** (signed by a recovery key): `{ person: bytes32, to: bytes32 }`, no community, no parent. Valid when the author's commitment is the `rc` of a root in `person`'s chain, `to` has an identity whose `of` is `person`, and `to` is nobody's root or device yet; §4.3.
**share**: `{ sealed: [{ d: bytes32, s: bytes }] (≤ 16), seq: uint }`, `parent` = the friend (a person) who keeps one share of the author's recovery line; `s` is the share (`x ‖ y`, 33 bytes) sealed to device `d`'s seal key, 81 bytes (§4.3). Latest `seq` per (author, friend) wins; an empty `sealed` takes it back. Needs scope `vote`, kept like a vouch.

**trust**: `{ keys: [bytes32] (≤ 256, sorted, each once) }`, no `parent`: the people (person ids) the author trusts, published in a community only by the author's choice. The latest per author and community replaces the earlier ones; an empty list takes it back. Needs scope `vote`, follows the community's retention, eight bits of stamp discount, 4 / h and 12 / 24 h. A client names only the friends known in that community (admitted there, or with records of their own there), and renews the statement before it expires.

**wiki**: `{ page: str, update: bytes (≤ 32 KiB) }`, no `parent`: one edit to the community's wiki page `page` (1 to 64 of `a-z`, `0-9`, `-`), as a Loro update to the page's document, whose text is the container `body`. A page is the merge of all its edits, whatever order they arrived in; an edit whose predecessors haven't arrived waits. Only admitted members, owners and moderators edit, and not while banned or blocked; others' edits are refused without penalty, and bytes that aren't a Loro update are the author's fault (content). A page holds at most 1 MiB of edits, a wiki at most 256 pages. Needs scope `post`, eight bits of stamp discount, 60 / h and 200 / 24 h, and is kept like a ledger entry (100 years): every edit builds on the ones before it, so none can be taken out. A bad edit is undone by editing; a banned author's later edits are refused.

**tally**: `{ counts: [[bytes32, uint, uint]] (≤ 500, sorted by post, each once) }`, no `parent`: an aggregator's counts of a community's posts, each row a post with the up- and down-votes it holds (each voter's latest vote once, unweighted). The latest per author and community replaces the earlier ones. Accepted only from aggregators the person follows; anyone else's is refused without penalty, and following reconciles every connection again. Needs scope `vote`, eight bits of stamp discount, 4 / h and 48 / 24 h, and lasts at most a week. Clients show the counts next to a post's own and may order a feed by them when the person asks; they never enter Hot, Loudest or any other ranking.

**blocklist**: `{ exact: [bytes32] (≤ 1000, sorted, each once), pdq: [bytes32] (≤ 1000, sorted, each once) }`, no `parent`: a list keeper's list of exact media hashes (BLAKE3) and picture fingerprints (PDQ). The latest per author and community replaces the earlier ones. Accepted only from keepers the person follows; anyone else's is refused without penalty, and following reconciles every connection again. A follower's devices refuse those hashes, and pictures any way round within the PDQ distance of §17 of those fingerprints, in every community, besides their own list; what they hold of it is deleted, and nothing goes into their own list. Needs scope `vote`, eight bits of stamp discount, 4 / h and 12 / 24 h.
**community**: genesis record, `author` = community key, §5.
**ledger**: moderation action, §6.
**label**: `{ label: str, note: str? }`, `parent` = target, `community` = the target's. Accepted only from labelers the community lists or the person follows; anyone else's is refused without penalty. A device that starts following a labeler reconciles every connection again, so their labels arrive with the next sync.
**report**: `{ reason: "spam" | "rate" | "dup" | "rule:<n>" | "illegal", evidence: [bytes32] }`, `parent` = reported person (root key). Evidence is record ids only, never content.
**vouch**: `{ value: 1 | 0 | -1, seq: uint, reason: str? }`, `parent` = the person's root key. `1` vouches, `-1` objects and MUST carry a hide reason (§16.1), `0` takes it back; only `-1` carries a reason. Nobody vouches for themselves. Latest `(seq, created_at, id)` per `(author root, person)` wins; nodes MAY delete superseded vouches and MUST then refuse them again.
**review**: `{ verdict: "ok" | "hide" | "veto" | "none", seq: uint, reason: str? }`, `parent` = a post, comment or ledger entry of the same community. `hide` needs a hide reason (§16.1) and applies to posts and comments; `veto` needs a free-text reason (1–200 chars) and applies to vetoable ledger entries (§16.3) and to proposals (§6.1), where it objects; `ok` says a post or comment belongs here, supports a ledger entry, or agrees to a proposal; `none` takes a verdict back; only `hide` and `veto` carry a reason. Nobody reviews their own record or vetoes or answers an entry they signed. Latest `(seq, created_at, id)` per `(author root, parent)` wins, except for answers to a proposal: nodes MUST keep every answer (their history decides it), only a person's first 16 answers to a proposal by `(created_at, id)` count and nodes SHOULD refuse the rest, and nodes MUST refuse answers created at or after the proposal's deadline.

## 4. Stamps and identity work

```
bits_required = community.pow_bits - kind_discount - age_discount(author)
kind_discount = 8 for vote, label, ledger, report, delete, vouch, review; else 0
age_discount  = 0 if key age < 30 d; 2 if < 90 d; 4 if < 365 d; 6 otherwise
valid iff leading_zero_bits(BLAKE3(id ‖ u64be(nonce))) >= bits_required
```

Default `pow_bits` = 22 (≈ 1 s on a laptop). Records outside any community (delegation, revoke, identity) and genesis records need 16 bits. Key age is measured from when the receiver first saw that key, never from `created_at`; authors SHOULD stamp for the full difficulty.

**The limits of a record's day.** Where a record is checked against the community's `pow_bits`, `max_retention`, `newcomer_posts_per_day` (§9.1) or `ladder.identity_bits` (§4.1), receivers use, field by field, the more lenient of the limits in effect at the record's `created_at` (§6.1) and the limits in effect now. Members accepted older records under the limits of their day, so a device that joins after the community tightened a limit takes them too; anything made since needs no more than today's limits. Tightening a limit binds new records; it doesn't refuse, or shorten, what was already made. A record backdated to before a change gets that day's limits, but still has to fit that day's retention window and the rate windows of §9.1, counted by `created_at`.

Receivers SHOULD check the **stamp floor** (the least any record of that kind could need: 16 bits for delegation, revoke, identity and genesis; `max(0, 8 - kind_discount - 6)` otherwise) before verifying any signature, so unstamped junk costs one hash each.

### 4.1 Identity work

A root key is an **identity** once an `identity` record carries its work:

```
w    = Argon2id(password = root ‖ u64be(work), salt = "hoi/identity/v1\0" (16 bytes),
                m = 8192 KiB, t = 1, p = 1, output = 32 bytes)
bits = leading_zero_bits(w)
```

- The work depends only on the key: it is done once, stays with the key for life, and cannot be transferred. Argon2id makes each try cost memory, so specialised hardware gains far less over a phone than with a plain hash.
- Receivers MUST refuse identity records below **4 bits** (and weak keys) and SHOULD penalise the forwarding peer. Clients mine 10 bits by default (about a second natively, ten in a browser) and MAY mine more later; the identity record with the most bits counts.
- Every record in a community other than the genesis MUST come from a root with a known identity carrying at least `ladder.identity_bits` (§5). Records whose root has no known identity yet are refused without penalty (they are fetched again once the identity arrives). Receivers MAY skip this threshold check for records read back from their own storage.
- Verification costs one Argon2id run; receivers SHOULD verify only after the stamp and signature passed, and cache results. They SHOULD NOT hold a lock other connections wait for while it runs (hosts that keep the node behind one verify a message's identity work before taking it in). One forged work is reason enough to drop the peer and the rest of what it sent.
- Identity work is a **throttle, not Sybil resistance.** At 8 MiB and one pass, 10 bits cost about a second of one core, so a thousand keys cost a quarter of an hour and rented hardware does better still. It makes each key cost something and floods of fresh keys slow; it does not stop one person holding many keys. What keeps many keys from counting for much is standing: newcomers' votes, hides and vouches count for nothing, and voices split among people vouched in together (§15, §19). Communities may ask for more bits (`ladder.identity_bits`), which slows everyone, honest newcomers on phones first.

### 4.2 Burning an identity

`revoke { device: <own root> }` signed by the root burns it. Receivers MUST drop every record by that root (and its devices) except the revoke itself, and refuse anything it signs afterwards, forever. A burn by a person's current root burns the person (§4.3); a replaced root can no longer burn anything. Standing never carries over: a new key starts from nothing in every community.


### 4.3 Recovery: replacing a root and keeping standing

A person's **recovery line** holds a 32-byte master secret `M`. The root of generation `g` is `Ed25519(BLAKE3-derive-key("hoi 2026 root key v1", M ‖ u32le(g)))`, and the recovery key of generation `g` is `Ed25519(BLAKE3-derive-key("hoi 2026 recovery key v1 by generation", M ‖ u32le(g)))`. The root of generation 0 is the **person**: their id for good, wherever standing, bans, ownership or vouches name them.

- The identity record of the root of generation `g` commits (`rc`) to the recovery key of generation `g + 1`; a root of `g ≥ 1` names the person in `of`.
- A `rotate { person, to }` signed by a committed recovery key replaces that root with `to` from the rotation's `created_at` on. The **chain** of a person is their first root followed by each rotation's `to`; its last root is the current one.
- Records signed at or after a root's replacement by that root, or by a device it blessed, MUST be refused. What it signed before stays.
- A record by a root that names someone in `of` speaks for that person once the rotation to it is known; until then it is refused without penalty, like a record of an unknown identity, and comes again with its carriers.
- From a root's devices, the author is the person. A record's carriers include, after the person's first identity, each later root's identity and each rotation, before the delegation they bless.
- The earliest valid rotation from a root counts; a later one from the same root is refused.
- A community's `identity_bits` is checked against the identity with the most work in the person's chain. A root that continues someone needs only the minimum work (4 bits).
- **Friends** (`share`): the line's master secret is split with Shamir's scheme over GF(2⁸) (the AES field), any 3 of 3 to 7 shares rebuilding it, each share sealed with HPKE (`seal_to`, info `"hoi recovery share v1"`, AAD `"hoi/recovery-share/1" ‖ person ‖ friend ‖ device`) to every live device of the friend. A device recovering shows a one-time X25519 key (`hoi-ask-1-` ‖ base64url) and six digits from `BLAKE3-derive-key("hoi recovery request digits v1", key)`; a friend's device answers with `person ‖ share` sealed to that key (info `"hoi recovery answer v1"`, AAD = the key) as `hoi-share-1-` ‖ base64url. Requests and answers travel however people send them; nothing about them is published.

## 5. Communities

A community is an Ed25519 keypair. Its **genesis** `community` record carries:

```
{ name: str, description: str, color: str,
  owners: [bytes32],            // 1–16 root keys allowed to sign rules/limits/mod changes
  threshold: uint,              // owner signatures required for those (1 ≤ threshold ≤ owners)
  rules_text_cid: bytes32?,
  limits: Limits }
Limits = { pow_bits: 22, max_retention: 15552000 (180 d), newcomer_posts_per_day: 2,
           newcomer_age: 2592000 (30 d), admission: "open" | "invite" | "age-proof" | "approve",
           autoban_threshold: 12, labelers: [bytes32], ladder: Ladder,
           scatter: Scatter?, trackers: [text]? }   // trackers: §10
Ladder = { identity_bits: 8,     // 0–20, §4.1
           admit_vouches: 2,     // 1–16, §15.1
           established_age: 1209600 (14 d),  // ≤ 365 d
           trusted_posts: 5,     // ≤ 1000, §15.2
           media_score: 10,      // ≤ 100000, §15.2
           hide_quorum: 3, block_quorum: 5, veto_quorum: 5,   // voices, 2–100, §16
           contest_window: 259200 (72 h) }   // ≤ 14 d, §6
```

- The **first** genesis a node accepts for a community key is the genesis, forever: later ones MUST be refused. Clients SHOULD forget the community secret key once the genesis is published; owners govern through the ledger.
- The genesis never expires. Owners **propose** changes as `ledger` entries of kind `limits` or `rules` meeting `threshold`; the community decides them (§6.1).
- **Membership** is implicit (a node that reconciles the community is a member). **Admission** decides what newcomers' records do (§15.3): `open` — shown, marked as unvalidated; `approve` — shown only to established members, moderators and owners until admitted; `invite` — held until at least one established member has vouched (the vouch is the invite; someone who vouches in many people at once gives each a thin voice, §15.4); `age-proof` — held until an owner or moderator signs an `admit` entry after their device checked a proof of age; vouches admit nobody here, though they still give voice to people already admitted.

**Age proofs.** `limits.age_issuers` lists 1 to 8 credential issuers' public keys (32 bytes Ed25519, or 33 or 65 bytes P-256 SEC1), required when admission is `age-proof`. A proof is an SD-JWT VC presentation: the issuer's JWT (ES256 or EdDSA, by one of those keys, not expired, `_sd_alg` SHA-256), its disclosures, and a key-binding JWT (`typ` `kb+jwt`) signed by the credential's `cnf.jwk` holder key (P-256 or Ed25519). It must say `age_over_18: true`, disclose nothing else, and its key binding must carry `aud` = `hoi:<community hex>`, `nonce` = base64url of the first 16 bytes of `BLAKE3-derive-key("hoi age proof nonce v1", person ‖ community)`, an `iat` within the last 15 minutes, and the `sd_hash` of what it binds. The newcomer's device checks it and makes a code, `hoi-age-1-` ‖ base64url(person ‖ community ‖ presentation), sent to a moderator however they like; the moderator's device checks it again and signs `admit` with reason `age proof`. Neither the presentation nor anything from it is published.

## 6. Moderation ledger

`ledger` records have `author` = an owner's or moderator's device key (scope `moderate:<community>`), and body:

```
{ seq: uint,
  action: "rules" | "limits" | "mod-add" | "mod-remove" | "remove" | "ban" | "unban" |
          "label" | "pin" | "unpin" | "admit" | "drop-key" | "retire",
  target: bytes32?, reason: str?, evidence: [bytes32]?,
  cosign: [ {key: bytes32, sig: bytes64} ]?, text: str?, limits: Limits? }
```

- `rules`, `limits`, `mod-add`, `mod-remove`, `retire` require owner signatures reaching `threshold` (cosigners sign `proposal_id = BLAKE3(canonical_cbor(Record with body.cosign removed, nonce = 0, author = 32 zero bytes))`; distinct owner roots count). A cosigned entry MUST be published within 7 days of its `created_at` and is exempt from the live ±10 min window. Those entries are kept until superseded (`expires_at = created_at + 100 years`); all others expire with `max_retention`. `mod-add` and `mod-remove` take effect when applied; `rules` and `limits` are proposals (§6.1).
- `remove` → the target **naps** (hidden, peekable, its body kept) until `effective_at = created_at + ladder.contest_window`; then receivers tombstone it (delete the body, keep the id and the entry, refuse re-ingestion). Removals whose reason starts with `illegal` or `privacy` take effect at once.
- `ban` → receivers refuse records by `target` in this community created at or after the entry. Owners can't be banned.
- `admit` → the target is admitted (§15.1). `label`, `pin`, `unpin` as named.
- `retire` → the owners say the target, an elector, left for good (§6.1). Owners can't be retired. Vetoable like other entries (§16.3).
- Entries are applied in `(at, seq, id)` order, `at` being the entry's effective time (§15.5), each checked against the state before it, **skipping entries the community overturned** (§16.3) and entries signed by a moderator at or after their suspension (§16.4). A removal overturned after it took effect lets the record back in. Accepted proposals take effect at their decision time, ordered right after their own entry when both fall on the same second.
- Any member can export the ledger; it is verifiable by anyone holding the genesis record and the community's reviews.

### 6.1 Changes by consent

A `limits` or `rules` entry that meets the owners' threshold is a **proposal** `p`. It changes nothing until the community decides it:

- **Electors** `S(p)`: everyone established at `p.at` (founders by then, or admitted at least `established_age` of the limits in effect at `p.at` before), counted as if ledger bans didn't exist (a moderator can't ban objectors out), without burned identities and without people blocked or struck now. People established later, including anyone vouched in after the proposal, are not electors for it.
- **Answers** are reviews of `p` by electors: `ok` agrees, `veto` objects (with a reason), `none` withdraws. The signers agree. An elector's answer at time `t` is their answer with the highest `(seq, created_at, id)` among those with `created_at ≤ t`. Each elector weighs their voice at `p.at` (§15.4).
- **Around** / **away**: an elector is around while they have at least one live post or comment in the community that is neither hidden by the community (§16.1) nor removed; otherwise away.
- **Deadline** `D = p.at + max_retention` (of the limits in effect at `p.at`): by then everything anyone had posted before the proposal has faded. Answers created at or after `D` are refused.
- **Passed, unanimous**: at the first time `t ∈ [p.at, D)` at which the electors whose answer isn't `ok` weigh less than half a voice (`DISSENT = VOICE/2`). Final: later answers don't undo it.
- **In effect** (now `< D`, not yet passed): the electors who object, plus those who are around and haven't agreed, weigh less than half a voice. People who are away don't hold a change up; if they come back and object before `D`, it stops again.
- **Passed at the deadline** if, at `D`, objections weigh less than half a voice: silence counts as consent once everyone has had a whole retention window to answer. Otherwise **lapsed**: it never takes effect.

**Retired electors.** Someone who left for good and whose posts still live counts as waiting, and once away still keeps a change from passing for good before `D`. A `retire` entry for elector `x` at time `r` counts when `x` is not an owner and the latest effective time of anything `x` signed in the community, or `x`'s admission if later, is at least `RETIRE_AWAY` (30 days) before `r`, and `x` has signed nothing since `r`. It takes effect at `r + RETIRE_NOTICE` (7 days): from then on, an elector `x` who hasn't answered weighs nothing, neither as waiting nor in the unanimity test, and the time it takes effect is a candidate time for unanimity. A retirement never takes away an answer `x` gave. It lapses as soon as `x` signs anything in the community, and is gone if the community vetoes it. Answers, `last_seen` and admission times are all effective times (§15.5).

A `limits` proposal changes only the fields in which its `limits` differ from the limits in effect at `p.at`; those fields take its values when it takes effect. Proposals about different fields compose; for the same field the one that takes effect later wins. A `rules` proposal carries, besides the full text its signers saw (`text`), its change as a Loro update (`patch`, ≤ 64 KiB) to the rules document, whose text is the container `rules`, as it stood when they wrote it. The rules in force are the passed proposals' patches imported in the order they took effect, starting from an empty document; each passed proposal bumps the version. Two proposals written against the same rules both count once both pass, rather than the later replacing the earlier. While nobody but the signers is established, a proposal passes at once.

## 7. Sync

**Live:** one gossip topic per community, `BLAKE3("hoi/gossip/1" ‖ community)`. Nodes forward valid records they haven't seen, each preceded by the author's identity record and, for device-signed records, the delegation.

**Carriers travel with company.** Identity, delegation and revoke records have no community and are kept for years. From a peer that is not one of its own paired devices or its home node, a receiver takes one only for a root it has heard from (it holds records of theirs in a community it carries, or their identity, or a device of theirs), or whose community record the same `recs` batch carries. A stranger's identities, devices or revocations on their own are dropped without penalty: otherwise anyone could fill every node with keys nobody uses, for good.

**Catch-up:** range-based set reconciliation (`rng`, §14) over the live records of a community, ordered by `(created_at, id)`, between peers that list `range`; a full `have` list to a peer that doesn't. Fetch missing records by id.

**What is never passed on:** records held back (§15.3) and records by people the community blocked (§16.2) are stored but not offered in `have` or `rng`, not served on `want`, not gossiped and not exported.

**Media** travels by hash (`bwant` / `blob`, §14, §17).

**Expiry:** nodes MUST delete records past `expires_at` and MUST NOT include them in reconciliation. A saved post stays local only.

## 8. Trust and ranking (local, informative)

- A node keeps a **ring**: keys it trusts directly (degree 1) and the keys those people's latest `trust` statements name, in any community the node holds (degree 2). A statement counts only on devices whose person trusts its author; nobody else's statement changes anyone's ring.
- Ranking weights (default): degree 1 → 1.0, degree 2 → 0.5, seen > 90 d → 0.25, newcomer → 0.05, banned → 0. Nothing here is on the wire; two nodes legitimately see different orders.
- A report counts by the larger of the reporter's ring weight and their community standing (trusted 1.0, established 0.5).

**Handles (informative).** A self-chosen name that is a domain (two labels or more of `a-z0-9-`, a letter top-level label or punycode, at most 253 characters) is a claim each device checks for itself: the domain names the person's key when `_hoi.<domain>` has a TXT record `did=<did:key>`, or `https://<domain>/.well-known/hoi` answers with a line `<did:key>` (the person id as an Ed25519 did:key: multicodec `0xed01`, base58btc). Native devices ask the system's resolver and then HTTPS (by the link preview rules: https, internet addresses only, no cookies); browsers ask HTTPS only, which needs `Access-Control-Allow-Origin` on that file. A device checks a domain again after a day (an hour after a failure), at most 60 checks an hour. A name that looks like a domain and isn't verified shows as such. Nothing about a check is published.

## 9. Spam

### 9.1 Rate windows

Every node limits how many records of a kind one author root may have in a community, **counted by the records' own `created_at`**, whatever path they arrived by (gossip, catch-up, import). A record is refused if any window of the given length containing it would hold more than the maximum. Owners and moderators are exempt. *Newcomer* means not admitted (§15).

| Kind | Newcomer | Member and up |
|---|---|---|
| post | `newcomer_posts_per_day` per 24 h | 12 / h, 48 / 24 h (with media: 4 / h, 6 / 24 h) |
| comment | 10 / h, 30 / 24 h | 60 / h, 400 / 24 h |
| edit, delete | 30 / h, 100 / 24 h | same |
| vote | 300 / h, 2000 / 24 h | same |
| vouch, review | 20 / 24 h | 60 / h, 300 / 24 h |
| label, report | 20 / 24 h | 200 / 24 h |
| trust | 4 / h, 12 / 24 h | same |

Counting by `created_at` makes the limit identical on every node and closes the gap where backdated records arriving as catch-up skipped live-only limits.

### 9.2 Copy-paste floods

Text fingerprint = first 16 bytes of `BLAKE3(lowercase, whitespace-collapsed title‖text‖link or comment text)`, for texts of at least 12 characters. Refused: the same fingerprint from the same author within 7 days, and from a newcomer when two other newcomers posted it within 24 h.

### 9.3 Strikes, reports, autoban

A node strikes keys locally (rate 1.0, content 3.0, duplicate 0.25; half-life 1 h; bounce at 10), penalises peers that forward invalid bytes, signatures or identities, and publishes a signed `report` with evidence ids when it bounces a key. When the weighted reports for `(key, community)` reach `autoban_threshold`, the receiver bans locally. **Reserved for v2:** Rate-Limiting Nullifiers.

A bounce is one device's verdict, and devices on different releases may read the content rules a little differently, so it lifts on its own: after a day, twice as long each time the key is bounced again within 30 days of the last bounce lifting, never longer than 30 days. While bounced, nothing new the key signs is stored; what was stored before stays, hidden. A record made before the rules it breaks took effect earns no strike, only a refusal: for rate windows, the community's current limits (§6.1) and the receiver's own rules; for content and copy-paste floods, the receiver's own rules. Implementations note when they last tightened a rule and set that date some weeks after the release that carries it, so devices on the previous release can update first.

## 10. Discovery

- **Connected-node directory:** peers supporting `hello.fx = ["catalog", …]` exchange bounded snapshots of the open communities they hold (§14). Each listing carries the signed genesis and public join routes; receivers validate it, keep it in a bounded directory of their own between connections and starts (§14.4), pass it on to peers that list `directory`, and do not subscribe, fetch posts or dial routes until the person accepts its invite. Only communities whose genesis and current admission policy are both `open` are advertised. A node sends its catalog to its own devices and home node, and to anyone else only while it accepts other people's connections (a public node, §11): a browser's, a phone's or a non-relaying node's list of what it holds is nobody else's business. Names remain display labels: independently created same-name communities retain distinct keys and MUST NOT be merged automatically.
- **Managed public names:** an optional registry assigns one canonical ASCII name to one community key with an atomic first claim. Canonicalization trims surrounding whitespace and lowercases ASCII; valid names match `[a-z0-9][a-z0-9._-]{0,31}`. At community creation, the community key signs the exact UTF-8 bytes `hoi community name v1\n<canonical name>\n<64-character lowercase community hex>` (no trailing newline). Clients retain the public Ed25519 signature and discard the community secret key as usual. A claim sends the canonical name, community key and signature; the registry verifies key strength and signature and stores only the name → key mapping. This proves control of the claimed key, not possession of user data or a reachable copy of the community. Uniqueness is scoped to the configured registry, requires a successful online claim, and cannot be inferred from a disconnected node's directory. Existing communities without a retained proof cannot register using a discarded key.
- **Versioned public index:** open communities additionally sign `hoi community index v2\n<canonical name>\n<community hex>\n<protocolVersion>\n<indexVersion>` (UTF-8, no final newline). These immutable fields are bound at creation to the checked-in release manifest. The registry keeps name reservations separately from discovery metadata; compatibility filtering MUST NOT release names or relabel old proofs. Exact name lookup includes hidden reservations. Legacy v1 names without a versioned proof remain reserved but are not silently indexed. See `registry/README.md` for HTTP contracts and `RELEASES.md` for upgrades.
- **Public node announcements:** a native node that carries public communities MAY tell a registry where it is. It signs, with its transport (iroh endpoint) key, the exact UTF-8 bytes `hoi public node v2\n<registry origin>\n<64 lowercase hex endpoint key>\n<issuedAt, Unix seconds>\n<routes joined by ,>\n<community keys joined by ,>` (no trailing newline), and sends them with the signature as `{registry, endpoint, routes, communities, issuedAt, signature}`. `registry` is the serialized origin of the registry URL the node was given (`https://host[:port]`, lowercase, default port omitted, no path; `http://` only for `localhost`, `127.0.0.1` or `[::1]`): an announcement is good at one registry only, so a registry cannot replay what a node sent it to another one while it is fresh. The other fields: 1–8 of the node's own routes in the order others should try them (`rtc:` routes, or native routes naming this same endpoint), and at most 64 sorted keys of open communities it holds. The registry answers to one origin: its configured public URL's, or, when none is configured, the loopback address a connection reached it at (from the socket, never a header; for development); without either it takes no announcements (403 `announcements-disabled`). It refuses an announcement naming another origin (400 `wrong-registry`), the unnamed version 1 statement (400 `invalid-announcement`; nodes older than this rule must be updated), a bad signature, an `issuedAt` more than ten minutes from its clock and any route whose address lies inside a visitor's own network (loopback, private, link-local, carrier-grade NAT); it keeps one announcement per endpoint for thirty minutes (a newer one replaces it), keeps at most 64 nodes (when full, the nodes that were there first stay and a newcomer is answered with `retained: false`), and answers discovery with at most 16 routes: the operator's configured routes always first, then the announced carriers of the wanted community, then the rest, each group one route per node per round so no announcement crowds the others out. An operator MAY run in **allowlist mode**, taking announcements only from listed endpoint keys (others: 403 `announcer-not-allowed`, and nodes that announced earlier are no longer listed), and MAY drop a node's announcements by key (403 `announcer-refused` from then on). An announcement is a lead, not proof of reachability or of the records' presence: receivers still verify the signed genesis and every record.
- **Withheld listings:** a registry's operator MAY withhold a listing by name or by key. The reservation stays: a claim or publication touching that name or key is answered 409 `withheld` and changes nothing, and nobody else gets the name. Lookups by name or key answer 404 `withheld` without the mapping, prefix lists leave it out, mirrors therefore do not copy it, and `/v2/nodes?community=` answers for a withheld key as for an unknown one. Withholding signs nothing and changes no community: the key, its records and its invites stay valid wherever else they are held.
- **The log of names:** a registry appends every claim it retains (a reservation's v1 proof, a publication's v2 proof) to an append-only log, in the transaction that keeps it, once. An entry is the UTF-8 text `hoi name log v1\n<claim version>\n<name>\n<community hex>\n<protocolVersion or empty>\n<indexVersion or empty>\n<signature hex>`; the log is an RFC 6962 Merkle tree over SHA-256(0x00 ‖ entry) leaves. The registry signs heads with its own Ed25519 log key over `hoi name log head v1\n<log key hex>\n<size>\n<root hex>\n<timestamp>`. Any app may read `GET /v2/log/head`, `/v2/log/entries?start&end` (at most 256; a withheld listing's text is `null`, its leaf stays), `/v2/log/proof?leaf&size` (an audit path, or 404) and `/v2/log/consistency?first&second`. A client checks every exact lookup and write: the new head extends the last one it kept for that registry, and the entry is in it; otherwise it stops, as for a changed pin. A mirror (`--registry-mirror`) audits the whole log of the registry it mirrors: it keeps every leaf, recomputes the root, refuses a name that moves to another key or a key that takes a second name, and imports only logged names.
- **Registry nodes:** the registry is a role a node can play (`hoi-node --registry`), with the same HTTP contract and the same database shape as the standalone service, so anyone can run one and a database can move between implementations. Each registry is its own namespace: a name is unique within the registry that arbitrated it, never across registries or while every registry is offline. A registry MAY mirror another registry's listings: it imports a verified `hoi community index v2` proof when both its name and its key are free locally and leaves conflicts alone, so when one registry stops another holds its names. Two communities that end up with the same name under different keys are parallel communities; clients show both keys and MUST NOT merge them.
- **Carry requests:** a public node that lists `carry` in `hello.fx` takes `carry` requests (§14): the signed genesis of an open community, as an invite carries it, and nothing else. No routes travel with a request, so a stranger never makes a public node dial an address of their choosing; the node finds the community's members the way it finds everyone, from peers that prove they hold records. Anyone holding the genesis may ask: a member, or anyone with its invite. The node verifies the genesis exactly as a `catalog` entry, refuses what its operator refused (`refused`), private and invite-only communities (`not open`), takes at most 4 requests per peer per minute (further ones in that minute are ignored and count against the peer), accepts at most 8 new communities per minute and 128 per day whoever asks (`busy`), carries at most its budget of communities and of bytes (requests may fill half the communities; the rest is kept for an index), and answers `carried`. Once taken on, the community is pinned, the requester gets a fresh `hello` and `have`, other connected peers that carry it get `have`, and the node's next announcement to a registry it mirrors lists it; nothing about the requester is kept. A community carried on request is let go, and its records and media deleted (they were a stranger's word, never the operator's choice), when no other peer delivered or fetched records of it for 14 days (while it holds nothing but its genesis, a peer listing that genesis in a `have` counts too), when its records never arrived within an hour and nobody else listed its genesis, or when the byte budget is exceeded (the longest untouched first); index-listed communities are kept as long as the index lists them. A `carried { ok: true }` is not proof of storage: Warmth (§20) measures what arrived.
- **Public key join:** `#/public?c=<64 hex key>&name=<canonical name>&p=<protocol family>` asks carrying peers for the definition of an explicitly chosen key; no private invite or pairing token is required. After normal signature validation, the received genesis MUST match that key, any expected name, and initial open admission. Current admission can subsequently change through the signed ledger and governs posting; public records are not confidential. Unknown keys remain pending rather than being reported joined. Registry access is never part of record validation; saved invites and membership work independently with compatible peers.
- **Invite URI:** new links use `hoi://join?code=<base64url, no padding>`. Decoded bytes are `"HOI" || 0x02 || checksum[16] || masked_payload`. The payload is canonical DAG-CBOR with required fields `v: 1`, `c: bytes(32)` (community key), `r: [text]` (relay URLs), `p: [text]` (peer routes), and optional `i: bytes(32)` (invite record id), `g: bytes` (signed genesis wire), `t: [text]` (the community's trackers, 1–2). Empty route arrays are included. The checksum is the first 16 bytes of BLAKE3(payload). XOR the payload with the same number of bytes from BLAKE3's derive-key-mode XOF, using context `hoi invite obfuscation v2` and the checksum as input. Readers reverse the XOR, verify the checksum, then decode the CBOR. This deterministic public mask prevents a plain Base64 decode from exposing readable details; it has no secret and provides no confidentiality or authentication. The checksum detects damage only. The same query MAY follow `#/join?` in an HTTPS link; the fragment is decoded locally and is not part of the page request. Anyone with the link can recover its routes; obfuscation does not provide anonymity. No password or resolver service is required. Codes over 128 KiB of encoded text, truncated/damaged envelopes, unknown envelope or CBOR versions, mistyped/unknown CBOR fields, duplicate codes, or codes mixed with legacy fields are rejected. Readers also accept the previous unmasked Base64url CBOR code and the original `hoi://join?c=<community key z-base32>&r=<relay url>,…&p=<node id>,…&i=<invite record id>?&g=<genesis wire, base64url>?` format; old clients need an update to open the new envelopes. Obfuscation changes neither route acceptance nor genesis validation.
- Native `p` hints are a 64-character hexadecimal iroh endpoint public key, optionally followed by `@<SocketAddr>` (IPv6 uses brackets). **WebRTC routes** are `rtc:` + the base64url (no padding, 43 characters) SHA-256 of the node's DTLS certificate + `@<SocketAddr>`, for browsers and phones (§14.1); native nodes don't dial them. Each hint is percent-encoded. `r` hints are operator-selected HTTPS relay URLs. Hints are persisted only after accepting an invite (or from a paired device, or a home node that delivered records, §14) and removed on leaving its community. No built-in relay or discovery service is selected.
- **Routes on trial:** routes a registry or an announcement suggested for a public key join are only tried: kept in memory for that attempt, never persisted, never put in an invite, a catalog or a hint. A route is kept (as a learned route, below) only once the node it reaches delivered valid records of the community. A peer that a route on trial led to is not believed about the ids it lists; it proves itself by delivering records.
- **Routes in an invite:** at most 16, in this order: the inviter's own (at most 4 direct addresses, its bare endpoint id when it has relays, at most 3 WebRTC routes), the routes the inviter keeps (§14.2) for each public node it dialed and is connected to that holds the community, lists it or was just asked to keep it, the routes of the invite it joined with, then routes of members it met, newest first, and, for an open community, the kept routes of up to 2 more public nodes the inviter reached (§14.2), freshest first, so the invite leads into the network when no member's node answers. The same order without those last nodes is what catalog entries and the hints for a node's own devices carry. A node learns a member's routes only from a connection that proves them: the source address of an authenticated QUIC connection, `hello.routes` entries naming that same endpoint, routes on trial naming it, and at most 2 `rtc:` routes bound to the connection, once the peer delivered valid records of the community or (when no route on trial led to it) listed ids the node holds besides the genesis. An `rtc:` route is bound to a connection when it names the certificate fingerprint the node pinned when it dialed that peer over WebRTC (the dialed routes come first), or, over QUIC, the address the authenticated connection came from; any other `rtc:` route a peer offers is ignored, so a peer cannot point newcomers at someone else's address. At most 16 learned routes per community, forgotten 30 days after they were last confirmed, stored apart from accepted invite routes.
- **Pairing URI:** `hoi://pair?code=<base64url, no padding>`, an envelope like the node URI's: `"HOP" || 0x01 || checksum[16] || masked_payload`, the payload raw DEFLATE of DAG-CBOR `{v: 1, u: text, t: text, w: [text], n: text?}` masked under context `hoi pairing link obfuscation v1`, so no address or token is readable from the link; codes over 4 KiB, damaged codes and codes mixed with `u`, `t`, `n` or `w` are rejected. Readers also accept the earlier clear-text form `hoi://pair?u=<ws(s) url>?&t=<pairing token>&n=<node name>?&w=<fp>@<SocketAddr>,…?`. `w` lists at most 4 WebRTC routes without their `rtc:` prefix; `u` may be empty or absent when `w` is present. A plain `ws://` address is accepted only for this machine (`localhost`, `127.0.0.0/8`, `::1`): anywhere else needs `wss://` or WebRTC routes, since the token travels in the first message. The same query MAY follow `#/pair?` in an https link to the web app, which is what a QR code holds, so a phone's camera opens the app. A node MUST refuse clients without a valid credential in their first `hello`. **A pairing link pairs one device:** the node answers the first device that shows its token with a credential of that device's own (`hello.token`, §14) and replaces the link's token at once, so a link left in a screenshot or a browser history pairs nobody else. Devices show their next device the node's current link (`hello.pt`), never their own credential. A node keeps a hash of at most 32 device credentials (the least recently seen goes first); issuing a new link (`--new-pairing`, "new pairing link") forgets them all, and devices pair again.
- **Node lists (§14.2):** nothing in the software names a node, a registry or a relay to start from. A device finds its first node in an invite, a node link (below) or through its home node, and every node that serves strangers passes on, in `nodes`, the other public nodes it reached itself. Routes in a list are only tried, like routes on trial: a node goes into the list a device keeps and passes on only after it answered, on a link that device dialed, as a public node.
- **Founders' and inviters' requests:** a device asks each connected node that lists `carry`, unasked, to carry the open communities its person founded and that node doesn't hold (at most 3 at a time, paced like any `carry`). So a community made in a browser is kept by the public nodes that browser reaches, with no registry and no invite step. Making an invite to an open community is asking too: the connected nodes that list `carry` and don't hold it get a `carry` at once, and, for the rest of that run, so does each such node that says `hello`. Other communities someone else founded wait for the person to ask.
- **Node URI:** `hoi://node?code=<base64url, no padding>`. Decoded bytes are `"HON" || 0x01 || checksum[16] || masked_payload`; `masked_payload` is raw DEFLATE of canonical DAG-CBOR `{v: 1, w: [text], p: [text], n: bytes?, s: bytes(64)?}`: at most 4 of each, a public node's WebRTC routes (`<fp>@<SocketAddr>`, without the `rtc:` prefix) and native routes (`<endpoint>@<SocketAddr>`), then the operator notice's canonical bytes and its signature (§14.5). Empty route arrays are included. The checksum is the first 16 bytes of BLAKE3 of the deflated bytes, which are XORed with BLAKE3's derive-key-mode XOF under context `hoi node link obfuscation v1`, keyed with the checksum, exactly as for invites. Like an invite's, this public mask keeps a plain Base64 decode from showing the node's IP addresses; it has no secret, and whoever opens the link (or connects) learns them. Codes over 16 KiB of encoded text, damaged or truncated envelopes, an invite's envelope, payloads that inflate past 6,144 bytes, unknown versions or fields, a signature without a notice, duplicate codes and codes mixed with `w`, `p`, `nt` or `ns` are rejected. Readers also accept the earlier clear-text form `hoi://node?w=<fp>@<SocketAddr>,…&p=<endpoint>@<SocketAddr>,…&nt=…&ns=…` (`:` and `@` unescaped). The same query MAY follow `#/node?` in an https link to the web app. It is an address, never a credential and never a community: opening one only makes the node a place to look first. A node that serves strangers shows its own, its internet routes first.
- **Trackers:** an open community MAY name at most 2 trackers in its limits (`trackers: [text]`, written only when not empty, changed by consent like any limit): `https://` addresses of registries (plain `http://` only for `localhost`, `127.0.0.1` or `[::1]`), without credentials, query or fragment, at most 200 bytes. Invites carry them (`t`, below) and joining keeps them until the community's own limits arrive. A public node announces itself (the announcement above, signed for that tracker's origin) to the trackers of the open communities it carries, at most 16, besides any registry its operator named; it stops when no community it carries names one. A device asks a community's trackers (`GET /v2/nodes?community=<hex>`, readable from any origin) only when no route it holds answers, and only tries what they return (routes on trial). A tracker sees the asking device's address and which community it looks for. Trackers are the community's choice, never the app's or the site's.
- **LAN beacons:** a native node MAY send, every 30 s, a UDP datagram to `239.255.70.74:7848` (TTL 1): `"HOIL" 0x01 ‖ n (≤ 8) ‖ n × (tag[16] ‖ sealed[74])`, one entry per open or invite-only community it carries. `tag = BLAKE3-keyed(BLAKE3-derive-key("hoi 2026 lan tag v1", community), u64be(now / 600))[..16]`; `sealed = 24-byte nonce ‖ XChaCha20-Poly1305` under `BLAKE3-derive-key("hoi 2026 lan route key v1", community)` with the context `"hoi 2026 lan route v1"` as associated data (`hoi_crypto::sealed`), of `endpoint[32] ‖ u16be(port)`. A node that carries the community (the current or previous window's tag) takes `endpoint@<source address>:port` as a route on trial. Private communities send none. Receivers take at most 120 beacons a minute.
- **Find:** a public node lists `find` in `hello.fx` and answers `find { c }` with `found { c, r }`: routes of nodes it knows carry `c` (its own when it does, the public nodes it is connected to that hold it, its directory's listing), at most 16, ten finds per peer a minute. A device asks the connected nodes that list `find` (at most 3) about each joined open community no connected peer serves, after their `hello` and at each sweep, at most every 10 minutes per community; `found` routes from a node it asked go on trial. A node that runs the DHT (below) also looks the community up there and answers the peers that asked within two minutes again when the DHT does.
- **The DHT among public nodes:** a public node MAY run a Mainline DHT node (BEP 5, IPv4, UDP 7846 by default) with no outside bootstrap. Public nodes that list `dht` in `hello.fx` send each other `dht { a }`, at most 4 `ip:port` where their DHT nodes listen, and start (or, while their routing table is empty, restart) from those; an operator MAY add outside nodes. A node announces each open community it carries, at most 64, every 15 minutes, under the DHT key `BLAKE3("hoi dht v1" ‖ community key)[0..20]` with the port of its **answer port** (UDP 7847 by default). A query to that port is exactly 512 bytes: `"HOI?"`, `0x01`, the community key (32 bytes), a nonce (16 bytes), zeros; the answer, at most 1,200 bytes, is `"HOI!"`, `0x01`, the nonce, and the canonical CBOR map `{c: bytes32, e: bytes32, r: [text], s: bytes64, t: uint}` where `s` signs, with the endpoint key `e`, the UTF-8 bytes `hoi dht node v1\n<endpoint hex>\n<community hex>\n<t>\n<routes joined by ,>`. A node answers only for communities it carries, at most 30 queries a source a minute. A finder refuses an answer for another community, a bad signature, a weak key, a `t` more than 10 minutes ahead or 30 minutes old, and native routes naming another endpoint; what is left goes on trial. The community key never enters the DHT. Private communities never do.
- Not built in: **pkarr** and **mDNS** `_hoi._udp`. The first would name outside infrastructure; the second reaches only the local network.

## 11. Relays

A relay forwards QUIC packets by destination node id, keeps an in-memory routing table and nothing else, and MAY admit only some endpoints. Relays are untrusted: they see node ids, timing and sizes, never content.

**The embedded packet relay:** `hoi-node --packet-relay ADDR` runs iroh's relay server (`iroh-relay` 1.4.1, the protocol its clients speak), on plain HTTP behind a TLS proxy or with its own certificate, and puts its public URL (`--packet-relay-url`) first among the node's relays, so invites carry it in `r`. It admits every endpoint but those on the operator's block list (`hoi-node admin relay block KEY`, a text file the relay reads again when it changes), or with `--packet-relay-members` only endpoints the node knows as members of communities it carries: connected now and sharing one, or named by a route it learned or was invited with. iroh's clients send no credentials of their own, so membership by endpoint stands in for a bearer token. Each client may send 1 MiB a second, in bursts of 4 MiB. Relay URLs are `https://`; plain `http://` only for a relay on this machine.

**Member relaying** is the switch native nodes ship today: while it is on, a node accepts connections from other people's devices (QUIC peers, WebRTC browsers in mode `p`) and serves the communities it holds with the usual sync messages; off, it accepts only its own paired devices and still dials out. A daily cap in bytes, counted per UTC day over connections other people's devices opened, turns them away until the next day. Such a node is a member's device: it reads the records it holds, and passes on only records that pass its own checks; signatures make sure it can't change them.

## 12. Versioning

`v` in the envelope. Records of an unknown version are refused by the strict decoder. Clients SHOULD refuse to run more than one major version behind the newest they observe. Sync speaks version 2; a `hello` with another version is answered with `bye` (by a home node only after the device showed its credential). Version 2 (October 2026, a preview break) salts the communities a hello names to strangers (`n`, `ch`) and gives each paired device its own credential (`token`, `pt`, `lk`); version-1 nodes and devices cannot talk to it, and devices paired before pair again: the first one to reconnect takes the old link, the others need the node's next link.

## 13. Test vectors

`core/vectors/`; `cargo run -p hoi-validate --example vectors` regenerates them deterministically and `cargo test -p hoi-validate` fails if they are stale: every kind with its canonical bytes, id, signature and stamp (`records.json`, including identity work bits), the identity → delegation → post chain with the delegation statement (`delegation-chain.json`), a removal (`tombstone.json`), a 2-of-3 threshold entry (`threshold.json`), a sync transcript (`sync-transcript.json`), invite encodings (`uris.json`), a ladder scenario with the expected levels, voices, hidden records, quorums and a proposal's outcome, and the same proposal after a retirement (`ladder.json`), a PNG before and after sanitizing (`media.json`) and a picture's eight PDQ hashes (`pdq.json`). PDQ fingerprints are tested against Meta's published hash of its reference photo in `hoi-validate`.

The separate `community-registration.json` fixture contains a fixed test seed, public key, exact name-claim statement and signature. `hoi-core` re-signs and checks it; the registry's HTTP tests verify that same Rust-generated proof and reject a changed name.

`scatter.json` fixes scattered delivery's arithmetic and bytes (§21): the size for every level, several generations and numbers of places, readings, a Reed–Solomon codeword, a sealed object, one generation of it with all its pieces, hashes and locators and the subsets that rebuild it, drops, a holder's published key, every envelope and the manifest; `node docs/spec/scatter-params.mjs --check core/vectors/scatter.json` recomputes its sizing with the reference definition. `warmth.json` holds observations with the reading every implementation must give them (§20): level, score, copies, durable copies, what a connected node's hint changes and the hint a node attaches to its catalog. `carry.json` holds encoded `carry` and `carried` messages and a `catalog` entry with `w` (§14). `node-announcement.json` holds a version 2 announcement body for `https://registry.example` with its seed and exact statement (§10); `hoi-core`, the Node service, the Worker and `hoi-node --registry` all check it. `hello-salted.json` holds a salt, two community keys with their salted hashes, a hello to a stranger naming one and hashing the other, and a home node's hello to a device with its credential, next link and link key (§14).

`shamir.json` fixes the recovery shares (§4.3): a secret, the coefficients for each byte, the five shares. `records.json` carries a committed identity, a root that continues it, the rotation between them and a share record. `name-log.json` fixes the registry's log of names (§10): entries, leaves, every root, audit and consistency paths and a signed head.

## 14. Sync messages (home-node link, `hoi/sync/1`)

Canonical CBOR maps with a `t` field, one per WebSocket binary frame or one length-prefixed frame on the iroh `hoi/sync/1` bidirectional stream (four-byte unsigned big-endian length, then the CBOR message; zero or oversized lengths are refused):

```
hello { t:"hello", v:2, node: bytes32, name: str?, token: str?, c: [bytes32] (≤ 1024), n: bytes16?, ch: [bytes32]? (≤ 1024, only with n),
        serves: bool, hints: [{c:bytes32,p:[str],r:[str]}]?, routes: [str]? (≤ 8, ≤ 128 bytes each), fx: [str]? (≤ 16, ≤ 16 bytes each),
        pt: str? (≤ 256 bytes), lk: bytes32?, nh: bytes32?, nt: bytes? (≤ 2048, §14.5) }
have  { t:"have", c: bytes32, ids: [bytes32] (≤ 100000) }
want  { t:"want", ids: [bytes32] (≤ 5000) }
recs  { t:"recs", recs: [bytes] (≤ 500) }   // dependencies first: genesis, identities, delegations, …
bwant { t:"bwant", ids: [bytes32] (≤ 32) }  // media by BLAKE3 hash
blob  { t:"blob", id: bytes32, data: bytes (≤ 16 MiB) }
bye   { t:"bye", reason: str }
link  { t:"link", m: bytes16, b: bytes (1 B to 512 KiB) }   // only between a home node and its paired devices
catalog { t:"catalog", entries: [{g:bytes,p:[str],r:[str],w:uint?,x:uint?,a:uint?}] } // optional connected-node community listings
carry   { t:"carry", g: bytes (1 B to 64 KiB) }            // to a public node: keep this community (no routes)
carried { t:"carried", c: bytes32, ok: bool, reason: str? (≤ 256 bytes) }                                             // its answer
nodes   { t:"nodes", routes: [str] (≤ 32, ≤ 128 bytes each) }   // other public nodes the sender reached itself (§14.2)
find    { t:"find", c: bytes32 }                                // to a node that lists `find`: who carries c? (§10)
found   { t:"found", c: bytes32, r: [str] (≤ 16, ≤ 128 bytes each) }   // its answer
dht     { t:"dht", a: [str] (≤ 4, ≤ 64 bytes each) }           // DHT contacts, between public nodes that run it (§10)
bind    { t:"bind", s: bytes64 }                               // a node's channel binding on WebRTC links (§14.1)
rng     { t:"rng", c: bytes32, n: uint, s: bool, r: [[uint, bytes (≤ 32), 0] | [uint, bytes, 1, bytes16] | [uint, bytes, 2, [bytes32]]] (1 to 8192) }
                                                               // range reconciliation, to a peer that lists `range`
```

- `rng` reconciles one community's live records, ordered by `(created_at, id)`, with a peer that listed `range` in its hello; to one that didn't, a node sends `have`. `r` partitions that whole order: each range runs from the previous one's upper bound (or the start) up to its own, exclusive, given as `[created_at, id]` with the id's trailing zero bytes left off, and every bound above the one before; the last bound is the end of the order (`created_at` 2⁶⁴−1, id all `ff`). A range is settled (`0`), a fingerprint of the sender's records in it (`1`: the first 16 bytes of BLAKE3, key derived from `hoi 2026 range fingerprint v1`, over the sum of their ids as 256-bit little-endian numbers modulo 2²⁵⁶, in four little-endian 64-bit limbs, then their count as u64 little-endian), or the sender's ids in it (`2`). At most 100,000 ids in one message. `n` is how many records the sender holds there; `s` says the sender began this session.
- Each side begins a session of its own after hello for every community both carry, even one it holds nothing of yet, and learns from it what the other holds. The other side answers a fingerprint that differs from its own records there by splitting the range into 16 fingerprints of equal counts, or into its ids when it holds 64 or fewer there, and answers ids with its own ids, or settles the range when they are the same. It always answers, even when everything is settled. The side that began takes the ids it lacks from the other's ids, settles them, splits differing fingerprints in turn, and stops once nothing is open. A side that would need more than 8192 ranges answers the rest of the differing ranges with its ids, which always settles them. What it lacks is asked for with `want` in pages as after a `have`. A session the other side stops answering ends after two minutes. Private communities are never reconciled (§19).

- `catalog` is sent only after the receiver advertises that feature. Its entire encoded frame is at most 256 KiB, with at most 64 entries: a signed genesis `g` of 1–65,536 bytes, at most 16 public peer hints `p` of at most 128 bytes each, and at most 8 HTTPS relay hints `r` of at most 512 bytes each. A new snapshot replaces the prior one, including an empty snapshot; unchanged snapshots are suppressed. Receivers verify genesis shape, proof of work, signature, content and time before presenting it, discard invalid entries, and accept at most 60 snapshots per connection per minute. No posts, identity records or pairing credentials appear in this message. Directory changes refresh connected clients, including when a home node learns a new community from a paired phone. Catalogs received from third parties are never forwarded as this node's own holdings; to peers that list `directory` they travel as relayed entries (`x: 1`, with their age `a`), which a receiver keeps in its directory and never takes for the sender's holdings (§14.4). An entry's optional `w` (0–3) is the sender's own Warmth hint for that community: the durable copies it saw first hand besides its own, quantised (§20). Receivers keep it for the connection's lifetime, take the largest hint among connected nodes that list `keeps` as a maximum and never add hints up; a value above 3 is read as 3, and a changed `w` alone is not a changed listing.
- `carry` is sent only to a peer that listed `carry` in its `hello.fx`: a node that did not and receives one answers `carried { ok: false, reason: "not accepting" }` and counts it against the sender. `g` is a signed genesis exactly as in a `catalog` entry or an invite; a request carries nothing else, and receivers ignore any other field. The receiver verifies the genesis the way it verifies a catalog entry (a damaged one is the peer's fault, answered `invalid`), refuses what its operator refused (`refused`), private and invite-only communities (`not open`), beyond its budget (`full`) and beyond its node-wide acceptance rate (`busy`), and answers `carried { ok: true }` for a community it carries or just took on; it then sends the requester a fresh `hello` and `have`, and `have` to every other connected peer that carries the community. A requester sends at most 4 `carry` per peer per minute; a receiver ignores further ones in that minute and counts them against the peer. A node ignores a `carried` for a community it did not ask that peer to carry.
- Optional `hello.hints` carries at most 64 community entries, each with at most 16 peer hints (128 bytes each) and 8 relay URLs (512 bytes each). Empty hints are omitted; older version-1 peers ignore the field. Paired browsers send their accepted invite routes to their home node; home nodes send their current endpoint routes back so browser-generated invites can reach them. Receivers MUST ignore hints from unpaired internet peers and hints for communities not both advertised by the sender and joined locally. A device takes its home node's hints, and `rtc:` routes for a certificate the pairing link didn't name, only once that home node delivered valid records the device lacked: a pairing link could have come from anyone; until then they wait in memory for that connection. This is routing metadata, not community authority. The authenticated QUIC endpoint key establishes peer identity; a self-asserted `hello.node` or `serves` flag does not.
- Optional `hello.routes` is the sender's own routes (direct `endpoint@addr`, its endpoint id, `rtc:` routes), on every link. A paired device takes its home node's `rtc:` routes from it, so a pairing survives an address change; peers use it as above (§10). Optional `hello.fx` names messages the sender understands beyond version 1, and properties it asserts about itself: `link`, `catalog`, `profile`, `profile-color`, `nodes`, `carry` (it takes carry requests), `keeps` (its copy outlives the connection: a desktop app or `hoi-node`, never a browser or phone), `unlisted` (it publishes no address of its own, §14.3), `directory` (it takes relayed `catalog` entries, §14.4), `range` (it reconciles with `rng`), `blobs` (it fetches media over iroh-blobs and serves the media of the communities it pins that way), `signal` (a native node: it passes `sig` between browsers that said `direct`, and its WebRTC port answers STUN, §14.6), `direct` (a browser: it takes direct links with other members' browsers, §14.6). Senders MUST NOT send a `link` to a peer that didn't list it, an `rng` to one that didn't list `range`, a `carry` to one that didn't list `carry`, `nodes` to one that didn't list `nodes`, nor a relayed `catalog` entry to one that didn't list `directory`. `keeps` is the sender's own word, used only to read Warmth (§20).
- **`link`** lets a device paired with a home node be linked to your key without copying codes. The secret is the home node's link key (`hello.lk`, 32 random bytes the node gives every device it pairs; each device's credential is its own, so it can't be the secret); `m` = the first 16 bytes of `BLAKE3-derive-key("hoi 2026 link mailbox v1", secret)`; `b` = 24-byte nonce ‖ XChaCha20-Poly1305 ciphertext under `BLAKE3-derive-key("hoi 2026 link box v1", secret)`, with that context string as associated data. Plaintexts are canonical CBOR maps: `{t:"req", code, c: bytes32}` (the link request code and a commitment `c = BLAKE3-derive-key("hoi 2026 link commit v1", N)` to a random 32-byte `N`), `{t:"chk", device: bytes32, n: bytes16}` (a root device's random nonce `R` for that request), `{t:"rev", device, n: N, to: R}` (the new device reveals `N` to that root device; at most 3 per request), `{t:"ok", code}` (the approval code, carrying up to 64 geneses and 256 KiB of them), `{t:"no", device}`, and for link previews `{t:"pv", u}` (a device asks its home node for the preview of `u`, at most 8 a minute) and `{t:"pvd", u, s, ti?, d?, i?, x?}` (the home node's answer: site, title, description, the picture's sanitized bytes, or `x` when the page couldn't be fetched). Only a home node answers `pv`, and only from its own devices; the device sanitizes the picture again. A home node passes a `link` from one paired device to all its others, unread, and processes it itself. The check digits are the first four bytes, little-endian mod 10000, of `BLAKE3-derive-key("hoi 2026 link check v1", device ‖ N ‖ R)`; the new device shows them on `chk`, a device holding the root key shows the request with them only after a `rev` whose `N` matches `c`, and answers only on the person's yes (`hoi-node`, with nobody to ask, doesn't answer). Since `N` is fixed before `R` is drawn and `R` before `N` is seen, neither side, nor any other device holding the link key, can steer the digits (numeric comparison, as in Bluetooth pairing). Approvals go through the same delegation checks as the manual codes. At most 12 `link` messages per peer per minute are acted on.
- **Remote signing.** A request code with `remote: 1` asks for a device that keeps no signing key: the root device makes the device key, blesses it, signs its consent, keeps it, and the approval carries the finished delegation in `remote`. That device then sends `{t:"sign", device, rec: bytes}` (the canonical, stamped record, authored by `device`) over the same `link` envelope; the device keeping the key answers `{t:"signed", device, id: bytes32, sig: bytes64}` (Ed25519 over the record id) or `{t:"unsigned", device, id, reason}`. It signs every kind but `ledger` at once and asks the person first for `ledger` entries. A revoked device's key is dropped and nothing more is signed for it.
- `hello.name` and the device's own stable id go only to the sender's own devices and its home node. To anyone else (members' nodes, public carriers) a node sends no name, and a browser sends a fresh random `node` per connection, so one carrier cannot tie a person's visits together.
- **Communities in a hello.** Between a home node and its devices, `c` lists every public community the sender is in. To anyone else, `c` names in the clear only the communities whose routes (accepted invite, learned, or on trial) reach that connection's authenticated endpoint or the WebRTC certificate the sender dialed: what a member came to a carrier for, so a carrier can take it on (`offered_unknown`). The sender's other public communities go in `ch` as `BLAKE3-derive-key("hoi 2026 hello community v1", n ‖ key)`, sorted, under a random 16-byte `n` drawn per connection. A receiver matches `ch` against the keys it holds itself, keeps `n` and `ch` for the connection, and matches them again when it takes a community on later (a `carry`, an index, an invite), so a member connected before is found; no key it didn't know can be read from them, and the same person's lists don't match across connections. Private communities appear in neither. A device that wants a public node to keep a community it doesn't hold sends `carry` with the genesis.
- **Pairing credentials.** A home node says nothing to a device that dialed it until the device's first `hello` shows a credential: no guarded hello, no version, and a missing or wrong one ends the link without a `bye` (a web page can't use the listener to learn that a node runs here). The credential is the pairing link's token, or a credential the node issued to that device. For the link's token the node issues a credential of 32 random bytes (base64url), keeps only `BLAKE3-derive-key("hoi 2026 pairing credential v1", credential)`, replaces the link's token, and answers with `token` = the device's credential in its `hello`; the device uses it from then on, and remembers a digest of the spent link (`BLAKE3-derive-key("hoi 2026 spent pairing link v1", token)`) so opening that link again keeps its pairing. Every `hello` a home node sends its devices carries `pt` (the token of its current pairing link, for showing the next device) and `lk` (the link key). A link that showed a credential stays trusted for its lifetime; a new pairing link ends every device's link with its next message.
- Control text fields are bounded before being retained: `t` at 16 UTF-8 bytes, `hello.name`, `hello.token` and `hello.pt` at 256 bytes each, and `bye.reason` at 1,024 bytes.
- Messages over the caps, or over 16 MiB + 64 KiB in total, MUST be refused. The first message MUST be `hello`; anything else first ends the connection. Peers that send more than 3000 messages a minute lose score.
- Receivers sort a `recs` batch by dependency (genesis, identity, delegation, revoke, ledger, vouch, post, comment, edit, review, vote, label, report, delete; then `created_at`) and settle the ladder before contributions, so people admitted by vouches in the same batch post with their new standing. Records newer than 10 minutes count as live. A node that learns a community from one peer (its genesis arrives) offers `have` for it to every peer that listed it in `hello` and doesn't take `rng`.
- Current clients request catch-up in pages of at most 200 record IDs, advancing only after the prior page arrives; the entire advertised set is retained until complete, with at most 100,000 pending IDs per peer. The wire decoder still accepts up to 5,000 IDs per `want`, but the record response is capped at 16 MiB before allocation; clients must use bounded pages rather than relying on an arbitrarily large response. Hosts reconnect an active catch-up after 120 seconds without a requested record arriving, even if unrelated traffic continues. This refreshes inventories when a record expires or is removed between `have` and `want`; it never marks an incomplete page as synchronized. Healthy idle links remain open. Full-ID `have` lists remain capped at 100,000, and range reconciliation takes the newest 100,000 live records of a community.
- Current clients request one missing media blob at a time, advancing on receipt and retrying missing media after the retry interval. Unattempted media is tried before timed-out media, then the least recently attempted item, so unavailable media cannot starve later items. Responses to older multi-blob requests may be a bounded prefix (at most 16 MiB of payload); omitted blobs remain eligible for retry. This avoids filling bounded transport queues with a valid multi-video catch-up.
- A `blob` whose bytes don't hash to its id is the peer's fault (penalise); bytes that hash right but fail §17 checks are the author's (§17).
- **Media over iroh-blobs.** Between two native nodes that both list `blobs`, the next missing blob is fetched over iroh-blobs (ALPN `/iroh-bytes/4`) on the same iroh endpoint, in place of a `bwant`: one `get` of the whole blob by its hash (iroh-blobs hashes are the BLAKE3 hashes media already has), its size exactly the one the post states, within 60 s. The bytes then go through the same checks as a `blob`. A node takes an iroh-blobs connection only from an endpoint it has a sync link with, and answers a `get` only for media of a community it pins and that link shares, that isn't private and that it holds; it refuses `get_many`, `push` and `observe`. When a fetch fails the fetcher sends that peer a `bwant` for it, and asks that peer for that community's media with `bwant` for the rest of the link.
- Hosts SHOULD cap WebSocket messages at the same size, finish handshakes within 10 s, drop links whose `hello` doesn't arrive within 15 s, and cap concurrent paired clients.

### 14.1 WebRTC data channels (browsers and phones)

A native node listens on UDP as an ICE-lite peer with one long-lived DTLS certificate; its `rtc:` route names the address and the certificate's SHA-256. No STUN, TURN or signaling server is involved:

- The browser makes its own certificate and picks one ICE username fragment, used for both sides and also as both passwords: `h1` + mode (`c` a paired device, `p` a member's browser) + its certificate's SHA-256 in standard base64 without padding (43 characters) + at least 8 random ICE characters (at most 256 in all). It writes the node's half of the session description itself: `a=ice-lite`, `a=setup:passive`, the route's fingerprint, the shared username fragment and password, and one host candidate per route of that node.
- Every binding request carries the fragment, so the password, in the clear: ICE consent proves nothing about who sent a packet or from where. The node therefore answers a first binding request whose username is that fragment twice (`x:x`) with a binding request of its own to the same address (same username and password, `ICE-CONTROLLED`, a transaction id the node can check without keeping anything) and holds no state for it. The browser, a full ICE agent, answers; that answer shows it receives at the address, and only its next binding request from there starts a session. The node takes the browser's certificate fingerprint from the fragment and requires it in the DTLS handshake, in which it is the server and asks for a cookie (`HelloVerifyRequest`, or a `HelloRetryRequest` cookie in DTLS 1.3) before any key exchange. Mode `p` sessions are refused while relaying is off (§11). Nodes cap sessions (32, 8 still opening), new sessions per source (an IPv4 address or IPv6 /64: 10 a minute), answered probes per source (30 a minute), and drop sessions whose channel isn't open in 15 s.
- One data channel, negotiated out of band with id 0, ordered and reliable, carries the sync messages, each preceded by its four-byte big-endian length like the iroh stream, split into data channel messages of at most 16 KiB. Zero and oversized lengths end the session. Mode `c` devices still send their credential (or the pairing link's token) in their first `hello`.
- Right after its `hello`, the node sends `bind { s }`: its endpoint key's signature over `hoi/rtc-binding/1\n` + node fingerprint hex + `\n` + browser fingerprint hex + `\n` + username fragment + `\n` + endpoint key hex. The browser takes only `hello`, `bind` and `bye` until that signature checks against the key the hello named and the channel it opened; anything else first, or a bad signature, ends the link. Browsers bind nothing: they name no lasting key to strangers.

### 14.2 Node lists

A node sends `nodes` after the peer's `hello` and whenever its list changes, to its own devices and home node always, and to anyone else only while it accepts other people's connections (§11): the message itself, even with an empty list, tells a stranger that this node serves strangers. A list holds routes of other public nodes the sender reached itself, freshest first, at most 32 routes; it never repeats what it was told, and never names the receiver itself or a node the sender failed to reach in the last 10 minutes. Unchanged lists are not sent again.

- **Scopes.** A route's address is on this machine (loopback), on a local network (private IPv4 ranges, link-local, carrier-grade NAT 100.64/10, IPv6 unique-local and link-local) or on the internet. A sender lists a route for a receiver only when the route reaches at least as far as the receiver's own connection came from, and a receiver takes a route only on the same condition about the sender: an internet peer never hears of, and can never point a node at, an address on a local network or this machine. A node's own devices and home node hear every scope.
- **Taking a list.** A receiver takes a list only on a connection it dialed, or from its own devices or home node. From a peer that dialed in it takes nothing that peer listed, only that peer's own native routes on the address its authenticated connection came from (at most 2), to dial back once. It takes at most 16 routes from one list and acts on at most 6 lists per peer per minute (more are ignored), ignores bare endpoint ids (they need relays a list never carries) and routes it already keeps, and holds what it took in memory for an hour, at most 32 routes, freshest first. A dial that fails drops every route taken for that node.
- **Keeping a node.** When a peer this device dialed sends `nodes`, it answered as a public node: the device keeps it, at the front of at most 32 kept nodes (persisted, forgotten 30 days after they last answered), with the routes bound to that connection exactly as for a learned member route (§10): the source address of the authenticated QUIC connection and `hello.routes` naming that endpoint, the `rtc:` routes it dialed with the pinned certificate, and `hello.routes` `rtc:` entries with that certificate or on that QUIC source address; at most 2 of each kind. Only kept nodes are passed on. A node link opened by the person is kept too, but tried first and passed on only once it answered.
- **Dialing.** A browser or phone keeps one link to a public node from its kept and taken routes (links opened by the person first, then kept nodes freshest first, then taken routes freshest first), besides its communities' links. A native node that serves strangers keeps links to at most 3 public nodes besides its communities' peers, so it hears of new ones and is heard of. Neither dials a node it is connected to already, nor one that failed in the last 10 minutes.

A list can make a device send packets to at most a handful of addresses an hour, each of them a route another node claims for itself; every record still passes the receiving device's checks, and a connection is still not a delivery receipt.

A node link (`hoi://node?code=…`, §10, or the same query after `#/node?` in the web app's address) names at most 4 routes of each kind and no credential, packed and masked so its addresses are not readable from the link. A node with an operator notice adds it and its signature (`n`, `s`; `nt=` and `ns=` in the earlier clear-text form, §14.5).

### 14.3 Only public nodes publish an address

A node that isn't a public node (a browser, a phone, a desktop app whose person didn't turn on *Be a public node*) publishes no internet address of its own: its `hello.routes` to anyone but its own devices and home node, its invites, and the hints it gives its own devices (which they pass on in theirs) carry its routes on this machine and on a local network only, never one on the internet; its own devices and home node still hear every route, and its pairing links still carry them. A headless node (`hoi-node`) publishes what its operator's flags say, as before. Such a node lists `unlisted` in `hello.fx`. A receiver MUST NOT remember the address a connection from an `unlisted` peer came from, nor the routes it gave, as a member route (§10), and so never passes it on in invites, catalogs or node lists; it MUST NOT dial an `unlisted` peer back from a `nodes` list (§14.2). An older node that ignores the feature remembers it as before; the address it learns is the one that connection came from, which that peer saw anyway.

An invite leads to public nodes: its routes are, in order, a public node's own routes (only when the inviter is one), the public nodes connected now that hold it or were just asked to keep it, the routes of the invite the inviter came with, the member routes it remembered (nodes that delivered the community's records), a non-public inviter's own routes on this machine and its local network, then, for an open community, up to 2 more public nodes the inviter reached (§10). A device shows, before an invite is shared, every address in it with whose it is (`yours`, `node`: holds it or was asked to, `entry`: a way into the network) and how far it reaches, and whether anyone on another network can use it.

**Spreading the load.** A public node that carries or is in an open community, and sees fewer than 3 durable copies of it, its own included (§20's `durable_copies`), sends a `carry` for it to another node that lists `carry` when that node says `hello`; at most 2 such requests per community per UTC day, and at most 3 to one node per `hello`. The receiver decides as for any request (§14).

### 14.4 Directory gossip

A catalog (§10) lists what its sender holds; a node also passes on what it heard of, so every node comes to know the open communities any of them carries, with nothing configured.

- **Remembering.** A receiver keeps every `catalog` entry it verified (§10) in a directory of its own, by community key: the signed genesis, at most 8 routes and 4 relays (the newest catalog's first), when it first heard of it, and `held`: when a node last listed it as its own. An entry the sender holds sets `held` to now; a relayed one (below) to now less its `a`. A device keeps at most 512 listings (a browser or phone) or 4096 (a native node); over that, listings only peers that dialed in brought go first, then the longest unconfirmed. It forgets a listing 21 days after `held`, takes at most 128 new listings from one connection an hour, and drops a listing at once when the node that listed it as its own stops doing so and no other connected node lists it (a changed admission withdraws it everywhere a hop away; held elsewhere, it comes back). A listing heard only from this device's own home node or devices is marked so and MUST NOT be passed on to strangers: it may be its person's own community. The directory is local state, written at most every 10 minutes (30 s after something new), and an unreadable directory starts over.
- **Relayed entries.** A peer that lists `directory` in `hello.fx` takes `catalog` entries marked `x: 1` (relayed): open communities the sender heard of and does not list as its own, with the routes where it heard they are carried, and `a`: seconds since a node last listed it as its own as far as the sender knows (at most 60 days). Passing a listing on never makes it younger, so one nobody holds any more fades from every directory instead of circling between them. An entry without `x` is the sender's own, as before; peers without `directory` never get an `x` entry. `x` and `a` of another shape are ignored. A relayed entry is never part of the sender's snapshot: it neither replaces nor withdraws what the sender holds.
- **Windows.** To a peer that lists `directory`, a catalog holds at most 48 of the sender's own communities (the snapshot); the rest of its 64 entries and 256 KiB is a window of relayed ones: own communities that didn't fit, then at most 16 listings first heard of within the hour, newest first, then the others in key order from where the previous window to that peer ended. Never a listing the peer itself lists as its own, one too old to pass on (`held` more than 7 days ago), a refused or private one, or, to a stranger, one heard only from this device's own devices. The window is picked again every 5 minutes (each host's sweep) for a peer that lists `keeps`, every 30 minutes for one that doesn't (a browser or phone passes little on, and a public node serves dozens of them), and with every `hello`; a catalog resent for other reasons stays the same catalog, and unchanged catalogs are not sent.
- **Who passes on.** A node passes its windows on to whoever gets its catalog (§10). A device that is not a public node (a browser, a phone, a desktop that isn't public) passes on only what strangers' public nodes listed, only to peers that list `carry` and that it dialed itself, and never its own communities: those stay salted hashes (§10). So what a browser learned on one public node reaches the next one it dials.
- **Choosing a node.** A browser or phone dials up to 3 of the public nodes it keeps (§14.2) at once and keeps the first that opens; the others are closed before they carry anything, and losing that race is no failure. The node that answers first serves it.

A listing is a lead, not a copy: routes in it are only tried once the person joins (§10), and Warmth (§20) counts only what arrived.

### 14.5 Operator notices: informed before connected

A public node's operator tells people who runs it before the node serves them. The notice is a `NodeNotice` (hoi-types `notice`): version `v` (1), the node's label `n`, the operator's name or pseudonym `o`, contact `c`, the server's country `k` (ISO 3166-1 alpha-2), each of the three possibly empty (an operator may leave who and where out; `o`, `c` and `k` are still written, empty, so readers that expect them read the notice, and a reader treats them as empty when absent), provider `h`?, what the node processes `p`, whether the host logs addresses `l` and for how many days `r`, purpose `u`?, community rules `g`?, more `x`?, and the date `d` (`YYYY-MM-DD`). Its canonical form is the DAG-CBOR map of those one-letter keys, text trimmed, empty optionals left out, at most 1,536 bytes; its hash is `BLAKE3(canonical)`. The fixed sentences people read around the fields are the client's, the same everywhere (`NodeNotice::sections`), so an operator cannot remove the parts that say what every node sees; where `o`, `c`, `k` or `h` are empty, those sentences say what the operator left out (no name, no contact and so no way to ask about one's data, no location). Only `p` (what the node processes) must be present for a notice to decode.

- **In the hello.** A node that carries a notice puts its hash in `nh` and its canonical bytes in `nt` in every hello to a stranger (not to its own devices or home node), and again, as a new hello, when the notice changes. A receiver checks `BLAKE3(nt) == nh` and that `nt` decodes; a hello with `nh` and no `nt`, or with bytes that don't hash to `nh`, is the peer's fault (penalise). Keys a reader doesn't know inside `nt` are ignored; the bytes are kept as received, so the hash stays theirs.
- **In the node link** (§14.2): `n` (the canonical bytes) and `s` (`sig`) in the code; the earlier clear-text form had `nt=<base64url(DEFLATE(canonical))>` and `ns=<base64url(sig)>`. `sig` is the Ed25519 signature by the node's endpoint key (the key named in `p`) over the UTF-8 statement `"hoi node notice v1\n" ‖ endpoint hex ‖ "\n" ‖ hash hex`, no trailing newline. A link with `nt` and no `ns`, or whose signature doesn't verify for the endpoint it names, or that names no endpoint, is not opened. Deflated, a plain notice keeps the link within a camera-readable QR code; inflation is capped at 2,048 bytes.
- **The consent gate** (a person's device: browsers, the desktop, phones; never a headless node). To a stranger it dialed, the device sends nothing, not even its hello, until the stranger's hello arrived: with no `nh`, or with a notice the person accepted before (by hash), the device's hello follows at once; otherwise the device keeps the notice to show it, ends the link without a word, and dials the node again only once the person accepted. A declined node is not dialed, and a hello from it is answered with silence, until the person forgets the decision. Routes that lead to a node whose notice is pending or declined are left out of the device's own dial lists (candidates, discovery, node lists taken). To a stranger that dialed in, the device is the node, and says hello as ever. A link the person opened is refused until its notice, verified as above, was accepted; a link without a notice opens after the app's own warning. What a node sees before the gate closes is the connection itself (the device's address and certificate), which the client's Network & Safety notice says before the first connection is ever made; what the device says of itself (its communities, named or hashed) waits for the answer.
- A node that answered with a notice is dialed again within the usual 15 s first-frame window once accepted, since the gate closes the link rather than holding a session open while a person reads.
- A node that sends a notice also sends its configuration declaration (§14.7); the person's answer then names both, and a changed declaration is a new question like a changed notice.

### 14.6 Direct links between browsers

- A native node lists `signal` while its WebRTC listener runs and it takes members. To each link that listed `direct` it gives a random 16-byte handle for that link, and sends `here { c, y, h }`: a public community `c`, the link's own handle `y`, and the handles `h` (at most 16, sorted) of the other links that listed `direct` and share `c`. It sends a `here` again whenever that list changed, and an empty `h` when nobody is left.
- `sig { p, d }`: from a browser, `d` (1 to 8,192 bytes, an offer or an answer) for the link whose handle is `p`; the node passes it on as `sig { p: sender's handle, d }` only between two links that listed `direct` and share a public community, at most 30 a minute from each. A browser that didn't list `direct`, or a node that didn't list `signal`, ignores both.
- `d` is a 16-byte key, random per start of the app, then UTF-8 JSON `{type: "offer"|"answer", sdp}`. A browser takes nothing whose key is its own, nor one whose key it already has an open or opening direct link with under another handle; should two open with the same key, both sides close all but the lowest by (the offering side's key, the lower handle, the higher handle). Of two handles in a `here`, the numerically lower (bytes compared in order) offers. Both sides use the node's `rtc:` addresses as `stun:` servers and no others, gather their candidates (3 s at most) before sending, and negotiate one data channel with id 0, ordered and reliable, framed as in §14.1. A side keeps at most 4 direct links open and offered together, and offers again to the same handle after 10 minutes.
- The node's WebRTC port answers a STUN binding request with no USERNAME and no MESSAGE-INTEGRITY with a binding success response carrying XOR-MAPPED-ADDRESS (RFC 8489), at most 20 a minute per source and 6,000 a minute in all, and only while it lists `signal`.
- Over a direct link both sides are peers that dialed nobody: hello as to a stranger, no `bind`, and neither side's relay switch applies.

### 14.7 Configuration declarations, blocks and the assurance policy

RFC-0006. A notice (§14.5) is the operator's words; a declaration is what the node's software says about how it runs. Neither proves what runs: a node's own key signs both, and a modified program signs whatever it likes. Verified execution needs protected remote attestation under an approved profile, which this version does not implement.

- **The declaration** is a `NodeDeclaration` (hoi-types `declaration`), generated by the node from its effective configuration, never written by the operator: version `v` (1), the endpoint key `e`, the hash of the notice it goes with `nh`, the software `s` {name `n`, version `v`, and `d`, the BLAKE3 of the program file the host started, when it could read it}, and the roles it serves with their configured limits: relaying for members `r` (bytes a day), carrying open communities `c` {policy `p` `any` or `index`, communities `n`, bytes `b`}, keeping encrypted pieces `h` (bytes), passing piece requests on `f` (true), serving its operator's own devices `m` (true). Absent roles are left out. Its canonical form is the DAG-CBOR map of those keys, at most 512 bytes; its hash is `BLAKE3(canonical)`. No secrets and no membership lists. A reader keeps unknown keys in the bytes (and so in the hash) and says it cannot show them.
- **In the hello.** A node that sends a notice sends its declaration beside it, `dc` = {`d`: canonical bytes, `s`: the endpoint key's Ed25519 signature over the UTF-8 statement `"hoi node declaration v1\n" ‖ endpoint hex ‖ "\n" ‖ hash hex`}, and a new hello whenever it changes: a new build, a changed role or limit, a new notice. A `dc` that is not a map of bytes `d` (at most 512) and a 64-byte `s` makes the hello undecodable. Older readers ignore `dc`. A declaration without a notice names none and means nothing. Node links do not carry it.
- **Checking** (under the consent gate of §14.5). Before anything else, a device checks that `d` decodes, that `e` is the endpoint the hello names and, where the transport proved an endpoint, that one, that `nh` is the hash of the notice beside it, and that `s` verifies under `e`. A failure stops the link: nothing queued goes out, the link ends, and the device blocks what the connection proved (the endpoint key the transport or a binding showed, the certificates its `rtc:` routes pin), never a key a hello merely names. On a WebRTC link it dialed, the device keeps an accepted hello unanswered until the node's `bind` (§14.1) checked out, and then reads it again; a `bind` that fails blocks that link's certificate the same way.
- **The answer** a person gives names the declaration's hash when the node sent one (it names the notice too), else the notice's. A changed declaration, an earlier one, or none where one was accepted is a new question: the link stops until the person answered again. Clients show the declaration after the notice: the software, the roles and limits, and that nothing was checked beyond the node's own signature; for a notice without a declaration, that nothing was declared.
- **Blocks** are kept across restarts (at most 256; the oldest go first), leave the node out of every dial list, and answer its hellos with silence until the person reconsiders. They name endpoints and certificates, not people: a new key is a new node, and a block is no proof of bad intent.
- **The assurance policy** is a device setting, or a headless node's `--assurance`: `declared` (the default) uses nodes as above; `verified` uses only participants whose execution was verified under an approved profile, and none can be in this version. Under `verified` a device or node dials nobody on its own initiative, says nothing to a stranger it dialed or to its home node and ends those links (with `bye` when it switches while linked), never falls back to declarations by itself, and reports itself offline with the reason. Strangers that dial in, and its own paired devices, are served as before. A headless node under `declared` checks no declaration and asks nobody, as before.

## 15. The ladder: earning standing

A person's standing in a community is a pure function of the community's signed records (genesis, ledger, vouches, reviews, votes, posts, comments) and the clock. Two nodes holding the same records compute the same standing.

### 15.1 Admission

*Founders* are the owners (since the genesis) and active moderators (since their `mod-add`). Replay positive vouches (`value = 1`, latest per pair) and `admit` entries in `(created_at, id)` order:

- A vouch **counts** if its author is not banned, burned or blocked, did not get struck before the vouch (§16.5), and was **established at the vouch's time** (its effective time, §15.5): a founder by then, or admitted at least `established_age` earlier.
- A person is **admitted** at the time of the vouch that brings their counted vouchers to `min(admit_vouches, max(1, E_t))`, where `E_t` is the number of established people at that time; or at the time of an `admit` entry. Admission counts people; voice (§15.4) weighs them.

### 15.2 Levels

| Level | Who | May |
|---|---|---|
| **newcomer** | not admitted, or blocked | text only (no links, no media) |
| **member** | admitted | + links; votes count toward response |
| **established** | admitted ≥ `established_age` ago | + vouches, reviews and vetoes count |
| **trusted** | established, ≥ `trusted_posts` live posts/comments created in the last `min(90 d, max_retention)`, and response ≥ `media_score` | + media |

Owners and moderators are trusted. Response = Σ over those contributions of `ups + oks − min(downs, 2) − 3·hides`, minus 5 per moderator removal of theirs in the window, where ups/downs are distinct members' latest votes (not their own), oks and hides are counted reviews (§16). Standing that depends on activity fades when the activity stops: people have to contribute to keep media.

### 15.3 What receivers do with it (holds)

A post, comment or edit is **held** — stored, shown to nobody but its author, never passed on (§7), media never fetched — when its author (not an owner or moderator) is below the level its content needs: links (link field, `http(s)://` or `www.` in title or text, or a markdown link target) need member; media needs trusted; in `invite` communities, any record needs at least one counted vouch. Posts whose media failed checks or is on the node's blocklist are held too. Holds are re-evaluated whenever standing changes. Senders MUST NOT publish what their own standing doesn't allow.

### 15.4 Voice

Hides, blocks, vetoes and objections weigh **voice**, in integer units of `VOICE = 720720` (divisible by every count from 1 to 16, so a voice shared among a few people adds back up). Founders (owners, active moderators) hold `VOICE`. Everyone else's voice is built from **gifts**: every counted vouch, and every `admit` entry (the admitting moderator's gift), replayed in `(time, id)` order:

```
epoch(t)        = t / VOUCH_EPOCH                  VOUCH_EPOCH = 90 days
n(g)            = gifts by g.from in epoch(g.at)
voice_at(x, t)  = VOICE if x is a founder since ≤ t, else x's voice after the gifts before t
on gift g:      ceiling(g.to) = max(ceiling(g.to), voice_at(g.from, g.at))
                voice(g.to)   = min(VOICE, ceiling(g.to), voice(g.to) + voice_at(g.from, g.at) / n(g))
```

Each established member gives away at most their own voice per epoch, split evenly among everyone they vouched for in it, and nobody holds more than their strongest voucher: friends vouched in together get a sliver each and can't, together, lift anyone above what they hold. Blocked people and newcomers hold none.

### 15.5 Late records

A node records, per community, when it last **caught up**: a peer's `have`, or a finished range session, for which it wanted nothing, or one whose every wanted record has since arrived. A `vouch`, `review` or `ledger` record that arrives (from a peer or an import) with `created_at + 1 h` earlier than that moment wasn't out there when the node caught up, so it **counts from when it arrived** (its effective time), wherever this document says "time" for it. Nodes persist these arrival times. A node that joined after a record's `created_at` can't tell and uses `created_at`; standing on such nodes catches up within `established_age`. This is the one place where two nodes holding the same records may disagree, by design: long-standing members' nodes are the ones not fooled by backdating.

### 15.6 Reading counts (client policy, informative)

An established member's client that showed them ≥ 2 records by a newcomer, or by a member whose voice is under a full one and whom they haven't vouched for, which they neither hid, flagged nor muted for 10 minutes, publishes a `vouch` for them (setting `autoVouch`, on by default). Validation becomes a side effect of reading, and so does filling out a thin voice; nothing about reading is published except the vouch, and the notes it takes stay on the device for at most 30 days.

## 16. Community review: hides, blocks, vetoes

Only **jurors** count: established members who are not struck (§16.5). Quorums are voice (§15.4); they never exceed a third of the established voice and never drop below one voice, and every hide, block or veto also needs at least two distinct people:

```
quorum(q) = clamp(q · VOICE, VOICE, max(VOICE, ⌈E_v / 3⌉))     E_v = established people's voice now
```

### 16.1 Hiding records

Hide reasons are a closed list: `spam`, `abuse`, `illegal`, `off-topic`, `rule:<n>` (1–3 digits). Disagreement is not a reason; downvotes are for that. For a post or comment: *hiders* = jurors with latest verdict `hide` (not the author, not overruled §16.5), *defenders* = jurors with `ok`; `W(·)` is their voice. The record **naps for everyone** (hidden, peekable, kept) when there are at least two hiders, `W(hiders) ≥ quorum(hide_quorum)` and `W(hiders) > 2·W(defenders)`. It is **defended** when there are at least two defenders, `W(defenders) ≥ quorum(hide_quorum)` and `W(defenders) ≥ 2·W(hiders)`; every hide on a defended record counts as overruled for its author.

### 16.2 Blocking people

For a person (not an owner): hiders = the union of hiders of their live records and jurors whose latest vouch for them is `-1`; defenders = the union of defenders of their records and jurors whose latest vouch is `1`. They are **blocked** when there are at least two hiders, `W(hiders) ≥ quorum(block_quorum)` (`quorum(hide_quorum)` while not admitted) and `W(hiders) > 2·W(defenders)`. Blocked people count as not admitted; their records are held from forwarding and nap; their vouches and reviews don't count. Blocks lift by themselves as the evidence expires or defenders speak up.

### 16.3 Vetoes

`remove`, `ban`, `unban`, `label`, `pin`, `admit` and `mod-add` entries are vetoable. *Vetoers* = people established when ledger bans are ignored (so a moderator can't ban the people who would overturn the bans), not blocked or struck, not a signer of the entry and not its target, with latest verdict `veto`; *supporters* likewise with `ok`. The entry is **overturned** when there are at least two vetoers, `W(vetoers) ≥ quorum(veto_quorum)` (the established voice counted over the same population) and `W(vetoers) > W(supporters)`, and is then skipped in the ledger replay.

### 16.4 Suspending moderators

A moderator (not an owner) whose third overturned entry was created at time *t* is suspended from *t*: their entries created at or after *t* are refused and they lose founder standing. A moderator the community blocks is suspended from then on.

### 16.5 Accountability for reviewers and vouchers

- A juror with ≥ 3 overruled hides stops counting as a hider.
- A person whose counted vouches admitted ≥ 2 people who are now blocked or banned is **struck**: no voice in reviews, objections, defences or vetoes, and their vouches created after the last of those vouches don't count (earlier admissions stand).

### 16.6 Settling

Standing, holds and the ledger depend on each other. Nodes compute them as a fixpoint: start with no penalties and no accepted proposals; replay the ledger skipping overturned entries and suspended moderators and applying accepted proposals; compute admission, voices, levels, hides, blocks, strikes, overruled hides, vetoes, suspensions and proposal outcomes; repeat until they stop changing, at most 6 rounds. Implementations MUST reach the outcomes in `core/vectors/ladder.json`.

## 17. Media

| Type | `mime` | Max | Dimensions in MediaRef |
|---|---|---|---|
| PNG, JPEG, GIF, WebP | `image/png`, `image/jpeg`, `image/gif`, `image/webp` | 2 MiB each | `w`, `h` required (1–16384, ≤ 16,777,216 pixels; 16-bit PNGs ≤ 8,388,608) |
| Ogg Opus | `audio/ogg` | 8 MiB | absent |
| MP4, WebM | `video/mp4`, `video/webm` | 16 MiB | absent |

`MediaRef = { hash: BLAKE3(bytes), mime, size, w?, h?, alt? (≤ 500 chars) }`, distinct hashes, ≤ 4 per post, ≤ 24 MiB in total. Only posts carry media.

**Sending** (sanitize, on the author's device): sniff the type from the bytes (never from names or claims) and strip everything that isn't the picture or the sound: PNG ancillary chunks other than `tRNS gAMA cHRM sRGB sBIT bKGD pHYs acTL fcTL fdAT` (so text, EXIF, ICC and time go); JPEG APPn, COM and thumbnails except an Adobe APP14 marker, keeping only the EXIF orientation as a minimal big-endian APP1; GIF comments, plain text and application extensions other than looping; WebP `EXIF`, `XMP `, `ICCP` and unknown chunks (clearing their VP8X flags); MP4 `udta`, `meta` and `uuid` boxes at any depth (renamed to `free` and zeroed, so offsets stay valid); WebM `Tags`, `Attachments`, `Chapters`, Info `Title`/`DateUTC`/`MuxingApp`/`WritingApp`/`SegmentUUID` and track `Name` (turned into zeroed `Void` elements); Ogg Opus comments (the tags packet becomes `OpusTags`, vendor `hoi`, no comments); and anything after the format's end marker.

**Receiving** (check, on every device): the bytes MUST hash to the reference, match its `size`, sniff as its `mime`, tile their container exactly (every chunk, box, segment, element or page accounted for, checksums valid where the format has them: PNG CRC32, Ogg CRC), carry only allow-listed parts (what sanitizing leaves), have nothing after the end marker, no unknown-size elements, and, for images, the stated dimensions. Polyglots, metadata and scriptable formats (SVG, HTML, PDF, …) are refused. Bytes that hash right but fail are the **author's** fault: their post is held and their key takes a content strike.

**Transfer:** nodes fetch media (`bwant`) only for posts they display and pass on whose author is trusted, if the person allows it (`autoMedia`); serve it (`blob`) only for such posts in communities the asking peer shares, and when asked for bytes still on their way to them (a home node relaying a post that just came from its author's device), remember the ask and serve it once they arrive; ask again for media still missing after a while, and whenever a peer offers a community; delete it when no stored record references it; cap what they keep (512 MiB by default). A hide or removal for `illegal` deletes the bytes on that device and blocklists their hash.

**Fingerprints:** receivers also compute a picture's **PDQ** hash (Meta's open 256-bit perceptual hash: luma, two tent-filter passes, 64×64 samples, the 16×16 low-frequency DCT corner, one bit per coefficient above the median; the first frame of an animation; JPEGs turned by their EXIF orientation) once its bytes pass the checks. A picture the device can't decode within the limits above is refused like bytes that fail the checks (the author's fault). A hide or removal for `illegal` also blocklists the picture's fingerprint, and people MAY load PDQ hash lists. A picture within 31 bits of a blocklisted fingerprint is refused, its exact hash blocklisted and its post held, and sending devices refuse to attach it. Pictures too flat to fingerprint (PDQ quality below 50) have none. Floats round differently across platforms, so fingerprints are compared by distance, never for equality (`core/vectors/pdq.json` fixes one picture's hashes within 4 bits).

**Turned and mirrored copies:** a receiver checks all eight dihedral variants of a picture (as shown, mirrored left to right, upside down, turned half way, and those four transposed), which PDQ derives from the one DCT by swapping and negating coefficients; a picture matches when any variant is within 31 bits of a listed hash. Lists keep the hash as shown.

**Video:** where a host can play a video, it fingerprints `FRAMES` = 8 frames at times `(i + ½)/8` of its length with PDQ, each with its variants. A hide or removal for `illegal` lists those frames' hashes; a video matches when at least half its frames, and at least two, match listed frames. Hosts that can't play a video (headless nodes, browsers without the codec) check its exact hash only. Audio has no fingerprint.

**Decoding elsewhere:** a host MAY decode media outside its core (a browser's core runs as wasm, where a decoder fault would end the node): media waits in memory until the host returns its look, and a host that can't read a picture returns *unreadable*, refused like bytes that fail the checks. The outcome is the same as decoding in the core.

### 17.1 Optional profile pictures

The optional `profile` record is a signed sidecar; existing post and comment encodings are unchanged. Its body is `{image: bytes, seq: uint}`. It requires a community and forbids a parent. The signer is the root or a device delegated `post` or `comment` scope. The entire record is limited to 5 KiB; `image` is either empty (restore the public-key identicon) or a sanitized, decodable square JPEG of at most 128 × 128 pixels and 4096 bytes. Receivers apply the ordinary image container checks and local exact/perceptual blocklists before storing it. Loading a blocklist also removes matching stored profile records and local picture selections.

Profiles follow the community's identity work, retention, bans and media standing. A photo is held until its author may share media; clearing a photo needs no media standing. The stamp has the small-action discount of eight bits. The independent profile rate group allows at most two updates per hour and six per day (the existing owner/moderator rate exemption applies). The greatest `(seq, created_at, record_id)` wins for an author within a community; older records remain until expiry so rate accounting is not reset by replacement. A local selection is stored in the encrypted identity state and copied into the encrypted key backup. It is published in a community after the next successful post or reply, and only when it differs from that community's current profile.

Public sync advertises `profile` in Hello's optional features and sends profile records only to peers advertising that feature, including inventory, direct requests and live gossip. Older peers keep exchanging normal posts without receiving unsupported profile records. Every MLS leaf and key package lists profile capability `0xff42`, which every private group requires (§21). Private profiles travel inside the community's MLS messages and cannot be requested through public sync; a context-free public person view does not borrow a photo from a private community.

Local edits carry a monotonically advancing revision. A newer signed public profile from another device of the same root updates the encrypted local selection, including removals, and that revision is included in key backups. An old device cannot re-publish its stale picture merely by posting; accepted public changes remain the default after their sidecar records expire. A private-only profile is never adopted as the public default.

Each receiver keeps the highest observed `(seq, created_at, record_id)` per author and community as encrypted local metadata until every older profile could have expired. Superseded records remain available for rate accounting but are neither displayed nor forwarded. This prevents a short-lived removal from revealing an older picture that was posted under a longer retention period, including after a restart. Clearing a locally blocked picture advances its revision before publishing the removal.

Pictures contain no remote URL. Default identicons use public key bytes only. Profile bytes are small signed records, so they can arrive during ordinary community sync even when attachment fetching is disabled; that setting hides other people's profile pictures. The settings text distinguishes those records from separately fetched attachments.

### 17.2 Optional identicon colors

The independent `profile-color` record has body `{color: text, seq: uint}` and a maximum encoded size of 512 bytes. `color` is exactly `mint`, `sky`, `lilac`, `peach`, or `sun`; empty text restores the color derived from the public key. It requires a community, forbids a parent, and uses the same delegated posting scopes, identity work, retention, admission and moderation checks as a profile picture. Palette choices are text, contain no external resources, and do not require media standing or automatic media loading. They have an independent rate group of two updates per hour and six per day, with the usual owner/moderator exemption and eight-bit small-action stamp discount.

Color and photo selections have independent revisions and encrypted ordering metadata. Both are copied into encrypted key backups and published after the next successful contribution. Color resets retain ordering metadata until superseded versions have expired, including across restarts. Newer public edits from another device of the same root update its local color choice; private-only selections never become a public default. A selected color can remain saved while a custom photo is displayed.

Public peers advertise the separate optional `profile-color` feature. MLS leaves list capability `0xff43`, which every private group requires (§21). A peer that supports photos but not colors still receives compatible photo records and removals. The original `profile` body and capability are unchanged. Initials badges in the local navigation and You header are UI decoration; they are not part of the signed color or image record.

## 18. Content rules

Records are immutable and signed, so nothing is cleaned on arrival: a record passes or is refused, by its author's device and every receiver.

### 18.1 Text

All text fields MUST NOT contain: control characters (newline and tab allowed in multi-line text); bidirectional embedding, override or isolate controls (U+202A–U+202E, U+2066–U+2069); invisible or format characters (U+034F, U+115F, U+1160, U+17B4–U+17B5, U+180E, U+200B, U+2028–U+2029, U+2060–U+2064, U+206A–U+206F, U+FEFF, U+FFF9–U+FFFB, U+3164, U+FFA0, U+1D159, U+1D173–U+1D17A, U+E0001); private-use characters and non-characters; variation selectors from the supplement (U+E0100–U+E01EF) or two variation selectors in a row; tag characters (U+E0020–U+E007F) outside an emoji flag sequence (U+1F3F4, 1–8 tags, U+E007F); two joiners (ZWJ/ZWNJ) in a row; more than 4 combining marks in a row (U+0300–U+036F, U+1AB0–U+1AFF, U+1DC0–U+1DFF, U+20D0–U+20FF, U+FE20–U+FE2F). One-line fields (titles, names, labels, reasons, alt text) additionally refuse direction marks (U+200E, U+200F, U+061C), soft hyphens and any whitespace other than a plain space.

Markdown bodies (posts ≤ 40000 chars and 2000 lines, comments ≤ 10000 and 500, rules ≤ 20000 and 1000) MUST NOT contain raw HTML (`<` followed by a letter, `/`, `!` or `?`), embedded media (`![`), or more than 8 bare links.

### 18.2 Addresses

The `link` field, every markdown link target (`[text](target)`, reference definitions `[ref]: target`) and every bare `http(s)://` address in text MUST be an absolute `http` or `https` URL (fragment-only targets `#…` excepted) of ≤ 2048 bytes with no whitespace or backslash in the authority, and MUST NOT: carry a username (`https://bank.example@evil.example`); have a non-ASCII host (international names MUST be punycode, `xn--`); point at loopback, private, link-local, carrier-grade NAT, multicast or reserved addresses (IPv4, IPv6, IPv4-mapped) or `localhost`, `.local`, `.internal`, `.lan`, `.home`, `.home.arpa`, `.localdomain`; use a numeric or hex top-level label (`http://2130706433/`, `http://0x7f.1/`); have a bad port. Link text that reads as a site name (`paypal.com`, `https://…`) MUST name the same site as the target (or a subdomain of it).

### 18.3 Names

Self-chosen names: 1–32 characters, no leading or trailing space, no runs of spaces, nothing that looks like a key (`·`, a `did:` prefix), plus the one-line rules above.

## 19. Security considerations (informative)

| Threat | What stops it |
|---|---|
| Forged or altered records | Ed25519 over canonical bytes; strict decoder; tampering changes the id |
| Claiming someone's device, attributing their posts to you | Delegations need the device's signature (consent) and the root's blessing; a device belongs to its first root |
| Rewriting a community's owners | First genesis wins; the community key is forgotten after the genesis |
| Sybil armies | Identity work per key, which slows them down but doesn't stop them (§4.1); vouches only from established members; newcomers' hides, votes and vouches count for nothing; decisions weigh voices that split when someone vouches in many at once, and never exceed the strongest voucher's; quorums need two people |
| Packing a vote | Electors for a change are fixed when it's proposed; people vouched in afterwards don't decide it; a bloc vouched in together shares one voucher's voice |
| Backdated vouches and answers | Records that turn up older than this node's last catch-up count from when they arrived (§15.5) |
| One side changing the rules | Owners only propose; every established member's consent is needed; away members excused only while away; objections stand until the deadline |
| Floods, backdated floods, copy-paste waves | Stamp floor before signatures; rate windows by `created_at`; duplicate fingerprints; strikes, bounces, peer scoring |
| Junk that is expensive to check | Cheapest checks first: hash, then signature, then Argon2id; caps on every sync message |
| Malicious text (spoofing, hidden instructions, phishing links) | §18: direction overrides, invisible and smuggling characters, raw HTML, non-http(s) targets, userinfo, homograph hosts, private-network hosts, deceptive link text |
| Malicious or leaky media | Earned (trusted only); sanitized on sending, strictly checked on receiving; type from bytes; no scriptable formats; displayed from checked bytes only (CSP `media-src blob:`) |
| Re-uploading blocked pictures with small changes | PDQ fingerprints of illegal-hidden or removed pictures and of loaded hash lists; near copies refused; pictures that can't be fingerprinted refused |
| A moderator silencing opinions | Removals nap and can be vetoed; vetoes overturn bans, labels, admits and new moderators; three overturns suspend a moderator; every action is public and signed |
| A mob hiding opinions | Closed hide reasons; 2:1 margin over defenders; defended hides count against the hider; blocks fade with their evidence; hidden things stay peekable |
| Owners raising the bar out of reach | Changes need the community's consent; quorums capped at a third of the established voice; an owner can't require more identity work than their own key carries |
| A leaked key | Revoke the device; burn the identity |
| Filling a public node with junk communities over `carry` | Open communities only; 4 requests per peer per minute, 8 acceptances per minute and 128 per day node-wide; a budget of communities and bytes of which requests may take half; a community whose records never come is let go after an hour, one nobody else touches for two weeks too, and what is let go is deleted; the operator can refuse keys; nothing about the requester is kept; index-listed communities are never pushed out (§10, §20) |
| Making a public node dial an address through `carry` | A request carries the genesis only; routes are learned solely from peers that prove they hold records (§10) |
| A registry giving one name two keys, to different people | Each answer must be in its signed log, and the log only grows: pins catch a change on one device, the log shows both entries to anyone, a mirror refuses the registry (§10) |
| A registry replaying a node's announcement to another registry | The announcement signs the registry's origin; another registry refuses it (`wrong-registry`) (§10) |
| A registry filling with junk announcements | First arrivals stay when full; one route per node per round; the operator's allowlist mode and per-key drops (§10) |
| An operator who must stop serving something | Local tools only (`hoi-node admin`, `registry/admin.mjs`, the Worker's configuration): delete it here and refuse it for good; no endpoint, nothing signed, nobody else's copy touched |
| Inflating a community's Warmth | A peer that dialed in counts only the records it delivered and was given, never its bare `have`; a dialed peer's listing counts because its endpoint was already trusted; one holder per endpoint; holders in one network or under one declared operator are one place, and an operator claim only merges places; a copy reached only over unguarded links reads colder; a peer's hint (`catalog.w`) is read only from peers that list `keeps`, capped at 3, taken as a maximum, never summed, and never changes the level; a node's own copy never warms its own reading (§20). Nodes spread over many networks by one party (a Sybil across cloud providers) still read as many places: Warmth is a reading, not a defence |

Residual risks, stated plainly: a patient attacker who pays for identity work and waits out `established_age` still gets voices, but each of their accounts passes on at most one voice per quarter and nobody they bring in rises above them, so a puppet tree grows at most doubling per epoch instead of without bound; an owner who mints identities can fill their own community with members (the ledger and vouches show it, their puppets share the owner's voice, and members can leave), but can no longer change its rules or limits without the members who were already there; colluding members can still backdate vouches against nodes that joined after the claimed time, until `established_age` has passed for real; any single elector with half a voice can hold a change up until the deadline (by design: slow but safe); clients that ignore the protocol can keep what others delete (honest nodes refuse it back and expiry bounds what they pass on, but nothing stops a screenshot); perceptual hashes catch re-encoded, resized, lightly edited, turned and mirrored copies of blocked images, not heavy crops; videos only where a host can play them, and only copies whose frames line up (not trimmed or re-cut ones); nothing catches altered audio.

## 20. Warmth: how many copies a node can see

Communities live on the devices that hold them, so the question a person has is not "is a socket open" but "how many copies of this are out there, and how independent are they". **Warmth** answers it the way a torrent client shows availability, from what one node observed first hand on its own links, counting copies once per independent **place** and reading colder when copies are reached only over unprotected links. People see one word and one bar; the parts are named only in the details, in plain words ("kept in 3 independent places", "every link checked"). It is informative: no node trusts another's Warmth, and a warm reading is not a delivery receipt.

**Observation.** For a public community a node is in, it reads:

- `records`: its own live, forwardable records; `own_durable`: whether this device keeps them across restarts and can be reached (a desktop or headless node, not a browser or phone).
- Per connected peer that holds the community, one holder per authenticated endpoint: `genesis`, whether it listed, delivered or was given the genesis; `listed`, the other live ids in its last `have` (or the count `n` its range session gave, the genesis aside); `given`, the records this node handed it since (served on `want` or gossiped); `delivered`, the valid records it sent since, new or already here; `still_missing`, the ids it listed that are not here yet, 0 once its catch-up stalled. What counts as held: `genesis + given + delivered`, plus `listed − still_missing` only for a peer this node dialed by a route it already trusted (a peer that dialed in is believed about what it showed, not about what it claims). `coverage = min(1, held / records)`, or 1 when this node holds nothing yet and the peer holds anything. `durable`: the peer listed `keeps` in its hello. `operator`: a key for who runs the peer, BLAKE3 over its notice's operator and contact (case and spacing ignored), when its hello carried a notice that hashes to its `nh` and names someone. `network`: a key for the network its address belongs to (the connection's address, else the dialed route's): the IPv4 /24, the IPv6 /48, or this machine for a loopback link. `guarded`: the link is encrypted (QUIC, WebRTC, TLS, or loopback) and its other end was checked (this node dialed a route it chose, pinning the certificate or key, or the transport authenticated the peer's endpoint).
- `missing`: the most any one peer listed that is not here; `complete = records / (records + missing)`, 0 when nothing is known.
- `media`: of the media hashes referenced by live posts here, the share present (1 when none is referenced).
- `live`: a connected peer lists it; `fresh = live` or a catch-up finished within 7 days.
- `hinted`: the largest `catalog.w` (0–3) a connected node that lists `keeps` attached for it (§14).

**Places.** Durable holders that share an `operator` key or a `network` key, directly or through another holder, are one place: one party or one outage takes them all, so ten nodes in one rack, or one operator's nodes in two data centres, are one copy's worth. A holder with neither key is a place of its own. An operator key is the peer's own word, so it can only merge places, never add one; network keys come from addresses this node saw.

**Reading.** The level and score count the durable copies *others* hold, once per place: a node's own copy never warms its own reading, so a lone home node and a lone browser both read cold, and a public node reads a community it alone carries as cold too.

```
copies          = 1 + Σ coverage                         (one decimal)
other_durable   = Σ_{durable} coverage
durable_copies  = (own_durable ? 1 : 0) + other_durable
kept            = Σ_{places} max coverage in the place   (one decimal; ≤ other_durable)
places          = places holding any of it
spread          = kept / other_durable                   (0 when other_durable = 0; two decimals)
guarded         = Σ_{durable, guarded} coverage / other_durable   (0 when none; two decimals)
network_copies  = max(other_durable, hinted)
level           = kept < 1 → cold · < 2 → cool · < 3 or complete < 0.95 → warm · else hot,
                  one level colder when complete < 0.5, not fresh, or guarded < 0.5 while
                  other_durable > 0 (never below cold)
score (0–100)   = round(100 × (0.6 × min(1, kept / 4) + 0.1 × spread + 0.1 × guarded
                               + 0.12 × complete + 0.08 × fresh))
hint (w)        = min(3, ⌊kept⌋)
```

So a browser alone reads cold (score 12 when its window is complete and never synced), a browser with its home node cool (55), two members' nodes in different places warm (70), three complete copies in three places that caught up recently hot (85), and three nodes in one network cool (48). The level follows first-hand copies only: a hint raises `network_copies`, never the level.

**Your connections.** The same keys read the device's own links (every peer or home node that said hello; devices it serves are its own): `places` as above, `effective = 1 / Σ share²` over links per place (two decimals), `largest` the share of links in the busiest place, `guarded` the share of guarded links. Cold with no link; cool when every link goes through one place or under half are guarded; hot from 3 effective places with no place over half the links and nine in ten guarded; warm in between. `score = round(100 × (0.6 × min(1, effective / 4) + 0.25 × guarded + 0.15 × (1 − largest)))`, kept inside its level's band (cool ≤ 49, warm 50–79, hot ≥ 80) so the bar never says more than the word. It tells a person whether one party could see or cut most of what this device sends, without the word for it. Vectors: `core/vectors/warmth.json` (`cases`, `links`).

**Where it travels.** A node attaches `w` to its `catalog` entries, read at most once a minute, so one hop of what a public node saw reaches the browsers it serves; nothing else is sent, and receivers never sum hints. Registries add their own count out of band: `/v2/nodes?community=` lists the public nodes that announced carrying a community, which a client shows beside its Warmth.

**The directory's warmth** is the client's reading of discovery itself: whether a registry answered, how many public nodes announced themselves there, how many connected nodes share a catalog and how many open communities they list, how many take `carry` requests. The core reports the connected-node counts; the registry's are the client's.

## 21. Scattered delivery: pieces, sizing and envelopes (draft)

Built in phase S1 (`hoi_crypto::scatter`, `hoi_crypto::envelope`, `hoi_types::scatter`); nothing
travels on the network yet. The messages that carry envelopes (`put { to, env }` and the rest),
pointer records and `Limits.scatter` come with S2 and S3 (SCATTER.md §7). Byte layouts are
big-endian; every string below is ASCII and part of the format.

**Size classes.** An object holds up to 4, 16, 64 or 256 KiB, or 1, 2 or 4 MiB of content (class
0–6); longer media are cut into 4 MiB objects. `plain = u32(len) ‖ content ‖ zeros` to 4 + the
class's size. `sealed = nonce[24] ‖ XChaCha20-Poly1305(K_obj, nonce, plain, aad = "hoi scatter
object v1" ‖ object id[32] ‖ class[1])`, so `|sealed| = class + 44`. The nonce is synthetic:
the first 24 bytes of `BLAKE3_keyed(BLAKE3-derive-key("hoi scatter nonce v1", K_obj),
object id ‖ class[1] ‖ plain)`, so the same content under the same key and id seals to the same
bytes. Opening checks the length, the tag, that the nonce is the one `plain` gives, `len ≤ class`
and zero padding. `sealed_hash = BLAKE3-derive-key("hoi scatter sealed v1")`
over `sealed` (the hasher in derive mode, as for fingerprints); every generation must rebuild to it.

**Transform (AONT-RS).** Per generation a fresh 32-byte `K_a`: `C = ChaCha20(K_a, nonce = 0,
counter 0)(sealed ‖ 16 zero bytes)`, `package = C ‖ (K_a ⊕ BLAKE3-derive-key("hoi scatter aont v1",
C))`; undoing it recomputes the mask from `C`, decrypts, and refuses a canary that is not zero.
`|package| = |sealed| + 48`.

**Pieces.** The package, zero-padded to `k · L` with `L = ⌈|package| / k⌉`, is cut into k data
pieces of L bytes; pieces must not exceed 1 MiB (so k ≥ 5 for 4 MiB objects). Piece r is row r of
the systematic Reed–Solomon matrix over GF(2⁸) with polynomial 0x11D: the Vandermonde matrix
with rows `[r⁰ … r^(k−1)]` (0⁰ = 1), r = 0…255, times the inverse of its top k × k block. Rows
0…k−1 are the identity; any k distinct rows rebuild the data pieces; row r depends on r and k
only. A rebuilt codeword whose padding after the package is not zero is refused.
`piece_hash = BLAKE3-derive-key("hoi scatter piece v1")` over `object id ‖ g[1] ‖ i[1] ‖ u32(L) ‖
piece`, first 16 bytes. `loc_g,i = BLAKE3-keyed(derive_key("hoi scatter loc v1", S_g), object id ‖
g[1] ‖ i[1])`.

**Generations.** `3 ≤ k ≤ n − 2`, `n ≤ 160`, `n ≤ m ≤ n + ⌈n/4⌉`, `m ≤ places`; indexes 0…n−1 at
first, fresh indexes n, n+1, … up to m by repair, never one twice, each in a place that never held
a sibling of that generation. `g` is 0–255.

**Sizing.** `size(ℓ, g, N)` for ℓ ∈ {10, 15, 20} and N places: with `A = ⌊ℓN/100⌋`, `ε_g = 1 /
(10⁶ · 2^(g+1))`, for n = 5 … min(N, 160) (160 above 1,000 places), `m = min(n + ⌈n/4⌉, N)`, and k
from `max(3, ⌈n/3⌉ + 1)` while `P(Bin(n, 4/5) ≥ k) ≥ 999/1000`: the first k with `P(X_m ≥ k) ≤ ε_g`,
if also `P(X_n ≥ n − k + 1) ≤ ε_g`, else the next n; none when nothing fits (under 12 places).
`X_d` is the party's share of d draws: hypergeometric over (N, A) for N ≤ 1,000, binomial with
p = ℓ/100 above. Exact rational arithmetic only. `reading(g, N, k, n, m)`, the level a manifest
shows, is the highest ℓ whose two bounds hold. Reference: [`scatter-params.mjs`](scatter-params.mjs).

**Manifest** (canonical DAG-CBOR, keys `c` class, `d` holders (endpoint ids, distinct), `e`
pieces' expiry, `g`, `h` piece hashes (16 bytes, index order), `k`, `l` level, `m`, `n`, `o`
object id, `p` places, `s` sealed hash, `v` 1, `x` `S_g`, `y` `K_obj`). Receivers refuse unknown
keys, versions other than 1, and any floor above broken, `h` and `d` of different lengths, fewer
than n or more than m entries, or pieces over 1 MiB for the class and k. It holds the object's
keys: it travels only sealed (§6 of SCATTER.md, pointer records, S2).

**Drops.** `D = BLAKE3-keyed(derive_key("hoi scatter drop v1", A), loc)` from the creator's drop
secret A; a put carries `BLAKE3-derive-key("hoi scatter drop hash v1")` of D. The community drop
key (Ed25519) signs `"hoi/scatter-drop/1\n" ‖ loc`; a put carries its id, the first 16 bytes of
`BLAKE3-derive-key("hoi scatter drop key v1")` over the public key.

**Holder keys.** A node's X25519 `xk` is published with an expiry, signed by its endpoint key over
`"hoi/scatter-xk/1\n" ‖ endpoint[32] ‖ xk[32] ‖ u64(expires_at)`; senders seal only to unexpired,
signed keys. `key_id` is the first 8 bytes of `BLAKE3-derive-key("hoi scatter xk id v1")` over xk.
Keys come from HPKE DeriveKeyPair over 32 random bytes.

**Envelopes.** HPKE (RFC 9180) base mode, DHKEM(X25519, HKDF-SHA256), HKDF-SHA256,
ChaCha20-Poly1305; `envelope = enc[32] ‖ ciphertext`. `info = "hoi scatter v1 "` + the kind
(`put`, `get`, `has`, `drop` to a holder; `stored`, `got`, `had`, `dropped` back). Requests use
`aad = "hoi scatter to v1" ‖ endpoint ‖ key_id`, answers `aad = "hoi scatter reply v1" ‖ loc` and
are sealed to the one-time X25519 key the request carried. Plaintexts (tag = 1…8 in that order):

```
put      tag ‖ loc[32] ‖ reply[32] ‖ u32(expiry day) ‖ drop_hash[32] ‖ drop_key_id[16] ‖ u64(stamp)
         ‖ u32(L) ‖ piece[L]                                          envelope = 177 + L
get      tag ‖ loc ‖ reply ‖ u32(want_len)                             zero-padded: envelope 256
has      tag ‖ loc ‖ reply                                             envelope 256
drop     tag ‖ loc ‖ reply ‖ 0 ‖ D[32]  |  tag ‖ loc ‖ reply ‖ 1 ‖ key[32] ‖ sig[64]   envelope 256
stored   tag ‖ status (0 ok, 1 full, 2 bad stamp, 3 refused, 4 taken)   envelope 64
had      tag ‖ 0 ‖ u32(0) | tag ‖ 1 ‖ u32(length)                    envelope 64
dropped  tag ‖ status (0 dropped, 1 unknown, 2 refused)                 envelope 64
got      tag ‖ found ‖ u32(len) ‖ piece ‖ zeros to 6 + want_len         envelope 54 + want_len
```

Parsers are strict: the tag of the kind, exact lengths, zero padding, `0 < L, want_len ≤ 1 MiB`,
`len = 0` exactly when not found.

**Built in S2 (first part).** *Holder keys in hellos:* a node that holds pieces puts `sk = { k:
xk[32], e: expires_at, s: sig[64] }` in every hello, the signature over the xk statement above by
its endpoint key; receivers keep it only when it verifies against the endpoint the transport
authenticated and has not expired, and older peers ignore the field. Holders derive each 30-day
period's key from their endpoint key (a local choice, not part of the format), publish it with an
expiry 7 days past the period's end, and open envelopes to the previous key until then.
*Messages:* `piece { o: op, r: rid, e: envelope, to? }`, `op` one of the eight envelope kinds,
`rid` a u32 the answer repeats so the asker can pair it with its one-time key, `to` the holder's
endpoint when the peer only forwards (S3; holders ignore such requests until then), `e` at most
1 MiB + 1 KiB. Decoders read and senders send at most 16 `fx` names.
*Stamps:* `BLAKE3-derive-key("hoi scatter stamp v1")` over `holder[32] ‖ loc[32] ‖ u32(len) ‖
u32(expiry day) ‖ u64(nonce)` must have `8 + ⌈log2(⌈len / 1024⌉)⌉` leading zero bits.
*Holders* keep a piece under its locator with its expiry day, drop hash and drop key id; refuse
expiry days in the past or more than 400 days ahead (`refused`) and stamps that do not verify
(`bad stamp`); answer `full` when their budget is spent after shedding expired and 30-day-idle
pieces; never overwrite a locator; answer `got` padded to the wanted length either way; drop on
the creator's token or the community drop key's signature; ignore envelopes not sealed to them;
rate-limit each link (600 requests and 16 MiB of puts a minute).
*Placement* caps (⌈n/8⌉ per IPv4 /16 or IPv6 /32, ⌈n/4⌉ per introducer) count over a
generation's whole life: a repair's holder is chosen with the earlier holders counted.
*`Limits.scatter`* = `{ resist: 10 | 15 | 20, open: bool }`, written in a genesis or a limits
proposal only when it differs from `{ resist: 10, open: false }`, and refused when written with
the default value (one encoding per value); a proposal patches `resist` and `open` like any other
field (`scatter_resist`, `scatter_open`).

**Built in S2 (second part): mailboxes for private groups (SCATTER.md §5.13).**
*The list* is the MLS group context extension 0xff44: canonical CBOR `{ v: 1, m: [ { e:
endpoint[32], r: [route] } ] }`, at most 8 entries of at most 4 routes of at most 128 characters
(anything else reads as no list). Every private group requires capability 0xff44 of every leaf
from its creation (below). A commit changes the list; the Welcome carries it. *Slots:* `slot_key = BLAKE3-derive-key("hoi mailbox slot v1", relay
secret)` of the epoch (the exporter secret `"hoi relay v2"` the relay rooms use); slot `seq` of
member `leaf` is `BLAKE3-keyed(slot_key, u32(leaf) ‖ u32(seq))`, both counted from 0 in every
epoch. A slot holds one relay object's sealed bytes (`CipherObject.body`, 4 KiB to 1 MiB, padded),
stored with an ordinary `put` (stamp, expiry at most 30 days, the writer's own drop hash, no drop
key) and read with `has` then `get` of the length `had` gave. A writer gives each queued object the
next slot of its current epoch at queue time, so a membership commit takes a slot of the epoch it
ends after everything queued before it; it counts an object sent once 2 mailboxes (or all, when
fewer) answered `stored ok`. A holder answers `taken` (status 4) when the locator already holds
different bytes and `ok` when it holds the same; a writer answered `taken` takes its next slot in
the same epoch. *Answers:* `had` now carries the stored length (`tag ‖ 1 ‖ u32(length)`, length
1…1 MiB) or `tag ‖ 0 ‖ u32(0)`. *The bell:* each epoch also has bell slots, `seq` at
`BLAKE3-keyed(slot_key, u32(0xffffffff) ‖ u32(seq))` (no leaf has that index). Once a message is
stored, its writer rings the bell of the message's epoch on every mailbox: a `put` of 32 random
bytes, kept two days, no drop hash, at the next bell slot it has not seen taken there; answered
`taken`, it rings the next one. *Pace* (local, not format): a member probes one mailbox's bell (the
one at its own leaf index, modulo the linked mailboxes), at once after a find, then backing off
from 5 to 120 seconds. A ring sends it over every other member that has gone quiet, each followed
while it has more and for three misses after; every ten minutes, and in every new epoch, it looks
at every member's next slot once and at one past the bell's next slot. A member's probe looks one
slot further every fourth miss.
*Notices:* key `s: true` in a notice's canonical map says the node keeps pieces for others; it is
written only when true, set by the node from how it runs, and shown as the fixed section
"Encrypted pieces" (`hoi_types::notice::PIECES`) after what the node processes. Older readers ignore
the key; the hash covers it. *Holders count* only after first-hand sightings (SCATTER.md §5.7; local
policy, not format).

**Built in S2 (third part): vouched holders, holders over WebRTC, desktops as holders.**
*Vouches:* a `nodes` message may carry `h: [ { e: endpoint[32], u: percent, d: days } ]`, at most
32 entries: holders the sender saw itself, at its own hourly samples, long and steadily enough to
take pieces, with the share of its samples the holder was linked in (0–100, senders round down to
5) and the days since it first saw it (receivers cap at 30). Senders vouch only for first-hand
sightings, never for what they were told. Older peers ignore `h`; a reader that finds `h` too long,
or an entry malformed or repeated, ignores that entry or the whole field, never the node list.
Receivers keep vouches only from the person's own devices and home node and from links they
dialed, as they keep node lists, and drop a node's vouch for itself. *Counting* (local policy): a
linked holder takes pieces when this device saw it long enough itself, or when its own devices
vouch for it, or when two links in different places (Warmth's places, neither the holder's own)
vouch with at least the device's own bar; it then counts against the introducer it was first
heard from for a generation's ⌈n/4⌉ cap. *Holder keys over WebRTC:* `sk` may carry `r = { e:
endpoint[32], c: cert[32], s: sig[64] }`, `c` the SHA-256 of the certificate the holder serves
WebRTC with and `s` its endpoint key's signature over `"hoi/rtc-endpoint/1\n" ‖ endpoint ‖ cert`.
A browser that pinned that very certificate when it dialed (the fingerprint in its `rtc:` route)
takes `e` as the link's endpoint and checks `sk` against it; with any other certificate, or over a
link whose endpoint the transport authenticated, `r` changes nothing. Without it a browser, which
sees no endpoint on WebRTC, could check no holder key. A malformed `r` is no binding; the key is
still read. *Desktops:* the setting `holdPiecesGb` (0.5–100 GB, 0 off) makes a desktop that is a
public node a holder (local, not format); its notice gains `s` like any holder's.

**Built in S3: forwarding, pointers, repair.**
*Forwarding:* a hello may carry `fw: true` (written only when true; older peers ignore it): the
sender passes `piece` requests that carry `to` on to that holder. A node sends `to` only to peers
that said `fw`. A forwarder passes a request on, under a request id of its own on its link to the
holder and with `to` removed, only to a peer whose authenticated endpoint is `to` and that gave a
valid holder key, or, after dialing it, to a holder it vouches for itself; the holder's answer
goes back unchanged under the asker's id. Otherwise, and when the holder is not linked within a
while, it answers `piece { o: "gone", r: rid, e: [0] }`: the eighth-plus-one op, never sealed,
sent only to a peer that asked with `to`. Answers are not inspected. *Vouches with keys:* an entry
of `nodes.h` may carry `k = { k: xk[32], e: expires_at, s: sig[64] }`, the holder's key exactly as
its own `sk` gave it (never `r`), and `r`, at most two of the holder's routes (128 characters
each). Receivers check `k` against the entry's endpoint like a hello's `sk` and seal envelopes to
it; a malformed `k` or `r` is left out and the entry kept. Routes are a claim: receivers use them
only to place a holder they have no link to, and only when every voucher's first route names the
same network. *Pointers:* `pointer = { v: 1, t: made_at, m: manifest }`, canonical DAG-CBOR,
strict. A later manifest of an object (`Manifest::update_to`) either keeps the generation and
every field, its earlier hashes and holders as a prefix, adding at least one index (a repair), or
is the next generation with the same object id, sealed hash, key and class, a locator secret of
its own and an expiry no earlier (a regeneration). Readers also require a size that meets its own
level and some level (`reading`), `t` at most ten minutes ahead, and at most one regeneration per
object accepted per day by `t`, the earliest `t` (then the smaller encoded manifest) winning when
two of the same generation compete; they keep the generation before as a fallback. *Private
groups:* leaf capability 0xff45 says a member's app reads the MLS payload `{ type: "pointer", v:
1, community, pointers: [base64(pointer)] }` (at most 64); the object each names holds a JSON
array of `records` and `media` payloads (at most eleven), applied as if they had come whole.
*Repair* (local policy, not format):
see SCATTER.md §5.10; probes are ordinary `has` through forwarders.

**Built in S3, rest.** *One object per post:* in a private group, each
post, comment and edit record is the only record of its object's `records` payload; media objects
carry up to eleven `media` payloads of one item. *Drop key:* the mailbox list may carry `d`, a
32-byte Ed25519 public key (the group's drop key); its id (`drop_key_id`) goes in every put of
the group's pieces, and drops signed by it over `"hoi/scatter-drop/1\n" ‖ loc` remove them. The
device that writes the list sets the key; every committer's device holds its secret since S4
(below). *Home-node
rebuild:* a hello may carry `rb: true` (only from a home node to a device it serves), and then
`rebuild { r: u32, m: manifest }` goes from the device and `rebuilt { r, c?: content }` back, `m`
at most 512 + 200·64 bytes, `c` at most 4 MiB. A node answers `rebuild` only from its own
devices; receivers ignore `rb` on any other link. *Mailboxes:* requests to a mailbox may carry
`to` like any other, through a forwarder. *Reading slots in order:* a member that finds a slot past a
member's next one (its look one ahead) reads it but keeps waiting for the next one; only after
four more misses does it move past it (a slot every mailbox lost).

**S4, first step: membership that follows the records.** A private community whose
genesis or limits admit by `invite` or `approve` follows its records. *Commits:* a receiver
refuses (and keeps for another try, as a message that came early) a commit that adds a leaf
whose person its records do not admit (§15.1) or that bans them, or removes a leaf whose person
is not banned, burned or revoked there. *Order:* before such a commit, the committer sends in
the same epoch a `records` payload with what backs it: the person's identity and delegation,
and the vouches and `admit` entries for them (adding), or the `ban` entries and revocations
(removing). An owner admitting by hand writes an `admit` entry first. *Requests:* MLS payload
`{ type: "request", v: 1, community, request }` carries a `hoi:private-request:` code to the
owner's device, which keeps at most 32 for a week; `{ type: "welcome", v: 1, community, device,
invitation }` carries the `hoi:private:` invitation it made back, accepted only from the owner.
Every leaf of a private group reads both.

**S4, several committers.** *Line:* in a group that follows its records, the devices
that may commit in an epoch are its leaves whose person is the owner or a moderator (not banned,
burned or revoked) and whose leaf blessing covers `moderate:<community>`, sorted by (role since,
root, leaf blessing `created_at`, device): the owner's role since the genesis `created_at`, a
moderator's since the `created_at` of the latest `mod-add` naming them that took effect.
A receiver refuses a
commit from a device outside the line. *Turns:* the device of rank r makes a change once it has
been due there for r · 3600 s, and only after reading the epoch for 60 s with the mailboxes
answering (a gap of 300 s without answers starts that again). *Requests* (`request` payloads) are
kept by every committer; `welcome` payloads are accepted from any device in the line. *Bootstrap
from a moderator:* the `bootstrap` payload carries the genesis, the moderator's identity and
delegation, and the `mod-add` with its signer's identity and delegation; the newcomer replays them
and follows the invitation only if they make the sender a moderator. *Contests:* of two commits of
one epoch, the one of lower rank counts. A member that follows a commit of rank r > 0 keeps,
for 86 400 s, that epoch's MLS state and the next unread slot of each committer of lower rank;
it reads those slots, takes `records` payloads found there, and on a commit of lower rank that
its records back it restores the epoch, follows that commit, drops what it queued since, and
sends everything again in the new epoch. Invitations that came in later epochs of the branch
left are dropped. *Slots:* a slot found ahead of a member's next one is forgotten when the epoch
changes.

**S4, a drop key every committer holds.** *List:* beside `d`, the mailbox list may carry
`h`, at most 64 device ids of 32 bytes: the committers' devices the key's secret was given to.
*Seal keys:* in a group that follows its records and has mailboxes, each device in line sends `{ type: "sealKey", v: 1, community, key }` once in each epoch,
`key` the base64 public part of `XKey::from_seed(BLAKE3-derive-key("hoi drop key seal v1", device
secret))`; receivers keep it only from a device in line. *Giving:* `{ type: "dropKey", v: 1,
community, to, key, sealed }`, `key` the drop key's public part, `sealed` 80 bytes: HPKE base mode
(DHKEM(X25519, HKDF-SHA256), HKDF-SHA256, ChaCha20-Poly1305) to `to`'s seal key, `info` `"hoi drop
key v1"`, AAD `"hoi drop key v1" ‖ community ‖ to ‖ key`, over the key's 32-byte secret. `to` takes
it only from a device in line, while it is in line itself, and only if the secret's public part is
`key`; other members ignore it. A giver first sends a `records` payload with the `mod-add` (and its
signer's identity and delegation) of itself and each device it gives to, unless owner, then the
`dropKey`s, then the commit of the list with them added to `h`, all in the epoch the commit ends.
*When* (each by the committers' turns, above): a device in line holding the listed key gives it to devices
in line not in `h` whose seal key it heard; a device in line makes a new key when the list names
none, or names in `h` a device out of the line, and gives it to every device in line whose seal key
it heard; a device in line that lacks the listed key waits one turn past its own and the last
holder's in line, whichever is later, then makes one. A device out of a group's line forgets the
group's drop keys. A replaced key is kept 2 678 400 s after a device first sees another listed.
*Drops:* a removed post's generations are dropped with every key a device holds, by the device of
rank r once r · 3600 s have passed since it let the post's object go; one of rank r > 0 first asks
one random index (`has`) and drops only if the holder does not answer that it is gone. *Requests:*
the device that passed a `request` on sends it again in each new epoch, until a commit it follows
adds the requesting device, for at most 604 800 s.

**The clean break.** Every private group's group context carries, from the epoch that
creates it, a RequiredCapabilities extension listing `0xff42`, `0xff43`, `0xff44`, `0xff45` and
`0xff46`; MLS refuses to add a key package that lacks one, and a committer refuses a join request
whose key package lacks one before it gets that far. No group has a leaf without them, so nothing
is sent or read differently for older leaves: profiles, colors, mailboxes, pointers and
moderators' commits are always on. *Slots found ahead:* a member that looks one slot past a
writer's next one and finds a commit or proposal of the current epoch holds it, unfollowed, until
the writer's slots before it are read (then follows it at once), or until those count as lost
(four misses, as for any slot found ahead). Application messages found ahead are taken at once,
as before. Saved private state has no relay version and no relay v1 fields; scatter state has no
`drop_keys`.

**S4, invitations picked up by the newcomer.** *Requests:* the JSON a
`hoi:private-request:` code carries gains `pickup: { key, holders }`, required: `key` the base64
public part of an X25519 key made for this request, `holders` at most 4 distinct `{ endpoint,
routes }` (at most 4 routes of at most 128 characters), possibly none. *Slots:* `loc_i =
BLAKE3-keyed(derive_key("hoi invitation pickup v1", key), u32(i))`, i = 0…7. *Sealing:* the
invitation's text, as `u32(len) ‖ text ‖ zeros` to a multiple of 4096 bytes, HPKE base mode
(DHKEM(X25519, HKDF-SHA256), HKDF-SHA256, ChaCha20-Poly1305) to `key`, `info` `"hoi invitation
v1"`, AAD `"hoi invitation v1" ‖ key`; `enc[32] ‖ ciphertext`, at most 1 MiB. *Storing:* the
device that makes an invitation for a request with holders stores the sealed bytes with an
ordinary `put` (stamp, expiry a week ahead, its own drop hash, no drop key) at `loc_0` of each
holder it reaches; answered `taken`, at the next slot. *Picking up:* while a device's request is
out (a week), it sends `has` for its next slot to each holder its request named, then `get` of the
length; an object that opens under its key into an invitation it can follow is followed (as if
pasted), anything else moves it to the next slot. The invitation also goes into the group as a
`welcome` payload, as before.

**S5, open communities in pieces.** *Limits:* a new open community's genesis carries
`scatter: { open: true }` (with `resist` when not 10); the default value read when absent stays
`{ resist: 10, open: false }`. *Pointer records:* kind `pointer` (rate group of its own, 16 KiB at
most), body `{ record: bytes32, of: "post" | "comment" | "edit", media?: bytes32, sealed: bytes }`,
a community, no parent, posting scope (`post` or `comment`); `media` only with `of: "post"`;
`sealed` at most 12 KiB: `hoi_crypto::sealed` (XChaCha20-Poly1305, context `"hoi open pointer v1"`)
under `BLAKE3-derive-key("hoi read v1", community key)` of the canonical `pointer` (§21, S3).
*Objects:* the content an object holds is the JSON array private groups use: one `records`
payload with the one record named, or up to eleven `media` payloads of the item named, each
with `posts`, the posts it was sent for. *Readers* (devices with an identity that joined)
take a pointer for a new object and fetch it; the record rebuilt must have the id named and be
signed by the pointer author's person, and is never offered whole by them. A pointer for an object
already kept counts only if it names the same record and media and its author's person is the
first pointer's, or is established in the community (§15), a moderator or an owner; it is then
checked as an update (§21, S3). *A device's own records* (post, comment, edit) in a community whose
limits of the record's day say `scatter.open` are not offered whole unless they could not be kept
in pieces. *Stand-ins:* any node that holds a pointer record checks votes, replies, edits,
deletions and reviews of the record it names (`of` post or comment) as if that record were there,
with the pointer author's person as its author. *Drop key:* ledger action `drop-key`, not under
the threshold, signed by an owner, no target, `drop_key: { key: bytes32, to: [ { d: device, s:
sealed[80] } ] }` (at most 64, devices distinct), `s` sealed as a private group's `dropKey` (AAD
`"hoi drop key v1" ‖ community ‖ device ‖ key`) to the device's seal key; kept until superseded;
the latest by effective time is listed. Delegations may carry `seal: bytes32`, the X25519 public
key `XKey::from_seed(BLAKE3-derive-key("hoi drop key seal v1", device secret))`, signed by the
device with the rest of the record. Nodes take identities and delegations of a held community's
owners and moderators from peers even before those post. Pieces of an open community are stored
with the listed key's id; removed posts are dropped by the line of owners' then moderators'
devices (by role since, root, blessing time, device), the one of rank r after r · 3600 s.

**S6, scale.** *Manifests* may carry `z`: one 32-byte X25519 key per holder, in index
order, the key the holder had when it took its piece (all or none; a repair keeps the earlier ones
and appends). *Holders* open `get`, `has` and `drop` envelopes sealed to any of their period keys
of the last ⌈400 days / 30 days⌉ periods, and `put` only to the current one and, for 7 days, the
one before; at each new period a holder sends every link a new hello (§14's replacing hello).
*Forwarding:* a forwarder asked for a holder it neither links to nor saw itself passes the request,
with `to` kept, to one of its own links that said `fw` and whose node list vouched for the holder
with a key, unless the request came from a link that said `fw`; answers return by request ids as
for any forwarded request. *Media objects* (private groups and open communities) hold
`"hoi slice v1\n" ‖ u32(len) ‖ header ‖ bytes`, the header canonical JSON `{ v: 1, community,
hash, total, offset, posts }` (`posts` at most 64), the bytes a slice of the item at `offset`; an
item's slices fill objects by `scatter::fit` (the smallest class holding the rest when it is at
most twice the rest, else the largest class it fills, header included). Receivers assemble an item
from parts by offset, whole 256 KiB chunks and slices alike. *Private groups' resends* are local
policy: after a commit, what was first sent in the epoch it ended goes again; after a commit that
adds a member, also the device's own records, identity and blessing, and from the committer all of
them.

**Rosters.** *Record:* `roster`, community-scoped, signed by a device with `vote`
scope, no parent, body `{ holders: [bytes32] }`: 1 to 32 holder endpoints, sorted ascending,
each once; `expires_at ≤ created_at + 3 days`; at most 2 KiB; rate group 10, 8 a day. Receivers
take one only from a person who is established in the community and neither banned nor struck
there; a device's newer roster replaces its older ones. *Use (local policy):* placing an object
of the community, a holder named by the unexpired rosters of at least two such people whose voices
sum to the community's hiding quorum counts as one more introducer place than its vouchers give,
and is counted against one introducer shared by all holders that count only thanks to rosters
(placement caps an introducer at ⌈n/4⌉ of a generation). A device publishes one only when its
person opted in.

**S7, address privacy.** *Messages:* `piece` ops `wrap` and `inject`, requests like the
others (rate-limited per link, bytes counted as puts), with `to` through a forwarder and without
at the node they are for; `wrap` gets the inner request's answer, `inject` none. *Wrap:* HPKE base
mode (the envelopes' suite) to the exit's holder key, `info` `"hoi scatter v1 wrap"`, AAD
`"hoi scatter to v1" ‖ exit endpoint ‖ key_id`, over `to[32] ‖ tag ‖ envelope`, `tag` the inner
request's kind (1 put, 2 get, 3 has, 4 drop) and `envelope` the request sealed to the holder `to`
as in §21. An exit (a node that forwards and holds pieces) opens it with its current key or, for 7
days, the one before, and forwards the inner request to `to` as it forwards any request; a node
that cannot open one ignores it. *Inject:* the same suite to the injector's holder key, `info`
`"hoi scatter v1 inject"`, the same AAD, over `u16(count) ‖ (u32(len) ‖ record)* ‖ zeros` to a
multiple of 1024 bytes, 1 to 16 records of at most 192 KiB in all. The injector takes the records
in after a random 0–20 seconds as if a peer had sent them, and passes them on. *Senders* (local
policy): a put goes wrapped through an entry link, in another place than the exit, whose node list
vouches for the exit, when one is linked; only the exit counts against the k−1 indexes of a
generation a place may carry. A device's own new record of an open community goes injected through
an entry link to a linked public node that carries the community and has a holder key, and is not
offered to the device's links until one of them sends it back, or 120 seconds pass.
A record kept in pieces is never injected (its pointer records are); a person's own
identity and blessings go first hand only to their own devices (paired clients, the home node)
and travel to others as what injected records carry, except a moderator's blessing for an open
community's line, injected through that community; a home node injects its paired devices' new
records as its own.

### 21.1 Durable author deletion (RFC-0001)

The contract is bounded, eventual deletion. A holder MUST reject replay after it has accepted
an authorized drop, through the piece's original inclusive UTC expiry day. A holder that has
not learned a deletion may still serve its existing piece. Catch-up with an uninformed peer
does not prove freshness; there is no lease, global deletion ledger or promise of rejection
throughout a partition. Expired signed content remains invalid after markers are collected.

**Capability.** A file-backed holder with durable marker storage advertises `drop-journal` in
Hello `fx`. Its holder key `sk` additionally carries optional `dj: bytes64`, the endpoint key's
Ed25519 signature over:

```
"hoi/scatter-xk/1\n" ‖ endpoint[32] ‖ xk[32] ‖ u64(expires_at) ‖ 0x00 ‖ "drop-journal"
```

The expiry is big-endian, as in the ordinary holder-key statement. The ordinary `sk.s` signature
is still required. Vouches preserve `dj` in `nodes.h[].k`, so a requester can verify the holder's
own capability through forwarders. A missing, malformed or invalid `dj` establishes no durable
capability; an ordinary holder-key signature, a forwarder's feature list and a release version
cannot replace it. The proof must be unexpired when the request is sent. In-memory holders do
not advertise this durability. Existing drop authentication and envelope layouts are unchanged.

**Holder transaction.** After verifying the existing creator token or moderator signature,
replace the payload with a marker atomically, and reply `dropped: 0` only after the write
succeeds. The marker retains exactly these fields:

| Field | Reason |
| --- | --- |
| `loc` | Refuse another put at the same locator |
| `expiry_day` | Bound retention to the original piece's inclusive UTC day |
| `drop_hash` | Authenticate repeated creator drops without retaining the token |
| `drop_key_id` | Authenticate repeated moderator drops without retaining their signatures |

No author/community identifier, content, raw token or signature is added to holder storage.
Retaining `drop_key_id` preserves the existing correlation between pieces stored with that key.
An authenticated repeat answers `dropped: 0` without extending retention; invalid authority
answers `refused`. Neither a piece nor an unexpired marker means `unknown`, not durable erasure.
Marked locators answer absence to `get`/`has`, and every put at them answers `refused`. Remove a
marker only when `today > expiry_day`; use the stored day, not a record's second-level expiry.
Reserve marker capacity when admitting pieces and never evict unexpired markers to admit more.

**Progress and retries.** The requester commits the deletion record, local suppression and its
known holder targets/capabilities before dispatch. It preserves unfinished work across restart
and retries transient failures with bounded backoff under the existing routing rules. A full
journal rejects new work rather than evicting unfinished jobs. Missing authority or unknown
generations remain coverage gaps. Acknowledgement means a matching one-time-key reply to a
request sent to a holder with a verified durable capability; an older holder's `dropped: 0`
means legacy removal observed. `Unknown`, refused, unreachable and expired are distinct outcomes;
timeout and expiry never prove erasure. Duplicate requests/replies cannot inflate counts.
Local removal, record submission and holder acknowledgements are separate. Home-node submission
does not transfer the originating device's responsibility or prove per-holder completion.

New clients do not initiate legacy home-delegated scatter placements: that request format does
not preserve origin correlation and deletion authority across an originating-device restart.
They use local scatter when eligible, otherwise the existing whole-record delivery fallback.
Legacy receivers and reply verification remain supported. While an already started legacy
delegated placement is unresolved, a local author-delete request fails retryably before mutation
rather than claim recoverable deletion; unknown legacy inventory remains an explicit gap.

Deletion suppresses queued and in-flight repair. Already dispatched placements are retained as
cleanup targets; their completion must not publish a replacement for deleted content. Other
authors' live content and shared media referenced by it remain available. A deletion received
before its target is bounded pending work, applied only after normal ownership validation.

**Local storage compatibility.** The versioned client journal uses the existing persistence and
vault boundary. It retains the signed deletion, target/author/community and expiry for
dissemination and ownership; object IDs and coverage gaps for late repair; holder/locator/expiry
and per-locator tokens for authorized retry; outcomes, attempt/retry times and publication state
for progress. Tokens are discarded after acknowledgement or expiry. It retains no deleted body
or object decryption key. Existing stores migrate without inventing prior acknowledgements.

The SQLite holder schema uses `user_version = 1`, `held_pieces` and `dropped_pieces`, with
`synchronous = FULL`. An incompatible sentinel at the former `pieces` table makes older
implementations fail initialization rather than ignore markers. Unknown newer versions are
refused. Turning holding off removes payloads while preserving unexpired markers. A supported
holder snapshot restore uses `SqlitePieces::restore`: it discards all restored payloads before
serving and preserves available markers. Direct disk rollback outside that path is outside the
guarantee. Recipients, backups and malicious operators can retain copies; these acknowledgements
are neither portable erasure receipts nor proof of physical erasure.

Full member snapshots use the Rust `Node::open_restored` API. Public content is quarantined
until ordinary synchronization applies the deletions available from that peer; this is not a
global freshness proof. Private snapshots discard cached posts, comments, edits, pointers,
associated repair manifests/capabilities, partial media and queued encrypted content before
serving. Media with surviving references is retained, as are signed deletions, their jobs,
skeletons and identity/membership records. Any still-unrecorded deletion authority must enter
its job before cache disposal; insufficient capacity or unresolved shared references refuse
restoration without discarding that authority. Cache disposal creates no author tombstones, so
fresh authenticated delivery can recover still-valid content. Private output stays quarantined
until an accepted fresh authenticated join; a bootstrap alone is not historical catch-up.
An already active restored MLS membership has no general reset/rejoin operation, so it remains
quarantined. Usable recovery in that case requires a fresh device and an authorized private join.
These are Rust snapshot-restoration APIs, not a shipped backup import UI or command. Key-file
import and recovery-line restoration are separate identity recovery paths and contain no MLS
snapshot state. Ordinary restarts continue to use `Node::open` and retain their caches.

### 21.2 Deleting known posts and comments (RFC-0002)

Bulk deletion is a client procedure over §21.1. It adds no record kind, wire field, envelope or
cryptography; each target receives its own ordinary `delete` record, validated and propagated as
any other.

**Inventory.** A preview selects live posts and comments held on this device whose resolved
author root (after delegation, revocation and rotation validation) is the person's. Display
names and possession of an old signing key select nothing; edits follow their parent and are not
targets; other authors' replies, posts and media are never targets. Pointer-only entries stay
unresolved until the record itself is held. Alongside the targets by community, a preview reports
the known piece generations with drop authority on this device and these gaps:
`private-state-unavailable` (no usable group state), `not-synchronized` (never caught up with a
peer, or a quarantined snapshot), `record-absent` (pointer only), `capability-missing` (count of
generations without authority), `generations-unknown` (kept in pieces with no known generation)
and `too-many` (targets beyond one batch). `inventoryComplete` is always false: a local scan does
not prove complete publication history.

**Confirmation.** The snapshot ID is BLAKE3 over `"hoi bulk author deletion snapshot v1"`, the
person's id and the sorted target ids. Confirming recomputes the inventory and refuses a changed
list; the confirmed list never expands. Confirming a saved ID again resumes it without signing a
second deletion for any target.

**Execution.** Before each signature the client rechecks that the target is held, unexpired and
the person's, then signs through the ordinary author-delete path with the current time. Rate
windows (§9.1) and owner/moderator exemptions apply per record; a rate-limited target waits for
its window and is never backdated. A browser that signs through its computer keeps a few
requests outstanding and asks again only after a request lapses. Cancellation stops targets that
have no accepted signed deletion; accepted deletions and their §21.1 jobs continue. Target states
(selected, queued, signed, failed, expired, cancelled) are separate from §21.1 progress; signed
is not propagation.

**Local storage.** Batches are a versioned blob in the existing encrypted client storage, saved
in the same transaction as the deletions they sign and read only from storage that commit
accepted. Client storage format 5 adds it; older software refuses that storage instead of
abandoning a batch, and format-4 stores open with no batches. A batch whose targets do not match
its snapshot ID, or of an unknown version, refuses to open. The inventory links an author to
communities and records: it is never logged or sent, and holders continue to receive locators
and drop authority only. A local wipe discards batches without reporting their work as finished;
identity recovery re-establishes signing authority but no batch, inventory or capability.

### 21.3 Clearing local data after deletion (RFC-0003)

Clearing local data is a client procedure. It adds no record kind, wire field, envelope,
cryptography or stored state, and it sends nothing.

**Local clearing.** An ordinary wipe, or recovery from a locked vault, is always allowed. It
abandons the device's §21.1 jobs and §21.2 batches: their remaining work stops and is never
reported as finished. Locked recovery cannot read the vault, so it cannot list that work either.
Neither sends a deletion, burns the identity or reaches other devices, exported backups or
copies other members keep, and neither guarantees physical storage erasure. An ordinary wipe
keeps the vault's encryption; locked recovery removes the vault.

**Readiness.** `cleanupReadiness` reports, for the whole installation and only from state that
storage accepted, whether clearing would strand deletion work. It is blocked while a deletion of
the person's is not yet signed (a waiting batch target, or a request a browser's computer has
not signed), while a signed deletion of theirs still needs local state under §21.1 (until its
record's validity and its pieces' storage days end; acknowledgements alone never end it), and
while the device holds drop authority for an unfinished piece of another author's deleted post.
It also counts known posts and comments that no deletion covers, and gives two revisions:
BLAKE3 over `"hoi cleanup inventory revision v1"`, the current preview snapshot and every saved
batch target's state, and over `"hoi cleanup deletion revision v1"`, every job and pending
remote deletion with whether it blocks.

**Guarded clearing.** `clearAfterDeletion(inventory, deletions)` commits, recomputes readiness
and both revisions under the same serialization as deletion work, and wipes only if the result
is ready and both revisions match. Otherwise it fails with `cleanup-blocked` or
`cleanup-changed` and changes nothing. A core without these calls cannot authorize it.
