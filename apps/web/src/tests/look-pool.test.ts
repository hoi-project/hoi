import { describe, expect, it } from 'vitest';
import { createLooker } from '../lib/core/look-pool';

type Behaviour = (msg: { type: string; job: number }, w: FakeWorker) => void;

class FakeWorker {
  onmessage: ((e: MessageEvent) => void) | null = null;
  onerror: ((e: ErrorEvent) => void) | null = null;
  terminated = false;
  seen: string[] = [];
  constructor(private behave: Behaviour) {}
  postMessage(msg: { type: string; job: number }) {
    this.seen.push(msg.type);
    queueMicrotask(() => this.behave(msg, this));
  }
  terminate() {
    this.terminated = true;
  }
  reply(job: number, result: unknown) {
    this.onmessage?.({ data: { job, ok: true, result } } as MessageEvent);
  }
  crash() {
    this.onerror?.({ message: 'RuntimeError: unreachable', preventDefault() {} } as unknown as ErrorEvent);
  }
}

const picture = { picture: { hashes: Array(8).fill('ab'.repeat(32)), quality: 80 } };

describe('the look pool', () => {
  it('looks at pictures in its worker', async () => {
    const w = new FakeWorker((m, w) => w.reply(m.job, picture));
    const looker = createLooker({ spawn: () => w as unknown as Worker });
    expect(await looker.look('image/png', new Uint8Array([1, 2, 3]))).toEqual(picture);
    expect(w.seen).toEqual(['look']);
  });

  it('counts a picture that kills the worker as unreadable, and starts another', async () => {
    const workers: FakeWorker[] = [];
    let first = true;
    const looker = createLooker({
      spawn: () => {
        const w = new FakeWorker((m, w) => {
          if (first) {
            first = false;
            w.crash();
          } else w.reply(m.job, picture);
        });
        workers.push(w);
        return w as unknown as Worker;
      },
    });
    const [bad, good] = await Promise.all([looker.look('image/jpeg', new Uint8Array(4)), looker.look('image/png', new Uint8Array(4))]);
    expect(bad).toBe('unreadable');
    expect(good).toEqual(picture);
    expect(workers.length).toBe(2);
    expect(workers[0].terminated).toBe(true);
  });

  it('gives up on a worker that hangs', async () => {
    const looker = createLooker({ spawn: () => new FakeWorker(() => {}) as unknown as Worker, timeoutMs: 20 });
    expect(await looker.look('image/png', new Uint8Array(4))).toBe('unreadable');
  });

  it('plays a video for its frames, and a video it cannot play says nothing', async () => {
    const frame = { hashes: Array(8).fill('cd'.repeat(32)), quality: 70 };
    const w = new FakeWorker((m, w) => w.reply(m.job, { video: [frame, frame] }));
    const frames = [{ luma: new Float32Array(64 * 64), w: 64, h: 64 }];
    const looker = createLooker({ spawn: () => w as unknown as Worker, sample: async () => frames });
    expect(await looker.look('video/mp4', new Uint8Array(8))).toEqual({ video: [frame, frame] });
    expect(w.seen).toEqual(['frames']);
    const deaf = createLooker({ spawn: () => w as unknown as Worker, sample: async () => null });
    expect(await deaf.look('video/webm', new Uint8Array(8))).toBe('nothing');
  });

  it('composes: the sanitized hash and look, or null when nothing could read it', async () => {
    const ok = createLooker({ spawn: () => new FakeWorker((m, w) => w.reply(m.job, { hash: 'ee'.repeat(32), mime: 'image/png', look: picture })) as unknown as Worker });
    expect(await ok.compose(new Uint8Array(4))).toEqual({ hash: 'ee'.repeat(32), mime: 'image/png', look: picture });
    const dead = createLooker({ spawn: () => new FakeWorker((_m, w) => w.crash()) as unknown as Worker });
    expect(await dead.compose(new Uint8Array(4))).toBeNull();
  });
});
