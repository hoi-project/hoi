# hoi-node

The hoi core without a window, in one of three roles that compose:

- a **home node** (the default): keeps your communities warm, serves your phone and browser, and
  relays for members while the switch is on. It is a **public relay** too
  unless started with `--no-public-relay`: it carries open communities for anyone who
  asks, has your router forward its UDP ports, tells whoever connects about the other public
  nodes it reached, and writes a link anyone can open to `DIR/node-link`;
- a **public node** (`--public`): the same with no person behind it and no pairing link. It stores
  and serves the open communities it is given (by connected peers that ask it to carry theirs,
  and by a registry it mirrors with `--index`, if any). It has no standing anywhere: it never
  signs a community record and cannot vouch anyone in;
- a **registry node** (`--registry ADDR`): the public-name API in this binary, one namespace, the
  same API and database shape as the Node service and the Worker in `registry/`, so anyone can
  run one on their own machine and a namespace can move between the two by copying its file.

Nothing in this binary names a node, a registry or a relay to start from. A node finds others
through the invites it joins (`--join`), the node links it is given (`--node`), the devices that
pair with it and the nodes that reach it, and passes on the public nodes it reached itself
([below](#a-public-relay-at-home-the-default), and [`deploy/home`](../../../deploy/home/README.md)
to install it as a service).

Every role connects to other native nodes over authenticated iroh QUIC using explicit peer and
relay hints. Operating an encrypted packet relay (embedded iroh-relay) remains separate work.

```sh
cargo run --release -p hoi-node -- --data ~/hoi-node --listen 127.0.0.1:7842 --name "lena's pi"
```

On start a public node (any node but a `--no-public-relay` one) prints the link anyone may open to reach it and writes it to `DIR/node-link`: `https://hoi.sh/app/#/node?code=…`, its internet routes first once the router forwarded its ports. The code packs the node's addresses (and its signed notice) the way a community invite packs its routes: compressed and masked, so the IP addresses can't be read from the link, not even with a Base64 decoder; hoi undoes it when the link is opened. Masking is not encryption: it keeps addresses out of casual sight, and whoever connects still learns the node's address. Older `#/node?w=…&p=…` links still open. It is an address, not a credential: share it where you want people to find the communities it carries. A home node writes its pairing links to `DIR/pairing-link`, a file only the node's user can read: a `hoi://pair?code=…` link and, when it found a network address, a `https://…/#/pair?code=…` link with its WebRTC routes. Like the node link, the code packs the address, token and routes out of sight: no IP address can be read from it. The links carry the pairing token, a credential, so they stay out of the terminal and therefore out of service journals and container logs; start with `--show-pairing` to print them as well (`cat DIR/pairing-link` works at any time, `docker exec hoi-node cat /data/pairing-link` in a container). Paste the first in hoi → **You → Home node** on a browser on the same machine; open the second on a phone (or make a QR code of it): the phone connects over WebRTC on your network, no certificate needed. Each link pairs one device: the first device to use it gets a credential of its own and the link is spent, so the node writes the next link to the same file at once (an already paired device shows it too). Devices without a valid credential hear nothing; browsers may open the listener only from the app's origin (`https://hoi.sh`) or one listed in `HOI_ALLOWED_ORIGINS` (comma separated). `--new-pairing` issues a new link and forgets every device's credential; all of them pair again. A node without a pairing listener (`--peer-only`, `--public`) writes no file and removes one left from an earlier run.

**Operator notice.** A public node tells people who runs it before it serves them: `--notice FILE` is a JSON file with what the node processes and whether the host keeps logs of addresses and for how long (the node software itself never logs them), and, optional but recommended, the operator's name or pseudonym, contact, the server's country and provider: left empty, people read that the operator left them out; filled in, they are shown to everyone who connects. The installers and the app's *A node on a server* page write it; `hoi-node admin notice check FILE` says what keeps one from being used, notes what was left out, and prints it as people read it. The node puts the notice into every hello to a stranger and, signed with its endpoint key, into its node link, so a person's browser shows it before it says anything of its own, and before it connects at all when given the link; a changed file reaches connected strangers within half a minute, and they read it again. `DIR/notice-hash` holds the hash of the notice the node carries now.

| Flag | Default | |
|---|---|---|
| `--data DIR` | `./hoi-node-data` | SQLite file `hoi.db` and the node's keys; `registry.db` with `--registry` |
| `--name NAME` | `home node` | what your devices call it ("via lena's pi"); a public node's label |
| `--public` | | a public node: `--peer-only --relaying --carry any`, and no person's key is made (one already in `DIR` is kept) |
| `--no-public-relay` | | a home node for its person's devices and its communities' members only: no `--carry any`, no `--port-mapping`, no node link (relaying for members stays on) |
| `--public-relay` | the default | a home node that is a public relay too: `--relaying --carry any --port-mapping`; kept for service files written before it was the default |
| `--node LINK` | none | a public node to look at first, by its link (`hoi://node?…` or `…#/node?…`), repeatable; kept once it answered, and passed on |
| `--notice FILE` | none | the operator's notice (JSON; `hoi-node admin notice check FILE` reads it): what this node processes and keeps and, optionally, who runs it, where and how to reach them. Carried in every hello to a stranger and, signed, in `DIR/node-link`; read again when the file changes; its hash in `DIR/notice-hash`. A `--public` node or a registry without one says so at start |
| `--registry ADDR` | | a registry node: serve the public-name API on `ADDR` (see below) |
| `--listen ADDR` | `127.0.0.1:7842` | where paired devices connect (home node) |
| `--new-pairing` | | rotate the pairing token and print the new links |
| `--show-pairing` | | print the pairing links (they are always in `DIR/pairing-link`, readable by the node's user only) |
| `--peer-only` | | run only peer transports; no paired-device control listener or pairing links |
| `--peer-bind ADDR` | automatic | native iroh UDP listening address, independent of the WebSocket port |
| `--relay HTTPS_URL` | none | explicit iroh relay, repeatable; URLs must not contain credentials, a query or a fragment |
| `--public-url WSS_URL` | local WebSocket address | public TLS proxy address to put in pairing links |
| `--join INVITE` | none | join a community and retain its network hints, repeatable |
| `--rtc-port PORT` | `7844` | UDP port for browsers and phones over WebRTC; `0` turns it off; any free port if taken |
| `--rtc-advertise ADDR` | none | also advertise this UDP address (a forwarded port, a public address), repeatable; with `--index` this is the address announced to the registry |
| `--rtc-bind IP` | every interface | serve WebRTC only on this local address, repeatable (several nodes on one machine, tests) |
| `--no-relay` / `--relaying` | relaying | whether members' browsers and nodes may connect and catch up |
| `--daily-cap-gb N` | `5` | gigabytes a day the node may exchange with other people's devices that dialed in; past it they wait for the next UTC day (kept in the node's settings) |
| `--port-mapping` / `--no-port-mapping` | on, except with `--no-public-relay` or `--public` | ask the router to forward the WebRTC and native UDP ports (PCP, NAT-PMP, then UPnP) and advertise the public address it forwards from |
| `--index URL` | none | optional: also announce this node's public routes to that registry (HTTPS; plain HTTP only on this machine) and carry what it lists; repeatable, one registry per flag |
| `--carry MODE` | `any`; with `--no-public-relay`: `index` with `--index`, else `off` | `off`: only communities joined by invite · `index`: what each registry lists, and what browsers offer under a listed key · `any`: that, and carry requests from connected peers for open communities |
| `--carry-limit N` | `256` | communities to carry at most, those still waiting for a member included; requests from peers may fill half |
| `--carry-mb N` | `256` | megabytes the communities carried on peers' requests may hold together; over it, the ones nobody touched for longest are let go and deleted |
| `--refuse KEY` | none | never carry this community, whoever asks or whichever index lists it; one carried already is let go and deleted (repeatable). For a lasting refusal that survives a changed command line, use `hoi-node admin suppress community KEY` (below) |
| `--registry-url URL` | none | the registry's public URL as carriers are given it; announcements must be signed for its origin. Without it, only announcements signed for the loopback address a connection reached (development) |
| `--registry-origin ORIGIN` | none | exact browser origin allowed to call the registry API, repeatable (`tauri://localhost` for the desktop app); without any, browsers are refused and non-browser clients still work |
| `--registry-public-node ROUTE` | none | a route the registry lists before any announced node, repeatable |
| `--registry-no-announcements` | | list only those configured routes; refuse node announcements |
| `--registry-announcer KEY` | none | allowlist mode: take announcements only from these endpoint keys, repeatable; others get 403 `announcer-not-allowed` |
| `--no-lan` | on | don't send or listen for beacons on the local network (UDP 7848 multicast), which members of the same communities find each other with |
| `--registry-mirror URL` | none | import another registry's verified names when they are free here, repeatable |
| `--registry-release PATH` | compiled `release.json` | a replacement release manifest, read on each request (at most 16 KiB; an invalid one is refused) |
| `--registry-app-url URL` | `https://hoi.sh/app/` | the app address the registry's metadata points people at |
| `--registry-limits R,C,A` | `600,30,120` | per-minute budgets for reads, name claims and announcements |

## Run a public node

```sh
cargo run --release -p hoi-node -- \
  --data ~/hoi-public \
  --public \
  --peer-bind 0.0.0.0:7843 \
  --rtc-advertise 203.0.113.10:7844 \
  --index https://registry.example
```

Replace the documentation address with the host's public IP and the registry with one you or your
community run. A public node has no person's key: it stores and serves records, never signs any.
It carries what the registry lists (next section), takes a newly published community from its
founder's browser, and, with the default `--carry any`, carries open communities that connected
peers ask it to carry: any member can ask from the app (the request carries the community's
signed definition and nothing else, no routes), and the node relays the community without being
a member, so newcomers can join by key without an invitation. A community taken on request is
let go and deleted when nobody else delivers or fetches its records for two weeks, when its
records never arrive within an hour, or when `--carry-mb` is used up; `--refuse KEY` keeps a
community off this node for good. `--carry index` turns the requests off; `--carry-limit` bounds
everything. No website or sign-up is served.
Public nodes persist and serve public records, including media, and their operator can read
them; peer-only does not mean storage-free. The independent public-peer Compose deployment
(`deploy/server`) runs this mode.

## A public relay at home (the default)

```sh
cargo run --release -p hoi-node -- \
  --data ~/hoi-node \
  --name "lena's pi" \
  --peer-bind 0.0.0.0:7843
```

No registry is needed. A home node stays a home node (its pairing link, its person's key, your
devices) and does what a public node does on top: relays, carries open communities for anyone
who asks, tells whoever connects about the other public nodes it reached (and dials a few of
them itself, so it hears of new ones), and has a link to share in `DIR/node-link`. With
`--index` it also announces its routes to that registry and carries what it lists. What makes that work from a home network is the router: the node asks it to forward UDP
7844 (browsers and phones) and the native port (7843 here), with PCP, NAT-PMP or UPnP, and
announces the public address the router names. It renews the forward every hour, which follows a
new address from your provider, and removes it when it stops (Ctrl-C, or the SIGTERM a service
manager sends). The log says what happened:

```text
port mapping: asking the router to forward UDP 7844 and 7843
port mapping: the router (UPnP) forwards UDP 203.0.113.7:7844 → 7844, 203.0.113.7:7843 → 7843
node link (anyone who opens it finds the communities this node carries, and the next nodes along):

  https://hoi.sh/app/#/node?code=SE9O…

  it holds 2 WebRTC routes and 1 native route, and no operator notice; the addresses are packed into the code
```

If the router answers none of the three, refuses, or is itself behind another NAT (its address
is not public: carrier-grade NAT, or a second router), the log says which and nothing is
announced from it; forward the ports by hand and pass `--no-port-mapping --rtc-advertise
PUBLIC-IP:7844`. Only the router is asked: no outside service learns about the node. Relaying
for others pauses for the day at `--daily-cap-gb` (5 GB by default). Whoever connects learns your
home's public address and which of its open communities they share, as from any public node; a
registry, if you give one, lists the address and the keys of the open communities the node
holds, your own open ones included. The node stores and can read the public records it carries,
and the operator tools below apply to it as to any public node. `--no-public-relay` turns all of
this off and keeps the node to your devices and your communities' members.

## Run a registry node

```sh
cargo run --release -p hoi-node -- \
  --data ~/hoi-registry \
  --public \
  --registry 127.0.0.1:7845 \
  --registry-url https://registry.example \
  --registry-origin https://hoi.sh \
  --registry-origin tauri://localhost
```

The registry listens on `ADDR` and keeps its names in `DIR/registry.db`. Put a TLS proxy in front
for browsers (they reach a registry only over `https://`), allow the actual web and desktop origins
with `--registry-origin`, and build your copy of the clients with `VITE_HOI_REGISTRY_URL` pointing
at it (`registry/README.md`); the published app uses no registry. `--registry-url` is that same public address: carriers sign each
announcement for one registry's origin, so this registry takes those meant for it and refuses any
replayed from elsewhere (`wrong-registry`); without it, it takes only announcements signed for the
loopback address it was reached at, which is for trying it out on one machine. If junk fills the
announcement list, `--registry-announcer KEY` (repeatable) switches to allowlist mode: only those
endpoint keys are listed and every other node is refused; `hoi-node admin --data DIR registry list`
shows the keys announced right now. The API, the status codes, the headers and the database are the same as
the Node service's and the Worker's: `node scripts/e2e-registry-node.mjs` runs one shared
conformance suite against both and then opens the database one wrote with the other.

A registry is one participant among others. It arbitrates names in its own namespace while it
runs; `--registry-mirror URL` makes it import another registry's verified publications every
five minutes when their names are free here, so a second registry holds the names when the first
stops. A name already taken here by another key stays with the local first claim: both communities
exist, under their distinct keys, and the app shows both. Independent registries remain
independent namespaces; when none is reachable, two communities can carry the same name, never the
same key. Node announcements are not mirrored: a public node announces to each registry it is
given (`--index` is repeatable).

Every registry keeps a public log of the names it reserved and published: an
append-only Merkle tree under its own log key, at `/v2/log/head`, `/v2/log/entries`,
`/v2/log/proof` and `/v2/log/consistency`, readable by any app. A mirror is also the other
registry's auditor. Each pass it checks the other's signed head, reads its new log entries, keeps
every leaf, recomputes the root, and checks that no name moved to another key. It imports only
names that are in that log. A registry whose log changed its key, shrank, does not match its head
or moved a name is no longer mirrored, and the node says why once. After looking into it,
`hoi-node admin registry unwitness URL` starts that audit over.

A registry node that should also carry what it lists adds `--index https://its-public-url`: the
carrier talks to the registry over its public address like any other node, so only public routes
are announced. The status line says `registry listening on ADDR · N names · M announced nodes`;
nothing about clients is ever printed or stored (no access log, no requester table).

## Join a community without a desktop window

Run from `core/`, replacing the quoted invite with one from a running member:

```sh
cargo run --release -p hoi-node -- \
  --data ~/hoi-node \
  --peer-bind 0.0.0.0:7843 \
  --join 'hoi://join?c=COMMUNITY&p=PEER&g=GENESIS'
```

The example invite is a placeholder; use the complete link including its signed genesis.
The community and hints persist in the node's database. `--join` can be repeated for more
communities. A browser paired to this node can also join an invite: its bootstrap hints are
passed to its home node. The native node advertises its own routes back to paired browsers
for fresh invites. A paired desktop can use the same home-node route exchange, including
when a founder adds a relay through `hoi-node --relay` rather than a desktop settings field.

## Carry a public index (optional)

```sh
cargo run --release -p hoi-node -- \
  --data ~/hoi-carrier \
  --public \
  --peer-bind 0.0.0.0:7843 \
  --rtc-advertise 203.0.113.10:7844 \
  --index https://registry.example.org
```

Replace the documentation address with the host's public IP. With `--index` the node does three
things on its own, every five minutes and whenever something changes (a home node can do this too;
`--carry off` keeps only the first):

- **Announces itself.** It signs its public routes (the `rtc:` route for browsers and phones, the
  native route for desktop nodes) and the keys of the open communities it holds with its
  transport key, for that registry's origin only, and sends them to the registry; no registry can
  pass the announcement on to another. Apps asking the registry for public nodes get this node's
  routes; the registry forgets it thirty minutes after its last announcement. Only routes with
  public addresses are announced (every route counts when the registry runs on the same machine,
  for tests). A LAN-only node announces nothing and says so. A registry in allowlist mode lists
  only the nodes its operator allowed: give its operator this node's endpoint key
  (`hoi-node admin --data DIR endpoint`).
- **Mirrors the index.** It asks for every community the registry lists for this build's
  protocol family and index edition, up to `--carry-limit`, and waits for a member to connect
  with the records. Nothing from the registry is trusted beyond the key and name: the signed
  genesis still has to match that name and be open, or the community is refused.
- **Takes new communities from browsers.** When a browser offers a community this node does not
  hold, the node looks its key up in the registry and, if it is listed, pulls the signed
  definition and records from that browser. A founder who makes a public community in the app
  and reserves its name is carried within a minute, without pairing or an invite.

It prints `now carrying c/<name>` for each community taken on and one status line per change
for the announcement, nothing about who connected. Relaying must be on for any of this to reach
anyone; the node warns when it is off. An index pass can list 2,000 communities; the node takes
them 32 at a time and lets its peers and listeners have it in between. See
[`deploy/server/README.md`](../../../deploy/server/README.md) for the container that runs this
next to the website, app and registry.

## Hold encrypted pieces (preview, optional)

`--hold-pieces GB` makes the node keep up to that many gigabytes of other people's encrypted
pieces in `DIR/pieces.db` and answer sealed requests for them, for scattered delivery: a post is
cut into pieces so that no single node holds or can read a whole one. The node publishes a key in
its hellos (derived from its endpoint key, changing every 30 days), checks the small proof of work
each piece carries, keeps nothing about who sent a piece, drops a piece only for whoever stored it
or a community's moderators, and lets expired and long-unasked pieces go when it is full. It can
read none of what it holds and holds at most one piece of any post. Devices reach holders
through other public nodes (forwarders), so a holder sees the forwarder's address, not the address
of whoever stores or fetches a piece; mailboxes of private groups are still reached directly.

Every public node also forwards, whether or not it holds pieces: it passes a sealed request on to
the holder it names and the sealed answer back, reading neither, and keeps nothing but what
routing an answer takes while it is on its way and two counters. It passes requests only to
holders it is linked to or saw itself (and dials those for a moment); anything else is answered
"gone" at once.

```sh
hoi-node --public --hold-pieces 50 --data ~/hoi-node   # hold up to 50 GB
hoi-node admin --data ~/hoi-node pieces                 # how many pieces, how much, nothing more
```

It is off by default while the feature is in preview.

## Stop serving something (operator tools)

The operator of a node can stop serving an identified community, record or media file, without
any admin endpoint and without signing anything on anyone's behalf: what is suppressed
is deleted here and refused for good, whoever offers it, asks for it or lists it, across restarts.
Other nodes keep their copies and decide for themselves; the community's key, signatures and
invites are untouched. The tools work on the data directory from a shell, with the node stopped
(a lock file stops them while it runs, and a second node from starting on the same directory):

```sh
hoi-node admin --data DIR suppress community KEY [--note TEXT] [--evidence DIR]
hoi-node admin --data DIR suppress record ID     [--note TEXT] [--evidence DIR]
hoi-node admin --data DIR suppress media HASH    [--note TEXT] [--evidence DIR]
hoi-node admin --data DIR suppressions           # what is suppressed, when, your note
hoi-node admin --data DIR unsuppress KEY|ID|HASH # lift it; nothing deleted comes back by itself
hoi-node admin notice check FILE                 # an operator notice: what keeps it from being used, or its hash and text
```

- **community KEY**: deletes the community's records and the media only they used, stops
  announcing it, and refuses it from every path: a peer's carry request (`refused`), an index
  listing, a browser offering it, an invite (`--join`), a public join and sync. Unlike `--refuse`,
  it is stored with the node's state, survives a command line without the flag, and also covers
  a community the operator joined by invite.
- **record ID**: deletes one post, comment, edit, report, profile or profile colour (a post or
  comment with its edits) and never takes it again; sync never asks for it. Records that decide
  who may speak or what is hidden (a community's definition, identities, devices, revocations,
  the ledger, vouches, reviews, deletions, labels, votes) cannot be suppressed one by one, since
  removing them could bring back what a community removed: suppress the community, or the post.
- **media HASH**: deletes the media file (its BLAKE3 hash, as in the post's media reference;
  a profile picture by the hash of its image) and refuses the bytes from now on; posts that show
  it are served to nobody.
- `--note TEXT` is your own short reference, a case number say; never anyone's personal details.
- `--evidence DIR` first copies what is about to go into a new directory under `DIR`, readable by
  this user only (0700/0600): each signed record exactly as it arrived (`records/<id>.hoi`, which
  verifies on its own wherever it is kept), the media (`media/<hash>`) and a `manifest.json` with
  ids, kinds, authors' keys and times. Off by default: keep a copy only if you need one, outside
  the node's data directory, and delete it when you no longer do. If the
  copy cannot be written, nothing is deleted.

The procedure: **identify** the item by its key, id or hash (the app shows a post's id and its
community's key; a carrier prints `now carrying c/<name>`, and a registry lookup gives the key);
**stop the node** (`docker compose -f deploy/server/compose.yml stop public-carrier`, or
`systemctl stop hoi-node`); **keep a copy if you want one** with `--evidence`; **suppress**; **start
the node** and **verify**: `hoi-node admin suppressions` lists it, the node no longer announces
it (`/v2/nodes?community=KEY` on its registry, within five minutes), and a carry request for it is
answered `refused`. If a registry lists the community, ask its operator to withhold the listing
(`registry/README.md`), so other carriers stop being pointed at it.

Backups: deletion runs inside SQLite with `secure_delete` on and the write-ahead log is flushed
afterwards, but a backup, volume snapshot or copy taken before still holds the item. Delete or
expire those as your retention rules say; a store restored from an older backup comes back without
the suppression, so suppress again before starting it. A store that holds suppressions refuses to
open in a hoi-node older than this one rather than fetch it all again.

## Remote browsers and explicit relays

With a TLS proxy for `home.example` forwarding WebSocket upgrades to `127.0.0.1:7842`:

```sh
cargo run --release -p hoi-node -- \
  --data ~/hoi-node \
  --listen 127.0.0.1:7842 \
  --public-url wss://home.example/ \
  --peer-bind 0.0.0.0:7843 \
  --relay https://relay.example/
```

Both domains are examples. Supply a real home-node hostname with a valid TLS certificate
and a compatible iroh relay operated by you or your community. `--public-url` only changes
the advertised pairing address; it does not create the proxy, certificate or DNS record.
`--relay` configures a connection to a packet relay; `--packet-relay` (below) runs one.

## Run a packet relay (optional)

Two nodes that can't reach each other directly, both behind routers that won't forward, still
connect through a packet relay: it forwards encrypted QUIC packets by endpoint key and can't read
them. `--packet-relay` runs iroh's own relay server beside the node. Behind Caddy, which
holds the certificate:

```sh
hoi-node --data ~/hoi-public --public --packet-relay 127.0.0.1:3340 --packet-relay-url https://relay.example/
```

```caddyfile
relay.example {
	reverse_proxy 127.0.0.1:3340
}
```

Or with a certificate of its own: `--packet-relay 0.0.0.0:443 --packet-relay-cert fullchain.pem
--packet-relay-key privkey.pem` (plain HTTP then answers on the port below, 442 here, for
captive-portal checks). The URL goes into this node's routes, so invites name the relay. Each
client may send 1 MiB a second (4 MiB bursts). Anyone may use it except the endpoints on its
block list:

```sh
hoi-node admin --data ~/hoi-public relay block 4f6a…   # 64 hex, an endpoint key
hoi-node admin --data ~/hoi-public relay list
```

A running relay reads the list again within half a minute. `--packet-relay-members` admits only
endpoints this node knows as members of the communities it carries. The relay keeps nothing but
the table of who is connected now and writes no logs; it sees endpoint keys, addresses, timing
and sizes.

For a container, publish the native UDP port separately from the browser WebSocket port:

```sh
docker build -f core/crates/hoi-node/Dockerfile -t hoi-node .
docker run -d --name hoi-node -v hoi-node:/data \
  -p 127.0.0.1:7842:7842 -p 7843:7843/udp -p 7844:7844/udp \
  hoi-node --listen 0.0.0.0:7842 --public-url wss://home.example/ --relay https://relay.example/
docker exec hoi-node cat /data/pairing-link
```

The container binds iroh to UDP 7843. Docker address translation may make its advertised
direct IP hints unreachable from another network; use an explicit relay and verify the
resulting connection, or configure networking so the advertised address is reachable.

The image's default is safe: without `--listen`, the paired-device listener stays on the
container's own loopback (`127.0.0.1:7842`), which nothing outside the container reaches, and
only UDP 7843 and 7844 are exposed. A loopback address inside a container cannot be published
with `-p`, so to pair from the host you pass `--listen 0.0.0.0:7842` (the container's own
interfaces) and publish that port on the host's loopback only, as above, for the TLS proxy.
Never publish 7842 on every host interface (`-p 7842:7842` or `-P`): Docker's published ports
bypass most host firewalls. With `--network host` the container shares the host's interfaces,
so keep the default loopback listener there. A public node (`--public`, as in
`deploy/server/compose.yml`) has no paired-device listener at all.

**Reaching it from other devices.** Browsers and phones reach it over WebRTC on UDP 7844 with nothing else to set up: on the same network directly, across the internet when the node has a public address (a VPS) or a forwarded port you advertise with `--rtc-advertise`. In a container, publish the UDP port (`-p 7844:7844/udp`) and advertise the host's address. A `wss://` TLS proxy in front of the WebSocket listener still works as well. Keep the pairing token secret, including in terminal output and service journals.

**Public peer only.** `--peer-only` disables the paired-device control listener and pairing links while retaining native/WebRTC peer transport; `--public` is that, with relaying and carry requests on. Neither makes a person's key. No website or account signup is served. Public peers still persist and serve public records, including media; peer-only does not mean storage-free.

**Logs.** hoi-node prints its own status, and the pairing link only when started with `--show-pairing` or `--new-pairing` (otherwise it says the link is in `DIR/pairing-link`). It never logs peers, addresses or records, and `hoi-node admin` prints counts and the ids you gave it, never a record's contents (CONTRIBUTING.md).

See the [home-node](../../../deploy/home/README.md) and [server-node](../../../deploy/server/README.md) guides for setup and reachability. The project supplies no default relay, registry or bootstrap node, and this binary names none. A home node can read the community records it stores; it is not an opaque packet relay.

Also here: `Dockerfile` and `hoi-node.service` (systemd, sandboxed, `DynamicUser`).
[`deploy/install`](../../../deploy/install/README.md) writes that unit (a launchd agent on macOS, a
scheduled task on Windows) for every role, a home node, a public relay, a public node or a registry,
from a release, a release folder or this checkout; `deploy/home/install.sh` is that installer set to
a public relay.
