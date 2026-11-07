// Shared by the static download page and browser app. Only bundled keys establish trust.
import { ed25519 } from '@noble/curves/ed25519.js';
import { blake2b } from '@noble/hashes/blake2.js';
import { validateRelease } from './release-schema.mjs';

const utf8 = new TextEncoder();
const decode = text => {
  if (!/^[A-Za-z0-9+/]+={0,2}$/.test(text)) throw new Error('Invalid signature encoding.');
  return Uint8Array.from(atob(text), c => c.charCodeAt(0));
};
export function verifyMinisign(bytes, signature, key) {
  if (!key || signature.length > 4096) throw new Error('Release signing key or signature unavailable.');
  const pk = decode(key), lines = signature.trim().split(/\r?\n/);
  if (pk.length !== 42 || pk[0] !== 69 || pk[1] !== 100 || lines.length !== 4
      || !lines[0].startsWith('untrusted comment: ') || !lines[2].startsWith('trusted comment: ')) throw new Error('Invalid release signature.');
  const sig = decode(lines[1]), commentSig = decode(lines[3]);
  if (sig.length !== 74 || sig[0] !== 69 || sig[1] !== 68 || commentSig.length !== 64
      || !sig.slice(2, 10).every((v, i) => v === pk[i + 2])) throw new Error('Release signature uses the wrong key or format.');
  const comment = utf8.encode(lines[2].slice('trusted comment: '.length));
  const signedComment = new Uint8Array(64 + comment.length);
  signedComment.set(sig.slice(10)); signedComment.set(comment, 64);
  if (!ed25519.verify(sig.slice(10), blake2b(bytes, { dkLen: 64 }), pk.slice(10), { zip215: false })
      || !ed25519.verify(commentSig, signedComment, pk.slice(10), { zip215: false })) throw new Error('Release signature did not verify.');
}

export function compareVersions(a, b) {
  const parse = value => {
    const m = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.exec(value);
    if (typeof value !== 'string' || value.length > 100 || !m || m[4]?.split('.').some(x => /^0\d+$/.test(x))) throw new Error('Invalid release version.');
    return m;
  };
  const x = parse(a), y = parse(b);
  for (let i = 1; i <= 3; i++) if (BigInt(x[i]) !== BigInt(y[i])) return BigInt(x[i]) > BigInt(y[i]) ? 1 : -1;
  if (x[4] === y[4]) return 0;
  if (!x[4] || !y[4]) return x[4] ? -1 : 1;
  const xp = x[4].split('.'), yp = y[4].split('.');
  for (let i = 0; i < Math.max(xp.length, yp.length); i++) {
    if (xp[i] === yp[i]) continue;
    if (xp[i] === undefined || yp[i] === undefined) return xp[i] === undefined ? -1 : 1;
    const xn = /^\d+$/.test(xp[i]), yn = /^\d+$/.test(yp[i]);
    if (xn && yn) return BigInt(xp[i]) > BigInt(yp[i]) ? 1 : -1;
    if (xn !== yn) return xn ? -1 : 1;
    return xp[i] > yp[i] ? 1 : -1;
  }
  return 0;
}

export function httpsUrl(raw) {
  const u = new URL(raw);
  if (u.protocol !== 'https:' || u.username || u.password || u.hash) throw new Error('Release URLs must use HTTPS without credentials or fragments.');
  return u.href;
}

export function parseManifest(bytes, minimumVersion = null) {
  const m = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
  if (m.schema !== 1 || typeof m.latest !== 'string' || !Array.isArray(m.releases) || m.releases.length !== 1) throw new Error('Unsupported release manifest.');
  const r = m.releases[0];
  if (r.version !== m.latest || compareVersions(m.latest, m.latest) !== 0 || !Array.isArray(r.files) || r.files.length > 100) throw new Error('Invalid release contents.');
  if (minimumVersion && compareVersions(m.latest, minimumVersion) < 0) throw new Error('An older release list was rejected.');
  m.release = validateRelease(m.release);
  if (r.notes) httpsUrl(r.notes);
  const names = new Set();
  for (const f of r.files) {
    if (!f || typeof f.name !== 'string' || !/^[A-Za-z0-9][A-Za-z0-9._ -]{0,199}$/.test(f.name)
      || names.has(f.name) || !['macos', 'windows', 'linux', 'node'].includes(f.platform)
      || !['x86_64', 'aarch64', 'universal'].includes(f.arch) || ![undefined, 'setup'].includes(f.product)
      || !['dmg', 'msi', 'exe', 'AppImage', 'deb', 'rpm', 'tar.gz', 'app.tar.gz', 'AppImage.tar.gz'].includes(f.kind)
      || !Number.isSafeInteger(f.size) || f.size <= 0 || f.size > 1024 ** 3 || !/^[a-f0-9]{64}$/.test(f.sha256)) throw new Error('Invalid release artifact.');
    httpsUrl(f.url); names.add(f.name);
  }
  return m;
}

export async function fetchBounded(url, max, fetcher = fetch) {
  const res = await fetcher(httpsUrl(url), { credentials: 'omit', referrerPolicy: 'no-referrer', cache: 'no-store', redirect: 'error', signal: AbortSignal.timeout(15_000) });
  if (!res.ok || Number(res.headers.get('content-length')) > max) throw new Error('Release download unavailable or too large.');
  const reader = res.body?.getReader();
  if (!reader) throw new Error('Release response has no body.');
  const parts = []; let length = 0;
  try {
    while (true) {
      const { value, done } = await reader.read(); if (done) break;
      length += value.length;
      if (length > max) throw new Error('Release response is too large.');
      parts.push(value);
    }
  } finally { await reader.cancel(); }
  const bytes = new Uint8Array(length); let offset = 0;
  for (const part of parts) { bytes.set(part, offset); offset += part.length; }
  return bytes;
}

export async function fetchVerifiedManifest(config, minimumVersion = null, fetcher = fetch) {
  if (!config.downloads || !config.manifestKey) throw new Error('Updates are unavailable in this development build.');
  const url = `${config.downloads.replace(/\/+$/, '')}/manifest.json`;
  const [bytes, sig] = await Promise.all([fetchBounded(url, 1024 * 1024, fetcher), fetchBounded(`${url}.minisig`, 4096, fetcher)]);
  verifyMinisign(bytes, new TextDecoder().decode(sig), config.manifestKey);
  return parseManifest(bytes, minimumVersion);
}
