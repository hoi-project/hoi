// Fetches the app's own geo data. Only same-origin files built into the app are read, only after a person
// asks for a check, and never from another URL when one fails.
export const LIMITS = { table: 1 << 20, inflated: 4 << 20, json: 512 << 10 } as const;

const tooLarge = () => new Error('The country data is larger than expected.');
const unreachable = () => new Error('The country data could not be loaded. Check your connection and try again.');

export async function fetchData(url: string, maxBytes: number, signal?: AbortSignal): Promise<Uint8Array> {
  const response = await fetch(url, {
    method: 'GET', mode: 'same-origin', credentials: 'omit', referrerPolicy: 'no-referrer', redirect: 'error', signal,
  }).catch(() => { throw unreachable(); });
  if (!response.ok || !response.body) throw unreachable();
  if (Number(response.headers.get('content-length')) > maxBytes) throw tooLarge();
  return readCapped(response.body, maxBytes);
}

async function readCapped(stream: ReadableStream<Uint8Array>, maxBytes: number): Promise<Uint8Array> {
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let used = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      used += value.byteLength;
      if (used > maxBytes) {
        void reader.cancel().catch(() => {});
        throw tooLarge();
      }
      chunks.push(value);
    }
  } finally { reader.releaseLock(); }
  const out = new Uint8Array(used);
  let at = 0;
  for (const chunk of chunks) { out.set(chunk, at); at += chunk.byteLength; }
  return out;
}

/** Inflates a zlib (deflate) stream without letting it grow past maxBytes. */
export async function inflate(bytes: Uint8Array, maxBytes: number): Promise<Uint8Array> {
  const stream = new Blob([bytes as BlobPart]).stream().pipeThrough(new DecompressionStream('deflate'));
  return readCapped(stream, maxBytes).catch((error) => {
    throw error instanceof Error && error.message.includes('larger') ? error : new Error('The country table is damaged (cannot inflate).');
  });
}

export async function fetchJson(url: string, signal?: AbortSignal): Promise<unknown> {
  const bytes = await fetchData(url, LIMITS.json, signal);
  try { return JSON.parse(new TextDecoder().decode(bytes)); }
  catch { throw new Error('The country data is damaged.'); }
}
