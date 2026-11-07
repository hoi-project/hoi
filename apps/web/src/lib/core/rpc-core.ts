// HoiCore over a message transport: the web worker (hoi-wasm) and the Tauri shell (native
// hoi-core) both speak `call(method, args)` to the core's JSON dispatcher. Records that need a
// real stamp (posts, replies, edits) go draft → mine → publish so mining can run on every core
// and the UI can show the work bar. A new key's identity work goes the same way, so the first
// run can show every try.
import {
  HoiError,
  stampEstimateMs,
  type CoreEvent,
  type ErrorCode,
  type HoiCore,
  type IdentityDraft,
  type IdentityTry,
  type MediaBlob,
  type Me,
  type PostDraft,
  type WorkProgress,
} from '@hoi/core-api';

export interface Transport {
  /** Rejects with `{ code, message }`. */
  call(method: string, args: unknown[]): Promise<unknown>;
  subscribe(fn: (e: CoreEvent) => void): void;
}

export interface Stamper {
  /** Hashes per second, for the work-bar estimate. */
  rate(): Promise<number>;
  mine(prefix: Uint8Array, suffix: Uint8Array, bits: number): Promise<number>;
  /** A new key's identity work. Hosts that can, report every try. */
  mineIdentity(rootKey: string, bits: number, onTry?: (t: IdentityTry) => void): Promise<number>;
  /** More work for a key that has some, which can take minutes: outside the core, so the app
   * keeps answering, reporting progress and stopping on abort (then it resolves null). Hosts
   * without it leave the work to the core. */
  mineMore?(rootKey: string, bits: number, onProgress: (p: WorkProgress) => void, signal?: AbortSignal): Promise<number | null>;
}

const stopped = () => new DOMException('The work was stopped.', 'AbortError');

interface DraftDto {
  token: number;
  prefix: string;
  suffix: string;
  bits: number;
}

