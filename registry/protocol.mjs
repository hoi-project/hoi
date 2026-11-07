import { createPublicKey, verify } from 'node:crypto';
import { isIP } from 'node:net';

const ED25519_SPKI = Buffer.from('302a300506032b6570032100', 'hex');
const COMMUNITY = /^[0-9a-f]{64}$/;
const SIGNATURE = /^[0-9a-f]{128}$/;
const PROTOCOL = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;

/** Routes a discovery response may carry; the browser client refuses longer lists. */
export const MAX_PUBLIC_NODES = 16;
/** A node's announcement stays in discovery this long after `issuedAt`; nodes re-announce every five minutes. */
export const NODE_TTL_SECONDS = 30 * 60;
/** An announcement's `issuedAt` may differ from the registry clock by at most this much. */
export const ANNOUNCEMENT_SKEW_SECONDS = 10 * 60;
/** Live announcements retained; older ones are evicted first. */
export const MAX_ANNOUNCED_NODES = 64;
export const MAX_ANNOUNCEMENT_ROUTES = 8;
export const MAX_ANNOUNCEMENT_COMMUNITIES = 64;

export function normalizeName(value) {
  if (typeof value !== 'string') return null;
  const name = value.trim();
  return /^[A-Za-z0-9][A-Za-z0-9._-]{0,31}$/.test(name) ? name.toLowerCase() : null;
}

export function registrationStatement(name, community) {
  return `hoi community name v1\n${name}\n${community}`;
}

export function publicationStatement(name, community, protocolVersion, indexVersion) {
  return `hoi community index v2\n${name}\n${community}\n${protocolVersion}\n${indexVersion}`;
}

/** The exact UTF-8 bytes a node's transport key signs to announce its public routes to one
 * registry (PROTOCOL.md §10). The registry's origin is part of it, so a registry cannot pass an
 * announcement it was sent on to another one while it is still fresh. */
export function announcementStatement(registry, endpoint, issuedAt, routes, communities) {
  return `hoi public node v2\n${registry}\n${endpoint}\n${issuedAt}\n${routes.join(',')}\n${communities.join(',')}`;
}

const LOOPBACK_HOSTS = ['localhost', '127.0.0.1', '[::1]'];

/** The origin a registry answers to, from its public URL as carriers are given it with
 * `--index` (path and all): HTTPS, or plain HTTP on this machine, nothing secret in it. */
export function registryOrigin(value) {
  let url;
  try { url = new URL(value); } catch { url = null; }
  if (!url || (url.protocol !== 'https:' && !(url.protocol === 'http:' && LOOPBACK_HOSTS.includes(url.hostname)))
    || url.username || url.password || url.search || url.hash) {
    throw new Error('The registry URL must be its public https:// address (http:// only on this machine), without credentials, a query or a fragment.');
  }
  return url.origin;
}

/** Without a configured URL a registry answers only to the loopback address a connection reached
 * it at, for development and tests: `http://127.0.0.1:<port>` or `http://[::1]:<port>`, and
 * `http://localhost:<port>`. The address comes from the socket, never from a request header. */
export function loopbackOrigins(address, port) {
  const at = port === 80 ? '' : `:${port}`;
  if (address === '127.0.0.1' || address === '::ffff:127.0.0.1') return [`http://127.0.0.1${at}`, `http://localhost${at}`];
  if (address === '::1') return [`http://[::1]${at}`, `http://localhost${at}`];
  return [];
}

/** The endpoint keys an operator allows to announce (allowlist mode), or null when any node
 * may. Accepts an array or the comma-separated `HOI_REGISTRY_ANNOUNCERS` value. */
export function validateAnnouncers(value) {
  if (value === undefined || value === null) return null;
  const keys = (Array.isArray(value) ? value : String(value).split(',')).map((key) => typeof key === 'string' ? key.trim() : key).filter((key) => key !== '');
  if (keys.length > 1024 || !keys.every(keyIsStrong)) throw new Error('Allowed announcers must be 64-character lowercase hex endpoint keys, at most 1024.');
  return keys.length ? new Set(keys) : null;
}

/** libsodium's small-order blocklist (sign bit ignored): a key here verifies forged signatures.
 * Node's verifier accepts such points, so the registry refuses them itself, like the Rust core. */
