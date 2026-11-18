import { describe, expect, it } from 'vitest';
import type { NetStatus } from '@hoi/core-api';
import { addressScope, connectionIps, connectionIpSummary, connectionTarget } from '../lib/connection-ip';

const net: NetStatus = { mode: 'local', label: 'offline', peers: 0, rtc: [], relaying: false };
const link = { peer: 'named-node', role: 'peer' as const, route: 'direct', connected: true, transport: 'quic', encryption: 'encrypted', observedAddress: '192.0.2.21' };

describe('current connection IP reports', () => {
  it('does not substitute a VPN interface or advertised listening address for observed egress', () => {
    const n = { ...net, vpn: { interface: 'wg0', up: true, addresses: ['10.2.0.2'] }, exposure: [{ address: '192.0.2.99:7844', scope: 'internet' as const, fromRouter: true, told: [] }] };
    expect(connectionIps(n, [])).toEqual([]);
    expect(connectionIpSummary({ ...n, connections: [{ ...link, observedAddress: undefined }] }, [])).toBe('No peer address reported');
  });

  it('preserves each observer and distinct paths rather than claiming one universal address', () => {
    const n = { ...net, connections: [link, { ...link, peer: 'second-node', observedAddress: '2001:db8::25' }] };
    expect(connectionIps(n, []).map(row => [row.observer, row.address])).toEqual([['Node named-node', '192.0.2.21'], ['Node second-node', '2001:db8::25']]);
    expect(connectionIpSummary(n, [])).toBe('192.0.2.21 · 2001:db8::25');
    expect(connectionIpSummary(n, [], '192.0.2.21')).toBe('2001:db8::25');
    expect(connectionIps({ ...n, connections: [{ ...link, connected: false }] }, [])).toEqual([]);
  });

  it('keeps browser source evidence labelled and clears disconnected observations', () => {
    const rtc = [{ id: 3, kind: 'node' as const, connected: true, address: '192.0.2.10' }];
    expect(connectionIps(net, rtc)[0]).toMatchObject({ source: 'browser', observer: 'Node connection 3' });
    expect(connectionIpSummary(net, [{ ...rtc[0], connected: false }])).toBe('No active connection');
  });

  it('identifies local, shared and special-use addresses without claiming public reachability', () => {
    for (const ip of ['127.8.9.1', '::1']) expect(addressScope(ip)).toContain('Loopback');
    for (const ip of ['10.1.2.3', '172.31.8.9', '192.168.1.7', 'fd00::1', '::ffff:192.168.0.1']) expect(addressScope(ip)).toContain('Private');
    for (const ip of ['169.254.8.7', 'fe80::1']) expect(addressScope(ip)).toContain('Link-local');
    expect(addressScope('100.127.99.1')).toContain('Shared carrier');
    expect(addressScope('2001:db8::5')).toContain('Special-use');
    expect(addressScope('8.8.8.8')).toContain('unverified');
  });

  it('names only the intended endpoint rather than invitation data or credentials', () => {
    expect(connectionTarget('wss://example.com:7842/a-private-path?token=secret')).toBe('example.com:7842');
    expect(connectionTarget('rtc:certificate@192.0.2.5:7844')).toBe('192.0.2.5:7844');
  });
});
