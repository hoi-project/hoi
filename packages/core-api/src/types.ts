// The one surface every hoi UI talks to. Implemented by:
//   - the web worker running hoi-wasm (apps/web/src/lib/core/worker-core.ts)
//   - the Tauri desktop shell calling native hoi-core (apps/web/src/lib/core/tauri-core.ts)
//   - a dev-only in-memory mock (apps/web/src/lib/core/mock-core.ts)
// The Rust DTOs in core/crates/hoi-core/src/dto.rs mirror these types field for field
// (serde camelCase). Change both together.
//
// Words: the UI never says "user", "account", "content" or "platform".
// Keys and ids are lowercase hex strings of 32 bytes unless noted.

export type Hex = string;
export type Did = string; // did:key:z6Mk…
export type Unix = number; // seconds, UTC

/** Observations of known deletion targets, never proof of complete inventory or erasure. */
export interface DeletionProgress {
  id: Hex;
  target: Hex;
  localRemoved: boolean;
  recordSubmitted: boolean;
  queued: number;
  submitted: number;
  acknowledged: number;
  legacyRemoval: number;
  unknown: number;
  refused: number;
  unreachable: number;
  expired: number;
  outstanding: number;
  coverageGaps: number;
  expiresAt: Unix;
  requiresLocalState: boolean;
  inventoryComplete: boolean;
}

export type CommunityColor = 'mint' | 'sky' | 'lilac' | 'peach' | 'sun' | 'coral';
export const COMMUNITY_COLORS: CommunityColor[] = ['mint', 'sky', 'lilac', 'peach', 'sun', 'coral'];
export type AvatarColor = 'mint' | 'sky' | 'lilac' | 'peach' | 'sun';
export const AVATAR_COLORS: AvatarColor[] = ['mint', 'sky', 'lilac', 'peach', 'sun'];

export type Admission = 'open' | 'invite' | 'age-proof' | 'approve';

/**
 * The trust ladder (PROTOCOL.md §15, §16): how newcomers earn their way up, and how the
 * community, not a moderator alone, decides what naps and who is blocked. Seconds for durations.
 * Quorums are in voices: someone vouched in alone holds a whole one, people vouched in together
 * share their voucher's (PROTOCOL.md §15.4). Changing any of this is a proposal the community
 * decides (§6.1).
 */
export interface Ladder {
  /** Memory-hard identity work every author's key must carry, in bits. */
  identityBits: number;
  /** Vouches from established members that make a newcomer a member. */
  admitVouches: number;
  /** Time as a member before someone is established (may vouch, review, veto). */
  establishedAge: number;
  /** Posts or comments in the last 90 days an established member needs to be trusted (media). */
  trustedPosts: number;
  /** Response from the community those contributions need for trust (media). */
  mediaScore: number;
  /** Voices of established members whose hides put a record to sleep for everyone. */
  hideQuorum: number;
  /** Voices of established hiders that block a member. */
  blockQuorum: number;
  /** Voices of established members whose vetoes overturn a moderator's or owner's action. */
  vetoQuorum: number;
  /** How long a removal naps (and can be vetoed) before its body is deleted. */
  contestWindow: number;
}

export const DEFAULT_LADDER: Ladder = {
  identityBits: 8,
  admitVouches: 2,
  establishedAge: 1_209_600, // 14 d
  trustedPosts: 5,
  mediaScore: 10,
  hideQuorum: 3,
  blockQuorum: 5,
  vetoQuorum: 5,
  contestWindow: 259_200, // 72 h
};

/** PROTOCOL.md §5. Seconds for durations. */
export interface Limits {
  powBits: number;
  maxRetention: number;
  newcomerPostsPerDay: number;
  newcomerAge: number;
  admission: Admission;
  autobanThreshold: number;
  labelers: Hex[];
  ladder: Ladder;
  /** Scattered delivery: percent of the places a post's pieces go to that a party may run
   * without being able to rebuild or withhold it (10, 15 or 20; SCATTER.md §6). */
  scatterResist: number;
  /** Open communities only: whether post bodies travel in pieces. */
  scatterOpen: boolean;
  /**
   * Trackers the community names: at most two https registries where public nodes
   * carrying it announce themselves, asked only when no route a device holds answers.
   */
  trackers: string[];
  /** Age-proof admission: the credential issuers the community trusts, as hex keys. */
  ageIssuers: string[];
}

/** A list keeper whose blocklist you follow, and what their lists hold. */
export interface BlocklistKeeper {
  person: Person;
  exact: number;
  pdq: number;
}

/** An aggregator you follow, and how many of your communities it counts for. */
export interface AggregatorView {
  person: Person;
  communities: number;
}

/** What an aggregator you follow counts for a post. */
export interface TallyView {
  by: string;
  key: Hex;
  up: number;
  down: number;
  at: Unix;
}

/** What a wallet needs to bind its age proof to you and this community. */
export interface AgeChallenge {
  aud: string;
  nonce: string;
  issuers: string[];
}

export const DEFAULT_LIMITS: Limits = {
  powBits: 22,
  maxRetention: 15_552_000, // 180 d
  newcomerPostsPerDay: 2,
  newcomerAge: 2_592_000, // 30 d
  admission: 'open',
  autobanThreshold: 12,
  labelers: [],
  ladder: DEFAULT_LADDER,
  scatterResist: 10,
  scatterOpen: false,
  trackers: [],
  ageIssuers: [],
};

/** Where someone stands in a community (PROTOCOL.md §15). */
export type Standing = 'newcomer' | 'member' | 'established' | 'trusted';

/** What a rung allows: newcomers text; members links and votes that count; established
 * members reviews, vouches and vetoes that count; trusted members media. */
export type Capability = 'text' | 'links' | 'vote' | 'review' | 'vouch' | 'veto' | 'media';

/** A member's judgement. `ok`/`hide` for posts and comments, `veto`/`ok` for ledger entries. */
export type Verdict = 'ok' | 'hide' | 'veto' | 'none';

/** Reasons to hide or object. Disagreeing isn't one: that's what downvotes are for. */
export type HideReason = 'spam' | 'abuse' | 'illegal' | 'off-topic' | `rule:${number}`;

// ---------------------------------------------------------------- people

/** How much you trust a key, from your device's point of view (PROTOCOL.md §8). */
export type TrustLevel = 'me' | 'friend' | 'fof' | 'member' | 'newcomer';

export interface Person {
  /** Signed identicon palette; absent uses the public key's deterministic color. */
  avatarColor?: AvatarColor;
  /** Sanitized JPEG, at most 4 KiB, encoded as base64url. */
  avatar?: string;
  /** Root key if a delegation is known, else the signing key. */
  key: Hex;
  /** 'a7f2·91c0' */
  short: string;
  /** What to show: your petname, else the name they use, else `short`. */
  name: string;
  /** The name they chose for themselves (carried on their posts). Anyone can claim any name; show `short` next to it. */
  nameHint?: string;
  /** Your private name for them, never shared. */
  petname?: string;
  trust: TrustLevel;
  muted: boolean;
  /** Only set when the person is shown in a community context. */
  role?: 'owner' | 'mod';
  banned?: boolean;
  /** Days since your device first saw a record from this key. */
  keyAgeDays: number;
  /** Their rung on the community's ladder (community context only). */
  standing?: Standing;
  /** A friend of a friend: your friends whose trust statements name them (at most 3). */
  trustedBy?: string[];
  /** Their name is a domain: whether the domain names this key. */
  handle?: HandleView;
}

/** A name that is a domain, and whether the domain says the key is theirs. */
export interface HandleView {
  domain: string;
  verified: boolean;
  /** Not checked on this device yet. */
  checking: boolean;
}

export interface DeviceInfo {
  key: Hex;
  short: string;
  label: string;
  thisDevice: boolean;
  expiresAt: Unix;
  revoked: boolean;
  /** This computer keeps that browser's key and signs for it. */
  signedHere: boolean;
}

