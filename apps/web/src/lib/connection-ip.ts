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

/** Names the exact destination without showing credentials, URL paths or invitation contents. */
export function connectionTarget(route: string): string {
  if (route.startsWith('rtc:')) return route.slice(route.lastIndexOf('@') + 1);
  try { return new URL(route).host; } catch { return route.slice(0, 96); }
}
