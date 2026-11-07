// What the playground plays: each way of joining, step by step, with the frames that cross the
// wire. Names, fields, caps and timings are the code's (each step lists its sources); example
// values are made up but shaped exactly like the real thing. The network around it is simulated.

export type Fam = 'hello' | 'disc' | 'sync' | 'pair' | 'carry' | 'rtc' | 'priv' | 'local';
export type Pipe = 'rtc' | 'quic' | 'ws' | 'udp' | 'https' | 'oob' | 'direct';

export interface Msg {
  from: string;
  to: string;
  name: string;
  fam: Fam;
  /** How many dots travel (bulk transfers send a few). */
  n?: number;
}
export interface Step {
  title: string;
  /** The transport and framing, or "on the device". */
  via: string;
  text: string;
  /** The exact shape of what is sent or stored. */
  data: string;
  /** Size and framing note. */
  bytes?: string;
  caps?: string[];
  src: string[];
  msgs: Msg[];
  /** A pipe this step opens: [a, b, kind]. */
  open?: [string, string, Pipe];
  /** Something happening on one device only. */
  local?: string;
  /** Counters after this step (absolute values). */
  set?: Partial<Counters>;
}
export interface Counters { links: number; known: number; candidates: number; listings: number; records: number; communities: number }
export interface Scenario {
  id: string;
  device: 'browser' | 'phone' | 'desktop' | 'node';
  title: string;
  short: string;
  /** Where "you" stands in the world. */
  you: string;
  intro: string;
  steps: Step[];
}

const NODE = '8e486e8fb79499dba2a789c6b1639d409495233c79e0eaf52e2ea1fc9dbbb73f';
const FRAME = '4-byte big-endian length + one CBOR message (≤ 16 MiB + 64 KiB)';

