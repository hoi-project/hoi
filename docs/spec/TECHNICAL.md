# hoi technical document

Version 0.3 · October 2026 · Read with `PROTOCOL.md` (the wire format, 0.3).

## 1. Constraints that shape everything

1. **Infrastructure does not grant community authority.** Members can run their own public carriers. There is an optional public-name registry and explicitly configured public infrastructure routes; carriers can announce their own routes to it and mirror what it lists; the registry and the public node are roles of `hoi-node` that anyone runs, a public node can be handed an open community over the link, and every device gets a Warmth reading of how many copies of a community it can see. The registry stores no posts, profiles or member addresses, only names, keys, versions and carriers' self-announced routes (kept thirty minutes). The static web deployment includes neither and names neither: the published app uses no registry and no relay, and finds communities through the nodes people run, which pass each other on; only a copy built with one names the registry and the opaque private-group relays it uses. A relay operator stores encrypted objects but holds no MLS membership keys; see [relay/README.md](../../relay/README.md).
2. **Every device validates.** Nothing is displayed, stored or forwarded until it passes the local pipeline (stamp, signature, identity work, time, rate, moderation state and standing, content rules).
3. **Nothing is deleted by hand; everything expires.** Records carry an expiry set by the community. Moderators and the community's established members label and hide. Time removes.
4. **Web, desktop and mobile** from one core, with honest degradation (browsers are relay-only clients; phones sync in bursts).
5. **Modern primitives only** (2025+): iroh 1.x, QUIC, BLAKE3, Ed25519, range-based set reconciliation, MLS for private groups.
6. **Standing is earned, not granted.** What a key may do in a community (links, votes that count, vouches, reviews, media) follows from the community's own signed records, computed the same way on every device. Decisions weigh voices that thin out when someone vouches in many people at once.
7. **Nobody changes the rules alone.** Owners propose rules and limits; everyone established decides.
8. **Collect nothing.** No accounts, no telemetry; what a device keeps for its own convenience (reading notes for vouching, saved posts, petnames) never leaves it and fades too.

## 2. Architecture overview

```
┌──────────────────────────────────────────────────────────────────┐
│  UI (Svelte or SolidJS, one codebase)                            │
│  desktop: Tauri 2    mobile: Capacitor or native shell    web: PWA│
└───────────────▲──────────────────────────────────────────────────┘
                │ typed API (UniFFI on native, wasm-bindgen on web)
┌───────────────┴──────────────────────────────────────────────────┐
│  hoi-core (Rust)                                             │
│  ├─ identity      keys, device delegations, recovery shares      │
│  ├─ records       canonical encoding, ids, signatures, PoW       │
│  ├─ validate      the pipeline every record passes               │
│  ├─ store         SQLite (rusqlite), search index; expiry sweeper │
│  ├─ sync          per-community gossip + range-based set recon   │
│  ├─ moderation    ledger state machine, labeler subscriptions    │
│  ├─ trust         ring, weights, local ranking                   │
│  ├─ spam          strikes, peer scoring, signed reports, autoban │
│  ├─ net           iroh Endpoint, ALPNs, discovery, optional relay│
│  └─ api           the single surface the UIs call               │
└───────────────▲──────────────────────────────────────────────────┘
                │
┌───────────────┴──────────────────────────────────────────────────┐
│  iroh 1.x: QUIC, hole punching, relays (user-run), iroh-gossip,  │
│  iroh-blobs (later), pkarr/DNS discovery, LAN beacons            │
└──────────────────────────────────────────────────────────────────┘
```

One Rust crate does all the thinking. UIs are thin and never touch keys, the network or storage directly.

## 3. Stack

