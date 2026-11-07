import { describe, expect, it } from 'vitest';
import { addressRows, routerWords, splitAddress, whoSees } from '../lib/public-node';

describe('what the router said', () => {
  it('names the public address people outside reach', () => {
    const w = routerWords({ state: 'forwarded', public: ['203.0.113.7:7844'], viaVpn: false });
    expect(w.state).toBe('reachable');
    expect(w.text).toContain('203.0.113.7:7844');
    expect(w.text).toContain('Your router');
  });

  it('is still asking before the router answers', () => {
    expect(routerWords(undefined).state).toBe('waiting');
    expect(routerWords({ state: 'asking', public: [], viaVpn: true }).text).toContain('your VPN');
  });

  it('says plainly when nobody outside can reach this computer, and what would change it', () => {
    const nat = routerWords({ state: 'behind-nat', public: [], detail: '100.64.1.2', viaVpn: false });
    expect(nat.state).toBe('blocked');
    expect(nat.text).toContain('100.64.1.2');
    expect(nat.text).toContain('carrier-grade NAT');
    expect(routerWords({ state: 'silent', public: [], viaVpn: false }).text).toContain('UPnP');
    expect(routerWords({ state: 'silent', public: [], viaVpn: true }).text).toContain('VPN');
    expect(routerWords({ state: 'refused', public: [], detail: 'PCP: not authorized', viaVpn: false }).text).toContain('PCP: not authorized');
  });
});

describe('your addresses, in words', () => {
  it('lists each IP once, with the ports computers and browsers use', () => {
    const rows = addressRows(
      [
        { address: '192.168.1.20:7843', scope: 'network', fromRouter: false, told: ['devices'] },
        { address: '192.168.1.20:7844', scope: 'network', fromRouter: false, told: ['devices', 'strangers'] },
        { address: '[2001:db8::20]:7844', scope: 'internet', fromRouter: true, told: ['devices', 'node-link'] },
      ],
      ['rtc:abc@192.168.1.20:7844', 'rtc:abc@[2001:db8::20]:7844'],
    );
    expect(rows).toEqual([
      { host: '192.168.1.20', scope: 'network', fromRouter: false, ports: [{ port: '7843', who: 'computers' }, { port: '7844', who: 'browsers and phones' }], told: ['devices', 'strangers'] },
      { host: '2001:db8::20', scope: 'internet', fromRouter: true, ports: [{ port: '7844', who: 'browsers and phones' }], told: ['devices', 'node-link'] },
    ]);
    expect(splitAddress('[::1]:7842')).toEqual({ host: '::1', port: '7842' });
  });

  it('counts who sees this connection, loopback links aside', () => {
    const net = {
      mode: 'local' as const, label: '', peers: 3, rtc: [], relaying: true,
      connections: [
        { peer: 'a', role: 'peer' as const, route: 'direct', connected: true, transport: 'webrtc', encryption: '' },
        { peer: 'b', role: 'client' as const, route: 'direct', connected: true, transport: 'webrtc', encryption: '' },
        { peer: 'c', role: 'client' as const, route: 'direct', connected: true, transport: 'websocket', encryption: 'stays on this device' },
        { peer: 'd', role: 'peer' as const, route: 'direct', connected: false, transport: 'quic', encryption: '' },
      ],
    };
    expect(whoSees(net)).toEqual({ own: 1, others: 1 });
    expect(whoSees(null)).toEqual({ own: 0, others: 0 });
  });
});
