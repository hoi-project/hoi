/// <reference lib="webworker" />
// The browser node: hoi-wasm in a Web Worker. Owns IndexedDB, the one-tab lock and the links to
// your home node and members' nodes. The page talks to it with {id, method, args} messages, and
// carries its WebRTC links (rtc-link.ts).

import init, { WasmNode, WasmVault } from './wasm/pkg/hoi_wasm.js';
import { apply, snapshot, vaultHeader, decodeSnapshot, migrate, openStore, resetStore, saveHeader, type Change, type Db } from './idb';
import { createDurableSettler } from './durable-settle';
import { groupRoutes } from './rtc-link';

declare const self: DedicatedWorkerGlobalScope;

type Req = { id: number; method: string; args: unknown[] };
type Out =
  | { kind: 'reply'; id: number; ok: true; result: unknown }
  | { kind: 'reply'; id: number; ok: false; error: { code: string; message: string } }
  | { kind: 'event'; event: unknown }
  | { kind: 'locked' };

let lockName = 'hoi-core';
let storeName = 'hoi';
// The scattered-delivery round (mailboxes of private groups, holder sightings) runs every few
// seconds; the expiry sweep every 36th round, about every three minutes.
const TICK_MS = 5_000;
const SWEEP_EVERY = 36;
const SYNC_WATCHDOG_MS = 30_000;

let node: WasmNode | null = null;
let db: Db | null = null;
let webBase = '';
let blocked = false;
let stolen = false;
let queue: Promise<unknown> = Promise.resolve();
let settleChanges: (() => Promise<void>) | null = null;
let storageFailed = false;
let vault: WasmVault | null = null;

