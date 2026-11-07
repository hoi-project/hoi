// A small pool of miners (Web Workers running hoi-wasm). Stamps are the "second of work" every
// post carries (PROTOCOL.md §4), identity work the few seconds a new key does once (§4.1);
// splitting the nonce space across cores keeps both short.
import type { IdentityTry, WorkProgress } from '@hoi/core-api';
import { hexToBytes } from '../util';

type Found = { type: 'found'; job: number; nonce: number; tries: number; ms: number };
type Try = { type: 'try'; job: number; work: number; hash: Uint8Array; bits: number };
type Failed = { type: 'failed'; job: number; message: string };
type Bench = { type: 'bench'; hashesPerSecond: number };

export interface MinerPool {
  /** Hashes per second across all miners. */
  rate(): Promise<number>;
  mine(prefix: Uint8Array, suffix: Uint8Array, bits: number): Promise<number>;
  /** A new key's identity work, every try reported as it happens. Resolves to the nonce. */
  mineIdentity(rootKey: string, bits: number, onTry?: (t: IdentityTry) => void, signal?: AbortSignal): Promise<number>;
  /** More work for a key that has some: tries counted a few times a second, null once stopped. */
  mineMore(rootKey: string, bits: number, onProgress: (p: WorkProgress) => void, signal?: AbortSignal): Promise<number | null>;
}

export function createMinerPool(): MinerPool {
  const n = Math.max(1, Math.min(4, (navigator.hardwareConcurrency || 2) - 1));
  const workers = Array.from({ length: n }, () => new Worker(new URL('./miner.worker.ts', import.meta.url), { type: 'module' }));
  let job = 0;
  let measured: Promise<number> | null = null;

  function rate(): Promise<number> {
    return (measured ??= new Promise<number>((resolve) => {
      const w = workers[0];
      const on = (e: MessageEvent<Bench>) => {
        if (e.data.type !== 'bench') return;
        w.removeEventListener('message', on);
        resolve(Math.max(50_000, e.data.hashesPerSecond) * n);
      };
      w.addEventListener('message', on);
      w.postMessage({ type: 'bench' });
    }));
  }

  function mine(prefix: Uint8Array, suffix: Uint8Array, bits: number): Promise<number> {
    const id = ++job;
    return new Promise((resolve, reject) => {
      let failed = 0;
      const done = () => {
        for (const w of workers) {
          w.removeEventListener('message', on);
          w.removeEventListener('error', onError);
          w.postMessage({ type: 'stop', job: id });
        }
      };
      // A miner that died (out of memory, a wasm that didn't load) must not leave the composer
      // on "Signing…" for good: once every miner failed, the caller mines another way.
      const fail = (message: string) => {
        if (++failed < workers.length) return;
        done();
        reject(new Error(message));
      };
      const onError = (e: ErrorEvent) => fail(e.message || 'a miner failed');
      const on = (e: MessageEvent<Found | Failed>) => {
        const m = e.data;
        if (m.job !== id) return;
        if (m.type === 'failed') fail(m.message);
        else if (m.type === 'found') {
          done();
          resolve(m.nonce);
        }
      };
      workers.forEach((w, i) => {
        w.addEventListener('message', on);
        w.addEventListener('error', onError);
        w.postMessage({ type: 'mine', job: id, prefix, suffix, bits, start: i, step: n });
      });
    });
  }

  function mineIdentity(rootKey: string, bits: number, onTry?: (t: IdentityTry) => void, signal?: AbortSignal): Promise<number> {
    const id = ++job;
    const root = hexToBytes(rootKey);
    return new Promise((resolve, reject) => {
      let failed = 0;
      const done = () => {
        signal?.removeEventListener('abort', abort);
        for (const w of workers) {
          w.removeEventListener('message', on);
          w.removeEventListener('error', onError);
          w.postMessage({ type: 'stop', job: id });
        }
      };
      const abort = () => {
        done();
        reject(new DOMException('The work was stopped.', 'AbortError'));
      };
      // A miner that can't run (no memory for its 8 MiB, say) leaves the others to it.
      const fail = (message: string) => {
        if (++failed < workers.length) return;
        done();
        reject(new Error(message));
      };
      const onError = (e: ErrorEvent) => fail(e.message || 'a miner failed');
      const on = (e: MessageEvent<Found | Try | Failed>) => {
        const m = e.data;
        if (m.job !== id) return;
        if (m.type === 'try') onTry?.({ work: m.work, hash: m.hash, bits: m.bits });
        else if (m.type === 'failed') fail(m.message);
        else if (m.type === 'found') {
          done();
          resolve(m.nonce);
        }
      };
      if (signal?.aborted) return abort();
      signal?.addEventListener('abort', abort, { once: true });
      workers.forEach((w, i) => {
        w.addEventListener('message', on);
        w.addEventListener('error', onError);
        w.postMessage({ type: 'identity', job: id, root, bits, start: i, step: n });
      });
    });
  }

  async function mineMore(rootKey: string, bits: number, onProgress: (p: WorkProgress) => void, signal?: AbortSignal): Promise<number | null> {
    const p: WorkProgress = { tries: 0, best: 0 };
    const timer = setInterval(() => onProgress({ ...p }), 250);
    try {
      return await mineIdentity(rootKey, bits, (t) => {
        p.tries++;
        p.best = Math.max(p.best, t.bits);
      }, signal);
    } catch (e) {
      if (signal?.aborted) return null;
      throw e;
    } finally {
      clearInterval(timer);
    }
  }

  return { rate, mine, mineIdentity, mineMore };
}