/** What a pairing link says, read by the core (the link itself is masked). */
export interface PairPreview {
  /** The name the link gives: whatever the link says. */
  name?: string;
  /** WebSocket address; empty when the link names only WebRTC routes. */
  url: string;
  /** WebRTC routes, `<fingerprint>@<address>`. */
  routes: string[];
  /** This device already paired through this link, or the home node's next one. */
  pairedHere: boolean;
}

export interface Me {
  avatarColor?: AvatarColor;
  /** Sanitized JPEG, at most 4 KiB, encoded as base64url. */
  avatar?: string;
  root: Did;
  rootKey: Hex;
  /** The root more identity work is mined for: the one this device signs with now. Same as
   * `rootKey` until a recovery replaces your root. */
  workKey: Hex;
  /** 'e04c · 2a9f · 7b31 · 0cd8', compare in person */
  fingerprint: string;
  short: string;
  name: string;
  createdAt: Unix;
  /** This device holds the root key (can bless new devices, sign as owner). */
  hasRoot: boolean;
  device: Hex;
  devices: DeviceInfo[];
  /** Memory-hard work behind this key (PROTOCOL.md §4.1). Communities may ask for a minimum. */
  identityBits: number;
  /** You burned this key: nothing it signs counts anywhere. Wipe and start fresh. */
  burned: boolean;
  /** This device still keeps your recovery line, until you say it is saved. */
  holdsLine: boolean;
  /** Brought back from the line; not yet sure it signs with your current root. Join one of
   * your communities so it can find your key's history. */
  restoring: boolean;
  /** Another device replaced the root this one signs with: nothing it signs counts anymore. */
  replaced: boolean;
  /** This browser keeps no signing key: your computer signs for it over the home link. */
  remote: boolean;
}

/** On a computer: something a browser it signs for asked to sign, waiting for your yes. */
export interface SignAsk {
  id: Hex;
  /** The browser, as this computer calls it. */
  device: string;
  /** 'ledger' for moderation, rules, limits and owners. */
  kind: string;
  community: Hex | null;
  at: Unix;
}

/** How your own recovery stands. */
export interface RecoveryStatus {
  holdsLine: boolean;
  restoring: boolean;
  /** How many times your root was replaced. */
  rotations: number;
  /** The friends who keep a share of your line. */
  friends: Hex[];
}

/** A request a new device shows: the code to send friends, the digits to read out. */
export interface RecoveryAsk {
  code: string;
  /** '123 456' */
  digits: string;
}

/** A friend's answer: the share sealed to the request, and the digits it was for. */
export interface RecoveryAnswer {
  digits: string;
  code: string;
}

/** Someone whose share of their recovery line this device keeps. */
export interface HeldShare {
  person: Hex;
  name: string;
  short: string;
  since: Unix;
}

/** A key made and waiting for its identity work: the first run shows that work as it happens. */
export interface IdentityDraft {
  root: Did;
  rootKey: Hex;
  fingerprint: string;
  short: string;
  /** Identity work to mine, bits. */
  bits: number;
}

/** One attempt at a new key's identity work (PROTOCOL.md §4.1), as a miner made it. */
export interface IdentityTry {
  /** The nonce tried. */
  work: number;
  /** The Argon2id output, 32 bytes. */
  hash: Uint8Array;
  /** Its leading zero bits: the work this nonce carries. */
  bits: number;
}

/** How far more identity work has come: tries so far, and the most zero bits any try reached. */
export interface WorkProgress {
  tries: number;
  best: number;
}

export interface StrengthenOptions {
  /** A few times a second while the work goes on. */
  onProgress?: (p: WorkProgress) => void;
  /** Abort to stop the work: the promise rejects with an `AbortError` and the key keeps the
   * work it had. */
  signal?: AbortSignal;
}

export interface MakeKeyOptions {
  /** Randomness from your gestures, already hashed (at most 64 bytes). The core mixes it into the
   * system's randomness, never uses it instead: the key is just as safe without it. */
  stir?: Uint8Array;
  /** The key exists; its identity work starts now. */
  onKey?: (draft: IdentityDraft) => void;
  /** Every attempt at the identity work, as it happens. Hosts that mine natively report none. */
  onTry?: (t: IdentityTry) => void;
}

// ---------------------------------------------------------------- communities

export interface CommunityRef {
  id: Hex;
  name: string;
  color: CommunityColor;
}

/** Open communities carried locally or advertised by connected nodes. This is a partial
 * directory, never a reservation or a guarantee that a name is globally unique. */
export interface DiscoveredCommunity extends CommunityRef {
  description: string;
  /** Signed community details and member routes; never a home-node pairing credential. */
  invite: string;
  joined: boolean;
  /** A node connected now lists it. False for a listing this device remembers from earlier. */
  live?: boolean;
  /** When a node last listed it as its own, as far as this device knows (unix seconds; 0: never heard). */
  seen?: number;
}

/** One-time proof signed by the community key before that key is discarded. */
export interface CommunityRegistration {
  name: string;
  community: Hex;
  signature: string;
}

export interface CommunityPublication extends CommunityRegistration {
  protocolVersion: string;
  indexVersion: number;
}

export interface PublicJoinStatus {
  id: Hex;
  status: 'unknown' | 'pending' | 'joined' | 'rejected';
  reason?: string;
}

export type WarmthLevel = 'cold' | 'cool' | 'warm' | 'hot';

/** How safe one community's live records are, read from this device's own links (PROTOCOL.md
 * §20), the way a torrent reads availability: cold with a copy here alone, cool with one other
 * independent place holding it, warm with two, hot with three complete copies that caught up
 * recently. Nodes in one network or under one operator are one place; copies reached only over
 * unguarded links read colder. These are copies this device could see, never a delivery receipt. */
export interface CommunityWarmth {
  community: Hex;
  name: string;
  level: WarmthLevel;
  /** 0 to 100, for a bar. */
  score: number;
  /** Copies on other durable nodes, once per independent place: what the level is read from. */
  kept: number;
  /** Independent places holding a durable copy (one network, one operator: one place). */
  places: number;
  /** 1 when every other durable copy is in a place of its own, lower when they share one. */
  spread: number;
  /** Share of those copies reached over a guarded link (encrypted, other end checked), 0 to 1. */
  guarded: number;
  /** Copies this device can see, its own included; one decimal. */
  copies: number;
  /** Of those, copies on nodes that outlive a browser tab (desktops, headless nodes). */
  durableCopies: number;
  /** The best a connected node reports, or this device's own count when higher; one hop, never added up. */
  networkCopies: number;
  /** Share of the live window known to exist that is here, 0 to 1. */
  complete: number;
  /** Share of referenced media that is here, 0 to 1. */
  media: number;
  /** A connected peer lists it, or a catch-up finished within the last seven days. */
  fresh: boolean;
  /** A connected peer lists it right now. */
  live: boolean;
  records: number;
  missing: number;
  holdersDurable: number;
  holdersTransient: number;
  /** Members' nodes met within 30 days, by route: leads, not copies. */
  remembered: number;
  ownDurable: boolean;
  lastSync?: Unix;
  /** Set when this node carries the community for others (native nodes). */
  carried?: 'index' | 'request';
  /** A public join still waiting for the signed definition. */
  pending: boolean;
}

/** What this device knows of the directory beyond any registry: connected nodes sharing their
 * catalogs, open communities they list, nodes that take carry requests. */
export interface DirectoryWarmth {
  catalogNodes: number;
  discoverable: number;
  carryNodes: number;
}

/** The warmth of this device's own connections (PROTOCOL.md §20): through how many independent
 * places it is linked, whether one carries most of its links, how many links are guarded. */
export interface LinkWarmth {
  level: WarmthLevel;
  /** 0 to 100, inside the level's band. */
  score: number;
  links: number;
  places: number;
  /** The effective number of places (two decimals). */
  effective: number;
  /** Share of links in the place that carries the most, 0 to 1. */
  largest: number;
  /** Share of guarded links, 0 to 1. */
  guarded: number;
}

