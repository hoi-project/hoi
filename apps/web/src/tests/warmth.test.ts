import { describe, expect, it } from 'vitest';
import type { CommunityWarmth } from '@hoi/core-api';
import type { LinkWarmth } from '@hoi/core-api';
import { directoryWarmth, linkWarmthWords, othersDurable, warmthWords, WARMTH_TAGS } from '../lib/warmth';

const base: CommunityWarmth = {
  community: 'a'.repeat(64), name: 'garden', level: 'cold', score: 12, kept: 0, places: 0, spread: 0, guarded: 0, copies: 1, durableCopies: 0, networkCopies: 0,
  complete: 1, media: 1, fresh: false, live: false, records: 40, missing: 0, holdersDurable: 0, holdersTransient: 0,
  remembered: 0, ownDurable: false, pending: false,
};

/** Every node in a place of its own, every link checked, unless the case says otherwise. */
function fill(w: CommunityWarmth & Partial<Pick<CommunityWarmth, 'kept' | 'places' | 'spread' | 'guarded'>>): CommunityWarmth {
  const others = othersDurable(w);
  const given = (k: 'kept' | 'places' | 'spread' | 'guarded') => w[k] !== base[k];
  return {
    ...w,
    kept: given('kept') ? w.kept : others,
    places: given('places') ? w.places : others > 0 ? w.holdersDurable : 0,
    spread: given('spread') ? w.spread : others > 0 ? 1 : 0,
    guarded: given('guarded') ? w.guarded : others > 0 ? 1 : 0,
  };
}

/** Never a receipt or a guarantee, and none of the words CONTRIBUTING.md bans. */
const FORBIDDEN = /\b(platform|content|users?|accounts?|delivered|guarantee[ds]?|is safe)\b/i;
const everyString = (words: ReturnType<typeof warmthWords>) => [words.summary, ...words.reasons, words.advice ?? ''];

