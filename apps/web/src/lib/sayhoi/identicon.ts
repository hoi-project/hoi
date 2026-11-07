// A key's picture: a tiny network of dots on a pastel disc, drawn from the key's own bytes
// (the logo's language: dots are people, lines join them, the coral one is a post). Mirrored,
// so it reads as a face at a glance. It's for recognising your key, not for checking one: two
// people compare the fingerprint for that.
import type { CommunityColor } from '@hoi/core-api';

export interface Dot {
  cx: number;
  cy: number;
  r: number;
  coral: boolean;
}

export interface Picture {
  /** Disc colour; coral stays for the dot. */
  ground: Exclude<CommunityColor, 'coral'>;
  dots: Dot[];
  links: [Dot, Dot][];
}

const GROUNDS = ['mint', 'sky', 'lilac', 'peach', 'sun'] as const;
const SIZES = [4.6, 5.6, 6.6, 7.6];
/** Cell centres on a 100 × 100 view box. */
const at = (i: number) => 18 + i * 16;

/** 5 × 5 cells, columns mirrored: 15 come from bytes 0–14 (5 in 8 are dots), the ground from 15,
 * the coral dot from 16. */
export function picture(key: Uint8Array): Picture {
  const grid: (Dot | null)[][] = Array.from({ length: 5 }, () => Array<Dot | null>(5).fill(null));
  const on: [number, number][] = [];
  for (let row = 0; row < 5; row++) {
    for (let col = 0; col < 3; col++) {
      const b = key[row * 3 + col] ?? 0;
      if ((b & 7) > 4) continue;
      const r = SIZES[(b >> 3) & 3];
      grid[row][col] = { cx: at(col), cy: at(row), r, coral: false };
      grid[row][4 - col] = { cx: at(4 - col), cy: at(row), r, coral: false };
      on.push([row, col]);
    }
  }
  if (on.length) {
    const [row, col] = on[(key[16] ?? 0) % on.length];
    grid[row][col]!.coral = true;
    grid[row][4 - col]!.coral = true;
  }
  const dots = grid.flat().filter((d): d is Dot => d !== null);
  const links: [Dot, Dot][] = [];
  for (let row = 0; row < 5; row++) {
    for (let col = 0; col < 5; col++) {
      const d = grid[row][col];
      if (!d) continue;
      const right = col < 4 ? grid[row][col + 1] : null;
      const down = row < 4 ? grid[row + 1][col] : null;
      if (right) links.push([d, right]);
      if (down) links.push([d, down]);
    }
  }
  return { ground: GROUNDS[(key[15] ?? 0) % GROUNDS.length], dots, links };
}
