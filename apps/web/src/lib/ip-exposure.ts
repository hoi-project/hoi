// What a checked IP tells others, as one status. Pure: the caller supplies observations, lookups and names.
// A report never proves that a VPN is on or off (RFC-0007), so no status reads as "safe".
export type Lookup =
  | { state: 'local' }
  | { state: 'idle' | 'loading' | 'failed' }
  | { state: 'ready'; code: string | null };

export interface Observation {
  address: string;
  /** Who saw the address, e.g. the IP service's host name or a node. */
  observer: string;
  lookup: Lookup;
}

export type ExposureKind = 'unchecked' | 'checking' | 'unlocated' | 'locating' | 'same' | 'other' | 'no-zone' | 'routes' | 'local' | 'unlisted' | 'unavailable';
export type Tone = 'idle' | 'peach' | 'sky' | 'sun' | 'lilac' | 'dashed';
export type StatusIcon = 'ring' | 'dot' | 'pin' | 'away' | 'fork' | 'home' | 'off';

export interface Exposure {
  kind: ExposureKind;
  tone: Tone;
  icon: StatusIcon;
  /** Short text for the pill. */
  short: string;
  /** Extra pill text shown when there is room. */
  note: string;
  /** One or two sentences for the card and the pill's accessible name. */
  long: string;
  /** Distinct listed countries, first observation first. */
  codes: string[];
  /** The address the status is about. */
  address?: string;
}

export interface ExposureInput {
  /** A browser check is running. */
  pending: boolean;
  /** Browser HTTPS sample first, then peer reports. */
  observations: Observation[];
  zone: string | null;
  zoneCode: string | null;
  name: (code: string) => string;
}

export const NO_VPN_CLAIM = 'This does not show whether a VPN is on.';
const SEEN = 'Nodes you connect to see this IP.';

export function exposureStatus({ pending, observations, zone, zoneCode, name }: ExposureInput): Exposure {
  const base = { codes: [] as string[] };
  if (!observations.length) {
    return pending
      ? { ...base, kind: 'checking', tone: 'idle', icon: 'dot', short: 'Checking IP…', note: '', long: 'Checking your IP.' }
      : { ...base, kind: 'unchecked', tone: 'idle', icon: 'ring', short: 'IP not checked', note: '', long: 'Your IP is not checked. Nodes you connect to see it.' };
  }
  const internet = observations.filter(o => o.lookup.state !== 'local');
  if (!internet.length) {
    const first = observations[0];
    return { ...base, kind: 'local', tone: 'lilac', icon: 'home', short: 'Local route', note: '', address: first.address,
      long: `${first.observer} sees you as ${first.address} on a local or special-use address. Peers on the internet can see a different IP.` };
  }
  const first = internet[0];
  const ready = internet.filter((o): o is Observation & { lookup: { state: 'ready'; code: string | null } } => o.lookup.state === 'ready');
  const codes = [...new Set(ready.map(o => o.lookup.code).filter((c): c is string => !!c))];
  if (codes.length > 1) {
    const one = (code: string) => ready.find(o => o.lookup.code === code)!;
    const [a, b] = [one(codes[0]), one(codes[1])];
    return { codes, kind: 'routes', tone: 'sun', icon: 'fork', short: 'Routes differ', note: codes.join(' · '), address: a.address,
      long: `${a.observer} saw ${a.address} in ${name(codes[0])}. ${b.observer} saw ${b.address} in ${name(codes[1])}. Your connections leave by different routes. ${NO_VPN_CLAIM}` };
  }
  if (codes.length === 1) {
    const code = codes[0];
    const located = ready.find(o => o.lookup.code === code)!;
    const country = name(code);
    if (zoneCode === code) {
      return { codes, kind: 'same', tone: 'peach', icon: 'pin', short: country, note: 'your time zone', address: located.address,
        long: `${located.address} is listed in ${country}, the same country as your device’s time zone. ${SEEN} ${NO_VPN_CLAIM}` };
    }
    if (zoneCode) {
      return { codes, kind: 'other', tone: 'sky', icon: 'away', short: country, note: 'not your time zone', address: located.address,
        long: `${located.address} is listed in ${country}. Your device’s time zone is in ${name(zoneCode)}. ${SEEN} ${NO_VPN_CLAIM}` };
    }
    return { codes, kind: 'no-zone', tone: 'sky', icon: 'pin', short: country, note: '', address: located.address,
      long: `${located.address} is listed in ${country}. ${zone ? `Your device’s time zone (${zone}) is not tied to a country.` : 'Your device’s time zone is unknown.'} ${SEEN} ${NO_VPN_CLAIM}` };
  }
  if (internet.some(o => o.lookup.state === 'loading')) {
    return { ...base, kind: 'locating', tone: 'idle', icon: 'dot', short: first.address, note: 'finding country…', address: first.address,
      long: `${first.observer} sees you as ${first.address}. Finding its country on this device.` };
  }
  if (ready.length) {
    return { ...base, kind: 'unlisted', tone: 'idle', icon: 'ring', short: 'Country unknown', note: '', address: first.address,
      long: `hoi’s country table does not list ${first.address}. ${SEEN} ${NO_VPN_CLAIM}` };
  }
  if (internet.some(o => o.lookup.state === 'failed')) {
    return { ...base, kind: 'unavailable', tone: 'dashed', icon: 'off', short: 'Country unavailable', note: '', address: first.address,
      long: `The country table could not be loaded. ${first.observer} sees you as ${first.address}.` };
  }
  return { ...base, kind: 'unlocated', tone: 'idle', icon: 'ring', short: first.address, note: '', address: first.address,
    long: `${first.observer} sees you as ${first.address}. Show its country to compare it with your time zone.` };
}
