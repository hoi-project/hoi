// Randomness from your own hands for a new key (the first-run game, act 1).
//
// The pool starts from the system's randomness (crypto.getRandomValues). Every pointer move,
// tap and key press is folded in with its timing (performance.now() and the gap since the last
// one) through SHA-256, a few dozen samples at a time. Samples live only until the next fold:
// they are never logged, sent or replayed, and everything is zeroed once the digest is taken.
//
// The core then mixes that digest into 32 fresh bytes of its own system randomness
// (hoi-crypto `Keypair::generate_stirred`), so the key is exactly as safe if nobody stirs at all,
// and the same wiggle never makes the same key. `points` only fills the ring: it is a game, not
// a measurement of anything.

/** Samples per fold. */
const BATCH = 32;
/** kind, x, y, pressure, time, gap */
const FIELDS = 6;
/** Points that fill the ring. */
export const FULL = 200;

const MOVE = 1;
const DOWN = 2;
const KEY = 3;

export class Stir {
  #pool = new Uint8Array(32);
  #batch = new Float64Array(BATCH * FIELDS);
  #n = 0;
  #folds = 0;
  #last = 0;
  #lastX = Number.NaN;
  #lastY = Number.NaN;
  #lastPoint = 0;
  #chain: Promise<void> = Promise.resolve();
  #taken = false;
  #broken = false;
  /** Game points toward the ring (FULL fills it). */
  points = 0;

  constructor() {
    crypto.getRandomValues(this.#pool);
  }

  /** Whether this page can fold samples (SHA-256 needs a secure context). */
  static supported(): boolean {
    return typeof crypto !== 'undefined' && typeof crypto.getRandomValues === 'function' && !!crypto.subtle;
  }

  /** 0..1 for the ring. */
  get filled(): number {
    return Math.min(1, this.points / FULL);
  }

  /** A pointer (mouse, pen, finger) moved. Returns the points it earned. */
  move(x: number, y: number, pressure = 0, t = performance.now()): number {
    if (this.#taken) return 0;
    const moved = !(Math.abs(x - this.#lastX) < 2 && Math.abs(y - this.#lastY) < 2);
    this.#push(MOVE, x, y, pressure, t);
    // Only real movement, at most ~50 a second, counts toward the ring.
    if (!moved || t - this.#lastPoint < 20) return 0;
    this.#lastX = x;
    this.#lastY = y;
    return this.#earn(1, t);
  }

  /** A tap or click. */
  down(x: number, y: number, pressure = 0, t = performance.now()): number {
    if (this.#taken) return 0;
    this.#push(DOWN, x, y, pressure, t);
    return this.#earn(4, t);
  }

  /** A key press. Its timing is what's random; held-down repeats are mixed in but earn nothing. */
  key(code: number, repeat: boolean, t = performance.now()): number {
    if (this.#taken) return 0;
    this.#push(KEY, code, repeat ? 1 : 0, 0, t);
    return repeat ? 0 : this.#earn(10, t);
  }

  /** The digest to stir into the key: 32 bytes. Zeroes everything; a stir is taken once. */
  async take(): Promise<Uint8Array> {
    if (this.#taken) throw new Error('This stir was already used.');
    this.#fold();
    try {
      await this.#chain;
      if (this.#broken) throw new Error('Stirring failed here; the key uses the system randomness alone.');
      return this.#pool.slice();
    } finally {
      this.wipe();
    }
  }

  /** Forget everything, now. */
  wipe(): void {
    this.#taken = true;
    this.#pool.fill(0);
    this.#batch.fill(0);
    this.#n = 0;
    this.points = 0;
    this.#lastX = this.#lastY = Number.NaN;
  }

  #earn(p: number, t: number): number {
    this.#lastPoint = t;
    this.points += p;
    return p;
  }

  #push(kind: number, a: number, b: number, c: number, t: number) {
    const o = this.#n * FIELDS;
    this.#batch[o] = kind;
    this.#batch[o + 1] = a;
    this.#batch[o + 2] = b;
    this.#batch[o + 3] = c;
    this.#batch[o + 4] = t;
    this.#batch[o + 5] = t - this.#last;
    this.#last = t;
    if (++this.#n === BATCH) this.#fold();
  }

  /** pool = SHA-256(pool ‖ fold counter ‖ samples). Copies the samples out and zeroes the batch at once. */
  #fold() {
    if (this.#n === 0) return;
    const samples = new Uint8Array(this.#batch.buffer.slice(0, this.#n * FIELDS * 8));
    this.#batch.fill(0);
    this.#n = 0;
    const counter = ++this.#folds;
    this.#chain = this.#chain.then(async () => {
      const input = new Uint8Array(32 + 8 + samples.length);
      input.set(this.#pool, 0);
      new DataView(input.buffer).setFloat64(32, counter);
      input.set(samples, 40);
      samples.fill(0);
      try {
        const out = new Uint8Array(await crypto.subtle.digest('SHA-256', input));
        // a fold still running after wipe() mustn't bring anything back
        if (!this.#taken) this.#pool.set(out);
        out.fill(0);
      } catch {
        this.#broken = true;
      } finally {
        input.fill(0);
      }
    });
  }
}
