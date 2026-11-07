// Web backend: hoi-wasm in a Web Worker (core.worker.ts), records in IndexedDB, stamps mined by
// a small pool of workers (miner-pool.ts), your home node over a WebSocket or WebRTC. The page
// carries the worker's WebRTC links (workers can't open them): rtc-link.ts.
import type { CoreEvent, HoiCore } from '@hoi/core-api';
import { createLooker } from './look-pool';
import { createMinerPool } from './miner-pool';
import { createRpcCore } from './rpc-core';
import { openRtc, type RtcLink, type RtcMode } from './rtc-link';
import { answerDirect, offerDirect, type DirectHandlers, type DirectLink } from './rtc-direct';
import { SANDBOX, getSandboxContext, sandboxStorageName } from '../sandbox';

type Reply = { kind: 'reply'; id: number; ok: true; result: unknown } | { kind: 'reply'; id: number; ok: false; error: { code: string; message: string } };
type FromWorker =
  | Reply
  | { kind: 'locked' }
  | { kind: 'event'; event: CoreEvent }
  | { kind: 'rtc-open'; id: number; routes: string[]; mode: RtcMode }
  | { kind: 'rtc-send'; id: number; bytes: Uint8Array }
  | { kind: 'rtc-close'; id: number }
  | { kind: 'direct-offer'; id: number; stun: string[] }
  | { kind: 'direct-answer'; id: number; stun: string[]; data: Uint8Array }
  | { kind: 'direct-accept'; id: number; data: Uint8Array }
  | { kind: 'look'; id: string; mime: string; data: Uint8Array };

interface WorkerSession { core: HoiCore; call(method: string, args: unknown[]): Promise<unknown>; destroy(): void; }
let session: Promise<WorkerSession> | null = null;
const getSession = () => session ??= connect();
export async function createWorkerCore(): Promise<HoiCore> { return (await getSession()).core; }
export async function workerVaultCall(method: string, args: unknown[] = []): Promise<unknown> {
  const connected = await getSession();
  const result = await connected.call(method, args);
  if (method === '__lock') { connected.destroy(); session = null; }
  return result;
}

async function connect(): Promise<WorkerSession> {
  const worker = new Worker(new URL('./core.worker.ts', import.meta.url), { type: 'module', name: 'hoi-core' });
  const pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: unknown) => void }>();
  const subscribers: ((e: CoreEvent) => void)[] = [];
  let next = 1;

  const rtc = new Map<number, Pick<RtcLink, 'send' | 'close'> | null>();
  /** Direct links with other browsers, by the worker's id, while they open too. */
  const direct = new Map<number, DirectLink>();
  const looker = createLooker();
  function destroy() {
    worker.terminate();
    for (const link of rtc.values()) link?.close();
    rtc.clear();
    for (const link of direct.values()) link.close();
    direct.clear();
    for (const request of pending.values()) request.reject({ code: 'offline', message: 'This device is locked.' });
    pending.clear();
  }
  const toWorker = (m: { ev: 'open' | 'msg' | 'close' | 'sig'; id: number; bytes?: Uint8Array; reason?: string; binding?: RtcLink['binding'] }) =>
    worker.postMessage({ kind: 'rtc', ...m }, m.bytes ? [m.bytes.buffer as ArrayBuffer] : []);
  const directHandlers = (id: number): DirectHandlers => ({
    open: () => {
      const link = direct.get(id);
      if (!link) return;
      rtc.set(id, link);
      toWorker({ ev: 'open', id });
    },
    message: (bytes) => toWorker({ ev: 'msg', id, bytes }),
    close: (reason) => {
      direct.delete(id);
      rtc.delete(id);
      toWorker({ ev: 'close', id, reason });
    },
  });
  const directFailed = (id: number) => (err: unknown) => {
    direct.delete(id);
    toWorker({ ev: 'close', id, reason: String((err as Error)?.message ?? err) });
  };

  worker.onmessage = (e: MessageEvent<FromWorker>) => {
    const m = e.data;
    if (m.kind === 'locked') { destroy(); location.reload(); return; }
    if (m.kind === 'event') {
      subscribers.forEach((s) => s(m.event));
      return;
    }
    if (m.kind === 'rtc-open') {
      rtc.set(m.id, null);
      openRtc(m.routes, m.mode, {
        message: (bytes) => toWorker({ ev: 'msg', id: m.id, bytes }),
        close: (reason) => {
          rtc.delete(m.id);
          toWorker({ ev: 'close', id: m.id, reason });
        },
      }).then(
        (link) => {
          if (!rtc.has(m.id)) return link.close(); // closed while it was opening
          rtc.set(m.id, link);
          toWorker({ ev: 'open', id: m.id, binding: link.binding });
        },
        (err) => {
          rtc.delete(m.id);
          toWorker({ ev: 'close', id: m.id, reason: String((err as Error)?.message ?? err) });
        },
      );
      return;
    }
    if (m.kind === 'direct-offer') {
      offerDirect(m.stun, directHandlers(m.id)).then(({ offer, link }) => {
        direct.set(m.id, link);
        toWorker({ ev: 'sig', id: m.id, bytes: offer });
      }, directFailed(m.id));
      return;
    }
    if (m.kind === 'direct-answer') {
      answerDirect(m.stun, m.data, directHandlers(m.id)).then(({ answer, link }) => {
        direct.set(m.id, link);
        toWorker({ ev: 'sig', id: m.id, bytes: answer });
      }, directFailed(m.id));
      return;
    }
    if (m.kind === 'direct-accept') {
      void direct.get(m.id)?.accept(m.data).catch(() => undefined);
      return;
    }
    if (m.kind === 'look') {
      // media the core wants looked at outside itself
      void looker.look(m.mime, m.data).then((look) => worker.postMessage({ kind: 'looked', id: m.id, look }));
      return;
    }
    if (m.kind === 'rtc-send') {
      rtc.get(m.id)?.send(m.bytes);
      return;
    }
    if (m.kind === 'rtc-close') {
      const link = rtc.get(m.id) ?? direct.get(m.id);
      rtc.delete(m.id);
      direct.delete(m.id);
      link?.close();
      return;
    }
    const p = pending.get(m.id);
    if (!p) return;
    pending.delete(m.id);
    if (m.ok) p.resolve(m.result);
    else p.reject(m.error);
  };
  worker.onerror = (e) => {
    const err = { code: 'internal', message: `The core worker failed: ${e.message || 'unknown error'}` };
    for (const p of pending.values()) p.reject(err);
    pending.clear();
  };

  const call = (method: string, args: unknown[]) =>
    new Promise<unknown>((resolve, reject) => {
      const id = next++;
      pending.set(id, { resolve, reject });
      try {
        // The core reads its arguments as JSON. A plain copy also keeps Svelte's state proxies
        // (a form's draft), which postMessage can't clone, out of the message.
        worker.postMessage({ id, method, args: JSON.parse(JSON.stringify(args)) });
      } catch (e) {
        pending.delete(id);
        reject({ code: 'internal', message: String((e as Error)?.message ?? e) });
      }
    });

  // Invite links point back at this app, wherever it is hosted.
  const base = location.origin + location.pathname;
  const sandbox = SANDBOX ? await getSandboxContext() : null;
  await call('__init', [base, sandbox ? sandboxStorageName(sandbox) : 'hoi']);

  return { core: createRpcCore({ call, subscribe: (fn) => void subscribers.push(fn) }, createMinerPool(), looker), call, destroy };
}
