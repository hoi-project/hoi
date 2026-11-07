// A few frames of a video, played by the browser's own player on the page (workers have no
// <video>), as luma for the look worker. The player already shows these videos when
// a post is opened; here it is muted, never attached to the document, and given up on quickly.

/** Frames taken from a video, spread over its length. */
export const FRAMES = 8;
/** The longest side a frame is drawn at: PDQ needs no more. */
const SIDE = 512;
const STEP_MS = 5_000;
const TOTAL_MS = 30_000;

export type Frame = { luma: Float32Array; w: number; h: number };

function once(el: HTMLMediaElement, ok: string, ms: number): Promise<void> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => done(new Error('the video took too long')), ms);
    const done = (err?: Error) => {
      clearTimeout(timer);
      el.removeEventListener(ok, onOk);
      el.removeEventListener('error', onErr);
      if (err) reject(err);
      else resolve();
    };
    const onOk = () => done();
    const onErr = () => done(new Error('the video cannot be played here'));
    el.addEventListener(ok, onOk);
    el.addEventListener('error', onErr);
  });
}

/** Luma of the canvas's pixels (PDQ's weights). */
export function lumaOf(rgba: Uint8ClampedArray, w: number, h: number): Float32Array {
  const out = new Float32Array(w * h);
  for (let i = 0; i < w * h; i++) out[i] = 0.299 * rgba[4 * i] + 0.587 * rgba[4 * i + 1] + 0.114 * rgba[4 * i + 2];
  return out;
}

/** `FRAMES` frames at evenly spread times, or null when this browser can't play it. */
export async function sampleFrames(data: Uint8Array, mime: string): Promise<Frame[] | null> {
  if (typeof document === 'undefined') return null;
  const v = document.createElement('video');
  if (!v.canPlayType(mime)) return null;
  const url = URL.createObjectURL(new Blob([data.slice()], { type: mime }));
  const started = Date.now();
  try {
    v.muted = true;
    v.preload = 'auto';
    v.src = url;
    await once(v, 'loadeddata', STEP_MS);
    const duration = Number.isFinite(v.duration) ? v.duration : 0;
    const scale = Math.min(1, SIDE / Math.max(v.videoWidth, v.videoHeight, 1));
    const w = Math.max(1, Math.round(v.videoWidth * scale));
    const h = Math.max(1, Math.round(v.videoHeight * scale));
    const canvas = document.createElement('canvas');
    canvas.width = w;
    canvas.height = h;
    const g = canvas.getContext('2d', { willReadFrequently: true });
    if (!g || !duration) return null;
    const frames: Frame[] = [];
    for (let i = 0; i < FRAMES && Date.now() - started < TOTAL_MS; i++) {
      v.currentTime = ((i + 0.5) / FRAMES) * duration;
      await once(v, 'seeked', STEP_MS);
      g.drawImage(v, 0, 0, w, h);
      frames.push({ luma: lumaOf(g.getImageData(0, 0, w, h).data, w, h), w, h });
    }
    return frames;
  } finally {
    v.removeAttribute('src');
    v.load();
    URL.revokeObjectURL(url);
  }
}