export interface WarmthView {
  /** Public communities this device is in, coldest first. */
  communities: CommunityWarmth[];
  directory: DirectoryWarmth;
  ownDurable: boolean;
  links: LinkWarmth;
}

/** The outcome of asking connected public nodes to keep a community available. */
export interface CarryRequest {
  community: Hex;
  /** Nodes asked just now; their answers arrive as notices. */
  asked: number;
  /** Connected nodes that take requests and already list this community. */
  carrying: number;
}

export interface CommunitySummary extends CommunityRef {
  description: string;
  createdAt: Unix;
  joined: boolean;
  /** Keep the full live window so newcomers can catch up from you. */
  pinning: boolean;
  role: 'owner' | 'mod' | 'member';
  posts: number;
  lastActivity: Unix;
}

export interface RulesView {
  text: string;
  version: number;
  at: Unix;
  signedBy: Person[];
}

export interface BanView {
  person: Person;
  reason: string;
  at: Unix;
  auto: boolean;
}

export interface CommunityView extends CommunitySummary {
  owners: Person[];
  /** Owner signatures needed for rules, limits and moderator changes. */
  threshold: number;
  mods: Person[];
  limits: Limits;
  rules: RulesView | null;
  stats: { posts: number; comments: number; people: number; storedBytes: number; ledgerEntries: number };
  /** Keys waiting at the door (admission: approve). Only filled for owners and mods. */
  pending: Person[];
  banned: BanView[];
  /** Signed flags from members, newest first. Only filled for owners and mods. */
  flags: FlagView[];
  /** Established members, owners and moderators included. */
  established: number;
  /** Their voice together, in voices. */
  establishedVoice: number;
  /** Your place on the ladder. */
  me: StandingView | null;
  /** Newcomers' posts waiting for your review (established members). */
  reviewQueue: number;
  /** Proposed rules and limits still being decided, or decided in the last 30 days. */
  proposals: ProposalView[];
  /**
   * You own this members-only community, whose records travel in the clear, and could make a
   * private copy of it (`privateCopy`): an encrypted group whose membership follows its records.
   */
  privateCopy?: boolean;
  /**
   * An open community whose posts go in pieces: the level the posts kept in pieces here meet
   * (percent of the places their pieces go to). Absent while posts here travel whole.
   */
  pieces?: number;
  /** Trackers to ask when no route answers: the community's, then any an invite named. */
  trackers: string[];
}

/**
 * open: waiting for answers · in-effect: everyone still around agreed, the rest are away (it
 * stops if they come back and object before the deadline) · passed: for good · lapsed: the
 * deadline passed with objections.
 */
export type ProposalStatus = 'open' | 'in-effect' | 'passed' | 'lapsed';

/** A field a limits proposal changes. `field` is the snake_case name (`hide_quorum`, `max_retention`, …). */
export interface ChangeView {
  field: string;
  from: string;
  to: string;
}

/**
 * A proposed change to the rules or limits (PROTOCOL.md §6.1). The people established when it
 * was proposed decide; answering is a `review` of it: 'ok' agrees, 'veto' objects (with a
 * reason), 'none' takes it back. Voices are in whole voices.
 */
export interface ProposalView {
  id: Hex;
  kind: 'limits' | 'rules';
  at: Unix;
  /** One retention window after `at`: answers close and silence counts as yes. */
  deadline: Unix;
  status: ProposalStatus;
  decidedAt?: Unix;
  /** Everyone agreed (rather than nobody objecting in time). */
  unanimous: boolean;
  by: Person[];
  changes: ChangeView[];
  /** The proposed rules text. */
  text?: string;
  electors: number;
  agreed: Person[];
  objecting: { person: Person; reason: string }[];
  /** Around and haven't answered: it waits for them. */
  waiting: Person[];
  /** Away (nothing of theirs left here) and haven't answered: they don't hold it up. */
  away: number;
  /** Retired by the owners after leaving for good: their silence counts for nothing. */
  retired: number;
  /** For owners: away electors silent here for 30 days, who may be retired. */
  retirable?: Person[];
  agreedVoice: number;
  objectingVoice: number;
  waitingVoice: number;
  myAnswer?: 'agree' | 'object';
  /** You're one of the people deciding it and answers are open. */
  canAnswer: boolean;
}

export interface StandingView {
  community: Hex;
  person: Person;
  level: Standing;
  capabilities: Capability[];
  admittedAt: Unix | null;
  establishedAt: Unix | null;
  /** Established members whose vouches counted. */
  vouchedBy: Person[];
  vouchesNeeded: number;
  objections: number;
  /** Posts and comments in the last 90 days. */
  contributions: number;
  contributionsNeeded: number;
  /** Upvotes and "looks good" from the community, minus hides and removals. */
  score: number;
  scoreNeeded: number;
  blocked: boolean;
  /** Vouched for people the community later blocked: no voice in reviews and vetoes. */
  struck: boolean;
  /** Your own vouch for them. */
  myVouch: -1 | 0 | 1;
  /** Their weight in hides, blocks, vetoes and objections, 0 to 1 (PROTOCOL.md §15.4). */
  voice: number;
  /** People they vouched for this quarter: each of those vouches carries `voice / n`. */
  vouchedThisQuarter: number;
  identityBits: number;
  identityNeeded: number;
  /** The next rung and what's missing, in plain words. */
  next: { level: Standing; missing: string[] } | null;
}

export interface MediaView {
  /** BLAKE3 of the bytes. */
  hash: Hex;
  mime: 'image/png' | 'image/jpeg' | 'image/gif' | 'image/webp' | 'audio/ogg' | 'video/mp4' | 'video/webm';
  size: number;
  w?: number;
  h?: number;
  alt?: string;
  /** The checked bytes are on this device; otherwise they're on their way. */
  available: boolean;
}

/** A file to attach. The core sniffs its type and strips metadata (location, camera, thumbnails)
 * before anything is hashed or shared. Images ≤ 2 MiB, Ogg Opus ≤ 8 MiB, MP4/WebM ≤ 16 MiB. */
export interface MediaDraft {
  data: Uint8Array;
  alt?: string;
}

export interface MediaBlob {
  hash: Hex;
  mime: MediaView['mime'];
  data: Uint8Array;
}

export interface FlagView {
  target: LedgerTarget;
  /** 'spam' | 'rule:<n>' | 'illegal' */
  reasons: string[];
  count: number;
  /** Sum of the flaggers' trust weights from your device's view (autoban when ≥ limits.autobanThreshold). */
  weight: number;
  lastAt: Unix;
}

export interface CommunityDraft {
  name: string;
  description: string;
  color: CommunityColor;
  limits?: Partial<Limits>;
  rules?: string;
  /** Other owners' keys (hex or did:key). You are always an owner. */
  coOwners?: string[];
  /** Owner signatures needed for rules, limits and moderator changes (default 1). */
  threshold?: number;
}

export interface Invite {
  /** hoi://join?code=… — reversible obfuscation, not encryption. Older code and c/g links still open. */
  uri: string;
  /** Same invite as a link that opens this app: https://…/#/join?code=… */
  web: string;
  /** The addresses inside, named before the invite is shared. Absent from older cores. */
  addresses?: InviteAddress[];
  /** Someone on another network can use it: an address inside is on the internet. */
  outside?: boolean;
}

/** One address an invite carries. */
export interface InviteAddress {
  /** ip:port, without the key or certificate fingerprint that comes with it. */
  address: string;
  /** device · network (this home or office) · internet. */
  scope: 'device' | 'network' | 'internet';
  /** yours: this device's or its home node's; node: a node that holds the community or was
   * asked to keep it; entry: a public node this device reached, a way into the network. */
  whose: 'yours' | 'node' | 'entry';
}

