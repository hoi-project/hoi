// Being a public node, in words: what the router said when asked to forward this computer's
// ports (hoi-net portmap), and what that means for people outside the home.
import type { Exposure, NetStatus, PortMapping } from '@hoi/core-api';

export interface RouterWords {
  /** reachable: people on other networks can reach this computer; waiting: still asking;
   * blocked: they can't, and `text` says what would change that. */
  state: 'reachable' | 'waiting' | 'blocked';
  text: string;
}

/** What the router's answer means for people outside, in one or two sentences. */
export function routerWords(pm: PortMapping | undefined): RouterWords {
  const through = pm?.viaVpn ? 'your VPN' : 'your router';
  if (!pm || pm.state === 'asking') return { state: 'waiting', text: `Asking ${through} to forward this computer's ports…` };
  switch (pm.state) {
    case 'forwarded':
      return { state: 'reachable', text: `${cap(through)} forwards from ${list(pm.public)}. That is the address people outside your home reach this computer at.` };
    case 'own-address':
      return { state: 'reachable', text: `This computer has a public address of its own (${pm.detail ?? 'from your provider'}), so there is nothing to forward.` };
    case 'behind-nat':
      return {
        state: 'blocked',
        text: `${cap(through)} forwards from ${pm.detail ?? 'an address'}, which isn't a public address: your provider puts another router in front of yours (carrier-grade NAT), so nobody outside can reach this computer this way. Ask your provider for a public IPv4 address, or run hoi-node on a small server.`,
      };
    case 'refused':
      return { state: 'blocked', text: `${cap(through)} said no${pm.detail ? ` (${pm.detail})` : ''}. Check its settings for which devices may open ports.` };
    case 'silent':
      return pm.viaVpn
        ? { state: 'blocked', text: "Your VPN didn't answer a port forwarding request. Only some VPNs forward ports (over NAT-PMP); without that, this computer can't be a public node through it." }
        : { state: 'blocked', text: "Your router didn't answer. Turn on UPnP or NAT-PMP in its settings (sometimes called automatic port forwarding), then try again." };
    case 'no-router':
      return { state: 'blocked', text: pm.viaVpn ? 'Your VPN is down, so its gateway can’t be asked.' : 'This computer has no route to the internet right now.' };
    case 'failed':
      return { state: 'blocked', text: `Port forwarding couldn't start${pm.detail ? `: ${pm.detail}` : ''}.` };
  }
}

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);
const list = (addrs: string[]) => (addrs.length ? addrs.join(' and ') : 'its public address');

/** One address of this node's, by IP: its ports merged (computers and browsers reach the same IP
 * on two ports), and everyone told any of them. */
export interface AddressRow {
  host: string;
  scope: Exposure['scope'];
  fromRouter: boolean;
  ports: { port: string; who: 'computers' | 'browsers and phones' }[];
  told: Exposure['told'];
}

/** `203.0.113.7:7844` → host and port; IPv6 comes bracketed (`[2001:db8::1]:7844`). */
export function splitAddress(address: string): { host: string; port: string } {
  const i = address.lastIndexOf(':');
  if (i < 0) return { host: address, port: '' };
  return { host: address.slice(0, i).replace(/^\[(.*)\]$/, '$1'), port: address.slice(i + 1) };
}

/** The exposure list by IP, in the order the node gave it. `rtc` routes (`rtc:<fp>@<addr>`) say
 * which ports browsers and phones use; the others are for computers. */
export function addressRows(exposure: Exposure[], rtc: string[] = []): AddressRow[] {
  const forBrowsers = new Set(rtc.map((r) => r.slice(r.lastIndexOf('@') + 1)));
  const order: Exposure['told'] = ['devices', 'strangers', 'node-link'];
  const rows = new Map<string, AddressRow>();
  for (const e of exposure) {
    const { host, port } = splitAddress(e.address);
    const row = rows.get(host) ?? { host, scope: e.scope, fromRouter: false, ports: [], told: [] };
    row.fromRouter ||= e.fromRouter;
    if (port && !row.ports.some((p) => p.port === port)) row.ports.push({ port, who: forBrowsers.has(e.address) ? 'browsers and phones' : 'computers' });
    row.told = order.filter((t) => row.told.includes(t) || e.told.includes(t));
    rows.set(host, row);
  }
  return [...rows.values()];
}

/** Who sees the address this device's connections come from, right now: every device that
 * answered, as with any connection; a link that stays on this computer (the loopback WebSocket
 * to its own browser) shows nobody anything. */
export function whoSees(net: NetStatus | null | undefined): { own: number; others: number } {
  const live = (net?.connections ?? []).filter((c) => c.connected && c.encryption !== 'stays on this device');
  return { own: live.filter((c) => c.role !== 'peer').length, others: live.filter((c) => c.role === 'peer').length };
}
