/** Profile images are re-encoded locally; no source metadata or remote image URL is shared. */
export const AVATAR_BYTES = 4096;
export const AVATAR_SIDE = 128;

export function cropRect(width: number, height: number, zoom: number, x: number, y: number) {
  const size = Math.min(width, height) / Math.max(1, Math.min(4, zoom));
  return { x: (width - size) * (Math.max(-1, Math.min(1, x)) + 1) / 2,
    y: (height - size) * (Math.max(-1, Math.min(1, y)) + 1) / 2, size };
}

export function drawCrop(canvas: HTMLCanvasElement, source: HTMLImageElement, zoom: number, x: number, y: number) {
  const context = canvas.getContext('2d');
  if (!context) throw new Error('This device cannot prepare a profile picture.');
  const crop = cropRect(source.naturalWidth, source.naturalHeight, zoom, x, y);
  context.fillStyle = '#ffffff';
  context.fillRect(0, 0, canvas.width, canvas.height);
  context.drawImage(source, crop.x, crop.y, crop.size, crop.size, 0, 0, canvas.width, canvas.height);
}

export async function encodeAvatar(source: HTMLImageElement, zoom: number, x: number, y: number): Promise<Uint8Array> {
  const canvas = document.createElement('canvas');
  for (const side of [AVATAR_SIDE, 96, 80]) {
    canvas.width = canvas.height = side;
    drawCrop(canvas, source, zoom, x, y);
    for (const quality of [0.85, 0.7, 0.55, 0.4]) {
      const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/jpeg', quality));
      if (blob && blob.size <= AVATAR_BYTES) return new Uint8Array(await blob.arrayBuffer());
    }
  }
  throw new Error('This picture cannot be made small enough. Try a simpler crop.');
}

/** Only bounded JPEG bytes from the core are rendered. Arbitrary URLs cannot track readers. */
export function avatarUrl(encoded?: string): string | undefined {
  if (!encoded || encoded.length > Math.ceil(AVATAR_BYTES * 4 / 3) || !/^[A-Za-z0-9_-]+$/.test(encoded)) return;
  return `data:image/jpeg;base64,${encoded.replace(/-/g, '+').replace(/_/g, '/')}`;
}
