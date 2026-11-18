import type { NetStatus } from '@hoi/core-api';
import type { RtcAddress } from './core/rtc-addresses';

export interface ConnectionIp {
  id: string;
  address: string;
  observer: string;
  source: 'node' | 'browser';
  path: string;
}

/** Reports about a live path, never a listening hint or a VPN interface address. */
export function connectionIps(net: NetStatus | null, rtc: readonly RtcAddress[]): ConnectionIp[] {
  const reported = (net?.connections ?? []).filter(c => c.connected && c.encryption !== 'stays on this device')
    .flatMap(c => {
      const address = c.observedAddress;
      return address ? [{
        id: `peer:${c.peer}`, address,
        observer: c.role === 'home' ? net?.homeNode?.name || 'Your home node' : `${c.kind === 'browser' ? 'Browser' : 'Node'} ${c.peer.slice(0, 12)}`,
        source: 'node' as const, path: `${c.transport} · ${c.route}`,
      }] : [];
    });
  const browser = rtc.filter(c => c.connected && c.address).map(c => ({
    id: `rtc:${c.id}`, address: c.address!, observer: `${c.kind === 'node' ? 'Node' : 'Browser'} connection ${c.id}`,
    source: 'browser' as const, path: 'WebRTC selected path',
  }));
  return [...reported, ...browser];
}

export function connectionIpSummary(net: NetStatus | null, rtc: readonly RtcAddress[]): string {
  const addresses = [...new Set(connectionIps(net, rtc).map(row => row.address))];
  if (addresses.length) return addresses.join(' · ');
  const connected = net?.connections?.some(c => c.connected && c.encryption !== 'stays on this device') || rtc.some(c => c.connected);
  return connected ? 'IP unavailable' : 'No active connection';
}

/** Scope of a numeric observation. Ordinary unicast alone does not prove public reachability. */
export function addressScope(address: string): string {
  if (address.includes(':')) {
    let ip: string;
    try { ip = new URL(`http://[${address}]/`).hostname.slice(1, -1); }
    catch { return 'Address scope unknown'; }
    if (ip === '::1') return 'Loopback · same device';
    const mapped = /^::ffff:([a-f0-9]{1,4}):([a-f0-9]{1,4})$/.exec(ip);
    if (mapped) {
      const hi = parseInt(mapped[1], 16), lo = parseInt(mapped[2], 16);
      return addressScope([hi >> 8, hi & 255, lo >> 8, lo & 255].join('.'));
    }
    if (/^f[cd]/.test(ip)) return 'Private network · local route';
    if (/^fe[89ab]/.test(ip)) return 'Link-local · local route';
    if (ip.startsWith('2001:db8:') || (ip.startsWith('3fff:') && parseInt(ip.split(':')[1] || '0', 16) < 0x1000) || ip === '::') return 'Special-use address';
  } else {
    const [a, b, c] = address.split('.').map(Number);
    if (a === 127) return 'Loopback · same device';
    if (a === 10 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168)) return 'Private network · local route';
    if (a === 169 && b === 254) return 'Link-local · local route';
    if (a === 100 && b >= 64 && b <= 127) return 'Shared carrier network address';
    if (a === 0 || a >= 224 || (a === 192 && b === 0 && c === 2)
      || (a === 198 && (b === 18 || b === 19 || (b === 51 && c === 100)))
      || (a === 203 && b === 0 && c === 113)) return 'Special-use address';
  }
  return 'Observed IP · public reachability unverified';
}

/** Names the exact destination without showing credentials, URL paths or invitation contents. */
export function connectionTarget(route: string): string {
  if (route.startsWith('rtc:')) return route.slice(route.lastIndexOf('@') + 1);
  try { return new URL(route).host; } catch { return route.slice(0, 96); }
}