export interface InvitePreview {
  community: CommunityRef & { description: string; limits: Limits; owners: Person[] };
  relays: string[];
  peers: string[];
  /** Trackers the invite names: asked only when no route in it answers. */
  trackers: string[];
  alreadyJoined: boolean;
  /** The invite carried the community's signed genesis, so it can be checked offline. */
  verified: boolean;
}

// ---------------------------------------------------------------- posts

/** `counted`: by the counts of the aggregator you follow, on request only. */
export type FeedSort = 'hot' | 'fresh' | 'loudest' | 'friends' | 'counted';

export interface FeedQuery {
  community?: Hex;
  sort: FeedSort;
  limit?: number;
}

export interface Napping {
  label: string;
  /** Who said so: a labeler's name or 'the mods'. */
  by: string;
}

export interface Removal {
  /** null when the author deleted it themselves */
  by: Person | null;
  reason: string;
  at: Unix;
  ledgerId?: Hex;
}

/** A page of a community's wiki, in a list. */
export interface WikiPageSummary {
  page: string;
  chars: number;
  updatedAt: Unix;
  editors: number;
}

/** A page of a community's wiki, as its edits made it (edits merge). */
export interface WikiPageView {
  page: string;
  text: string;
  /** 0 for a page nobody wrote yet. */
  updatedAt: Unix;
  edits: number;
  /** Most recent editors first, at most 8. */
  editors: Person[];
  canEdit: boolean;
}

/** A link's preview, fetched by a desktop app or hoi-node and kept on this device; never published. */
export interface LinkPreview {
  url: string;
  title?: string;
  description?: string;
  site: string;
  /** The picture, sanitized, as a data: URL. */
  image?: string;
}

/** Someone whose labels count on this device: you follow them, or a community you are in names them. */
export interface LabelerView {
  person: Person;
  followed: boolean;
  /** Communities that name them as a default labeler. */
  defaultIn: CommunityRef[];
  /** Their labels held here. */
  labels: number;
}

/** Whether you share whom you trust in a community: everyone there sees the friends it names. */
export interface TrustSharing {
  community: Hex;
  shared: boolean;
  /** Friends the statement names: those known in the community. */
  named: number;
  /** Friends in all. */
  friends: number;
}

export interface PostSummary {
  id: Hex;
  /** 'b3:9e1c…4f0a' */
  short: string;
  community: CommunityRef;
  author: Person;
  title: string;
  excerpt: string;
  link?: string;
  createdAt: Unix;
  expiresAt: Unix;
  /** Net votes this device has seen (up minus down). Ranking weights them by trust; two devices legitimately see different numbers. */
  score: number;
  myVote: -1 | 0 | 1;
  /** Up-votes from your friends (counted fully) and friends of friends (by half): why Hot puts it here. */
  friendsUp: number;
  fofUp: number;
  /** What the aggregators you follow count for it: shown, never ranked on their own. */
  tallies?: TallyView[];
  replies: number;
  pinned: boolean;
  napping?: Napping;
  labels: string[];
  edited: boolean;
  mine: boolean;
  saved: boolean;
  /** Author not admitted yet (admission: approve). Only shown to reviewers and the author. */
  waiting: boolean;
  /** The author is a newcomer the community hasn't validated yet. */
  unvalidated: boolean;
  media: MediaView[];
  /** Only on your own posts: why other devices hold it back for now. */
  held?: string;
}

export interface Checks {
  signature: 'valid';
  signedBy: 'root' | 'device';
  /** Leading zero bits the stamp actually has. */
  stampBits: number;
  requiredBits: number;
  /** Peers that handed this record to you; 0 if it was made here. */
  receivedFrom: number;
  createdAt: Unix;
  fadesAt: Unix;
  /** The author's identity work, bits. */
  identityBits: number;
  /** The author's standing in this community. */
  standing: Standing;
  /** Established members whose vouches validated the author. */
  vouchedBy: number;
  /**
   * Only on your own posts in open communities: the node you connect through can't tell the
   * post is yours (an injector took it and this device never offered your records first).
   * Absent from older cores.
   */
  addressHidden?: boolean;
  /** Authenticated home report; the home still knows who posted. */
  homeDelivery?: string;
}

export interface CommentView {
  id: Hex;
  short: string;
  parent: Hex;
  author: Person;
  text: string;
  createdAt: Unix;
  expiresAt: Unix;
  score: number;
  myVote: -1 | 0 | 1;
  mine: boolean;
  edited: boolean;
  napping?: Napping;
  removed?: Removal;
  waiting: boolean;
  unvalidated: boolean;
  held?: string;
  children: CommentView[];
}

export interface PostView extends PostSummary {
  text: string;
  comments: CommentView[];
  checks: Checks;
  removed?: Removal;
}

export interface PostDraft {
  community: Hex;
  title: string;
  text: string;
  link?: string;
  /** Trusted members only (PROTOCOL.md §17). At most 4. */
  media?: MediaDraft[];
}

// ---------------------------------------------------------------- ledger

export type LedgerAction =
  | 'rules' | 'limits' | 'mod-add' | 'mod-remove' | 'remove' | 'ban' | 'unban'
  | 'label' | 'pin' | 'unpin' | 'admit' | 'drop-key' | 'retire';

export type LedgerTarget =
  | {
      kind: 'record';
      id: Hex;
      short: string;
      /** Post title, or the start of a comment. Gone once the record is removed. */
      title?: string;
      author?: Person;
      recordKind?: 'post' | 'comment';
      /** The post whose thread holds this record (itself for posts), when known. */
      post?: Hex;
    }
  | { kind: 'key'; person: Person };

export interface LedgerEntryView {
  id: Hex;
  seq: number;
  at: Unix;
  action: LedgerAction;
  /** Signer first, then cosigners. */
  by: Person[];
  /** Signed by the community's automation (autoban). */
  auto: boolean;
  target?: LedgerTarget;
  reason?: string;
  text?: string;
  limits?: Limits;
  evidence: number;
  /** null for entries kept until superseded (rules, limits, mod changes). */
  expiresAt: Unix | null;
  /** Established members vetoing it, their voice, and the voice it takes (PROTOCOL.md §16.3). */
  vetoes: number;
  vetoesVoice: number;
  vetoesNeeded: number;
  support: number;
  overturned: boolean;
  vetoable: boolean;
  myReview?: 'veto' | 'ok';
  /** Removals: the body is deleted at this time unless vetoed first. */
  pendingUntil?: Unix;
  /** Rules and limits: where the community's decision stands. */
  proposal?: ProposalStatus;
}

/** Owner actions need `threshold` signatures. With threshold 1 they publish at once. Rules and
 * limits then become proposals the community decides (`CommunityView.proposals`). */
export type LedgerResult =
  | { status: 'published'; entry: LedgerEntryView }
  | { status: 'needs-cosign'; proposal: string; have: number; need: number };

// ---------------------------------------------------------------- node, network, settings

export interface NodeStats {
  records: number;
  storedBytes: number;
  communities: number;
  /** Keys this device refuses (rate, invalid records, weighted reports). */
  bouncedKeys: number;
  bouncedRecords: number;
  peers: number;
  passedAlongBytes: number;
  /** Checked media bytes kept here. */
  mediaBytes: number;
  /** Posts you saved: on this device only, never passed on. */
  saved: number;
  /** People whose posts you read, remembered up to 30 days so the app can vouch for them. Never sent. */
  readingNotes: number;
  /** When the last post or comment stored here fades. */
  fadesBy: Unix;
  /** Pictures this device refuses by fingerprint, near copies included. */
  blockedPictures: number;
  /** While this node keeps mailboxes and pieces for others: how many, how many bytes, and of how
   * many at most (0 when it keeps none). None of it readable here. Absent from older cores. */
  heldPieces?: number;
  heldBytes?: number;
  heldBudget?: number;
}

