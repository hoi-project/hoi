/// <reference lib="webworker" />
// Looks at pictures (and video frames) for the core, in a worker of its own. The core
// runs as wasm with panic = "abort": a decoder bug in there would take the whole node down. Here
// it takes down this worker, the pool starts another, and the picture counts as unreadable.

import init, { look_luma, look_media, sanitize_and_look } from './wasm/pkg/hoi_wasm.js';

declare const self: DedicatedWorkerGlobalScope;

export type LookMsg =
  | { type: 'look'; job: number; mime: string; data: Uint8Array }
  | { type: 'sanitize'; job: number; data: Uint8Array }
  | { type: 'frames'; job: number; frames: { luma: Float32Array; w: number; h: number }[] };

const ready = init();

self.onmessage = async (e: MessageEvent<LookMsg>) => {
  const m = e.data;
  await ready;
  try {
    if (m.type === 'look') self.postMessage({ job: m.job, ok: true, result: JSON.parse(look_media(m.mime, m.data)) });
    else if (m.type === 'sanitize') self.postMessage({ job: m.job, ok: true, result: JSON.parse(sanitize_and_look(m.data)) });
    else {
      const frames = m.frames.map((f) => JSON.parse(look_luma(f.luma, f.w, f.h))).filter((f) => f !== null);
      self.postMessage({ job: m.job, ok: true, result: frames.length ? { video: frames } : 'nothing' });
    }
  } catch (err) {
    self.postMessage({ job: m.job, ok: false, error: String((err as Error)?.message ?? err) });
  }
};
