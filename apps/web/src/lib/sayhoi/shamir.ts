// Shamir's secret sharing over GF(2^8), byte by byte: k of n pieces rebuild the bytes, fewer
// give bytes that mean nothing. The first-run game uses it to rehearse recovery (3 of 5
// friends, TECHNICAL.md §9) on the PUBLIC key, so the lesson is real maths with nothing secret
// in it. Real recovery shares are made in the core and encrypted to each friend;
// this public-key exercise does not create or store those shares.

// log/exp tables for the field with the AES polynomial x^8 + x^4 + x^3 + x + 1, generator 3
const EXP = new Uint8Array(510);
const LOG = new Uint8Array(256);
{
  let x = 1;
  for (let i = 0; i < 255; i++) {
    EXP[i] = x;
    LOG[x] = i;
    // x *= 3 in GF(2^8)
    x ^= (x << 1) ^ (x & 0x80 ? 0x11b : 0);
    x &= 0xff;
  }
  for (let i = 255; i < 510; i++) EXP[i] = EXP[i - 255];
}

const mul = (a: number, b: number): number => (a && b ? EXP[LOG[a] + LOG[b]] : 0);
const div = (a: number, b: number): number => (a ? EXP[LOG[a] + 255 - LOG[b]] : 0);

export interface Share {
  /** 1..255 */
  x: number;
  y: Uint8Array;
}

/** Splits `secret` into `n` shares, any `k` of which rebuild it. */
export function split(
  secret: Uint8Array,
  n: number,
  k: number,
  random: (b: Uint8Array<ArrayBuffer>) => void = (b) => crypto.getRandomValues(b),
): Share[] {
  if (k < 2 || k > n || n > 255) throw new Error('need 2 <= k <= n <= 255');
  const shares = Array.from({ length: n }, (_, i) => ({ x: i + 1, y: new Uint8Array(secret.length) }));
  const coeffs = new Uint8Array(k - 1);
  for (let byte = 0; byte < secret.length; byte++) {
    random(coeffs);
    for (const s of shares) {
      // Horner: secret + c1·x + c2·x² + …
      let y = 0;
      for (let j = coeffs.length - 1; j >= 0; j--) y = mul(y ^ coeffs[j], s.x);
      s.y[byte] = y ^ secret[byte];
    }
  }
  coeffs.fill(0);
  return shares;
}

/** Lagrange interpolation at 0. With fewer shares than the threshold the result is noise. */
export function combine(shares: Share[]): Uint8Array {
  const len = shares[0]?.y.length ?? 0;
  const out = new Uint8Array(len);
  shares.forEach((si, i) => {
    // basis polynomial for share i, at x = 0: ∏ xj / (xj - xi), and minus is xor here
    let l = 1;
    shares.forEach((sj, j) => {
      if (i !== j) l = mul(l, div(sj.x, sj.x ^ si.x));
    });
    for (let b = 0; b < len; b++) out[b] ^= mul(si.y[b], l);
  });
  return out;
}