export interface Settings {
  relay: boolean;
  dailyCapGb: number;
  keepModerated: boolean;
  linkPreviews: boolean;
  tor: boolean;
  theme: 'system' | 'light' | 'dark';
  motion: 'system' | 'reduced';
  /** Vouch for people whose posts you read and didn't hide (established members): newcomers
   * get in, members vouched in with many others get a fuller voice. */
  autoVouch: boolean;
  /** Fetch media of trusted members' posts by itself. */
  autoMedia: boolean;
  /** The person saw what relaying means; native nodes relay by default (TECHNICAL.md §4). */
  relayNoticeSeen: boolean;
  /** Desktop: open as a small companion (status, traffic, pairing) instead of the full app. */
  companion: boolean;
  /** A browser or phone without a home node: the person saw, once, that nobody can read what it
   * posts until a home node carrying the community has it. Absent from older cores. */
  postingNoticeSeen?: boolean;
  /** Desktop: serve strangers as a public node: carry open communities whoever asks, pass on
   * other public nodes, share a node link and have the router forward this computer's ports.
   * Off, this computer publishes no address of its own. Absent from older cores. */
  publicNode?: boolean;
  /** Desktop: only through this VPN, the network interface every connection to the internet
   * must use; empty for every interface. Absent from older cores. */
  vpn?: string;
  /** With a VPN: the home network may still be used directly, so a phone on the Wi-Fi keeps working. */
  homeNetwork?: boolean;
  /** Desktop, while a public node: keep mailboxes and encrypted pieces for others, up to this many
   * gigabytes (0.5 to 100); 0 is off, and turning it off deletes what was kept. Absent from older cores. */
  holdPiecesGb?: number;
  /** Communities marked as favorites in this app's list, in the order shown. This device only:
   * never sent, never in a backup. Absent from older cores. */
  favorites?: Hex[];
  /** The order the person put their other communities in, this device only. */
  communityOrder?: Hex[];
  /** Browsers and phones: "Use less data". Posts kept in pieces are rebuilt by the home node
   * alone, waiting for it rather than fetching here; with no home node, one spare piece fewer.
   * Absent from older cores. */
  lessData?: boolean;
  /** Desktops and nodes: "Share the holders I see". Where the person is established, publish
   * the holders this device saw up for days, which other devices count as one more vouch.
   * Off at first: it tells this device's links whose it is. Absent from older cores. */
  shareHolders?: boolean;
  /** Browsers: "Direct links with members' browsers". A public node this browser is linked to
   * introduces it to other members' browsers that turned it on, and records and media then go
   * between them directly, four links at most. Off at first: the browsers linked
   * learn each other's addresses. Absent from older cores. */
  directLinks?: boolean;
}

/** A network interface the person can pick as their VPN. */
export interface VpnInterface {
  name: string;
  /** What the system calls it, when that differs ("ProtonVPN"). */
  label?: string | null;
  /** A tunnel or point-to-point link: what VPNs look like. */
  tunnel: boolean;
  addresses: string[];
}

/** The VPN every internet connection must go through, as the computer sees it now. */
export interface VpnStatus {
  interface: string;
  /** Up, and the system sends internet traffic through it. While it isn't, nothing goes to the internet. */
  up: boolean;
  addresses: string[];
  /** Why it can't be used right now. */
  detail?: string;
}

/** What the router said when asked to forward a public node's ports. */
export interface PortMapping {
  /** asking · forwarded · own-address (this computer's address is public already) · no-router ·
   * behind-nat (carrier-grade NAT or a second router) · refused · silent (no PCP, NAT-PMP or
   * UPnP answer) · failed. */
  state: 'asking' | 'forwarded' | 'own-address' | 'no-router' | 'behind-nat' | 'refused' | 'silent' | 'failed';
  /** The public addresses visitors reach this node at, as the router named them. */
  public: string[];
  /** The router's reason for a refusal, or the address that isn't public. */
  detail?: string;
  /** Asked through a VPN's gateway rather than the home router. */
  viaVpn: boolean;
}

export interface NetStatus {
  /** Bound native endpoint; this alone does not mean a peer is reachable. */
  transport?: { endpointId: string; peers: string[]; relays: string[]; error?: string };
  connections?: {
    peer: string;
    role: 'peer' | 'home' | 'client';
    route: string;
    connected: boolean;
    /** quic, webrtc or websocket. */
    transport: string;
    /** How the bytes travel, in words: "encrypted · WebRTC, DTLS", "stays on this device". */
    encryption: string;
    /** home: this device's home node · device: one of your devices it serves · public: a public
     * node (takes carry requests) · node: another node that keeps its copy · browser: a browser
     * or phone whose copy closes with it. Absent from older cores. */
    kind?: 'home' | 'device' | 'public' | 'node' | 'browser';
    /** The other end keeps records after the link closes. */
    keeps?: boolean;
    /** The other end dialed this device. */
    incoming?: boolean;
    /** A direct link with another member's browser. Absent from older cores. */
    direct?: boolean;
    /** Your public communities this link syncs: where a post in one of them goes from here. */
    communities?: Hex[];
  }[];
  /** Most recent completed community catch-up, Unix seconds. */
  lastSync?: number;
  /** Joined communities, including those still waiting for details. Peers count only after
   * their greeting advertises this exact ID. A catch-up is not a per-post delivery receipt. */
  communities?: { community: Hex; connectedPeers: number; lastSync?: Unix }[];
  /** local: nothing to talk to yet; home-node: paired with your desktop or hoi-node; iroh: full peer (desktop). */
  mode: 'local' | 'home-node' | 'iroh';
  /** Honest one-liner for the relay pill: "only on this device", "via your home node", "via mira's relay". */
  label: string;
  peers: number;
  homeNode?: {
    url: string;
    connected: boolean;
    name?: string;
    error?: string;
    /** The pairing link as a web address (WebRTC routes included): show it as a QR code and a
     * phone's camera opens the app, pairs with the same home node and can ask to be linked. */
    pairingWeb: string;
    /** The pairing link names a route a phone can use (a WebRTC route or wss://). The link is
     * masked, so this is how the app knows. */
    phoneCanReach?: boolean;
    /** websocket or webrtc, once connected. */
    via?: string;
  };
  /** Desktop and hoi-node: browsers and phones may pair with this node. */
  serving?: { url: string; pairing: string; pairingWeb: string; phoneCanReach?: boolean; clients: number };
  /** This node's own WebRTC routes (rtc:<fingerprint>@<addr>). */
  rtc: string[];
  /** Members' browsers and nodes may connect and catch up here right now. */
  relaying: boolean;
  /** Public nodes this device knows of besides its communities' routes (PROTOCOL.md §14.2):
   * `known` answered it as public nodes, `waiting` it was told of or given a link to and hasn't
   * reached yet, `connected` of the known ones are connected now. Nothing is built in. */
  directory?: DirectoryStatus;
  /** Desktop and hoi-node while they serve strangers: a link anyone can open to reach this node
   * and see the open communities it carries (`…#/node?…`). An address, never a credential. */
  nodeLink?: string;
  /** While the router is asked to forward this node's ports: what it said. */
  portMapping?: PortMapping;
  /** Every address this node gives out, and who hears it. Empty in a browser;
   * absent from older cores. */
  exposure?: Exposure[];
  /** While only a VPN may be used: whether it is up. */
  vpn?: VpnStatus;
  /** Operators' notices waiting for the person's answer: public nodes this device met
   * and will not talk to until they are read. Answer with acceptNotice or declineNotice. */
  pendingNotices?: NoticeView[];
}

/** One part of an operator's notice as people read it. */
export interface NoticeSection {
  title: string;
  text: string;
}

/** An operator's notice as this device keeps it: the node it belongs to, what the
 * person decided, and the words, fixed sentences with the operator's fields in them. */
