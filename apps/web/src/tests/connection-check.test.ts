import { afterEach, describe, expect, it, vi } from 'vitest';
import { canCheckRoute, checkConnection, readObservation } from '../lib/core/connection-check';

const bytes = (s: string) => new TextEncoder().encode(s);
afterEach(() => vi.unstubAllGlobals());

describe('isolated pre-join address check', () => {
  it('accepts only a numeric IP observation', () => {
    expect(readObservation(bytes('hoi/address/1 2001:db8::7'))).toBe('2001:db8::7');
    for (const value of ['192.0.2.1', 'hoi/address/1 ', 'hoi/address/1 hidden.local', 'hoi/address/1 0.0.0.0', 'hoi/address/1 192.0.2.1:8000']) {
      expect(readObservation(bytes(value))).toBeNull();
    }
  });

  it('refuses unrelated destinations and credential-bearing URLs', () => {
    for (const url of ['https://relay.example', 'ws://node.example', 'wss://user@example.com', 'wss://node.example?token=foo']) {
      expect(canCheckRoute(url)).toBe(false);
    }
  });

  it('sends just the fixed request on a new connection to the selected node, then closes', async () => {
    const sent = vi.fn();
    const close = vi.fn();
    class Socket {
      onopen?: () => void;
      onmessage?: (ev: { data: ArrayBufferLike }) => void;
      constructor(public url: string) { expect(url).toBe('wss://node.example/'); queueMicrotask(() => this.onopen?.()); }
      send(frame: Uint8Array) { sent(new TextDecoder().decode(frame)); this.onmessage?.({ data: bytes('hoi/address/1 192.0.2.7').buffer }); }
      close = close;
    }
    vi.stubGlobal('WebSocket', Socket);
    expect(await checkConnection('wss://node.example/', new AbortController().signal)).toEqual({ address: '192.0.2.7', transport: 'websocket' });
    expect(sent.mock.calls).toEqual([['hoi/address-check/1']]);
    expect(close).toHaveBeenCalledOnce();
  });
});