export const SCENARIOS: Scenario[] = [
  /* ------------------------------------------------------------------ (a) */
  {
    id: 'node-link',
    device: 'browser',
    title: 'Open a node link',
    short: 'node link',
    you: 'you-browser',
    intro: 'A browser can\'t accept connections and names no node of its own. A node link someone shared gives it a first public node to dial over WebRTC; that node tells it about communities and other public nodes.',
    steps: [
      {
        title: 'Read the link, offline', via: 'on the device · nothing sent', local: 'you',
        text: 'The link carries a checksummed, masked, deflated CBOR map: up to four WebRTC routes, up to four native routes, and the operator\'s signed notice. Masking means no address or token is readable in the link itself.',
        data: `hoi://node?code=<base64url>
code = "HON\\x01" ‖ checksum[16] ‖ masked(deflate(CBOR {
  v: 1,
  w: ["rtc:<fp 43>@203.0.113.9:7844"],      // ≤ 4 WebRTC routes
  p: ["${NODE.slice(0, 12)}…@203.0.113.9:7843"], // ≤ 4 native routes
  n: <operator notice, canonical bytes>,
  s: <Ed25519 signature, 64 B>
}))`,
        bytes: 'code ≤ 16 KiB',
        src: ['core/crates/hoi-types/src/uri.rs:28-37', 'core/crates/hoi-types/src/uri.rs:426-576'],
        msgs: [],
      },
      {
        title: 'Check the operator\'s notice', via: 'on the device · nothing sent', local: 'you',
        text: 'The notice says who runs the node and what it keeps. Its signature must verify against the first native route\'s endpoint key. You read it and accept it before anything connects.',
        data: `signed = "hoi node notice v1\\n<endpoint hex>\\n<notice hash hex>"

NodeLinkPreview {
  host: "203.0.113.9:7844", scope: "internet",
  endpoint: "${NODE.slice(0, 8)}…${NODE.slice(-4)}",
  notice: { … }, verified: true
}`,
        caps: ['notice ≤ 1536 B canonical, ≤ 2048 B on the wire', 'notices kept per device: 64'],
        src: ['core/crates/hoi-types/src/notice.rs:414-418', 'core/crates/hoi-core/src/notice.rs:207-283', 'core/crates/hoi-core/src/dto.rs:1305-1319'],
        msgs: [],
      },
      {
        title: 'Remember the node', via: 'on the device · nothing sent', local: 'you',
        text: 'The node goes to the front of the known-node list with seen = 0, its routes trimmed to two WebRTC and two native ones.',
        data: `KnownNode { routes: ["rtc:…@203.0.113.9:7844", "${NODE.slice(0, 8)}…@203.0.113.9:7843"], seen: 0 }

DirectoryStatus { known: 0, waiting: 1, connected: 0, listings: 0 }`,
        caps: ['known nodes ≤ 32, forgotten after 30 days unanswered', 'routes per node: 2 rtc + 2 native'],
        src: ['core/crates/hoi-core/src/nodes.rs:44-45', 'core/crates/hoi-core/src/nodes.rs:464-485'],
        msgs: [],
        set: { known: 0, candidates: 1 },
      },
      {
        title: 'Dial over WebRTC', via: 'WebRTC · UDP 7844 · ICE-lite · DTLS · SCTP',
        text: 'Every 15 s (3 s after start) the worker races up to 3 candidates and keeps the first that opens. The node answers a stateless STUN binding probe, then a session. The browser writes the node\'s answer SDP itself: the route already holds its certificate fingerprint.',
        data: `a=ice-lite
a=setup:passive
a=sctp-port:5000
a=max-message-size:262144
ice-ufrag = ice-pwd = "h1" + "p" + <browser cert SHA-256, 43> + <10 random>
data channel: negotiated id 0, label "hoi", ordered`,
        caps: ['open timeout 12 s (browser), 15 s (node)', 'node: 32 sessions, 8 opening, 10 new per source per minute'],
        src: ['apps/web/src/lib/core/rtc-link.ts:36', 'apps/web/src/lib/core/rtc-link.ts:78-115', 'apps/web/src/lib/core/core.worker.ts:672-736', 'core/crates/hoi-net/src/rtc.rs:12-70'],
        open: ['you', 'p0', 'rtc'],
        msgs: [{ from: 'you', to: 'p0', name: 'STUN', fam: 'rtc' }, { from: 'p0', to: 'you', name: 'STUN ok', fam: 'rtc' }, { from: 'you', to: 'p0', name: 'DTLS', fam: 'rtc' }, { from: 'p0', to: 'you', name: 'DTLS', fam: 'rtc' }],
        set: { links: 1 },
      },
      {
        title: 'The node speaks first: hello', via: `WebRTC channel "hoi" · ${FRAME}`,
        text: 'A headless node has no consent gate, so it says hello at once, with its notice hash and the notice itself. The browser sends nothing until it has checked that hello.',
        data: `{ "t": "hello", "v": 2,
  "node": "${NODE.slice(0, 16)}…",
  "c": [], "n": "5a5a…5a", "ch": [],
  "fx": ["link","catalog","profile","profile-color","nodes","directory",
         "range","keeps","carry","find","dht","blobs","signal"],
  "nh": "c706162e9554880d0793370fe83f2c36…",
  "nt": <381 B canonical notice>,
  "routes": ["${NODE.slice(0, 12)}…@203.0.113.9:7843"],
  "serves": false }`,
        caps: ['first hello within 15 s', 'c, ch ≤ 1024 each · routes ≤ 8 · fx ≤ 16'],
        src: ['core/crates/hoi-sync/src/lib.rs:686-972', 'core/crates/hoi-core/src/net.rs:747-786', 'core/vectors/node-notice.json'],
        msgs: [{ from: 'p0', to: 'you', name: 'hello', fam: 'hello' }],
      },
      {
        title: 'bind: this channel is the node\'s', via: `WebRTC channel "hoi" · ${FRAME}`,
        text: 'On a link the browser dialed, only hello, bind and bye pass until bind verifies: the node signs both certificate fingerprints, the ICE username and its endpoint, so the channel provably reaches the key the link named.',
        data: `{ "t": "bind", "s": <Ed25519 signature, 64 B> }

signed = "hoi/rtc-binding/1\\n<node fp hex>\\n<browser fp hex>\\n<ufrag>\\n<endpoint hex>"`,
        src: ['core/crates/hoi-core/src/bind.rs:32-60', 'core/crates/hoi-core/src/net.rs:1117-1120'],
        msgs: [{ from: 'p0', to: 'you', name: 'bind', fam: 'hello' }],
      },
      {
        title: 'Consent checked; your hello', via: `WebRTC channel "hoi" · ${FRAME}`,
        text: 'The notice hash matches the one you accepted, so the browser answers. To a stranger it sends 32 fresh random bytes as its node key, names no community in the clear, and salts the hashes of the public ones it has.',
        data: `{ "t": "hello", "v": 2,
  "node": <32 random bytes, new per connection>,
  "c": [], "serves": false,
  "n": "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a",          // salt
  "ch": ["1995a3150a7c89c8abc4c04f18d4267f…"],      // derive_key("hoi 2026 hello community v1", salt ‖ key)
  "fx": ["link","catalog","profile","profile-color","nodes","directory","range","unlisted"] }`,
        src: ['core/crates/hoi-core/src/notice.rs:164-206', 'core/crates/hoi-core/src/net.rs:618-639', 'core/vectors/hello-salted.json'],
        msgs: [{ from: 'you', to: 'p0', name: 'hello', fam: 'hello' }],
      },
      {
        title: 'The node\'s catalog', via: `WebRTC channel "hoi" · ${FRAME}`,
        text: 'A public node lists the open communities it carries, then fills the window with listings relayed from other nodes. Each listing carries the community\'s signed genesis and its routes.',
        data: `{ "t": "catalog", "entries": [
  { "g": <signed genesis ≤ 64 KiB>,
    "p": ["rtc:aaaa…@203.0.113.9:7844", "${NODE.slice(0, 8)}…@203.0.113.9:7843"],
    "r": [], "w": 2 },                      // warmth hint, own entries
  { "g": <genesis>, "p": ["rtc:dddd…@198.51.100.4:7844"],
    "r": [], "x": 1, "a": 5400 }             // relayed, 90 minutes old
] }`,
        caps: ['≤ 64 entries, ≤ 256 KiB', 'own entries ≤ 48 of 64 to a relaying peer', 'routes ≤ 16 × 128 B, relays ≤ 8 × 512 B'],
        src: ['core/crates/hoi-core/src/directory.rs:47-49', 'core/crates/hoi-core/src/directory.rs:275-314', 'core/crates/hoi-sync/src/lib.rs:337-355'],
        msgs: [{ from: 'p0', to: 'you', name: 'catalog', fam: 'disc', n: 3 }],
      },
      {
        title: 'The node\'s public nodes', via: `WebRTC channel "hoi" · ${FRAME}`,
        text: 'A public node passes on the public nodes it reached itself, never ones it was only told about, with vouches: uptime it saw and for how many days.',
        data: `{ "t": "nodes",
  "routes": ["rtc:cccc…@203.0.113.2:7844", "5f3c…e2a1@203.0.113.2:7843", …],
  "h": [{ "e": <endpoint 32>, "u": 96, "d": 12,
          "r": ["5f3c…@203.0.113.2:7843"] }] }`,
        caps: ['≤ 32 routes per message, ≤ 32 vouches', 'taken: 16 routes per list; candidates ≤ 32 for 1 hour', '6 nodes messages per peer per minute'],
        src: ['core/crates/hoi-core/src/nodes.rs:178-350', 'core/crates/hoi-sync/src/lib.rs:129-262'],
        msgs: [{ from: 'p0', to: 'you', name: 'nodes', fam: 'disc' }],
        set: { known: 1, candidates: 16 },
      },
      {
        title: 'Verify and keep the listings', via: 'on the device · nothing sent', local: 'you',
        text: 'Each genesis is decoded and checked (open admission, stamp floor, signature, content, time) before it is listed. The browser can now show communities to join, and knows 16 more public nodes to try.',
        data: `DirectoryStatus { known: 1, waiting: 16, connected: 1, listings: 38 }

DiscoveredCommunity { id, name: "c/privacy", color: "mint",
  description, invite, joined: false, live: 4, seen: 1793000000 }`,
        caps: ['listings ≤ 512 in a browser (4096 native)', 'directory written ≤ 30 s after new listings, else every 10 min'],
        src: ['core/crates/hoi-core/src/directory.rs:35-36', 'core/crates/hoi-core/src/directory.rs:435-599', 'core/crates/hoi-core/src/dto.rs:244-255'],
        msgs: [],
        set: { listings: 38 },
      },
    ],
  },

  /* ------------------------------------------------------------------ (b) */
  {
    id: 'invite',
    device: 'browser',
    title: 'Open an invite link',
    short: 'invite link',
    you: 'you-browser',
    intro: 'An invite names one community and the routes to reach it: members\' nodes and carriers. The browser joins, dials up to two of them over WebRTC and catches up by comparing ranges.',
    steps: [
      {
        title: 'Read the invite', via: 'on the device · nothing sent', local: 'you',
        text: 'Checksummed and masked CBOR, not deflated. It can carry the community\'s genesis, so the app can show the community before connecting.',
        data: `hoi://join?code=<base64url>
code = "HOI\\x02" ‖ checksum[16] ‖ masked(CBOR {
  v: 1, c: <community key 32>,
  r: ["https://relay.example"],              // ≤ 8 relays
  p: ["rtc:…@198.51.100.7:7844", …],          // ≤ 16 peers
  g: <signed genesis>, t: ["https://names.example"]  // ≤ 2 trackers
})`,
        bytes: 'code ≤ 128 KiB',
        src: ['core/crates/hoi-types/src/uri.rs:223-324', 'core/crates/hoi-core/src/api.rs:1043-1113'],
        msgs: [],
      },
      {
        title: 'Network & Safety, once', via: 'on the device · nothing sent', local: 'you',
        text: 'Before the first connection the app explains what connecting reveals (your IP address to the nodes you reach) and asks you to accept it once.',
        data: `routes/NetworkSafety.svelte → accepted once per device`,
        src: ['apps/web/src/routes/NetworkSafety.svelte'],
        msgs: [],
      },
      {
        title: 'Join', via: 'on the device · nothing sent', local: 'you',
        text: 'The genesis is ingested and the community joined; the invite\'s peers, relays and trackers are remembered for it.',
        data: `state.joined[c]    = { at: 1793000000, pinning: false }
state.bootstrap[c] = { peers: ≤ 16, relays: ≤ 8 }
trackers           ≤ 2`,
        src: ['core/crates/hoi-core/src/api.rs:1115-1138', 'core/crates/hoi-core/src/net.rs:384-417'],
        msgs: [],
        set: { communities: 1, candidates: 3 },
      },
      {
        title: 'Dial members over WebRTC', via: 'WebRTC · UDP 7844 · mode p',
        text: 'Every 15 s the worker picks up to 8 candidates (invite routes, trial routes, members it learned) and keeps up to 2 member links open. A failed node waits 60 s, doubling up to 30 minutes.',
        data: `rtc_candidates(): mailbox routes, accepted invite routes, trial routes,
                  learned members (newest first) · ≤ 8 · never the home node
MAX_MEMBER_LINKS = 2`,
        caps: ['retry 60 s → 30 min', 'a link closed within 60 s counts as a refusal'],
        src: ['core/crates/hoi-core/src/net.rs:231-262', 'apps/web/src/lib/core/core.worker.ts:617-670'],
        open: ['you', 'm0', 'rtc'],
        msgs: [{ from: 'you', to: 'm0', name: 'STUN', fam: 'rtc' }, { from: 'm0', to: 'you', name: 'DTLS', fam: 'rtc' }],
        set: { links: 1 },
      },
      {
        title: 'hello, naming the community', via: `WebRTC channel "hoi" · ${FRAME}`,
        text: 'The node speaks first and binds the channel. Because the invite\'s routes led here, the browser names this community in the clear; everything else stays salted.',
        data: `{ "t": "hello", "v": 2, "node": <random 32>,
  "c": ["8a88…5c"],                // routed here: named in the clear
  "n": <salt 16>, "ch": [ … ],      // the rest, salted
  "fx": ["link","catalog","profile","profile-color","nodes","directory","range","unlisted"] }`,
        src: ['core/crates/hoi-core/src/net.rs:480-492', 'core/crates/hoi-core/src/net.rs:1121-1273'],
        msgs: [{ from: 'm0', to: 'you', name: 'hello', fam: 'hello' }, { from: 'm0', to: 'you', name: 'bind', fam: 'hello' }, { from: 'you', to: 'm0', name: 'hello', fam: 'hello' }],
      },
      {
        title: 'Compare ranges', via: `WebRTC channel "hoi" · ${FRAME}`,
        text: 'Both sides start range reconciliation over (created_at, id) for the shared community. Fingerprints of ranges that match end there; ranges that differ split until the ids that differ are listed.',
        data: `{ "t": "rng", "c": <community 32>, "n": 42, "s": true,
  "r": [[0, "", 1, <fingerprint 16>]] }
{ "t": "rng", "c": <community 32>, "n": 3, "s": false, "r": [ … ] }`,
        caps: ['range: list ids under 64 · 16 branches · ≤ 8192 ranges', 'range session ends after 120 s'],
        src: ['core/crates/hoi-core/src/net.rs:1551-1592', 'core/crates/hoi-sync/src/range.rs:22-27'],
        msgs: [{ from: 'you', to: 'm0', name: 'rng', fam: 'sync' }, { from: 'm0', to: 'you', name: 'rng', fam: 'sync' }, { from: 'you', to: 'm0', name: 'rng', fam: 'sync' }],
      },
      {
        title: 'Ask for what\'s missing', via: `WebRTC channel "hoi" · ${FRAME}`,
        text: 'What the browser lacks it asks for by id, a page at a time, one page in flight per link across all communities.',
        data: `{ "t": "want", "ids": [<id 32>, <id 32>, …] }   // ≤ 200 per page`,
        caps: ['ids per want ≤ 5000 (pages of 200)', 'stalled pages reconnect after 120 s'],
        src: ['core/crates/hoi-core/src/net.rs:1513-1549', 'core/crates/hoi-sync/src/lib.rs:47-52'],
        msgs: [{ from: 'you', to: 'm0', name: 'want', fam: 'sync' }],
      },
      {
        title: 'Records arrive', via: `WebRTC channel "hoi" · ${FRAME}`,
        text: 'Records come back as their signed wire bytes. Each passes the full validation pipeline; geneses, identities and delegations first.',
        data: `{ "t": "recs", "recs": [<record wire>, <record wire>, …] }
// sent ≤ 200 or about 1 MiB · accepted ≤ 500`,
        caps: ['records per recs: 200 sent / 500 accepted'],
        src: ['core/crates/hoi-sync/src/lib.rs:1167-1185'],
        msgs: [{ from: 'm0', to: 'you', name: 'recs', fam: 'sync', n: 5 }],
        set: { records: 200 },
      },
      {
        title: 'A second member, more pages', via: `WebRTC channel "hoi" · ${FRAME}`,
        text: 'A second member link opens; the remaining pages come from both. A peer that delivered valid records is remembered as a member of the community.',
        data: `state.learned[c] += peer routes   // ≤ 16 per community, 30 days
state.synced[c], state.sync_completed[c] when the want pages run dry`,
        caps: ['learned member routes: 16 per community, 30 days'],
        src: ['core/crates/hoi-core/src/net.rs:301-358', 'core/crates/hoi-core/src/net.rs:1535-1543'],
        open: ['you', 'm1', 'rtc'],
        msgs: [{ from: 'you', to: 'm1', name: 'hello', fam: 'hello' }, { from: 'm1', to: 'you', name: 'recs', fam: 'sync', n: 4 }, { from: 'm0', to: 'you', name: 'recs', fam: 'sync', n: 3 }],
        set: { links: 2, records: 612 },
      },
      {
        title: 'Live', via: `WebRTC channel "hoi" · ${FRAME}`,
        text: 'From now on new records reach the browser in recs on the same links, as members post them.',
        data: `{ "t": "recs", "recs": [<new post>] }`,
        src: ['core/crates/hoi-sync/src/lib.rs:12'],
        msgs: [{ from: 'm1', to: 'you', name: 'recs', fam: 'sync' }, { from: 'm0', to: 'you', name: 'recs', fam: 'sync' }],
        set: { records: 614 },
      },
    ],
  },

  /* ------------------------------------------------------------------ (c) */
  {
    id: 'home-join',
    device: 'node',
    title: 'A home node joins',
    short: 'invite on a node',
    you: 'you-home',
    intro: 'A desktop or hoi-node speaks iroh QUIC: it can dial and be dialed, carry communities for others and find public nodes by itself.',
    steps: [
      {
        title: 'Join from the invite', via: 'on the device · nothing sent', local: 'you',
        text: 'Same as in a browser, but a native node needs no identity to join: it carries, it doesn\'t post.',
        data: `join(invite) → state.joined[c], state.bootstrap[c]`,
        src: ['core/crates/hoi-core/src/api.rs:1115-1138'],
        msgs: [],
        set: { communities: 1, candidates: 4 },
      },
      {
        title: 'Dial over QUIC', via: 'iroh QUIC · ALPN hoi/sync/1 · one bidirectional stream',
        text: 'Every 5 s the dial loop takes native routes from invites, trial routes, learned members and (while public) up to 3 public nodes. Relays from the invite are added. A failed endpoint rests 600 s.',
        data: `ep.connect(addr, "hoi/sync/1")   // 20 s timeout
open one bidi stream within 15 s · 4 MiB windows · 0 uni streams
retry per endpoint: 2, 4, 8, 16, 32, 60 s`,
        caps: ['32 iroh peers in + out', 'new incoming per source: 10 per minute'],
        src: ['core/crates/hoi-net/src/p2p.rs:24-35', 'core/crates/hoi-net/src/p2p.rs:270-399', 'core/crates/hoi-core/src/net.rs:180-229'],
        open: ['you', 'm2', 'quic'],
        msgs: [{ from: 'you', to: 'm2', name: 'QUIC', fam: 'rtc' }, { from: 'm2', to: 'you', name: 'QUIC', fam: 'rtc' }],
        set: { links: 1 },
      },
      {
        title: 'Hellos both ways', via: `QUIC stream · ${FRAME}`,
        text: 'Both sides send hello. The node names in the clear the communities whose routes lead to the other; a public node lists every feature it offers.',
        data: `"fx": ["link","catalog","profile","profile-color","nodes","directory","range",
       "keeps","carry","find","dht","blobs","signal"]`,
        src: ['core/crates/hoi-core/src/net.rs:747-786'],
        msgs: [{ from: 'you', to: 'm2', name: 'hello', fam: 'hello' }, { from: 'm2', to: 'you', name: 'hello', fam: 'hello' }],
      },
      {
        title: 'Catch up', via: `QUIC stream · ${FRAME}`,
        text: 'rng, want and recs as in a browser, for every shared community.',
        data: `rng → rng → want{ids ≤ 200} → recs{≤ 200 / ~1 MiB}`,
        src: ['core/crates/hoi-core/src/net.rs:1513-1592'],
        msgs: [{ from: 'you', to: 'm2', name: 'rng', fam: 'sync' }, { from: 'm2', to: 'you', name: 'rng', fam: 'sync' }, { from: 'you', to: 'm2', name: 'want', fam: 'sync' }, { from: 'm2', to: 'you', name: 'recs', fam: 'sync', n: 5 }],
        set: { records: 400 },
      },
      {
        title: 'Catalogs and nodes', via: `QUIC stream · ${FRAME}`,
        text: 'Catalogs go both ways. A public peer passes on its public nodes; ones the node dialed itself are remembered as public nodes, and their lists become candidates.',
        data: `{ "t": "catalog", "entries": [ … ≤ 64 ] }
{ "t": "nodes", "routes": [ … ≤ 32 ], "h": [ … ] }`,
        caps: ['candidates ≤ 32, 1 hour', 'directory ≤ 4096 listings on a native node'],
        src: ['core/crates/hoi-core/src/nodes.rs:234-331', 'core/crates/hoi-core/src/directory.rs:36'],
        msgs: [{ from: 'm2', to: 'you', name: 'catalog', fam: 'disc', n: 2 }, { from: 'you', to: 'm2', name: 'catalog', fam: 'disc' }, { from: 'm2', to: 'you', name: 'nodes', fam: 'disc' }],
        set: { known: 1, candidates: 16, listings: 212 },
      },
      {
        title: 'Dial three public nodes', via: 'iroh QUIC · ALPN hoi/sync/1',
        text: 'A native public node keeps up to 3 other public nodes dialed (DIAL_NATIVE). Each one that answers with its own nodes list is kept as a known public node.',
        data: `DIAL_NATIVE = 3
note_public_node(P) → state.nodes (≤ 32, 30 days)`,
        src: ['core/crates/hoi-core/src/nodes.rs:52', 'core/crates/hoi-core/src/nodes.rs:413-424'],
        open: ['you', 'p1', 'quic'],
        msgs: [{ from: 'you', to: 'p1', name: 'hello', fam: 'hello' }, { from: 'p1', to: 'you', name: 'nodes', fam: 'disc' }, { from: 'p1', to: 'you', name: 'catalog', fam: 'disc' }],
        set: { links: 2, known: 2 },
      },
      {
        title: 'Ask a carrier to hold it', via: `QUIC stream · ${FRAME}`,
        text: 'A founder\'s device asks public carriers to hold its open community. A carrier says yes or why not.',
        data: `{ "t": "carry", "g": <signed genesis> }
{ "t": "carried", "c": <community 32>, "ok": true }
{ "t": "carried", "c": <community 32>, "ok": false, "reason": "full" }
// reasons: full · busy · refused · not open · not accepting · invalid`,
        caps: ['≤ 3 per hello, ≤ 4 per minute per peer', 'accept 8 per minute, 128 per day; 256 communities, 256 MiB', 'idle carried community dropped after 14 days'],
        src: ['core/crates/hoi-core/src/carry.rs:61-80', 'core/crates/hoi-core/src/carry.rs:461-555'],
        open: ['you', 'p2', 'quic'],
        msgs: [{ from: 'you', to: 'p2', name: 'carry', fam: 'carry' }, { from: 'p2', to: 'you', name: 'carried', fam: 'carry' }],
        set: { links: 3, known: 3 },
      },
      {
        title: 'find what nobody connected serves', via: `QUIC stream · ${FRAME}`,
        text: 'For a community no connected peer serves, the node asks up to 3 nodes that said find, at most every 10 minutes per community.',
        data: `{ "t": "find",  "c": <community 32> }
{ "t": "found", "c": <community 32>,
  "r": ["${NODE.slice(0, 8)}…@203.0.113.9:7843", "rtc:aaaa…@203.0.113.9:7844"] }`,
        caps: ['found routes ≤ 16', 'finds answered ≤ 10 per peer per minute'],
        src: ['core/crates/hoi-core/src/find.rs:29-34', 'core/crates/hoi-core/src/find.rs:185-224'],
        msgs: [{ from: 'you', to: 'p1', name: 'find', fam: 'disc' }, { from: 'p1', to: 'you', name: 'found', fam: 'disc' }],
      },
    ],
  },

  /* ------------------------------------------------------------------ (d) */
  {
    id: 'public-node',
    device: 'node',
    title: 'Start a public node',
    short: 'new public node',
    you: 'you-public',
    intro: 'hoi-node --public starts with no registry, no relay and no node configured. Given one node link it finds the rest: nodes lists, a DHT among public nodes, and whoever dials in.',
    steps: [
      {
        title: 'Start', via: 'on the device · nothing sent', local: 'you',
        text: 'Relaying on, carry policy "any", the operator notice loaded. Its node link is written to DIR/node-link and refreshed when its addresses change.',
        data: `hoi-node --public --node <link> --notice notice.txt

ports: UDP auto  iroh QUIC (hoi/sync/1)
       UDP 7844  WebRTC for browsers and phones
       UDP 7846  Mainline DHT · UDP 7847 DHT answers
       UDP 7848  LAN beacons (239.255.70.74)`,
        src: ['core/crates/hoi-node/src/main.rs:166-202', 'core/crates/hoi-node/src/main.rs:553-619', 'core/crates/hoi-node/src/main.rs:812-879'],
        msgs: [],
      },
      {
        title: 'Dial the node link', via: 'iroh QUIC · ALPN hoi/sync/1',
        text: 'The link\'s native route is dialed. The other node\'s hello carries its notice; a headless node has no consent gate.',
        data: `{ "t": "hello", "v": 2, "node": <endpoint>, "nh": …, "nt": …,
  "fx": [ …, "carry", "find", "dht", "signal" ] }`,
        src: ['core/crates/hoi-node/src/main.rs:608-619'],
        open: ['you', 'p0', 'quic'],
        msgs: [{ from: 'you', to: 'p0', name: 'hello', fam: 'hello' }, { from: 'p0', to: 'you', name: 'hello', fam: 'hello' }],
        set: { links: 1 },
      },
      {
        title: 'nodes, then three more', via: `QUIC stream · ${FRAME}`,
        text: 'The public node\'s list becomes candidates; the new node dials up to three at a time and keeps those that answer as public nodes.',
        data: `{ "t": "nodes", "routes": [ … ], "h": [ … ] }`,
        src: ['core/crates/hoi-core/src/nodes.rs:291-350'],
        msgs: [{ from: 'p0', to: 'you', name: 'nodes', fam: 'disc' }],
        set: { known: 1, candidates: 16 },
      },
      {
        title: 'Meet the DHT', via: `QUIC stream · ${FRAME}`,
        text: 'Peers that said dht swap contact addresses. The DHT is built with no outside bootstrap: it starts from these contacts, so it only ever contains public nodes.',
        data: `{ "t": "dht", "a": ["203.0.113.9:7846"] }   // ≤ 4 contacts, ≤ 64 B each`,
        caps: ['contact peers kept: 64', 'empty table rebuilt from contacts at most every 60 s'],
        src: ['core/crates/hoi-core/src/find.rs:150-183', 'core/crates/hoi-net/src/dht.rs:117-197'],
        open: ['you', 'p1', 'quic'],
        msgs: [{ from: 'you', to: 'p1', name: 'dht', fam: 'disc' }, { from: 'p1', to: 'you', name: 'dht', fam: 'disc' }],
        set: { links: 2, known: 2 },
      },
      {
        title: 'Announce on the DHT', via: 'UDP 7846 · Mainline DHT (BEP 5)',
        text: 'Each open community it carries (up to 64) is announced under the first 20 bytes of BLAKE3("hoi dht node v1" ‖ key), with the answer port (the code comments say "hoi dht v1"; the constant is DHT_DOMAIN), every 15 minutes or when the list changes.',
        data: `info_hash = BLAKE3(DHT_DOMAIN ‖ community key)[..20]   // DHT_DOMAIN = "hoi dht node v1"
announce_peer(info_hash, port = 7847)   // ≤ 20 s each`,
        caps: ['64 communities announced', 'tick 10 s, announce every 15 min'],
        src: ['core/crates/hoi-net/src/dht.rs:48-57', 'core/crates/hoi-core/src/find.rs:71-79'],
        open: ['you', 'dht', 'udp'],
        msgs: [{ from: 'you', to: 'dht', name: 'announce', fam: 'disc', n: 3 }],
      },
      {
        title: 'Answer a DHT lookup', via: 'UDP 7847 · hoi DHT answers',
        text: 'Someone looking for a community finds this node\'s address on the DHT and asks it directly. The query is padded to exactly 512 bytes; the signed answer fits in 1200.',
        data: `query  = "HOI?" ‖ 0x01 ‖ community[32] ‖ nonce[16] ‖ zeros   // 512 B
answer = "HOI!" ‖ 0x01 ‖ nonce[16] ‖ CBOR {
  c: <community 32>, e: <endpoint 32>, r: [routes ≤ 8],
  t: 1793000000, s: <sig 64> }                               // ≤ 1200 B
signed = "hoi dht node v1\\n<endpoint>\\n<community>\\n<t>\\n<routes>"`,
        caps: ['30 queries per source per minute, 4096 sources', 'answers valid 30 min (+10 min clock slack)'],
        src: ['core/crates/hoi-net/src/dht.rs:37-115', 'core/crates/hoi-core/src/find.rs:44-118'],
        msgs: [{ from: 'p3', to: 'you', name: 'HOI?', fam: 'disc' }, { from: 'you', to: 'p3', name: 'HOI!', fam: 'disc' }],
      },
      {
        title: 'Directory gossip', via: `QUIC stream · ${FRAME}`,
        text: 'Every 5 minutes it sends each peer a fresh catalog window (light peers without keeps every 30 minutes) and forgets listings nobody confirmed for 21 days.',
        data: `{ "t": "catalog", "entries": [ … ] }   // every 300 s`,
        caps: ['new listings per connection: 128 per hour', 'catalog messages per peer: 60 per minute, then bye'],
        src: ['core/crates/hoi-core/src/directory.rs:38-58', 'core/crates/hoi-core/src/directory.rs:406-449'],
        msgs: [{ from: 'you', to: 'p0', name: 'catalog', fam: 'disc' }, { from: 'you', to: 'p1', name: 'catalog', fam: 'disc' }, { from: 'p1', to: 'you', name: 'catalog', fam: 'disc' }],
        set: { listings: 340 },
      },
      {
        title: 'Strangers dial in', via: 'WebRTC · UDP 7844',
        text: 'Browsers that got its route from someone\'s node link or nodes list connect. A dial-in that sends nodes leaves only its own two routes on its connection address as candidates, unless it says unlisted.',
        data: `caller_routes: ≤ 2, only on the caller's connection address
refuse when not relaying: bye "this node isn't relaying right now"`,
        caps: ['32 WebRTC sessions, 32 iroh peers', 'relay daily cap 5 GB by default'],
        src: ['core/crates/hoi-core/src/nodes.rs:234-284', 'core/crates/hoi-core/src/net.rs:1092-1097'],
        open: ['b1', 'you', 'rtc'],
        msgs: [{ from: 'b1', to: 'you', name: 'STUN', fam: 'rtc' }, { from: 'you', to: 'b1', name: 'hello', fam: 'hello' }, { from: 'you', to: 'b1', name: 'catalog', fam: 'disc' }],
        set: { links: 3 },
      },
    ],
  },

  /* ------------------------------------------------------------------ (e) */
  {
    id: 'lan',
    device: 'desktop',
    title: 'Find a node on your LAN',
    short: 'same network',
    you: 'you-home',
    intro: 'Native devices beacon on the local network for the public communities they joined. The beacon is sealed per community, so only members can read where to connect.',
    steps: [
      {
        title: 'Listen on the LAN', via: 'UDP 7848 · multicast 239.255.70.74 · TTL 1', local: 'you',
        text: 'Desktops and nodes open a multicast socket that never leaves the local network. Browsers don\'t beacon. "Only through my VPN" turns it off.',
        data: `bind 0.0.0.0:7848 (SO_REUSEADDR/PORT)
join 239.255.70.74 · loop on · TTL 1`,
        src: ['core/crates/hoi-net/src/lan.rs:28-60', 'core/crates/hoi-core/src/lan.rs:26-29'],
        msgs: [],
      },
      {
        title: 'Beacon every 30 s', via: 'UDP multicast · 6 + 90 × n bytes',
        text: 'One entry per joined public community (up to 8): a tag that changes every 10 minutes and the endpoint and port sealed with a key only members can derive.',
        data: `"HOIL" 0x01 ‖ n ‖ n × (tag[16] ‖ sealed[74])
tag    = BLAKE3_keyed(derive_key("hoi 2026 lan tag v1", community),
                      u64be(now / 600))[..16]
sealed = seal("hoi 2026 lan route v1",
              derive_key("hoi 2026 lan route key v1", community),
              endpoint[32] ‖ u16be(port))`,
        bytes: 'one community: 96 B; at most 726 B',
        src: ['core/crates/hoi-core/src/lan.rs:8-85'],
        open: ['you', 'lan', 'udp'],
        msgs: [{ from: 'you', to: 'lan', name: 'HOIL', fam: 'disc' }, { from: 'lan', to: 'you', name: 'HOIL', fam: 'disc' }],
      },
      {
        title: 'Recognize a member', via: 'on the device · nothing sent', local: 'you',
        text: 'A tag for this or the previous 10-minute window matches; the sealed route opens. It becomes a trial route for that community.',
        data: `route = "<endpoint>@<source ip>:<port>"
try_routes(c, [route])   // trial routes ≤ 16 per community, memory only`,
        caps: ['≤ 120 beacons heard per minute'],
        src: ['core/crates/hoi-core/src/lan.rs:88-100', 'core/crates/hoi-core/src/net.rs:460-470'],
        msgs: [],
        set: { candidates: 1 },
      },
      {
        title: 'Connect and sync', via: 'iroh QUIC · ALPN hoi/sync/1 · direct on the LAN',
        text: 'The dial loop picks the trial route. Either side may dial. A desktop or node takes a stranger\'s incoming link only while its relay switch is on, which native nodes turn on the first time they start; turned off, only its own dial opens a link. Since it reached the peer, the community is named in the clear. Range sync runs; once records arrive the route is kept as a learned member.',
        data: `hello → rng → want → recs
remember_member: trial route → state.learned[c]`,
        src: ['core/crates/hoi-core/src/net.rs:327-333', 'core/crates/hoi-core/src/net.rs:472-478'],
        open: ['you', 'lan', 'quic'],
        msgs: [{ from: 'you', to: 'lan', name: 'hello', fam: 'hello' }, { from: 'lan', to: 'you', name: 'hello', fam: 'hello' }, { from: 'you', to: 'lan', name: 'rng', fam: 'sync' }, { from: 'lan', to: 'you', name: 'recs', fam: 'sync', n: 4 }],
        set: { links: 1, records: 180 },
      },
    ],
  },

  /* ------------------------------------------------------------------ (f) */
  {
    id: 'pair',
    device: 'phone',
    title: 'Pair a phone with your home node',
    short: 'pair a device',
    you: 'you-phone',
    intro: 'A home node serves its own devices. A pairing link hands a new device the address and a one-time token; the home node swaps it for a credential, and the root device approves the new device key.',
    steps: [
      {
        title: 'The home node offers a pairing link', via: 'on the home node', local: 'home',
        text: 'Serving creates a pairing token and a link key. The link may only point to wss://, or ws:// on loopback.',
        data: `hoi://pair?code=<base64url>
code = "HOP\\x01" ‖ checksum[16] ‖ masked(deflate(CBOR {
  v: 1, u: "ws://127.0.0.1:7842", t: <token, 18 random bytes>,
  w: ["rtc:…@192.168.1.20:7844"], n: "lena's laptop" }))   // ≤ 4096`,
        src: ['core/crates/hoi-core/src/net.rs:1740-1753', 'core/crates/hoi-types/src/uri.rs:326-424'],
        msgs: [],
      },
      {
        title: 'Open it on the phone', via: 'on the device · nothing sent', local: 'you',
        text: 'The app previews it, stores the home, and removes the token from the address bar.',
        data: `PairPreview { name, url, routes, pairedHere }
state.home = PairInfo { url, token, name, rtc, spent: None,
                        next: None, link_key: None, proven: false }`,
        src: ['core/crates/hoi-core/src/net.rs:1773-1795', 'apps/web/src/routes/Pair.svelte'],
        msgs: [],
      },
      {
        title: 'Connect to home', via: 'WebRTC · mode c (phones skip loopback ws://)',
        text: 'A desktop browser opens the WebSocket on 127.0.0.1:7842; a phone, which can\'t reach the laptop\'s loopback, uses the WebRTC routes. Backoff 1 s, doubling to 60 s.',
        data: `ws://127.0.0.1:7842       (5 s open timeout; Origin checked)
rtc:<fp>@192.168.1.20:7844 (mode "c": paired device)`,
        caps: ['32 paired devices served at once'],
        src: ['apps/web/src/lib/core/core.worker.ts:390-470', 'core/crates/hoi-net/src/lib.rs:49-94'],
        open: ['you', 'home', 'rtc'],
        msgs: [{ from: 'you', to: 'home', name: 'STUN', fam: 'rtc' }, { from: 'home', to: 'you', name: 'DTLS', fam: 'rtc' }],
        set: { links: 1 },
      },
      {
        title: 'hello with the pairing token', via: `${FRAME}`,
        text: 'The home node waits; the device speaks first with the token, all its public communities and hints.',
        data: `{ "t": "hello", "v": 2, "node": <device endpoint>,
  "token": "<pairing token>", "name": "lena's phone",
  "c": [ … ], "hints": [{ "c": …, "p": [ ≤ 16 ], "r": [ ≤ 8 ] }],
  "fx": [ … ] }`,
        src: ['core/crates/hoi-core/src/net.rs:927-995'],
        msgs: [{ from: 'you', to: 'home', name: 'hello+token', fam: 'pair' }],
      },
      {
        title: 'Admitted: a credential', via: `${FRAME}`,
        text: 'The token matches (constant-time). The home node issues a fresh credential, a new pairing token for the next device and the link key, joins the device\'s communities, and tells its other devices about the new pairing link.',
        data: `{ "t": "hello", "v": 2, "serves": true, "name": "lena's laptop",
  "token": "ZGV2aWNlIGNyZWRlbnRpYWwgZm9yIGEgdGVzdCB2ZWN0b3I",  // credential
  "pt":    "bmV4dCBwYWlyaW5nIGxpbmsh",                          // next pairing token
  "lk":    <link key 32>, "c": [ … ], "fx": ["link"] }`,
        caps: ['paired devices ≤ 32 (least recently seen evicted)', 'communities a device can make home join: 256'],
        src: ['core/crates/hoi-core/src/net.rs:796-858', 'core/vectors/hello-salted.json'],
        msgs: [{ from: 'home', to: 'you', name: 'hello+credential', fam: 'pair' }],
        set: { communities: 3 },
      },
      {
        title: 'Ask to link the device key', via: 'link · sealed with the link key',
        text: 'The phone asks to join your identity. The request is sealed with the link key; the home node passes it on, unread, to your other devices.',
        data: `{ "t": "link", "m": <mailbox 16>, "b": seal({ "t": "req", "code": "hoi:link:…",
  "c": derive_key("hoi 2026 link commit v1", nonce) }) }`,
        caps: ['≤ 12 link messages per minute per peer', 'sealed ≤ 512 KiB'],
        src: ['core/crates/hoi-core/src/link.rs:32-213', 'core/crates/hoi-core/src/link.rs:276-283'],
        open: ['home', 'root', 'ws'],
        msgs: [{ from: 'you', to: 'home', name: 'link req', fam: 'pair' }, { from: 'home', to: 'root', name: 'link req', fam: 'pair' }],
      },
      {
        title: 'Compare four digits', via: 'link · chk / rev',
        text: 'The root device challenges; the phone reveals its commitment; both show the same four digits. You compare them and say yes on the root device.',
        data: `root → phone: chk { device, n: ours[16] }
phone → root: rev { device, n: nonce[32], to: ours[16] }
digits = derive_key("hoi 2026 link check v1", device ‖ nonce ‖ ours)[..4] (LE u32) % 10000`,
        caps: ['≤ 4 asks, ≤ 3 challenges, 600 s'],
        src: ['core/crates/hoi-core/src/link.rs:85-93', 'core/crates/hoi-core/src/link.rs:349-439'],
        msgs: [{ from: 'root', to: 'home', name: 'chk', fam: 'pair' }, { from: 'home', to: 'you', name: 'chk', fam: 'pair' }, { from: 'you', to: 'home', name: 'rev', fam: 'pair' }, { from: 'home', to: 'root', name: 'rev', fam: 'pair' }],
      },
      {
        title: 'Approved', via: 'link · ok',
        text: 'The root signs the device into the identity and sends the approval with the geneses of your communities. The phone is linked.',
        data: `{ "t": "ok", "code": <approval: delegation + ≤ 64 geneses, ≤ 256 KiB> }`,
        src: ['core/crates/hoi-core/src/link.rs:243-256'],
        msgs: [{ from: 'root', to: 'home', name: 'ok', fam: 'pair' }, { from: 'home', to: 'you', name: 'ok', fam: 'pair' }],
      },
    ],
  },

  /* ------------------------------------------------------------------ (g) */
  {
    id: 'direct',
    device: 'browser',
    title: 'Browser to browser, directly',
    short: 'direct link',
    you: 'you-browser',
    intro: 'Two browsers with Direct links on, linked to the same node and sharing an open community, can be introduced: the node relays their WebRTC offer and answer, then steps aside.',
    steps: [
      {
        title: 'Both on one node', via: 'WebRTC · each browser to N',
        text: 'Each browser says direct in its hello; the node says signal (it runs a WebRTC listener and accepts members).',
        data: `browser fx: [ …, "direct" ]      node fx: [ …, "signal" ]`,
        src: ['core/crates/hoi-core/src/net.rs:747-786', 'core/crates/hoi-core/src/signal.rs:11-16'],
        open: ['you', 'p0', 'rtc'],
        msgs: [{ from: 'you', to: 'p0', name: 'hello', fam: 'hello' }, { from: 'b0', to: 'p0', name: 'hello', fam: 'hello' }],
        set: { links: 1 },
      },
      {
        title: 'here: who else is here', via: `WebRTC channel "hoi" · ${FRAME}`,
        text: 'The node gives every direct link a random 16-byte handle and tells each browser, per shared community, the others\' handles.',
        data: `{ "t": "here", "c": <community 32>, "y": <own handle 16>,
  "h": [<handle 16>, …] }    // ≤ 16, sorted`,
        src: ['core/crates/hoi-core/src/signal.rs:111-145'],
        msgs: [{ from: 'p0', to: 'you', name: 'here', fam: 'disc' }, { from: 'p0', to: 'b0', name: 'here', fam: 'disc' }],
      },
      {
        title: 'The lower handle offers', via: 'on the device', local: 'you',
        text: 'Of two handles the lower one makes the offer, using the node as its STUN server and gathering for up to 3 s.',
        data: `RTCPeerConnection({ iceServers: [{ urls: "stun:203.0.113.9:7844" }] })
offer = { "type": "offer", "sdp": "v=0…" }   // ≤ 8 KiB`,
        caps: ['≤ 4 direct links', 'same pair retried after 600 s'],
        src: ['apps/web/src/lib/core/rtc-direct.ts:13-79', 'core/crates/hoi-core/src/signal.rs:184-232'],
        msgs: [],
      },
      {
        title: 'sig through the node', via: `WebRTC channel "hoi" · ${FRAME}`,
        text: 'The node checks both said direct, share a public community and stay under 30 sig per minute, then passes the offer on. The answer comes back the same way.',
        data: `{ "t": "sig", "p": <their handle 16>,
  "d": <my key 16> ‖ {"type":"offer","sdp":"v=0…"} }`,
        caps: ['sig payload ≤ 8 KiB', '30 sig per minute per link'],
        src: ['core/crates/hoi-core/src/signal.rs:147-172', 'core/crates/hoi-core/src/signal.rs:303-318'],
        open: ['p0', 'b0', 'rtc'],
        msgs: [{ from: 'you', to: 'p0', name: 'sig offer', fam: 'rtc' }, { from: 'p0', to: 'b0', name: 'sig offer', fam: 'rtc' }, { from: 'b0', to: 'p0', name: 'sig answer', fam: 'rtc' }, { from: 'p0', to: 'you', name: 'sig answer', fam: 'rtc' }],
      },
      {
        title: 'The direct channel opens', via: 'WebRTC · browser ↔ browser',
        text: 'Within 15 s the channel opens. From here it is the ordinary protocol: hello with hashed communities, rng, recs. Never for private communities.',
        data: `net_connect("direct-<id>", Peer) → hello → rng → recs`,
        src: ['apps/web/src/lib/core/rtc-direct.ts:15', 'core/crates/hoi-core/src/signal.rs:288-299'],
        open: ['you', 'b0', 'direct'],
        msgs: [{ from: 'you', to: 'b0', name: 'hello', fam: 'hello' }, { from: 'b0', to: 'you', name: 'hello', fam: 'hello' }, { from: 'b0', to: 'you', name: 'recs', fam: 'sync', n: 3 }],
        set: { links: 2, records: 40 },
      },
    ],
  },

  /* ------------------------------------------------------------------ (h) */
  {
    id: 'private',
    device: 'browser',
    title: 'Join a private community',
    short: 'private invite',
    you: 'you-browser',
    intro: 'Private communities are MLS groups. You hand a join request to a member who can admit people; they commit you in and leave a sealed invitation for you to pick up.',
    steps: [
      {
        title: 'Make a join request', via: 'out of band · you hand it over', local: 'you',
        text: 'Your device makes an MLS key package and a one-time pickup key, and names up to 4 holders where the invitation can wait.',
        data: `hoi:private-request:<base64url CBOR JoinRequest {
  v: 1, package: <MLS KeyPackage>, records: [<identity wires>],
  pickup: { key: <X25519 public>, holders: [ ≤ 4 mailboxes ] } }>
// reused for 7 days`,
        src: ['core/crates/hoi-core/src/private.rs:1132-1164', 'core/crates/hoi-core/src/pickup.rs:37'],
        open: ['you', 'founder', 'oob'],
        msgs: [{ from: 'you', to: 'founder', name: 'request', fam: 'priv' }],
      },
      {
        title: 'A member commits you in', via: 'on the member\'s device', local: 'founder',
        text: 'The commit is sealed for the old epoch\'s room, so current members move on; the Welcome and a bootstrap (identity, genesis, role proof) are for the new epoch.',
        data: `private_invite(c, request):
  commit    → sealed for the old epoch's room
  Welcome   → for you
  Bootstrap → sealed for the new epoch's room
invitation = hoi:private:<{ v: 2, welcome, bootstrap }>`,
        caps: ['≤ 128 members'],
        src: ['core/crates/hoi-core/src/private.rs:1214-1288'],
        msgs: [],
      },
      {
        title: 'The invitation waits on holders', via: 'piece put · through forwarders',
        text: 'Padded to a multiple of 4096 bytes and sealed to your one-time key, the invitation is stored at slot locators only your key can compute.',
        data: `locator(i) = BLAKE3_keyed(derive_key("hoi invitation pickup v1", key), u32(i))   // i < 8
{ "t": "piece", "o": "put", … }   // ≤ 1 MiB, kept 7 days`,
        caps: ['≤ 32 pickups'],
        src: ['core/crates/hoi-core/src/pickup.rs:11-80'],
        open: ['founder', 'm3', 'quic'],
        msgs: [{ from: 'founder', to: 'm3', name: 'piece put', fam: 'carry' }],
      },
      {
        title: 'Pick it up', via: 'piece get · polling',
        text: 'Your device asks each holder for slot 0, every 5 s growing to 120 s, until the invitation is there.',
        data: `{ "t": "piece", "o": "get", <locator> }`,
        src: ['core/crates/hoi-core/src/pickup.rs:47-48'],
        open: ['you', 'm3', 'rtc'],
        msgs: [{ from: 'you', to: 'm3', name: 'piece get', fam: 'carry' }, { from: 'm3', to: 'you', name: 'invitation', fam: 'priv' }],
        set: { links: 1 },
      },
      {
        title: 'Process the Welcome', via: 'on the device · nothing sent', local: 'you',
        text: 'The Welcome puts you in the group\'s current epoch; the bootstrap gives you the community\'s genesis and identities.',
        data: `private_join(invitation) → MLS group at epoch N, room key for epoch N`,
        src: ['core/crates/hoi-core/src/private.rs:1290'],
        msgs: [],
        set: { communities: 1 },
      },
      {
        title: 'Read through the encrypted relay', via: 'HTTPS · encrypted relay (optional, none built in)',
        text: 'Objects for the epoch\'s room are fetched with signed read requests. The relay sees room ids, rounded sizes and times, never member keys or community ids.',
        data: `GET {relay}/v2/rooms/{room}/objects?after={cursor}
  x-hoi-read-time, x-hoi-read-nonce, x-hoi-read-signature   // 300 s window
GET {relay}/v2/rooms/{room}/objects/{id}   // 4096 ≤ size ≤ 1 MiB, sha256 = id
PUT {relay}/v2/rooms/{room}/objects/{id}   x-hoi-expires, x-hoi-signature`,
        caps: ['≤ 64 objects per page, ≤ 64 pages', '15 s timeout · ≤ 8 epochs and ≤ 64 MiB per pass', 'relay keeps ≤ 31 days; the app asks for 30'],
        src: ['apps/web/src/lib/private-relay.ts:5-161', 'relay/protocol.mjs:4-12'],
        open: ['you', 'relay', 'https'],
        msgs: [{ from: 'you', to: 'relay', name: 'GET objects', fam: 'priv' }, { from: 'relay', to: 'you', name: 'objects', fam: 'priv', n: 4 }],
        set: { links: 2, records: 96 },
      },
    ],
  },
];

export const DEVICES: { id: Scenario['device']; label: string }[] = [
  { id: 'browser', label: 'Browser' },
  { id: 'phone', label: 'Phone' },
  { id: 'desktop', label: 'Desktop' },
  { id: 'node', label: 'hoi-node' },
];

/** Which scenarios each device can play. */
export const FOR_DEVICE: Record<Scenario['device'], string[]> = {
  browser: ['node-link', 'invite', 'direct', 'private'],
  phone: ['pair', 'invite', 'private'],
  desktop: ['home-join', 'lan', 'private'],
  node: ['public-node', 'home-join', 'lan'],
};