export interface NoticeView {
  /** The node's endpoint key, hex: what the decision is kept under. */
  node: Hex;
  /** BLAKE3 of the notice's canonical bytes, hex: a changed notice is a new question. */
  hash: Hex;
  state: 'pending' | 'accepted' | 'declined';
  title: string;
  sections: NoticeSection[];
  footer: string;
  /** The operator's own fields. */
  name: string;
  operator: string;
  contact: string;
  country: string;
  provider?: string;
  ipLogging: boolean;
  retentionDays: number;
  updated: string;
  /** Where the node was met (an address), when known. */
  host?: string;
  seen: Unix;
  decidedAt?: Unix;
}

/** What a node link says before this device dials it. */
export interface NodeLinkPreview {
  /** The first address the link dials, and how far it reaches. */
  host: string;
  scope: 'device' | 'network' | 'internet';
  endpoint?: Hex;
  /** The operator's notice in the link, when it carries one (kept as pending until accepted). */
  notice?: NoticeView;
  /** The notice is signed by the node the link names. */
  verified: boolean;
  /** Why the link cannot be opened, in a sentence. */
  problem?: string;
}

/** One address of this node's own, and who is told it. */
export interface Exposure {
  /** ip:port. */
  address: string;
  /** device · network (this home or office) · internet. */
  scope: 'device' | 'network' | 'internet';
  /** The router forwards it here (port mapping): this computer doesn't hold it itself. */
  fromRouter: boolean;
  /** devices: your own, and their pairing links · strangers: whoever connects, and invites ·
   * node-link: the link anyone can open. */
  told: ('devices' | 'strangers' | 'node-link')[];
}

export interface DirectoryStatus {
  known: number;
  waiting: number;
  connected: number;
  /** Open communities this device heard of from nodes and keeps between visits. */
  listings?: number;
}

/** What passed through this device since it started: amounts and times, never content. */
export interface Traffic {
  since: Unix;
  now: Unix;
  bytesIn: number;
  bytesOut: number;
  /** Signed records that arrived and passed every check here. */
  recordsIn: number;
  /** Records sent on, to your devices and to members. */
  recordsOut: number;
  mediaIn: number;
  mediaOut: number;
  /** Records refused here: bad bytes, signatures, stamps or rules. */
  refused: number;
  /** The last hour, one entry a minute, oldest first. */
  minutes: { at: Unix; bytesIn: number; bytesOut: number }[];
  connections: {
    peer: string;
    /** home: your home node · client: one of your devices · member: someone else's device. */
    role: 'home' | 'client' | 'member';
    /** Your own devices only. */
    name?: string;
    transport: string;
    encryption: string;
    bytesIn: number;
    bytesOut: number;
    recordsIn: number;
    recordsOut: number;
    since: Unix;
    lastAt: Unix;
  }[];
  relay: {
    on: boolean;
    /** Native nodes only: a browser can't take connections. */
    possible: boolean;
    capBytes: number;
    todayBytes: number;
    /** Over today's cap: members are turned away until tomorrow (UTC). */
    napping: boolean;
  };
}

/** Another of your devices asks to be linked to your key, through your home node. */
export interface LinkAsk {
  id: number;
  label: string;
  /** Four digits the asking device shows too. */
  check: string;
  device: Hex;
  at: Unix;
}

export interface Boot {
  platform: 'web' | 'desktop' | 'mobile';
  version: string;
  protocol: number;
  hasIdentity: boolean;
  /** Web only: another tab owns the local store. Call takeOver() to move it here. */
  blockedByOtherTab?: boolean;
  /** Legacy API counter. Unsupported records are now preserved instead of deleted. */
  cleaned: number;
  /** Unsupported records retained on disk but not indexed or served by this build. */
  preservedRecords?: number;
}

export interface ImportReport {
  accepted: number;
  duplicates: number;
  rejected: number;
  reasons: Record<string, number>;
}

export interface PersonView {
  person: Person;
  communities: CommunityRef[];
  posts: number;
  firstSeen: Unix;
}

export type CoreEvent =
  /** Records arrived, expired or were removed. Refresh what is on screen. */
  | { type: 'changed'; communities: Hex[] }
  | { type: 'net'; status: NetStatus }
  /** A new device of yours asks to be linked (root devices only). Answer with linkAnswer. */
  | { type: 'linkAsk'; ask: LinkAsk }
  /** On the new device: the four digits to compare, one set per device holding your key that
   * answered (usually one). They arrive once that device has seen the request. */
  | { type: 'linkCheck'; checks: string[] }
  /** This device was linked to your key through the home node. */
  | { type: 'linked'; me: Me }
  /** On a computer: a browser it signs for asks for a signature that waits for a yes. */
  | { type: 'signAsk'; ask: SignAsk }
  /** On a browser your computer signs for: the answer to something it sent. */
  | { type: 'signed'; id: Hex; ok: boolean; message: string | null }
  /** A link preview arrived: ask `linkPreview(url)` again. */
  | { type: 'preview'; url: string }
  /** A public node this device met asks the person to read its operator's notice;
   * `net().pendingNotices` holds the words. */
  | { type: 'nodeNotice'; node: Hex }
  /** Proof-of-work for a record you are signing. estimateMs is from this device's benchmark. */
  | { type: 'stamp'; phase: 'start' | 'done'; bits: number; estimateMs: number; tookMs?: number }
  /** Something worth a toast, e.g. "Bounced 14 records from f30a·c812". */
  | { type: 'notice'; text: string };

export type ErrorCode =
  | 'no-identity' | 'not-found' | 'not-allowed' | 'rate-limited' | 'invalid'
  | 'bad-invite' | 'bad-passphrase' | 'expired' | 'banned' | 'too-big' | 'offline' | 'storage-incompatible'
  | 'deletion-full' | 'deletion-pending' | 'internal';

export class HoiError extends Error {
  constructor(public code: ErrorCode, message: string) {
    super(message);
    this.name = 'HoiError';
  }
}

// ---------------------------------------------------------------- the API

export interface PrivateGroup {
  community: Hex;
  room: Hex;
  epoch: number;
  active: boolean;
  isOwner: boolean;
  members: { device: Hex; root: Hex }[];
  pending: number;
  /** Ciphertext hashes already processed; never message plaintext or decryption keys. */
  received: string[];
  /**
   * How the group syncs: `mailboxes` on public nodes the group agreed on (they keep sealed
   * messages they cannot read), or `waiting` while the owner's device is still choosing them
   * (an encrypted relay carries it meanwhile, where the build has one).
   */
  transport: 'mailboxes' | 'waiting';
  /** The group's mailboxes, and whether this device is linked to each now. */
  mailboxes: { endpoint: Hex; linked: boolean }[];
  /**
   * How posts go out now: `pieces` (sealed, cut into pieces and sent through other public nodes
   * to holders in independent places; only a small pointer goes through the mailboxes), or
   * `whole` (too few places are around: posts go whole through the mailboxes).
   */
  pieces: 'pieces' | 'whole';
  /**
   * The level this group's posts kept in pieces here meet, the lowest among them: the percent
   * of places someone would need to stop one (10, 15 or 20). Absent when none is kept here.
   */
  piecesLevel?: number;
  /**
   * Membership follows the community's own records (admission by invitation or approval): the
   * owner's and moderators' devices add whom they admit and remove whom they ban; every member
   * refuses anything else.
   */
  followsRecords: boolean;
  /** Invitations a committing device made for requests passed on, ready to hand over. */
  invitations: { device: Hex; invitation: string }[];
  /**
   * How many devices change membership in this epoch: the owner's, then moderators' by how long
   * they have been moderators. The first makes each change at once; when it stays silent, the
   * next takes over after an hour, the one after it after two.
   */
  committers: number;
  /** This device's place among them (0: first in line), when it is one. */
  rank?: number | null;
}

/** A sealed, padded object for relay protocol v2, signed by its epoch's room key. */
export interface PrivateRelayObject {
  id: string;
  /** The room of the epoch the message was made in; a membership commit goes to the room it leaves. */
  room: Hex;
  body: string;
  expires: number;
  size: number;
  signature: string;
}

