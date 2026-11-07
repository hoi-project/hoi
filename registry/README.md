# Public community name registry

Optional. The published app uses no registry and finds communities through the nodes people run;
a registry is for public names, in copies of the app built to use one. Anyone can run
this independent API node. It reserves a public name for a community
key and supplies signed discovery metadata. It serves no HTML, app assets or user content.
Browsing, signature checking, name pinning and peer selection happen in the client app.
The Node server needs no frontend build or package dependencies. This is preview software;
`release.json` defines protocol compatibility and index edition.

The database keeps permanent `name → community` pairs with two uniqueness constraints,
plus separate `community → {protocolVersion, indexVersion}` public index metadata and retained
owner-signed registration/publication proofs. It has no post, message, profile, owner,
client-address column, and no access log. The one table with a time in it holds **public node
announcements**: the routes a carrier signed for itself, with the keys of the open communities
it carries, listed for thirty minutes after the announcement and replaced by the next one; stale rows are pruned on subsequent announcements. These
are routes deliberately advertised by a node; addresses and public keys can still relate to people.
Two more tables are the operator's own lists: listings withheld by name or key, and
nodes whose announcements are refused, each with the time it was added and nothing else.
The registry cannot deliver posts: a reachable community node must still hold those records
until their expiry. It points apps at such nodes: the ones the operator configured and the
ones that announced themselves.

The registry is a naming and discovery service, never an authorization service or the
community's identity. A central operator can omit an entry or stop the service; it cannot
change the community key, sign its records or disable an existing key-based invitation.
Compatibility filtering never releases an old name for somebody else. Communities remain
reachable by their keys and compatible software, even when absent from current discovery.

Browser clients verify retained Ed25519 proofs before accepting a lookup, list or conflict.
They atomically pin each verified name/key and publication version in IndexedDB, scoped to the
registry URL. A changed key/version, publication downgrade, missing pinned name, or unavailable
pin storage stops discovery; there is no bypass/reset button. Browser tabs share transactional
pins. A signature proves the community key approved a name, not a real-world identity or who
registered first. Clearing browser data removes local pins.

Every registry keeps a public log of names: each claim it retained, in order, in an
append-only Merkle tree (RFC 6962) whose head it signs with its own log key, kept in its database.
Any app may read `GET /v2/log/head`, `/v2/log/entries?start=&end=`, `/v2/log/proof?leaf=&size=` and
`/v2/log/consistency?first=&second=`. Apps check every exact lookup and write against it: the head
must extend the last one the device kept, and the name must be in it. A registry can still choose
what it answers, but it cannot tell two people two keys for one name without logging both, where
anyone reading the log sees it. `hoi-node --registry-mirror URL` reads the whole log of the
registry it mirrors and stops mirroring one that rewrites its log or moves a name. Directory pages
are not checked one by one; a name is checked when it is opened. A withheld listing's entry is
served as its leaf alone.

Database schema 3 retains public signatures. Schema 0–2 migration preserves all reservations
and versions but cannot recover discarded signatures: legacy rows are never released or
invented as verified entries. Their owners must retry the original signed claim/publication.
Unsigned publications are excluded from current discovery, and new clients reject unsigned
exact lookups rather than calling those names available. Back up the database before upgrading;
older servers reject schema 3 instead of silently dropping the proof protections. The database
file is now at version 5: 4 added the operator's two lists, 5 the log of names and its key; a
registry older than that refuses the file rather than serving what its operator withheld. The API's `schemaVersion` (the shape
of its answers) stays 3.

Names are 1–32 ASCII letters, digits, `.`, `_` or `-`, starting with a letter or digit.
They are trimmed and lowercased before reservation. A creation made without reaching the
registry stays provisional. Only a successful reservation makes its name registered;
checking availability first does not reserve anything. Duplicate offline communities
remain different communities and are never silently merged.

## Three ways to run one API

The registry is a role of the node binary as well as a service. All three speak the
same HTTP contract and keep the same SQLite tables, so a database file moves between the first
two by copying it, and one conformance suite (`registry/conformance.mjs`) runs against each:

