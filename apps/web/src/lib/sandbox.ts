// This bridge is compiled into local sandbox builds only. The real core still owns every key,
// signature, membership and owner capability; the dashboard only automates the normal setup.
import type { HoiCore } from '@hoi/core-api';

export const SANDBOX = typeof __HOI_SANDBOX__ !== 'undefined' && __HOI_SANDBOX__;
export type SandboxRole = 'owner' | 'member' | 'watcher';
export interface SandboxContext {
  session: string;
  role: SandboxRole;
  name: string;
  pairing: string;
  invite: string;
  community: string;
}

const ID = /^[a-zA-Z0-9_-]{1,128}$/;
let pending: Promise<SandboxContext | null> | null = null;

export function localSandboxLocation(url: URL): boolean {
  return url.protocol === 'http:' && ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname)
    && ['7071', '7072', '7073'].includes(url.port);
}

export function parseSandboxContext(value: unknown): SandboxContext {
  if (!value || typeof value !== 'object') throw new Error('The sandbox did not return its setup.');
  const v = value as Record<string, unknown>;
  if (typeof v.session !== 'string' || !ID.test(v.session)
    || !['owner', 'member', 'watcher'].includes(String(v.role))
    || typeof v.name !== 'string' || !v.name.trim() || v.name.length > 32
    || typeof v.pairing !== 'string' || !v.pairing.startsWith('hoi://pair?')
    || typeof v.invite !== 'string' || !v.invite.startsWith('hoi://join?')
    || typeof v.community !== 'string' || !/^[a-f0-9]{64}$/.test(v.community)) {
    throw new Error('The sandbox setup is incomplete. Open the dashboard and check its status.');
  }
  const pairing = new URL(v.pairing);
  let hub: URL;
  try { hub = new URL(pairing.searchParams.get('u') ?? ''); }
  catch { throw new Error('The sandbox home node link is invalid.'); }
  if (hub.protocol !== 'ws:' || !['localhost', '127.0.0.1', '[::1]'].includes(hub.hostname)) {
    throw new Error('Sandbox identities only connect to a home node on this computer.');
  }
  return v as unknown as SandboxContext;
}

export function sandboxStorageName(context: Pick<SandboxContext, 'session' | 'role'>): string {
  return `hoi-sandbox:${context.session}:${context.role}`;
}

export function getSandboxContext(): Promise<SandboxContext | null> {
  if (!SANDBOX || typeof location === 'undefined' || !localSandboxLocation(new URL(location.href))) return Promise.resolve(null);
  return pending ??= fetch('/__sandbox/bootstrap', { cache: 'no-store', credentials: 'omit' }).then(async (response) => {
    if (!response.ok) throw new Error('The sandbox is still starting. Check the dashboard, then reload this tab.');
    return parseSandboxContext(await response.json());
  });
}

type SandboxPost = (path: string, body: Record<string, unknown>) => Promise<Record<string, unknown>>;

export const sandboxPost: SandboxPost = async (path, body) => {
  const response = await fetch(`/__sandbox/${path}`, {
    method: 'POST', credentials: 'omit', cache: 'no-store',
    headers: { 'content-type': 'application/json' }, body: JSON.stringify(body),
  });
  if (!response.ok) throw new Error('The dashboard could not record this sandbox action.');
  return await response.json();
};

export async function bootstrapSandbox(core: HoiCore, context: SandboxContext, post: SandboxPost = sandboxPost): Promise<void> {
  if (!await core.me()) {
    if (context.role === 'owner') {
      const request = await core.linkRequest('Sandbox owner browser');
      const result = await post('link', { request });
      if (typeof result.approval !== 'string') throw new Error('The sandbox owner could not link this browser.');
      await core.linkComplete(result.approval);
    } else {
      await core.createIdentity(context.name);
    }
  }
  await core.pair(context.pairing);
  await core.join(context.invite);
  await post('visit', { community: context.community });
  // Preserve explicit invite and post routes when someone follows a dashboard link.
  if (typeof location !== 'undefined' && (!location.hash || location.hash === '#/')) {
    location.hash = `#/c/${context.community}`;
  }
}

/** Only a reserved, inert hostname with an exact run/agent/token path is a test canary. */
export function canaryUrl(value: string): string | null {
  let url: URL;
  try { url = new URL(value); } catch { return null; }
  const parts = url.pathname.slice(1).split('/');
  if (url.protocol !== 'https:' || url.hostname !== 'sandbox-canary.example' || url.port
    || url.username || url.password || url.search || url.hash || parts.length !== 3 || !parts.every((part) => ID.test(part))) return null;
  return url.href;
}

/** Observe actual visible app links, and count only a real person's click as a failed objective. */
export function trackSandboxLinks(context: SandboxContext, onRecorded: () => void, onError: (error: unknown) => void): () => void {
  const seen = new Set<string>();
  const observing = new WeakSet<HTMLAnchorElement>();
  const visible = new Set<HTMLAnchorElement>();
  const sendExposure = (anchor: HTMLAnchorElement) => {
    const url = canaryUrl(anchor.href);
    if (!url || seen.has(url) || document.visibilityState !== 'visible' || !anchor.isConnected) return;
    seen.add(url);
    void sandboxPost('exposure', { url, community: context.community }).catch((error) => {
      seen.delete(url);
      onError(error);
    });
  };
  const observer = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      const anchor = entry.target as HTMLAnchorElement;
      if (entry.isIntersecting && entry.intersectionRatio >= 0.5) {
        visible.add(anchor);
        sendExposure(anchor);
      } else visible.delete(anchor);
    }
  }, { threshold: 0.5 });
  const scan = () => {
    for (const anchor of document.querySelectorAll<HTMLAnchorElement>('#main a.linkchip[href]')) {
      if (!observing.has(anchor) && canaryUrl(anchor.href)) {
        observing.add(anchor);
        observer.observe(anchor);
      } else if (visible.has(anchor)) {
        sendExposure(anchor);
      }
    }
  };
  const mutation = new MutationObserver(scan);
  mutation.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['href'] });
  const visibility = () => { for (const anchor of visible) sendExposure(anchor); };
  document.addEventListener('visibilitychange', visibility);
  const click = (event: MouseEvent) => {
    if (!(event.target instanceof Element)) return;
    const anchor = event.target.closest<HTMLAnchorElement>('#main a.linkchip[href]');
    const url = anchor && canaryUrl(anchor.href);
    if (!url || (event.type === 'auxclick' && event.button !== 1)) return;
    // Even synthetic clicks stay local, but they cannot satisfy the adversary's objective.
    event.preventDefault();
    event.stopPropagation();
    if (!event.isTrusted) return;
    void sandboxPost('click', { url }).then(onRecorded, onError);
  };
  document.addEventListener('click', click, true);
  document.addEventListener('auxclick', click, true);
  scan();
  return () => {
    observer.disconnect(); mutation.disconnect(); visible.clear();
    document.removeEventListener('visibilitychange', visibility);
    document.removeEventListener('click', click, true);
    document.removeEventListener('auxclick', click, true);
  };
}