/** Read signatures for the current epoch's room, one per requested target, each accepted once. */
export interface PrivateRelayRead {
  room: Hex;
  reads: { time: number; nonce: string; signature: string }[];
}

export interface HoiCore {
  boot(): Promise<Boot>;
  takeOver(): Promise<Boot>;
  on(listener: (e: CoreEvent) => void): () => void;

  // you
  me(): Promise<Me | null>;
  /** Makes this device's key and does its identity work, once. */
  createIdentity(name: string, opts?: MakeKeyOptions): Promise<Me>;
  setName(name: string): Promise<Me>;
  /** Save a small cropped JPEG; null restores the public-key identicon. Shared on the next contribution. */
  setAvatar(image: Uint8Array | null): Promise<Me>;
  setAvatarColor(color: AvatarColor | null): Promise<Me>;
  /** Encrypted with Argon2id + XChaCha20-Poly1305 under a passphrase of at least 15 characters. */
  exportIdentity(passphrase: string): Promise<Uint8Array>;
  importIdentity(data: Uint8Array, passphrase: string): Promise<Me>;
  /** Your recovery line (`hoi-key-1-…`) for a password manager or paper, while this device still
   * keeps it. Whoever holds it can always replace your root and keep your standing. */
  recoveryKey(): Promise<string>;
  /** The line is saved: this device forgets it, so a stolen device can't replace your root. */
  forgetRecoveryKey(): Promise<RecoveryStatus>;
  recoveryStatus(): Promise<RecoveryStatus>;
  /** On a device without a key: brings you back from your recovery line under a new root that
   * takes over from the old one, with a fresh device key. Your standing stays. */
  restoreIdentity(recoveryKey: string, name: string, opts?: Pick<MakeKeyOptions, 'onKey' | 'onTry'>): Promise<Me>;
  /** People in `community` who could keep one share of your recovery line. */
  shareCandidates(community: Hex): Promise<Person[]>;
  /** Splits your recovery line among 3 to 7 friends in `community`; any 3 of them bring it back.
   * Needs the line on this device. */
  giveShares(community: Hex, friends: Hex[]): Promise<RecoveryStatus>;
  /** The people whose share this device keeps. */
  heldShares(): Promise<HeldShare[]>;
  /** On a device without a key: a request for friends to answer with their shares. */
  askFriends(): Promise<RecoveryAsk>;
  /** The digits a friend's request shows, to compare with them before answering. */
  requestDigits(code: string): Promise<string>;
  /** Answers `person`'s request with the share this device keeps for them. */
  answerRequest(person: Hex, code: string): Promise<RecoveryAnswer>;
  /** Takes one friend's answer; the third brings you back. */
  takeAnswer(code: string, name: string): Promise<{ answers: number; me: Me | null }>;
  /** On a new device: returns a code to show/scan on a device that holds your root key. */
  linkRequest(label: string): Promise<string>;
  /** On the root device: bless the new device; returns a code for the new device. */
  linkApprove(request: string): Promise<string>;
  /** On the new device: finish linking. */
  linkComplete(approval: string): Promise<Me>;
  /** On a new device paired with your home node: ask your other devices to link this one, no
   * codes to copy. Resolves once the request is out (`sent`: devices it reached right away);
   * the digits to compare follow in a `linkCheck` event, and a `linked` event follows the yes. */
  linkHome(label: string): Promise<{ sent: number }>;
  /** Like linkHome, for a browser that keeps no signing key: your computer makes it, keeps it
   * and signs for this browser over the home link. */
  linkHomeRemote(label: string): Promise<{ sent: number }>;
  /** On a computer: what its browsers asked to sign that waits for a yes. */
  signAsks(): Promise<SignAsk[]>;
  signAnswer(id: Hex, yes: boolean): Promise<void>;
  linkCancel(): Promise<void>;
  /** Asks waiting here for an answer (this device holds your root key). */
  linkAsks(): Promise<LinkAsk[]>;
  linkAnswer(id: number, yes: boolean): Promise<void>;
  revokeDevice(key: Hex): Promise<Me>;
  /** Names one of your devices, on this device only, to tell them apart. Never sent. */
  setDeviceLabel(key: Hex, label: string): Promise<Me>;
  /** Mines more identity work for your key (a community may ask for more). Every 2 bits take
   * about 4× as long as the last: seconds to minutes. Hosts that can mine it outside the core,
   * report progress and stop on abort, and the rest of the app keeps answering meanwhile. */
  strengthenIdentity(bits: number, opts?: StrengthenOptions): Promise<Me>;
  /** Burns your key for good. `confirm` is `Me.short`. Nothing it signed survives, anywhere. */
  burnIdentity(confirm: string): Promise<void>;

  // communities
  communities(): Promise<CommunitySummary[]>;
  discoverCommunities(): Promise<DiscoveredCommunity[]>;
  /** A public node's link (`hoi://node?…` or `…#/node?…`): its routes are tried first, and the
   * communities it carries show up in `discoverCommunities` once it answered. */
  openNodeLink(link: string): Promise<DirectoryStatus>;
  /** What a node link says before anything is dialed: where it points and the operator's notice
   * in it, verified against the node the link names. A link whose notice was not
   * accepted yet is refused by openNodeLink. */
  previewNodeLink(link: string): Promise<NodeLinkPreview>;
  /** Operators' notices this device met, pending first. */
  nodeNotices(): Promise<NoticeView[]>;
  /** The person read a node's notice (by its hash) and accepts it: the node is talked to again. */
  acceptNotice(node: Hex, hash: Hex): Promise<NoticeView>;
  /** The person declines: the node is left alone until the notice is forgotten. */
  declineNotice(node: Hex): Promise<NoticeView>;
  /** Forgets a notice and the answer to it: the next contact asks again. */
  forgetNotice(node: Hex): Promise<void>;
  communityRegistration(id: Hex): Promise<CommunityRegistration>;
  communityPublication(id: Hex): Promise<CommunityPublication>;
  /** Fetch an open community's signed definition by key from explicitly selected public nodes. */
  publicJoin(id: Hex, peers: string[], name?: string): Promise<PublicJoinStatus>;
  publicJoinStatus(id: Hex): Promise<PublicJoinStatus>;
  /** How safe each public community's records are, from this device's own links. Not a receipt. */
  warmth(): Promise<WarmthView>;
  communityWarmth(id: Hex): Promise<CommunityWarmth>;
  /** Ask the connected public nodes that take requests to keep an open community available.
   * Their answers arrive as `notice` events; Warmth shows what actually arrived. */
  requestCarry(id: Hex): Promise<CarryRequest>;
  community(id: Hex): Promise<CommunityView>;
  createCommunity(draft: CommunityDraft): Promise<CommunityView>;
  createPrivateCommunity(draft: CommunityDraft): Promise<CommunityView>;
  privateGroups(): Promise<PrivateGroup[]>;
  privateGroup(community: Hex): Promise<PrivateGroup>;
  privateJoinRequest(): Promise<string>;
  privatePreviewRequest(request: string): Promise<{ device: Hex; root: Hex }>;
  privateInvite(community: Hex, request: string): Promise<string>;
  privateJoin(invite: string): Promise<CommunityView>;
  privateRemove(community: Hex, device: Hex): Promise<PrivateGroup>;
  /**
   * In a private community whose membership follows its records (`followsRecords`): passes a
   * join request on to the owner's device, vouching for its person where admission is by
   * invitation. The invitation comes back in `PrivateGroup.invitations` once the records admit
   * them.
   */
  privatePassRequest(community: Hex, request: string): Promise<{ device: Hex; root: Hex }>;
  /** For an owner of a members-only community: a private copy whose membership follows its records. */
  privateCopy(community: Hex): Promise<CommunityView>;
  privateOutbox(community: Hex): Promise<PrivateRelayObject[]>;
  privateReceive(community: Hex, body: string): Promise<PrivateGroup>;
  privateAck(community: Hex, ids: string[]): Promise<PrivateGroup>;
  /** Targets are `objects?after=<n>` or `objects/<id>` below the current room; at most 65. */
  privateRead(community: Hex, targets: string[]): Promise<PrivateRelayRead>;
  previewInvite(link: string): Promise<InvitePreview>;
  join(link: string): Promise<CommunityView>;
  leave(id: Hex): Promise<void>;
  setPinning(id: Hex, on: boolean): Promise<CommunityView>;
  invite(id: Hex): Promise<Invite>;
  exportBundle(id: Hex): Promise<Uint8Array>;
  importBundle(data: Uint8Array): Promise<ImportReport>;

