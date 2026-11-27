// The dotted world map written by scripts/geo-assets.mjs (Natural Earth, Equal Earth projection).
export interface MapCountry { p: [number, number]; b?: [number, number, number, number]; d?: string }
export interface WorldMap {
  v: 1;
  w: number;
  h: number;
  step: number;
  codes: string[];
  rows: number[][];
  dots: number[];
  countries: Record<string, MapCountry>;
}

const damaged = () => new Error('The map data is damaged.');
const isNumber = (n: unknown): n is number => typeof n === 'number' && Number.isFinite(n);

export function parseWorldMap(value: unknown): WorldMap {
  const m = value as WorldMap;
  if (!m || m.v !== 1 || !isNumber(m.w) || !isNumber(m.h) || !isNumber(m.step) || m.step <= 0) throw damaged();
  if (!Array.isArray(m.codes) || !Array.isArray(m.rows) || !Array.isArray(m.dots) || !m.countries || typeof m.countries !== 'object') throw damaged();
  if (m.rows.some(row => !Array.isArray(row) || row.length % 2 !== 1 || !row.every(isNumber))) throw damaged();
  const runs = m.rows.reduce((n, [, ...r]) => n + r.reduce((a, v, i) => (i % 2 ? a + v : a), 0), 0);
  if (runs !== m.dots.length || m.dots.some(i => !isNumber(i) || i < 0 || i >= m.codes.length)) throw damaged();
  for (const [code, c] of Object.entries(m.countries)) {
    if (!/^[A-Z]{2}$/.test(code) || !Array.isArray(c.p) || !c.p.every(isNumber)) throw damaged();
    if (c.d !== undefined && (typeof c.d !== 'string' || !/^[Mmlz0-9.\s-]*$/.test(c.d))) throw damaged();
    if (c.b !== undefined && !(Array.isArray(c.b) && c.b.length === 4 && c.b.every(isNumber))) throw damaged();
  }
  return m;
}

export interface Dot { x: number; y: number; code: string }

export function mapDots(map: WorldMap): Dot[] {
  const dots: Dot[] = [];
  let k = 0;
  for (const [r, ...runs] of map.rows) {
    const y = r * map.step + map.step / 2;
    for (let i = 0; i < runs.length; i += 2) {
      for (let c = runs[i]; c < runs[i] + runs[i + 1]; c++) dots.push({ x: c * map.step + map.step / 2, y, code: map.codes[map.dots[k++]] });
    }
  }
  return dots;
}

/** One path for every land dot: zero-length segments drawn with round caps. */
export function dotsPath(dots: Dot[]): string {
  return dots.map(d => `M${d.x} ${d.y}h0`).join('');
}
