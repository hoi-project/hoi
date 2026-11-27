import { afterEach, describe, expect, it, vi } from 'vitest';
import { fetchData, fetchJson, inflate } from '../lib/geo/load';

afterEach(() => { vi.unstubAllGlobals(); });

async function deflate(data: Uint8Array): Promise<Uint8Array> {
  const stream = new Blob([data as BlobPart]).stream().pipeThrough(new CompressionStream('deflate'));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

describe('loading the built-in geo data', () => {
  it('fetches same-origin, without credentials, referrer or redirects', async () => {
    const call = vi.fn().mockResolvedValue(new Response(new Uint8Array([1, 2, 3])));
    vi.stubGlobal('fetch', call);
    expect(await fetchData('/assets/table.bin', 10)).toEqual(new Uint8Array([1, 2, 3]));
    expect(call).toHaveBeenCalledTimes(1);
    expect(call.mock.calls[0][1]).toEqual({ method: 'GET', mode: 'same-origin', credentials: 'omit', referrerPolicy: 'no-referrer', redirect: 'error', signal: undefined });
  });

  it('stops reading past the byte limit, with or without Content-Length', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(new Uint8Array(11), { headers: { 'content-length': '11' } })));
    await expect(fetchData('/a', 10)).rejects.toThrow(/larger/);
    const body = new ReadableStream<Uint8Array>({ start(c) { c.enqueue(new Uint8Array(6)); c.enqueue(new Uint8Array(6)); c.close(); } });
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(body)));
    await expect(fetchData('/a', 10)).rejects.toThrow(/larger/);
  });

  it('reports failures without trying another source', async () => {
    const call = vi.fn().mockResolvedValue(new Response('', { status: 404 }));
    vi.stubGlobal('fetch', call);
    await expect(fetchData('/a', 10)).rejects.toThrow(/could not be loaded/);
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('offline')));
    await expect(fetchData('/a', 10)).rejects.toThrow(/could not be loaded/);
    expect(call).toHaveBeenCalledTimes(1);
  });

  it('inflates within a limit', async () => {
    const data = new Uint8Array(1000).fill(7);
    expect(await inflate(await deflate(data), 1000)).toEqual(data);
    await expect(inflate(await deflate(data), 999)).rejects.toThrow(/larger/);
    await expect(inflate(new Uint8Array([1, 2, 3]), 1000)).rejects.toThrow(/damaged/);
  });

  it('parses JSON data and rejects anything else', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('{"Test/Alpha":"XA"}')));
    expect(await fetchJson('/z.json')).toEqual({ 'Test/Alpha': 'XA' });
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('<html>')));
    await expect(fetchJson('/z.json')).rejects.toThrow(/damaged/);
  });
});
