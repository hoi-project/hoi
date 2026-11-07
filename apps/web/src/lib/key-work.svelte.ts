// More identity work for your key (You → Key work): one job at a time, kept here rather than in
// the page so it carries on while the person looks elsewhere, and can be watched and stopped.
// The core mines it outside itself where the host can (HoiCore.strengthenIdentity), so the rest
// of the app, a recovery key for one, keeps answering meanwhile.
import type { WorkProgress } from '@hoi/core-api';
import { app } from './app.svelte';
import { toast, toastError } from './toast.svelte';

/** Bits added per step. Each step is about 4× the work of the one before. */
export const STEP = 2;

/** What a job's numbers mean, for the progress line: tries against the usual count for this many
 * bits (2^bits), and a time left read from the pace so far. Luck decides the real end. */
export function workOutlook(target: number, p: WorkProgress, elapsedMs: number): { usual: number; share: number; leftMs: number | null } {
  const usual = 2 ** target;
  const share = p.tries / usual;
  const rate = elapsedMs > 1500 && p.tries > 0 ? p.tries / elapsedMs : 0;
  const leftMs = rate && p.tries < usual ? (usual - p.tries) / rate : null;
  return { usual, share, leftMs };
}

class KeyWork {
  /** Bits the running job aims for; 0 while idle. */
  target = $state(0);
  started = $state(0);
  progress = $state.raw<WorkProgress>({ tries: 0, best: 0 });
  #stop: AbortController | null = null;

  get running(): boolean {
    return this.target > 0;
  }

  async start(): Promise<void> {
    const me = app.me;
    if (!me || this.running) return;
    const from = me.identityBits;
    const stop = new AbortController();
    this.#stop = stop;
    this.target = from + STEP;
    this.started = Date.now();
    this.progress = { tries: 0, best: 0 };
    try {
      const m = await app.core.strengthenIdentity(this.target, { onProgress: (p) => (this.progress = p), signal: stop.signal });
      app.me = m;
      toast(`Done. Your key now carries ${m.identityBits} bits of work.`);
    } catch (e) {
      if (stop.signal.aborted) toast(`Stopped. Your key keeps its ${from} bits.`);
      else toastError(e);
    } finally {
      this.#stop = null;
      this.target = 0;
    }
  }

  stop(): void {
    this.#stop?.abort();
  }
}

export const keyWork = new KeyWork();
