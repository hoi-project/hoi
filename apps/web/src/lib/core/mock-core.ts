import { toBase64Url } from './rpc-core';
// Dev-only, in-memory HoiCore. No crypto, no network: plausible behaviour so every screen can be exercised.
// - starts with no identity (first run shows); seeds the prototype's world once a key exists
// - emits `stamp` events for posts, replies and votes, `changed` after writes, `net` on pairing
// - owner actions need `threshold` signatures (needs-cosign); simulated co-owners sign a few seconds later
// - role checks: owners change limits/rules/mods; mods remove/pin/label/ban/admit
// URL switches for testing: `?blocked` (first boot says another tab has the store), `?me=<name>` (start with a key),
// `?platform=desktop` (the desktop app: serving, relaying, traffic; it opens as the companion, `?full` as the full app),
// `?ask` (a phone asks to be linked a moment after boot), `?paired` (start paired with a home node).
import {
  DEFAULT_LIMITS,
  DEFAULT_LADDER,
  COMMUNITY_COLORS,
  AVATAR_COLORS,
  HoiError,
  formatRecoveryKey,
  leadingZeroBits,
  now,
  recoveryKeyBody,
  shortId,
  shortKey,
  zbase32Decode,
  zbase32Encode,
} from '@hoi/core-api';
import type {
  AvatarColor,
  Boot,
  InviteAddress,
  LabelerView,
  LinkPreview,
  CommentView,
  CommunityColor,
  CommunitySummary,
  CommunityDraft,
  CommunityView,
  CommunityWarmth,
  LinkWarmth,
  CoreEvent,
  DeviceInfo,
  DeletionProgress,
  BulkDeletionStatus,
  CleanupReadiness,
  FeedQuery,
  FlagView,
  Hex,
  HoiCore,
  ImportReport,
  Invite,
  InvitePreview,
  LedgerAction,
  LedgerEntryView,
  LedgerResult,
  LedgerTarget,
  Limits,
  MediaView,
  Me,
  Napping,
  NetStatus,
  NodeStats,
  Person,
  PersonView,
  PostDraft,
  PostSummary,
  PostView,
  ProposalStatus,
  ProposalView,
  Settings,
  LinkAsk,
  Traffic,
  Standing,
  StandingView,
  TrustLevel,
  Unix,
  Verdict,
  NoticeView,
  NodeLinkPreview,
} from '@hoi/core-api';
import { COMMENTS, COMMUNITIES, CONSENTS, FLAGS, LEDGER, PEOPLE, POSTS, PROPOSALS } from './mock-data';
import { bytesToHex, didFromKey, hexToBytes, parseKey } from '../util';
import { BEYOND_THIS_NODE, OTHER_PARTICIPANTS } from '../notice';

// ---------------------------------------------------------------- internal model

interface MPerson { key: Hex; name: string; avatar?: string; avatarColor?: AvatarColor; trust: TrustLevel; baseTrust: TrustLevel; muted: boolean; petname?: string; firstSeen: Unix }
interface MRemoval { by: Hex | null; reason: string; at: Unix; ledgerId?: Hex }
type MTarget = { kind: 'record'; id: Hex } | { kind: 'key'; key: Hex };
interface MFlag { target: MTarget; reasons: string[]; count: number; weight: number; lastAt: Unix; mine: boolean }
interface MCommunity {
  id: Hex; name: string; description: string; color: CommunityColor; createdAt: Unix;
  joined: boolean; pinning: boolean;
  owners: Hex[]; threshold: number; mods: Hex[];
  limits: Limits;
  rules: { text: string; version: number; at: Unix; signedBy: Hex[] } | null;
  pending: Hex[];
  banned: { key: Hex; reason: string; at: Unix; auto: boolean }[];
  flags: MFlag[];
  people: number;
  seq: number;
  /** subject → voucher → 1 | -1 */
  vouches: Map<Hex, Map<Hex, 1 | -1>>;
  /** record or ledger entry → reviewer → verdict */
  reviews: Map<Hex, Map<Hex, { verdict: Verdict; reason?: string }>>;
  /** Your own rung here (the mock doesn't run the ladder for you). */
  myLevel: Standing;
}
interface MPost {
  id: Hex; community: Hex; author: Hex; title: string; text: string; excerpt?: string; link?: string;
  createdAt: Unix; expiresAt: Unix; score: number; myVote: -1 | 0 | 1; pinned: boolean; napping?: Napping;
  labels: string[]; edited: boolean; saved: boolean; waiting: boolean; removed?: MRemoval;
  receivedFrom: number; stampBits: number; signedBy: 'root' | 'device'; media?: MediaView[];
  friendsUp?: number; fofUp?: number;
}
interface MComment {
  id: Hex; post: Hex; parent: Hex; author: Hex; text: string; createdAt: Unix; expiresAt: Unix;
  score: number; myVote: -1 | 0 | 1; edited: boolean; napping?: Napping; removed?: MRemoval; waiting: boolean;
}
interface MEntry {
  id: Hex; community: Hex; seq: number; at: Unix; action: LedgerAction; by: Hex[]; auto: boolean;
  target?: MTarget; reason?: string; text?: string; limits?: Limits; evidence: number; expiresAt: Unix | null;
  /** Snapshot of the target record, kept when its body is gone. */
  recordTitle?: string; recordAuthor?: Hex;
}
type OwnerAction = 'rules' | 'limits' | 'mod-add' | 'mod-remove' | 'retire';
interface MProposal { code: string; community: Hex; action: OwnerAction; text?: string; limits?: Limits; key?: Hex; signers: Hex[]; need: number }
interface MIdentity { me: Me; passphraseHint?: string }
/** A rules or limits change the community decides (PROTOCOL.md §6.1). Voices are 1 each here. */
interface MConsent {
  id: Hex; community: Hex; kind: 'limits' | 'rules'; at: Unix; deadline: Unix; by: Hex[];
  base: Limits; limits?: Limits; text?: string; prevRules?: MCommunity['rules'];
  electors: Hex[]; away: Set<Hex>; answers: Map<Hex, { verdict: 'ok' | 'veto'; reason?: string }>;
  status: ProposalStatus; decidedAt?: Unix; unanimous: boolean; applied: boolean;
}

// ---------------------------------------------------------------- helpers

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
const jitter = () => sleep(50 + Math.random() * 150);
const DAY = 86_400;
const err = (code: ConstructorParameters<typeof HoiError>[0], message: string) => new HoiError(code, message);
/** Node links opened in the demo: it reaches none of them, it only counts them. */
const nodeLinks = new Set<string>();
/** Operators' notices met: by endpoint key, as the core keeps them. */
const notices = new Map<string, NoticeView>();
/** Nodes that asked on a connection (shown unasked); a link's preview only waits on its page. */
const asked = new Set<string>();
const MOCK_NODE = 'a1'.repeat(32);
/** Where the mock node of a coded link stands. */
const MOCK_PUBLIC_NODE = '203.0.113.9:7844';
function mockNotice(node: string, host?: string): NoticeView {
  return {
    node, hash: 'c0ffee' + node.slice(0, 58), state: 'pending', title: 'Join notice – sleepy-walrus-48213',
    sections: [
      { title: 'Operator', text: 'This node is operated by Lena. Contact: lena at example dot org.' },
      { title: 'Server location', text: 'The server is located in XX and hosted by Example Hosting.' },
      { title: 'What this node processes', text: 'To connect you with communities, this node processes your IP address, connection times and technical connection data. Nothing beyond that: the node keeps the open communities it carries (their posts and media, which are public) and the addresses of the other public nodes it reached.' },
      { title: 'Logging and retention', text: 'IP addresses are logged for 7 days for security and abuse prevention, then deleted.' },
      { title: 'Purpose', text: 'The data is used only to operate this node and the communities it carries. It is not sold or used for advertising.' },
      { title: 'Other participants', text: OTHER_PARTICIPANTS },
      { title: 'Beyond this node', text: BEYOND_THIS_NODE },
      { title: 'Your rights', text: 'You can ask the operator what data about you this node holds and request its deletion via the contact above.' },
    ],
    footer: 'Notice version c0ffee' + node.slice(0, 6) + ', last updated 2026-11-03.',
    name: 'sleepy-walrus-48213', operator: 'Lena', contact: 'lena at example dot org', country: 'XX', provider: 'Example Hosting', ipLogging: true, retentionDays: 7, updated: '2026-11-03',
    host, seen: Math.floor(Date.now() / 1000),
  };
}
/** A node link's code as the mock writes it: opaque, like the core's masked one (which the mock
 * cannot make or read without BLAKE3), and always the mock node. */
const MOCK_NODE_CODE = 'SE9OAW1vY2stbm9kZS1saW5rLWNvZGUtd2l0aC1hLXNpZ25lZC1ub3RpY2U';
/** The node a link names: its `p=` endpoint, or the mock node for links without one. */
function linkNode(q: URLSearchParams): string {
  if (q.get('code')) return MOCK_NODE;
  const p = (q.get('p') ?? '').split(',')[0] ?? '';
  const id = p.split('@')[0];
  return /^[0-9a-f]{64}$/.test(id) ? id : MOCK_NODE;
}
function pendingNotices(): NoticeView[] {
  return [...notices.values()].filter((n) => n.state === 'pending' && asked.has(n.node));
}

function randomHex(bytes = 32): Hex {
  const b = new Uint8Array(bytes);
  crypto.getRandomValues(b);
  return Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
}

/** Stable 64-char hex from a label, optionally starting with a fixed prefix (to match the prototype's short keys). */
function hexFrom(seed: string, prefix = ''): Hex {
  let out = prefix;
  let h = 0x811c9dc5;
  let i = 0;
  while (out.length < 64) {
    for (const ch of `${seed}:${i}`) h = Math.imul(h ^ ch.charCodeAt(0), 0x01000193) >>> 0;
    out += h.toString(16).padStart(8, '0');
    i++;
  }
  return out.slice(0, 64);
}

const enc = new TextEncoder();
const dec = new TextDecoder();
function b64url(s: string): string {
  let bin = '';
  for (const b of enc.encode(s)) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}
function unb64url(s: string): string {
  const bin = atob(s.replace(/-/g, '+').replace(/_/g, '/'));
  return dec.decode(Uint8Array.from(bin, (c) => c.charCodeAt(0)));
}

function urlParam(name: string): string | null {
  if (typeof location === 'undefined') return null;
  return new URLSearchParams(location.search).get(name);
}

function fingerprint(hex: Hex): string {
  return [0, 4, 8, 12].map((i) => hex.slice(i, i + 4)).join(' · ');
}

function excerptOf(text: string): string {
  const first = text.split(/\n\s*\n/)[0].replace(/\s+/g, ' ').trim();
  return first.length > 180 ? first.slice(0, 177).trimEnd() + '…' : first;
}

// ---------------------------------------------------------------- the mock