| | Run | Namespace | Notes |
|---|---|---|---|
| `hoi-node --registry ADDR` | `cargo run -p hoi-node -- --public --registry 127.0.0.1:7845 --registry-url https://registry.example.org --registry-origin https://hoi.sh` | `registry.db` in the node's data directory | one binary for a public node and its registry; `--registry-mirror URL` imports another registry's verified listings; see the [hoi-node guide](../core/crates/hoi-node/README.md) |
| Node service | `node registry/server.mjs` | `HOI_REGISTRY_DB` | no package dependencies; the sections below |
| Worker | `wrangler deploy --config registry/wrangler.toml` | one SQLite Durable Object | serves the API under `/registry` |

A registry is one participant among others. It arbitrates names within its own namespace while
it runs; a registry that mirrors it (`hoi-node --registry-mirror URL`) copies the versioned
listings whose proofs it can verify, when the name is still free there, and holds them when it
stops; legacy v1 names without a versioned proof are not mirrored, and nothing is ever taken
over. Independent registries cannot promise one global name: when none is reachable, two
communities may carry the same name under different keys, and the apps show both keys without
merging them.

## Separate from the static app

The static website and app (`deploy/static`) are files only. They contain no registry or relay
and do not forward discovery requests. Deploy a registry separately and build the client with
its explicit URL. No endpoint is chosen by default.

