import { afterEach, describe, expect, it, vi } from 'vitest';
import { createRtcAddresses, rtcAddresses, selectedAddress, type RtcAddress } from '../lib/core/rtc-addresses';
import { openRtc } from '../lib/core/rtc-link';

function report(address = '192.0.2.10', candidateType = 'srflx'): RTCStatsReport {
  return new Map([
    ['transport', { id: 'transport', type: 'transport', selectedCandidatePairId: 'pair' }],
    ['pair', { id: 'pair', type: 'candidate-pair', state: 'succeeded', localCandidateId: 'local', remoteCandidateId: 'remote' }],
    ['local', { id: 'local', type: 'local-candidate', candidateType, address }],
    ['remote', { id: 'remote', type: 'remote-candidate', candidateType: 'host', address: '192.0.2.99' }],
  ]) as unknown as RTCStatsReport;
}

class Peer extends EventTarget {
  connectionState: RTCPeerConnectionState = 'connected';
  iceConnectionState: RTCIceConnectionState = 'connected';
  iceGatheringState: RTCIceGatheringState = 'complete';
  ice = new EventTarget();
  sctp = { transport: { iceTransport: this.ice } };
  getStats = vi.fn<() => Promise<RTCStatsReport>>().mockResolvedValue(report());
  asRtc() { return this as unknown as RTCPeerConnection; }
  state(connection: RTCPeerConnectionState, ice: RTCIceConnectionState = connection as RTCIceConnectionState) {
    this.connectionState = connection;
    this.iceConnectionState = ice;
    this.dispatchEvent(new Event('connectionstatechange'));
  }
}

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

describe('the source address reported for the selected RTC pair', () => {
  it('reads only the selected local reflexive candidate, including IPv6', () => {
    expect(selectedAddress(report())).toBe('192.0.2.10');
    expect(selectedAddress(report('2001:db8::10', 'prflx'))).toBe('2001:db8::10');
  });

  it('does not substitute a node address, interface address, relay, or other candidate', () => {
    for (const type of ['host', 'relay', 'unknown']) expect(selectedAddress(report('192.0.2.10', type))).toBeNull();
    const stats = report() as unknown as Map<string, Record<string, unknown>>;
    stats.get('pair')!.localCandidateId = 'remote';
    expect(selectedAddress(stats as unknown as RTCStatsReport)).toBeNull();
    stats.delete('transport');
    expect(selectedAddress(stats as unknown as RTCStatsReport)).toBeNull();
  });

  it('rejects redacted, malformed, and unspecified addresses', () => {
    for (const address of ['', 'hidden.local', '192.0.2.999', '01.2.3.4', '2001:db8:::1', '[2001:db8::1]', '0.0.0.0', '::']) {
      expect(selectedAddress(report(address))).toBeNull();
    }
  });

  it('does not use an old successful or nominated pair in place of the selected pair', () => {
    const stats = report() as unknown as Map<string, Record<string, unknown>>;
    stats.get('transport')!.selectedCandidatePairId = 'missing';
    stats.get('pair')!.nominated = true;
    expect(selectedAddress(stats as unknown as RTCStatsReport)).toBeNull();
    stats.get('transport')!.selectedCandidatePairId = 'pair';
    stats.get('pair')!.state = 'failed';
    expect(selectedAddress(stats as unknown as RTCStatsReport)).toBeNull();
  });
});

