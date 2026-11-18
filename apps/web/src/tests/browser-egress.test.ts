import { afterEach, describe, expect, it, vi } from 'vitest';
import { BROWSER_EGRESS_URL, fetchBrowserEgress } from '../lib/core/browser-egress';

const start = () => fetchBrowserEgress(new AbortController().signal);
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

describe('the voluntary browser HTTPS address check', () => {
  it.each(['1.1.1.1', '2606:4700:4700::1111'])('accepts a numeric public address from the one named observer', async ip => {
    // Published resolver addresses are used only as synthetic response bodies.
    const call = vi.fn().mockResolvedValue(new Response(ip + '\n'));
    vi.stubGlobal('fetch', call);
    expect(call).not.toHaveBeenCalled();
    expect(await start()).toBe(ip);
    expect(call).toHaveBeenCalledTimes(1);
    const [url, init] = call.mock.calls[0];
    expect(url).toBe(BROWSER_EGRESS_URL);
    expect(init).toEqual({
      method: 'GET', mode: 'cors', credentials: 'omit', referrerPolicy: 'no-referrer',
      cache: 'no-store', redirect: 'error', signal: expect.any(AbortSignal),
    });
  });

  it.each([
    '', '<html>', '1.1.1.1\n1.1.1.1', '1.1.1.999', '01.1.1.1', '1.1.1.1:80', '2001:::2',
    '0.1.1.1', '10.1.2.3', '127.0.0.1', '169.254.1.1', '172.31.1.2', '192.168.1.1', '100.64.0.1',
    '192.0.2.1', '198.18.0.1', '224.0.0.1', '255.255.255.255', '::', '::1', 'fc00::1',
    'fe80::1', 'ff02::1', '::ffff:7f00:1', '2001:db8::1', '2002::1', '3fff:100::1',
  ])('rejects invalid or non-public input %j', async text => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(text)));
    await expect(start()).rejects.toThrow('invalid or non-public');
  });

  it('bounds the actual streamed bytes even when Content-Length is missing', async () => {
    const cancel = vi.fn();
    const body = new ReadableStream<Uint8Array>({
      start(c) { c.enqueue(new Uint8Array(40)); c.enqueue(new Uint8Array(40)); },
      cancel,
    });
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(body)));
    await expect(start()).rejects.toThrow('invalid or non-public');
    expect(cancel).toHaveBeenCalledOnce();
  });

  it('rejects an oversized declared body without reading it', async () => {
    const body = new Response('1.1.1.1', { headers: { 'Content-Length': '50000000' } });
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(body));
    await expect(start()).rejects.toThrow('invalid or non-public');
    expect(body.bodyUsed).toBe(false);
  });

  it('handles a blocked connection or HTTP rejection without contacting another provider', async () => {
    const call = vi.fn().mockRejectedValueOnce(new TypeError('Failed to fetch'))
      .mockResolvedValueOnce(new Response('', { status: 429 }));
    vi.stubGlobal('fetch', call);
    await expect(start()).rejects.toThrow('Could not reach');
    await expect(start()).rejects.toThrow('HTTP 429');
    expect(call.mock.calls.map(args => args[0])).toEqual([BROWSER_EGRESS_URL, BROWSER_EGRESS_URL]);
  });

  it('respects cancellation before sending and while reading a stalled body', async () => {
    const cancel = vi.fn();
    const body = new ReadableStream({ cancel });
    const call = vi.fn().mockResolvedValue(new Response(body));
    vi.stubGlobal('fetch', call);
    await expect(fetchBrowserEgress(AbortSignal.abort())).rejects.toMatchObject({ name: 'AbortError' });
    expect(call).not.toHaveBeenCalled();
    const controller = new AbortController();
    const result = fetchBrowserEgress(controller.signal);
    await vi.waitFor(() => expect(body.locked).toBe(true));
    controller.abort();
    await expect(result).rejects.toMatchObject({ name: 'AbortError' });
    expect(cancel).toHaveBeenCalledOnce();
  });

  it('times out a stalled response and releases its reader', async () => {
    vi.useFakeTimers();
    const cancel = vi.fn();
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(new ReadableStream({ cancel }))));
    const assertion = expect(start()).rejects.toThrow('timed out');
    await vi.advanceTimersByTimeAsync(8_000);
    await assertion;
    expect(cancel).toHaveBeenCalledOnce();
  });
});
