// Looks for the core: pictures go to a look worker (look.worker.ts), videos are played
// on the page for a few frames (video-frames.ts) whose fingerprints the worker takes. A worker
// that dies or hangs is replaced, and what it was looking at counts as unreadable: one picture
// lost, never the node.
import { sampleFrames } from './video-frames';

/** A look as the core reads it (hoi_core::look::LookDto). */
export type LookDto =
  | { picture: { hashes: string[]; quality: number } }
  | { video: { hashes: string[]; quality: number }[] }
  | 'nothing'
  | 'unreadable';

export interface Looker {
  /** The look of media that arrived, already checked by the core. */
  look(mime: string, data: Uint8Array): Promise<LookDto>;
  /** A file someone attaches: sanitized as the core will, then looked at. `hash` is empty when
   * the file won't do (the core then says why); null when nothing here could read it. */
  compose(data: Uint8Array): Promise<{ hash: string; mime: string; look: LookDto } | null>;
}

type Reply = { job: number; ok: true; result: unknown } | { job: number; ok: false; error: string };
type Job = { msg: Record<string, unknown>; transfer: Transferable[]; resolve: (v: unknown) => void; reject: (e: Error) => void };

/** The worker died or hung on a job: nothing here could read it. */
class WorkerGone extends Error {}

/** Longest a look may take before the worker counts as stuck. */
const LOOK_MS = 20_000;

export interface LookerOptions {
  /** Makes a worker (tests pass a fake). */
  spawn?: () => Worker;
  sample?: typeof sampleFrames;
  timeoutMs?: number;
}

export function createLooker(opts: LookerOptions = {}): Looker {
  const spawn = opts.spawn ?? (() => new Worker(new URL('./look.worker.ts', import.meta.url), { type: 'module', name: 'hoi-look' }));
  const sample = opts.sample ?? sampleFrames;
  const timeoutMs = opts.timeoutMs ?? LOOK_MS;
  let worker: Worker | null = null;
  let next = 1;
  let busy: { id: number; job: Job; timer: ReturnType<typeof setTimeout> } | null = null;
  const queue: Job[] = [];

  function replace(reason: string) {
    worker?.terminate();
    worker = null;
    const b = busy;
    busy = null;
    if (b) {
      clearTimeout(b.timer);
      b.job.reject(new WorkerGone(reason));
    }
    pump();
  }

  function ensure(): Worker {
    if (worker) return worker;
    const w = spawn();
    w.onmessage = (e: MessageEvent<Reply>) => {
      const m = e.data;
      if (!busy || m.job !== busy.id) return;
      const b = busy;
      busy = null;
      clearTimeout(b.timer);
      if (m.ok) b.job.resolve(m.result);
      else b.job.reject(new Error(m.error));
      pump();
    };
    // a wasm abort (a decoder bug) or a worker that couldn't load: start over without it
    w.onerror = (e) => {
      e.preventDefault?.();
      replace(e.message || 'the look worker failed');
    };
    worker = w;
    return w;
  }

  function pump() {
    if (busy) return;
    const job = queue.shift();
    if (!job) return;
    const id = next++;
    busy = { id, job, timer: setTimeout(() => replace('the look took too long'), timeoutMs) };
    ensure().postMessage({ ...job.msg, job: id }, job.transfer);
  }

  function run(msg: Record<string, unknown>, transfer: Transferable[] = []): Promise<unknown> {
    return new Promise((resolve, reject) => {
      queue.push({ msg, transfer, resolve, reject });
      pump();
    });
  }

  async function lookVideo(mime: string, data: Uint8Array): Promise<LookDto> {
    // a video this browser can't play has nothing to say; the exact hash list still applies
    const frames = await sample(data, mime).catch(() => null);
    if (!frames?.length) return 'nothing';
    return (await run({ type: 'frames', frames }, frames.map((f) => f.luma.buffer))) as LookDto;
  }

  return {
    async look(mime, data) {
      try {
        if (mime.startsWith('video/')) return await lookVideo(mime, data);
        return (await run({ type: 'look', mime, data: data.slice() })) as LookDto;
      } catch {
        return mime.startsWith('video/') ? 'nothing' : 'unreadable';
      }
    },
    async compose(data) {
      let r: { hash: string; mime: string; look: LookDto };
      try {
        r = (await run({ type: 'sanitize', data: data.slice() })) as typeof r;
      } catch (e) {
        // the worker died on it, or the file itself won't do (the core says why)
        if (e instanceof WorkerGone) return null;
        return { hash: '', mime: '', look: 'nothing' };
      }
      if (r.mime.startsWith('video/')) r.look = await lookVideo(r.mime, data).catch(() => 'nothing' as const);
      return r;
    },
  };
}
