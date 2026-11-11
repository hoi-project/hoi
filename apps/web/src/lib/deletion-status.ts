// "Delete known posts and comments" (RFC-0002) and clearing local data (RFC-0003), in the words
// the app shows. Every line says what this device saw. None says a post is gone everywhere:
// a holder's answer covers that holder, an end date ends storage validity, not the bytes.
import type { BulkDeletionGap, BulkDeletionReason, BulkDeletionStatus, CleanupReadiness, DeletionProgress, Unix } from '@hoi/core-api';
import { plural } from './util';

/** Where a batch's deletions are, counted from the core's own status. */
export interface BatchSummary {
  total: number;
  /** Saved here, not signed yet (selected or queued). */
  saved: number;
  /** Why the saved ones wait. */
  waiting: Partial<Record<BulkDeletionReason, number>>;
  /** Signed deletions, sent to the community or not yet. */
  signed: number;
  submitted: number;
  /** Signed, with no progress here to say how far it went. */
  untracked: number;
  failed: Partial<Record<BulkDeletionReason, number>>;
  /** The post's own lifetime ended before it was signed. */
  lapsed: number;
  cancelled: number;
  /** Storage holders, over the signed deletions this device follows. */
  holders: { queued: number; sent: number; acknowledged: number; legacy: number; unresolved: number; expired: number; gaps: number };
  /** Some signed deletion still needs this device. */
  needsDevice: boolean;
}

export function summarize(batch: BulkDeletionStatus, progress: DeletionProgress[]): BatchSummary {
  const byId = new Map(progress.map((p) => [p.id, p]));
  const s: BatchSummary = {
    total: batch.count, saved: 0, waiting: {}, signed: 0, submitted: 0, untracked: 0, failed: {}, lapsed: 0, cancelled: 0,
    holders: { queued: 0, sent: 0, acknowledged: 0, legacy: 0, unresolved: 0, expired: 0, gaps: 0 }, needsDevice: false,
  };
  const add = (into: Partial<Record<BulkDeletionReason, number>>, why: BulkDeletionReason | null) => {
    const key = why ?? 'refused';
    into[key] = (into[key] ?? 0) + 1;
  };
  for (const t of batch.targets) {
    if (t.state === 'selected' || t.state === 'queued') {
      s.saved++;
      if (t.reason) add(s.waiting, t.reason);
    } else if (t.state === 'failed') add(s.failed, t.reason);
    else if (t.state === 'expired') s.lapsed++;
    else if (t.state === 'cancelled') s.cancelled++;
    else {
      s.signed++;
      const p = t.deletion ? byId.get(t.deletion) : undefined;
      if (!p) { s.untracked++; continue; }
      if (p.recordSubmitted) s.submitted++;
      s.needsDevice ||= p.requiresLocalState;
      s.holders.queued += p.queued;
      s.holders.sent += p.submitted;
      s.holders.acknowledged += p.acknowledged;
      s.holders.legacy += p.legacyRemoval;
      s.holders.unresolved += p.unknown + p.refused + p.unreachable;
      s.holders.expired += p.expired;
      s.holders.gaps += p.coverageGaps;
    }
  }
  return s;
}

const WAITING: Record<BulkDeletionReason, string> = {
  'rate-limited': 'the hourly limit',
  'awaiting-signature': 'your computer to sign',
  'deletion-full': 'room for more deletion work',
  'placement-pending': 'an upload to finish',
  'record-absent': 'the post to arrive',
  'not-allowed': 'permission',
  refused: 'another try',
};
const FAILED: Record<BulkDeletionReason, string> = {
  'rate-limited': 'over the limit',
  'awaiting-signature': 'not signed by your computer',
  'deletion-full': 'no room for deletion work',
  'placement-pending': 'an upload never finished',
  'record-absent': 'no longer on this device',
  'not-allowed': 'this device may no longer delete them',
  refused: 'refused',
};

function reasons(map: Partial<Record<BulkDeletionReason, number>>, words: Record<BulkDeletionReason, string>): string {
  return Object.entries(map).map(([why, n]) => `${words[why as BulkDeletionReason]} (${n})`).join(', ');
}