describe('local RTC address observations', () => {
  it('releases the observation when opening a node connection fails', async () => {
    const pc = new Peer();
    pc.connectionState = 'new';
    pc.iceConnectionState = 'new';
    const dc = Object.assign(new EventTarget(), { close: vi.fn() });
    const closePc = vi.fn();
    Object.assign(pc, {
      createDataChannel: () => dc,
      createOffer: () => Promise.reject(new Error('offer failed')),
      close: closePc,
    });
    class MockPeer {
      constructor() { return pc; }
      static async generateCertificate() {
        return { getFingerprints: () => [{ algorithm: 'sha-256', value: Array(32).fill('00').join(':') }] };
      }
    }
    vi.stubGlobal('RTCPeerConnection', MockPeer);
    let rows: RtcAddress[] = [];
    const unsubscribe = rtcAddresses.subscribe((value) => { rows = value; });
    const onClose = vi.fn();
    try {
      await expect(openRtc([`rtc:${'A'.repeat(43)}@192.0.2.1:7844`], 'c', { message: () => undefined, close: onClose })).rejects.toThrow('offer failed');
      expect(rows).toEqual([]);
      expect(closePc).toHaveBeenCalledOnce();
      expect(dc.close).toHaveBeenCalledOnce();
      expect(onClose).toHaveBeenCalledOnce();
    } finally { unsubscribe(); }
  });

  it('only reads stats while the address display is subscribed', async () => {
    vi.useFakeTimers();
    const addresses = createRtcAddresses();
    const pc = new Peer();
    const close = addresses.track(pc.asRtc(), 'node');
    await vi.advanceTimersByTimeAsync(4000);
    expect(pc.getStats).not.toHaveBeenCalled();
    const unsubscribe = addresses.subscribe(() => undefined);
    await vi.advanceTimersByTimeAsync(0);
    expect(pc.getStats).toHaveBeenCalledOnce();
    unsubscribe();
    expect(vi.getTimerCount()).toBe(0);
    await vi.advanceTimersByTimeAsync(4000);
    expect(pc.getStats).toHaveBeenCalledOnce();
    close();
  });

  it('reports live connections, clears immediately on disconnect, and removes closed links', async () => {
    const addresses = createRtcAddresses();
    const pc = new Peer();
    let rows: RtcAddress[] = [];
    const unsubscribe = addresses.subscribe((value) => { rows = value; });
    const close = addresses.track(pc.asRtc(), 'node');
    await vi.waitFor(() => expect(rows[0]?.address).toBe('192.0.2.10'));
    expect(rows[0]).toMatchObject({ kind: 'node', connected: true });
    pc.state('disconnected');
    expect(rows[0]).toMatchObject({ connected: false, address: null });
    close();
    expect(rows).toEqual([]);
    unsubscribe();
  });

  it('shows unavailable when stats are missing or fail, without retaining an old address', async () => {
    vi.useFakeTimers();
    const addresses = createRtcAddresses();
    const pc = new Peer();
    let rows: RtcAddress[] = [];
    const unsubscribe = addresses.subscribe((value) => { rows = value; });
    const close = addresses.track(pc.asRtc(), 'browser');
    await vi.advanceTimersByTimeAsync(0);
    expect(rows[0].address).toBe('192.0.2.10');
    pc.getStats.mockRejectedValueOnce(new Error('unavailable'));
    await vi.advanceTimersByTimeAsync(2000);
    expect(rows[0]).toMatchObject({ kind: 'browser', connected: true, address: null });
    pc.getStats.mockResolvedValue(new Map() as unknown as RTCStatsReport);
    await vi.advanceTimersByTimeAsync(2000);
    expect(rows[0].address).toBeNull();
    close();
    unsubscribe();
  });

  it('invalidates the old address on a selected-pair change and ignores stale reads', async () => {
    const addresses = createRtcAddresses();
    const pc = new Peer();
    let rows: RtcAddress[] = [];
    const unsubscribe = addresses.subscribe((value) => { rows = value; });
    const close = addresses.track(pc.asRtc(), 'node');
    await vi.waitFor(() => expect(rows[0]?.address).toBe('192.0.2.10'));
    let oldRead!: (value: RTCStatsReport) => void;
    pc.getStats.mockImplementationOnce(() => new Promise((resolve) => { oldRead = resolve; }));
    pc.ice.dispatchEvent(new Event('selectedcandidatepairchange'));
    expect(rows[0].address).toBeNull();
    pc.getStats.mockResolvedValue(report('192.0.2.20'));
    pc.ice.dispatchEvent(new Event('selectedcandidatepairchange'));
    await vi.waitFor(() => expect(rows[0].address).toBe('192.0.2.20'));
    oldRead(report('192.0.2.10'));
    await Promise.resolve();
    expect(rows[0].address).toBe('192.0.2.20');
    close();
    unsubscribe();
  });

  it('cannot resurrect an address after closing while getStats is pending', async () => {
    const addresses = createRtcAddresses();
    const pc = new Peer();
    let finish!: (value: RTCStatsReport) => void;
    pc.getStats.mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
    let rows: RtcAddress[] = [];
    const unsubscribe = addresses.subscribe((value) => { rows = value; });
    const close = addresses.track(pc.asRtc(), 'node');
    close();
    finish(report());
    await Promise.resolve();
    expect(rows).toEqual([]);
    unsubscribe();
  });

  it('clears a previously known address when stats stop answering and cancels timers on close', async () => {
    vi.useFakeTimers();
    const addresses = createRtcAddresses();
    const pc = new Peer();
    let rows: RtcAddress[] = [];
    const unsubscribe = addresses.subscribe((value) => { rows = value; });
    const close = addresses.track(pc.asRtc(), 'node');
    await vi.advanceTimersByTimeAsync(0);
    expect(rows[0].address).toBe('192.0.2.10');
    pc.getStats.mockImplementation(() => new Promise(() => undefined));
    await vi.advanceTimersByTimeAsync(3000);
    expect(rows[0].address).toBeNull();
    close();
    expect(vi.getTimerCount()).toBe(0);
    unsubscribe();
  });

  it('clears during an ICE restart and reads the new route after reconnection', async () => {
    const addresses = createRtcAddresses();
    const pc = new Peer();
    let rows: RtcAddress[] = [];
    const unsubscribe = addresses.subscribe((value) => { rows = value; });
    const close = addresses.track(pc.asRtc(), 'node');
    await vi.waitFor(() => expect(rows[0]?.address).toBe('192.0.2.10'));
    pc.iceConnectionState = 'checking';
    pc.dispatchEvent(new Event('iceconnectionstatechange'));
    expect(rows[0]).toMatchObject({ connected: false, address: null });
    pc.getStats.mockResolvedValue(report('2001:db8::20', 'prflx'));
    pc.iceConnectionState = 'connected';
    pc.dispatchEvent(new Event('iceconnectionstatechange'));
    await vi.waitFor(() => expect(rows[0]?.address).toBe('2001:db8::20'));
    close();
    unsubscribe();
  });
});