const SMALL_ORDER = [
  "0000000000000000000000000000000000000000000000000000000000000000",
  "0100000000000000000000000000000000000000000000000000000000000000",
  "26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc05",
  "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a",
  "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
  "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
  "eeffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
].map((hex) => Buffer.from(hex, "hex"));

/** A valid, canonical Ed25519 point outside the small-order subgroup. */
export function keyIsStrong(hex) {
  if (typeof hex !== "string" || !COMMUNITY.test(hex)) return false;
  const bytes = Buffer.from(hex, "hex");
  const masked = Buffer.from(bytes);
  masked[31] &= 0x7f;
  if (SMALL_ORDER.some((point) => point.equals(masked))) return false;
  // Non-canonical encodings (y >= p) are refused as well; p = 2^255 - 19.
  if (masked[31] === 0x7f && masked.subarray(1, 31).every((b) => b === 0xff) && masked[0] >= 0xed) return false;
  try { publicKey(hex); return true; } catch { return false; }
}

function publicKey(hex) {
  return createPublicKey({ key: Buffer.concat([ED25519_SPKI, Buffer.from(hex, 'hex')]), format: 'der', type: 'spki' });
}

export function verifiedClaim(claim, version = 1) {
  if (!claim || typeof claim !== 'object' || Array.isArray(claim)) return null;
  if (Object.keys(claim).sort().join(',') !== (version === 2 ? 'community,indexVersion,name,protocolVersion,signature' : 'community,name,signature')) return null;
  const name = normalizeName(claim.name);
  if (!name || !keyIsStrong(claim.community)
    || typeof claim.signature !== 'string' || !SIGNATURE.test(claim.signature)) return null;
  if (version === 2 && (typeof claim.protocolVersion !== 'string' || !PROTOCOL.test(claim.protocolVersion)
    || !Number.isInteger(claim.indexVersion) || claim.indexVersion < 1 || claim.indexVersion > 2_147_483_647)) return null;
  try {
    const statement = version === 2
      ? publicationStatement(name, claim.community, claim.protocolVersion, claim.indexVersion)
      : registrationStatement(name, claim.community);
    if (!verify(null, Buffer.from(statement, 'utf8'), publicKey(claim.community), Buffer.from(claim.signature, 'hex'))) return null;
    return { name, community: claim.community, signature: claim.signature, ...(version === 2 ? { protocolVersion: claim.protocolVersion, indexVersion: claim.indexVersion } : {}) };
  } catch {
    return null;
  }
}

/** A community key as the registry prints it, or null. */
export function normalizeCommunityKey(value) {
  return typeof value === 'string' && COMMUNITY.test(value) ? value : null;
}

/** The 16-bit groups of an IPv6 address, or null. Handles `::` and a trailing dotted IPv4. */
function ipv6Groups(ip) {
  const [head, tail = ''] = ip.includes('::') ? ip.split('::') : [ip, undefined];
  const expand = (part) => part === '' ? [] : part.split(':').flatMap((g) => {
    if (g.includes('.')) {
      const [a, b, c, d] = g.split('.').map(Number);
      return [(a << 8) | b, (c << 8) | d];
    }
    return [parseInt(g, 16)];
  });
  const front = expand(head);
  const back = tail === undefined ? [] : expand(tail);
  if (front.length + back.length > 8 || (tail === undefined && front.length !== 8)) return null;
  return [...front, ...Array(8 - front.length - back.length).fill(0), ...back];
}

/** Whether `ip` lies outside every visitor's own network: no loopback, private, link-local,
 * carrier-grade NAT, unspecified or multicast ranges. A third party's announcement must not
 * make every device that reads the directory probe addresses on its own LAN or host; the
 * documentation ranges (`203.0.113.0/24`, `2001:db8::/32`) are unroutable and stay allowed so
 * the shared test vectors keep working. */
