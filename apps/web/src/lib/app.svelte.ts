// The app's small reactive store: the core, who you are, your communities, the network and settings.
// Screens read from here and refetch their own data when `rev` bumps (a `changed` event arrived).
import type {
  Boot,
  CommunitySummary,
  CoreEvent,
  Hex,
  HoiCore,
  LinkAsk,
  Me,
  NetStatus,
  NodeStats,
  PrivateGroup,
  Settings,
  WarmthView,
} from '@hoi/core-api';
import { getCore, getVaultStatus, unlockVault, unlockWithPasskey, unlockWithOs, takeOverVault, lockVault, type VaultStatus } from './core';
import { errorText, toast, toastError } from './toast.svelte';
import { SANDBOX, bootstrapSandbox, getSandboxContext } from './sandbox';
import { canPublishPublic, carriersOf, postsThroughOthers, savedForSync } from './network-status';
import { PublicConnectionDiscovery } from './public-connections';
import { REGISTRY_URL, registryClient } from './registry';
import { RELAY_URLS, syncPrivateGroup, type PrivateSyncStatus } from './private-relay';
import { networkConsent } from './consent.svelte';

type Phase = 'booting' | 'blocked' | 'error' | 'locked' | 'ready';
type StampEvent = Extract<CoreEvent, { type: 'stamp' }>;

const THEME_KEY = 'hoi:theme';
const MOTION_KEY = 'hoi:motion';

class AppState {
  phase: Phase = $state('booting');
  bootError = $state('');
  vaultState: VaultStatus['state'] = $state('locked');
  boot: Boot | null = $state.raw(null);
  me: Me | null = $state.raw(null);
  communities: CommunitySummary[] = $state.raw([]);
  /** Bumped when a browser this computer signs for asks for a yes. */
  signAskRev = $state(0);
  /** Bumped when a link preview arrives. */
  previewRev = $state(0);
  privateGroups: PrivateGroup[] = $state.raw([]);
  privateSync: Record<string, PrivateSyncStatus> = $state.raw({});
  net: NetStatus | null = $state.raw(null);
  publicConnection = $state({ busy: false, message: '' });
  /** How safe each public community's records are, from this device's own links (PROTOCOL.md §20). */
  warmth: WarmthView | null = $state.raw(null);
  /** What the registry said last, for the directory's Warmth reading. */
  directory = $state({ registryReachable: false, checkedAt: 0, announcedNodes: 0 });
  settings: Settings | null = $state.raw(null);
  stats: NodeStats | null = $state.raw(null);
  /** Bumps whenever records changed; screens refetch on it. */
  rev = $state(0);
  /** Communities touched by the last `changed` event (empty = anything). */
  lastChanged: Hex[] = $state.raw([]);
  /** Hashes per second, measured from the last real stamp; 22 bits ≈ 1 s on a laptop until then. */
  hashesPerSecond = $state(2 ** 22);
  /** "+ Post" opens the composer on Home (or in the community you're looking at). */
  composer = $state<{ open: boolean; community?: Hex; nonce: number }>({ open: false, nonce: 0 });
  /** Set right after a community is born, so its page opens the invite modal once. */
  inviteFor: Hex | null = $state(null);
  /** Your new devices asking to be linked; this device holds your root key. */
  linkAsks: LinkAsk[] = $state.raw([]);
  /** On a new device asking to be linked: the digits to compare, once a device with your key answered. */
  linkChecks: string[] = $state.raw([]);
  /** Asked once on a browser or phone without a home node, before its first post, reply, vote or
   * vouch: nobody can read it until a home node carrying the community has it (PostingNotice). */
  postingNotice: { community?: Hex; carriers: number } | null = $state.raw(null);
  #postingAnswer: ((ok: boolean) => void) | null = null;