export function createMockCore(): HoiCore {
  const listeners = new Set<(e: CoreEvent) => void>();
  const emit = (e: CoreEvent) => {
    for (const l of [...listeners]) {
      try {
        l(e);
      } catch (x) {
        console.error(x);
      }
    }
  };
  const changed = (...ids: Hex[]) => queueMicrotask(() => emit({ type: 'changed', communities: ids }));

  let identity: MIdentity | null = null;
  let seeded = false;
  /** Friends the mock's person gave shares to. */
  const mockShares: string[] = [];
  let blockedOnce = urlParam('blocked') !== null;
  const people = new Map<Hex, MPerson>();
  /** Communities where you share whom you trust. */
  const trustShared = new Set<Hex>();
  /** People whose labels you follow. */
  const followedLabelers = new Set<Hex>();
  /** Aggregators followed, and communities this device counts for. */
  const followedAggregators = new Set<Hex>();
  const counting = new Set<Hex>();
  /** List keepers followed. */
  const keepers = new Set<Hex>();
  /** Wiki pages: community → page → (text, when, editors). */
  const wiki = new Map<Hex, Map<string, { text: string; at: Unix; editors: Hex[]; edits: number }>>();
  const communities = new Map<Hex, MCommunity>();
  const posts = new Map<Hex, MPost>();
  const deletions = new Map<Hex, DeletionProgress>();
  const bulk = new Map<Hex, BulkDeletionStatus>();
  const comments = new Map<Hex, MComment>();
  const ledgerEntries = new Map<Hex, MEntry>();
  const proposals = new Map<string, MProposal>();
  const consents = new Map<Hex, MConsent>();
  const blockedPictures = new Set<string>();
  const pendingLinks = new Map<string, { device: Hex; label: string }>();
  const platform: Boot['platform'] = urlParam('platform') === 'desktop' ? 'desktop' : 'web';
  const desktop = platform === 'desktop';
  const MOCK_FP = 'Qm9rLXRoaXMtaXMtYS1tb2NrLWZpbmdlcnByaW50MDA';
  const appBase = () => (typeof location === 'undefined' ? 'https://hoi.sh/app/' : location.origin + location.pathname);
  const MOCK_RTC = [`rtc:${MOCK_FP}@192.168.1.20:7844`];
  const pairQuery = (name: string) => `u=ws%3A%2F%2F127.0.0.1%3A7842&t=mock-token-4f2a&n=${encodeURIComponent(name)}&w=${MOCK_FP}@192.168.1.20:7844`;
  /** A pairing link's code as the mock writes it: opaque like the core's masked one, which the
   * mock can't make or read (no BLAKE3 here); it always means the mock home node. */
  const MOCK_PAIR_CODE = 'SE9QAW1vY2stcGFpcmluZy1saW5rLWNvZGUtZm9yLWEtaG9tZS1ub2Rl';
  const MOCK_PAIR = { u: '', t: 'mock-token-4f2a', n: "your computer", w: `${MOCK_FP}@192.168.1.20:7844` };
  /** A pairing link's fields: from its code (the mock home node) or its clear-text query. */
  function pairFields(link: string): { query: string; u: string; t: string | null; n: string | null; w: string | null } | null {
    const m = link.trim().match(/(?:^hoi:\/\/pair\/?|#\/pair)\?(.+)$/i);
    if (!m) return null;
    const q = new URLSearchParams(m[1]);
    if (q.get('code')) return /^[A-Za-z0-9_-]{27,}$/.test(q.get('code')!) ? { query: m[1], ...MOCK_PAIR } : null;
    return { query: m[1], u: q.get('u') ?? '', t: q.get('t'), n: q.get('n'), w: q.get('w') };
  }
  let settings: Settings = defaultSettings();
  let net: NetStatus = localNet();
  /** A notice was answered or arrived: the network status carries the pending ones. */
  function noticeNet() {
    net = { ...net, pendingNotices: pendingNotices() };
    emit({ type: 'net', status: { ...net } });
  }
  let peerTimer: ReturnType<typeof setInterval> | null = null;
  let bounced = { keys: 23, records: 312 };
  const asks = new Map<number, LinkAsk>();
  let nextAsk = 1;
  let linkWait: ReturnType<typeof setTimeout> | null = null;
  const started = now();

  function defaultSettings(): Settings {
    return {
      relay: desktop,
      dailyCapGb: 5,
      keepModerated: true,
      linkPreviews: false,
      tor: false,
      theme: 'system',
      motion: 'system',
      autoVouch: true,
      autoMedia: true,
      relayNoticeSeen: false,
      postingNoticeSeen: false,
      companion: desktop && urlParam('full') === null,
      publicNode: false,
      holdPiecesGb: 0,
      favorites: [],
      communityOrder: [],
      vpn: '',
      homeNetwork: true,
      lessData: false,
      shareHolders: false,
      directLinks: false,
      assurance: 'declared',
    };
  }
  /** The demo computer has a WireGuard tunnel next to its Wi-Fi. */
  const MOCK_INTERFACES = [
    { name: 'wg0', label: null, tunnel: true, addresses: ['10.2.0.2'] },
    { name: 'en0', label: 'Wi-Fi', tunnel: false, addresses: ['192.168.1.20'] },
  ];
  function vpnNow(): NetStatus['vpn'] {
    if (!settings.vpn) return undefined;
    const i = MOCK_INTERFACES.find((x) => x.name === settings.vpn);
    return i?.tunnel ? { interface: i.name, up: true, addresses: i.addresses } : { interface: settings.vpn, up: false, addresses: [], detail: "doesn't carry this computer's internet traffic (not connected, or split tunnelling)" };
  }
  /** The demo router forwards from a documentation address, after a moment. */
  const MOCK_PUBLIC = '203.0.113.7:7844';
  let previewTimer: ReturnType<typeof setTimeout> | null = null;
  function askRouter() {
    net = { ...net, portMapping: { state: 'asking', public: [], viaVpn: false } };
    emit({ type: 'net', status: { ...net } });
    if (previewTimer) clearTimeout(previewTimer);
    previewTimer = setTimeout(() => {
      net = { ...net, portMapping: { state: 'forwarded', public: [MOCK_PUBLIC], viaVpn: false } };
      if (settings.publicNode) publish();
      emit({ type: 'net', status: { ...net } });
    }, 700);
  }
  function publish() {
    net = {
      ...net,
      rtc: [`rtc:${MOCK_FP}@${MOCK_PUBLIC}`, ...MOCK_RTC],
      nodeLink: `${appBase()}#/node?code=${MOCK_NODE_CODE}`,
      exposure: [
        { address: MOCK_PUBLIC, scope: 'internet', fromRouter: true, told: ['devices', 'strangers', 'node-link'] },
        { address: '192.168.1.20:7844', scope: 'network', fromRouter: false, told: ['devices', 'strangers', 'node-link'] },
      ],
    };
  }
  function unpublish() {
    if (previewTimer) clearTimeout(previewTimer);
    net = { ...net, rtc: desktop ? MOCK_RTC : [], nodeLink: undefined, portMapping: undefined, exposure: desktop ? mockExposure() : [] };
  }
  /** What a paired browser sees on the network map: its home node with every community, a
   * public node and a member's computer with some, and a member's browser with one. */
  function mockLinks(via: string): Pick<NetStatus, 'connections' | 'communities' | 'directory'> {
    const joined = [...communities.values()].filter((c) => c.joined && c.limits.admission === 'open').map((c) => c.id);
    const connections: NonNullable<NetStatus['connections']> = [
      { peer: 'home', role: 'home', route: via, connected: true, transport: via, encryption: via === 'webrtc' ? 'encrypted · WebRTC, DTLS' : 'stays on this device', kind: 'home', keeps: true, incoming: false, communities: joined },
      { peer: 'node-1', role: 'peer', route: 'direct', connected: true, transport: 'webrtc', encryption: 'encrypted · WebRTC, DTLS', kind: 'public', keeps: true, incoming: false, communities: joined.slice(0, 3) },
      { peer: 'member-2', role: 'peer', route: 'direct', connected: true, transport: 'webrtc', encryption: 'encrypted · WebRTC, DTLS', kind: 'node', keeps: true, incoming: false, communities: joined.slice(1, 2) },
      { peer: 'member-3', role: 'peer', route: 'direct', connected: true, transport: 'webrtc', encryption: 'encrypted · WebRTC, DTLS', kind: 'browser', keeps: false, incoming: false, communities: joined.slice(0, 1) },
    ];
    return {
      connections,
      communities: joined.map((c) => ({ community: c, connectedPeers: connections.filter((x) => x.communities?.includes(c)).length, lastSync: now() - 40 })),
      directory: { known: 7, waiting: 2, connected: 1, listings: 46 },
    };
  }
  function mockExposure(): NetStatus['exposure'] {
    return [
      { address: '192.168.1.20:7843', scope: 'network', fromRouter: false, told: ['devices'] },
      { address: '192.168.1.20:7844', scope: 'network', fromRouter: false, told: ['devices', 'strangers'] },
    ];
  }
  function localNet(): NetStatus {
    const n: NetStatus = {
      mode: 'local', label: desktop ? 'only on this computer' : 'only in this browser', peers: 0, rtc: desktop ? MOCK_RTC : [], relaying: desktop && settings.relay,
      directory: { known: 0, waiting: nodeLinks.size, connected: 0 },
      exposure: desktop ? mockExposure() : [],
    };
    if (desktop) {
      n.serving = {
        url: 'ws://127.0.0.1:7842',
        pairing: `hoi://pair?code=${MOCK_PAIR_CODE}`,
        pairingWeb: `${appBase()}#/pair?code=${MOCK_PAIR_CODE}`,
        phoneCanReach: true,
        clients: 0,
      };
    }
    return n;
  }

  /** A plausible hour of traffic: quiet minutes, a few bursts, more on the desktop. */
  function mockTraffic(): Traffic {
    const t = now();
    const minute = t - (t % 60);
    const scale = desktop ? 180_000 : 40_000;
    const minutes = Array.from({ length: 60 }, (_, i) => {
      const at = minute - (59 - i) * 60;
      const wave = Math.max(0, Math.sin(at / 420) + Math.sin(at / 97) * 0.6);
      const burst = (at / 60) % 17 === 3 ? 6 : 1;
      return { at, bytesIn: Math.round(wave * scale * burst * 0.6), bytesOut: Math.round(wave * scale * burst) };
    });
    const connected = net.mode !== 'local' || desktop;
    const bytesIn = minutes.reduce((a, m) => a + m.bytesIn, 0);
    const bytesOut = minutes.reduce((a, m) => a + m.bytesOut, 0);
    const connections: Traffic['connections'] = [];
    if (net.homeNode?.connected) {
      connections.push({ peer: 'home', role: 'home', name: net.homeNode.name, transport: net.homeNode.via ?? 'websocket', encryption: net.homeNode.via === 'webrtc' ? 'encrypted · WebRTC, DTLS' : 'stays on this device', bytesIn, bytesOut, recordsIn: 412, recordsOut: 37, since: started, lastAt: t - 4 });
    }
    if (desktop) {
      connections.push(
        { peer: 'client-rtc-1', role: 'client', name: `${identity?.me.name ?? 'lena'}'s phone`, transport: 'webrtc', encryption: 'encrypted · WebRTC, DTLS', bytesIn: Math.round(bytesIn * 0.2), bytesOut: Math.round(bytesOut * 0.4), recordsIn: 12, recordsOut: 388, since: started + 120, lastAt: t - 9 },
        { peer: 'peer-2', role: 'member', transport: 'quic', encryption: 'encrypted · QUIC, TLS 1.3', bytesIn: Math.round(bytesIn * 0.7), bytesOut: Math.round(bytesOut * 0.5), recordsIn: 220, recordsOut: 140, since: started + 30, lastAt: t - 21 },
        { peer: 'rtc-3', role: 'member', transport: 'webrtc', encryption: 'encrypted · WebRTC, DTLS', bytesIn: Math.round(bytesIn * 0.1), bytesOut: Math.round(bytesOut * 0.1), recordsIn: 3, recordsOut: 96, since: started + 600, lastAt: t - 40 },
      );
    }
    const cap = settings.dailyCapGb * 1e9;
    return {
      since: started,
      now: t,
      bytesIn: connected ? bytesIn : 0,
      bytesOut: connected ? bytesOut : 0,
      recordsIn: connected ? 635 : 0,
      recordsOut: connected ? 661 : 0,
      mediaIn: connected ? 2_400_000 : 0,
      mediaOut: connected ? 5_100_000 : 0,
      refused: connected ? 14 : 0,
      minutes: connected ? minutes : minutes.map((m) => ({ ...m, bytesIn: 0, bytesOut: 0 })),
      connections: connected ? connections : [],
      relay: { on: desktop && settings.relay, possible: desktop, capBytes: cap, todayBytes: desktop ? 812_000_000 : 0, napping: false },
    };
  }

  function emitAsk(label: string) {
    const ask: LinkAsk = { id: nextAsk++, label, check: String(1000 + Math.floor(Math.random() * 9000)), device: randomHex(), at: now() };
    asks.set(ask.id, ask);
    emit({ type: 'linkAsk', ask });
  }

  // ------------------------------------------------------------ identity

  const meKey = (): Hex => {
    if (!identity) throw err('no-identity', 'Make a key first.');
    return identity.me.rootKey;
  };
  const isMe = (key: Hex) => identity?.me.rootKey === key;

  function makeMe(name: string, rootKey = randomHex(), hasRoot = true): Me {
    const t = now();
    const device = randomHex();
    const devices: DeviceInfo[] = [
      { key: device, short: shortKey(device), label: 'This browser', thisDevice: true, expiresAt: t + 180 * DAY, revoked: false, signedHere: false },
    ];
    return {
      root: didFromKey(rootKey),
      rootKey,
      workKey: rootKey,
      fingerprint: fingerprint(rootKey),
      short: shortKey(rootKey),
      name,
      createdAt: t,
      hasRoot,
      device,
      devices,
      identityBits: 10,
      burned: false,
      holdsLine: hasRoot,
      restoring: false,
      replaced: false,
      remote: false,
    };
  }

  function setIdentity(me: Me) {
    identity = { me };
    people.set(me.rootKey, { key: me.rootKey, name: me.name, avatar: me.avatar, avatarColor: me.avatarColor, trust: 'me', baseTrust: 'me', muted: false, firstSeen: me.createdAt });
    if (!seeded) seed();
  }

  // ------------------------------------------------------------ seeding

  function seed() {
    seeded = true;
    const T = now();
    const me = identity!.me;
    const keyOf = new Map<string, Hex>([['me', me.rootKey]]);
    for (const p of PEOPLE) {
      const key = hexFrom(`person:${p.id}`, p.prefix);
      keyOf.set(p.id, key);
      people.set(key, { key, name: p.name, trust: p.trust, baseTrust: p.trust, muted: false, firstSeen: T - p.ageDays * DAY });
    }
    const k = (id: string) => keyOf.get(id)!;
    const cid = (id: string) => hexFrom(`community:${id}`);
    const rid = (id: string) => hexFrom(`record:${id}`);
    const lid = (id: string) => hexFrom(`ledger:${id}`);
    const target = (t: { record: string } | { key: string }): MTarget =>
      'record' in t ? { kind: 'record', id: rid(t.record) } : { kind: 'key', key: k(t.key) };

    // A phone, so "Revoke" has something to do.
    const phone = hexFrom('device:phone', '9b7e');
    me.devices.push({ key: phone, short: shortKey(phone), label: 'Phone', thisDevice: false, expiresAt: T + 150 * DAY, revoked: false, signedHere: false });

    for (const c of COMMUNITIES) {
      const limits: Limits = { ...DEFAULT_LIMITS, ...c.limits };
      communities.set(cid(c.id), {
        id: cid(c.id), name: c.name, description: c.description, color: c.color, createdAt: T - c.sinceDays * DAY,
        joined: true, pinning: c.pinning,
        owners: c.owners.map(k), threshold: c.threshold, mods: c.mods.map(k), limits,
        rules: c.rules ? { text: c.rules.text, version: c.rules.version, at: T - c.rules.hoursAgo * 3600, signedBy: c.rules.signedBy.map(k) } : null,
        pending: (c.pending ?? []).map(k),
        banned: (c.banned ?? []).map((b) => ({ key: k(b.key), reason: b.reason, at: T - b.hoursAgo * 3600, auto: b.auto })),
        flags: [],
        people: c.people,
        seq: 1200,
        vouches: new Map(),
        reviews: new Map(),
        myLevel: 'established',
      });
    }
    for (const p of POSTS) {
      const c = communities.get(cid(p.c))!;
      const createdAt = T - Math.round(p.hoursAgo * 3600);
      posts.set(rid(p.id), {
        id: rid(p.id), community: c.id, author: k(p.by), title: p.title, text: p.text, excerpt: p.excerpt, link: p.link,
        createdAt, expiresAt: createdAt + c.limits.maxRetention, score: p.score, myVote: 0, pinned: !!p.pinned,
        napping: p.napping, labels: p.labels ?? [], edited: false, saved: false, waiting: !!p.waiting,
        friendsUp: p.friendsUp, fofUp: p.fofUp,
        receivedFrom: p.receivedFrom ?? 8 + Math.floor(Math.random() * 50), stampBits: c.limits.powBits + (p.hoursAgo % 3 === 0 ? 1 : 0),
        signedBy: Math.random() < 0.5 ? 'root' : 'device',
      });
    }
    for (const cm of COMMENTS) {
      const post = posts.get(rid(cm.post))!;
      const c = communities.get(post.community)!;
      const createdAt = T - cm.minutesAgo * 60;
      comments.set(rid(cm.id), {
        id: rid(cm.id), post: post.id, parent: cm.parent ? rid(cm.parent) : post.id, author: k(cm.by), text: cm.removed ? '' : cm.text,
        createdAt, expiresAt: createdAt + c.limits.maxRetention, score: cm.score, myVote: 0, edited: false,
        napping: cm.napping, waiting: false,
        removed: cm.removed ? { by: k(cm.removed.by), reason: cm.removed.reason, at: T - 35 * 60, ledgerId: lid(cm.removed.ledger) } : undefined,
      });
    }
    let seq = 1201;
    for (const e of LEDGER) {
      const c = communities.get(cid(e.c))!;
      const t = e.target ? target(e.target) : undefined;
      const at = T - Math.round(e.hoursAgo * 3600);
      const entry: MEntry = {
        id: lid(e.id), community: c.id, seq: seq++, at, action: e.action, by: e.by.map(k), auto: !!e.auto,
        target: t, reason: e.reason, evidence: e.evidence ?? 0,
        text: e.action === 'rules' ? c.rules?.text : undefined,
        expiresAt: keptUntilSuperseded(e.action) ? null : at + c.limits.maxRetention,
      };
      snapshotTarget(entry);
      ledgerEntries.set(entry.id, entry);
      c.seq = Math.max(c.seq, entry.seq);
    }
    for (const f of FLAGS) {
      const c = communities.get(cid(f.c))!;
      c.flags.push({ target: target(f.target), reasons: f.reasons, count: f.count, weight: f.weight, lastAt: T - Math.round(f.hoursAgo * 3600), mine: false });
    }
    for (const p of PROPOSALS) {
      const c = communities.get(cid(p.c))!;
      proposals.set(p.code, { code: p.code, community: c.id, action: 'limits', limits: { ...c.limits, ...p.patch }, signers: p.signers.map(k), need: p.need });
    }
    for (const p of CONSENTS) {
      const c = communities.get(cid(p.c))!;
      const at = T - Math.round(p.hoursAgo * 3600);
      const limits: Limits = { ...c.limits, ...p.patch, ladder: { ...c.limits.ladder, ...p.ladder } };
      const entry: MEntry = { id: lid(p.id), community: c.id, seq: ++c.seq, at, action: 'limits', by: p.by.map(k), auto: false, limits, evidence: 0, expiresAt: null };
      ledgerEntries.set(entry.id, entry);
      const consent = startConsent(c, entry, { kind: 'limits', limits });
      consent.at = at;
      consent.deadline = at + c.limits.maxRetention;
      for (const a of p.agreed) consent.answers.set(k(a), { verdict: 'ok' });
      evaluateConsent(c, consent);
    }
    // One honest notice a little later, like a real node would raise.
    setTimeout(() => {
      if (!identity) return;
      bounced = { keys: bounced.keys + 1, records: bounced.records + 14 };
      emit({ type: 'notice', text: 'Bounced 14 records from f30a·c812' });
    }, 9000);
  }

  const keptUntilSuperseded = (a: LedgerAction) => a === 'rules' || a === 'limits' || a === 'mod-add' || a === 'mod-remove' || a === 'retire';

  function snapshotTarget(e: MEntry) {
    if (e.target?.kind !== 'record') return;
    const p = posts.get(e.target.id);
    const cm = comments.get(e.target.id);
    if (p) {
      e.recordTitle = p.title;
      e.recordAuthor = p.author;
    } else if (cm) {
      e.recordTitle = posts.get(cm.post)?.title;
      e.recordAuthor = cm.author;
    }
  }

  // ------------------------------------------------------------ views

  function roleIn(c: MCommunity, key: Hex): 'owner' | 'mod' | 'member' {
    if (c.owners.includes(key)) return 'owner';
    if (c.mods.includes(key)) return 'mod';
    return 'member';
  }
  const myRole = (c: MCommunity) => (identity ? roleIn(c, identity.me.rootKey) : 'member');
  const canModerate = (c: MCommunity) => myRole(c) !== 'member';

  // ------------------------------------------------------------ the ladder (simplified)

  const RANK: Standing[] = ['newcomer', 'member', 'established', 'trusted'];
  const atLeast = (a: Standing, b: Standing) => RANK.indexOf(a) >= RANK.indexOf(b);
  const CAPS: Record<Standing, StandingView['capabilities']> = {
    newcomer: ['text'],
    member: ['text', 'links', 'vote'],
    established: ['text', 'links', 'vote', 'review', 'vouch', 'veto'],
    trusted: ['text', 'links', 'vote', 'review', 'vouch', 'veto', 'media'],
  };
  function vouchersOf(c: MCommunity, key: Hex): Hex[] {
    return [...(c.vouches.get(key)?.entries() ?? [])].filter(([, v]) => v > 0).map(([k]) => k);
  }
  function levelIn(c: MCommunity, key: Hex): Standing {
    if (c.owners.includes(key) || c.mods.includes(key)) return 'trusted';
    if (isMe(key)) return c.myLevel;
    const base: Standing = { me: 'trusted', friend: 'trusted', fof: 'established', member: 'established', newcomer: 'newcomer' }[people.get(key)?.baseTrust ?? 'newcomer'] as Standing;
    if (base === 'newcomer' && vouchersOf(c, key).length >= Math.min(c.limits.ladder.admitVouches, establishedIn(c))) return 'member';
    return base;
  }
  function establishedIn(c: MCommunity): number {
    const keys = new Set<Hex>([...c.owners, ...c.mods]);
    for (const p of posts.values()) if (p.community === c.id) keys.add(p.author);
    return [...keys].filter((k) => atLeast(levelIn0(c, k), 'established')).length;
  }
  // levelIn without the vouch bump (keeps establishedIn from recursing)
  function levelIn0(c: MCommunity, key: Hex): Standing {
    if (c.owners.includes(key) || c.mods.includes(key)) return 'trusted';
    if (isMe(key)) return c.myLevel;
    return ({ me: 'trusted', friend: 'trusted', fof: 'established', member: 'established', newcomer: 'newcomer' }[people.get(key)?.baseTrust ?? 'newcomer'] as Standing);
  }
  const quorum = (q: number, est: number) => Math.min(Math.max(q, 2), Math.max(2, Math.ceil(est / 3)));
  const myVerdict = (c: MCommunity, id: Hex) => (identity ? c.reviews.get(id)?.get(identity.me.rootKey) : undefined);
  function standingOf(c: MCommunity, key: Hex): StandingView {
    const level = levelIn(c, key);
    const vouchers = vouchersOf(c, key);
    const need = Math.min(c.limits.ladder.admitVouches, Math.max(1, establishedIn(c)));
    const objections = [...(c.vouches.get(key)?.values() ?? [])].filter((v) => v < 0).length;
    const mine = identity ? c.vouches.get(key)?.get(identity.me.rootKey) ?? 0 : 0;
    const contributions = [...posts.values()].filter((p) => p.community === c.id && p.author === key).length;
    const score = [...posts.values()].filter((p) => p.community === c.id && p.author === key).reduce((n, p) => n + p.score, 0);
    const next: StandingView['next'] =
      level === 'newcomer'
        ? { level: 'member', missing: [`vouches from established members: ${vouchers.length} of ${need}`] }
        : level === 'member'
          ? { level: 'established', missing: [`time as a member: ${Math.ceil(c.limits.ladder.establishedAge / DAY)} more days`] }
          : level === 'established'
            ? { level: 'trusted', missing: [`posts or comments in the last 90 days: ${contributions} of ${c.limits.ladder.trustedPosts}`, `response from the community: ${score} of ${c.limits.ladder.mediaScore}`].filter((_, i) => (i === 0 ? contributions < c.limits.ladder.trustedPosts : score < c.limits.ladder.mediaScore)) }
            : null;
    return {
      community: c.id, person: personOf(key, c), level, capabilities: [...CAPS[level]],
      admittedAt: atLeast(level, 'member') ? c.createdAt : null,
      establishedAt: atLeast(level, 'established') ? c.createdAt : null,
      vouchedBy: vouchers.map((k) => personOf(k, c)), vouchesNeeded: need, objections,
      contributions, contributionsNeeded: c.limits.ladder.trustedPosts, score, scoreNeeded: c.limits.ladder.mediaScore,
      blocked: c.banned.some((b) => b.key === key), struck: false, myVouch: mine as -1 | 0 | 1,
      voice: voiceIn(c, key, level),
      vouchedThisQuarter: [...c.vouches.values()].filter((m) => m.get(key) === 1).length,
      identityBits: isMe(key) ? identity!.me.identityBits : 10, identityNeeded: c.limits.ladder.identityBits, next,
    };
  }
  /** The mock's voices: founders whole, friends of friends half, everyone else a third. */
  function voiceIn(c: MCommunity, key: Hex, level: Standing): number {
    if (c.owners.includes(key) || c.mods.includes(key)) return 1;
    if (level === 'newcomer') return 0;
    if (isMe(key)) return 0.5;
    return ({ me: 1, friend: 1, fof: 0.5, member: 1 / 3, newcomer: 1 / 3 } as Record<TrustLevel, number>)[people.get(key)?.baseTrust ?? 'newcomer'];
  }
  function reviewQueueOf(c: MCommunity): MPost[] {
    if (!identity || !atLeast(levelIn(c, identity.me.rootKey), 'established')) return [];
    return [...posts.values()].filter((p) => p.community === c.id && alive(p) && !p.removed && !isMe(p.author) && levelIn(c, p.author) === 'newcomer' && !myVerdict(c, p.id));
  }
  const media = new Map<Hex, { mime: MediaView['mime']; data: Uint8Array }>();
  function sniff(b: Uint8Array): MediaView['mime'] | null {
    const at = (s: string, o = 0) => [...s].every((ch, i) => b[o + i] === ch.charCodeAt(0));
    if (b[0] === 0x89 && at('PNG', 1)) return 'image/png';
    if (b[0] === 0xff && b[1] === 0xd8) return 'image/jpeg';
    if (at('GIF8')) return 'image/gif';
    if (at('RIFF') && at('WEBP', 8)) return 'image/webp';
    if (at('OggS')) return 'audio/ogg';
    if (at('ftyp', 4)) return 'video/mp4';
    if (b[0] === 0x1a && b[1] === 0x45) return 'video/webm';
    return null;
  }

  function personOf(key: Hex, community?: MCommunity): Person {
    let p = people.get(key);
    if (!p) {
      p = { key, name: '', trust: 'newcomer', baseTrust: 'newcomer', muted: false, firstSeen: now() };
      people.set(key, p);
    }
    const mine = isMe(key);
    const hint = mine ? identity!.me.name : p.name || undefined;
    const out: Person = {
      key,
      short: shortKey(key),
      name: p.petname || hint || shortKey(key),
      nameHint: hint,
      avatar: p.avatar,
      avatarColor: p.avatarColor,
      petname: p.petname,
      trust: mine ? 'me' : p.trust,
      muted: p.muted,
      keyAgeDays: Math.max(0, Math.floor((now() - p.firstSeen) / DAY)),
    };
    if (community) {
      const r = roleIn(community, key);
      if (r !== 'member') out.role = r;
      out.banned = community.banned.some((b) => b.key === key);
      out.standing = levelIn(community, key);
    }
    return out;
  }

  const ref = (c: MCommunity) => ({ id: c.id, name: c.name, color: c.color });
  const alive = (r: { expiresAt: Unix }) => r.expiresAt > now();

  function commentsOf(postId: Hex): MComment[] {
    return [...comments.values()].filter((c) => c.post === postId && alive(c));
  }

  function summaryOfPost(p: MPost): PostSummary {
    const c = communities.get(p.community)!;
    return {
      id: p.id,
      short: shortId(p.id),
      community: ref(c),
      author: personOf(p.author, c),
      title: p.title,
      excerpt: p.excerpt ?? excerptOf(p.text),
      link: p.link,
      createdAt: p.createdAt,
      expiresAt: p.expiresAt,
      score: p.score,
      myVote: p.myVote,
      friendsUp: p.friendsUp ?? 0,
      fofUp: p.fofUp ?? 0,
      replies: commentsOf(p.id).filter((x) => !x.removed).length,
      pinned: p.pinned,
      napping: p.napping ?? (myVerdict(c, p.id)?.verdict === 'hide' ? { label: 'you hid this', by: 'you' } : undefined),
      labels: [...p.labels],
      edited: p.edited,
      mine: isMe(p.author),
      saved: p.saved,
      waiting: p.waiting,
      unvalidated: levelIn(c, p.author) === 'newcomer',
      media: (p.media ?? []).map((m) => ({ ...m, available: media.has(m.hash) })),
    };
  }

  function removalView(r: MRemoval | undefined, c: MCommunity) {
    if (!r) return undefined;
    return { by: r.by ? personOf(r.by, c) : null, reason: r.reason, at: r.at, ledgerId: r.ledgerId };
  }

  function commentTree(post: MPost): CommentView[] {
    const c = communities.get(post.community)!;
    const all = commentsOf(post.id).filter((x) => !x.waiting || canModerate(c) || isMe(x.author));
    const byParent = new Map<Hex, MComment[]>();
    for (const x of all) {
      if (people.get(x.author)?.muted) continue;
      const list = byParent.get(x.parent) ?? [];
      list.push(x);
      byParent.set(x.parent, list);
    }
    const rank = (x: MComment) => {
      const t = isMe(x.author) ? 'me' : people.get(x.author)?.trust;
      return t === 'me' || t === 'friend' ? 0 : 1; // friends first
    };
    const build = (parent: Hex): CommentView[] =>
      (byParent.get(parent) ?? [])
        .sort((a, b) => rank(a) - rank(b) || b.score - a.score || a.createdAt - b.createdAt)
        .map((x) => ({
          id: x.id,
          short: shortId(x.id),
          parent: x.parent,
          author: personOf(x.author, c),
          text: x.removed ? '' : x.text,
          createdAt: x.createdAt,
          expiresAt: x.expiresAt,
          score: x.score,
          myVote: x.myVote,
          mine: isMe(x.author),
          edited: x.edited,
          napping: x.napping ?? (myVerdict(c, x.id)?.verdict === 'hide' ? { label: 'you hid this', by: 'you' } : undefined),
          removed: removalView(x.removed, c),
          waiting: x.waiting,
          unvalidated: levelIn(c, x.author) === 'newcomer',
          children: build(x.id),
        }));
    return build(post.id);
  }

  function communitySummary(c: MCommunity): CommunitySummary {
    const ps = [...posts.values()].filter((p) => p.community === c.id && alive(p) && !p.removed);
    return {
      id: c.id, name: c.name, color: c.color, description: c.description, createdAt: c.createdAt,
      joined: c.joined, pinning: c.pinning, role: myRole(c),
      posts: ps.length,
      lastActivity: ps.reduce((m, p) => Math.max(m, p.createdAt), c.createdAt),
    };
  }

  /** A plausible Warmth reading (PROTOCOL.md §20) from the mock's connection state: alone it is
   * cold, whatever this device is; a home node or other peers make it cool, warm or hot, since
   * only copies besides this device's own count. Varied a little per community so the list has
   * something to sort. */
  /** Every mock link in a place of its own, all guarded: the arithmetic of hoi_sync::warmth::assess_links. */
  function mockLinkWarmth(n: number): LinkWarmth {
    if (n <= 0) return { level: 'cold', score: 0, links: 0, places: 0, effective: 0, largest: 0, guarded: 0 };
    const level = n === 1 ? 'cool' : n >= 3 ? 'hot' : 'warm';
    const raw = Math.round(100 * (0.6 * Math.min(1, n / 4) + 0.25 + 0.15 * (1 - 1 / n)));
    const score = level === 'cool' ? Math.min(raw, 49) : level === 'warm' ? Math.min(Math.max(raw, 50), 79) : Math.max(raw, 80);
    return { level, score, links: n, places: n, effective: n, largest: Math.round(100 / n) / 100, guarded: 1 };
  }

  function mockWarmth(c: MCommunity): CommunityWarmth {
    const connected = net.peers > 0;
    const seed = parseInt(c.id.slice(0, 2), 16);
    const holdersDurable = connected ? (net.homeNode?.connected ? 1 : 0) + (seed % 3) : 0;
    const holdersTransient = connected ? seed % 2 : 0;
    const others = holdersDurable;
    // Every third community's nodes share one network: they count as one place.
    const shared = others > 1 && seed % 3 === 1;
    const kept = shared ? 1 : others;
    const places = shared ? 1 : others;
    const spread = others > 0 ? Math.round((kept / others) * 100) / 100 : 0;
    const guarded = others > 0 ? 1 : 0;
    const durableCopies = (desktop ? 1 : 0) + others;
    const copies = 1 + holdersDurable + holdersTransient;
    const complete = connected && seed % 5 === 0 ? 0.8 : 1;
    const fresh = connected;
    let level: CommunityWarmth['level'] = kept < 1 ? 'cold' : kept < 2 ? 'cool' : kept < 3 || complete < 0.95 ? 'warm' : 'hot';
    if (complete < 0.5 || !fresh) level = level === 'hot' ? 'warm' : level === 'warm' ? 'cool' : 'cold';
    const records = [...posts.values()].filter((p) => p.community === c.id && alive(p)).length + 1;
    return {
      community: c.id, name: c.name, level, kept, places, spread, guarded,
      score: Math.round(100 * (0.6 * Math.min(1, kept / 4) + 0.1 * spread + 0.1 * guarded + 0.12 * complete + (fresh ? 0.08 : 0))),
      copies, durableCopies, networkCopies: Math.max(others, connected && seed % 4 === 0 ? 3 : 0),
      complete, media: 1, fresh, live: connected, records, missing: Math.round(records * (1 - complete)),
      holdersDurable, holdersTransient, remembered: connected ? seed % 4 : 0, ownDurable: desktop,
      lastSync: connected ? now() - 90 : undefined, pending: false,
    };
  }

  function flagView(c: MCommunity, f: MFlag): FlagView {
    return { target: targetView(c, f.target), reasons: [...f.reasons], count: f.count, weight: Math.round(f.weight * 100) / 100, lastAt: f.lastAt };
  }

  function communityView(c: MCommunity): CommunityView {
    const mod = canModerate(c);
    const cs = [...comments.values()].filter((x) => posts.get(x.post)?.community === c.id && alive(x));
    const ps = [...posts.values()].filter((p) => p.community === c.id && alive(p));
    const bytes = ps.reduce((n, p) => n + 900 + p.text.length * 2, 0) + cs.reduce((n, x) => n + 600 + x.text.length * 2, 0);
    return {
      ...communitySummary(c),
      owners: c.owners.map((k) => personOf(k, c)),
      threshold: c.threshold,
      mods: c.mods.map((k) => personOf(k, c)),
      limits: { ...c.limits, labelers: [...c.limits.labelers], trackers: [...(c.limits.trackers ?? [])], ageIssuers: [...(c.limits.ageIssuers ?? [])] },
      trackers: [...(c.limits.trackers ?? [])],
      rules: c.rules ? { text: c.rules.text, version: c.rules.version, at: c.rules.at, signedBy: c.rules.signedBy.map((k) => personOf(k, c)) } : null,
      stats: {
        posts: ps.filter((p) => !p.removed).length,
        comments: cs.filter((x) => !x.removed).length,
        people: c.people,
        storedBytes: bytes,
        ledgerEntries: [...ledgerEntries.values()].filter((e) => e.community === c.id).length,
      },
      pending: mod ? c.pending.map((k) => personOf(k, c)) : [],
      banned: c.banned.map((b) => ({ person: personOf(b.key, c), reason: b.reason, at: b.at, auto: b.auto })),
      flags: mod ? [...c.flags].sort((a, b) => b.lastAt - a.lastAt).map((f) => flagView(c, f)) : [],
      established: establishedIn(c),
      establishedVoice: establishedIn(c),
      me: identity ? standingOf(c, identity.me.rootKey) : null,
      reviewQueue: reviewQueueOf(c).length,
      proposals: [...consents.values()]
        .filter((p) => p.community === c.id && (p.status === 'open' || p.status === 'in-effect' || (p.decidedAt ?? 0) + 30 * DAY > now()))
        .sort((a, b) => b.at - a.at)
        .map((p) => proposalView(c, p)),
    };
  }

  function targetView(c: MCommunity, t: MTarget, e?: MEntry): LedgerTarget {
    if (t.kind === 'key') return { kind: 'key', person: personOf(t.key, c) };
    const p = posts.get(t.id);
    const cm = comments.get(t.id);
    const author = e?.recordAuthor ?? p?.author ?? cm?.author;
    const title = e?.recordTitle ?? (p ? p.title : cm ? posts.get(cm.post)?.title : undefined);
    return { kind: 'record', id: t.id, short: shortId(t.id), title: title || undefined, author: author ? personOf(author, c) : undefined };
  }

  const VETOABLE: LedgerAction[] = ['remove', 'ban', 'unban', 'label', 'pin', 'admit', 'mod-add'];
  function entryView(e: MEntry): LedgerEntryView {
    const c = communities.get(e.community)!;
    const rs = [...(c.reviews.get(e.id)?.values() ?? [])];
    const vetoes = rs.filter((r) => r.verdict === 'veto').length;
    const support = rs.filter((r) => r.verdict === 'ok').length;
    const need = quorum(c.limits.ladder.vetoQuorum, establishedIn(c));
    const mine = myVerdict(c, e.id)?.verdict;
    return {
      id: e.id, seq: e.seq, at: e.at, action: e.action, by: e.by.map((k) => personOf(k, c)), auto: e.auto,
      target: e.target ? targetView(c, e.target, e) : undefined,
      reason: e.reason, text: e.text, limits: e.limits ? { ...e.limits } : undefined,
      evidence: e.evidence, expiresAt: e.expiresAt,
      vetoes, vetoesVoice: vetoes, vetoesNeeded: need, support, overturned: vetoes >= need && vetoes > support, vetoable: VETOABLE.includes(e.action),
      proposal: consents.get(e.id)?.status,
      myReview: mine === 'veto' || mine === 'ok' ? mine : undefined,
      pendingUntil: e.action === 'remove' && e.at + c.limits.ladder.contestWindow > now() ? e.at + c.limits.ladder.contestWindow : undefined,
    };
  }

  // ------------------------------------------------------------ lookups and checks

  const c0 = (id: Hex) => getCommunity(id);
  function getCommunity(id: Hex): MCommunity {
    const c = communities.get(id);
    if (!c) throw err('not-found', 'No community with that key on this device.');
    return c;
  }
  function getRecord(id: Hex): { post?: MPost; comment?: MComment; community: MCommunity } {
    const post = posts.get(id);
    if (post && alive(post)) return { post, community: communities.get(post.community)! };
    const comment = comments.get(id);
    if (comment && alive(comment)) return { comment, community: communities.get(posts.get(comment.post)!.community)! };
    throw err('not-found', "That isn't on this device. It may have faded.");
  }
  function authorDelete(id: Hex) {
    const me = meKey();
    const r = getRecord(id);
    const rec = r.post ?? r.comment!;
    if (rec.author !== me) throw err('not-allowed', 'Only the author can delete this.');
    rec.removed = { by: null, reason: 'deleted by the author', at: now() };
    rec.text = '';
    deletions.set(id, {
      id, target: id, localRemoved: true, recordSubmitted: false,
      queued: 0, submitted: 0, acknowledged: 0, legacyRemoval: 0, unknown: 0,
      refused: 0, unreachable: 0, expired: 0, outstanding: 0, coverageGaps: 0,
      expiresAt: rec.expiresAt, requiresLocalState: true, inventoryComplete: false,
    });
    changed(r.community.id);
  }
  /** The mock's known posts and comments: everything of mine here that is not deleted yet. */
  function bulkInventory() {
    const me = meKey();
    const busy = new Set([...bulk.values()].flatMap(b => b.targets.map(t => t.id)));
    const open = (r: { id: Hex; author: Hex; expiresAt: Unix }) => r.author === me && alive(r) && !deletions.has(r.id) && !busy.has(r.id);
    const byCommunity = new Map<Hex, { id: Hex; name: string; posts: Hex[]; comments: Hex[] }>();
    const entry = (c: Hex) => {
      let e = byCommunity.get(c);
      if (!e) byCommunity.set(c, e = { id: c, name: communities.get(c)?.name ?? '', posts: [], comments: [] });
      return e;
    };
    for (const p of posts.values()) if (open(p)) entry(p.community).posts.push(p.id);
    for (const c of comments.values()) if (open(c)) entry(posts.get(c.post)!.community).comments.push(c.id);
    const ids = [...byCommunity.values()].flatMap(c => [...c.posts, ...c.comments]).sort((a, b) => a.localeCompare(b));
    let h = 0x811c9dc5;
    let snapshot = '';
    for (let round = 0; round < 8; round++) {
      for (const ch of `${me}|${ids.join(',')}|${round}`) h = Math.imul(h ^ ch.charCodeAt(0), 0x01000193) >>> 0;
      snapshot += h.toString(16).padStart(8, '0');
    }
    return { communities: [...byCommunity.values()], ids, snapshot };
  }
  /** RFC-0003 in the mock: deletions here stay in effect until their target's lifetime ends. */
  function cleanupReadiness(): CleanupReadiness {
    const t = now();
    const inEffect = [...deletions.values()].filter(d => d.requiresLocalState && d.expiresAt > t);
    const unsigned = [...bulk.values()].flatMap(b => b.targets).filter(x => x.state === 'selected' || x.state === 'queued');
    const blocked: CleanupReadiness['blocked'] = [];
    if (unsigned.length) blocked.push({ reason: 'unsigned', count: unsigned.length });
    if (inEffect.length) blocked.push({ reason: 'in-effect', count: inEffect.length });
    const until = Math.max(0, ...inEffect.map(d => d.expiresAt));
    const inv = identity ? bulkInventory() : { ids: [], snapshot: '' };
    const revision = (text: string) => {
      let h = 0x811c9dc5;
      let out = '';
      for (let round = 0; round < 8; round++) {
        for (const ch of `${text}|${round}`) h = Math.imul(h ^ ch.charCodeAt(0), 0x01000193) >>> 0;
        out += h.toString(16).padStart(8, '0');
      }
      return out;
    };
    return {
      ready: blocked.length === 0,
      blocked,
      until: until || null,
      unlisted: inv.ids.length,
      inventory: revision(`${inv.snapshot}|${[...bulk.values()].map(b => b.snapshot + b.targets.map(x => x.state).join()).join()}`),
      deletions: revision([...deletions.values()].map(d => `${d.id}:${d.requiresLocalState && d.expiresAt > t}`).join()),
    };
  }
  function needOwner(c: MCommunity) {
    meKey();
    if (myRole(c) !== 'owner') throw err('not-allowed', 'Only owners can change this.');
  }
  function needMod(c: MCommunity) {
    meKey();
    if (!canModerate(c)) throw err('not-allowed', 'Only mods can do that here.');
  }
  function needKey(key: Hex): Hex {
    if (!/^[0-9a-f]{64}$/.test(key)) throw err('invalid', "That isn't a key. Paste 64 hex characters or a did:key.");
    return key;
  }
  function needReason(reason: string): string {
    const r = reason.trim();
    if (!r) throw err('invalid', 'Say why. The reason goes in the public ledger.');
    if (r.length > 200) throw err('too-big', 'Keep the reason under 200 characters.');
    return r;
  }

  function appendLedger(c: MCommunity, e: Omit<MEntry, 'id' | 'seq' | 'community' | 'at' | 'expiresAt' | 'evidence' | 'auto'> & { evidence?: number; auto?: boolean }): MEntry {
    const at = now();
    c.seq += 1;
    const entry: MEntry = {
      id: randomHex(), community: c.id, seq: c.seq, at, evidence: e.evidence ?? 0, auto: e.auto ?? false,
      expiresAt: keptUntilSuperseded(e.action) ? null : at + c.limits.maxRetention,
      ...e,
    };
    snapshotTarget(entry);
    ledgerEntries.set(entry.id, entry);
    return entry;
  }

  async function stamp(bits: number, small: boolean): Promise<{ bits: number; tookMs: number }> {
    const est = small ? 60 + Math.random() * 80 : (900 + Math.random() * 500) * 2 ** (bits - 22);
    const estimateMs = Math.round(Math.min(small ? 200 : 6000, Math.max(small ? 50 : 300, est)));
    emit({ type: 'stamp', phase: 'start', bits, estimateMs });
    const tookMs = Math.round(estimateMs * (0.85 + Math.random() * 0.3));
    await sleep(tookMs);
    emit({ type: 'stamp', phase: 'done', bits, estimateMs, tookMs });
    return { bits: bits + (Math.random() < 0.3 ? 1 : 0), tookMs };
  }

  // ------------------------------------------------------------ changes by consent

  /** Who decides a change: everyone established here when it's proposed. */
  function electorsOf(c: MCommunity): Hex[] {
    const keys = new Set<Hex>([...c.owners, ...c.mods]);
    for (const p of posts.values()) if (p.community === c.id && alive(p)) keys.add(p.author);
    if (identity) keys.add(identity.me.rootKey);
    return [...keys].filter((key) => atLeast(levelIn(c, key), 'established'));
  }
  function aroundIn(c: MCommunity, key: Hex): boolean {
    if (isMe(key)) return true;
    return [...posts.values()].some((p) => p.community === c.id && p.author === key && alive(p) && !p.removed && !p.napping)
      || [...comments.values()].some((x) => x.author === key && alive(x) && !x.removed && posts.get(x.post)?.community === c.id);
  }
  function startConsent(c: MCommunity, entry: MEntry, what: { kind: 'limits' | 'rules'; limits?: Limits; text?: string }): MConsent {
    const electors = electorsOf(c);
    const consent: MConsent = {
      id: entry.id, community: c.id, kind: what.kind, at: entry.at, deadline: entry.at + c.limits.maxRetention, by: [...entry.by],
      base: { ...c.limits, ladder: { ...c.limits.ladder }, labelers: [...c.limits.labelers] }, limits: what.limits, text: what.text,
      electors, away: new Set(electors.filter((x) => !aroundIn(c, x))), answers: new Map(),
      status: 'open', unanimous: false, applied: false,
    };
    consents.set(consent.id, consent);
    return consent;
  }
  /** `cur` with the fields `want` changes relative to `base` taken from `want`. */
  function patchLimits(base: Limits, want: Limits, cur: Limits): Limits {
    const out: Limits = { ...cur, ladder: { ...cur.ladder }, labelers: [...cur.labelers] };
    for (const f of Object.keys(want) as (keyof Limits)[]) {
      if (f === 'ladder' || f === 'labelers' || f === 'trackers') continue;
      if (base[f] !== want[f]) (out as unknown as Record<string, unknown>)[f] = want[f];
    }
    if (base.labelers.join() !== want.labelers.join()) out.labelers = [...want.labelers];
    if ((base.trackers ?? []).join() !== (want.trackers ?? []).join()) out.trackers = [...(want.trackers ?? [])];
    for (const f of Object.keys(want.ladder) as (keyof Limits['ladder'])[]) if (base.ladder[f] !== want.ladder[f]) out.ladder[f] = want.ladder[f];
    return out;
  }
  const snake = (s: string) => s.replace(/[A-Z]/g, (m) => `_${m.toLowerCase()}`);
  function changesOf(p: MConsent): ProposalView['changes'] {
    if (!p.limits) return [];
    const out: ProposalView['changes'] = [];
    for (const f of Object.keys(p.limits) as (keyof Limits)[]) {
      if (f === 'ladder' || f === 'labelers' || f === 'trackers') continue;
      if (p.base[f] !== p.limits[f]) out.push({ field: snake(f), from: String(p.base[f]), to: String(p.limits[f]) });
    }
    const list = (t?: string[]) => (t?.length ? t.join(', ') : 'none');
    if (list(p.base.trackers) !== list(p.limits.trackers)) out.push({ field: 'trackers', from: list(p.base.trackers), to: list(p.limits.trackers) });
    for (const f of Object.keys(p.limits.ladder) as (keyof Limits['ladder'])[]) {
      if (p.base.ladder[f] !== p.limits.ladder[f]) out.push({ field: snake(f), from: String(p.base.ladder[f]), to: String(p.limits.ladder[f]) });
    }
    return out;
  }
  /** Electors the owners retired here; in the mock it counts at once, without the week's notice. */
  const retiredIn = new Map<Hex, Set<Hex>>();
  const isRetired = (c: MCommunity, x: Hex) => retiredIn.get(c.id)?.has(x) ?? false;
  function evaluateConsent(c: MCommunity, p: MConsent) {
    if (p.status === 'passed' || p.status === 'lapsed') return;
    const agreed = (x: Hex) => p.by.includes(x) || p.answers.get(x)?.verdict === 'ok';
    const objecting = p.electors.filter((x) => p.answers.get(x)?.verdict === 'veto');
    const waiting = p.electors.filter((x) => !agreed(x) && !p.answers.has(x) && !p.away.has(x));
    let status: ProposalStatus;
    if (p.electors.every((x) => agreed(x) || (!p.answers.has(x) && isRetired(c, x)))) {
      status = 'passed';
      p.unanimous = true;
      p.decidedAt = now();
    } else if (now() >= p.deadline) {
      status = objecting.length ? 'lapsed' : 'passed';
      p.decidedAt = p.deadline;
    } else status = objecting.length + waiting.length === 0 ? 'in-effect' : 'open';
    const inForce = status === 'passed' || status === 'in-effect';
    if (inForce && !p.applied) {
      if (p.limits) c.limits = patchLimits(p.base, p.limits, c.limits);
      if (p.text !== undefined) {
        p.prevRules = c.rules;
        c.rules = { text: p.text, version: (c.rules?.version ?? 0) + 1, at: now(), signedBy: [...p.by] };
      }
      p.applied = true;
    } else if (!inForce && p.applied) {
      if (p.limits) c.limits = patchLimits(p.limits, p.base, c.limits);
      if (p.text !== undefined) c.rules = p.prevRules ?? null;
      p.applied = false;
    }
    p.status = status;
  }
  function proposalView(c: MCommunity, p: MConsent): ProposalView {
    evaluateConsent(c, p);
    const me = identity?.me.rootKey;
    const agreed = p.electors.filter((x) => p.by.includes(x) || p.answers.get(x)?.verdict === 'ok');
    const objecting = p.electors.filter((x) => p.answers.get(x)?.verdict === 'veto');
    const waiting = p.electors.filter((x) => !agreed.includes(x) && !p.answers.has(x) && !p.away.has(x));
    const silentAway = p.electors.filter((x) => !agreed.includes(x) && !p.answers.has(x) && p.away.has(x));
    const mine = me ? (p.by.includes(me) ? 'ok' : p.answers.get(me)?.verdict) : undefined;
    const open = p.status === 'open' || p.status === 'in-effect';
    const owner = !!me && c.owners.includes(me);
    return {
      id: p.id, kind: p.kind, at: p.at, deadline: p.deadline, status: p.status, decidedAt: p.decidedAt, unanimous: p.unanimous,
      by: p.by.map((x) => personOf(x, c)), changes: changesOf(p), text: p.text,
      electors: p.electors.length,
      agreed: agreed.map((x) => personOf(x, c)),
      objecting: objecting.map((x) => ({ person: personOf(x, c), reason: p.answers.get(x)?.reason ?? '' })),
      waiting: waiting.map((x) => personOf(x, c)),
      away: silentAway.filter((x) => !isRetired(c, x)).length,
      retired: silentAway.filter((x) => isRetired(c, x)).length,
      retirable: open && owner ? silentAway.filter((x) => !isRetired(c, x) && !c.owners.includes(x)).map((x) => personOf(x, c)) : undefined,
      agreedVoice: agreed.length, objectingVoice: objecting.length, waitingVoice: waiting.length,
      myAnswer: mine === 'ok' ? 'agree' : mine === 'veto' ? 'object' : undefined,
      canAnswer: open && !!me && p.electors.includes(me) && !p.by.includes(me),
    };
  }
  /** In the mock, the other people deciding answer within a few seconds. */
  function simulateElectors(c: MCommunity, p: MConsent) {
    const others = p.electors.filter((x) => !p.by.includes(x) && !isMe(x) && !p.away.has(x));
    others.forEach((x, i) => {
      setTimeout(() => {
        if (p.status !== 'open' && p.status !== 'in-effect') return;
        p.answers.set(x, { verdict: 'ok' });
        evaluateConsent(c, p);
        if ((p.status as ProposalStatus) === 'passed') emit({ type: 'notice', text: `Everyone agreed: the change to c/${c.name}'s ${p.kind} is in effect.` });
        changed(c.id);
      }, 4000 * (i + 1));
    });
  }

  // ------------------------------------------------------------ owner actions (threshold)

  function applyOwnerAction(c: MCommunity, p: Pick<MProposal, 'action' | 'text' | 'limits' | 'key' | 'signers'>): MEntry {
    switch (p.action) {
      case 'limits': {
        // a proposal: the community decides
        const entry = appendLedger(c, { action: 'limits', by: p.signers, limits: { ...p.limits! } });
        const consent = startConsent(c, entry, { kind: 'limits', limits: { ...p.limits!, ladder: { ...p.limits!.ladder } } });
        evaluateConsent(c, consent);
        simulateElectors(c, consent);
        return entry;
      }
      case 'rules': {
        const entry = appendLedger(c, { action: 'rules', by: p.signers, text: p.text, reason: `Rules v${(c.rules?.version ?? 0) + 1}` });
        const consent = startConsent(c, entry, { kind: 'rules', text: p.text });
        evaluateConsent(c, consent);
        simulateElectors(c, consent);
        return entry;
      }
      case 'mod-add':
        if (!c.mods.includes(p.key!)) c.mods.push(p.key!);
        return appendLedger(c, { action: 'mod-add', by: p.signers, target: { kind: 'key', key: p.key! }, reason: `Welcomed ${personOf(p.key!).name} as a moderator` });
      case 'mod-remove':
        c.mods = c.mods.filter((k) => k !== p.key);
        return appendLedger(c, { action: 'mod-remove', by: p.signers, target: { kind: 'key', key: p.key! }, reason: `${personOf(p.key!).name} stepped down` });
      case 'retire': {
        if (!retiredIn.has(c.id)) retiredIn.set(c.id, new Set());
        retiredIn.get(c.id)!.add(p.key!);
        const entry = appendLedger(c, { action: 'retire', by: p.signers, target: { kind: 'key', key: p.key! }, reason: `${personOf(p.key!).name} left for good` });
        for (const q of consents.values()) if (q.community === c.id) evaluateConsent(c, q);
        return entry;
      }
    }
  }

  function ownerAction(c: MCommunity, p: Pick<MProposal, 'action' | 'text' | 'limits' | 'key'>): LedgerResult {
    needOwner(c);
    const signers = [meKey()];
    if (c.threshold <= 1) {
      const entry = applyOwnerAction(c, { ...p, signers });
      changed(c.id);
      return { status: 'published', entry: entryView(entry) };
    }
    const code = `hoi://cosign?p=${randomHex(8)}`;
    const proposal: MProposal = { code, community: c.id, ...p, signers, need: c.threshold };
    proposals.set(code, proposal);
    simulateCoOwners(proposal);
    return { status: 'needs-cosign', proposal: code, have: 1, need: c.threshold };
  }

  function forgetProposal(p: MProposal) {
    for (const [code, q] of proposals) if (q === p) proposals.delete(code);
  }

  /** In the mock, the other owners are quick: each signs a few seconds apart. */
  function simulateCoOwners(p: MProposal) {
    const c = communities.get(p.community)!;
    const others = c.owners.filter((k) => !p.signers.includes(k) && !isMe(k));
    others.slice(0, p.need - p.signers.length).forEach((k, i) => {
      setTimeout(() => {
        if (![...proposals.values()].includes(p) || p.signers.includes(k)) return;
        p.signers.push(k);
        const who = personOf(k).name;
        if (p.signers.length >= p.need) {
          forgetProposal(p);
          applyOwnerAction(c, p);
          emit({ type: 'notice', text: `${who} cosigned. ${p.signers.length} of ${c.owners.length} signed: published in c/${c.name}.` });
          changed(c.id);
        } else {
          emit({ type: 'notice', text: `${who} cosigned your proposal for c/${c.name} (${p.signers.length} of ${p.need}).` });
        }
      }, 6000 * (i + 1));
    });
  }

  // ------------------------------------------------------------ feed

  function visiblePost(p: MPost, c: MCommunity): boolean {
    if (!alive(p) || p.removed) return false;
    if (people.get(p.author)?.muted) return false;
    if (p.waiting && !canModerate(c) && !isMe(p.author)) return false;
    return true;
  }

  function sortPosts(list: MPost[], sort: FeedQuery['sort']): MPost[] {
    const t = now();
    const hot = (p: MPost) => p.score / Math.pow((t - p.createdAt) / 3600 + 2, 1.5);
    switch (sort) {
      case 'fresh': return list.sort((a, b) => b.createdAt - a.createdAt);
      case 'loudest': return list.sort((a, b) => b.score - a.score);
      case 'friends':
        return list
          .filter((p) => isMe(p.author) || people.get(p.author)?.trust === 'friend')
          .sort((a, b) => b.createdAt - a.createdAt);
      default: return list.sort((a, b) => hot(b) - hot(a));
    }
  }

  // ------------------------------------------------------------ invites and bundles

  interface Genesis { n: string; d: string; k: CommunityColor; l: Limits; o: Hex[]; t: number; at: Unix }
  function genesisOf(c: MCommunity): Genesis {
    return { n: c.name, d: c.description, k: c.color, l: c.limits, o: c.owners, t: c.threshold, at: c.createdAt };
  }
  function parseInvite(link: string): { id: Hex; genesis: Genesis | null; relays: string[]; peers: string[] } {
    const s = link.trim();
    const m = s.match(/^(?:web\+)?hoi:\/\/join\/?\?(.+)$/i) ?? s.match(/#\/join\?(.+)$/) ?? s.match(/^\??(c=.+)$/);
    if (!m) throw err('bad-invite', "That isn't a hoi invite. Invites start with hoi://join? or end in #/join?…");
    const q = new URLSearchParams(m[1]);
    const id = q.get('c') ?? '';
    if (!/^[0-9a-f]{64}$/.test(id)) throw err('bad-invite', "The invite doesn't name a community key.");
    let genesis: Genesis | null = null;
    const g = q.get('g');
    if (g) {
      try {
        genesis = JSON.parse(unb64url(g)) as Genesis;
        if (typeof genesis.n !== 'string' || !COMMUNITY_COLORS.includes(genesis.k)) throw new Error('shape');
      } catch {
        throw err('bad-invite', "The invite is damaged: the community's signed details don't check out.");
      }
    }
    if (!genesis && !communities.has(id)) throw err('bad-invite', "This invite doesn't carry enough to find the community. Ask for a fresh one.");
    const split = (v: string | null) => (v ? v.split(',').filter(Boolean) : []);
    return { id, genesis, relays: split(q.get('r')), peers: split(q.get('p')) };
  }

  const BUNDLE_MAGIC = 'HOIBUNDLE1\n';
  const KEY_MAGIC = 'HOIKEY1\n';

  // ------------------------------------------------------------ net

  function startPeerWobble() {
    if (peerTimer) return;
    peerTimer = setInterval(() => {
      if (net.mode === 'local') return;
      const peers = Math.max(6, Math.min(18, net.peers + (Math.random() < 0.5 ? -1 : 1)));
      net = { ...net, peers };
      emit({ type: 'net', status: { ...net } });
    }, 8000);
  }

  // ------------------------------------------------------------ the API

  const core: HoiCore = {
    async boot(): Promise<Boot> {
      await jitter();
      const auto = urlParam('me');
      if (auto && !identity) setIdentity(makeMe(auto.slice(0, 32)));
      const b: Boot = { platform, version: '0.1.0-mock', protocol: 1, hasIdentity: !!identity, cleaned: 0, preservedRecords: 0 };
      if (blockedOnce) b.blockedByOtherTab = true;
      if (urlParam('paired') !== null && !net.homeNode) await core.pair(`hoi://pair?${pairQuery("lena's computer")}`);
      if (urlParam('ask') !== null) setTimeout(() => emitAsk('Pixel 9'), 2500);
      // `?notice`: a public node this browser met asks to be read first, a moment after boot.
      if (urlParam('notice') !== null && !notices.has(MOCK_NODE)) setTimeout(() => { notices.set(MOCK_NODE, mockNotice(MOCK_NODE, '203.0.113.9:7844')); asked.add(MOCK_NODE); emit({ type: 'nodeNotice', node: MOCK_NODE }); noticeNet(); }, 1500);
      net = { ...net, ...(desktop ? { serving: localNet().serving } : {}) };
      return b;
    },
    async takeOver(): Promise<Boot> {
      await jitter();
      blockedOnce = false;
      return { platform, version: '0.1.0-mock', protocol: 1, hasIdentity: !!identity, cleaned: 0, preservedRecords: 0 };
    },
    on(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },

    // -------------------------------------------------------- you
    async me() {
      await jitter();
      return identity ? structuredClone(identity.me) : null;
    },
    async createIdentity(name, opts) {
      await sleep(250 + Math.random() * 200);
      const n = name.trim();
      if (!n) throw err('invalid', 'Pick a name first. Any name.');
      if (n.length > 32) throw err('too-big', 'Keep the name under 32 characters.');
      if (identity) throw err('invalid', 'There is already a key on this device.');
      const me = makeMe(n);
      opts?.onKey?.({ root: me.root, rootKey: me.rootKey, fingerprint: me.fingerprint, short: me.short, bits: me.identityBits });
      if (opts?.onTry) {
        // Plausible tries, no Argon2id: random hashes short of the work, then one that carries it.
        const tries = 40 + Math.floor(Math.random() * 40);
        for (let i = 0; i < tries; i++) {
          await sleep(30);
          const hash = new Uint8Array(32);
          crypto.getRandomValues(hash);
          if (i < tries - 1) hash[0] ||= 1;
          else hash.set([0, hash[1] & 0x1f]);
          opts.onTry({ work: i, hash, bits: leadingZeroBits(hash) });
        }
      }
      setIdentity(me);
      return structuredClone(identity!.me);
    },
    async setName(name) {
      await jitter();
      meKey();
      const n = name.trim();
      if (!n) throw err('invalid', 'A name needs at least one character.');
      if (n.length > 32) throw err('too-big', 'Keep the name under 32 characters.');
      identity!.me.name = n;
      people.get(identity!.me.rootKey)!.name = n;
      changed();
      return structuredClone(identity!.me);
    },
    async setAvatar(image) {
      meKey();
      if (image && (image.length > 4096 || image[0] !== 255 || image[1] !== 216)) throw err('invalid', 'Choose a small JPEG profile picture.');
      const avatar = image?.length ? toBase64Url(image) : undefined;
      identity!.me.avatar = avatar;
      people.get(identity!.me.rootKey)!.avatar = avatar;
      changed();
      return structuredClone(identity!.me);
    },
    async setAvatarColor(color) {
      meKey();
      if (color !== null && !AVATAR_COLORS.includes(color)) throw err('invalid', 'Choose a supported identicon color.');
      identity!.me.avatarColor = color ?? undefined;
      people.get(identity!.me.rootKey)!.avatarColor = color ?? undefined;
      changed();
      return structuredClone(identity!.me);
    },
    async exportIdentity(passphrase) {
      await sleep(400);
      meKey();
      if ([...passphrase].length < 15) throw err('invalid', 'Use a backup passphrase of at least 15 characters. Several unrelated words work well.');
      const body = JSON.stringify({ me: identity!.me, p: passphrase });
      return enc.encode(KEY_MAGIC + b64url(body));
    },
    async recoveryKey() {
      await jitter();
      meKey();
      if (!identity!.me.holdsLine) throw err('not-allowed', 'This device no longer keeps your recovery key: it is in your password manager, on paper, or with your friends.');
      if (identity!.me.burned) throw err('not-allowed', 'This key is burned. There is nothing left to back up.');
      // The mock has no secrets: the public key bytes stand in for the root secret, with a fixed check.
      const payload = new Uint8Array(35);
      payload.set(hexToBytes(identity!.me.rootKey));
      payload.set([0x68, 0x6f, 0x69], 32);
      return formatRecoveryKey(zbase32Encode(payload));
    },
    async forgetRecoveryKey() {
      await jitter();
      meKey();
      identity!.me.holdsLine = false;
      changed();
      return { holdsLine: false, restoring: false, rotations: 0, friends: [...mockShares] };
    },
    async recoveryStatus() {
      await jitter();
      meKey();
      return { holdsLine: identity!.me.holdsLine, restoring: identity!.me.restoring, rotations: 0, friends: [...mockShares] };
    },
    async shareCandidates(community) {
      await jitter();
      meKey();
      const keys = new Set<string>();
      for (const p of posts.values()) if (p.community === community && p.author !== identity!.me.rootKey) keys.add(p.author);
      return [...keys].slice(0, 64).map((k) => personOf(k));
    },
    async giveShares(community, friends) {
      await sleep(200);
      meKey();
      if (!identity!.me.holdsLine) throw err('not-allowed', 'This device no longer keeps your recovery key. Give shares from the line: bring it back here first.');
      if (new Set(friends).size !== friends.length || friends.includes(identity!.me.rootKey) || friends.length < 3 || friends.length > 7) {
        throw err('invalid', 'Pick 3 to 7 different people other than you.');
      }
      void community;
      mockShares.splice(0, mockShares.length, ...friends);
      return { holdsLine: true, restoring: false, rotations: 0, friends: [...mockShares] };
    },
    async heldShares() {
      await jitter();
      meKey();
      return [];
    },
    async askFriends() {
      await jitter();
      if (identity) throw err('not-allowed', 'This device already has a key.');
      return { code: `hoi-ask-1-${randomHex().slice(0, 43)}`, digits: '314 159' };
    },
    async requestDigits(code) {
      await jitter();
      if (!code.trim().startsWith('hoi-ask-1-')) throw err('invalid', "That isn't a recovery request. It starts with hoi-ask-1-.");
      return '314 159';
    },
    async answerRequest() {
      await jitter();
      meKey();
      throw err('not-found', 'This device keeps no share for that person.');
    },
    async takeAnswer(code) {
      await jitter();
      if (!code.trim().startsWith('hoi-share-1-')) throw err('invalid', "That isn't a friend's answer. It starts with hoi-share-1-.");
      return { answers: 1, me: null };
    },
    async restoreIdentity(recoveryKey, name, opts) {
      await sleep(250);
      if (identity) throw err('not-allowed', 'This device already has a key. Wipe it first to bring another one.');
      const n = name.trim();
      if (!n) throw err('invalid', 'Pick a name first. Any name.');
      if (n.length > 32) throw err('too-big', 'Keep the name under 32 characters.');
      const body = recoveryKeyBody(recoveryKey);
      if (!body) throw err('invalid', "That isn't a hoi recovery key. It starts with hoi-key-1-.");
      const payload = zbase32Decode(body);
      if (!payload || payload.length !== 35 || payload[32] !== 0x68 || payload[33] !== 0x6f || payload[34] !== 0x69) {
        throw err('bad-passphrase', 'That recovery key has a typo or is damaged. Check it against your password manager and try again.');
      }
      const me = makeMe(n, bytesToHex(payload.subarray(0, 32)), true);
      opts?.onKey?.({ root: me.root, rootKey: me.rootKey, fingerprint: me.fingerprint, short: me.short, bits: me.identityBits });
      if (opts?.onTry) {
        for (let i = 0; i < 20; i++) {
          await sleep(30);
          const hash = new Uint8Array(32);
          crypto.getRandomValues(hash);
          if (i < 19) hash[0] ||= 1;
          else hash.set([0, hash[1] & 0x1f]);
          opts.onTry({ work: i, hash, bits: leadingZeroBits(hash) });
        }
      }
      setIdentity(me);
      changed();
      return structuredClone(identity!.me);
    },
    async importIdentity(data, passphrase) {
      await sleep(400);
      const text = dec.decode(data);
      if (!text.startsWith(KEY_MAGIC)) throw err('invalid', "That file isn't a hoi key (.hoikey).");
      let parsed: { me: Me; p: string };
      try {
        parsed = JSON.parse(unb64url(text.slice(KEY_MAGIC.length)));
      } catch {
        throw err('invalid', 'That key file is damaged.');
      }
      if (parsed.p !== passphrase) throw err('bad-passphrase', "That passphrase doesn't open this key.");
      const me = makeMe(parsed.me.name, parsed.me.rootKey, true);
      if (identity && identity.me.rootKey !== me.rootKey) {
        people.delete(identity.me.rootKey);
      }
      setIdentity(me);
      changed();
      return structuredClone(me);
    },
    async linkHomeRemote(label) {
      return this.linkHome(label);
    },
    async signAsks() {
      await jitter();
      return [];
    },
    async signAnswer() {
      await jitter();
      throw err('not-found', 'Nothing waits for a yes here.');
    },
    async linkHome(label) {
      await jitter();
      if (identity) throw err('not-allowed', 'This device already has a key.');
      if (!net.homeNode) throw err('not-allowed', 'Pair with your home node first: scan its code or open its pairing link.');
      const check = String(1000 + Math.floor(Math.random() * 9000));
      if (linkWait) clearTimeout(linkWait);
      // Your other device sees the request, then says yes a few seconds later.
      linkWait = setTimeout(() => {
        emit({ type: 'linkCheck', checks: [check] });
        linkWait = setTimeout(() => {
          linkWait = null;
          const me = makeMe('lena', randomHex(), false);
          me.devices[0].label = label.trim() || 'Phone';
          setIdentity(me);
          emit({ type: 'linked', me: structuredClone(me) });
          emit({ type: 'notice', text: `Linked. This device signs as ${me.name} now.` });
          changed();
        }, 3400);
      }, 600);
      return { sent: 1 };
    },
    async linkCancel() {
      if (linkWait) clearTimeout(linkWait);
      linkWait = null;
    },
    async linkAsks() {
      await jitter();
      return [...asks.values()];
    },
    async linkAnswer(id, yes) {
      await sleep(300);
      const a = asks.get(id);
      if (!a) throw err('not-found', 'Not found: link request (it may have timed out)');
      asks.delete(id);
      if (yes && identity) {
        identity.me.devices.push({ key: a.device, short: shortKey(a.device), label: a.label, thisDevice: false, expiresAt: now() + 180 * DAY, revoked: false, signedHere: false });
        changed();
      }
    },
    async linkRequest(label) {
      await jitter();
      const device = randomHex();
      const code = `hoi-link-req:${b64url(JSON.stringify({ d: device, l: label.trim() || 'New device' }))}`;
      pendingLinks.set(device, { device, label: label.trim() || 'New device' });
      return code;
    },
    async linkApprove(request) {
      await sleep(300);
      meKey();
      if (!identity!.me.hasRoot) throw err('not-allowed', "This device doesn't hold your root key, so it can't bless another one.");
      const m = request.trim().match(/^hoi-link-req:(.+)$/);
      if (!m) throw err('invalid', "That isn't a link request. It starts with hoi-link-req:");
      let req: { d: Hex; l: string };
      try {
        req = JSON.parse(unb64url(m[1]));
      } catch {
        throw err('invalid', 'That link request is damaged.');
      }
      const me = identity!.me;
      if (!me.devices.some((d) => d.key === req.d)) {
        me.devices.push({ key: req.d, short: shortKey(req.d), label: req.l, thisDevice: false, expiresAt: now() + 180 * DAY, revoked: false, signedHere: false });
      }
      changed();
      return `hoi-link-ok:${b64url(JSON.stringify({ r: me.rootKey, n: me.name, d: req.d, l: req.l }))}`;
    },
    async linkComplete(approval) {
      await sleep(300);
      const m = approval.trim().match(/^hoi-link-ok:(.+)$/);
      if (!m) throw err('invalid', "That isn't an approval code. It starts with hoi-link-ok:");
      let ok: { r: Hex; n: string; d: Hex; l: string };
      try {
        ok = JSON.parse(unb64url(m[1]));
      } catch {
        throw err('invalid', 'That approval code is damaged.');
      }
      if (!pendingLinks.has(ok.d)) throw err('invalid', 'That approval is for a different device.');
      pendingLinks.delete(ok.d);
      if (identity && identity.me.rootKey === ok.r) return structuredClone(identity.me);
      if (identity) throw err('invalid', 'This device already has a different key. Wipe it first.');
      const me = makeMe(ok.n, ok.r, false);
      me.devices = [{ key: ok.d, short: shortKey(ok.d), label: ok.l, thisDevice: true, expiresAt: now() + 180 * DAY, revoked: false, signedHere: false }];
      me.device = ok.d;
      setIdentity(me);
      changed();
      return structuredClone(me);
    },
    async strengthenIdentity(bits, opts) {
      meKey();
      // Seeded data: a few seconds of pretend tries, reported and stoppable like the real thing.
      const steps = 8 + bits;
      for (let i = 1; i <= steps; i++) {
        if (opts?.signal?.aborted) throw new DOMException('The work was stopped.', 'AbortError');
        await sleep(250);
        opts?.onProgress?.({ tries: i * 37, best: Math.min(bits, identity!.me.identityBits + Math.floor((i / steps) * 3)) });
      }
      identity!.me.identityBits = Math.max(identity!.me.identityBits, bits);
      return { ...identity!.me };
    },
    async burnIdentity(confirm) {
      meKey();
      if (confirm.trim() !== identity!.me.short) throw err('invalid', "Type your key's short form to confirm. Burning can't be undone.");
      await sleep(300);
      identity!.me.burned = true;
      changed(...communities.keys());
    },
    async setDeviceLabel(key, label) {
      await jitter();
      meKey();
      const me = identity!.me;
      const d = me.devices.find((x) => x.key === key);
      if (!d) throw err('not-found', 'No such device.');
      const clean = label.split(/\s+/).filter(Boolean).join(' ');
      if (!clean || [...clean].length > 32) throw err('invalid', 'A name is 1 to 32 characters.');
      d.label = clean;
      changed();
      return structuredClone(me);
    },
    async revokeDevice(key) {
      await jitter();
      meKey();
      const me = identity!.me;
      const d = me.devices.find((x) => x.key === key);
      if (!d) throw err('not-found', 'No such device.');
      if (d.thisDevice) throw err('not-allowed', "You can't revoke the device you're using. Do it from another one.");
      if (!me.hasRoot) throw err('not-allowed', 'Only a device with your root key can revoke others.');
      d.revoked = true;
      changed();
      return structuredClone(me);
    },

    // -------------------------------------------------------- communities
    async communities() {
      await jitter();
      if (!identity) return [];
      return [...communities.values()].map(communitySummary).sort((a, b) => a.createdAt - b.createdAt);
    },
    async openNodeLink(link: string) {
      await jitter();
      if (!/(^hoi:\/\/|#\/)node\?(code=[A-Za-z0-9_-]{27,}|.*[A-Za-z0-9_-]{43}@)/.test(link.trim())) throw err('invalid', "That isn't a node link. A node link starts with hoi://node? or has #/node? in it.");
      const q = new URLSearchParams(link.trim().split('?')[1] ?? '');
      if ((q.get('nt') || q.get('code')) && notices.get(linkNode(q))?.state !== 'accepted') throw err('invalid', "Read and accept the operator's notice first.");
      nodeLinks.add(link.trim());
      net = { ...net, directory: { known: 0, waiting: nodeLinks.size, connected: 0 } };
      return { known: 0, waiting: nodeLinks.size, connected: 0 };
    },
    // The mock cannot inflate or verify a real notice: a link with `nt=` and `ns=` shows the
    // fixture notice for the node its `p=` names (or the mock node), as the core would. A coded
    // link (`code=`, masked by the core) is the mock node on the internet, with its notice.
    async previewNodeLink(link: string) {
      await jitter();
      const trimmed = link.trim();
      if (!/(^hoi:\/\/|#\/)node\?/.test(trimmed)) throw err('invalid', "That isn't a node link. A node link starts with hoi://node? or has #/node? in it.");
      const q = new URLSearchParams(trimmed.split('?')[1] ?? '');
      if (q.get('code')) {
        if (!/^[A-Za-z0-9_-]{27,}$/.test(q.get('code')!)) throw err('invalid', "That isn't a node link. A node link starts with hoi://node? or has #/node? in it.");
        if (!notices.has(MOCK_NODE)) notices.set(MOCK_NODE, mockNotice(MOCK_NODE, MOCK_PUBLIC_NODE));
        return { host: MOCK_PUBLIC_NODE, scope: 'internet', endpoint: MOCK_NODE, verified: true, notice: structuredClone(notices.get(MOCK_NODE)) } satisfies NodeLinkPreview;
      }
      const first = (q.get('w') ?? q.get('p') ?? '').split(',')[0] ?? '';
      const host = first.includes('@') ? first.slice(first.indexOf('@') + 1) : '';
      const scope: NodeLinkPreview['scope'] = /^(127\.|\[::1\]|localhost)/.test(host) ? 'device' : /^(10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.)/.test(host) ? 'network' : 'internet';
      const node = linkNode(q);
      const preview: NodeLinkPreview = { host, scope, endpoint: q.get('p') ? node : undefined, verified: false };
      if (q.get('nt')) {
        if (!q.get('ns')) preview.problem = 'The notice in this link is not signed.';
        else if (!q.get('p')) preview.problem = 'This link carries a notice but names no node to check it against.';
        else {
          if (!notices.has(node)) notices.set(node, mockNotice(node, host));
          preview.verified = true;
          preview.notice = structuredClone(notices.get(node));
        }
      }
      return preview;
    },
    async nodeNotices() {
      await jitter();
      return [...notices.values()].sort((a, b) => Number(a.state !== 'pending') - Number(b.state !== 'pending') || b.seen - a.seen).map((n) => structuredClone(n));
    },
    async acceptNotice(node: string, hash: string) {
      await jitter();
      const n = notices.get(node);
      if (!n) throw err('not-found', 'No such notice.');
      if (n.hash !== hash) throw err('invalid', "This node's notice or configuration changed since; read it again.");
      n.state = 'accepted';
      n.decidedAt = now();
      noticeNet();
      return structuredClone(n);
    },
    async declineNotice(node: string) {
      await jitter();
      const n = notices.get(node);
      if (!n) throw err('not-found', 'No such notice.');
      n.state = 'declined';
      n.decidedAt = now();
      noticeNet();
      return structuredClone(n);
    },
    async forgetNotice(node: string) {
      await jitter();
      if (!notices.delete(node)) throw err('not-found', 'No such notice.');
      asked.delete(node);
      noticeNet();
    },
    // The mock's nodes all declare what they should: nothing is ever blocked here.
    async nodeBlocks() {
      await jitter();
      return [];
    },
    async unblockNode() {
      await jitter();
      throw err('not-found', 'No such block.');
    },
    async discoverCommunities() {
      await jitter();
      return [...communities.values()]
        .filter((c) => c.limits.admission === 'open')
        .map((c) => ({
          ...ref(c), description: c.description, joined: c.joined, live: true, seen: now(),
          invite: `hoi://join?c=${c.id}&g=${b64url(JSON.stringify(genesisOf(c)))}`,
        }))
        .sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id));
    },
    async communityRegistration() {
      throw err('not-allowed', 'The demo cannot register public names. Use the real app to create a signed community.');
    },
    async communityPublication() {
      throw err('not-allowed', 'The demo cannot publish public names. Use the real app to create a signed community.');
    },
    async publicJoin(id, _peers, name) {
      const c = getCommunity(id);
      if (c.limits.admission !== 'open' || (name && c.name.toLowerCase() !== name)) throw err('not-allowed', 'This public community could not be verified.');
      c.joined = true;
      changed(c.id);
      return { id, status: 'joined' };
    },
    async publicJoinStatus(id) {
      const c = getCommunity(id);
      return { id, status: c.joined ? 'joined' : 'unknown' };
    },
    async warmth() {
      await jitter();
      const list = [...communities.values()].filter((c) => c.joined).map(mockWarmth);
      list.sort((a, b) => a.score - b.score || a.name.localeCompare(b.name) || a.community.localeCompare(b.community));
      const connected = net.peers > 0;
      return {
        communities: list,
        directory: { catalogNodes: connected ? 1 : 0, discoverable: connected ? Math.max(0, communities.size - list.length) : 0, carryNodes: connected && !desktop ? 1 : 0 },
        ownDurable: desktop,
        links: mockLinkWarmth(net.peers),
      };
    },
    async communityWarmth(id) {
      await jitter();
      const c = getCommunity(id);
      if (!c.joined) throw err('not-found', 'Join the community first.');
      return mockWarmth(c);
    },
    async requestCarry(id) {
      await jitter();
      const c = getCommunity(id);
      if (c.limits.admission !== 'open') throw err('not-allowed', 'Public nodes carry open communities only. An invite-only community is kept by its members’ own nodes.');
      const asked = desktop || net.peers === 0 ? 0 : 1;
      if (asked) setTimeout(() => emit({ type: 'notice', text: `A public node now keeps c/${c.name} available. Warmth shows when its copy is complete.` }), 1000);
      return { community: id, asked, carrying: 0 };
    },
    async community(id) {
      await jitter();
      return communityView(getCommunity(id));
    },
    async privateGroups() { return []; },
    async createPrivateCommunity() { throw err('not-allowed', 'Private communities require the secure core.'); },
    async privateGroup() { throw err('not-found', 'Private community not found.'); },
    async privateJoinRequest() { throw err('not-allowed', 'Private communities require the secure core.'); },
    async privatePreviewRequest() { throw err('not-allowed', 'Private communities require the secure core.'); },
    async privateInvite() { throw err('not-allowed', 'Private communities require the secure core.'); },
    async privateJoin() { throw err('not-allowed', 'Private communities require the secure core.'); },
    async privateRemove() { throw err('not-allowed', 'Private communities require the secure core.'); },
    async privatePassRequest() { throw err('not-allowed', 'Private communities require the secure core.'); },
    async privateCopy() { throw err('not-allowed', 'Private communities require the secure core.'); },
    async privateOutbox() { return []; },
    async privateReceive() { throw err('not-allowed', 'Private communities require the secure core.'); },
    async privateAck() { throw err('not-found', 'Private community not found.'); },
    async privateRead() { throw err('not-found', 'Private community not found.'); },
    async createCommunity(draft: CommunityDraft) {
      await sleep(500);
      const me = meKey();
      const name = draft.name.trim().replace(/^c\//i, '');
      if (!name) throw err('invalid', 'A community needs a name.');
      if (name.length > 32) throw err('too-big', 'Keep the name under 32 characters.');
      if (!/^[\p{L}\p{N}._-]+$/u.test(name)) throw err('invalid', 'Names use letters, numbers, dots, dashes and underscores.');
      if (draft.description.length > 280) throw err('too-big', 'Keep the description under 280 characters.');
      if (!COMMUNITY_COLORS.includes(draft.color)) throw err('invalid', 'Pick one of the six colours.');
      const limits: Limits = { ...DEFAULT_LIMITS, ...draft.limits };
      const owners: Hex[] = [me];
      (draft.coOwners ?? []).forEach((raw, i) => {
        const key = parseKey(raw);
        if (!key) throw err('invalid', `Owner key ${i + 1} isn't a key. Paste 64 hex characters or a did:key.`);
        if (!owners.includes(key)) owners.push(key);
      });
      const threshold = Math.round(draft.threshold ?? 1);
      if (threshold < 1 || threshold > owners.length) throw err('invalid', `The threshold has to be between 1 and ${owners.length}.`);
      const id = randomHex();
      const t = now();
      const c: MCommunity = {
        id, name, description: draft.description.trim(), color: draft.color, createdAt: t, joined: true, pinning: true,
        owners, threshold, mods: [me], limits, rules: null, pending: [], banned: [], flags: [], people: owners.length, seq: 0,
        vouches: new Map(), reviews: new Map(), myLevel: 'trusted',
      };
      communities.set(id, c);
      const rules = draft.rules?.trim();
      if (rules) {
        c.rules = { text: rules, version: 1, at: t, signedBy: [me] };
        appendLedger(c, { action: 'rules', by: [me], text: rules, reason: 'First rules' });
      }
      changed(id);
      return communityView(c);
    },
    async previewInvite(link): Promise<InvitePreview> {
      await jitter();
      const inv = parseInvite(link);
      const known = communities.get(inv.id);
      const g = inv.genesis;
      const base = known ?? null;
      return {
        community: {
          id: inv.id,
          name: base?.name ?? g!.n,
          color: base?.color ?? g!.k,
          description: base?.description ?? g!.d,
          limits: { ...(base?.limits ?? g!.l) },
          owners: (base?.owners ?? g!.o).map((k) => personOf(k)),
        },
        relays: inv.relays,
        peers: inv.peers,
        trackers: [],
        alreadyJoined: !!known?.joined,
        verified: !!g,
      };
    },
    async join(link) {
      await sleep(400);
      meKey();
      const inv = parseInvite(link);
      let c = communities.get(inv.id);
      if (!c) {
        const g = inv.genesis!;
        c = {
          id: inv.id, name: g.n, description: g.d, color: g.k, createdAt: g.at ?? now(), joined: true, pinning: false,
          owners: g.o, threshold: g.t ?? 1, mods: [...g.o], limits: { ...DEFAULT_LIMITS, ...g.l }, rules: null,
          pending: [], banned: [], flags: [], people: g.o.length + 1, seq: 0,
          vouches: new Map(), reviews: new Map(), myLevel: 'newcomer',
        };
        communities.set(c.id, c);
      }
      c.joined = true;
      changed(c.id);
      return communityView(c);
    },
    async leave(id) {
      await jitter();
      const c = getCommunity(id);
      c.joined = false;
      c.pinning = false;
      changed(id);
    },
    async setPinning(id, on) {
      await jitter();
      const c = getCommunity(id);
      if (!c.joined && on) throw err('not-allowed', 'Join first, then you can pin it.');
      c.pinning = on;
      changed(id);
      return communityView(c);
    },
    async invite(id): Promise<Invite> {
      await jitter();
      const c = getCommunity(id);
      const query = `c=${c.id}&g=${b64url(JSON.stringify(genesisOf(c)))}${net.homeNode ? `&r=${encodeURIComponent(net.homeNode.url)}` : ''}`;
      const base = typeof location === 'undefined' ? 'https://hoi.sh/app/' : location.origin + location.pathname;
      // As the core: a public node's own address, else the nodes that carried it (none in the demo).
      const addresses: InviteAddress[] = desktop
        ? [
            ...(settings.publicNode && net.portMapping?.state === 'forwarded' ? [{ address: MOCK_PUBLIC, scope: 'internet' as const, whose: 'yours' as const }] : []),
            { address: '192.168.1.20:7844', scope: 'network', whose: 'yours' },
          ]
        : [];
      return { uri: `hoi://join?${query}`, web: `${base}#/join?${query}`, addresses, outside: addresses.some((a) => a.scope === 'internet') };
    },
    async exportBundle(id) {
      await sleep(200);
      const c = getCommunity(id);
      const ps = [...posts.values()].filter((p) => p.community === id && alive(p));
      const body = {
        community: { ...c, flags: [] },
        posts: ps,
        comments: [...comments.values()].filter((x) => ps.some((p) => p.id === x.post)),
        ledger: [...ledgerEntries.values()].filter((e) => e.community === id),
      };
      return enc.encode(BUNDLE_MAGIC + JSON.stringify(body));
    },
    async importBundle(data): Promise<ImportReport> {
      await sleep(300);
      meKey();
      const text = dec.decode(data);
      if (!text.startsWith(BUNDLE_MAGIC)) throw err('invalid', "That file isn't a hoi bundle (.hoibundle).");
      let body: { community: MCommunity; posts: MPost[]; comments: MComment[]; ledger: MEntry[] };
      try {
        body = JSON.parse(text.slice(BUNDLE_MAGIC.length));
      } catch {
        throw err('invalid', 'That bundle is damaged.');
      }
      const report: ImportReport = { accepted: 0, duplicates: 0, rejected: 0, reasons: {} };
      if (!communities.has(body.community.id)) {
        communities.set(body.community.id, { ...body.community, joined: true, flags: [], pending: [], vouches: new Map(), reviews: new Map(), myLevel: 'newcomer' });
        report.accepted++;
      } else report.duplicates++;
      const take = <T extends { id: Hex; expiresAt?: Unix | null }>(list: T[], into: Map<Hex, T>) => {
        for (const r of list) {
          if (into.has(r.id)) report.duplicates++;
          else if (r.expiresAt != null && r.expiresAt <= now()) {
            report.rejected++;
            report.reasons.expired = (report.reasons.expired ?? 0) + 1;
          } else {
            into.set(r.id, r);
            report.accepted++;
          }
        }
      };
      take(body.posts, posts);
      take(body.comments, comments);
      take(body.ledger, ledgerEntries);
      changed(body.community.id);
      return report;
    },

    // -------------------------------------------------------- owners
    async setLimits(id, limits) {
      await sleep(200);
      const c = getCommunity(id);
      if (limits.powBits < 8 || limits.powBits > 30) throw err('invalid', 'The stamp has to be between 8 and 30 bits.');
      if (limits.maxRetention < 3600 || limits.maxRetention > 400 * DAY) throw err('invalid', 'Posts can live between an hour and a year.');
      if (limits.newcomerPostsPerDay < 0 || limits.newcomerPostsPerDay > 100) throw err('invalid', 'Newcomer posts per day: 0 to 100.');
      if (limits.autobanThreshold < 1) throw err('invalid', 'Auto-bounce needs at least one flag.');
      if (limits.admission === 'age-proof') throw err('not-allowed', 'Age proofs are coming later.');
      return ownerAction(c, { action: 'limits', limits: { ...limits, labelers: [...limits.labelers] } });
    },
    async setRules(id, text) {
      await sleep(200);
      const c = getCommunity(id);
      const t = text.trim();
      if (!t) throw err('invalid', 'Rules need at least one line.');
      if (t.length > 8000) throw err('too-big', 'Keep the rules under 8,000 characters.');
      return ownerAction(c, { action: 'rules', text: t });
    },
    async addMod(id, key) {
      await sleep(200);
      const c = getCommunity(id);
      needKey(key);
      if (c.mods.includes(key)) throw err('invalid', 'Already a moderator.');
      return ownerAction(c, { action: 'mod-add', key });
    },
    async removeMod(id, key) {
      await sleep(200);
      const c = getCommunity(id);
      if (!c.mods.includes(key)) throw err('not-found', "That key isn't a moderator here.");
      return ownerAction(c, { action: 'mod-remove', key });
    },
    async retire(id, key) {
      await sleep(200);
      const c = getCommunity(id);
      needKey(key);
      if (c.owners.includes(key)) throw err('not-allowed', "Owners can't be retired.");
      return ownerAction(c, { action: 'retire', key });
    },
    async cosign(code) {
      await sleep(250);
      const me = meKey();
      const p = proposals.get(code.trim());
      if (!p) throw err('not-found', "No proposal with that code on this device. Check you copied all of it.");
      const c = communities.get(p.community)!;
      needOwner(c);
      if (p.signers.includes(me)) throw err('invalid', `You already signed this one. It needs ${p.need - p.signers.length} more.`);
      p.signers.push(me);
      if (p.signers.length >= p.need) {
        forgetProposal(p);
        const entry = applyOwnerAction(c, p);
        changed(c.id);
        return { status: 'published', entry: entryView(entry) };
      }
      // A real proposal code carries the signatures so far, so each cosigner passes on a longer one.
      const next = `${p.code.split('&')[0]}&s=${p.signers.length}${randomHex(4)}`;
      p.code = next;
      proposals.set(next, p);
      return { status: 'needs-cosign', proposal: next, have: p.signers.length, need: p.need };
    },

    // -------------------------------------------------------- mods
    async ledger(id) {
      await jitter();
      getCommunity(id);
      return [...ledgerEntries.values()]
        .filter((e) => e.community === id && (e.expiresAt === null || e.expiresAt > now()))
        .sort((a, b) => b.seq - a.seq)
        .map(entryView);
    },
    async remove(target, reason) {
      await sleep(200);
      const r = getRecord(target);
      needMod(r.community);
      const why = needReason(reason);
      const me = meKey();
      const entry = appendLedger(r.community, { action: 'remove', by: [me], target: { kind: 'record', id: target }, reason: why });
      const removal: MRemoval = { by: me, reason: why, at: now(), ledgerId: entry.id };
      if (r.post) {
        r.post.removed = removal;
        r.post.text = '';
      } else if (r.comment) {
        r.comment.removed = removal;
        r.comment.text = '';
      }
      r.community.flags = r.community.flags.filter((f) => !(f.target.kind === 'record' && f.target.id === target));
      changed(r.community.id);
      return entryView(entry);
    },
    async pin(target, on) {
      await sleep(150);
      const r = getRecord(target);
      needMod(r.community);
      if (!r.post) throw err('invalid', 'Only posts can be pinned.');
      r.post.pinned = on;
      const entry = appendLedger(r.community, { action: on ? 'pin' : 'unpin', by: [meKey()], target: { kind: 'record', id: target } });
      changed(r.community.id);
      return entryView(entry);
    },
    async label(target, label, reason) {
      await sleep(150);
      const r = getRecord(target);
      needMod(r.community);
      const l = label.trim();
      if (!l) throw err('invalid', 'A label needs a word or two.');
      const rec = r.post ?? r.comment!;
      rec.napping = { label: l, by: 'the mods' };
      if (r.post && !r.post.labels.includes(l)) r.post.labels.push(l);
      const entry = appendLedger(r.community, { action: 'label', by: [meKey()], target: { kind: 'record', id: target }, reason: reason?.trim() ? `${l} · ${reason.trim()}` : l });
      changed(r.community.id);
      return entryView(entry);
    },
    async ban(communityId, key, reason) {
      await sleep(200);
      const c = getCommunity(communityId);
      needMod(c);
      needKey(key);
      const why = needReason(reason);
      if (isMe(key)) throw err('invalid', "You can't ban yourself.");
      if (c.owners.includes(key)) throw err('not-allowed', "Owners can't be banned. They can step down.");
      if (c.banned.some((b) => b.key === key)) throw err('invalid', 'That key is already banned here.');
      c.banned.push({ key, reason: why, at: now(), auto: false });
      c.pending = c.pending.filter((k) => k !== key);
      c.flags = c.flags.filter((f) => !(f.target.kind === 'key' && f.target.key === key));
      const entry = appendLedger(c, { action: 'ban', by: [meKey()], target: { kind: 'key', key }, reason: why });
      changed(c.id);
      return entryView(entry);
    },
    async unban(communityId, key, reason) {
      await sleep(200);
      const c = getCommunity(communityId);
      needMod(c);
      const why = needReason(reason);
      if (!c.banned.some((b) => b.key === key)) throw err('not-found', "That key isn't banned here.");
      c.banned = c.banned.filter((b) => b.key !== key);
      const entry = appendLedger(c, { action: 'unban', by: [meKey()], target: { kind: 'key', key }, reason: why });
      changed(c.id);
      return entryView(entry);
    },
    async admit(communityId, key) {
      await sleep(150);
      const c = getCommunity(communityId);
      needMod(c);
      if (!c.pending.includes(key)) throw err('not-found', "That key isn't waiting at the door.");
      c.pending = c.pending.filter((k) => k !== key);
      for (const p of posts.values()) if (p.community === c.id && p.author === key) p.waiting = false;
      const entry = appendLedger(c, { action: 'admit', by: [meKey()], target: { kind: 'key', key } });
      changed(c.id);
      return entryView(entry);
    },

    // -------------------------------------------------------- the ladder
    async standing(communityId, key) {
      await jitter();
      const c = getCommunity(communityId);
      return standingOf(c, key ?? meKey());
    },
    async vouch(communityId, key, value, reason) {
      await stamp(c0(communityId).limits.powBits - 8, true);
      const me = meKey();
      const c = getCommunity(communityId);
      if (key === me) throw err('invalid', 'Nobody vouches for themselves.');
      if (value === -1 && !reason) throw err('invalid', 'Say why: spam, abuse, illegal, off-topic or rule:<n>.');
      const m = c.vouches.get(key) ?? new Map();
      if (value === 0) m.delete(me);
      else m.set(me, value);
      c.vouches.set(key, m);
      changed(c.id);
      return standingOf(c, key);
    },
    async review(target, verdict, reason) {
      const me = meKey();
      const consent = consents.get(target);
      if (consent) {
        const c = getCommunity(consent.community);
        await stamp(c.limits.powBits - 8, true);
        if (consent.by.includes(me)) throw err('not-allowed', 'You proposed it: your signature already says yes.');
        if (!consent.electors.includes(me)) throw err('not-allowed', "You weren't established here when it was proposed, so it isn't yours to decide.");
        if (consent.status === 'passed' || consent.status === 'lapsed') throw err('not-allowed', 'Answers to that proposal closed: it was decided.');
        if (verdict === 'veto' && !reason?.trim()) throw err('invalid', 'Say why you object. The reason is public.');
        if (verdict === 'none') consent.answers.delete(me);
        else if (verdict === 'ok' || verdict === 'veto') consent.answers.set(me, { verdict, reason: reason?.trim() || undefined });
        evaluateConsent(c, consent);
        changed(c.id);
        return;
      }
      const entry = ledgerEntries.get(target);
      const c = entry ? getCommunity(entry.community) : getRecord(target).community;
      await stamp(c.limits.powBits - 8, true);
      if (entry ? entry.by.includes(me) : (posts.get(target)?.author ?? comments.get(target)?.author) === me) throw err('not-allowed', 'Not your own.');
      if (verdict === 'hide' && !/^(spam|abuse|illegal|off-topic|rule:\d+)$/.test(reason ?? '')) throw err('invalid', 'Say why: spam, abuse, illegal, off-topic or rule:<n>. Disagreeing is what downvotes are for.');
      if (verdict === 'veto' && !reason?.trim()) throw err('invalid', 'Say why this action was wrong. The reason is public.');
      const m = c.reviews.get(target) ?? new Map();
      if (verdict === 'none') m.delete(me);
      else m.set(me, { verdict, reason });
      c.reviews.set(target, m);
      changed(c.id);
    },
    async reviewQueue(communityId) {
      await jitter();
      return reviewQueueOf(getCommunity(communityId)).map(summaryOfPost);
    },
    async media(hash) {
      await jitter();
      const m = media.get(hash);
      if (!m) throw err('not-found', 'No media like that on this device.');
      return { hash, mime: m.mime, data: m.data };
    },

    // -------------------------------------------------------- posts
    async feed(q) {
      await jitter();
      if (!identity) return [];
      let list = [...posts.values()].filter((p) => {
        const c = communities.get(p.community)!;
        if (q.community ? p.community !== q.community : !c.joined) return false;
        return visiblePost(p, c);
      });
      list = sortPosts(list, q.sort);
      if (q.community) list.sort((a, b) => Number(b.pinned) - Number(a.pinned));
      return list.slice(0, q.limit ?? 50).map(summaryOfPost);
    },
    async post(id): Promise<PostView> {
      await jitter();
      const p = posts.get(id);
      if (!p || !alive(p)) throw err('not-found', "That post isn't on this device. It may have faded.");
      const c = communities.get(p.community)!;
      const s = summaryOfPost(p);
      return {
        ...s,
        title: p.removed ? '' : s.title,
        excerpt: p.removed ? '' : s.excerpt,
        text: p.removed ? '' : p.text,
        comments: commentTree(p),
        checks: {
          signature: 'valid',
          signedBy: p.signedBy,
          stampBits: p.stampBits,
          requiredBits: Math.max(8, c.limits.powBits - ageDiscount(p.author)),
          receivedFrom: p.receivedFrom,
          createdAt: p.createdAt,
          fadesAt: p.expiresAt,
          identityBits: isMe(p.author) ? identity!.me.identityBits : 10,
          standing: levelIn(c, p.author),
          vouchedBy: vouchersOf(c, p.author).length,
          // the mock has no network: its own posts in open communities read as an injector took them
          ...(isMe(p.author) && c.limits.admission === 'open' ? { addressHidden: true } : {}),
        },
        removed: removalView(p.removed, c),
      };
    },
    async createPost(draft: PostDraft) {
      const me = meKey();
      const c = getCommunity(draft.community);
      if (!c.joined) throw err('not-allowed', 'Join the community first.');
      if (c.banned.some((b) => b.key === me)) throw err('banned', "You're banned in this community.");
      const title = draft.title.trim();
      if (!title) throw err('invalid', 'A post needs a title.');
      if (title.length > 300) throw err('too-big', 'Keep the title under 300 characters.');
      if (draft.text.length > 40_000) throw err('too-big', 'That post is too big. Split it up.');
      const link = draft.link?.trim();
      if (link && !/^https?:\/\/\S+$/i.test(link)) throw err('invalid', 'Links start with https:// or http://');
      const level = levelIn(c, me);
      if (!atLeast(level, 'member') && (link || /https?:\/\/|www\./i.test(draft.text))) throw err('not-allowed', 'Newcomers share text only. Links unlock once members here vouch for you.');
      const files = draft.media ?? [];
      if (files.length && !atLeast(level, 'trusted')) throw err('not-allowed', 'Images and other media unlock for trusted members: established, active, and well received.');
      if (files.length > 4) throw err('invalid', 'A post carries at most 4 media items.');
      const views: MediaView[] = files.map((f) => {
        const mime = sniff(f.data);
        if (!mime) throw err('invalid', "That file won't do: hoi shares PNG, JPEG, GIF and WebP images, Ogg Opus audio and MP4 or WebM video.");
        const hash = randomHex();
        media.set(hash, { mime, data: f.data });
        const png = mime === 'image/png' && f.data.length > 24;
        const be = (o: number) => ((f.data[o] << 24) | (f.data[o + 1] << 16) | (f.data[o + 2] << 8) | f.data[o + 3]) >>> 0;
        return { hash, mime, size: f.data.length, w: png ? be(16) : undefined, h: png ? be(20) : undefined, alt: f.alt, available: true };
      });
      const { bits } = await stamp(c.limits.powBits, false);
      const t = now();
      const p: MPost = {
        id: randomHex(), community: c.id, author: me, title, text: draft.text.trim(), link: link || undefined,
        createdAt: t, expiresAt: t + c.limits.maxRetention, score: 1, myVote: 1, pinned: false, labels: [], edited: false,
        saved: false, waiting: false, receivedFrom: 0, stampBits: bits, signedBy: 'device', media: views,
      };
      posts.set(p.id, p);
      changed(c.id);
      return summaryOfPost(p);
    },
    async reply(parent, text) {
      const me = meKey();
      const t0 = text.trim();
      if (!t0) throw err('invalid', 'Write something first.');
      if (t0.length > 10_000) throw err('too-big', 'That reply is too long.');
      const r = getRecord(parent);
      const post = r.post ?? posts.get(r.comment!.post)!;
      if (post.removed) throw err('not-allowed', 'That post was removed. Replies would go nowhere.');
      if (r.community.banned.some((b) => b.key === me)) throw err('banned', "You're banned in this community.");
      await stamp(r.community.limits.powBits, false);
      const t = now();
      const cm: MComment = {
        id: randomHex(), post: post.id, parent, author: me, text: t0, createdAt: t, expiresAt: t + r.community.limits.maxRetention,
        score: 1, myVote: 1, edited: false, waiting: false,
      };
      comments.set(cm.id, cm);
      changed(r.community.id);
      return commentTree(post).flatMap(function flat(x): CommentView[] { return [x, ...x.children.flatMap(flat)]; }).find((x) => x.id === cm.id)!;
    },
    async edit(id, text, title) {
      await sleep(200);
      const me = meKey();
      const r = getRecord(id);
      const rec = r.post ?? r.comment!;
      if (rec.author !== me) throw err('not-allowed', 'Only the author can edit this.');
      if (rec.removed) throw err('not-allowed', 'Removed records stay removed.');
      if (r.post) {
        if (title !== undefined) {
          if (!title.trim()) throw err('invalid', 'A post needs a title.');
          r.post.title = title.trim();
        }
        r.post.text = text.trim();
        r.post.excerpt = undefined;
      } else {
        if (!text.trim()) throw err('invalid', 'Write something first.');
        r.comment!.text = text.trim();
      }
      rec.edited = true;
      changed(r.community.id);
    },
    async deleteOwn(id) {
      await sleep(200);
      authorDelete(id);
    },
    async deletionProgress() { return [...deletions.values()].map(p => ({ ...p })); },
    async previewBulkDeletion() {
      await sleep(150);
      const inv = bulkInventory();
      return { snapshot: inv.snapshot, communities: inv.communities, count: inv.ids.length, capabilities: 0, gaps: [], inventoryComplete: false };
    },
    async confirmBulkDeletion(snapshot) {
      await sleep(200);
      const saved = bulk.get(snapshot);
      if (saved) return structuredClone(saved);
      const inv = bulkInventory();
      if (inv.snapshot !== snapshot) throw err('invalid', 'Your known posts and comments changed since the preview. Preview again.');
      if (!inv.ids.length) throw err('invalid', 'There are no known posts or comments to delete.');
      const status: BulkDeletionStatus = {
        snapshot, confirmedAt: now(), cancelledAt: null, count: inv.ids.length,
        selected: 0, queued: 0, signed: 0, failed: 0, expired: 0, cancelled: 0,
        targets: [], gaps: [], inventoryComplete: false,
      };
      for (const c of inv.communities) {
        for (const [kind, ids] of [['post', c.posts], ['comment', c.comments]] as const) {
          for (const id of ids) {
            authorDelete(id);
            status.signed++;
            status.targets.push({ id, community: c.id, kind, state: 'signed', reason: null, deletion: id, nextTry: null });
          }
        }
      }
      bulk.set(snapshot, status);
      return structuredClone(status);
    },
    async cancelBulkDeletion(snapshot) {
      const saved = bulk.get(snapshot);
      if (!saved) throw err('not-found', "That deletion batch isn't on this device.");
      saved.cancelledAt ??= now();
      return structuredClone(saved);
    },
    async bulkDeletions() { return [...bulk.values()].map(b => structuredClone(b)); },
    async vote(target, value) {
      meKey();
      const r = getRecord(target);
      const rec = r.post ?? r.comment!;
      await stamp(Math.max(8, r.community.limits.powBits - 8), true);
      rec.score = rec.score - rec.myVote + value;
      rec.myVote = value;
      changed(r.community.id);
      return rec.score;
    },
    async flag(target, reason) {
      await sleep(200);
      meKey();
      if (!/^(spam|illegal|rate|dup|rule:\d{1,2})$/.test(reason)) throw err('invalid', 'Pick a reason: spam, a rule, or illegal.');
      const r = getRecord(target);
      const existing = r.community.flags.find((f) => f.target.kind === 'record' && f.target.id === target);
      if (existing?.mine) throw err('invalid', 'You already flagged this. Once is enough.');
      if (existing) {
        existing.count++;
        existing.weight += 1;
        existing.lastAt = now();
        existing.mine = true;
        if (!existing.reasons.includes(reason)) existing.reasons.push(reason);
      } else {
        r.community.flags.push({ target: { kind: 'record', id: target }, reasons: [reason], count: 1, weight: 1, lastAt: now(), mine: true });
      }
      changed(r.community.id);
    },
    async save(id, on) {
      await jitter();
      const p = posts.get(id);
      if (!p) throw err('not-found', "That post isn't on this device.");
      p.saved = on;
      changed(p.community);
    },
    async saved() {
      await jitter();
      return [...posts.values()].filter((p) => p.saved && alive(p)).sort((a, b) => b.createdAt - a.createdAt).map(summaryOfPost);
    },
    async search(q, community) {
      await sleep(80 + Math.random() * 120);
      const needle = q.trim().toLowerCase();
      if (!needle) return [];
      return [...posts.values()]
        .filter((p) => {
          const c = communities.get(p.community)!;
          if (community && p.community !== community) return false;
          if (!visiblePost(p, c)) return false;
          if (`${p.title}\n${p.text}\n${p.link ?? ''}`.toLowerCase().includes(needle)) return true;
          return commentsOf(p.id).some((x) => !x.removed && x.text.toLowerCase().includes(needle));
        })
        .sort((a, b) => b.createdAt - a.createdAt)
        .slice(0, 50)
        .map(summaryOfPost);
    },

    // -------------------------------------------------------- people
    async person(key, communityId): Promise<PersonView> {
      await jitter();
      const c = communityId ? communities.get(communityId) : undefined;
      const theirs = [...posts.values()].filter((p) => p.author === key && alive(p) && !p.removed);
      const common = [...communities.values()].filter(
        (x) => x.joined && (x.owners.includes(key) || x.mods.includes(key) || theirs.some((p) => p.community === x.id) ||
          [...comments.values()].some((cm) => cm.author === key && posts.get(cm.post)?.community === x.id)),
      );
      return {
        person: personOf(key, c),
        communities: common.map(ref),
        posts: theirs.length,
        firstSeen: people.get(key)?.firstSeen ?? now(),
      };
    },
    async setTrust(key, friend) {
      await jitter();
      if (isMe(key)) throw err('invalid', "That's you. You already trust yourself, hopefully.");
      const p = people.get(key) ?? (personOf(key), people.get(key)!);
      p.trust = friend ? 'friend' : p.baseTrust === 'friend' ? 'member' : p.baseTrust;
      changed();
      return personOf(key);
    },
    async trustSharing(community) {
      await jitter();
      if (!communities.has(community)) throw err('not-found', 'No such community here.');
      const friends = [...people.values()].filter((p) => p.trust === 'friend').length;
      return { community, shared: trustShared.has(community), named: friends, friends };
    },
    async shareTrust(community, on) {
      await jitter();
      const c = communities.get(community);
      if (!c) throw err('not-found', 'No such community here.');
      if (!c.joined) throw err('not-allowed', 'Join the community first.');
      if (on) trustShared.add(community);
      else trustShared.delete(community);
      changed();
      return this.trustSharing(community);
    },
    async blocklistKeepers() {
      await jitter();
      return [...keepers].map((k) => ({ person: personOf(k), exact: 0, pdq: 0 }));
    },
    async followBlocklist(key, on) {
      await jitter();
      if (isMe(key)) throw err('invalid', 'Your own list always counts here.');
      if (on) keepers.add(key);
      else keepers.delete(key);
      changed();
      return this.blocklistKeepers();
    },
    async publishBlocklist(_community, text) {
      await jitter();
      const lines = text.split('\n').map((l) => l.trim()).filter((l) => l && !l.startsWith('#'));
      if (lines.some((l) => !/^(blake3:)?[0-9a-f]{64}/i.test(l))) throw err('invalid', "That isn't a fingerprint (64 hex) or blake3:<hash>.");
      return lines.length;
    },
    async aggregators() {
      await jitter();
      return [...followedAggregators].map((k) => ({ person: personOf(k), communities: 0 }));
    },
    async followAggregator(key, on) {
      await jitter();
      if (isMe(key)) throw err('invalid', 'Your own counts always show here.');
      if (on) followedAggregators.add(key);
      else followedAggregators.delete(key);
      changed();
      return this.aggregators();
    },
    async aggregating(community) {
      await jitter();
      return counting.has(community);
    },
    async setAggregating(community, on) {
      await jitter();
      if (on) counting.add(community);
      else counting.delete(community);
      changed();
      return on;
    },
    async ageChallenge(community) {
      await jitter();
      const c = communities.get(community);
      if (!c) throw err('not-found', 'No such community here.');
      return { aud: `hoi:${community}`, nonce: 'mock-nonce-' + community.slice(0, 8), issuers: [...(c.limits.ageIssuers ?? [])] };
    },
    async ageProofCode(community, presentation) {
      await jitter();
      if (!presentation.includes('~')) throw err('invalid', "That proof won't do: not an SD-JWT presentation.");
      return `hoi-age-1-mock${community.slice(0, 8)}`;
    },
    async admitByAge(code) {
      await jitter();
      throw err('invalid', code.startsWith('hoi-age-1-') ? "That proof won't do: the mock has no wallet." : "That isn't an age proof code.");
    },
    async wikiPages(community) {
      await jitter();
      if (!communities.has(community)) throw err('not-found', 'No such community here.');
      return [...(wiki.get(community) ?? new Map()).entries()]
        .map(([page, p]) => ({ page, chars: p.text.length, updatedAt: p.at, editors: new Set(p.editors).size }))
        .sort((a, b) => b.updatedAt - a.updatedAt);
    },
    async wikiPage(community, page) {
      await jitter();
      const c = communities.get(community);
      if (!c) throw err('not-found', 'No such community here.');
      if (!/^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/.test(page)) throw err('invalid', "A page's name is 1 to 64 of a-z, 0-9 and -.");
      const p = wiki.get(community)?.get(page);
      return { page, text: p?.text ?? '', updatedAt: p?.at ?? 0, edits: p?.edits ?? 0, editors: [...new Set(p?.editors ?? [])].reverse().slice(0, 8).map((k) => personOf(k, c)), canEdit: c.joined && !!identity };
    },
    async editWiki(community, page, text) {
      await jitter();
      const c = communities.get(community);
      if (!c || !c.joined || !identity) throw err('not-allowed', "Members edit the wiki: once you're admitted here, you can too.");
      if (text.length > 64 * 1024) throw err('invalid', 'A page holds 64 KB of text at most. Split it into two pages.');
      const pages = wiki.get(community) ?? new Map();
      const p = pages.get(page) ?? { text: '', at: 0, editors: [], edits: 0 };
      pages.set(page, { text, at: now(), editors: [...p.editors, identity.me.rootKey], edits: p.edits + 1 });
      wiki.set(community, pages);
      changed();
      return this.wikiPage(community, page);
    },
    async linkPreview(url): Promise<LinkPreview | null> {
      await jitter();
      if (!settings.linkPreviews) throw err('not-allowed', 'Link previews are off. Turn them on in You → Privacy.');
      if (!url.startsWith('https://')) return null;
      const site = new URL(url).host.replace(/^www\./, '');
      return { url, site, title: `A page on ${site}`, description: 'What the linked page says about itself, fetched by your computer and kept on this device.' };
    },
    async labelers() {
      await jitter();
      const out = new Map<Hex, LabelerView>();
      for (const k of followedLabelers) out.set(k, { person: personOf(k), followed: true, defaultIn: [], labels: 0 });
      for (const c of communities.values()) {
        if (!c.joined) continue;
        for (const k of c.limits.labelers) {
          const v = out.get(k) ?? { person: personOf(k), followed: false, defaultIn: [], labels: 0 };
          v.defaultIn.push(ref(c));
          out.set(k, v);
        }
      }
      return [...out.values()];
    },
    async followLabeler(key, on) {
      await jitter();
      if (isMe(key)) throw err('invalid', 'Your own labels always count here.');
      if (on) followedLabelers.add(key);
      else followedLabelers.delete(key);
      changed();
      return this.labelers();
    },
    async publishLabel(target, label) {
      await jitter();
      const p = posts.get(target);
      if (!p) throw err('not-found', 'No such post here.');
      if (!label.trim()) throw err('invalid', 'Say what the label is: spam, nsfw, off-topic, or a short note.');
      if (!p.labels.includes(label.trim())) p.labels.push(label.trim());
      changed();
    },
    async setMuted(key, muted) {
      await jitter();
      if (isMe(key)) throw err('invalid', "You can't mute yourself.");
      const p = people.get(key) ?? (personOf(key), people.get(key)!);
      p.muted = muted;
      changed();
      return personOf(key);
    },
    async setPetname(key, petname) {
      await jitter();
      const p = people.get(key) ?? (personOf(key), people.get(key)!);
      const n = petname?.trim();
      if (n && n.length > 32) throw err('too-big', 'Keep it under 32 characters.');
      p.petname = n || undefined;
      changed();
      return personOf(key);
    },

    // -------------------------------------------------------- node
    async stats(): Promise<NodeStats> {
      await jitter();
      const records = [...posts.values()].filter(alive).length + [...comments.values()].filter(alive).length + ledgerEntries.size;
      const storedBytes = [...communities.values()].reduce((n, c) => n + communityView(c).stats.storedBytes, 0) + ledgerEntries.size * 420;
      return {
        records,
        storedBytes,
        communities: [...communities.values()].filter((c) => c.joined).length,
        bouncedKeys: identity ? bounced.keys : 0,
        bouncedRecords: identity ? bounced.records : 0,
        peers: net.peers,
        passedAlongBytes: 0,
        mediaBytes: [...media.values()].reduce((n, m) => n + m.data.length, 0),
        saved: [...posts.values()].filter((p) => p.saved).length,
        readingNotes: 2,
        blockedPictures: blockedPictures.size,
        fadesBy: Math.max(0, ...[...posts.values()].filter(alive).map((p) => p.expiresAt)),
        // As the core: what this desktop keeps for others while its switch is on.
        heldPieces: (settings.holdPiecesGb ?? 0) > 0 ? 37 : 0,
        heldBytes: (settings.holdPiecesGb ?? 0) > 0 ? 4_200_000 : 0,
        heldBudget: Math.round((settings.holdPiecesGb ?? 0) * 1e9),
      };
    },
    async settings() {
      await jitter();
      return { ...settings };
    },
    async setSettings(patch) {
      await jitter();
      if (patch.relay && !desktop) throw err('not-allowed', "A browser can't relay: it can't accept connections. The desktop app can.");
      if (patch.tor) throw err('not-allowed', 'Routing through Tor needs the desktop app.');
      if (patch.dailyCapGb !== undefined && (patch.dailyCapGb < 1 || patch.dailyCapGb > 500)) throw err('invalid', 'Daily cap: 1 to 500 GB.');
      if (patch.companion && !desktop) throw err('not-allowed', 'The companion is the desktop app.');
      if (patch.publicNode && !desktop) throw err('not-allowed', "A browser can't be a public node: it can't accept connections. The desktop app can.");
      if (patch.holdPiecesGb !== undefined && patch.holdPiecesGb !== 0 && (patch.holdPiecesGb < 0.5 || patch.holdPiecesGb > 100)) {
        throw err('invalid', 'Mailboxes for others take from 0.5 to 100 GB.');
      }
      if (patch.holdPiecesGb && !desktop) throw err('not-allowed', 'Only the desktop app keeps mailboxes for others; on a server, hoi-node --hold-pieces does.');
      settings = { ...settings, ...patch };
      // As the core: a desktop rebuilds for itself, so "Use less data" stays off there, and only
      // a durable device (a desktop) shares the holders it sees.
      if (desktop) settings.lessData = false;
      if (!desktop) settings.shareHolders = false;
      // and only a browser links directly with other browsers
      if (desktop) settings.directLinks = false;
      // As the core: a public node relays, and without relaying nobody is served.
      if (patch.publicNode) settings.relay = true;
      if (patch.relay === false) settings.publicNode = false;
      // Nobody could reach a holder that isn't a public node.
      if (!settings.publicNode) settings.holdPiecesGb = 0;
      if (patch.vpn !== undefined || patch.homeNetwork !== undefined) {
        if (!desktop) throw err('not-allowed', 'Only the desktop app chooses which network it uses.');
        net = { ...net, vpn: vpnNow() };
        emit({ type: 'net', status: { ...net } });
      }
      if (patch.relay !== undefined || patch.publicNode !== undefined) {
        net = { ...net, relaying: desktop && settings.relay };
        if (settings.publicNode && net.portMapping?.state === 'forwarded') publish();
        else if (settings.publicNode) askRouter();
        else unpublish();
        emit({ type: 'net', status: { ...net } });
      }
      return { ...settings };
    },
    async net() {
      await jitter();
      return { ...net, homeNode: net.homeNode ? { ...net.homeNode } : undefined };
    },
    async traffic() {
      await jitter();
      return mockTraffic();
    },
    async previewPair(link) {
      await jitter();
      const f = pairFields(link);
      if (!f) throw err('invalid', "That pairing link doesn't work: not a hoi pair link.");
      const pairedHere = !!net.homeNode && (net.homeNode.pairingWeb.includes(f.query) || (!!f.t && net.homeNode.pairingWeb.includes(`t=${f.t}`)));
      return { name: f.n ?? undefined, url: f.u, routes: (f.w ?? '').split(',').filter(Boolean), pairedHere };
    },
    async pair(link) {
      await sleep(600);
      const f = pairFields(link);
      if (!f) throw err('invalid', "That isn't a pairing link. It starts with hoi://pair? or has #/pair? in it.");
      const { u, t, w } = f;
      if ((!u && !w) || (u && !/^wss?:\/\//.test(u))) throw err('invalid', 'The pairing link has no node address (u=ws://… or w=…).');
      if (!t) throw err('invalid', 'The pairing link has no token (t=…).');
      const name = f.n ?? 'your home node';
      const via = !u || /Android|iPhone/i.test(globalThis.navigator?.userAgent ?? '') ? 'webrtc' : 'websocket';
      net = {
        ...net,
        mode: 'home-node',
        label: `via ${name}`,
        peers: 4,
        homeNode: { url: u, connected: true, name, pairingWeb: `${appBase()}#/pair?${f.query}`, phoneCanReach: !!w || u.startsWith('wss://'), via },
        ...mockLinks(via),
      };
      startPeerWobble();
      emit({ type: 'net', status: { ...net } });
      return { ...net };
    },
    async unpair() {
      await jitter();
      net = localNet();
      emit({ type: 'net', status: { ...net } });
      return { ...net };
    },
    async rotatePairing() {
      await jitter();
      if (!desktop) throw err('not-allowed', 'Only a home node has a pairing link.');
      return { ...net };
    },
    async vpnInterfaces() {
      await jitter();
      if (!desktop) throw err('not-allowed', 'Only the desktop app chooses which network it uses.');
      return MOCK_INTERFACES.map((i) => ({ ...i }));
    },
    async previewPublicNode(on) {
      await jitter();
      if (!desktop) throw err('not-allowed', 'Only the desktop app can ask a router to forward its ports.');
      if (on && !net.portMapping) askRouter();
      if (!on && !settings.publicNode) {
        unpublish();
        emit({ type: 'net', status: { ...net } });
      }
      return { ...net };
    },
    async importBlocklist(text) {
      await sleep(150);
      const hashes = text.split('\n').map((l) => l.trim().split(/[\s,]/)[0].toLowerCase()).filter((h) => /^[0-9a-f]{64}$/.test(h));
      if (!hashes.length) throw err('invalid', 'No picture fingerprints in that file. Expected one 64-character PDQ hash per line.');
      const before = blockedPictures.size;
      for (const h of hashes) blockedPictures.add(h);
      return blockedPictures.size - before;
    },
    async wipe() {
      await sleep(400);
      identity = null;
      seeded = false;
      people.clear();
      communities.clear();
      posts.clear();
      deletions.clear();
      bulk.clear();
      comments.clear();
      ledgerEntries.clear();
      proposals.clear();
      pendingLinks.clear();
      settings = defaultSettings();
      net = localNet();
      asks.clear();
      media.clear();
      bounced = { keys: 23, records: 312 };
      if (peerTimer) clearInterval(peerTimer);
      peerTimer = null;
      emit({ type: 'net', status: { ...net } });
    },
    async cleanupReadiness() {
      await sleep(80);
      return cleanupReadiness();
    },
    async clearAfterDeletion(inventory, deletions) {
      await sleep(120);
      const r = cleanupReadiness();
      if (!r.ready) throw err('cleanup-blocked', 'Deletion work still needs this device. Nothing was cleared.');
      if (r.inventory !== inventory || r.deletions !== deletions) throw err('cleanup-changed', 'Your posts or deletions changed since you checked. Nothing was cleared; check again.');
      await core.wipe();
    },
  };

  function ageDiscount(key: Hex): number {
    const days = personOf(key).keyAgeDays;
    return days < 30 ? 0 : days < 90 ? 2 : days < 365 ? 4 : 6;
  }

  return core;
}