export function publicIp(ip) {
  const family = isIP(ip);
  if (family === 4) {
    const [a, b] = ip.split('.').map(Number);
    return !(a === 0 || a === 10 || a === 127 || a >= 224
      || (a === 100 && b >= 64 && b <= 127) || (a === 169 && b === 254) || (a === 172 && b >= 16 && b <= 31)
      || (a === 192 && b === 168) || /^192\.0\.0\./.test(ip));
  }
  if (family !== 6) return false;
  const groups = ipv6Groups(ip);
  if (!groups) return false;
  const [g0, g1, g2, g3, g4, g5, g6, g7] = groups;
  if (g0 === 0 && g1 === 0 && g2 === 0 && g3 === 0 && g4 === 0) {
    if (g5 === 0xffff) return publicIp(`${g6 >> 8}.${g6 & 255}.${g7 >> 8}.${g7 & 255}`); // IPv4-mapped
    if (g5 === 0 && g6 === 0 && (g7 === 0 || g7 === 1)) return false; // unspecified, loopback
  }
  if ((g0 & 0xfe00) === 0xfc00) return false; // unique local fc00::/7
  if ((g0 & 0xffc0) === 0xfe80) return false; // link-local
  if ((g0 & 0xff00) === 0xff00) return false; // multicast
  return true;
}

/** One public infrastructure route: `rtc:<fingerprint>@<ip>:<port>`, `<endpoint key>@<ip>:<port>` or a bare endpoint key.
 * `publicOnly` refuses addresses other networks cannot reach (what a third party announces);
 * the operator's own configuration may still name a loopback address for development. */
export function validateRoute(route, publicOnly = false) {
  if (typeof route !== 'string' || route.length > 128) throw new Error('Invalid public node route.');
  if (COMMUNITY.test(route)) return route;
  const rtc = route.startsWith('rtc:');
  const match = /^(?:rtc:)?([A-Za-z0-9_-]+)@(?:\[([0-9a-fA-F:.]+)\]|([0-9.]+)):(\d{1,5})$/.exec(route);
  if (!match || (rtc ? !/^[A-Za-z0-9_-]{43}$/.test(match[1]) : !COMMUNITY.test(match[1]))) {
    throw new Error('Public nodes need an RTC certificate fingerprint or native endpoint key and a socket address.');
  }
  const ip = match[2] || match[3];
  const fingerprint = rtc ? Buffer.from(match[1], 'base64url') : null;
  if (!isIP(ip) || (match[2] ? isIP(ip) !== 6 : isIP(ip) !== 4) || Number(match[4]) < 1 || Number(match[4]) > 65535
    || (rtc && (fingerprint.length !== 32 || fingerprint.toString('base64url') !== match[1]))
    || ip === '0.0.0.0' || (isIP(ip) === 6 && /^[0:]+$/.test(ip))
    || /^(?:22[4-9]|23\d)\./.test(ip) || /^ff/i.test(ip)) throw new Error('Invalid public node route.');
  if (publicOnly && !publicIp(ip)) throw new Error('Announced routes must name a public address.');
  return route;
}

export function validatePublicNodes(nodes) {
  if (!Array.isArray(nodes) || nodes.length > MAX_PUBLIC_NODES) throw new Error(`Public nodes must be an array of at most ${MAX_PUBLIC_NODES} peer routes.`);
  return [...new Set(nodes.map((route) => validateRoute(route)))];
}

/** A node announcing its own public routes to this registry, signed by its transport (endpoint)
 * key. Returns the stored shape or null. `now` is the registry clock in Unix seconds; stale and
 * future-dated announcements are refused, so a captured announcement cannot be replayed later,
 * and one signed for another registry (`registry` not among `registries`, the origins this one
 * answers to) cannot be replayed here at all. `publicOnly` is false only in tests. The version 1
 * statement named no registry and is refused. */