/** The lines a batch shows, most pressing first. */
export function batchLines(s: BatchSummary): string[] {
  const out: string[] = [];
  if (s.saved) {
    const why = reasons(s.waiting, WAITING);
    out.push(`${plural(s.saved, 'deletion')} saved here, not signed yet${why ? `: waiting for ${why}` : ''}.`);
  }
  if (s.signed) {
    const unsent = s.signed - s.submitted - s.untracked;
    out.push(`${plural(s.signed, 'deletion')} signed. ${s.submitted.toLocaleString('en')} sent to the community${unsent ? `, ${unsent.toLocaleString('en')} not sent yet` : ''}.`);
  }
  if (s.untracked) out.push(`${plural(s.untracked, 'signed deletion')} with no progress here: how far ${s.untracked === 1 ? 'it' : 'they'} travelled is unknown.`);
  const h = s.holders;
  const pending = h.queued + h.sent;
  if (pending) out.push(`${plural(pending, 'stored piece')} still to drop: ${h.sent.toLocaleString('en')} asked, ${h.queued.toLocaleString('en')} queued.`);
  if (h.unresolved) out.push(`${plural(h.unresolved, 'stored piece')} not confirmed: the holder refused, didn't answer or can't confirm. This device asks again.`);
  if (h.legacy) out.push(`${plural(h.legacy, 'older storage holder')} removed ${h.legacy === 1 ? 'its piece' : 'their pieces'} but ${h.legacy === 1 ? 'keeps' : 'keep'} no marker, so a copy could be stored there again.`);
  if (h.acknowledged) out.push(`${plural(h.acknowledged, 'storage holder')} confirmed removing ${h.acknowledged === 1 ? 'its piece' : 'their pieces'}. Other copies aren't proven erased.`);
  if (h.expired) out.push(`${plural(h.expired, 'stored piece')} reached ${h.expired === 1 ? 'its' : 'their'} end date. Storage validity ended; erasure wasn't observed.`);
  if (h.gaps) out.push(`${plural(h.gaps, 'stored piece')} this device can't ask to drop.`);
  if (Object.keys(s.failed).length) out.push(`Not deleted: ${reasons(s.failed, FAILED)}.`);
  if (s.lapsed) out.push(`${plural(s.lapsed, 'post')} faded before ${s.lapsed === 1 ? 'it was' : 'they were'} signed. That isn't proof ${s.lapsed === 1 ? 'it was' : 'they were'} erased.`);
  if (s.cancelled) out.push(`${plural(s.cancelled, 'deletion')} cancelled before signing.`);
  return out;
}

const GAPS: Record<BulkDeletionGap['reason'], (n: number) => string> = {
  'private-state-unavailable': (n) => `${n ? `${plural(n, 'post')} in a` : 'A'} private community this device can't use: posts may be missing, and deletions there can't be sent.`,
  'not-synchronized': (n) => `${n ? `${plural(n, 'post')} in a` : 'A'} community this device hasn't caught up with: posts may be missing.`,
  'record-absent': (n) => `${plural(n, 'post')} known only by a pointer: the post itself isn't here to delete.`,
  'capability-missing': (n) => `${plural(n, 'stored copy', 'stored copies')} this device holds no drop authority for.`,
  'generations-unknown': (n) => `${plural(n, 'post')} kept in pieces this device doesn't know.`,
  'too-many': (n) => `${plural(n, 'more post')} than one list holds: review again after this one.`,
};

/** What a preview or batch cannot account for, summed per reason. Never counted as deleted. */
export function gapLines(gaps: BulkDeletionGap[]): string[] {
  const sums = new Map<BulkDeletionGap['reason'], number>();
  for (const g of gaps) sums.set(g.reason, (sums.get(g.reason) ?? 0) + g.count);
  return [...sums].map(([reason, n]) => GAPS[reason](n));
}

/** 'in 12 days', 'in 5 hours', 'within an hour' */
export function within(until: Unix, at: Unix): string {
  const s = until - at;
  if (s <= 3600) return 'within an hour';
  if (s < 86400 * 2) return `in ${plural(Math.round(s / 3600), 'hour')}`;
  return `in ${plural(Math.round(s / 86400), 'day')}`;
}

export interface ReadinessWords {
  /** The core said ready: the combined clear may be offered, with a fresh confirmation. */
  ready: boolean;
  lines: string[];
}

/** Whether clearing now would strand deletion work. `null`: this core couldn't say. */
export function readinessWords(r: CleanupReadiness | null, at: Unix, device: string): ReadinessWords {
  if (!r) return { ready: false, lines: [`This ${device} couldn't check its deletion work, so it can't clear only once that work is done. Clearing local data still works, and stops any deletion here.`] };
  const lines: string[] = [];
  for (const b of r.blocked) {
    if (b.reason === 'unsigned') lines.push(`${plural(b.count, 'deletion')} of yours ${b.count === 1 ? "isn't" : "aren't"} signed yet.`);
    else if (b.reason === 'in-effect') lines.push(`${plural(b.count, 'signed deletion')} of yours still ${b.count === 1 ? 'needs' : 'need'} this ${device}: it keeps sharing ${b.count === 1 ? 'it' : 'them'} and asks storage holders to drop pieces.`);
    else lines.push(`${plural(b.count, 'deletion')} by other people still ${b.count === 1 ? 'needs' : 'need'} this ${device} to drop pieces it stored.`);
  }
  if (!r.ready && r.until) lines.push(`If nothing new comes up, that work ends ${within(r.until, at)}.`);
  if (r.unlisted) lines.push(`${plural(r.unlisted, 'known post or comment', 'known posts and comments')} of yours ${r.unlisted === 1 ? 'is' : 'are'} in no deletion. Clearing leaves ${r.unlisted === 1 ? 'it' : 'them'} on the network until ${r.unlisted === 1 ? 'it fades' : 'they fade'}.`);
  return { ready: r.ready, lines };
}
