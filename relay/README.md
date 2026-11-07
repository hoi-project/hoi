# Opaque encrypted storage

This service stores client-encrypted objects separately from the public-name registry.
Clients keep the content-encryption and membership keys. The service checks room signatures,
hashes, padded sizes, quotas and expiry; it does not decrypt messages, decide membership,
moderate plaintext or authorize changes to signed community records.

The default deployment is deliberately bounded: one SQLite-backed Durable Object enforces
the entire service's capacity. It is a pilot relay, not an unlimited global archive or a
consensus network. Hosting more objects does not grant voting or signing authority.

## Deploy

The static website deployment contains no relay. Deploy this Worker on its own:

```sh
pnpm dlx wrangler@4 deploy --config relay/wrangler.toml
```

Authenticate Wrangler first. The config creates the SQLite namespace through a migration;
no database identifier or content-decryption secret needs to be provisioned. Keep the
Worker name, class name, binding and migration history stable to retain its stored objects.
Another operator should deploy it in their own account and supply their own HTTPS URL.
Several instances controlled by the same operator are not independent replicas.

Build the app with the complete relay API base, including `/relay`, for example
`VITE_HOI_RELAY_URLS='["https://relay.example/relay"]'`. The client supports up to three
explicitly configured relay bases and has no default.

Set `HOI_RELAY_ORIGINS` to comma-separated exact app origins for an externally hosted app
or native shell, for example `https://hoi.sh,http://tauri.localhost,tauri://localhost`.
The request URL's own origin is always allowed. No cookies or credentials are required.
Opaque browser origin `null` is refused.

The checked-in config disables Worker observability. Application code does not log requests,
IP addresses or bodies. Hosting infrastructure still processes connection metadata; these
settings do not promise anonymity or control every provider-level log.

## Protocol version 2

Version 2 keeps the relay from learning who is in a group or which community it is,
and keeps people without the group's keys from reading its traffic pattern.

**Rooms.** Every MLS epoch of a private group has its own room. Each member exports a 32-byte
relay secret from the epoch (MLS exporter, label `hoi relay v2`, empty context). BLAKE3
`derive_key("hoi relay v2 room key", secret)` seeds an Ed25519 room key; its public half is
the room id. Every member of the epoch derives the same key and signs uploads and reads with
it, so the relay sees one key per room and no member's device or identity key. Knowing the
community key, or holding an invitation meant for someone else, derives nothing.

A membership commit is uploaded to the room of the epoch it ends: members still in that
epoch read it there and move on. A newcomer joins through a Welcome for the new epoch and
reads the new room, where members resend the live history they keep. A removed device reads
its removal in the old room and cannot derive the next one. It can still sign for the old
room until those objects expire, which reveals nothing new and which other members no longer read.

**Objects.** The client seals each MLS message with XChaCha20-Poly1305 under BLAKE3
`derive_key("hoi relay v2 object", secret)` and a random 24-byte nonce:
`nonce ‖ AEAD(length as u32 big-endian ‖ MLS message ‖ zeros)`. The relay therefore never sees
the MLS header (whose group id is the community key, and which names the epoch and whether a
message is a commit). Sealed objects are padded to sizes of at least 4 KiB with at most four
significant bits (4,096, 4,608 … 7,680, 8,192, 9,216 … 1 MiB): 65 sizes, at most one eighth
above the content past 4 KiB. The relay refuses every other size.

## HTTP contract

All responses use `Cache-Control: no-store`.

- `GET /relay/v2/meta` returns `apiVersion: 2`, object/size/page/lifetime/read-window limits,
  configured quotas and `removed`, the number of identifiers removed by the operator.
- `PUT /relay/v2/rooms/<room>/objects/<id>` stores one object. `room` is the lowercase 64-hex
  room key; `id` is the lowercase SHA-256 hash of the binary body.