describe('warmth in words', () => {
  it('reads a copy alone in a browser as cold with advice', () => {
    const w = warmthWords(base, { platformName: 'browser' });
    expect(w.word).toBe('cold');
    expect(w.tag).toBe('tag-peach');
    expect(w.summary).toBe('No node is currently known to have a copy. All known records are saved in this browser.');
    expect(w.advice).toContain('Ask a public node');
    expect(w.reasons).toContain('This device has not finished syncing with another device yet.');
  });

  it('names durable holders, incomplete windows, missing pictures and stale catch-ups', () => {
    const w = warmthWords(fill({ ...base, level: 'warm', score: 60, copies: 3, durableCopies: 2, networkCopies: 2, holdersDurable: 2, holdersTransient: 1, complete: 0.7, missing: 12, media: 0.5, fresh: false, lastSync: Math.floor(Date.now() / 1000) - 10 * 86_400 }), { platformName: 'browser' });
    expect(w.tag).toBe('tag-mint');
    expect(w.summary).toBe('2 other nodes have a copy. This browser has 70% of the known records.');
    expect(w.reasons).toHaveLength(3);
    expect(w.reasons[0]).toBe('12 records still to download.');
    expect(w.reasons[1]).toBe('50% of the pictures referenced by saved posts are downloaded.');
    expect(w.reasons[2]).toContain('No device sharing this community is connected. Last sync:');
  });

  it('shows a connected node’s hint and announced public nodes as hints, not counts', () => {
    const w = warmthWords(fill({ ...base, level: 'cool', score: 60, copies: 2, durableCopies: 1, networkCopies: 3, holdersDurable: 1, fresh: true, live: true }), { announced: 2 });
    expect(w.word).toBe('cool');
    expect(w.summary).toBe('1 other node has a copy. All known records are saved in this browser.');
    expect(w.reasons).toContain('A connected node reports 3 copies on other nodes. These are not included in the warmth score.');
    expect(w.reasons).toContain('The registry lists 2 public nodes for this community; they may be offline.');
    expect(w.advice).toContain('Another node with a full copy');
  });

  it('never lets this computer’s own copy warm its own reading, and says when it is kept for others', () => {
    const w = warmthWords(fill({ ...base, level: 'cold', score: 20, copies: 1, durableCopies: 1, ownDurable: true, fresh: true, live: true, carried: 'request' }), { platformName: 'computer' });
    expect(w.word).toBe('cold');
    expect(w.summary).toBe('No other node is currently known to have a copy. All known records are saved on this computer.');
    expect(w.reasons).toContain('This device also shares the community at someone’s request.');
    expect(w.advice).toContain('Invite someone who runs a node');
    const cool = warmthWords(fill({ ...base, level: 'cool', score: 40, copies: 2, durableCopies: 2, holdersDurable: 1, ownDurable: true, fresh: true, live: true }), { platformName: 'computer' });
    expect(cool.summary).toBe('1 other node has a copy. All known records are saved on this computer.');
    const hot = warmthWords(fill({ ...base, level: 'hot', score: 80, copies: 4.5, durableCopies: 3.5, networkCopies: 3.5, holdersDurable: 4, fresh: true, live: true, remembered: 3 }));
    expect(hot.tag).toBe('tag-trust');
    expect(hot.summary).toBe('4 other nodes hold parts of this community (3.5 copies in total). All known records are saved in this browser.');
    expect(hot.advice).toBeUndefined();
    expect(hot.reasons).toEqual([]);
  });

  it('waits plainly for a pending public join', () => {
    const w = warmthWords(fill({ ...base, pending: true, records: 0, missing: 12, complete: 0 }));
    expect(w.summary).toBe('Waiting to download this community.');
    expect(w.advice).toContain('public node');
  });

  it('does not describe partial copies or browser copies as absent or lost on tab close', () => {
    const partial = warmthWords(fill({ ...base, durableCopies: 0.5, holdersDurable: 2, fresh: true }));
    expect(partial.summary).toContain('2 other nodes hold parts of this community (0.5 copies in total).');
    expect(partial.advice).toBeDefined();
    const transient = warmthWords(fill({ ...base, holdersTransient: 1, fresh: true }));
    expect(transient.summary).not.toContain('only copy');
    expect(transient.reasons).toEqual(['1 browser or phone also has records, but can only share while hoi is open.']);
  });

  it('does not suggest a second node when missing records lowered the level despite two full copies', () => {
    const w = warmthWords(fill({ ...base, level: 'cool', durableCopies: 2, holdersDurable: 2, complete: 0.4, missing: 60 }));
    expect(w.advice).toBeUndefined();
    expect(w.reasons).toContain('60 records still to download.');
  });

  it('never claims delivery or uses banned words', () => {
    const cases: CommunityWarmth[] = [
      base,
      { ...base, pending: true },
      { ...base, level: 'cool', durableCopies: 1, holdersDurable: 1, fresh: true, live: true },
      { ...base, level: 'warm', durableCopies: 2.5, holdersDurable: 3, holdersTransient: 2, complete: 0.4, missing: 9, media: 0.2, networkCopies: 3, carried: 'index', remembered: 2 },
      { ...base, level: 'hot', durableCopies: 3, holdersDurable: 3, fresh: true, live: true, ownDurable: true },
    ];
    cases.push({ ...base, level: 'cool', durableCopies: 3, holdersDurable: 3, kept: 1, places: 1, spread: 0.33, guarded: 0.33, fresh: true, live: true });
    for (const c of cases.map(fill)) for (const platformName of ['browser', 'computer', 'phone']) {
      for (const s of everyString(warmthWords(c, { platformName, announced: 1 }))) expect(s).not.toMatch(FORBIDDEN);
    }
    for (const level of ['cold', 'cool', 'warm', 'hot'] as const) expect(WARMTH_TAGS[level]).toMatch(/^tag-/);
  });
});

