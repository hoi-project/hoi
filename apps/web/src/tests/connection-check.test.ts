import { afterEach, describe, expect, it, vi } from 'vitest';
import * as rtc from '../lib/core/rtc-link';
import { canCheckRoute, checkConnection, readObservation } from '../lib/core/connection-check';

const bytes = (s: string) => new TextEncoder().encode(s);
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

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

  it('requires a literal numeric socket for browser RTC checks', () => {
    const fp = 'A'.repeat(43);
    for (const socket of ['2130706433:3478', '127.1:3478', '0.0.0.0:3478', 'example.org:3478', '999.100.1.1:3478']) {
      expect(canCheckRoute(`rtc:${fp}@${socket}`)).toBe(false);
    }
  });

  it('offers a numeric direct QUIC destination only in the native shell', () => {
    const route = '1'.repeat(64) + '@127.0.0.1:7843';
    expect(canCheckRoute(route)).toBe(false);
    vi.stubGlobal('window', { __TAURI_INTERNALS__: {} });
    expect(canCheckRoute(route)).toBe(true);
    expect(canCheckRoute('1'.repeat(64) + '@node.example:7843')).toBe(false);
    expect(canCheckRoute('1'.repeat(64))).toBe(false);
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


  it.each([
    ['hoi/address/1 ', 'withheld'],
    ['hoi/address/1 not-an-address', 'unexpected'],
    ['old protocol', 'unexpected'],
  ])('explains the actual WebSocket response %j', async (reply, kind) => {
    const close = vi.fn();
    class Socket {
      onopen?: () => void;
      onmessage?: (ev: { data: ArrayBufferLike }) => void;
      constructor() { queueMicrotask(() => this.onopen?.()); }
      send() { this.onmessage?.({ data: bytes(reply).buffer }); }
      close = close;
    }
    vi.stubGlobal('WebSocket', Socket);
    await expect(checkConnection('wss://node.example', new AbortController().signal)).rejects.toMatchObject({ kind });
    expect(close).toHaveBeenCalledOnce();
  });

  it('distinguishes failure to open from a node closing after open', async () => {
    let opened = false;
    class Socket {
      onopen?: () => void;
      onclose?: () => void;
      constructor() { queueMicrotask(() => { if (opened) this.onopen?.(); this.onclose?.(); }); }
      send() {}
      close() {}
    }
    vi.stubGlobal('WebSocket', Socket);
    await expect(checkConnection('wss://node.example', new AbortController().signal)).rejects.toMatchObject({ kind: 'unreachable' });
    opened = true;
    await expect(checkConnection('wss://node.example', new AbortController().signal)).rejects.toMatchObject({ kind: 'closed' });
  });

  describe('same-socket UDP measurement', () => {
    const route = 'rtc:' + 'A'.repeat(43) + '@[2001:db8::7]:7844';
    const server = 'stun:[2001:db8::7]:7844';
    type Candidate = { type: string; protocol: string; address: string; url?: string };
    function olderNodeAndIce(onGather: (emit: (candidate: Candidate | null) => void) => void) {
      vi.spyOn(rtc, 'openRtc').mockImplementation(async (_routes, _mode, on) => ({
        binding: { node: '', browser: '', ufrag: '' },
        send: () => on.close('closed'), close: () => {},
      }));
      const close = vi.fn();
      class Peer {
        onicecandidate?: (ev: { candidate: Candidate | null }) => void;
        constructor(config: RTCConfiguration) { expect(config).toEqual({ iceServers: [{ urls: server }] }); }
        createDataChannel() {}
        async createOffer() { return { type: 'offer', sdp: '' }; }
        async setLocalDescription() { onGather(candidate => this.onicecandidate?.({ candidate })); }
        close = close;
      }
      vi.stubGlobal('RTCPeerConnection', Peer);
      return close;
    }
    const rejected = [
      { type: 'host', protocol: 'udp', address: '10.1.2.3' },
      { type: 'relay', protocol: 'udp', address: '192.0.2.17' },
      { type: 'prflx', protocol: 'udp', address: '192.0.2.18' },
      { type: 'srflx', protocol: 'tcp', address: '192.0.2.20' },
      { type: 'srflx', protocol: 'udp', address: 'hidden.local' },
      { type: 'srflx', protocol: 'udp', address: '192.0.2.22', url: 'stun:other.example:7844' },
    ];
    it('accepts a numeric UDP srflx mapping from only the selected socket after an old node closes', async () => {
      const close = olderNodeAndIce(emit => {
        rejected.forEach(emit);
        emit({ type: 'srflx', protocol: 'udp', address: '2001:db8::5', url: server });
      });
      await expect(checkConnection(route, new AbortController().signal)).resolves.toEqual({ address: '2001:db8::5', transport: 'udp' });
      expect(close).toHaveBeenCalledOnce();
    });
    it('reports the failed node probe when the browser returns no mapping', async () => {
      const close = olderNodeAndIce(emit => { rejected.forEach(emit); emit(null); });
      await expect(checkConnection(route, new AbortController().signal)).rejects.toMatchObject({
        kind: 'closed', message: expect.stringContaining('no usable UDP address from that same node'),
      });
      expect(close).toHaveBeenCalledOnce();
    });
    it('closes the gathering socket on cancellation and preserves the cancellation reason', async () => {
      const controller = new AbortController();
      const reason = new Error('changed selection');
      const close = olderNodeAndIce(() => controller.abort(reason));
      await expect(checkConnection(route, controller.signal)).rejects.toBe(reason);
      expect(close).toHaveBeenCalledOnce();
    });
    it('ends a stalled UDP measurement', async () => {
      vi.useFakeTimers();
      const close = olderNodeAndIce(() => {});
      const checked = expect(checkConnection(route, new AbortController().signal)).rejects.toMatchObject({ kind: 'closed' });
      await vi.advanceTimersByTimeAsync(6100);
      await checked;
      expect(close).toHaveBeenCalledOnce();
    });
  });
});
