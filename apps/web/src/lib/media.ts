// Photos straight off a phone are often bigger than a post may carry (2 MiB, PROTOCOL.md §17).
// Rather than refuse them, the composer scales them down and re-encodes them here, in the
// browser's own image decoder. Re-encoding also drops everything that isn't the picture
// (location, camera, thumbnails), before the core strips and checks it again.

const MAX_BYTES = 2 * 1024 * 1024;
const MAX_SIDE = 2560;
const STILL = new Set(['image/jpeg', 'image/png', 'image/webp']);

export interface Fitted {
  data: Uint8Array;
  /** What changed, for a toast; absent when the file went through untouched. */
  note?: string;
}

async function encode(bitmap: ImageBitmap, w: number, h: number, type: string, quality: number): Promise<Blob | null> {
  if (typeof OffscreenCanvas !== 'undefined') {
    const c = new OffscreenCanvas(w, h);
    c.getContext('2d')?.drawImage(bitmap, 0, 0, w, h);
    return c.convertToBlob({ type, quality });
  }
  const c = document.createElement('canvas');
  c.width = w;
  c.height = h;
  c.getContext('2d')?.drawImage(bitmap, 0, 0, w, h);
  return new Promise((resolve) => c.toBlob(resolve, type, quality));
}

/** Scales a still image down to fit, or hands it back as it is. Animations, audio and video pass untouched. */
export async function fitImage(file: File): Promise<Fitted> {
  const raw = new Uint8Array(await file.arrayBuffer());
  if (!STILL.has(file.type) || typeof createImageBitmap === 'undefined') return { data: raw };
  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(file);
  } catch {
    return { data: raw };
  }
  try {
    const side = Math.max(bitmap.width, bitmap.height);
    if (raw.length <= MAX_BYTES && side <= MAX_SIDE) return { data: raw };
    // JPEG for photos; WebP keeps a PNG's transparency
    const type = file.type === 'image/jpeg' ? 'image/jpeg' : 'image/webp';
    for (const [limit, quality] of [[MAX_SIDE, 0.86], [2048, 0.82], [1600, 0.78], [1280, 0.72]] as const) {
      const scale = Math.min(1, limit / side);
      const w = Math.max(1, Math.round(bitmap.width * scale));
      const h = Math.max(1, Math.round(bitmap.height * scale));
      const blob = await encode(bitmap, w, h, type, quality);
      if (blob && blob.size <= MAX_BYTES) {
        return { data: new Uint8Array(await blob.arrayBuffer()), note: `Resized ${file.name} to ${w}×${h} to fit, without its metadata.` };
      }
    }
    return { data: raw };
  } finally {
    bitmap.close();
  }
}