describe('directory warmth', () => {
  const facts = { registryConfigured: true, registryReachable: true, announcedNodes: 0, catalogNodes: 0, discoverable: 0, carryNodes: 0 };
  it('is cold with no source, cool with one, warm once a connected node joins a registry or another node, hot with a registry, two announced nodes and a catalog', () => {
    expect(directoryWarmth({ ...facts, registryConfigured: false }).level).toBe('cold');
    expect(directoryWarmth({ ...facts, registryReachable: false }).level).toBe('cold');
    expect(directoryWarmth(facts).level).toBe('cool');
    expect(directoryWarmth({ ...facts, registryConfigured: false, catalogNodes: 1, discoverable: 3 }).level).toBe('cool');
    // A registry's list is a lead, not a connection: listings alone never read warmer than cool.
    expect(directoryWarmth({ ...facts, announcedNodes: 5 }).level).toBe('cool');
    expect(directoryWarmth({ ...facts, catalogNodes: 1 }).level).toBe('warm');
    expect(directoryWarmth({ ...facts, registryReachable: false, catalogNodes: 2 }).level).toBe('warm');
    expect(directoryWarmth({ ...facts, announcedNodes: 2, catalogNodes: 1 }).level).toBe('hot');
  });
  it('says where names come from in plain words', () => {
    expect(directoryWarmth({ ...facts, registryConfigured: false }).summary).toBe('No registry in this build; no connected node shares a directory.');
    expect(directoryWarmth({ ...facts, announcedNodes: 2, catalogNodes: 1, discoverable: 4, carryNodes: 1 }).summary)
      .toBe('The registry answers and lists 2 public nodes; 1 connected node shares 4 open communities; 1 connected node takes requests to carry communities.');
    expect(directoryWarmth({ ...facts, registryReachable: false, catalogNodes: 2, discoverable: 1 }).summary)
      .toBe('The registry is not reachable right now; 2 connected nodes share 1 open community.');
    for (const s of [directoryWarmth(facts).summary, directoryWarmth({ ...facts, registryConfigured: false }).summary]) expect(s).not.toMatch(FORBIDDEN);
  });
});

describe('places and checked links', () => {
  it('says when nodes share a place, and that another place helps more than another node there', () => {
    const rack = warmthWords(fill({ ...base, level: 'cool', copies: 4, durableCopies: 3, kept: 1, places: 1, spread: 0.33, holdersDurable: 3, fresh: true, live: true }));
    expect(rack.summary).toBe('3 other nodes have a copy. They are in 1 independent place. All known records are saved in this browser.');
    expect(rack.reasons[0]).toBe('Some of these nodes share a network or an operator, so they count once: one outage or one person could take them together.');
    expect(rack.advice).toBe('A copy on a node somewhere else, run by someone else, would help more than another node in the same place.');
    const spread = warmthWords(fill({ ...base, level: 'hot', copies: 4, durableCopies: 3, holdersDurable: 3, fresh: true, live: true }));
    expect(spread.summary).toBe('3 other nodes have a copy. All known records are saved in this browser.');
    expect(spread.reasons).toEqual([]);
  });

  it('says how much reaches here over links that were not checked', () => {
    const w = warmthWords(fill({ ...base, level: 'warm', copies: 4, durableCopies: 3, holdersDurable: 3, guarded: 0.33, fresh: true, live: true }));
    expect(w.reasons).toContain('67% of these copies are reached over links that are not encrypted or whose other end was not checked, so they count for less.');
  });

  it('reads this device’s connections by place and checked links, without technical terms', () => {
    const l = (p: Partial<LinkWarmth>): LinkWarmth => ({ level: 'warm', score: 60, links: 2, places: 2, effective: 2, largest: 0.5, guarded: 1, ...p });
    expect(linkWarmthWords(l({ level: 'cold', links: 0, places: 0 })).summary).toBe('This browser is not connected to anyone right now.');
    const one = linkWarmthWords(l({ level: 'cool', links: 3, places: 1, effective: 1, largest: 1 }), 'computer');
    expect(one.summary).toBe('Connected through 1 independent place (3 links). Everything this computer sends goes through one place. Every link is encrypted and its other end checked.');
    expect(one.advice).toContain('spread your links over more places');
    const hot = linkWarmthWords(l({ level: 'hot', links: 3, places: 3, effective: 3, largest: 0.33 }));
    expect(hot.summary).toBe('Connected through 3 independent places. Every link is encrypted and its other end checked.');
    expect(hot.advice).toBeUndefined();
    const open = linkWarmthWords(l({ level: 'cool', links: 3, places: 3, effective: 3, largest: 0.33, guarded: 0.33 }));
    expect(open.summary).toContain('33% of links are encrypted and checked.');
    expect(open.advice).toContain('not encrypted or not checked');
    for (const words of [one, hot, open]) {
      for (const text of [words.summary, words.advice ?? '']) {
        expect(text).not.toMatch(FORBIDDEN);
        expect(text).not.toMatch(/secur|decentrali|sybil|attack/i);
      }
    }
  });
});
