// Reading a pairing link before trusting it: where it really points. The name in a link
// ("lena's laptop") is whatever the link says; the address is what the device will dial.

export type HomeScope = 'device' | 'network' | 'internet' | 'none';

/** Plain words for the addresses a pairing link carries: this device, your network, or the internet. */
export function describeHome(u: string | null, w: string | null): { host: string; scope: HomeScope } {
  const hosts: string[] = [];
  if (u) {
    try {
      hosts.push(new URL(u).hostname.replace(/^\[|\]$/g, ''));
    } catch {
      hosts.push(u);
    }
  }
  for (const route of (w ?? '').split(',').filter(Boolean)) {
    const m = /@(?:\[([^\]]+)\]|([^:]+)):\d+$/.exec(route);
    if (m) hosts.push(m[1] ?? m[2]);
  }
  if (!hosts.length) return { host: '', scope: 'none' };
  // Only IP literals are graded by their prefix; a name such as 127.example.com or droid.example is a
  // public hostname whatever it starts with.
  const v4 = (h: string) => /^\d{1,3}(\.\d{1,3}){3}$/.test(h);
  const v6 = (h: string) => h.includes(':');
  const local = (h: string) => h === 'localhost' || (v6(h) && h === '::1') || (v4(h) && /^127\./.test(h));
  const lan = (h: string) =>
    (v4(h) && /^(10|192\.168|169\.254|172\.(1[6-9]|2\d|3[01]))\./.test(h)) || (v6(h) && /^(f[cd]|fe80)/i.test(h)) || h.endsWith('.local');
  const scope: HomeScope = hosts.every(local) ? 'device' : hosts.every((h) => local(h) || lan(h)) ? 'network' : 'internet';
  return { host: [...new Set(hosts)].join(', '), scope };
}