const b64url = (b: Uint8Array) => btoa(String.fromCharCode(...b)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
const unb64url = (s: string) => Uint8Array.from(atob(s.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0));
let sweepTimer: ReturnType<typeof setInterval> | null = null;
let rounds = 0;

const post = (m: Out) => self.postMessage(m);
const emit = (event: unknown) => post({ kind: 'event', event });

// ------------------------------------------------------------------ the one-tab lock

/** Holds the lock until the worker dies or another tab steals it. */
function holdLock(steal: boolean): Promise<boolean> {
  if (!('locks' in navigator)) return Promise.reject(new Error('This browser cannot safely open your saved data. Use an up-to-date browser.'));
  return new Promise((resolve) => {
    navigator.locks
      .request(lockName, steal ? { steal: true } : { ifAvailable: true }, (lock) => {
        if (!lock) {
          resolve(false);
          return;
        }
        resolve(true);
        // Never resolves: the lock is ours for the worker's lifetime.
        return new Promise<void>(() => {});
      })
      .catch(() => {
        // Another tab took over: stop touching storage and sockets.
        stolen = true;
        releaseRuntime();
        post({ kind: 'locked' });
      });
  });
}

// ------------------------------------------------------------------ opening the node

async function vaultStatus() {
  if (blocked || stolen) return { state: 'blocked' };
  if (node && vault && sweepTimer && !storageFailed) return { state: 'ready' };
  db ??= await openStore(storeName);
  try {
    const saved = await snapshot(db);
    return { state: vaultHeader(saved) ? 'locked' : saved.blobs.length || saved.records.length ? 'migrate' : 'create' };
  } finally {
    letStoreGo();
  }
}

/** While locked nothing holds the store open: OPFS files stay free for another tab or a reset. */
function letStoreGo() {
  if (node) return;
  db?.close();
  db = null;
}

function releaseRuntime() {
  closeAll();
  if (sweepTimer) clearInterval(sweepTimer);
  sweepTimer = null;
  node?.free();
  node = null;
  vault?.free();
  vault = null;
  settleChanges = null;
  db?.close();
  db = null;
}

async function unlock(passphrase: string) {
  return unlockWith((header) => header ? WasmVault.unlock(header, passphrase) : new WasmVault(passphrase));
}

/** A passkey's answer opens the vault; it never creates one. */
async function unlockPasskey(credential: Uint8Array, prf: Uint8Array) {
  return unlockWith((header) => {
    if (!header) throw new Error('Set a passphrase first; a passkey can be added after.');
    return WasmVault.unlock_passkey(header, credential, prf);
  });
}

async function unlockWith(open: (header: Uint8Array | null) => WasmVault) {
  if (blocked || stolen) throw new Error('Open hoi in this tab before unlocking.');
  if (node && vault) {
    // Migration may have committed the vault before its first core flush failed. A retry
    // must authenticate that installed password, never appear to install a different one.
    const check = open(vault.header());
    check.free();
    await settle();
    activateRuntime();
    return;
  }
  db ??= await openStore(storeName);
  const store = db;
  const saved = await snapshot(store);
  const header = vaultHeader(saved);
  let opened: WasmVault | null = null;
  let candidate: WasmNode | null = null;
  let authenticated = false;
  try {
    opened = open(header);
    authenticated = !!header;
    const { records, blobs } = decodeSnapshot(saved, header ? opened : null);
    candidate = new WasmNode('web', webBase, records, blobs);
    if (stolen) throw new Error('hoi moved to another tab.');
    // Validate the existing core format before changing the database. The rewrite is atomic.
    if (!header) await migrate(store, saved, opened);
    if (stolen) throw new Error('hoi moved to another tab.');
    node = candidate;
    candidate = null;
    vault = opened;
    opened = null;
    settleChanges = createDurableSettler(node, changes => saveChanges(store, changes), flushOutbox, emit);
    await settle();
    activateRuntime();
  } catch (e) {
    candidate?.free();
    opened?.free();
    letStoreGo();
    // A failed initial flush retains the live journal and can be retried; a failed unlock
    // never creates a new identity, changes the password, or rewrites existing ciphertext.
    if (!node) throw authenticated
      ? { code: 'saved-data', message: `The passphrase opened your vault, but its saved data could not be loaded. No replacement key was created. ${toError(e).message}` }
      : { code: 'invalid', message: header
        ? 'Could not unlock. Check this browser’s local passphrase; a key-file backup can have a different passphrase. A damaged vault header can also cause this. No data was replaced.'
        : 'Could not protect your saved data. Use a passphrase of at least 15 characters and try again.' };
    throw e;
  }
}

function activateRuntime() {
  startLinks();
  if (!sweepTimer) {
    sweepTimer = setInterval(() => {
      const sweep = rounds++ % SWEEP_EVERY === 0;
      void run(() => {
        if (sweep) node?.sweep();
        node?.scatterTick();
      }).catch(() => undefined);
    }, TICK_MS);
  }
}

/** Writes storage changes, sends queued messages, forwards events, hands out media to look at. */
async function settle() {
  if (!node || !settleChanges) return;
  await settleChanges();
  handOutLooks();
  checkHandles();
  handOutDirect();
}

/** Names that are domains: a browser asks the domain itself, over HTTPS, without
 * cookies or referrer. Only domains that allow it (CORS) answer; the rest show as not verified. */
function checkHandles() {
  if (!node) return;
  for (const domain of node.handleChecks() as string[]) void checkHandle(domain);
}

async function checkHandle(domain: string) {
  let lines: string[] = [];
  const stop = new AbortController();
  const timer = setTimeout(() => stop.abort(), 5000);
  try {
    const r = await fetch(`https://${domain}/.well-known/hoi`, { credentials: 'omit', referrerPolicy: 'no-referrer', cache: 'no-store', mode: 'cors', signal: stop.signal });
    if (r.ok) lines = (await r.text()).slice(0, 4096).split('\n').map((l) => l.trim()).filter(Boolean).slice(0, 8);
  } catch {
    // not there, or the domain doesn't allow other sites to read it
  } finally {
    clearTimeout(timer);
  }
  if (!node) return;
  node.handleChecked(domain, lines);
  await settle();
}

/** Media the core wants looked at outside itself: the page's look worker. */
function handOutLooks() {
  if (!node) return;
  const wanted = node.looks_wanted() as { id: string; mime: string; data: Uint8Array<ArrayBuffer> }[];
  for (const w of wanted) self.postMessage({ kind: 'look', id: w.id, mime: w.mime, data: w.data }, [w.data.buffer]);
}

/** Report storage failures separately from any later socket or event delivery failure. */
async function saveChanges(store: Db, changes: Change[]) {
  try {
    if (!vault || stolen) throw new Error('This device is locked.');
    await apply(store, changes, vault);
  } catch (e) {
    const message = 'This browser could not save changes. Sync is paused. Keep this tab open, check free storage and allow site storage; saving will retry.';
    if (!storageFailed) emit({ type: 'notice', text: message });
    storageFailed = true;
    throw { code: 'internal', message: `${message} ${toError(e).message}` };
  }
  if (storageFailed) emit({ type: 'notice', text: 'Storage is working again. Pending changes are saved and syncing can resume.' });
  storageFailed = false;
}

function toError(e: unknown): { code: string; message: string } {
  if (typeof e === 'string') {
    try {
      const j = JSON.parse(e);
      if (j && typeof j === 'object' && 'code' in j) return j;
    } catch {
      /* plain message */
    }
    return { code: 'internal', message: e };
  }
  if (e && typeof e === 'object' && 'code' in e) return e as { code: string; message: string };
  return { code: 'internal', message: String((e as Error)?.message ?? e) };
}

function call(method: string, args: unknown[]): unknown {
  if (!node) throw { code: 'offline', message: stolen ? 'hoi is open in another tab. Reload to use it here.' : 'hoi is still starting.' };
  try {
    return JSON.parse(node.call(method, JSON.stringify(args)));
  } catch (e) {
    throw toError(e);
  }
}

/** Serializes everything that touches the node and storage. */
function run<T>(f: () => T | Promise<T>, retryPending = true): Promise<T> {
  const p = queue.then(async () => {
    // Do not accept another mutation while an earlier one still cannot be saved.
    if (storageFailed && retryPending) await settle();
    return f();
  }).then(async (v) => {
    await settle();
    return v;
  });
  queue = p.catch(() => undefined);
  return p;
}

// ------------------------------------------------------------------ links
//
// `home`: your home node, over its WebSocket (same computer, or a wss:// address) or, failing
// that, over WebRTC (a phone on the same network as your computer). `member-N`: members' nodes
// from accepted invites or public joins, over WebRTC, alongside a home when needed. WebRTC
// lives on the page (workers can't open it): we ask for it with `rtc-open` and swap whole sync
// messages with `rtc-send` / `rtc` events.

interface Link {
  send(bytes: Uint8Array<ArrayBuffer>): void;
  close(): void;
}

type Home = { url: string; token: string; name?: string; rtc?: string[] } | null;

const links = new Map<string, Link>();
let wantHome = false;
let homeGen = 0;
let backoff = 1000;
let retry: ReturnType<typeof setTimeout> | null = null;
let watchdog: ReturnType<typeof setInterval> | null = null;
let membersTimer: ReturnType<typeof setInterval> | null = null;

function netStatus() {
  try {
    emit({ type: 'net', status: call('net', []) });
  } catch {
    /* not open */
  }
}

function flushOutbox() {
  if (!node) return;
  const out = node.take_outbox() as [string, Uint8Array<ArrayBuffer>][];
  for (const [peer, bytes] of out) links.get(peer)?.send(bytes);
}

function dropLink(peer: string) {
  const l = links.get(peer);
  if (!l) return;
  links.delete(peer);
  memberOpen.delete(peer);
  discoveryOpen.delete(peer);
  l.close();
  node?.net_disconnect(peer);
}

function closeHome(keepWanting: boolean) {
  wantHome = keepWanting;
  homeGen++;
  if (retry) clearTimeout(retry);
  retry = null;
  dropLink('home');
}

function closeAll() {
  closeHome(false);
  for (const id of [...directs.keys()]) dropDirect(id);
  for (const peer of [...links.keys()]) dropLink(peer);
  if (watchdog) clearInterval(watchdog);
  if (membersTimer) clearInterval(membersTimer);
  watchdog = membersTimer = null;
}

const UNREACHABLE = 'Your home node is unreachable. Is it running, and is this the right link?';
const blockedText = (space: 'loopback' | 'local') =>
  `Your browser blocks this page from reaching ${space === 'loopback' ? 'apps on this device' : 'your local network'}, so it can't reach your home node. Open the site settings (the icon left of the address), allow "${space === 'loopback' ? 'Apps on device' : 'Local network'}", then reload.`;

/** A home node on this computer or the local network, which browsers guard (Local Network Access). */
function isLocal(url: string): 'loopback' | 'local' | null {
  let host: string;
  try {
    host = new URL(url).hostname.replace(/^\[|\]$/g, '');
  } catch {
    return null;
  }
  if (host === 'localhost' || host === '::1' || /^127\./.test(host)) return 'loopback';
  if (/^(10|192\.168|169\.254|172\.(1[6-9]|2\d|3[01]))\./.test(host) || (host.includes(':') && /^(f[cd]|fe80)/i.test(host)) || host.endsWith('.local')) return 'local';
  return null;
}

/** The message to show when the browser refused this page the home node's address space, else null. */
async function blockedByBrowser(url: string): Promise<string | null> {
  const space = isLocal(url);
  if (!space || !('permissions' in navigator)) return null;
  // Chrome 142–144 named one permission; later versions split it into two
  const names = space === 'loopback' ? ['loopback-network', 'local-network-access'] : ['local-network', 'local-network-access'];
  for (const name of names) {
    try {
      const st = await navigator.permissions.query({ name } as unknown as PermissionDescriptor);
      return st.state === 'denied' ? blockedText(space) : null;
    } catch {
      // this browser doesn't know the name
    }
  }
  return null;
}

/** Wires an open link into the node as `peer`; `gone` runs once when it closes. `dialed`: the
 * WebRTC routes it was opened with (one node, one certificate), so the core names to that node
 * only the communities whose routes led there, and learns its routes from it only for that
 * certificate. */
function attach(peer: string, role: 'home' | 'peer', transport: 'websocket' | 'webrtc', local: boolean, tls: boolean, link: Link, dialed: string[] = [], binding?: RtcBinding) {
  links.set(peer, link);
  void run(() => {
    node?.net_connect(peer, role);
    node?.set_peer_meta(peer, transport, local, tls, dialed.length ? JSON.stringify(dialed) : undefined, binding ? JSON.stringify(binding) : undefined);
  }).then(netStatus, () => undefined);
}

function received(peer: string, link: Link, bytes: Uint8Array) {
  void run(() => {
    if (!node || links.get(peer) !== link) return;
    if (!node.net_receive(peer, bytes)) {
      link.close();
      lost(peer, link);
    }
  }).then(netStatus, () => undefined);
}

function reconnectHomeLater() {
  if (!wantHome || stolen) return;
  if (retry) clearTimeout(retry);
  retry = setTimeout(connectHome, backoff);
  backoff = Math.min(backoff * 2, 60_000);
}

/** A link closed or was given up on: forget it, and come back for the home node. */
function lost(peer: string, link: Link) {
  if (links.get(peer) !== link) return;
  links.delete(peer);
  // Ended by the core for an operator's notice the person has not read: no refusal,
  // no wait; the node is dialed again once the person answered.
  const consent = run(() => {
    const closed = node?.consent_closed(peer) ?? false;
    node?.net_disconnect(peer);
    return closed;
  });
  void consent.then(netStatus, () => undefined);
  if (peer === 'home') reconnectHomeLater();
  const m = memberOpen.get(peer) ?? discoveryOpen.get(peer);
  if (m) {
    memberOpen.delete(peer);
    discoveryOpen.delete(peer);
    void consent.then((closed) => {
      if (closed) memberRetry.delete(m.key);
      // Turned away right after opening (not relaying, or done for today): wait before the next try.
      else if (Date.now() - m.at < MEMBER_SHORT_MS) memberWait(m.key);
      else memberRetry.delete(m.key);
    }, () => undefined);
  }
}

/** One WebSocket attempt; resolves true once it's open (and keeps it), false if it never opened. */
function tryWebSocket(url: string, gen: number): Promise<boolean> {
  return new Promise((resolve) => {
    let sock: WebSocket;
    try {
      sock = new WebSocket(url);
    } catch (e) {
      node?.set_net_error(String((e as Error).message ?? e));
      resolve(false);
      return;
    }
    sock.binaryType = 'arraybuffer';
    let opened = false;
    const link: Link = { send: (b) => sock.readyState === WebSocket.OPEN && sock.send(b), close: () => ((sock.onclose = null), sock.close()) };
    const t = setTimeout(() => {
      if (!opened) {
        sock.onclose = null;
        sock.close();
        resolve(false);
      }
    }, 5000);
    sock.onopen = () => {
      clearTimeout(t);
      if (gen !== homeGen) return link.close();
      opened = true;
      backoff = 1000;
      attach('home', 'home', 'websocket', isLocal(url) === 'loopback', url.startsWith('wss://'), link);
      resolve(true);
    };
    sock.onmessage = (e) => received('home', link, new Uint8Array(e.data as ArrayBuffer));
    sock.onerror = () => {
      void run(() => node?.set_net_error(UNREACHABLE));
      void blockedByBrowser(url).then((msg) => {
        if (msg) void run(() => node?.set_net_error(msg)).then(netStatus, () => undefined);
      });
    };
    sock.onclose = () => {
      clearTimeout(t);
      if (opened) lost('home', link);
      else resolve(false);
    };
  });
}

// WebRTC links through the page.
let nextRtc = 1;
const rtcWaiting = new Map<number, { resolve: (ok: boolean) => void; peer: string; link: Link; opened: boolean; binding?: RtcBinding }>();

/** `keep`: asked once the link opened; false closes it unused (another attempt won a race). */
function tryRtc(peer: string, routes: string[], mode: 'c' | 'p', role: 'home' | 'peer', gen: number, keep: () => boolean = () => true): Promise<boolean> {
  const id = nextRtc++;
  const link: Link = {
    send: (bytes) => self.postMessage({ kind: 'rtc-send', id, bytes }, [bytes.buffer]),
    close: () => self.postMessage({ kind: 'rtc-close', id }),
  };
  return new Promise((resolve) => {
    rtcWaiting.set(id, {
      peer,
      link,
      opened: false,
      resolve: (ok) => {
        if (ok && ((peer === 'home' && gen !== homeGen) || !keep())) {
          rtcWaiting.delete(id);
          link.close();
          return resolve(false);
        }
        if (ok) {
          if (peer === 'home') backoff = 1000;
          attach(peer, role, 'webrtc', false, false, link, routes, rtcWaiting.get(id)?.binding);
        }
        resolve(ok);
      },
    });
    self.postMessage({ kind: 'rtc-open', id, routes, mode });
  });
}

type RtcBinding = { node: string; browser: string; ufrag: string };

// Direct links with other members' browsers. The core says whom to offer to, and hands
// on offers and answers that came through a node (`takeDirect`); the page makes the connection
// (rtc-direct.ts); an open one is a link to a peer like any other, named `direct-<id>`.
type DirectEvent =
  | { t: 'offer'; via: string; to: string; stun: string[] }
  | { t: 'signal'; via: string; from: string; stun: string[]; data: number[] }
  | { t: 'close'; peer: string };
const directs = new Map<number, { via: string; handle: string; incoming: boolean; peer: string; link: Link; opened: boolean }>();

function directLink(id: number): Link {
  return {
    send: (bytes) => self.postMessage({ kind: 'rtc-send', id, bytes }, [bytes.buffer]),
    close: () => self.postMessage({ kind: 'rtc-close', id }),
  };
}

function handOutDirect() {
  if (!node) return;
  for (const e of JSON.parse(node.takeDirect()) as DirectEvent[]) {
    if (e.t === 'close') {
      // another direct link goes to the same browser
      const found = [...directs].find(([, d]) => d.peer === e.peer);
      if (found) dropDirect(found[0]);
      continue;
    }
    if (e.t === 'offer') {
      const id = nextRtc++;
      directs.set(id, { via: e.via, handle: e.to, incoming: false, peer: `direct-${id}`, link: directLink(id), opened: false });
      self.postMessage({ kind: 'direct-offer', id, stun: e.stun });
      continue;
    }
    const data = Uint8Array.from(e.data);
    const ours = [...directs].find(([, d]) => d.via === e.via && d.handle === e.from);
    if (ours && !ours[1].incoming) {
      // the answer to our offer
      self.postMessage({ kind: 'direct-accept', id: ours[0], data }, [data.buffer]);
      continue;
    }
    if (ours) dropDirect(ours[0]);
    const id = nextRtc++;
    directs.set(id, { via: e.via, handle: e.from, incoming: true, peer: `direct-${id}`, link: directLink(id), opened: false });
    self.postMessage({ kind: 'direct-answer', id, stun: e.stun, data }, [data.buffer]);
  }
}

function dropDirect(id: number) {
  const d = directs.get(id);
  if (!d) return;
  directs.delete(id);
  if (d.opened) dropLink(d.peer);
  else d.link.close();
}

function onDirectEvent(e: { ev: 'open' | 'msg' | 'close' | 'sig'; id: number; bytes?: Uint8Array }) {
  const d = directs.get(e.id);
  if (!d) return;
  if (e.ev === 'sig' && e.bytes) {
    const bytes = e.bytes;
    void run(() => node?.signal(d.via, d.handle, bytes)).catch(() => undefined);
  } else if (e.ev === 'open') {
    d.opened = true;
    links.set(d.peer, d.link);
    void run(() => {
      node?.net_connect(d.peer, 'peer');
      node?.setDirectMeta(d.peer, d.via, d.handle, d.incoming);
    }).then(netStatus, () => undefined);
  } else if (e.ev === 'msg' && e.bytes) {
    received(d.peer, d.link, e.bytes);
  } else if (e.ev === 'close') {
    directs.delete(e.id);
    if (d.opened) lost(d.peer, d.link);
  }
}

function onRtcEvent(e: { ev: 'open' | 'msg' | 'close' | 'sig'; id: number; bytes?: Uint8Array; reason?: string; binding?: RtcBinding }) {
  if (directs.has(e.id)) return onDirectEvent(e);
  const w = rtcWaiting.get(e.id);
  if (!w) return;
  if (e.ev === 'open') {
    w.opened = true;
    w.binding = e.binding;
    w.resolve(true);
  } else if (e.ev === 'msg' && e.bytes) {
    received(w.peer, w.link, e.bytes);
  } else if (e.ev === 'close') {
    rtcWaiting.delete(e.id);
    if (w.opened) lost(w.peer, w.link);
    else w.resolve(false);
  }
}

/** Phones and tablets never run the desktop app: a loopback address there is someone else's. */
const onPhone = /Android|iPhone|iPad|iPod/i.test(navigator.userAgent);

/** WebSocket first (fast on the same computer), then the home node's WebRTC routes. */
async function connectHome() {
  if (!node || stolen) return;
  const home = JSON.parse(node.home()) as Home;
  closeHome(!!home);
  if (!home) {
    netStatus();
    return;
  }
  const gen = homeGen;
  const skipLoopback = onPhone && isLocal(home.url) === 'loopback' && !!home.rtc?.length;
  if (home.url && !skipLoopback && (await tryWebSocket(home.url, gen))) return;
  if (gen !== homeGen) return;
  for (const group of groupRoutes(home.rtc ?? [])) {
    if (await tryRtc('home', group, 'c', 'home', gen)) return;
    if (gen !== homeGen) return;
  }
  netStatus();
  if (gen === homeGen) reconnectHomeLater();
}

// Bounded member/public-node connections. A node that didn't answer waits before retrying.
const MAX_MEMBER_LINKS = 2;
/** A member link that closes sooner than this counts as a refusal. */
const MEMBER_SHORT_MS = 60_000;
const memberRetry = new Map<string, { at: number; wait: number }>();
/** Open member links: which node they reach, and since when. */
const memberOpen = new Map<string, { key: string; at: number }>();

function memberWait(key: string) {
  const wait = Math.min((memberRetry.get(key)?.wait ?? 30_000) * 2, 30 * 60_000);
  memberRetry.set(key, { at: Date.now() + wait, wait });
}
let memberN = 1;
let membersBusy = false;
let preferredMembers = new Set<string>();

async function maintainMembers() {
  if (!node || stolen || membersBusy) return;
  const memberPeers = [...links.keys()].filter((p) => p.startsWith('member-'));
  // A paired home need not carry a publicly selected community. Keep the bounded
  // explicit/member routes available alongside it; rtc_candidates excludes our home.
  // Private groups' mailboxes come first among the candidates and get links of their own.
  const limit = MAX_MEMBER_LINKS + node.mailbox_links();
  if (memberPeers.length >= limit) return;
  membersBusy = true;
  try {
    const now = Date.now();
    const candidates = JSON.parse(node.rtc_candidates()) as string[];
    const groups = groupRoutes([...new Set([...candidates.filter(r => preferredMembers.has(r.slice(0, 47))), ...candidates])]);
    const connected = new Set([...memberOpen.values()].map(m => m.key));
    let open = memberPeers.length;
    for (const g of groups) {
      if (open >= limit) break;
      const key = g[0].slice(0, 47);
      if (connected.has(key)) continue;
      const r = memberRetry.get(key);
      if (r && r.at > now) continue;
      const peer = `member-${memberN++}`;
      // Registered before the attempt: a refusal can close the link before tryRtc resolves.
      memberOpen.set(peer, { key, at: Date.now() });
      const ok = await tryRtc(peer, g, 'p', 'peer', homeGen);
      if (ok) {
        if (links.has(peer)) open++;
      } else {
        memberOpen.delete(peer);
        memberWait(key);
      }
    }
  } finally {
    membersBusy = false;
  }
}

// One link to a public node for finding communities, having new ones carried and hearing of
// other nodes. Nothing in the app names a node: the core hands over node links opened here,
// nodes this browser reached before, then what those nodes passed on (hoi-core `nodes`).
// A few are dialed at once and the first to answer is kept: the nearest, least busy node
// serves this browser, and the others are closed before they carry anything.
const discoveryOpen = new Map<string, { key: string; at: number }>();
let discoveryN = 1;
let discoveryBusy = false;
/** Public nodes dialed at the same time; a node link opened here is always among them. */
const DISCOVERY_RACE = 3;

async function maintainDiscovery() {
  if (!node || stolen || discoveryBusy || discoveryOpen.size) return;
  discoveryBusy = true;
  try {
    const now = Date.now();
    const candidates = JSON.parse(node.discovery_candidates()) as string[];
    const connected = new Set([...memberOpen.values()].map(m => m.key));
    const groups = groupRoutes(candidates).filter((g) => {
      const key = g[0].slice(0, 47);
      const r = memberRetry.get(key);
      return !connected.has(key) && !(r && r.at > now);
    });
    for (let i = 0; i < groups.length && !discoveryOpen.size; i += DISCOVERY_RACE) {
      let won = false;
      const round = groups.slice(i, i + DISCOVERY_RACE).map(async (g) => {
        const key = g[0].slice(0, 47);
        const peer = `node-${discoveryN++}`;
        // Registered before the attempt: a refusal can close the link before tryRtc resolves.
        discoveryOpen.set(peer, { key, at: Date.now() });
        const ok = await tryRtc(peer, g, 'p', 'peer', homeGen, () => !won && (won = true));
        if (ok) return;
        discoveryOpen.delete(peer);
        // Losing the race is no failure; a node that never answered rests, and routes another
        // node passed on for it are dropped.
        if (won) return;
        memberWait(key);
        await run(() => { for (const route of g) node?.dial_failed(route); });
      });
      await Promise.all(round);
    }
  } finally {
    discoveryBusy = false;
  }
}

function startLinks() {
  void connectHome();
  if (!watchdog) {
    watchdog = setInterval(() => {
      void run(() => {
        // The check shares the core queue with incoming records, so it sees current links only.
        if (!node || stolen) return;
        for (const [peer, l] of [...links.entries()]) {
          if (node.sync_stalled(peer)) {
            if (peer === 'home') node.set_net_error('Sync stopped making progress. Reconnecting to your home node…');
            l.close();
            lost(peer, l);
          }
        }
      }).then(netStatus, () => undefined);
    }, SYNC_WATCHDOG_MS);
  }
  if (!membersTimer) {
    membersTimer = setInterval(() => { void maintainMembers(); void maintainDiscovery(); }, 15_000);
    setTimeout(() => { void maintainMembers(); void maintainDiscovery(); }, 3000);
  }
}

// ------------------------------------------------------------------ requests

const ready = init();

async function handle(r: Req): Promise<unknown> {
  await ready;
  switch (r.method) {
    case '__init': {
      webBase = String(r.args[0] ?? '');
      if (__HOI_SANDBOX__ && typeof r.args[1] === 'string' && /^hoi-sandbox:[a-zA-Z0-9_-]{1,128}:(owner|member|watcher)$/.test(r.args[1])) {
        storeName = r.args[1];
        lockName = storeName + ':core';
      }
      const got = await holdLock(false);
      if (!got) {
        blocked = true;
        return null;
      }
      return null;
    }
    case '__vaultStatus': return vaultStatus();
    case '__unlock': return run(() => unlock(String(r.args[0] ?? '')), false);
    // Passkeys: the page asks the authenticator, the vault takes its answer.
    case '__vaultPasskeys': return run(async () => {
      db ??= await openStore(storeName);
      let header: Uint8Array | null;
      try {
        header = vault?.header() ?? vaultHeader(await snapshot(db));
      } finally {
        letStoreGo();
      }
      if (!header) return [];
      return (WasmVault.passkeys(header) as { credential: Uint8Array; salt: Uint8Array }[]).map((p) => ({ credential: b64url(p.credential), salt: b64url(p.salt) }));
    }, false);
    case '__unlockPasskey': return run(() => unlockPasskey(unb64url(String(r.args[0])), unb64url(String(r.args[1]))), false);
    case '__addPasskey': return run(async () => {
      if (!vault || !db || stolen) throw new Error('Unlock this device first.');
      vault.add_passkey(unb64url(String(r.args[0])), unb64url(String(r.args[1])), unb64url(String(r.args[2])));
      await saveHeader(db, vault);
    }, false);
    case '__removePasskeys': return run(async () => {
      if (!vault || !db || stolen) throw new Error('Unlock this device first.');
      vault.remove_passkeys();
      await saveHeader(db, vault);
    }, false);
    case '__resetVault': return run(async () => {
      if (blocked || stolen) throw new Error('Open hoi in this tab before clearing its local data.');
      if (r.args[0] !== 'RESET') throw new Error('Type RESET to confirm clearing local data.');
      if (node && !storageFailed) throw new Error('Lock hoi before using recovery.');
      releaseRuntime();
      await resetStore(storeName, 'RESET');
      storageFailed = false;
    }, false);
    case '__lock': return run(async () => { await settle(); releaseRuntime(); });
    case '__takeOver': {
      if (blocked || stolen) {
        await holdLock(true);
        blocked = false;
        stolen = false;
      }
      return vaultStatus();
    }
    case 'boot': {
      if (blocked && !node) {
        return { platform: 'web', version: '', protocol: 1, hasIdentity: false, blockedByOtherTab: true };
      }
      return run(() => call('boot', []));
    }
    case 'takeOver': {
      if (!node) {
        await holdLock(true);
        blocked = false;
        stolen = false;
        return { platform: 'web', version: '', protocol: 1, hasIdentity: false, blockedByOtherTab: false };
      }
      return run(() => call('boot', []));
    }
    case 'pair':
    case 'unpair': {
      const res = await run(() => call(r.method, r.args));
      backoff = 1000;
      void connectHome();
      return res;
    }
    case 'publicJoin': {
      const result = await run(() => call(r.method, r.args));
      // An explicit retry should not wait behind a previous transport backoff. Prefer
      // the selected public carrier if the bounded member slots are occupied elsewhere.
      const peers = r.args[1] as string[];
      preferredMembers = new Set(peers.filter(p => p.startsWith('rtc:')).map(p => p.slice(0, 47)));
      for (const key of preferredMembers) memberRetry.delete(key);
      if (preferredMembers.size && ![...memberOpen.values()].some(m => preferredMembers.has(m.key)) && memberOpen.size >= MAX_MEMBER_LINKS) {
        const spare = [...memberOpen].find(([, m]) => !preferredMembers.has(m.key));
        if (spare) dropLink(spare[0]);
      }
      void maintainMembers();
      return result;
    }
    case 'openNodeLink': {
      const result = await run(() => call(r.method, r.args));
      // Opened on purpose: tried now, ahead of any wait left from an earlier attempt.
      const opened = String(r.args[0] ?? '').match(/[?&,=]([A-Za-z0-9_-]{43})@/g) ?? [];
      for (const m of opened) memberRetry.delete(`rtc:${m.slice(1, 44)}`);
      const spare = [...discoveryOpen.keys()][0];
      if (spare) dropLink(spare);
      void maintainDiscovery();
      return result;
    }
    case 'acceptNotice': {
      // The person read a node's notice: dial it now, whatever wait an earlier closing left.
      const result = await run(() => call(r.method, r.args));
      memberRetry.clear();
      void maintainDiscovery();
      void maintainMembers();
      return result;
    }
    case 'setSettings': {
      const res = (await run(() => call(r.method, r.args))) as { directLinks?: boolean };
      // direct links turned off: the ones open close now
      if (!res.directLinks) for (const id of [...directs.keys()]) dropDirect(id);
      return res;
    }
    case 'wipe': {
      const res = await run(() => call('wipe', []));
      closeAll();
      startLinks();
      return res;
    }
    default:
      return run(() => call(r.method, r.args));
  }
}

type Looked = { kind: 'looked'; id: string; look: unknown };

self.onmessage = (e: MessageEvent<Req | ({ kind: 'rtc' } & Parameters<typeof onRtcEvent>[0]) | Looked>) => {
  const d = e.data;
  if ('kind' in d && d.kind === 'rtc') {
    onRtcEvent(d);
    return;
  }
  if ('kind' in d && d.kind === 'looked') {
    void run(() => node?.looked(d.id, JSON.stringify(d.look))).catch(() => undefined);
    return;
  }
  const r = d as Req;
  handle(r).then(
    (result) => post({ kind: 'reply', id: r.id, ok: true, result }),
    (err) => post({ kind: 'reply', id: r.id, ok: false, error: toError(err) }),
  );
};
