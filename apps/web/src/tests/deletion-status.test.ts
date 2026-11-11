import { describe, expect, it } from 'vitest';
import { batchLines, gapLines, readinessWords, summarize, within } from '../lib/deletion-status';
import { DAY, T, batch, community, progress, readiness, signed, target } from './fixtures/deletion';

const lines = (...args: Parameters<typeof summarize>) => batchLines(summarize(...args));

describe('deleting known posts and comments, in the words the app shows', () => {
  it('keeps saved work apart from signed and sent work', () => {
    const queued = batch([target(1), target(2, { state: 'queued', reason: 'rate-limited' }), target(3, { state: 'queued', reason: 'awaiting-signature' })]);
    expect(lines(queued, [])).toEqual(['3 deletions saved here, not signed yet: waiting for the hourly limit (1), your computer to sign (1).']);
    const s = summarize(queued, []);
    expect(s).toMatchObject({ saved: 3, signed: 0, submitted: 0 });

    const mixed = batch([signed(1), signed(2), target(3)]);
    const p = [progress(1, { recordSubmitted: true }), progress(2)];
    expect(lines(mixed, p)).toEqual([
      '1 deletion saved here, not signed yet.',
      '2 deletions signed. 1 sent to the community, 1 not sent yet.',
    ]);
    expect(summarize(mixed, p).needsDevice).toBe(true);
  });

  it('separates asked, acknowledged, unresolved and expired holder work, and never calls it erased', () => {
    const b = batch([signed(1), signed(2)]);
    const p = [
      progress(1, { recordSubmitted: true, submitted: 2, queued: 1, acknowledged: 3, outstanding: 3 }),
      progress(2, { recordSubmitted: true, refused: 1, unknown: 1, unreachable: 1, legacyRemoval: 1, expired: 2, coverageGaps: 1, outstanding: 4 }),
    ];
    const out = lines(b, p);
    expect(out).toContain('3 stored pieces still to drop: 2 asked, 1 queued.');
    expect(out).toContain('3 stored pieces not confirmed: the holder refused, didn\'t answer or can\'t confirm. This device asks again.');
    expect(out).toContain('1 older storage holder removed its piece but keeps no marker, so a copy could be stored there again.');
    expect(out).toContain('3 storage holders confirmed removing their pieces. Other copies aren\'t proven erased.');
    expect(out).toContain('2 stored pieces reached their end date. Storage validity ended; erasure wasn\'t observed.');
    expect(out).toContain('1 stored piece this device can\'t ask to drop.');
    expect(out.join(' ')).not.toMatch(/\b(all|every|everywhere|was erased|were erased)\b/i);
  });

  it('says propagation is unknown when a signed deletion has no progress here', () => {
    const out = lines(batch([signed(1), signed(2)]), [progress(1)]);
    expect(out).toContain('1 signed deletion with no progress here: how far it travelled is unknown.');
    expect(out[0]).toBe('2 deletions signed. 0 sent to the community, 1 not sent yet.');
  });

  it('names failures, lapsed posts and cancellations without counting them as deleted', () => {
    const b = batch([
      target(1, { state: 'failed', reason: 'not-allowed' }),
      target(2, { state: 'failed', reason: 'record-absent' }),
      target(3, { state: 'expired', nextTry: null }),
      target(4, { state: 'cancelled', nextTry: null }),
    ], { cancelledAt: T + 60 });
    expect(lines(b, [])).toEqual([
      'Not deleted: this device may no longer delete them (1), no longer on this device (1).',
      '1 post faded before it was signed. That isn\'t proof it was erased.',
      '1 deletion cancelled before signing.',
    ]);
  });

  it('reports an incomplete inventory per reason, summed', () => {
    expect(gapLines([
      { reason: 'capability-missing', community, target: 'a'.repeat(64), count: 2 },
      { reason: 'capability-missing', community, target: 'b'.repeat(64), count: 1 },
      { reason: 'private-state-unavailable', community, target: null, count: 0 },
      { reason: 'not-synchronized', community, target: null, count: 4 },
      { reason: 'record-absent', community, target: 'c'.repeat(64), count: 1 },
      { reason: 'too-many', community: null, target: null, count: 12 },
    ])).toEqual([
      '3 stored copies this device holds no drop authority for.',
      'A private community this device can\'t use: posts may be missing, and deletions there can\'t be sent.',
      '4 posts in a community this device hasn\'t caught up with: posts may be missing.',
      '1 post known only by a pointer: the post itself isn\'t here to delete.',
      '12 more posts than one list holds: review again after this one.',
    ]);
    expect(gapLines([])).toEqual([]);
  });
});

describe('clearing local data after deleting', () => {
  it('offers the combined clear only when the core says ready', () => {
    expect(readinessWords(readiness(), T, 'browser')).toEqual({ ready: true, lines: [] });
    const blocked = readinessWords(readiness({
      ready: false,
      blocked: [{ reason: 'unsigned', count: 2 }, { reason: 'in-effect', count: 1 }, { reason: 'holder-drops', count: 3 }],
      until: T + 12 * DAY,
    }), T, 'browser');
    expect(blocked.ready).toBe(false);
    expect(blocked.lines).toEqual([
      '2 deletions of yours aren\'t signed yet.',
      '1 signed deletion of yours still needs this browser: it keeps sharing it and asks storage holders to drop pieces.',
      '3 deletions by other people still need this browser to drop pieces it stored.',
      'If nothing new comes up, that work ends in 12 days.',
    ]);
  });

  it('never offers it when this core cannot check, and says a local clear still works', () => {
    const words = readinessWords(null, T, 'computer');
    expect(words.ready).toBe(false);
    expect(words.lines[0]).toMatch(/couldn't check its deletion work.*Clearing local data still works, and stops any deletion here\./);
  });

  it('says what a clear leaves on the network', () => {
    expect(readinessWords(readiness({ unlisted: 3 }), T, 'phone').lines).toEqual(['3 known posts and comments of yours are in no deletion. Clearing leaves them on the network until they fade.']);
    expect(readinessWords(readiness({ unlisted: 1 }), T, 'phone').lines[0]).toContain('leaves it on the network until it fades');
  });

  it('rounds when work ends', () => {
    expect(within(T + 30, T)).toBe('within an hour');
    expect(within(T + 5 * 3600, T)).toBe('in 5 hours');
    expect(within(T + 3 * DAY, T)).toBe('in 3 days');
  });
});