An optional standalone Worker is available with `wrangler deploy --config registry/wrangler.toml`.
It serves the API under `/registry` and no website. Its single SQLite Durable Object owns
one namespace. Set `HOI_REGISTRY_ORIGINS` to the client origins, and optionally
`HOI_PUBLIC_NODES`, `HOI_APP_URL`, rate limits, `HOI_REGISTRY_NODE_ANNOUNCEMENTS`,
`HOI_REGISTRY_URL` and `HOI_REGISTRY_ANNOUNCERS` as below. Without `HOI_REGISTRY_URL` the
Worker takes announcements signed for the origin requests reach it at (Cloudflare routes by host
name, so no other registry's origin reaches it); set it when carriers are given a different
address for the same Worker. The operator's lists are configuration here, see
[Operator tools](#operator-tools-withhold-a-listing-drop-a-node).
The Node server exposes the same API at its root. Neither deploy command is part of the
static site's deployment workflow ([static deployment guide](../deploy/static/README.md)).

## Run a standalone Node registry

Requires Node 22.13 or newer with built-in `node:sqlite` (tested with Node 24). The running
server has no package dependencies. From the repository root:

```powershell
$env:HOI_REGISTRY_ORIGINS = 'https://hoi.sh,http://tauri.localhost,tauri://localhost'
$env:HOI_REGISTRY_DB = 'G:\hoi-data\community-names.db'
node registry/server.mjs
```

The database parent directory must already exist. The default listener is
`127.0.0.1:7845`, with `hoi-registry.db` in the working directory. Configure
`HOI_REGISTRY_HOST`, `HOI_REGISTRY_PORT`, `HOI_REGISTRY_DB` and comma-separated
`HOI_REGISTRY_ORIGINS` as needed. Origins must be exact HTTP(S) origins with no trailing
slash, or the explicit desktop origin `tauri://localhost`. The opaque browser origin
`null` is always refused. With no allowed origins, browser requests carrying an Origin are refused;
non-browser administrative clients can still use the API. No cookies or credentials are
required or sent.

`HOI_APP_URL` sets the browser application's HTTPS base URL (default `https://hoi.sh/app/`;
loopback HTTP is allowed for development). `HOI_PUBLIC_NODES` is an optional JSON array of at
most 16 public infrastructure routes listed ahead of announced ones. Supply RTC routes for
browsers/phones and native endpoint routes for desktop nodes, for example
`rtc:<base64url certificate fingerprint>@203.0.113.10:7844` and
`<64 lowercase hex endpoint key>@203.0.113.10:7843`. IPv6 socket addresses use brackets.
Use the routes actually advertised by your deployed node. These are public deployment
settings, not member addresses submitted by communities. Carriers started with
`hoi-node --index <this registry>` list themselves without any of this; set
`HOI_REGISTRY_NODE_ANNOUNCEMENTS=off` to refuse that and `HOI_REGISTRY_ANNOUNCEMENTS_PER_MINUTE`
(default 120) to bound it.

Set `HOI_REGISTRY_URL` to the public URL carriers are given (`https://registry.example.org`, or
`https://example.org/registry`; only its origin counts). Every announcement is signed for one
registry's origin, so this registry refuses one meant for another (400 `wrong-registry`) and
nobody can replay what a carrier sent elsewhere. Without `HOI_REGISTRY_URL` the service takes
only announcements signed for the loopback address a connection reached it at
(`http://127.0.0.1:7845` or `http://localhost:7845`), for development; behind a TLS proxy, or
listening on `0.0.0.0`, it takes none until the URL is set, and says so at startup.

**Allowlist mode.** If the registry fills with junk announcements, set `HOI_REGISTRY_ANNOUNCERS`
to a comma-separated list of the endpoint keys whose announcements you accept (at most 1024).
Every other node is refused with 403 `announcer-not-allowed`, and nodes that announced before
are no longer listed. `node registry/admin.mjs list` shows the endpoint keys announced right now
to choose from; a carrier's operator finds theirs with `hoi-node admin --data DIR endpoint`.
Empty or unset: any node may announce. A node must be reachable from the visitor's network and retain the
community's signed records for invitationless joining and offline-author delivery; a listing
cannot create that availability, and the registry cannot check reachability. Apps try the
listed routes and verify everything they receive.

`HOI_RELEASE_CONFIG` optionally points to a replacement release manifest. The registry
validates it at startup and reads it for each metadata/directory request, so updated notices
and compatibility filters take effect without a rebuild. A malformed replacement fails
closed with 503. Changing this file does not upgrade old records or installed clients.

Put a TLS reverse proxy in front of the loopback listener for production and point clients
at its HTTPS URL. Disable access logs and request-body capture on that proxy too if the
deployment must retain no request metadata. Do not place the database under the static
website directory. Back it up with SQLite's backup tooling, including committed WAL
contents; copying the main file alone while the process is running is not a safe backup.
Run a single registry database for a public namespace. Independent copies cannot give a
shared uniqueness guarantee. Multiple workers may use the same local SQLite file; do not
place it on an unsupported network filesystem or replicate it with last-write-wins KV.
Unique names are guaranteed within this registry's namespace; independently operated
registries can reserve the same name for different IDs.

The container runs as the unprivileged `node` user (UID 1000). Build from the repository
root and use a persistent named volume, initially owned by that user:

```sh
docker build -f registry/Dockerfile -t hoi-registry .
docker run --name hoi-registry --restart unless-stopped \
  --read-only --cap-drop ALL --security-opt no-new-privileges \
  -p 127.0.0.1:7845:7845 -v hoi-registry-data:/data \
  -e HOI_REGISTRY_ORIGINS=https://hoi.sh,http://tauri.localhost,tauri://localhost \
  -e HOI_REGISTRY_URL=https://registry.example.org \
  hoi-registry
```

Keep the published host port on loopback behind the HTTPS reverse proxy. For a bind mount,
create the directory with write access for UID 1000 before starting the container. SQLite
needs to create its WAL and shared-memory files alongside the database in `/data`.

`HOI_REGISTRY_REQUESTS_PER_MINUTE` (default 600) and
`HOI_REGISTRY_WRITES_PER_MINUTE` (default 30) bound work with global, in-memory counters.
No requester identifiers are retained. The global counters do not provide fairness under
attack: an operator should size them and configure upstream abuse controls for their
deployment. A valid community key proves control of that ID, not that a name claim is
desirable. Registration policy, name squatting and operator-mediated dispute handling
remain operational decisions. There is no rename, transfer or delete API, and no administrative
endpoint of any kind: the operator's tools below work on the database from a shell.

## Operator tools: withhold a listing, drop a node

An operator can stop serving an identified listing or node without an admin endpoint and without
signing anything on a community's behalf. Withholding a listing keeps the reservation:
nobody else can claim the name or the key (409 `withheld`), lookups by name or key answer 404
`withheld` without the mapping, prefix lists and mirrors leave it out, and `/v2/nodes?community=`
names no carrier for it. Dropping a node deletes its current announcement and refuses its next
ones (403 `announcer-refused`). Neither touches the community itself: its key, records and
invites stay valid wherever else they are held, and carriers keep what they already carry until
their own operators act ([hoi-node](../core/crates/hoi-node/README.md#stop-serving-something-operator-tools)).

| | Withhold a listing | Drop a node | See both lists |
|---|---|---|---|
| Node service | `node registry/admin.mjs hide NAME\|KEY` | `node registry/admin.mjs drop-node ENDPOINT` | `node registry/admin.mjs list` |
| `hoi-node --registry` | `hoi-node admin --data DIR registry hide NAME\|KEY` | `hoi-node admin --data DIR registry drop-node ENDPOINT` | `hoi-node admin --data DIR registry list` |
| Worker | add it to `HOI_REGISTRY_WITHHELD` | add it to `HOI_REGISTRY_REFUSED_NODES` | the two variables |

`unhide` and `allow-node` undo them. The Node service's tool takes `--db FILE` (default
`HOI_REGISTRY_DB`, then `hoi-registry.db`) and refuses to create a missing file; it and the
`hoi-node` subcommand work while the registry runs, which reads the database on every request.
In a container: `docker exec hoi-registry node registry/admin.mjs list`. The Worker keeps its
namespace in a SQLite Durable Object, which `wrangler d1 execute` cannot reach (it is not a D1
database) and no shell can; its lists are therefore configuration, comma-separated names, keys or
endpoint keys, set with `wrangler secret put HOI_REGISTRY_WITHHELD --config registry/wrangler.toml`
(a secret, so the list is not published with the code) or as a `[vars]` entry, and applied when
the new version starts: configuration is the whole list, so removing an entry lifts it.

The procedure, whichever implementation:

1. **Identify** the listing by its exact name or community key (`GET /v2/communities?name=…`),
   or the node by the endpoint key `list` shows next to the announcements.
2. **Keep a copy if you want one.** The registry holds only public signed claims: save the
   lookup answer (it carries the owner's signed proof) before withholding, and keep it outside
   the registry's data directory. Content lives on carriers, not here.
3. **Act**: `hide` or `drop-node` as above. Note the time; the tool keeps no reason or reporter.
4. **Verify**: the lookup answers 404 `withheld`, the name is absent from `?prefix=`, and
   `/v1/meta` no longer lists a dropped node's routes. Tell the operators of the carriers that
   hold the community if it must stop being served too.

Backups: a database backup taken before the change still holds the listing as served; a
restored backup serves it again until you repeat the step. Keep backups only as long as your
retention rules say, and re-apply the lists after restoring one.

## Connect the apps

All app builds sharing a public namespace must use the same registry. Every build defaults
to no registry, and the project's own deployment pins it empty. Set an independent service's
base URL before building your copy of the client:

```powershell
$env:VITE_HOI_REGISTRY_URL = 'https://registry.example.org'
pnpm build:web
pnpm --filter @hoi/desktop build
```

The web build allows that exact origin in its content security policy. The desktop package
scripts wrap the Tauri CLI and add the same exact origin; use those scripts rather than
bypassing them with a raw `tauri build`. For desktop builds, set the shell environment
variable, not only a Vite `.env` file, so the native policy and web bundle agree. Add the
actual web and desktop origins to `HOI_REGISTRY_ORIGINS` on the registry. For local browser
development use `VITE_HOI_REGISTRY_URL=http://127.0.0.1:7845` and allow the Vite origin.

All builds without a registry URL keep names local and unregistered.
Building or deploying the static app never starts or deploys a registry. The app sends no cookies, authentication tokens,
profile names, posts or member node routes to this service. It sends the public name, community
key, protocol/index versions and public publication proof. As with any HTTPS service, its network/proxy layer
processes connection metadata; the service's database and application logs do not retain it.

New communities save their version-bound publication proof on the creating device. A failed or uncertain
reservation can retry from that device with the same key. Older communities have no proof
because their community signing keys were discarded, and remain usable by invite. The
registry does not merge old same-name communities, rename them, or move their content.

## API

All responses use `Cache-Control: no-store`. Paths below are relative to the registry base:
use `/registry/v1/meta`, for example, on the standalone registry Worker, or `/v1/meta`
on the standalone Node service.

- `GET /v1/meta`: `{release, registry: {apiVersion: 2, schemaVersion: 3, indexVersion},
  publicNodes, appUrl}`. `publicNodes` holds at most 16 routes: the operator's configured
  ones first, then those of nodes announced within the last 30 minutes, newest first, one
  route per node per round.
  `GET /release.json` returns just the validated release manifest.
- `GET /v2/nodes?community=<64 hex>`: `{publicNodes}` for one community: the operator's
  configured routes first, then routes of announced nodes that list that key, then the other
  announced nodes, each group one route per node per round. At most 16 routes. Apps use it
  to reach a carrier of the wanted community before trying the rest.
- `PUT /v2/nodes`: a node's own announcement, JSON `{registry, endpoint, routes, communities,
  issuedAt, signature}`, maximum 8192 bytes. `registry` is this registry's origin as the node
  was given it (`https://registry.example.org`; see `HOI_REGISTRY_URL`); `endpoint` is the node's
  32-byte Ed25519 transport public key (its iroh endpoint id) as lowercase hex; `routes` holds
  1–8 unique routes of that node (`rtc:<fingerprint>@<ip>:<port>` or `<endpoint>@<ip>:<port>`;
  a native route must name this same endpoint); `communities` holds at most 64 sorted, unique
  community keys; `issuedAt` is Unix seconds within ten minutes of the registry's clock, so a
  captured announcement cannot be replayed later, and the signed origin means it cannot be
  replayed to another registry at all. The transport key signs these exact UTF-8 bytes, without
  a final newline:

```text
hoi public node v2
<registry origin>
<64 lowercase hex endpoint key>
<issuedAt>
<routes joined by ,>
<community keys joined by ,>
```

  The unnamed version 1 statement is refused (400 `invalid-announcement`): carriers built before
  this change must be updated. An announcement naming another origin gets 400 `wrong-registry`;
  a registry that does not know its origin answers 403 `announcements-disabled`; in allowlist
  mode an unlisted endpoint gets 403 `announcer-not-allowed`; a dropped node 403 `announcer-refused`.

  A newer announcement from the same endpoint replaces the earlier one; an older one changes
  nothing. Routes must name addresses outside a visitor's own network (no loopback, private,
  link-local or carrier-grade NAT ranges). The response is what discovery now holds for that
  node. At most 64 announced nodes are kept: when the list is full, the nodes that were there
  first stay and a newcomer gets its announcement back with `retained: false`; a node unheard
  of for 30 minutes drops out and frees its place. Invalid or unverifiable announcements
  receive 400; 403 `announcements-disabled` when the operator turned them off; 429 with
  `Retry-After` under their own budget (reads, name claims and announcements each have one,
  so a flood of reads cannot block announcements). There is no withdrawal API: stop
  announcing and the listing lapses.
- `GET /v2/communities?community=<64 hex>`: the same listing as the exact name lookup, for a
  carrier that was offered a key and wants to know whether the index lists it. 404 for an
  unknown key.
- `GET /v2/communities?name=hiking`: `{name, community, protocolVersion, indexVersion,
  compatible, indexed, proof}` or 404 (`not-found`, or `withheld` for a listing its operator
  withholds: still reserved, never served). `proof` contains `{version: 2, signature,
  protocolVersion, indexVersion}` for a publication, `{version: 1, signature}` for a
  signed reservation, or `null` for an unsigned legacy row. Clients recompute compatibility
  from authenticated versions and their own supported protocols. An old reservation without version metadata has null
  versions and false flags. An incompatible entry remains reserved with false flags.
- `GET /v2/communities?prefix=hi[&after=hiking]`: at most 50 alphabetically ordered,
  currently compatible entries. The last returned name is the next page's exclusive
  `after` cursor. Filtering happens before pagination; empty prefix lists all current names.
- `PUT /v2/communities`: JSON `{name, community, protocolVersion, indexVersion, signature}`,
  maximum 2048 bytes. The key signs these exact UTF-8 bytes, without a final newline:

```text
hoi community index v2
hiking
<64 lowercase hex community ID>
0.3
1
```

Both version fields are signed. Reservations and index metadata are inserted atomically;
retrying an identical claim is safe. Metadata for a community is immutable: a conflicting
version returns 409 `index-version-conflict`. A legacy reservation can gain metadata only
with a valid version-bound proof for the same name/key. The normal legacy app cannot
generate one after discarding its community key. Old reservations are never silently
assigned the current protocol. Unknown future database schema versions are refused.

The v1 API remains available with its original signature statement and an additional `proof`
response field. V1 reservations alone do not appear in current v2 discovery:

- `GET /v1/communities?name=hiking`: `{ "name": "hiking", "community": "<64 hex>", "proof": ... }`
  or 404. A lookup is not a reservation.
- `GET /v1/communities?prefix=hi`: an alphabetically ordered array of at most 50 mappings.
  An empty prefix lists the first 50 names. This is name discovery; invitations and node
  routes still come from community nodes.
- `PUT /v1/communities`: JSON `{ "name", "community", "signature" }`, maximum 2048
  bytes. `community` is the 32-byte Ed25519 community public key as lowercase hex;
  `signature` is its 64-byte signature as lowercase hex. The community key signs the
  exact UTF-8 bytes below, with no final newline:

```text
hoi community name v1
hiking
<64 lowercase hex community ID>
```

The core creates this proof while creating the community and can retain it locally for
later registration after an outage. The registry checks and retains the public signature
alongside the canonical name and community ID. The same mapping is idempotent (200).
A different community claiming a reserved name receives 409 `name-taken`; the existing
mapping is returned so the UI can offer that community. A community trying to reserve a
second name receives 409 `community-registered`. Malformed or invalid proofs receive
400; origin restrictions 403; oversized requests 413; unsupported content types 415;
rate limits 429 with `Retry-After`; temporary storage failures 503. Retry the same signed
claim after a timeout: an uncertain response must never cause creation of a replacement
community ID.

### Per-address limits at the edge

The registry's own limits are global budgets (reads, claims and announcements each have
their own), so it never keeps a table of client addresses. To stop one address from using
up a budget, put a per-address rule in front of it instead, where the provider already sees
addresses: on Cloudflare, a WAF rate-limiting rule on the registry's hostname (for example
60 requests per 10 seconds per IP for `GET`, 10 per minute per IP for `PUT` and `POST`, with
a block of a minute); behind nginx, `limit_req_zone $binary_remote_addr`. Keep the rule's
logging at the provider's minimum.

