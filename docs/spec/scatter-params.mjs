#!/usr/bin/env node
// Sizing for scattered delivery (SCATTER.md §3, §5.4): the smallest k-of-n that keeps a party
// holding a given share of the independent places from rebuilding or withholding an object, with
// exact integer arithmetic (BigInt), never floats, so every implementation reaches the same answer.
// This is the reference the Rust sizing function and core/vectors/scatter.json are checked against.
//
//   node docs/spec/scatter-params.mjs            tables used in SCATTER.md
//   node docs/spec/scatter-params.mjs 10 0 300   one answer: level %, generation, places seen
//   node docs/spec/scatter-params.mjs --check core/vectors/scatter.json
//                                                    every sizing case and reading in the vectors

const LIFE = 1_000_000n; // lifetime risk target: one object in a million (1 / LIFE)
const UP = [4n, 5n]; // the uptime bar for holders, 0.8, used for the availability check
const AVAIL = [999n, 1000n]; // readable while ≥ k of n are up, per repair interval
const EXACT_UP_TO = 1000; // places seen: hypergeometric up to here, binomial (stricter) above
const MAX_N = 160;
const LINKS = 3; // a browser's links: each carries at most k−1 pieces, so n ≤ LINKS·(k−1)

const fact = [1n];
const memo = new Map();
const C = (n, r) => {
  if (r < 0 || r > n) return 0n;
  const key = n * 4096 + r;
  let v = memo.get(key);
  if (v === undefined) {
    while (fact.length <= n) fact.push(fact[fact.length - 1] * BigInt(fact.length));
    v = fact[n] / (fact[r] * fact[n - r]);
    memo.set(key, v);
  }
  return v;
};
const pow = (b, e) => b ** BigInt(e);

// P(Bin(n, a/b) ≥ k) as an exact fraction [num, den]
function binTail(n, a, b, k) {
  let num = 0n;
  for (let i = k; i <= n; i++) num += C(n, i) * pow(a, i) * pow(b - a, n - i);
  return [num, pow(b, n)];
}
// P(X ≥ k), X ~ Hypergeometric(N places, A the party's, m drawn)
function hypTail(N, A, m, k) {
  let num = 0n;
  for (let i = k; i <= Math.min(A, m); i++) num += C(A, i) * C(N - A, m - i);
  return [num, C(N, m)];
}
const le = ([n1, d1], [n2, d2]) => n1 * d2 <= n2 * d1;
const ge = ([n1, d1], [n2, d2]) => n1 * d2 >= n2 * d1;

// The risk that a party with `level`% of N places holds ≥ k of m draws (N = Infinity: binomial).
function risk(level, N, m, k) {
  if (N === Infinity || N > EXACT_UP_TO) return binTail(m, BigInt(level), 100n, k);
  return hypTail(N, Math.floor((level * N) / 100), m, k);
}
const budget = (n) => n + Math.ceil(n / 4);

// The generation's size: the smallest n, then k, meeting the rebuild, withhold and availability
// bounds, and small enough for a browser's three links to carry. Generation g may spend LIFE⁻¹ / 2^(g+1), so all generations together stay under LIFE⁻¹.
export function size(level, g, N) {
  const eps = [1n, LIFE * 2n ** BigInt(g + 1)];
  const top = N === Infinity ? MAX_N : Math.min(N, MAX_N);
  for (let n = 5; n <= top; n++) {
    const M = Math.min(budget(n), N === Infinity ? Infinity : N);
    for (let k = 3; k <= n - 2; k++) {
      if (!ge(binTail(n, UP[0], UP[1], k), AVAIL)) break; // larger k is only less available
      if (n > LINKS * (k - 1)) continue; // three links could not carry the upload
      if (le(risk(level, N, M, k), eps) && le(risk(level, N, n, n - k + 1), eps)) return { k, n, M };
    }
  }
  return null; // not scattered: too few places for this level
}