- `GET /relay/v2/rooms/<room>/objects?after=<sequence>` returns `{objects:[{id,sequence,size}],nextCursor?}`.
  Pages contain at most 64 entries in increasing sequence order. `after` is exclusive;
  `nextCursor` is present only when more entries remain.
- `GET /relay/v2/rooms/<room>/objects/<id>` returns the body and nothing else about it.
- Every `/relay/v1/` path answers 410 `protocol-gone`.

Uploads use `Content-Type: application/octet-stream`, a padded body and these headers:

```text
X-Hoi-Expires: <integer Unix seconds, at most 31 days ahead>
X-Hoi-Signature: <128 lowercase hex signature by the room key>
```

The room key signs the exact UTF-8 statement, with no final newline:

```text
hoi relay object v2
<room>
<id>
<expires>
<bodyByteLength>
```

Version 1 writer and grant headers (`X-Hoi-Author`, `X-Hoi-Grant-*`) are refused. Uploads
return `{id,sequence,size,expires}`: 201 for a new object, 200 for an identical retry.

Listing and fetching each carry a read signature made for that one request:

```text
X-Hoi-Read-Time: <integer Unix seconds within 300 s of the relay's clock>
X-Hoi-Read-Nonce: <32 lowercase hex, fresh per request>
X-Hoi-Read-Signature: <128 lowercase hex signature by the room key>
```

```text
hoi relay read v2
<room>
<target>
<time>
<nonce>
```

`target` is the request below the room exactly as sent: `objects?after=<sequence>` or
`objects/<id>`. The relay accepts each read once. It keeps
`SHA-256("hoi relay read v2\n<room>\n<nonce>")` with the end of its window and refuses the
same nonce again, also after a restart; the digest does not name the room. Readers receive no
expiry, signature, writer or grant: only what they need to fetch and check an object.

Failures include 400 for invalid proofs, hashes, cursors or `unpadded` sizes, 401
`read-denied` for a missing, stale, forged or replayed read signature, 403 for a disallowed
origin, 404 for missing or expired objects, 408 for a slow upload, 409 for conflicting
envelopes, 410 for operator-removed objects and rooms (only after a valid room signature)
and for version 1 paths, 413 for oversized objects, 429 for rate limits, and 507 for exhausted
storage quotas. A storage failure returns a generic 503 without database details.

## Storage and limits

Defaults and their maximum configurable values:

| Setting | Default / maximum |
| --- | --- |
| Object bytes | 4 KiB to 1 MiB in padded sizes, fixed |
| Object lifetime | 31 days from upload, fixed |
| Read signature window | 300 seconds either way, fixed |
| `HOI_RELAY_MAX_BYTES` | 512 MiB |
| `HOI_RELAY_ROOM_BYTES` | 64 MiB |
| `HOI_RELAY_ROOM_OBJECTS` | 4,096 |
| `HOI_RELAY_MAX_OBJECTS` | 32,768 |
| `HOI_RELAY_REQUESTS_PER_MINUTE` | 600, configurable up to 100,000 |
| `HOI_RELAY_WRITES_PER_MINUTE` | 60, configurable up to 10,000 |

Storage byte/count limits can be reduced, not raised above these bounds. Rate counters are
global and in memory; they retain no requester identifiers and reset when the object restarts.
They bound work but do not provide per-person fairness. Admission is cryptographic possession
of an epoch's room key, not proof that an account is a distinct human.

The database holds, in `objects_v2`, ciphertext with its room key, hash, padded size, expiry
and insertion sequence; in `reads`, one digest per accepted read until its window closes; in
`removed`, the identifiers the operator removed. It holds no writer keys, grants, community
keys, MLS headers, content keys, names, profiles, member lists or posts in plaintext. Clients
must encrypt before uploading: the relay cannot prove that an arbitrary binary payload is encrypted.

What the relay still observes: a room's object count, padded sizes, upload and read times and
expiry, and the network addresses and timing of each request. The rooms of one group are not
linked by any identifier, but uploads that follow each other from the same addresses can link
them, and the number of objects resent after a commit hints at a group's size and history.

