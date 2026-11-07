/// <reference lib="webworker" />
// One miner. Several run in parallel; each tries every n-th nonce until one finds a stamp (or a
// new key's identity work) or the page tells it to stop.

import { leadingZeroBits } from '@hoi/core-api';
import init, { IdentityMiner, mine_range } from './wasm/pkg/hoi_wasm.js';

declare const self: DedicatedWorkerGlobalScope;

type Job = { job: number; prefix: Uint8Array; suffix: Uint8Array; bits: number; start: number; step: number };
type IdentityJob = { job: number; root: Uint8Array; bits: number; start: number; step: number };
type Msg = ({ type: 'mine' } & Job) | ({ type: 'identity' } & IdentityJob) | { type: 'stop'; job: number } | { type: 'bench' };

const CHUNK = 20_000;
const ready = init();
/** Jobs this miner works on. A post's stamp can come while a key's identity work runs (You →
 * Key work, minutes): both go on, taking turns, instead of the stamp ending the identity work. */
const active = new Set<number>();

async function mine(j: Job) {
  active.add(j.job);
  await ready;
  let nonce = j.start;
  let tries = 0;
  const t0 = performance.now();
  while (active.has(j.job)) {
    const found = mine_range(j.prefix, j.suffix, j.bits, nonce, j.step, CHUNK);
    tries += CHUNK;
    if (found >= 0) {
      active.delete(j.job);
      self.postMessage({ type: 'found', job: j.job, nonce: found, tries, ms: performance.now() - t0 });
      return;
    }
    nonce += j.step * CHUNK;
    // let a stop message in
    await new Promise((r) => setTimeout(r, 0));
  }
}

// Lets a stop message in between identity tries without setTimeout's 4 ms clamp (a try is ~15 ms).
// Every waiting job gets its turn, in order.
const tick = new MessageChannel();
const waiting: (() => void)[] = [];
tick.port1.onmessage = () => waiting.shift()?.();
const yieldToMessages = () =>
  new Promise<void>((r) => {
    waiting.push(r);
    tick.port2.postMessage(0);
  });

/** Identity work (PROTOCOL.md §4.1), one Argon2id try at a time: every try is reported, so the
 * first run can show it. */
async function identity(j: IdentityJob) {
  active.add(j.job);
  await ready;
  const miner = new IdentityMiner(j.root);
  const t0 = performance.now();
  let tries = 0;
  try {
    for (let work = j.start; active.has(j.job); work += j.step) {
      const hash = miner.attempt(work);
      const bits = leadingZeroBits(hash);
      tries++;
      self.postMessage({ type: 'try', job: j.job, work, hash, bits }, [hash.buffer]);
      if (bits >= j.bits) {
        self.postMessage({ type: 'found', job: j.job, nonce: work, tries, ms: performance.now() - t0 });
        return;
      }
      await yieldToMessages();
    }
  } finally {
    active.delete(j.job);
    miner.free();
  }
}

async function bench() {
  await ready;
  const p = new Uint8Array(180).fill(0xa9);
  const s = new Uint8Array(60).fill(1);
  const n = 30_000;
  const t0 = performance.now();
  mine_range(p, s, 255, 0, 1, n);
  self.postMessage({ type: 'bench', hashesPerSecond: (n / (performance.now() - t0)) * 1000 });
}

self.onmessage = (e: MessageEvent<Msg>) => {
  const m = e.data;
  if (m.type === 'mine') void mine(m);
  else if (m.type === 'identity') void identity(m).catch((err) => self.postMessage({ type: 'failed', job: m.job, message: String(err?.message ?? err) }));
  else if (m.type === 'stop') active.delete(m.job);
  else if (m.type === 'bench') void bench();
};