// What a reader shows: the highest level whose rebuild and withhold bounds a generation meets.
export function reading(g, N, { k, n, m }) {
  const eps = [1n, LIFE * 2n ** BigInt(g + 1)];
  for (const level of [20, 15, 10]) {
    if (le(risk(level, N, m, k), eps) && le(risk(level, N, n, n - k + 1), eps)) return level;
  }
  return null;
}
const hard = (level, N, k) => N !== Infinity && N <= EXACT_UP_TO && Math.floor((level * N) / 100) < k;

const sci = ([num, den]) => {
  if (num === 0n) return '0';
  const x = Number((num * 10n ** 30n) / den) / 1e30;
  return x.toExponential(1);
};

// u32::MAX in the vectors stands for any large network.
const places = (p) => (p >= 4294967295 ? Infinity : p);

if (process.argv[2] === '--check') {
  const { readFileSync } = await import('node:fs');
  const v = JSON.parse(readFileSync(process.argv[3], 'utf8')).sizing;
  let bad = 0;
  for (const c of v.cases) {
    const s = size(c.level, c.generation, places(c.places));
    const want = c.size && { k: c.size.k, n: c.size.n, M: c.size.m };
    const same = s === null ? want === null : want !== null && s.k === want.k && s.n === want.n && s.M === want.M && hard(c.level, places(c.places), s.k) === c.size.hard;
    if (!same) {
      bad++;
      console.error('differs:', JSON.stringify(c), '→ reference', JSON.stringify(s));
    }
  }
  for (const r of v.readings) {
    const got = reading(r.generation, places(r.places), r);
    if (got !== r.level) {
      bad++;
      console.error('reading differs:', JSON.stringify(r), '→ reference', got);
    }
  }
  console.log(`${v.cases.length} sizing cases and ${v.readings.length} readings: ${bad ? `${bad} differ` : 'all agree with the reference'}`);
  process.exit(bad ? 1 : 0);
} else if (process.argv.length > 2) {
  const [level, g, N] = process.argv.slice(2).map(Number);
  console.log(size(level, g, Number.isFinite(N) ? N : Infinity));
} else {
  console.log('Large networks (binomial), generations 0–8: k/n (budget M)');
  for (const level of [10, 15, 20]) {
    const row = [];
    for (let g = 0; g <= 8; g++) {
      const s = size(level, g, Infinity);
      row.push(`g${g} ${s.k}/${s.n} (${s.M})`);
    }
    console.log(`  ${level}%: ${row.join('  ')}`);
  }
  console.log('\nGeneration 0 by places seen N (hard: the party holds fewer places than k)');
  for (const N of [9, 12, 15, 20, 25, 30, 40, 50, 60, 80, 90, 100, 150, 200, 300, 500, 1000, Infinity]) {
    const row = [10, 15, 20].map((level) => {
      const s = size(level, 0, N);
      if (!s) return `${level}%: —`;
      const A = N === Infinity ? '' : Math.floor((level * N) / 100);
      const hard = N !== Infinity && A < s.k ? ' hard' : '';
      return `${level}%: ${s.k}/${s.n} (${s.M})${hard}`;
    });
    console.log(`  N=${String(N).padStart(8)}  ${row.join('   ')}`);
  }
  console.log('\nGeneration 0 at 10% (15/28, M 35) against other shares, and what came before');
  for (const f of [5, 10, 15, 20]) {
    const rebuild = sci(binTail(35, BigInt(f), 100n, 15));
    const withhold = sci(binTail(28, BigInt(f), 100n, 14));
    const before = sci(binTail(6, BigInt(f), 100n, 3));
    const today = (1 - (1 - f / 100) ** 3).toFixed(2);
    console.log(`  ${f}%: rebuild ${rebuild}  withhold ${withhold}  | 3-of-6 ${before}  | one of three whole copies ${today}`);
  }
  console.log('\nThe 3-of-6 design with repair re-placing lost indexes, at a 10% share');
  for (const t of [1, 2, 3, 5]) {
    // each index placed t times: the party holds it with 1 − 0.9^t
    const den = 10n ** BigInt(t);
    const a = den - 9n ** BigInt(t);
    console.log(`  each index placed ${t}×: ${sci(binTail(6, a, den, 3))}`);
  }
}