  #core: HoiCore | null = null;
  #off: (() => void) | null = null;
  #stampListeners = new Set<(e: StampEvent) => void>();
  #privateRunning = new Map<string, Promise<PrivateSyncStatus>>();
  #privateTimer: ReturnType<typeof setInterval> | undefined;
  #privateSoon: ReturnType<typeof setTimeout> | undefined;
  #privateAbort = new AbortController();
  #locking = false;
  #publicDiscovery = new PublicConnectionDiscovery(REGISTRY_URL ? () => registryClient(REGISTRY_URL).metadata() : null, Date.now,
    (tracker, community) => registryClient(tracker).nodesFor(community));
  #publicRunning: Promise<void> | undefined;
  #publicTimer: ReturnType<typeof setInterval> | undefined;
  #warmthSoon: ReturnType<typeof setTimeout> | undefined;

  get core(): HoiCore {
    if (!this.#core) throw new Error('core not ready');
    return this.#core;
  }

  get joined(): CommunitySummary[] {
    return this.communities.filter((c) => c.joined && !this.privateGroups.some(g => g.community === c.id && !g.active));
  }

  get platformName(): string {
    switch (this.boot?.platform) {
      case 'desktop': return 'computer';
      case 'mobile': return 'phone';
      default: return 'browser';
    }
  }

  async start(): Promise<void> {
    this.phase = 'booting';
    try {
      const vault = await getVaultStatus();
      this.vaultState = vault.state;
      if (vault.state === 'blocked') { this.phase = 'blocked'; return; }
      if (vault.state !== 'ready') { this.phase = 'locked'; return; }
      this.#core = await getCore();
      const boot = await this.#core.boot();
      this.boot = boot;
      if (boot.blockedByOtherTab) {
        this.phase = 'blocked';
        return;
      }
      if (SANDBOX) {
        const sandbox = await getSandboxContext();
        if (sandbox) await bootstrapSandbox(this.#core, sandbox);
      }
      await this.#ready();
    } catch (e) {
      this.bootError = errorText(e);
      this.phase = 'error';
    }
  }

  async takeOver(): Promise<void> {
    try {
      await takeOverVault();
      await this.start();
    } catch (e) {
      toastError(e);
    }
  }

  /** Opens the vault with a passkey's answer, then starts as after the passphrase. */
  async unlockPasskey(credential: string, prf: string): Promise<void> {
    await unlockWithPasskey(credential, prf);
    await this.start();
  }

  async unlockOs(): Promise<void> {
    await unlockWithOs();
    await this.start();
  }

  async unlock(passphrase: string): Promise<void> {
    try { await unlockVault(passphrase); }
    catch (error) {
      // An atomic migration may already have installed the password, even if the next
      // save failed. Show an unlock form on retry instead of offering to set another one.
      try {
        const status = await getVaultStatus();
        if (status.state !== 'ready') {
          this.vaultState = status.state;
          this.phase = status.state === 'blocked' ? 'blocked' : 'locked';
        }
      } catch { /* Preserve the original storage/authentication error. */ }
      throw error;
    }
    await this.start();
  }

  async lock(): Promise<void> {
    this.#locking = true;
    this.#publicDiscovery.reset();
    clearInterval(this.#publicTimer);
    this.#publicTimer = undefined;
    clearTimeout(this.#warmthSoon);
    this.#warmthSoon = undefined;
    this.#privateAbort.abort();
    if (this.#privateTimer) clearInterval(this.#privateTimer);
    if (this.#privateSoon) clearTimeout(this.#privateSoon);
    this.#privateTimer = this.#privateSoon = undefined;
    await Promise.allSettled(this.#privateRunning.values());
    try { await lockVault(); }
    catch (error) {
      this.#locking = false;
      this.#privateAbort = new AbortController();
      this.#privateTimer = setInterval(() => void this.syncPrivateGroups(), 15_000);
      this.#publicTimer = setInterval(() => void this.connectPublic(), 60_000);
      throw error;
    }
  }

  async #ready(): Promise<void> {
    const core = this.core;
    this.#off?.();
    this.#off = core.on((e) => this.#onEvent(e));
    const [me, settings, net] = await Promise.all([core.me(), core.settings(), core.net()]);
    this.me = me;
    this.applySettings(settings);
    this.net = net;
    if (me) await this.refresh();
    if (me?.hasRoot) this.linkAsks = await core.linkAsks().catch(() => []);
    // The desktop can open as the companion: status, traffic and pairing, without the forum.
    if (this.boot?.platform === 'desktop' && settings.companion && (location.hash === '' || location.hash === '#/')) location.hash = '#/companion';
    this.phase = 'ready';
    if (!this.#publicTimer) this.#publicTimer = setInterval(() => void this.connectPublic(), 60_000);
    void this.connectPublic();
    if (!this.#privateTimer) this.#privateTimer = setInterval(() => void this.syncPrivateGroups(), 15_000);
    void this.syncPrivateGroups();
    void this.refreshWarmth();
  }

  /** Re-read Warmth from the core: after records or connections changed, and after a directory check. */
  async refreshWarmth(): Promise<void> {
    if (!this.#core || this.#locking || !this.me) return;
    try {
      this.warmth = await this.core.warmth();
    } catch (e) {
      console.warn(e);
    }
  }

  /** Warmth changes with every record and connection; one read half a second after the last event is enough. */
  #warmthLater(): void {
    clearTimeout(this.#warmthSoon);
    this.#warmthSoon = setTimeout(() => void this.refreshWarmth(), 500);
  }

  /** Refetch the shared bits (communities, stats). Cheap; called after `changed`. */
  async refresh(): Promise<void> {
    if (!this.#core) return;
    try {
      const [communities, stats, privateGroups] = await Promise.all([this.core.communities(), this.core.stats(), this.core.privateGroups()]);
      this.communities = communities;
      this.stats = stats;
      this.privateGroups = privateGroups;
      void this.connectPublic();
    } catch (e) {
      console.warn(e);
    }
  }

  async refreshMe(): Promise<void> {
    try {
      this.me = await this.core.me();
    } catch (e) {
      toastError(e);
    }
  }

  /** After createIdentity / importIdentity / linkComplete. */
  async identityChanged(me: Me): Promise<void> {
    this.me = me;
    await this.refresh();
    this.rev++;
  }

  async syncPrivate(community: string): Promise<PrivateSyncStatus> {
    if (this.#locking) return { busy: false, replicas: 0, total: RELAY_URLS.length };
    // Mailboxes sync inside the core on their own. Asking for the outbox here gives anything new
    // its place in this device's slots, so a membership change can follow it at once.
    const known = this.privateGroups.find(g => g.community === community);
    if (known && (known.transport === 'mailboxes' || !RELAY_URLS.length)) {
      await this.core.privateOutbox(community).catch(() => undefined);
      if (!this.#locking) await this.refresh();
      const group = this.privateGroups.find(g => g.community === community) ?? known;
      return { busy: false, replicas: group.mailboxes.filter(m => m.linked).length, total: group.mailboxes.length };
    }
    const running = this.#privateRunning.get(community);
    if (running) return running;
    const previous = this.privateSync[community];
    this.privateSync = { ...this.privateSync, [community]: { ...previous, busy: true, replicas: previous?.replicas ?? 0, total: RELAY_URLS.length } };
    const job = syncPrivateGroup(this.core, community, RELAY_URLS, fetch, this.#privateAbort.signal).catch((): PrivateSyncStatus => ({ busy: false, replicas: 0, total: RELAY_URLS.length, error: 'Private syncing could not finish. Your saved data is unchanged.' })).then(async status => {
      this.privateSync = { ...this.privateSync, [community]: { ...status, lastSync: status.lastSync ?? previous?.lastSync } };
      if (!this.#locking) await this.refresh();
      return status;
    }).finally(() => this.#privateRunning.delete(community));
    this.#privateRunning.set(community, job);
    return job;
  }

  async syncPrivateGroups(): Promise<void> {
    if (this.#locking || !this.me || !RELAY_URLS.length) return;
    // Groups on mailboxes need no relay; the core reads and writes their mailboxes itself.
    for (const group of this.privateGroups) if (group.active && group.transport !== 'mailboxes' && !this.#locking) await this.syncPrivate(group.community);
  }

  /** Public relays replace the need for a personal node; membership stays in the core. */
  async connectPublic(community?: string, force = false): Promise<void> {
    if (this.#publicRunning) {
      await this.#publicRunning;
      if (!force) return;
    }
    if (this.#locking || this.phase !== 'ready' || !this.me) return;
    const core = this.core;
    this.publicConnection.busy = true;
    const job = this.#publicDiscovery.connect(core, this.communities, this.privateGroups, this.boot?.platform ?? 'web', this.net,
      { community, force, active: () => !this.#locking && this.#core === core && !!this.me }).then(result => {
        if (result.message) this.publicConnection.message = result.message;
        if (result.registry !== 'unchecked') this.directory = { registryReachable: result.registry === 'reachable', checkedAt: Date.now(), announcedNodes: result.announced };
        this.#warmthLater();
      }).finally(() => { this.publicConnection.busy = false; this.#publicRunning = undefined; });
    this.#publicRunning = job;
    await job;
  }

  /** Re-read live transport state immediately before signing. Private groups keep their
   * encrypted outbox flow. This is UI policy, never a claim that a relay stored the post. */
  async requirePostingConnection(community?: string): Promise<void> {
    if (community && this.privateGroups.some(group => group.community === community)) return;
    this.net = await this.core.net();
    if (!canPublishPublic(this.net, community)) throw new Error('Connect to a home node carrying this community before posting: yours, a member\'s or a public one. Your text stays in the editor.');
    if (!(await this.confirmPosting(community))) throw new Error('Not posted. Your text stays in the editor.');
  }

  /** Before the first signed post, reply, vote or vouch on a browser or phone with no home node of
   * its own: says, once, that nobody can read it until a home node carrying the community has it.
   * Resolves false when the person would rather not (or gets the home node first). */
  confirmPosting(community?: Hex): Promise<boolean> {
    if (community && this.privateGroups.some(group => group.community === community)) return Promise.resolve(true);
    if (!postsThroughOthers(this.net, this.boot?.platform ?? 'web') || this.settings?.postingNoticeSeen) return Promise.resolve(true);
    this.#postingAnswer?.(false);
    return new Promise(done => {
      this.#postingAnswer = done;
      this.postingNotice = { community, carriers: carriersOf(this.net, community) };
    });
  }

  answerPostingNotice(ok: boolean): void {
    const done = this.#postingAnswer;
    this.#postingAnswer = null;
    this.postingNotice = null;
    if (ok) void this.setSettings({ postingNoticeSeen: true });
    done?.(ok);
  }

  /** After answering a link ask, here or on another device. */
  dropAsk(id: number): void {
    this.linkAsks = this.linkAsks.filter((a) => a.id !== id);
  }

  /** After wipe(): back to first run, and the next person reads Network & Safety first. */
  async wiped(): Promise<void> {
    networkConsent.reset();
    this.#publicDiscovery.reset();
    this.publicConnection.message = '';
    this.warmth = null;
    this.directory = { registryReachable: false, checkedAt: 0, announcedNodes: 0 };
    this.me = null;
    this.communities = [];
    this.privateGroups = [];
    this.privateSync = {};
    this.stats = null;
    this.rev++;
    try {
      this.applySettings(await this.core.settings());
      this.net = await this.core.net();
    } catch (e) {
      console.warn(e);
    }
  }

  applySettings(s: Settings): void {
    this.settings = s;
    const root = document.documentElement;
    if (s.theme === 'system') delete root.dataset.theme;
    else root.dataset.theme = s.theme;
    if (s.motion === 'reduced') root.dataset.motion = 'reduced';
    else delete root.dataset.motion;
    try {
      localStorage.setItem(THEME_KEY, s.theme);
      localStorage.setItem(MOTION_KEY, s.motion);
    } catch {
      /* private mode: fine, settings still come from the core */
    }
  }

  async setSettings(patch: Partial<Settings>): Promise<Settings | null> {
    try {
      const s = await this.core.setSettings(patch);
      this.applySettings(s);
      return s;
    } catch (e) {
      toastError(e);
      return null;
    }
  }

  openComposer(community?: Hex): void {
    this.composer = { open: true, community, nonce: this.composer.nonce + 1 };
  }

  closeComposer(): void {
    this.composer = { ...this.composer, open: false };
  }

  /** Subscribe to stamp events (the composer's work bar listens while signing). */
  onStamp(fn: (e: StampEvent) => void): () => void {
    this.#stampListeners.add(fn);
    return () => this.#stampListeners.delete(fn);
  }

  /** Rough wall clock for a stamp of `bits` on this device. */
  stampMs(bits: number): number {
    return (2 ** bits / Math.max(1, this.hashesPerSecond)) * 1000;
  }

  #onEvent(e: CoreEvent): void {
    switch (e.type) {
      case 'changed':
        this.lastChanged = e.communities;
        this.rev++;
        void this.refresh();
        this.#warmthLater();
        clearTimeout(this.#privateSoon);
        this.#privateSoon = setTimeout(() => void this.syncPrivateGroups(), 750);
        break;
      case 'net':
        this.net = e.status;
        void this.refresh();
        this.#warmthLater();
        break;
      case 'stamp':
        if (e.phase === 'done' && e.tookMs && e.bits >= 16) {
          this.hashesPerSecond = 2 ** e.bits / Math.max(0.001, e.tookMs / 1000);
        }
        for (const fn of this.#stampListeners) fn(e);
        break;
      case 'notice':
        toast(e.text);
        break;
      case 'linkAsk':
        this.linkAsks = [...this.linkAsks.filter((a) => a.id !== e.ask.id && a.device !== e.ask.device), e.ask];
        break;
      case 'linkCheck':
        this.linkChecks = e.checks;
        break;
      case 'preview':
        this.previewRev++;
        break;
      case 'signAsk':
        this.signAskRev++;
        toast(`${e.ask.device} asks this ${this.platformName} to sign a ${e.ask.kind === 'ledger' ? 'moderation step' : e.ask.kind}. Answer under You → Your devices.`);
        break;
      case 'linked':
        this.linkChecks = [];
        void this.identityChanged(e.me);
        break;
      case 'nodeNotice':
        // A node asks to be read first: the words are in `net.pendingNotices`.
        void this.refreshNet();
        break;
    }
  }

  async refreshNet(): Promise<void> {
    if (!this.#core || this.#locking) return;
    try {
      this.net = await this.core.net();
    } catch (e) {
      console.warn(e);
    }
  }
}

export const app = new AppState();

/** Apply the remembered theme before the core answers, so the first paint is right. */
export function prepaintTheme(): void {
  try {
    const t = localStorage.getItem(THEME_KEY);
    if (t === 'light' || t === 'dark') document.documentElement.dataset.theme = t;
    if (localStorage.getItem(MOTION_KEY) === 'reduced') document.documentElement.dataset.motion = 'reduced';
  } catch {
    /* ignore */
  }
}

/** Run a core call; toast errors; return undefined on failure. */
export async function attempt<T>(fn: () => Promise<T>, ok?: string | ((v: T) => string)): Promise<T | undefined> {
  try {
    const v = await fn();
    if (ok) toast(typeof ok === 'function' ? ok(v) : ok);
    return v;
  } catch (e) {
    toastError(e);
    return undefined;
  }
}

/** Local acceptance is known; a peer count does not prove delivery. */
export function sentTo(community?: Hex): string {
  if (community && app.privateGroups.some(g => g.community === community)) return 'Saved here for encrypted delivery.';
  return savedForSync(app.net, community, app.boot?.platform ?? 'web');
}

/** Confirm signing without inventing a receipt from connected peers. */
export function signedAndSent(what: string, community?: Hex): string {
  return `${what} signed. ${sentTo(community)}`;
}

/** Page title: 'hoi', 'c/privacy · hoi'. */
export function setTitle(t?: string): void {
  if (typeof document !== 'undefined') document.title = t ? `${t} · hoi` : 'hoi';
}