export function verifiedAnnouncement(claim, now, { registries = [], publicOnly = true } = {}) {
  if (!claim || typeof claim !== 'object' || Array.isArray(claim)) return null;
  if (Object.keys(claim).sort().join(',') !== 'communities,endpoint,issuedAt,registry,routes,signature') return null;
  const { registry, endpoint, issuedAt, routes, communities, signature } = claim;
  if (typeof registry !== 'string' || !registries.includes(registry)) return null;
  if (!keyIsStrong(endpoint) || typeof signature !== 'string' || !SIGNATURE.test(signature)) return null;
  if (!Number.isSafeInteger(issuedAt) || issuedAt < 0 || Math.abs(issuedAt - now) > ANNOUNCEMENT_SKEW_SECONDS) return null;
  if (!Array.isArray(routes) || routes.length < 1 || routes.length > MAX_ANNOUNCEMENT_ROUTES || new Set(routes).size !== routes.length) return null;
  if (!Array.isArray(communities) || communities.length > MAX_ANNOUNCEMENT_COMMUNITIES || new Set(communities).size !== communities.length) return null;
  try {
    for (const route of routes) {
      validateRoute(route, publicOnly);
      // A bare key or native route names this node's own endpoint; it cannot point at someone else's.
      if (!route.startsWith('rtc:') && route.split('@')[0] !== endpoint) return null;
    }
  } catch {
    return null;
  }
  for (let i = 0; i < communities.length; i++) {
    if (!normalizeCommunityKey(communities[i]) || (i > 0 && communities[i] <= communities[i - 1])) return null;
  }
  try {
    const statement = announcementStatement(registry, endpoint, issuedAt, routes, communities);
    if (!verify(null, Buffer.from(statement, 'utf8'), publicKey(endpoint), Buffer.from(signature, 'hex'))) return null;
  } catch {
    return null;
  }
  return { endpoint, routes: [...routes], communities: [...communities], issuedAt };
}

/** What a registry answers to `PUT /v2/nodes` before it touches its database: `{ announcement }`
 * to store, or `{ status, error, message }` to refuse. `registries` are the origins it answers to
 * (none: it does not know its public URL and takes no announcements); `announcers` the endpoint
 * keys its operator allows, or null for any; `publicOnly` is false only in tests. Shared by the
 * Node service and the Worker; `hoi-node --registry` answers the same. */
export function admitAnnouncement(claim, now, { registries, announcers = null, publicOnly = true }) {
  if (!registries.length) {
    return { status: 403, error: 'announcements-disabled', message: 'This registry does not know its own public URL, so it cannot tell which registry an announcement was signed for. Its operator must configure that URL.' };
  }
  if (claim && typeof claim === 'object' && typeof claim.registry === 'string' && !registries.includes(claim.registry)) {
    return { status: 400, error: 'wrong-registry', message: 'This announcement was signed for another registry. A node signs a separate announcement for each registry it is given.' };
  }
  const announcement = verifiedAnnouncement(claim, now, { registries, publicOnly });
  if (!announcement) {
    return { status: 400, error: 'invalid-announcement', message: 'The node announcement could not be verified: its transport key must sign this registry\'s origin, its current routes and community keys, dated within ten minutes.' };
  }
  if (announcers && !announcers.has(announcement.endpoint)) {
    return { status: 403, error: 'announcer-not-allowed', message: 'This registry lists only the public nodes its operator allowed.' };
  }
  return { announcement };
}

/** Refusal of an announcement from a node the operator dropped (`registry/admin.mjs drop-node`). */
export const ANNOUNCER_REFUSED = { error: 'announcer-refused', message: 'This registry\'s operator does not list this node.' };
/** A listing the operator withholds: still reserved, never served. */
export const WITHHELD = { error: 'withheld', message: 'This registry withholds this public name: it stays reserved and is not listed.' };

/** Routes of `nodes` taken one per node per round, so no single announcement fills the list. */
function roundRobin(nodes) {
  const out = [];
  const queues = nodes.map((node) => [...node.routes]);
  while (queues.some((q) => q.length)) for (const q of queues) if (q.length) out.push(q.shift());
  return out;
}

/** Discovery routes in the order a client should try them: the operator's configured routes
 * always first (nobody can announce them out of the list), then nodes that announced carrying
 * `community`, then the other live nodes, each group taken one route per node per round so a
 * node announcing eight routes cannot crowd out the rest. */
export function selectPublicNodes(configured, live, community = null) {
  const carrying = community ? live.filter((node) => node.communities.includes(community)) : [];
  const others = live.filter((node) => !carrying.includes(node));
  const ordered = [...configured, ...roundRobin(carrying), ...roundRobin(others)];
  return [...new Set(ordered)].slice(0, MAX_PUBLIC_NODES);
}

export function validateAppUrl(value) {
  const url = new URL(value);
  const loopback = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
  if ((url.protocol !== 'https:' && !(url.protocol === 'http:' && loopback)) || url.username || url.password || url.search || url.hash) {
    throw new Error('The app URL must be HTTPS (or loopback HTTP), without credentials, a query or a fragment.');
  }
  return url.href;
}