| Layer | Choice | Why | Alternative considered |
|---|---|---|---|
| Networking | **iroh 1.x** (stable June 2026) | QUIC with hole punching, stateless E2E relays anyone can run, gossip, blobs, pkarr/DNS discovery, official Swift/Kotlin/Node bindings, relay-only WASM for browsers | libp2p: heavier, better browser↔browser WebRTC; revisit for browser peers in M4 |
| Signatures | **Ed25519** | fast, tiny, iroh-native | – |
| Hashing / PoW | **BLAKE3** | fast, parallel, verified streaming for blobs later | – |
| Encoding | **DAG-CBOR** (canonical) | deterministic bytes = stable ids and no signature malleability | fixed binary layout |
| Sync | **range-based set reconciliation** (after Negentropy and Meyer's range-based reconciliation) | fetch only what's missing per community/time range | CRDT logs (overkill for immutable records) |
| Live fan-out | **iroh-gossip**, one topic per community | epidemic broadcast, cheap | – |
| Storage | **SQLite via rusqlite**, search from an index in memory | single file, portable; no words of a post in plaintext on disk | redb |
| Private groups | **MLS (RFC 9420)**, via `openmls` | encrypted group delivery; explicit membership and epoch updates; see [relay/README.md](../../relay/README.md) for the relay's scope and limits | BeeKEM (pre-alpha) |
| Local keystore | **Argon2id + XChaCha20-Poly1305** under a local passphrase; passkey PRF unlock is planned | protects keys at rest | – |
| Mutable community metadata | **Loro** CRDT (sidebar, rules text, wiki) | the one place concurrent edits matter | Automerge |
| Desktop shell | **Tauri 2** | small, Rust-native, system webview | Electron |
| Mobile | **UniFFI** bindings + native shell (SwiftUI/Compose) or Capacitor | iroh ships Swift/Kotlin bindings | – |
| Web | **wasm-bindgen** build of core in a Web Worker, PWA | relay-only transport; OPFS/IndexedDB store | – |
| UI | **Svelte 5** (or SolidJS) | one codebase across shells | – |
| Motion | **Anime.js 4.x** | already used in early prototypes; small | – |

Versions to pin at M0: iroh 1.x latest minor, animejs 4.1.4 (verified CDN) or newer once verified, openmls current.

## 4. Platforms and honest degradation

| Platform | Transport | Role it can play | Limits |
|---|---|---|---|
| Desktop (Win/macOS/Linux) | full iroh: direct QUIC + relay fallback | full peer, pinner, **relay** (opt-out when reachable), home node | – |
| Headless (`hoi-node`) | full iroh | pinner, relay, community home node; Docker/systemd | – |
| Public node (`hoi-node --public`) | full iroh + WebRTC for browsers | stores and serves open communities it is given (an index it mirrors, carry requests from connected peers), announces its routes; no person's key, no standing, signs nothing | carries open communities only; a bounded budget of communities and bytes; lets go of, and deletes, what nobody else touched for two weeks or whose records never came; refuses keys its operator lists |
| Registry node (`hoi-node --registry`) | HTTP API + full iroh | arbitrates public names in one namespace, lists compatible communities and the public nodes that announced themselves; may mirror another registry | names unique within its namespace only; same API and database shape as `registry/` |
| Android / iOS | full iroh in foreground; sync bursts via background tasks | full peer while open | no background sockets; pairs with a home node; optional user-run push relay (wake-only, no content) |
| Browser (PWA) | WebRTC data channels to native nodes (pinned DTLS certificate, no signaling server); loopback WebSocket to the desktop on the same computer | light client | can't accept connections or hole-punch; syncs through your own home node or members' nodes from invites; the UI says so ("via lena-laptop", "via a member's computer") |

**Home node pattern.** A desktop or the cheapest VPS running the same core keeps subscriptions warm and serves the user's phone and browser. The app pairs them with a QR code a phone's camera opens, over WebRTC on the local network (or a public address), and links the phone to your key with four check digits. This is how phones get a good experience without the project running anything.

## 5. Crate layout (`core/`)

```
core/
├── Cargo.toml                 workspace
├── crates/
│   ├── hoi-types          records, ids, canonical encoding, errors
│   ├── hoi-crypto         keys, signatures, PoW, keystore, recovery shares
│   ├── hoi-validate       the pipeline (pure functions, heavily tested)
│   ├── hoi-store          SQLite schema, queries, search index, expiry sweeper
│   ├── hoi-sync           reconciliation engine + gossip glue
│   ├── hoi-moderation     ledger state machine, labelers, limits
│   ├── hoi-trust          ring, weights, ranking (hot/fresh/loudest/friends)
│   ├── hoi-spam           strikes, peer scoring, reports, autoban
│   ├── hoi-net            iroh endpoint, ALPNs, discovery, relay toggle
│   ├── hoi-core           composes the above; the public API
│   ├── hoi-ffi            UniFFI bindings (Swift, Kotlin)
│   ├── hoi-wasm           wasm-bindgen bindings
│   └── hoi-node           headless binary (pinner / relay / home node)
└── vectors/                   protocol conformance vectors (PROTOCOL.md §13)
```

The apps live at the repository root: `apps/site` (hoi.sh), `apps/web` (PWA, also the desktop UI), `apps/desktop` (Tauri 2), `apps/mobile` (Swift package and Android library over `hoi-ffi`). Implemented today: `hoi-types`, `hoi-crypto`, `hoi-validate`, `hoi-store` (in-memory index, SQLite, OPFS or IndexedDB persistence), `hoi-sync` (range reconciliation and want messages, PROTOCOL.md §14), `hoi-moderation`, `hoi-trust`, `hoi-spam`, `hoi-net` (home-node WebSocket link, iroh QUIC between native nodes, WebRTC for browsers and phones), `hoi-core`, `hoi-wasm`, `hoi-ffi`, `hoi-node`.

## 6. The validation pipeline

Every inbound record, from gossip, reconciliation, an import or a local compose, passes these stages in order. A failure at any stage stops it; the cheapest check that can see a problem comes first. *Peer* strikes fall on whoever forwarded broken bytes; *key* strikes fall on the author of valid records that break the rules.

| # | Stage | Check | On failure |
|---|---|---|---|
| 1 | Decode | size ≤ the kind's cap; strict canonical DAG-CBOR; `id = BLAKE3(bytes)`; not already stored or tombstoned | drop; peer strike for bad bytes |
| 2 | Stamp floor | one BLAKE3 hash: the least any record of this kind could need | drop silently |
| 3 | Signature | Ed25519 strict over the id; device keys need a consented, blessed, unexpired, unrevoked delegation; burned identities refused | drop; peer strike |
| 4 | Stamp | full difficulty: community `pow_bits` minus kind and key-age discounts | drop silently |
| 5 | Identity | the author's root has an `identity` record (Argon2id, cached; native hosts verify a message's identity work before taking the node lock, `hoi_core::IdentityWork`) with at least `ladder.identity_bits` | wait for the identity, or drop; forged work drops the peer and the rest of its batch |
| 6 | Time | ≤ 10 min in the future; live gossip ≤ 10 min old; not expired; within retention | drop |
| 7 | Moderation | not banned at `created_at`; not tombstoned; not locally bounced | drop |
| 8 | Rate | rate windows counted by `created_at` (PROTOCOL.md §9.1); owners and mods exempt | drop; key strike if live |
| 9 | Content | text and link rules (§18), media references (§17), copy-paste fingerprints (§9.2) | drop; key strike |
| 10 | Semantics | kind-specific: same author for edits and deletes, owner thresholds, moderator scope and suspension, capabilities of the author's standing (vouch, review, veto) | drop |
| 11 | Accept | store, index (vouches, reviews and ledger entries that arrive older than this device's last catch-up count from arrival, PROTOCOL.md §15.5), settle the ladder, then hold or forward | – |

After acceptance, records whose author's standing doesn't cover their content (links, media, anything before a vouch in `invite` communities) are **held**: stored, shown to their author only, never forwarded, re-checked when standing changes. Media bytes are checked separately when they arrive (§17 of PROTOCOL.md), and pictures are fingerprinted (PDQ) and compared with the device's blocklist.

Stages 1–4, 6, 8 and 9 are pure functions in `hoi-validate` with vectors and tests; standing, hides, blocks and vetoes are pure functions in `hoi-moderation` (`ladder.rs`); `hoi-core` composes them.

## 7. Spam: automatic, local, evidenced

- **Cost to exist.** Every identity did memory-hard work once (Argon2id); every record carries a stamp. Neither stops a patient attacker, both make floods expensive. Identity work is a throttle, not Sybil resistance: at 8 MiB and one pass, the default 10 bits cost about a second of one core, so a thousand keys cost a quarter of an hour. What keeps many keys from counting for much is standing (§8): fresh keys weigh nothing.
- **Rate windows** per author root, kind and community, counted by the records' own `created_at`, stricter for newcomers (PROTOCOL.md §9.1). Every node computes the same answer whatever path a record took.
- **Copy-paste floods.** Text fingerprints refuse the same text twice from one author within 7 days, and from newcomers when two others posted it within 24 h.
- **Local strikes.** Each key accumulates a decaying score per node (rate 1.0, content 3.0, duplicate 0.25; half-life 1 h). At 10 the key is bounced locally, nothing new it signs is stored or forwarded, and a signed **report** is published (record ids as evidence, never content). A bounce lifts on its own after a day, twice as long for each repeat within a month, at most 30 days: devices on different releases may read the content rules differently, and one device's verdict must not shut an honest person out for good. Records made before the rules they break took effect are refused without a strike (PROTOCOL.md §9.3).
- **Peer scoring.** Peers that forward invalid bytes, signatures or identity work, or flood messages, lose score and are disconnected.
- **Community review does the rest.** Newcomers can post text only; links wait for a vouch, media for trusted standing. Spam that gets through is hidden by established members' `hide` reviews and its author blocked (§8). Fresh keys weigh nothing, and keys vouched in as a batch share their voucher's voice, so a Sybil army cannot hide or block anyone.
- **Later (M5):** Rate-Limiting Nullifiers (RLN) make "too many posts per epoch" cryptographically slashable.

## 8. Moderation, review and expiry

- The **ledger** is the community's append-only log of signed actions: `rules`, `limits` (owner threshold to propose, the community to decide), `mod-add`, `mod-remove` (owner threshold), `remove`, `ban`, `unban`, `label`, `pin`, `unpin`, `admit` (any moderator).
- **Changes by consent** (PROTOCOL.md §6.1): a rules or limits proposal applies once everyone established when it was proposed agrees; while the only ones holding it up are away (nothing of theirs left), it is in effect; one retention window later silence is consent and only objections weighing half a voice stop it. Limits proposals patch only the fields they change.
- **The ladder** (PROTOCOL.md §15): newcomer → member (vouched in by established members, or admitted) → established (after `established_age`) → trusted (recent contributions with a good response). Each level unlocks a capability: links, counted votes, vouches and reviews, media. Reading a newcomer's posts without hiding them vouches for them (client policy, on by default).
- **Voice** (PROTOCOL.md §15.4): admission counts people, decisions weigh them. Each established member gives away at most one voice per quarter, split among everyone they vouched for then; nobody rises above their strongest voucher.
- **Reviews** (PROTOCOL.md §16): established members say `ok` or `hide` (closed reasons) about posts and comments, and `ok` or `veto` about ledger entries. A record naps for everyone when hiders' voice reaches the quorum and outweighs defenders two to one; a person is blocked on the same terms. Hides the community defends count against the hider.
- **Nobody moderates alone.** Removals nap for 72 h before they take effect (except `illegal` and `privacy`), and established members can veto removals, bans, unbans, labels, pins, admissions and new moderators. Three overturned entries suspend a moderator. Quorums are capped at a third of the established voice so owners can't make review impossible, and every decision needs two people. All of it is signed and public.
- `remove`, once in effect, produces a **tombstone**: compliant nodes delete the record body, keep only its id and the removal entry, and never re-ingest it. The UI shows "Poof." with the reason and a link to the ledger.
- **Labelers** are any key publishing `label` records. Communities list labelers; every user can add or override on their device. A hidden post is "napping" and can be peeked.
- **Expiry** is the real deletion mechanism. The store runs a sweeper every few minutes; expired records are deleted and never re-requested in reconciliation (ranges exclude anything past `now - max_retention`). Saved posts are kept locally if the user opts in, but never re-shared after expiry.
- **Private communities** use a separate MLS delivery path with persisted guards against plaintext synchronization. Removal and an epoch update protect subsequent messages; they cannot erase plaintext or old keys a member has retained. Existing public history is not retroactively private. See [relay/README.md](../../relay/README.md) for deployment and limits.

## 9. Identity

- **Root key** (Ed25519), ideally kept offline or in the passphrase-protected keystore. Expressed as `did:key`.
- **Identity work.** A root becomes an identity by publishing an `identity` record with Argon2id work over the key (10 bits by default, about a second natively). The work stays with the key; communities require a minimum (`ladder.identity_bits`). Weak keys are refused. It throttles new keys; it does not stop anyone holding many (§7, PROTOCOL.md §4.1).
- **Making keys.** Keys come from the system's randomness. The first run may add a *stir*, a 32-byte digest of the person's gestures, mixed in as `BLAKE3-derive-key("hoi 2026 key seed v1", csprng32 ‖ stir)`, never used instead. Hosts that can show the identity work mine it themselves (draft → mine → publish).
- **Device keys** consent to a `delegation` with their own signature; the root blesses it with `root_sig` over a domain-separated statement. Scope (post, vote, moderate-for-community-X) and expiry (≤ 400 days) are part of the statement. A device belongs to its first root. Revocation is a signed record; revoked devices lose the ability to sign as you from that moment.
- **Burning.** A root that revokes itself is gone: everything it signed is dropped everywhere and nothing it signs is accepted again.
- **No transfer, but recovery.** Standing never moves to another person. A new key is a newcomer in every community; that is what makes standing impossible to steal or sell. The person's own recovery line can replace their root and keep it (PROTOCOL.md §4.3): the line's master secret derives every root and recovery key, each identity commits to the next recovery key, and a `rotate` signed by it hands over to a new root that names the person.
- **Backup and recovery.** A key lives only on the person's devices, so the person keeps the copy. The **recovery key** (`hoi_crypto::recovery`) is the master secret as one checksummed line of z-base-32, `hoi-key-1-…`, made for a password manager (which encrypts it) or a drawer: whoever reads it can always replace your root, so it is you. The device keeps it until the person says it is saved, then forgets it; a stolen device then gives a thief the root of the day, never the power to replace it. Restoring from it on a fresh device always rotates: a new root takes over, the old root and its devices stop counting, standing stays. Friends can keep it in shares: any 3 of 3 to 7 (`hoi_crypto::shamir`, constant time), each sealed to the friend's devices. The **key file** (`.hoikey`, `hoi_crypto::keystore`) is the key with the name, picture, joined communities, friends, petnames and the proofs for public names of communities the person made (version 2; version 1 files still import), sealed with Argon2id (64 MiB, 3 passes) and XChaCha20-Poly1305 under a passphrase of at least 15 characters (the local vault's standard), for a disk or a cloud. The file brings this root back on a fresh device with a new device key; the lost device's key is then revoked by hand. Only the file brings the name proofs back: the community key that signed them is forgotten at creation, so a device restored from the recovery key can't claim or publish those names again. Neither carries private-group (MLS) state.
- **Handles:** local petnames by default; self-chosen display names are checked for spoofing (no key-like text, invisible or direction characters) and always shown next to the key; optional DNS-verified handle: a name that is a domain shows ✓ when `_hoi.<domain>` TXT holds `did=<did:key>` or `https://<domain>/.well-known/hoi` holds the did:key.
- **Transport key** is separate from the posting key by default, so peers cannot link network identity to authorship.

## 10. Discovery and configured services

Nothing in the site, the app or the binaries names a node, a registry or a relay to start from; the first address always comes from a person.

1. **Invite links / QR**: `hoi://join?c=<community key>&r=<relay addr>,<relay addr>&p=<peer id>…` carry everything a new node needs.
2. **Node links**: `hoi://node?w=…&p=…` (or `#/node?…`) is a public node's address and nothing more; every public node prints its own, and the desktop shows it with a QR code.
3. **Node lists** (PROTOCOL.md §14.2): every public node (`hoi-node` by default, the desktop app once its person chooses) passes on the public nodes it reached itself; a device tries what it is told, keeps and passes on only nodes that answered it, and keeps addresses on private networks among peers on them. A browser keeps one link to a public node; a native public node keeps links to three.
4. **LAN beacons** on the local network: a multicast beacon per node, sealed per community.
5. **pkarr records on the BitTorrent Mainline DHT**: not used; it would name outside bootstrap routers, which node lists do without.
6. **Auto-relay**: a reachable desktop (public IP, UPnP/NAT-PMP, or user-opened port) runs an embedded `iroh-relay` by default (opt-out), announces itself, keeps no logs, and can be gated to its communities with bearer tokens (not built yet; `hoi-node` already asks the router to forward its ports).

7. **Carry requests** (PROTOCOL.md §10): a public node that advertises `carry` takes an open community's signed genesis from any connected peer (the genesis only, never routes) and keeps it available; a founder's browser asks the public nodes it reaches unasked for every open community its person founded. Bounded by policy, operator refusals, rates per peer and per node, a budget of communities and bytes, an hour for the records to arrive and two weeks of idleness; what is let go is deleted.
8. **Warmth** (PROTOCOL.md §20): every device reads how many copies of a community it can see besides its own, from its own links (a dialed peer's list, a dialed-in peer's deliveries, durability on the peer's own `keeps` word), plus one hop of hints in catalog entries; the apps show it beside the sync state, and the directory gets its own reading.

Public peer routes come from accepted invites, paired devices, node links, the node lists of
public nodes a device reached, and, in a copy built with one, a registry. The published web
build uses no registry or relay; a copy can be built with one (`VITE_HOI_REGISTRY_URL`,
`VITE_HOI_RELAY_URLS`). Private relay origins are fixed by build configuration,
not supplied by an invitation. There is no default labeler.

## 11. Security

- Canonical encoding everywhere (no malleability); ids are content hashes; signatures over the canonical bytes.
- QUIC/TLS 1.3 with key-pinned peers (iroh) protects transport hops. An ordinary public-community carrier can read the records it carries. The separate MLS opaque relay stores private-group ciphertext without membership keys; see [relay/README.md](../../relay/README.md).
- Replay: ids dedupe; `created_at` windows; per-author `seq` for votes, vouches and reviews; first genesis wins; superseded vouches and reviews are remembered and refused.
- Hostile input: strict decoding, size caps per kind and per sync message, the stamp floor before signature work, Argon2id only after both; text and link rules refuse spoofing and smuggling characters, raw HTML, non-http(s) targets, userinfo, homograph and private-network hosts and deceptive link text (PROTOCOL.md §18).
- Eclipse/Sybil: no mandatory public DHT for content, peer diversity across bootstrap sources, signature verification means a malicious peer can withhold but not forge.
- Metadata: transport keys are separate from posting keys. A durable opaque relay still sees per-epoch room keys, padded sizes, timing, expiry and network metadata; encryption does not establish a no-log policy or provide anonymity. Deployment logging and credential separation must be managed explicitly.
- Keystore: Argon2id + XChaCha20-Poly1305 under a local passphrase (passkey PRF unlock is planned); the key file is encrypted, the recovery key is the key itself and belongs in a password manager.
- Strangers learn little: no name and no stable id in hellos to members' nodes or carriers, and of the communities a device is in only those whose routes led to that node, the rest as hashes salted per connection that only a node holding the same key can match; no catalog from a node that isn't serving the public; a home node says nothing to a device until it shows its credential, and its loopback listener refuses web pages from other origins; a pairing link pairs one device, which gets a credential of its own; routes a registry suggests are kept and passed on only once they delivered records, WebRTC routes only for the certificate that was dialed, and a freshly paired home node's routes only once it delivered records; identities, devices and revocations from a peer only for roots this node has heard from, or alongside their community records; at most 16 live devices per root; owner cosignatures only from devices whose blessing was valid when the entry was made; a stranger's early `revoke` can't shadow the real root's.
- Supply chain: signed releases (a minisign-signed SHA-256 manifest and build-provenance attestations; the release workflow refuses to run without its signing secrets and names unsigned installers in the release notes when an operator allows them on purpose), `cargo-deny` and `cargo audit` on both Rust graphs, `cargo-vet` for `core/` (imported audits plus exemptions at today's versions in `core/supply-chain/`, so a new or updated crate fails until someone reviews it), `pnpm audit`, workflow actions pinned by commit and base images by digest, published hashes on the website. The wasm package, hoi-node and the static site build byte for byte the same on any machine with the pinned toolchains (`scripts/reproduce.mjs`); the desktop installers don't. The PWA has no third-party scripts and a strict CSP sent as a header (no inline scripts or styles, `frame-ancestors 'none'`); SRI covers the files the HTML references, and the service worker checks every other file it caches (dynamic imports, workers, the wasm) against the SHA-256 the build recorded, refusing bytes that differ. In this model the client binary is the trust boundary.
- Media: earned (trusted standing only), sanitized on the sending device (metadata stripped, type sniffed from bytes), strictly checked on every receiving device, never scriptable formats, displayed only from checked bytes via `blob:` URLs under CSP `media-src blob:`.
- CSAM: a hide or removal for `illegal` deletes the bytes on that device and blocklists their hash and their PDQ fingerprint, so re-encoded and resized copies are refused too; nodes never fetch media for held or untrusted posts; PDQ hash lists can be loaded; pictures a device can't fingerprint are refused. The static site never receives media; whoever runs a public node stores public media and can stop serving any of it (`deploy/server/README.md`).
- Governance attacks: rules and limits change only by consent of the people established when proposed (so packing the electorate afterwards does nothing); removals nap and can be vetoed; vetoes count established members as if bans didn't exist; three overturns suspend a moderator; quorums capped at a third of the established voice; voices thin out for people vouched in together; vouches and answers that arrive late count from arrival; owners can't require more identity work than their own key carries (PROTOCOL.md §6.1, §15.4, §15.5, §16, §19).

## 12. Storage schema (sketch)

```sql
records(id BLOB PK, community BLOB, kind TEXT, author BLOB, parent BLOB,
        created_at INT, expires_at INT, bytes BLOB, tombstoned INT DEFAULT 0);
CREATE INDEX ON records(community, created_at);           -- reconciliation ranges
votes(target BLOB, author BLOB, value INT, created_at INT, PRIMARY KEY(target, author));
ledger(community BLOB, seq INT, id BLOB, kind TEXT, bytes BLOB, PRIMARY KEY(community, seq));
delegations(device BLOB PK, root BLOB, scope TEXT, expires_at INT, revoked INT);
trust(peer BLOB PK, degree INT, weight REAL, since INT);
strikes(key BLOB, community BLOB, score REAL, updated INT, PRIMARY KEY(key, community));
labels(labeler BLOB, target BLOB, label TEXT, created_at INT, PRIMARY KEY(labeler, target, label));
CREATE VIRTUAL TABLE records_fts USING fts5(title, body, content=records);
```

## 13. Ranking (local only)

`hot = wilson(up, down, trusted_only) / (age_hours + 2)^1.5`, where votes are weighted by the voter's trust weight (friend 1.0, friend-of-friend 0.5, member-by-age 0.25, newcomer 0.05). "Fresh" is by `created_at`; "Loudest" by weighted score; "Friends" filters to degree ≤ 1. No node ever ships a tally to another node in M0–M4; optional community-run aggregators (like Bluesky feed generators) are M5.

## 14. Build, test, release

- `cargo test --workspace` with property tests on `hoi-validate` and `hoi-types`, and ladder, voice, consent, review, veto, late-record, media and fingerprint tests in `hoi-moderation`, `hoi-validate` and `hoi-core`.
- Protocol conformance vectors in `core/vectors/` (canonical bytes, ids, signatures, stamps, identity work, delegation statements, ladder outcomes, media sanitizing) shared by all bindings.
- Simulation harness: N in-process nodes, scripted churn, spam floods; assert convergence and ban behaviour.
- Reproducible builds via Nix or `cargo-dist` with pinned toolchain; release artifacts signed; hashes published on the website.
- CI: GitHub Actions matrix (Linux, macOS, Windows, wasm32, aarch64-apple-ios, aarch64-linux-android).

## 15. Things not to build

- No service-operated signing authority for community membership, posts or moderation.
- No invitation-supplied private relay origins or default labelers; infrastructure origins are deployment configuration.
- No analytics, crash reporting or update checks that phone home beyond fetching a signed release manifest from the static site.
- No server-computed feed, tally or recommendation.