Objects are create-only. An identical signed retry succeeds; a conflicting signed envelope
for an existing object fails. Expired ciphertext may be uploaded again only with a fresh valid
signed envelope. Clients must keep durable copies and arrange renewal/replication for any
longer retention requirement.

## Version 1 objects

The project is a preview and version 1 is not served. A relay upgraded from version 1 keeps
its v1 rows in the `objects` table, unreadable through the API, counts them against the
service quota and deletes each at its expiry, at most 31 days after upload. To delete them
at once, remove `v1` with the operator tool below. Clients upgraded to version 2 send live
records and media again into the current epoch's room; a membership change that was still
waiting for a v1 relay is lost, so remove and re-invite the devices it concerned, and ask
for a new invitation where one was made before the upgrade.

## Removing objects (operator)

There is no HTTP endpoint for deletion. An operator who wants to remove identified objects or
rooms (a report that names a room id or a room and object id) deploys the
list as the Worker secret `HOI_RELAY_PURGE` with the local tool, which runs Wrangler:

```sh
node relay/operator.mjs remove --dry-run <room>/<id> <room>
node relay/operator.mjs remove --check https://relay.example/relay <room>/<id> <room>
```

Entries are `<room>` (every object of that room, and no uploads to it afterwards),
`<room>/<id>` (one object, which cannot be stored again) or `v1` (every legacy object),
separated by commas or whitespace, at most 4 KiB per batch. Setting the secret deploys a new
Worker version; the Durable Object applies the list when it next starts, inside one
transaction with the schema check, and records each identifier in `removed`. `--check` wakes
it through `/relay/v2/meta` and prints the `removed` count before and after. Each run replaces
the secret; earlier batches stay recorded in storage. Members of a removed room get 410
`removed` after a valid room signature; nobody else learns whether a room existed or was
removed. Removal does not touch members' devices, which keep their own copies, and cannot
identify who uploaded an object. For a local Worker, pass the same list to
`wrangler dev --var HOI_RELAY_PURGE:<list>`. An invalid list stops the relay with 503 rather
than serving data it was told to remove; the tool validates entries before calling Wrangler.
The meta count is public, like a transparency report; the identifiers are not.

## Retention and backups

- Objects are deleted at their expiry, at most 31 days after upload (clients choose 30), by a
  Durable Object alarm or when a request touches them.
- Read digests are deleted by an alarm shortly after their 300-second window closes.
- Removed identifiers stay in `removed` for the life of the storage so the same bytes and rooms
  cannot return. They are random hashes and keys, not content.
- This repository makes no backup copies. Cloudflare offers point-in-time recovery of
  SQLite-backed Durable Object storage for the preceding 30 days; this Worker never invokes
  it. A restore to an earlier time also restores the earlier `removed` table, so run every
  removal batch again after any restore. An operator who exports or copies the storage must
  apply removals to those copies or delete them within 31 days.

## Trust boundary

An operator can withhold, delete or replay stored ciphertext and can inspect the metadata
above. Clients must validate downloaded hashes and authenticated content themselves and
must not treat the service's list, ordering or receipt as permission to accept a message.
Independent replicas reduce dependence on one host; they do not solve malicious membership,
collusion or the authenticity of a newly downloaded client application. The relay is not a
membership authority and makes no “51% attack impossible” claim.

## Verify

```sh
node --test relay/*.test.mjs
pnpm dlx wrangler@4 deploy --config relay/wrangler.toml --dry-run
pnpm dlx wrangler@4 dev --config relay/wrangler.toml --local
```

The tests exercise real SQLite transactions and Ed25519 signatures, including signed and
replayed reads, padding bounds, tampering, immutable retries, pagination, expiry, quotas,
operator removal, the version 1 upgrade, restart persistence, and a vector shared with the
core's `relay_v2_shared_vector` test.
