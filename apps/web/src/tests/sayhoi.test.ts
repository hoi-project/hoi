import { afterEach, describe, expect, it, vi } from 'vitest';
import { leadingZeroBits } from '@hoi/core-api';
import { FULL, Stir } from '../lib/sayhoi/stir';
import { combine, split } from '../lib/sayhoi/shamir';
import { picture } from '../lib/sayhoi/identicon';

/** The same little wiggle, sample for sample, with the same timing. */
function wiggle(s: Stir) {
  for (let i = 0; i < 80; i++) s.move(10 + i * 3, 40 + (i % 7) * 5, 0.5, 1000 + i * 25);
  s.down(50, 50, 0.5, 4000);
  s.key(65, false, 4100);
}

describe('stirring a key', () => {
  afterEach(() => vi.restoreAllMocks());

  it('starts from the system randomness, so the same wiggle never gives the same stir', async () => {
    const a = new Stir();
    const b = new Stir();
    wiggle(a);
    wiggle(b);
    const [x, y] = await Promise.all([a.take(), b.take()]);
    expect(x).toHaveLength(32);
    expect(x).not.toEqual(y);
  });

  it('folds every move and its timing in, not just the count', async () => {
    // with the system randomness pinned, only the gestures differ
    vi.spyOn(crypto, 'getRandomValues').mockImplementation((b) => b);
    const a = new Stir();
    const b = new Stir();
    const c = new Stir();
    wiggle(a);
    wiggle(b);
    wiggle(c);
    c.move(200, 200, 0.5, 9000);
    const [x, y, z] = await Promise.all([a.take(), b.take(), c.take()]);
    expect(x).toEqual(y);
    expect(z).not.toEqual(x);
    const late = new Stir();
    for (let i = 0; i < 80; i++) late.move(10 + i * 3, 40 + (i % 7) * 5, 0.5, 1000 + i * 25 + 0.1);
    late.down(50, 50, 0.5, 4000);
    late.key(65, false, 4100);
    expect(await late.take()).not.toEqual(x);
  });

  it('is taken once and forgets everything', async () => {
    const s = new Stir();
    wiggle(s);
    expect(s.points).toBeGreaterThan(0);
    await s.take();
    expect(s.points).toBe(0);
    expect(s.move(1, 2)).toBe(0);
    await expect(s.take()).rejects.toThrow();
  });

  it('says so when it cannot fold, instead of handing over a half-stirred pool', async () => {
    vi.spyOn(crypto.subtle, 'digest').mockRejectedValue(new Error('no SHA-256 here'));
    const s = new Stir();
    wiggle(s);
    await expect(s.take()).rejects.toThrow(/system randomness alone/);
    expect(s.points).toBe(0);
  });

  it('fills the ring with real movement only: no points for standing still or held-down keys', () => {
    const s = new Stir();
    expect(s.move(10, 10, 0, 100)).toBe(1);
    expect(s.move(30, 10, 0, 110)).toBe(0); // too soon after the last point
    expect(s.move(30, 10, 0, 200)).toBe(1);
    expect(s.move(31, 10, 0, 300)).toBe(0); // under 2 px since then
    expect(s.key(32, true, 400)).toBe(0);
    expect(s.key(32, false, 500)).toBeGreaterThan(0);
    for (let i = 0; i < 400; i++) s.move(i * 5, 0, 0, 1000 + i * 30);
    expect(s.points).toBeGreaterThan(FULL);
    expect(s.filled).toBe(1);
    s.wipe();
  });
});

describe('3 of 5 friends', () => {
  const key = Uint8Array.from({ length: 32 }, (_, i) => (i * 37 + 11) & 0xff);
  const shares = split(key, 5, 3);

  it('rebuilds the key from any three, four or five pieces', () => {
    for (let a = 0; a < 5; a++)
      for (let b = a + 1; b < 5; b++)
        for (let c = b + 1; c < 5; c++) expect(combine([shares[a], shares[b], shares[c]])).toEqual(key);
    expect(combine(shares.slice(1))).toEqual(key);
    expect(combine(shares)).toEqual(key);
  });

  it('gives nothing back from one or two', () => {
    expect(combine([shares[0], shares[3]])).not.toEqual(key);
    expect(combine([shares[2]])).not.toEqual(key);
    // and every piece on its own looks nothing like the key
    for (const s of shares) expect(s.y).not.toEqual(key);
  });

  it('splits with fresh randomness each time', () => {
    expect(split(key, 5, 3)[0].y).not.toEqual(shares[0].y);
  });
});

describe("a key's picture", () => {
  it('is drawn from the key alone, mirrored, with one coral pair', () => {
    const k = Uint8Array.from({ length: 32 }, (_, i) => (i * 91 + 7) & 0xff);
    const p = picture(k);
    expect(picture(k)).toEqual(p);
    for (const d of p.dots) expect(p.dots.some((e) => e.cx === 100 - d.cx && e.cy === d.cy && e.r === d.r)).toBe(true);
    const coral = p.dots.filter((d) => d.coral);
    expect(coral.length === 1 || coral.length === 2).toBe(true);
    const other = Uint8Array.from(k, (b, i) => (i === 3 ? b ^ 0xff : b));
    expect(picture(other)).not.toEqual(p);
  });
});

describe('leading zero bits', () => {
  it('counts the work a hash carries', () => {
    expect(leadingZeroBits(new Uint8Array(32))).toBe(256);
    expect(leadingZeroBits(Uint8Array.from([0, 0b0001_0000]))).toBe(11);
    expect(leadingZeroBits(Uint8Array.from([0x80]))).toBe(0);
    expect(leadingZeroBits(Uint8Array.from([0, 0, 1]))).toBe(23);
  });
});
