// Small pure helpers for the UI. Display rules for keys, ids and time live in @hoi/core-api/format.
import type { CommunityColor, Did, Hex, Unix, Admission, Limits, Napping } from '@hoi/core-api';
import { now, duration } from '@hoi/core-api';

// ---------------------------------------------------------------- names and avatars

/** Two initials for an avatar: 'jonas.w' → 'JW', 'mira' → 'MI'. */
export function initials(name: string): string {
  const parts = name.trim().split(/[^\p{L}\p{N}]+/u).filter(Boolean);
  if (parts.length === 0) return '··';
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[1][0]).toUpperCase();
}

const AVATAR_COLORS: CommunityColor[] = ['mint', 'sky', 'peach', 'lilac'];

/** A stable pastel for someone's avatar, from their key. Yours is always lilac. */
export function avatarColor(key: string, mine = false): CommunityColor {
  if (mine) return 'lilac';
  let h = 0;
  for (let i = 0; i < key.length; i++) h = (h * 31 + key.charCodeAt(i)) >>> 0;
  return AVATAR_COLORS[h % 3]; // mint, sky, peach; lilac stays yours
}

/** Strip a leading 'c/', trim, collapse spaces to dashes, lowercase. */
export function normalizeCommunityName(input: string): string {
  return input
    .trim()
    .replace(/^c\//i, '')
    .replace(/\s+/g, '-')
    .toLowerCase()
    .slice(0, 32);
}

// ---------------------------------------------------------------- text

/** Split post text into paragraphs (blank-line separated). Rendered as text, never HTML. */
export function paragraphs(text: string): string[] {
  return text
    .replace(/\r\n?/g, '\n')
    .split(/\n\s*\n/)
    .map((p) => p.trim())
    .filter(Boolean);
}

/** Who put something to sleep, as the start of a sentence. */
export function napWho(n: Napping): string {
  if (n.label.startsWith('removal pending')) return `${n.by === 'the mods' ? 'The mods' : n.by} removed it, and members can still veto that`;
  switch (n.by) {
    case 'the mods':
      return 'The mods labelled it';
    case 'the community':
      return 'The community put it to sleep';
    case 'you':
      return 'You hid it';
    default:
      return `${n.by} (a labeler you follow) labelled it`;
  }
}

export function plural(n: number, one: string, many = one + 's'): string {
  return `${n.toLocaleString('en')} ${n === 1 ? one : many}`;
}

/** Numbered rules ("1. Be kind") → picker options for "Flag for the mods". */
export function ruleOptions(text: string | undefined | null): { n: number; label: string }[] {
  if (!text) return [];
  const out: { n: number; label: string }[] = [];
  for (const line of text.split('\n')) {
    const m = line.match(/^\s*(\d{1,2})[.)]\s+(.+)$/);
    if (m) out.push({ n: Number(m[1]), label: m[2].trim() });
  }
  return out;
}

/** 'spam' → 'spam', 'rule:4' → 'rule 4', 'illegal' → 'illegal' */
export function reasonLabel(reason: string): string {
  const m = reason.match(/^rule:(\d+)$/);
  return m ? `rule ${m[1]}` : reason;
}