## Verify

```sh
node --test registry/server.test.mjs registry/worker.test.mjs
node scripts/e2e-registry-node.mjs     # the same conformance suite against hoi-node --registry; cargo build -p hoi-node first
node scripts/e2e-registry.mjs          # after pnpm core:wasm
node scripts/e2e-registry-browser.mjs  # real browser; needs installed Chromium
node scripts/e2e-public-network.mjs   # real WebRTC carrier; also cargo build -p hoi-node
```

Tests cover competing reservations, idempotent retries, restarts, legacy migrations,
version-bound signatures, compatibility filtering without name reuse, pagination, the
public-only storage schema, CORS, rate limits and body limits, announcements bound to one
registry, allowlist mode and the operator's tools. `registry/conformance.mjs` exports three
suites, run against the Node service and `hoi-node --registry` (and the last two against the
Worker too): the API itself, allowlist mode (`runAllowlistConformance`, against a registry that
allows only `ALLOWED_ANNOUNCER.endpoint`) and the operator's tools (`runOperatorConformance`,
given functions that withhold, lift and drop the way each implementation does). The shared
`core/vectors/community-registration.json`, `community-publication.json` and
`node-announcement.json` fixtures check that Node and the Rust core agree on each exact
statement and Ed25519 proof.

The public-network test uses two separate browser identities and a real node started with
`--index` against a registry that lists no node. The node announces itself; Robin publishes
a community from a browser that never pairs with the node; the node takes the community from
that browser, saves Robin's post and announces that it carries the community; Robin goes
offline. Lena then joins from the registry and receives the post over WebRTC without an
invite, pairing credential or local WebSocket. Its loopback UDP setup verifies the protocol
and durable carrier path; internet deployment still needs a reachable advertised node address.
Because everything runs on one machine, the test starts its registry with
`createRegistryServer({ allowPrivateRoutesForTests: true })`, the one way to make the Node service
take loopback routes in an announcement. It is an option of the function only, never read from
the environment, and the Worker refuses to start if `HOI_REGISTRY_ALLOW_PRIVATE_ROUTES` is set:
production validation of announced routes is unchanged.