export function toBase64Url(u8: Uint8Array): string {
  let s = '';
  for (let i = 0; i < u8.length; i += 0x8000) s += String.fromCharCode(...u8.subarray(i, i + 0x8000));
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

export function fromBase64Url(s: string): Uint8Array {
  const b = atob(s.replace(/-/g, '+').replace(/_/g, '/') + '==='.slice((s.length + 3) % 4));
  const u8 = new Uint8Array(b.length);
  for (let i = 0; i < b.length; i++) u8[i] = b.charCodeAt(i);
  return u8;
}

export function toHoiError(e: unknown): HoiError {
  if (e instanceof HoiError) return e;
  if (e && typeof e === 'object' && 'code' in e) {
    const x = e as { code: string; message?: string };
    return new HoiError(x.code as ErrorCode, x.message ?? x.code);
  }
  return new HoiError('internal', String((e as Error)?.message ?? e));
}

/** Media bytes cross the JSON boundary as base64url. */
function wireDraft(d: PostDraft): unknown {
  return { ...d, media: d.media?.map((m) => ({ data: toBase64Url(m.data), alt: m.alt ?? null })) };
}

/** Hosts whose core looks at media elsewhere (the browser) pass one. */
export interface ComposeLooker {
  compose(data: Uint8Array): Promise<{ hash: string; mime: string; look: unknown } | null>;
}

export function createRpcCore(t: Transport, stamper: Stamper, looker?: ComposeLooker): HoiCore {
  const listeners = new Set<(e: CoreEvent) => void>();
  const emit = (e: CoreEvent) => listeners.forEach((l) => l(e));
  t.subscribe(emit);

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const c = async (method: string, ...args: unknown[]): Promise<any> => {
    try {
      return await t.call(method, args);
    } catch (e) {
      throw toHoiError(e);
    }
  };

  /** Records sent to the computer to be signed, waiting for its answer. */
  const waiting = new Map<string, { done: (v: unknown) => void; fail: (e: unknown) => void }>();
  t.subscribe((e) => {
    if (e.type !== 'signed') return;
    const w = waiting.get(e.id);
    if (!w) return;
    waiting.delete(e.id);
    if (e.ok) void c('published', e.id).then(w.done, w.fail);
    else w.fail(toHoiError({ code: 'not-allowed', message: e.message ?? "Your computer didn't sign it." }));
  });
  function signedThere(id: string): Promise<unknown> {
    return new Promise((done, fail) => {
      waiting.set(id, { done, fail });
      setTimeout(() => {
        if (waiting.delete(id)) fail(toHoiError({ code: 'offline', message: "Your computer didn't answer. Is it on and connected?" }));
      }, 120_000);
    });
  }

  async function stamped<T>(draftMethod: string, args: unknown[]): Promise<T> {
    const d = (await c(draftMethod, ...args)) as DraftDto;
    const estimateMs = stampEstimateMs(d.bits, await stamper.rate());
    emit({ type: 'stamp', phase: 'start', bits: d.bits, estimateMs });
    const t0 = performance.now();
    let tookMs = 0;
    try {
      let nonce: number;
      try {
        nonce = await stamper.mine(fromBase64Url(d.prefix), fromBase64Url(d.suffix), d.bits);
      } catch {
        // The miners couldn't run here: the core stamps it itself (slower, one thread).
        nonce = (await c('mine', d.prefix, d.suffix, d.bits)) as number;
      }
      tookMs = Math.round(performance.now() - t0);
      const published = await c('publishDraft', d.token, nonce);
      // A browser whose computer signs for it: wait for the signature, then show it.
      if (published && typeof published === 'object' && 'signing' in published) return (await signedThere(published.signing as string)) as T;
      return published as T;
    } finally {
      emit({ type: 'stamp', phase: 'done', bits: d.bits, estimateMs, tookMs: tookMs || Math.round(performance.now() - t0) });
    }
  }

  return {
    boot: () => c('boot'),
    takeOver: () => c('takeOver'),
    on(fn) {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },

    me: () => c('me'),
    createIdentity: async (name, opts) => {
      // The key is made in the core, from the system's randomness with the stir mixed in.
      const stir = opts?.stir?.length ? toBase64Url(opts.stir) : null;
      const d = (await c('draftIdentity', name, stir)) as IdentityDraft;
      opts?.onKey?.(d);
      let work: number;
      try {
        work = await stamper.mineIdentity(d.rootKey, d.bits, opts?.onTry);
      } catch {
        // The miners couldn't run here: the core does the work itself (slower, unseen).
        return (await c('createIdentity', name, stir)) as Me;
      }
      return (await c('publishIdentity', d.rootKey, work)) as Me;
    },
    setName: (name) => c('setName', name),
    setAvatar: (image) => c('setAvatar', image ? toBase64Url(image) : null),
    setAvatarColor: (color) => c('setAvatarColor', color),
    exportIdentity: async (passphrase) => fromBase64Url(await c('exportIdentity', passphrase)),
    importIdentity: (data, passphrase) => c('importIdentity', toBase64Url(data), passphrase),
    recoveryKey: () => c('recoveryKey'),
    forgetRecoveryKey: () => c('forgetRecoveryKey'),
    recoveryStatus: () => c('recoveryStatus'),
    // The person comes back from the line under a new root; the core mines the small
    // identity work a new root needs itself.
    restoreIdentity: (recoveryKey, name) => c('restoreIdentity', recoveryKey, name),
    shareCandidates: (community) => c('shareCandidates', community),
    giveShares: (community, friends) => c('giveShares', community, friends),
    heldShares: () => c('heldShares'),
    askFriends: () => c('askFriends'),
    requestDigits: (code) => c('requestDigits', code),
    answerRequest: (person, code) => c('answerRequest', person, code),
    takeAnswer: (code, name) => c('takeAnswer', code, name),
    linkRequest: (label) => c('linkRequest', label),
    linkApprove: (request) => c('linkApprove', request),
    linkComplete: (approval) => c('linkComplete', approval),
    linkHome: (label) => c('linkHome', label),
    linkHomeRemote: (label) => c('linkHomeRemote', label),
    signAsks: () => c('signAsks'),
    signAnswer: (id, yes) => c('signAnswer', id, yes),
    linkCancel: async () => {
      await c('linkCancel');
    },
    linkAsks: () => c('linkAsks'),
    linkAnswer: async (id, yes) => {
      await c('linkAnswer', id, yes);
    },
    revokeDevice: (key) => c('revokeDevice', key),
    setDeviceLabel: (key, label) => c('setDeviceLabel', key, label),
    strengthenIdentity: async (bits, opts) => {
      const me = (await c('me')) as Me | null;
      if (!me?.hasRoot || !stamper.mineMore || me.identityBits >= bits) return c('strengthenIdentity', bits);
      let work: number | null;
      try {
        // For the root the core signs the stronger identity with: after a recovery that is not
        // the person's first (rootKey), and work for the first would be refused as too weak.
        work = await stamper.mineMore(me.workKey || me.rootKey, bits, opts?.onProgress ?? (() => undefined), opts?.signal);
      } catch {
        if (opts?.signal?.aborted) throw stopped();
        // The miners couldn't run here: the core does the work itself (slower, unseen).
        return c('strengthenIdentity', bits);
      }
      if (work === null || opts?.signal?.aborted) throw stopped();
      return c('strengthenIdentity', bits, work);
    },
    burnIdentity: async (confirm) => {
      await c('burnIdentity', confirm);
    },

    communities: () => c('communities'),
    discoverCommunities: () => c('discoverCommunities'),
    openNodeLink: (link) => c('openNodeLink', link),
    previewNodeLink: (link) => c('previewNodeLink', link),
    nodeNotices: () => c('nodeNotices'),
    acceptNotice: (node, hash) => c('acceptNotice', node, hash),
    declineNotice: (node) => c('declineNotice', node),
    forgetNotice: async (node) => {
      await c('forgetNotice', node);
    },
    communityRegistration: (id) => c('communityRegistration', id),
    communityPublication: (id) => c('communityPublication', id),
    publicJoin: (id, peers, name) => c('publicJoin', id, peers, name),
    publicJoinStatus: (id) => c('publicJoinStatus', id),
    warmth: () => c('warmth'),
    communityWarmth: (id) => c('communityWarmth', id),
    requestCarry: (id) => c('requestCarry', id),
    community: (id) => c('community', id),
    createCommunity: (draft) => c('createCommunity', draft),
    createPrivateCommunity: (draft) => c('createPrivateCommunity', draft),
    privateGroups: () => c('privateGroups'),
    privateGroup: (id) => c('privateGroup', id),
    privateJoinRequest: () => c('privateJoinRequest'),
    privatePreviewRequest: (request) => c('privatePreviewRequest', request),
    privateInvite: (id, request) => c('privateInvite', id, request),
    privateJoin: (invite) => c('privateJoin', invite),
    privateRemove: (id, device) => c('privateRemove', id, device),
    privatePassRequest: (id, request) => c('privatePassRequest', id, request),
    privateCopy: (id) => c('privateCopy', id),
    privateOutbox: (id) => c('privateOutbox', id),
    privateReceive: (id, body) => c('privateReceive', id, body),
    privateAck: (id, ids) => c('privateAck', id, ids),
    privateRead: (id, targets) => c('privateRead', id, targets),
    previewInvite: (link) => c('previewInvite', link),
    join: (link) => c('join', link),
    leave: async (id) => {
      await c('leave', id);
    },
    setPinning: (id, on) => c('setPinning', id, on),
    invite: (id) => c('invite', id),
    exportBundle: async (id) => fromBase64Url(await c('exportBundle', id)),
    importBundle: (data) => c('importBundle', toBase64Url(data)),

    setLimits: (id, limits) => c('setLimits', id, limits),
    setRules: (id, text) => c('setRules', id, text),
    addMod: (id, key) => c('addMod', id, key),
    removeMod: (id, key) => c('removeMod', id, key),
    retire: (id, key) => c('retire', id, key),
    cosign: (proposal) => c('cosign', proposal),

    ledger: (id) => c('ledger', id),
    remove: (target, reason) => c('remove', target, reason),
    pin: (target, on) => c('pin', target, on),
    label: (target, label, reason) => c('label', target, label, reason ?? null),
    ban: (community, key, reason) => c('ban', community, key, reason),
    unban: (community, key, reason) => c('unban', community, key, reason),
    admit: (community, key) => c('admit', community, key),

    standing: (community, key) => c('standing', community, key ?? null),
    vouch: (community, key, value, reason) => c('vouch', community, key, value, reason ?? null),
    review: async (target, verdict, reason) => {
      await c('review', target, verdict, reason ?? null);
    },
    reviewQueue: (community) => c('reviewQueue', community),
    media: async (hash) => {
      const b = (await c('media', hash)) as Omit<MediaBlob, 'data'> & { data: string };
      return { ...b, data: fromBase64Url(b.data) };
    },

    feed: (q) => c('feed', q),
    post: (id) => c('post', id),
    createPost: async (draft) => {
      // pictures are looked at outside the core first, and the core told what was seen
      for (const m of looker ? draft.media ?? [] : []) {
        const r = await looker!.compose(m.data);
        if (r === null) throw new HoiError('invalid', "That picture can't be read here, so nobody else could check it either. Try saving it as JPEG or PNG.");
        if (r.hash) await c('looked', r.hash, r.look);
      }
      return stamped('draftPost', [wireDraft(draft)]);
    },
    reply: (parent, text) => stamped('draftReply', [parent, text]),
    edit: async (id, text, title) => {
      await stamped('draftEdit', [id, text, title ?? null]);
    },
    deleteOwn: async (id) => {
      await c('deleteOwn', id);
    },
    vote: (target, value) => c('vote', target, value),
    flag: async (target, reason) => {
      await c('flag', target, reason);
    },
    save: async (id, on) => {
      await c('save', id, on);
    },
    saved: () => c('saved'),
    search: (q, community) => c('search', q, community ?? null),

    person: (key, community) => c('person', key, community ?? null),
    setTrust: (key, friend) => c('setTrust', key, friend),
    trustSharing: (community) => c('trustSharing', community),
    shareTrust: (community, on) => c('shareTrust', community, on),
    labelers: () => c('labelers'),
    followLabeler: (key, on) => c('followLabeler', key, on),
    publishLabel: (target, label, note) => c('publishLabel', target, label, note ?? null),
    linkPreview: (url) => c('linkPreview', url),
    wikiPages: (community) => c('wikiPages', community),
    ageChallenge: (community) => c('ageChallenge', community),
    blocklistKeepers: () => c('blocklistKeepers'),
    followBlocklist: (key, on) => c('followBlocklist', key, on),
    publishBlocklist: (community, text) => c('publishBlocklist', community, text),
    aggregators: () => c('aggregators'),
    followAggregator: (key, on) => c('followAggregator', key, on),
    aggregating: (community) => c('aggregating', community),
    setAggregating: (community, on) => c('setAggregating', community, on),
    ageProofCode: (community, presentation) => c('ageProofCode', community, presentation),
    admitByAge: (code) => c('admitByAge', code),
    wikiPage: (community, page) => c('wikiPage', community, page),
    editWiki: (community, page, text) => c('editWiki', community, page, text),
    setMuted: (key, muted) => c('setMuted', key, muted),
    setPetname: (key, petname) => c('setPetname', key, petname),

    stats: () => c('stats'),
    settings: () => c('settings'),
    setSettings: (patch) => c('setSettings', patch),
    net: () => c('net'),
    traffic: () => c('traffic'),
    pair: (link) => c('pair', link),
    previewPair: (link) => c('previewPair', link),
    unpair: () => c('unpair'),
    rotatePairing: () => c('rotatePairing'),
    previewPublicNode: (on) => c('previewPublicNode', on),
    vpnInterfaces: () => c('vpnInterfaces'),
    importBlocklist: (text) => c('importBlocklist', text),
    wipe: async () => {
      await c('wipe');
    },
  };
}