/** Only http(s) links become clickable; everything else is shown as text. */
export function safeHttpUrl(link: string | undefined): URL | null {
  if (!link) return null;
  try {
    const u = new URL(link);
    return u.protocol === 'https:' || u.protocol === 'http:' ? u : null;
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------- time

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/** 'Jan 2026' */
export function monthYear(t: Unix): string {
  const d = new Date(t * 1000);
  return `${MONTHS[d.getMonth()]} ${d.getFullYear()}`;
}

/** 'today 11:02', 'yesterday 18:40', '12 Oct' , '12 Oct 2025' */
export function clock(t: Unix, at: Unix = now()): string {
  const d = new Date(t * 1000);
  const ref = new Date(at * 1000);
  const hm = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  const sameDay = (a: Date, b: Date) =>
    a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
  if (sameDay(d, ref)) return `today ${hm}`;
  const y = new Date(ref);
  y.setDate(ref.getDate() - 1);
  if (sameDay(d, y)) return `yesterday ${hm}`;
  const base = `${d.getDate()} ${MONTHS[d.getMonth()]}`;
  return d.getFullYear() === ref.getFullYear() ? base : `${base} ${d.getFullYear()}`;
}

/** Key age for people: '3 days', '5 months', '2 years' */
export function keyAge(days: number): string {
  if (days < 1) return 'new today';
  if (days < 60) return plural(Math.round(days), 'day');
  if (days < 730) return plural(Math.round(days / 30.4), 'month');
  return plural(Math.round(days / 365), 'year');
}

export const DAY = 86_400;

/** Retention presets offered in the limits editor and when starting a community. */
export const RETENTION_PRESETS: { seconds: number; label: string }[] = [
  { seconds: 1 * DAY, label: '1 d' },
  { seconds: 7 * DAY, label: '7 d' },
  { seconds: 30 * DAY, label: '30 d' },
  { seconds: 90 * DAY, label: '90 d' },
  { seconds: 180 * DAY, label: '180 d' },
  { seconds: 365 * DAY, label: '365 d' },
];

export const NEWCOMER_AGE_PRESETS: { seconds: number; label: string }[] = [
  { seconds: 0, label: 'none' },
  { seconds: 7 * DAY, label: '7 d' },
  { seconds: 30 * DAY, label: '30 d' },
  { seconds: 90 * DAY, label: '90 d' },
];

/** '0.9 s', '12 ms', '1 min 4 s' */
export function formatMs(ms: number): string {
  if (ms < 1000) return `${Math.max(1, Math.round(ms))} ms`;
  const s = ms / 1000;
  if (s < 10) return `${s.toFixed(1)} s`;
  if (s < 60) return `${Math.round(s)} s`;
  const m = Math.floor(s / 60);
  return `${m} min ${Math.round(s - m * 60)} s`;
}

/** Plain words for how a newcomer gets in. */
export function admissionLabel(a: Admission): string {
  switch (a) {
    case 'open': return 'Open · stamp only';
    case 'approve': return 'A mod says yes';
    case 'invite': return 'Invite: a member vouches first';
    case 'age-proof': return 'Age proof (anonymous)';
  }
}

/** A voice (0–1) in words: "a full voice", "half a voice", "0.33 of a voice". */
export function voiceText(v: number): string {
  if (v >= 0.995) return 'a full voice';
  if (v <= 0) return 'no voice yet';
  if (Math.abs(v - 0.5) < 0.005) return 'half a voice';
  return `${v.toFixed(2).replace(/0$/, '')} of a voice`;
}

/** Voices for tallies: "2", "1.5", "0.33". */
export function voices(v: number): string {
  return Number.isInteger(Math.round(v * 100) / 100) ? String(Math.round(v)) : (Math.round(v * 100) / 100).toString();
}

const FIELD_LABELS: Record<string, string> = {
  pow_bits: 'Stamp',
  max_retention: 'Posts fade after',
  newcomer_posts_per_day: 'Newcomer posts a day',
  newcomer_age: 'Newcomer age',
  admission: 'Joining',
  autoban_threshold: 'Auto-bounce after',
  labelers: 'Labelers',
  identity_bits: 'Key work',
  admit_vouches: 'Vouches to get in',
  established_age: 'Established after',
  trusted_posts: 'Posts for media',
  media_score: 'Response for media',
  hide_quorum: 'Voices to nap a post',
  block_quorum: 'Voices to block',
  veto_quorum: 'Voices to overturn',
  contest_window: 'Removals wait',
  trackers: 'Trackers',
};
const DURATIONS = new Set(['max_retention', 'newcomer_age', 'established_age', 'contest_window']);

/** One field a proposal changes, in words: { label: 'Posts fade after', from: '90 d', to: '60 d' }. */
export function changeText(field: string, from: string, to: string): { label: string; from: string; to: string } {
  const fmt = (v: string) => {
    if (DURATIONS.has(field)) return Number(v) === 0 ? 'no wait' : duration(Number(v));
    if (field === 'admission') return admissionLabel(v as Admission).toLowerCase();
    if (field === 'pow_bits' || field === 'identity_bits') return `${v} bits`;
    return v;
  };
  return { label: FIELD_LABELS[field] ?? field.replace(/_/g, ' '), from: fmt(from), to: fmt(to) };
}

export function retentionLabel(seconds: number): string {
  return duration(seconds);
}

export function limitsEqual(a: Limits, b: Limits): boolean {
  return (
    a.powBits === b.powBits &&
    a.maxRetention === b.maxRetention &&
    a.newcomerPostsPerDay === b.newcomerPostsPerDay &&
    a.newcomerAge === b.newcomerAge &&
    a.admission === b.admission &&
    a.autobanThreshold === b.autobanThreshold &&
    a.labelers.join() === b.labelers.join() &&
    (a.trackers ?? []).join() === (b.trackers ?? []).join() &&
    (Object.keys(a.ladder) as (keyof Limits['ladder'])[]).every((k) => a.ladder[k] === b.ladder[k])
  );
}

/** A copy deep enough to edit in a form without touching the original. */
export function copyLimits(l: Limits): Limits {
  return { ...l, labelers: [...l.labelers], trackers: [...(l.trackers ?? [])], ladder: { ...l.ladder } };
}

// ---------------------------------------------------------------- keys

const B58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';

export function base58Encode(bytes: Uint8Array): string {
  const digits: number[] = [0];
  for (const byte of bytes) {
    let carry = byte;
    for (let i = 0; i < digits.length; i++) {
      carry += digits[i] << 8;
      digits[i] = carry % 58;
      carry = (carry / 58) | 0;
    }
    while (carry > 0) {
      digits.push(carry % 58);
      carry = (carry / 58) | 0;
    }
  }
  let out = '';
  for (let k = 0; k < bytes.length - 1 && bytes[k] === 0; k++) out += '1';
  for (let i = digits.length - 1; i >= 0; i--) out += B58[digits[i]];
  return out;
}

export function base58Decode(s: string): Uint8Array | null {
  const bytes: number[] = [0];
  for (const ch of s) {
    const v = B58.indexOf(ch);
    if (v < 0) return null;
    let carry = v;
    for (let i = 0; i < bytes.length; i++) {
      carry += bytes[i] * 58;
      bytes[i] = carry & 0xff;
      carry >>= 8;
    }
    while (carry) {
      bytes.push(carry & 0xff);
      carry >>= 8;
    }
  }
  let zeros = 0;
  for (const ch of s) {
    if (ch !== '1') break;
    zeros++;
  }
  const body = bytes.reverse();
  while (body.length > 0 && body[0] === 0) body.shift();
  return new Uint8Array([...new Array(zeros).fill(0), ...body]);
}

export function hexToBytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

export function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
}

export const isHexKey = (s: string): boolean => /^[0-9a-f]{64}$/i.test(s);

/** did:key for an Ed25519 public key (multicodec 0xed 0x01, base58btc 'z'). */
export function didFromKey(hex: Hex): Did {
  return 'did:key:z' + base58Encode(new Uint8Array([0xed, 0x01, ...hexToBytes(hex)]));
}

/** Accepts a 64-char hex key or a did:key:z6Mk… and returns the hex key, or null. */
export function parseKey(input: string): Hex | null {
  const s = input.trim();
  if (isHexKey(s)) return s.toLowerCase();
  const m = s.match(/^did:key:z([1-9A-HJ-NP-Za-km-z]+)$/);
  if (!m) return null;
  const bytes = base58Decode(m[1]);
  if (!bytes || bytes.length !== 34 || bytes[0] !== 0xed || bytes[1] !== 0x01) return null;
  return bytesToHex(bytes.slice(2));
}

/** 'did:key:z6Mk…a7f2' for compact display. */
export function shortDid(did: Did): string {
  return did.length > 22 ? `${did.slice(0, 13)}…${did.slice(-4)}` : did;
}

// ---------------------------------------------------------------- links

/**
 * Pull the invite query out of anything a person might paste:
 * hoi://join, web hash links, or bare queries. Both encoded `code=` and legacy `c=` invites.
 * Returns the query without the leading '?', or null.
 */
export function inviteQuery(link: string): string | null {
  const s = link.trim();
  if (!s) return null;
  let q: string | null = null;
  const hoi = s.match(/^(?:web\+)?hoi:\/\/join\/?\?(.+)$/i);
  if (hoi) q = hoi[1];
  else {
    const hash = s.match(/#\/join\?(.+)$/);
    if (hash) q = hash[1];
    else if (/^\??(?:code|c)=/.test(s)) q = s.replace(/^\?/, '');
  }
  if (!q) return null;
  const params = new URLSearchParams(q);
  return params.get('code') || params.get('c') ? q : null;
}

/** The invite string handed to previewInvite/join. */
export const joinLink = (query: string): string => `hoi://join?${query}`;

// ---------------------------------------------------------------- browser bits

/**
 * Hand a file to the person: identity exports, community bundles, signed logs.
 * In a browser this is a download; the desktop shell swaps in its native save dialog here.
 */
export async function saveFile(name: string, bytes: Uint8Array): Promise<void> {
  if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
    // WebViews don't do downloads; the desktop shell opens a native save dialog.
    const [{ invoke }, { toBase64Url }] = await Promise.all([import('@tauri-apps/api/core'), import('./core/rpc-core')]);
    const saved = await invoke<boolean>('hoi_save_file', { name, data: toBase64Url(bytes) });
    if (!saved) throw new Error('Save cancelled. No file was saved. Try again when you are ready.');
    return;
  }
  const blob = new Blob([bytes as BlobPart], { type: 'application/octet-stream' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.rel = 'noopener';
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 2000);
}

/** A .hoikey is a few kilobytes (keys, a tiny picture, some geneses); anything bigger isn't one. */
export const MAX_KEY_FILE = 1024 * 1024;
/** A community bundle: records and their media, well under this. */
export const MAX_BUNDLE_FILE = 64 * 1024 * 1024;

export async function readFile(file: File): Promise<Uint8Array> {
  return new Uint8Array(await file.arrayBuffer());
}

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.setAttribute('readonly', '');
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.select();
    let ok = false;
    try {
      ok = document.execCommand('copy');
    } catch {
      ok = false;
    }
    ta.remove();
    return ok;
  }
}

/** File-name-safe slug. */
export function slug(s: string): string {
  return s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'hoi';
}

/** did:key for a hex key; anything else is returned as is. */
export function didFromKeySafe(key: string): string {
  return isHexKey(key) ? didFromKey(key.toLowerCase()) : key;
}