  // owners (threshold signatures); rules and limits become proposals the community decides
  setLimits(id: Hex, limits: Limits): Promise<LedgerResult>;
  setRules(id: Hex, text: string): Promise<LedgerResult>;
  addMod(id: Hex, key: Hex): Promise<LedgerResult>;
  removeMod(id: Hex, key: Hex): Promise<LedgerResult>;
  /**
   * Says an elector left for good: after a week's notice their silence stops holding
   * up decisions; void as soon as they sign anything here. Vetoable like other entries.
   */
  retire(id: Hex, key: Hex): Promise<LedgerResult>;
  cosign(proposal: string): Promise<LedgerResult>;

  // mods
  ledger(id: Hex): Promise<LedgerEntryView[]>;
  remove(target: Hex, reason: string): Promise<LedgerEntryView>;
  pin(target: Hex, on: boolean): Promise<LedgerEntryView>;
  label(target: Hex, label: string, reason?: string): Promise<LedgerEntryView>;
  ban(community: Hex, key: Hex, reason: string): Promise<LedgerEntryView>;
  unban(community: Hex, key: Hex, reason: string): Promise<LedgerEntryView>;
  admit(community: Hex, key: Hex): Promise<LedgerEntryView>;

  // the ladder
  /** Where someone (you, without `key`) stands, and what's missing. */
  standing(community: Hex, key?: Hex): Promise<StandingView>;
  /** 1 vouches, -1 objects (with a reason), 0 takes it back. Counts once you're established. */
  vouch(community: Hex, key: Hex, value: -1 | 0 | 1, reason?: HideReason): Promise<StandingView>;
  /** Hide a post or comment for the community (with a reason) or say it's fine; veto a
   * moderator's or owner's action (with a reason) or support it; answer a proposal ('ok'
   * agrees, 'veto' objects with a reason); 'none' takes it back. */
  review(target: Hex, verdict: Verdict, reason?: string): Promise<void>;
  /** Newcomers' posts waiting for your review. Reading without hiding vouches for them. */
  reviewQueue(community: Hex): Promise<PostSummary[]>;
  media(hash: Hex): Promise<MediaBlob>;

  // posts
  feed(q: FeedQuery): Promise<PostSummary[]>;
  post(id: Hex): Promise<PostView>;
  createPost(draft: PostDraft): Promise<PostSummary>;
  reply(parent: Hex, text: string): Promise<CommentView>;
  edit(id: Hex, text: string, title?: string): Promise<void>;
  deleteOwn(id: Hex): Promise<void>;
  deletionProgress(): Promise<DeletionProgress[]>;
  vote(target: Hex, value: -1 | 0 | 1): Promise<number>;
  /** Signed flag for the mods (a `report` record). reason: 'spam' | 'rule:<n>' | 'illegal' */
  flag(target: Hex, reason: string): Promise<void>;
  save(id: Hex, on: boolean): Promise<void>;
  saved(): Promise<PostSummary[]>;
  search(q: string, community?: Hex): Promise<PostSummary[]>;

  // people
  person(key: Hex, community?: Hex): Promise<PersonView>;
  setTrust(key: Hex, friend: boolean): Promise<Person>;
  /** Whether you share whom you trust in a community. */
  trustSharing(community: Hex): Promise<TrustSharing>;
  /** Share whom you trust there, publicly, or take it back: it names your friends known there. */
  shareTrust(community: Hex, on: boolean): Promise<TrustSharing>;
  /** The labelers you follow, and the ones your communities name by default. */
  labelers(): Promise<LabelerView[]>;
  /** Follow someone's labels (they count on this device), or stop. */
  followLabeler(key: Hex, on: boolean): Promise<LabelerView[]>;
  /** Label a post or comment as a labeler: for whoever follows you, or whose community names you. */
  publishLabel(target: Hex, label: string, note?: string): Promise<void>;
  /** A link's preview if this device keeps one; else it is asked for (a `preview` event says when it came) and this is null.
   * Needs previews on, and a desktop app or hoi-node: this device, or a browser's home node. */
  linkPreview(url: string): Promise<LinkPreview | null>;
  /** A community's wiki pages, most recently edited first. */
  wikiPages(community: Hex): Promise<WikiPageSummary[]>;
  /** Age-proof communities: what your wallet binds its presentation to. */
  ageChallenge(community: Hex): Promise<AgeChallenge>;
  /** List keepers whose blocklists you follow. */
  blocklistKeepers(): Promise<BlocklistKeeper[]>;
  followBlocklist(key: Hex, on: boolean): Promise<BlocklistKeeper[]>;
  /** Publish a list in a community, one hash a line (PDQ, or blake3:<hash>): for whoever follows your lists. */
  publishBlocklist(community: Hex, text: string): Promise<number>;
  /** Aggregators whose counts you follow. */
  aggregators(): Promise<AggregatorView[]>;
  followAggregator(key: Hex, on: boolean): Promise<AggregatorView[]>;
  /** Whether this device counts a community's votes and publishes them, every hour. */
  aggregating(community: Hex): Promise<boolean>;
  setAggregating(community: Hex, on: boolean): Promise<boolean>;
  /** Check a wallet's presentation here and make the code to send to a moderator. */
  ageProofCode(community: Hex, presentation: string): Promise<string>;
  /** A moderator: check a newcomer's age proof code and let them in. */
  admitByAge(code: string): Promise<LedgerEntryView>;
  /** One page as its edits made it; a page nobody wrote yet is empty. */
  wikiPage(community: Hex, page: string): Promise<WikiPageView>;
  /** Save a page as `text`: members only; edits made elsewhere meanwhile merge in. */
  editWiki(community: Hex, page: string, text: string): Promise<WikiPageView>;
  setMuted(key: Hex, muted: boolean): Promise<Person>;
  setPetname(key: Hex, petname: string | null): Promise<Person>;

  // node
  stats(): Promise<NodeStats>;
  settings(): Promise<Settings>;
  setSettings(patch: Partial<Settings>): Promise<Settings>;
  net(): Promise<NetStatus>;
  /** Amounts and times of what passed through this device; never content. */
  traffic(): Promise<Traffic>;
  /** Pair with a home node (desktop companion or hoi-node): hoi://pair?u=ws://…&t=…&w=… or its
   * web form https://…/#/pair?… */
  pair(link: string): Promise<NetStatus>;
  /** What a pairing link says before pairing: the links are masked (`#/pair?code=…`). */
  previewPair(link: string): Promise<PairPreview>;
  unpair(): Promise<NetStatus>;
  /** Desktop and hoi-node: a new pairing link; devices paired with the old one pair again. */
  rotatePairing(): Promise<NetStatus>;
  /** Desktop: ask the router to forward this computer's ports without publishing anything yet,
   * so the person sees the public address before turning on `publicNode`; `false` removes the
   * forwards again unless the computer is a public node by then. */
  previewPublicNode(on: boolean): Promise<NetStatus>;
  /** Desktop: the network interfaces that are up, tunnels first, to pick a VPN from. */
  vpnInterfaces(): Promise<VpnInterface[]>;
  /** Load fingerprints of pictures to refuse, near copies included: a PDQ hash list, one
   * 64-hex hash per line. Kept on this device only. Resolves to how many were new. */
  importBlocklist(text: string): Promise<number>;
  /** Delete everything on this device: keys, records, settings. */
  wipe(): Promise<void>;
}
