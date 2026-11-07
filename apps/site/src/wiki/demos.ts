// Live demos for the wiki's stages:
//   <div class="wk-stage" data-demo="name" data-title="…"><p class="nojs">…</p><p class="wk-stage-cap">…</p></div>
// Each demo builds its own DOM inside the stage body and may return a replay function.
// Numbers come from the code where it matters (rate windows, stamp format, Shamir's field);
// the records they work on are simplified, and the captions say so.
import { animate, createTimeline, stagger, utils } from 'animejs';
import { blake3, hex, leadingZeroBits } from './blake3';

type Replay = () => void;
export type Demo = (body: HTMLElement, reduce: boolean, stage: HTMLElement) => Replay | void;

function h<K extends keyof HTMLElementTagNameMap>(tag: K, cls = '', text = ''): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text) e.textContent = text;
  return e;
}
function btn(label: string, cls = 'wk-btn'): HTMLButtonElement {
  const b = h('button', cls, label);
  b.type = 'button';
  return b;
}
const sleep = (ms: number) => new Promise<void>((r) => window.setTimeout(r, ms));
/** Runs `fn` when the stage first scrolls into view (once). */
function whenSeen(el: HTMLElement, fn: () => void): void {
  if (!('IntersectionObserver' in window)) { fn(); return; }
  const io = new IntersectionObserver((es) => { if (es.some((e) => e.isIntersecting)) { io.disconnect(); fn(); } }, { threshold: 0.25 });
  io.observe(el);
}

/* =========================================================================================
   hash: a simplified record, its canonical CBOR, its BLAKE3 id and whether a signature holds
   ========================================================================================= */

// Canonical CBOR (DAG-CBOR order: shorter keys first, then bytewise) for the few types we need.
function cborHead(major: number, n: number, out: number[]): void {
  if (n < 24) out.push((major << 5) | n);
  else if (n < 0x100) out.push((major << 5) | 24, n);
  else if (n < 0x10000) out.push((major << 5) | 25, n >> 8, n & 0xff);
  else if (n < 0x100000000) out.push((major << 5) | 26, (n >>> 24) & 0xff, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff);
  else { const hi = Math.floor(n / 0x100000000); out.push((major << 5) | 27, 0, 0, (hi >> 8) & 0xff, hi & 0xff, (n >>> 24) & 0xff, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff); }
}
type V = number | string | Uint8Array | { [k: string]: V };
const enc = new TextEncoder();
function cbor(v: V, out: number[] = []): number[] {
  if (typeof v === 'number') cborHead(0, v, out);
  else if (typeof v === 'string') { const b = enc.encode(v); cborHead(3, b.length, out); out.push(...b); }
  else if (v instanceof Uint8Array) { cborHead(2, v.length, out); out.push(...v); }
  else {
    const keys = Object.keys(v).sort((a, b) => a.length - b.length || (a < b ? -1 : a > b ? 1 : 0));
    cborHead(5, keys.length, out);
    for (const k of keys) { cbor(k, out); cbor(v[k], out); }
  }
  return out;
}

const hashDemo: Demo = (body, reduce) => {
  const community = blake3(enc.encode('c/privacy'));
  const created = 1_793_000_000;
  const wrap = h('div', 'dm-grid');
  const left = h('div');
  const label = h('label', 'dm-k', 'Post text');
  const input = h('input', 'sb-q');
  input.value = 'Which messengers survived the audit?';
  input.maxLength = 140;
  input.id = 'dm-hash-' + Math.random().toString(36).slice(2, 8);
  label.htmlFor = input.id;
  const fields = h('pre', 'dm-hex');
  left.append(label, input, fields);
  const right = h('div');
  const bytesK = h('div', 'dm-k', 'Canonical bytes');
  const bytes = h('div', 'dm-hex');
  const idK = h('div', 'dm-k', 'id = BLAKE3-256(bytes)');
  const id = h('div', 'dm-hex');
  const sigRow = h('div', 'dm-row');
  const sig = h('span', 'tag t-mint', 'signature checks');
  const sign = btn('Sign this version');
  sigRow.append(sig, sign);
  right.append(bytesK, bytes, idK, id, sigRow);
  wrap.append(left, right);
  body.append(wrap);
  let signed = '';
  let prev: number[] = [];
  const render = () => {
    const rec = { v: 1, kind: 'post', community, created_at: created, expires_at: created + 15_552_000, body: { title: '', text: input.value }, nonce: 4_211_337 };
    const b = cbor(rec);
    fields.textContent = `{ v: 1, kind: "post",\n  community: ${hex(community).slice(0, 12)}…,\n  created_at: ${created},\n  expires_at: ${created + 15_552_000},\n  body: { title: "", text: ${JSON.stringify(input.value)} },\n  nonce: 4211337 }`;
    bytes.replaceChildren();
    b.forEach((x, i) => {
      const s = h('span', x === prev[i] ? 'nz' : 'z', x.toString(16).padStart(2, '0'));
      bytes.append(s, document.createTextNode(i % 2 ? ' ' : ''));
    });
    const d = hex(blake3(Uint8Array.from(b)));
    if (!signed) signed = d;
    id.textContent = 'b3:' + d;
    const ok = signed === d;
    sig.textContent = ok ? 'signature checks ✓' : 'signature no longer matches the id ✗';
    sig.className = 'tag ' + (ok ? 't-mint' : 't-coral');
    if (!reduce && prev.length) animate(id, { opacity: [0.4, 1], duration: 300 });
    prev = b;
    bytesK.textContent = `Canonical bytes (${b.length} B; changed bytes in green)`;
  };
  input.addEventListener('input', render);
  sign.addEventListener('click', () => {
    signed = '';
    render();
    if (!reduce) animate(sig, { scale: [1.25, 1], rotate: [-6, 0], duration: 500, ease: 'outBack(3)' });
  });
  render();
  return () => { input.value = 'Which messengers survived the audit?'; signed = ''; prev = []; render(); };
};

/* =========================================================================================
   stamp: searching nonces until BLAKE3(id ‖ u64be(nonce)) has enough leading zero bits
   ========================================================================================= */
const stampDemo: Demo = (body, reduce) => {
  const top = h('div', 'dm-row');
  const lab = h('label', 'dm-k', 'Bits');
  const range = h('input');
  range.type = 'range';
  range.min = '4';
  range.max = '20';
  range.value = '14';
  range.id = 'dm-bits-' + Math.random().toString(36).slice(2, 8);
  lab.htmlFor = range.id;
  const bitsV = h('b', 'dm-k', '14');
  const go = btn('Mine', 'wk-btn on');
  top.append(lab, range, bitsV, go);
  const cells = h('div', 'dm-row');
  const cellEls: HTMLElement[] = [];
  for (let i = 0; i < 32; i++) { const c = h('span', 'dm-cell', ''); cellEls.push(c); cells.append(c); }
  const stats = h('div', 'dm-grid');
  const mk = (k: string) => { const d = h('div'); const kk = h('div', 'dm-k', k); const v = h('div', 'dm-big', '0'); d.append(kk, v); stats.append(d); return v; };
  const tries = mk('tries');
  const expect = mk('expected ≈ 2^bits');
  const rate = mk('hashes / second (this tab)');
  const hashLine = h('div', 'dm-hex');
  const stamp = h('span', 'tag t-sun', '');
  stamp.hidden = true;
  body.append(top, cells, stats, hashLine, stamp);
  range.style.setProperty('accent-color', 'var(--sun)');
  const id = blake3(enc.encode('a record id'));
  const buf = new Uint8Array(40);
  buf.set(id, 0);
  let run = 0;
  const showHash = (hsh: Uint8Array, need: number) => {
    const z = leadingZeroBits(hsh);
    for (let i = 0; i < 32; i++) {
      const bit = (hsh[i >> 3] >> (7 - (i & 7))) & 1;
      cellEls[i].textContent = String(bit);
      cellEls[i].className = 'dm-cell' + (i < need ? (i < z ? ' on' : ' bad') : '');
    }
    const hx = hex(hsh);
    hashLine.replaceChildren(h('span', 'z', hx.slice(0, Math.floor(z / 4))), h('span', 'nz', hx.slice(Math.floor(z / 4))));
  };
  const mine = async () => {
    const my = ++run;
    const need = Number(range.value);
    expect.textContent = (2 ** need).toLocaleString('en-US');
    stamp.hidden = true;
    go.disabled = true;
    let n = 0;
    const t0 = performance.now();
    let last = performance.now();
    for (;;) {
      const until = performance.now() + 12;
      let found = -1;
      let hsh: Uint8Array = new Uint8Array(32);
      while (performance.now() < until) {
        for (let j = 0; j < 400; j++) {
          // u64 big-endian nonce after the id
          let x = n;
          for (let b = 39; b >= 32; b--) { buf[b] = x & 0xff; x = Math.floor(x / 256); }
          hsh = blake3(buf);
          if (leadingZeroBits(hsh) >= need) { found = n; break; }
          n++;
        }
        if (found >= 0) break;
      }
      if (my !== run) return;
      if (performance.now() - last > 60 || found >= 0) {
        last = performance.now();
        tries.textContent = (n + 1).toLocaleString('en-US');
        rate.textContent = Math.round((n + 1) / Math.max(0.001, (performance.now() - t0) / 1000)).toLocaleString('en-US');
        showHash(hsh, need);
      }
      if (found >= 0) {
        stamp.hidden = false;
        stamp.textContent = `stamped · nonce ${found} · ${leadingZeroBits(hsh)} zero bits`;
        if (!reduce) animate(stamp, { scale: [2.2, 1], rotate: [-18, -3], opacity: [0, 1], duration: 650, ease: 'outElastic(1, .6)' });
        go.disabled = false;
        return;
      }
      await new Promise((r) => requestAnimationFrame(r));
    }
  };
  range.addEventListener('input', () => { bitsV.textContent = range.value; });
  go.addEventListener('click', () => void mine());
  whenSeen(body, () => void mine());
  return () => void mine();
};

/* =========================================================================================
   pipeline: records passing the validation stages in order
   ========================================================================================= */
const STAGES: [string, string][] = [
  ['Decode', 'canonical bytes, id matches, size cap'],
  ['Stamp floor', 'the least work any record of this kind needs'],
  ['Signature', 'author or a consenting, blessed device'],
  ['Stamp', 'enough work for this community and key age'],
  ['Identity', 'the key did enough identity work'],
  ['Time', 'within 10 minutes of now; fades in the window'],
  ['Moderation', 'author not banned, record not removed'],
  ['Rate', 'within the key\'s windows, by created_at'],
  ['Rules', 'text, links and media rules'],
  ['Standing', 'may this author do this here?'],
  ['Accept', 'stored, indexed, passed along'],
];
const CASES: { label: string; fail: number; result: string }[] = [
  { label: 'A good post', fail: -1, result: 'accepted' },
  { label: 'Forged signature', fail: 2, result: 'dropped · sender gets a strike' },
  { label: 'Too little work', fail: 3, result: 'dropped quietly' },
  { label: 'From next week', fail: 5, result: 'dropped' },
  { label: 'The 13th post this hour', fail: 7, result: 'dropped · key gets a strike' },
  { label: 'Raw HTML', fail: 8, result: 'dropped · key gets a strike' },
  { label: 'Newcomer posting a link', fail: 9, result: 'held back: stored, never passed on' },
];
const pipelineDemo: Demo = (body, reduce) => {
  const choose = h('div', 'dm-row');
  const list = h('div', 'dm-stagelist');
  const rows = STAGES.map(([n, d], i) => {
    const r = h('div');
    r.append(h('span', 'n', String(i + 1).padStart(2, '0')), h('span', '', `${n} · ${d}`), h('span', 'r', ''));
    list.append(r);
    return r;
  });
  const btns = CASES.map((c, i) => {
    const b = btn(c.label);
    b.addEventListener('click', () => void play(i));
    choose.append(b);
    return b;
  });
  body.append(choose, list);
  list.style.setProperty('margin-top', '14px');
  let run = 0;
  const play = async (ci: number) => {
    const my = ++run;
    const c = CASES[ci];
    btns.forEach((b, i) => b.classList.toggle('on', i === ci));
    rows.forEach((r) => { r.className = ''; (r.lastChild as HTMLElement).textContent = ''; });
    for (let i = 0; i < rows.length; i++) {
      if (my !== run) return;
      const fail = i === c.fail;
      rows[i].className = fail ? 'fail' : 'pass';
      (rows[i].lastChild as HTMLElement).textContent = fail ? c.result : i === rows.length - 1 ? c.result : 'ok';
      if (!reduce) animate(rows[i], { x: fail ? [0, -6, 6, -3, 0] : [-8, 0], duration: fail ? 420 : 260, ease: 'outQuad' });
      if (fail) return;
      if (!reduce) await sleep(170);
    }
  };
  whenSeen(body, () => void play(0));
  let k = 0;
  return () => { k = (k + 1) % CASES.length; void play(k); };
};

/* =========================================================================================
   reconcile: comparing range fingerprints, splitting where they differ
   ========================================================================================= */
const reconcileDemo: Demo = (body, reduce) => {
  const N = 32;
  const box = h('div');
  const mkRow = (name: string) => {
    const r = h('div', 'dm-row');
    const k = h('span', 'dm-k', name);
    k.style.setProperty('width', '86px');
    r.append(k);
    const cells: HTMLElement[] = [];
    for (let i = 0; i < N; i++) { const c = h('span', 'dm-cell', ''); cells.push(c); r.append(c); }
    box.append(r);
    return cells;
  };
  const a = mkRow('device A');
  const ranges = h('div', 'dm-row');
  const rk = h('span', 'dm-k', 'ranges');
  rk.style.setProperty('width', '86px');
  ranges.append(rk);
  const rangeCells: HTMLElement[] = [];
  for (let i = 0; i < N; i++) { const c = h('span', 'dm-cell', ''); c.style.setProperty('height', '8px'); rangeCells.push(c); ranges.append(c); }
  box.append(ranges);
  const b = mkRow('device B');
  const log = h('div', 'dm-hex');
  body.append(box, log);
  box.style.setProperty('display', 'grid');
  box.style.setProperty('gap', '6px');
  let run = 0;
  const play = async () => {
    const my = ++run;
    // Both hold records at these time slots; a few differ.
    const has = (seed: number) => Array.from({ length: N }, (_, i) => ((i * 7 + seed) % 5 !== 0) || i % 9 === 3);
    const ha = has(1);
    const hb = ha.slice();
    for (const i of [5, 6, 19, 27]) hb[i] = !hb[i];
    a.forEach((c, i) => { c.className = 'dm-cell' + (ha[i] ? ' on' : ' lost'); c.textContent = ''; });
    b.forEach((c, i) => { c.className = 'dm-cell' + (hb[i] ? ' on' : ' lost'); c.textContent = ''; });
    rangeCells.forEach((c) => { c.className = 'dm-cell'; });
    let msgs = 0;
    let queue: [number, number][] = [[0, N]];
    const lines: string[] = [];
    const say = (s: string) => { lines.push(s); log.textContent = lines.slice(-4).join('\n'); };
    say('A → B: fingerprint of [0, 32)');
    while (queue.length) {
      if (my !== run) return;
      const next: [number, number][] = [];
      for (const [lo, hi] of queue) {
        msgs++;
        let same = true;
        for (let i = lo; i < hi; i++) if (ha[i] !== hb[i]) same = false;
        for (let i = lo; i < hi; i++) rangeCells[i].className = 'dm-cell ' + (same ? 'on' : 'bad');
        if (same) continue;
        if (hi - lo <= 4) {
          for (let i = lo; i < hi; i++) {
            if (ha[i] === hb[i]) continue;
            const to = ha[i] ? b : a;
            to[i].className = 'dm-cell on';
            to[i].textContent = '↓';
            if (!reduce) animate(to[i], { scale: [0.3, 1], duration: 450, ease: 'outBack(3)' });
          }
          say(`[${lo}, ${hi}) differs: send the ids, fetch what's missing`);
          continue;
        }
        const mid = (lo + hi) >> 1;
        next.push([lo, mid], [mid, hi]);
        say(`[${lo}, ${hi}) differs: split into [${lo}, ${mid}) and [${mid}, ${hi})`);
      }
      queue = next;
      if (!reduce) await sleep(650);
    }
    say(`in step after ${msgs} range comparisons, not ${N} id lists`);
  };
  whenSeen(body, () => void play());
  return () => void play();
};

/* =========================================================================================
   gossip: one record spreading through a mesh, duplicates dropped
   ========================================================================================= */
const gossipDemo: Demo = (body, reduce) => {
  const W = 640, H = 240;
  const NS = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(NS, 'svg');
  svg.setAttribute('viewBox', `0 0 ${W} ${H}`);
  svg.setAttribute('role', 'img');
  svg.setAttribute('aria-label', 'A record spreading through a mesh of devices');
  svg.classList.add('dm-canvas');
  const stats = h('div', 'dm-row');
  const sent = h('span', 'tag t-sky', '');
  const dup = h('span', 'tag t-soft', '');
  const hops = h('span', 'tag t-mint', '');
  stats.append(sent, dup, hops);
  sent.textContent = '0 messages';
  dup.textContent = '0 duplicates dropped';
  hops.textContent = '1 device has it';
  body.append(svg, stats);
  const rnd = utils.createSeededRandom(7);
  const pts: { x: number; y: number; nb: number[]; c: SVGCircleElement }[] = [];
  const cols = 8, rows = 3;
  for (let r = 0; r < rows; r++) for (let c = 0; c < cols; c++) {
    pts.push({ x: 40 + c * ((W - 80) / (cols - 1)) + rnd(-18, 18), y: 40 + r * ((H - 80) / (rows - 1)) + rnd(-16, 16), nb: [], c: null as unknown as SVGCircleElement });
  }
  pts.forEach((p, i) => {
    const near = pts.map((q, j) => ({ j, d: Math.hypot(p.x - q.x, p.y - q.y) })).filter((o) => o.j !== i).sort((x, y) => x.d - y.d).slice(0, 3);
    near.forEach(({ j }) => { if (!p.nb.includes(j)) { p.nb.push(j); pts[j].nb.push(i); } });
  });
  const drawn = new Set<string>();
  pts.forEach((p, i) => p.nb.forEach((j) => {
    const k = Math.min(i, j) + '-' + Math.max(i, j);
    if (drawn.has(k)) return;
    drawn.add(k);
    const l = document.createElementNS(NS, 'line');
    l.setAttribute('x1', String(p.x)); l.setAttribute('y1', String(p.y)); l.setAttribute('x2', String(pts[j].x)); l.setAttribute('y2', String(pts[j].y));
    l.setAttribute('class', 'd-line');
    svg.append(l);
  }));
  pts.forEach((p) => {
    const c = document.createElementNS(NS, 'circle');
    c.setAttribute('cx', String(p.x)); c.setAttribute('cy', String(p.y)); c.setAttribute('r', '9');
    c.setAttribute('class', 'd-box');
    svg.append(c);
    p.c = c;
  });
  let run = 0;
  const play = async () => {
    const my = ++run;
    svg.querySelectorAll('.dm-pkt').forEach((e) => e.remove());
    pts.forEach((p) => p.c.setAttribute('class', 'd-box'));
    const seen = new Set<number>([0]);
    pts[0].c.setAttribute('class', 'd-box acc c-coral');
    let frontier = [0];
    let nSent = 0, nDup = 0, nHop = 0;
    const show = () => { sent.textContent = `${nSent} messages`; dup.textContent = `${nDup} duplicates dropped`; hops.textContent = `${nHop} hops · ${seen.size}/${pts.length} devices`; };
    show();
    while (frontier.length) {
      if (my !== run) return;
      nHop++;
      const next: number[] = [];
      const flights: Promise<void>[] = [];
      for (const i of frontier) for (const j of pts[i].nb) {
        nSent++;
        const fresh = !seen.has(j) && !next.includes(j);
        if (fresh) next.push(j); else nDup++;
        if (reduce) continue;
        const dot = document.createElementNS(NS, 'circle');
        dot.setAttribute('r', '4');
        dot.setAttribute('class', 'd-dot dm-pkt ' + (fresh ? 'c-coral' : 'c-mute'));
        dot.setAttribute('cx', String(pts[i].x)); dot.setAttribute('cy', String(pts[i].y));
        svg.append(dot);
        const o = { x: pts[i].x, y: pts[i].y };
        flights.push(new Promise((res) => animate(o, {
          x: pts[j].x, y: pts[j].y, duration: 520, ease: 'inOutQuad',
          onUpdate: () => { dot.setAttribute('cx', String(o.x)); dot.setAttribute('cy', String(o.y)); },
          onComplete: () => { dot.remove(); res(); },
        })));
      }
      await Promise.all(flights);
      if (my !== run) return;
      next.forEach((j) => { seen.add(j); pts[j].c.setAttribute('class', 'd-box acc c-mint'); });
      if (!reduce) animate(next.map((j) => pts[j].c), { r: [14, 9], duration: 380, ease: 'outBack(3)' });
      show();
      frontier = next;
      if (!reduce) await sleep(140);
    }
  };
  whenSeen(body, () => void play());
  return () => void play();
};

/* =========================================================================================
   shamir: 3-of-5 over GF(2^8) with the AES polynomial, as hoi-crypto/src/shamir.rs
   ========================================================================================= */
function gmul(a: number, b: number): number {
  let p = 0;
  while (b) { if (b & 1) p ^= a; a = (a << 1) ^ (a & 0x80 ? 0x11b : 0); a &= 0xff; b >>= 1; }
  return p;
}
function ginv(a: number): number { let r = 1; for (let i = 0; i < 254; i++) r = gmul(r, a); return r; }
function split(secret: Uint8Array, n: number, k: number): { x: number; y: Uint8Array }[] {
  const coeffs = Array.from(secret, (s) => [s, ...crypto.getRandomValues(new Uint8Array(k - 1))]);
  return Array.from({ length: n }, (_, i) => {
    const x = i + 1;
    return { x, y: Uint8Array.from(coeffs, (c) => c.reduceRight((acc, co) => gmul(acc, x) ^ co, 0)) };
  });
}
function combine(shares: { x: number; y: Uint8Array }[]): Uint8Array {
  const len = shares[0].y.length;
  const out = new Uint8Array(len);
  for (let i = 0; i < shares.length; i++) {
    let li = 1;
    for (let j = 0; j < shares.length; j++) if (i !== j) li = gmul(li, gmul(shares[j].x, ginv(shares[j].x ^ shares[i].x)));
    for (let b = 0; b < len; b++) out[b] ^= gmul(shares[i].y[b], li);
  }
  return out;
}
const shamirDemo: Demo = (body, reduce) => {
  const NAMES = ['mira', 'tobi', 'lea', 'jonas', 'p. tran'];
  const keyK = h('div', 'dm-k', 'Root key (16 bytes shown)');
  const key = h('div', 'dm-hex');
  const row = h('div', 'dm-row');
  const outK = h('div', 'dm-k', 'Rebuilt from the pieces you pick');
  const out = h('div', 'dm-hex', '—');
  const verdict = h('span', 'tag t-soft', 'pick three friends');
  body.append(keyK, key, row, outK, out, verdict);
  row.style.setProperty('margin', '12px 0');
  let secret = new Uint8Array(16);
  let shares: { x: number; y: Uint8Array }[] = [];
  const picked = new Set<number>();
  const cards = NAMES.map((n, i) => {
    const b = btn(`${n} · piece ${i + 1}`);
    b.setAttribute('aria-pressed', 'false');
    b.addEventListener('click', () => {
      if (picked.has(i)) picked.delete(i); else picked.add(i);
      b.setAttribute('aria-pressed', String(picked.has(i)));
      update();
    });
    row.append(b);
    return b;
  });
  const update = () => {
    if (picked.size < 2) { out.textContent = '—'; verdict.textContent = 'pick three friends'; verdict.className = 'tag t-soft'; return; }
    const got = combine([...picked].map((i) => shares[i]));
    const ok = hex(got) === hex(secret);
    out.textContent = hex(got);
    verdict.textContent = ok ? `✓ ${picked.size} pieces rebuild the key` : picked.size < 3 ? '✗ two pieces give a random-looking value: they say nothing about the key' : '✗';
    verdict.className = 'tag ' + (ok ? 't-mint' : 't-coral');
    if (!reduce) animate(out, { opacity: [0.2, 1], duration: 400 });
  };
  const deal = () => {
    secret = crypto.getRandomValues(new Uint8Array(16));
    shares = split(secret, 5, 3);
    key.textContent = hex(secret);
    picked.clear();
    cards.forEach((c) => c.setAttribute('aria-pressed', 'false'));
    update();
    if (!reduce) animate(cards, { y: [-10, 0], opacity: [0, 1], delay: stagger(70), duration: 500, ease: 'outBack(2)' });
  };
  deal();
  return deal;
};

/* =========================================================================================
   scatter: seal → transform → k data + parity pieces → lose some → rebuild from any k
   ========================================================================================= */
const scatterDemo: Demo = (body, reduce) => {
  const K = 4, M = 8;
  const steps = h('div', 'dm-row');
  ['object', 'sealed', 'all-or-nothing', 'k = 4 data pieces', '+ 4 parity'].forEach((t, i) => {
    const s = h('span', 'tag ' + ['t-soft', 't-lilac', 't-lilac', 't-mint', 't-sky'][i], t);
    steps.append(s);
  });
  const pieces = h('div', 'dm-row');
  const cells = Array.from({ length: M }, (_, i) => {
    const c = h('span', 'dm-cell on', String(i));
    c.style.setProperty('--c', i < K ? 'var(--mint)' : 'var(--sky)');
    pieces.append(c);
    return c;
  });
  const ctl = h('div', 'dm-row');
  const lose = btn('Take a node offline');
  const reset = btn('Bring them back');
  ctl.append(lose, reset);
  const verdict = h('span', 'tag t-mint', '8 of 8 pieces reachable: any 4 rebuild the object');
  body.append(steps, pieces, ctl, verdict);
  pieces.style.setProperty('margin', '14px 0');
  let lost = new Set<number>();
  const show = () => {
    const left = M - lost.size;
    verdict.textContent = left >= K ? `${left} of ${M} pieces reachable: any ${K} rebuild the object` : `${left} of ${M}: fewer than ${K}, and the transform means no part of it can be read`;
    verdict.className = 'tag ' + (left >= K ? 't-mint' : 't-coral');
    lose.disabled = lost.size >= M;
  };
  lose.addEventListener('click', () => {
    const alive = cells.map((_, i) => i).filter((i) => !lost.has(i));
    const i = alive[Math.floor(Math.random() * alive.length)];
    lost.add(i);
    cells[i].className = 'dm-cell lost';
    if (!reduce) animate(cells[i], { scale: [1, 0.6, 1], rotate: [0, 30, 0], duration: 500 });
    show();
  });
  const play = () => {
    lost = new Set();
    cells.forEach((c) => { c.className = 'dm-cell on'; });
    show();
    if (!reduce) createTimeline()
      .add(Array.from(steps.children), { opacity: [0, 1], x: [-10, 0], delay: stagger(140), duration: 400 })
      .add(cells, { scale: [0, 1], delay: stagger(60), duration: 420, ease: 'outBack(3)' }, '-=200');
  };
  reset.addEventListener('click', play);
  whenSeen(body, play);
  return play;
};

/* =========================================================================================
   ladder: newcomer → member → established → trusted, and fading back
   ========================================================================================= */
const LADDER: { at: number; text: string }[] = [
  { at: 0, text: 'A new key does its identity work (8 bits of Argon2id by default). Newcomer: text only, 2 posts a day.' },
  { at: 0, text: 'An established member reads two of its posts and vouches. One vouch of the 2 needed.' },
  { at: 1, text: 'A second vouch. Member: links allowed, votes count.' },
  { at: 2, text: '14 days later: established. Its vouches, reviews and vetoes now count.' },
  { at: 3, text: '5 live posts in 90 days that people answered well (score 10): trusted. Images, audio, video.' },
  { at: 2, text: 'It stops posting; its contributions age out. Back to established: trusted has to be kept.' },
];
const ladderDemo: Demo = (body, reduce) => {
  const RUNGS = ['Newcomer', 'Member', 'Established', 'Trusted'];
  const rungs = h('div', 'dm-stagelist');
  const rows = RUNGS.map((r, i) => {
    const d = h('div');
    d.append(h('span', 'n', String(i)), h('span', '', r), h('span', 'r', ''));
    rungs.prepend(d);
    return d;
  });
  const text = h('p', 'wk-stage-cap', '');
  text.style.setProperty('border', '0');
  text.style.setProperty('padding', '12px 0 0');
  body.append(rungs, text);
  let run = 0;
  const play = async () => {
    const my = ++run;
    for (const step of LADDER) {
      if (my !== run) return;
      rows.forEach((r, i) => { r.className = i === step.at ? 'pass' : ''; (r.lastChild as HTMLElement).textContent = i === step.at ? '● here' : ''; });
      text.textContent = step.text;
      if (!reduce) animate(rows[step.at], { x: [-10, 0], duration: 420, ease: 'outBack(2)' });
      await sleep(reduce ? 0 : 1900);
      if (reduce) break;
    }
  };
  whenSeen(body, () => void play());
  return () => void play();
};

/* =========================================================================================
   expiry: records fading at expires_at; a removal leaving only its id and reason
   ========================================================================================= */
const expiryDemo: Demo = (body, reduce) => {
  const items = [
    { t: 'Meetup Thursday', life: 0.35, removed: false },
    { t: 'Field summary of the audit', life: 0.8, removed: false },
    { t: 'Spam link (removed by a mod)', life: 0.9, removed: true },
    { t: 'Data-retention bill, 2nd draft', life: 0.6, removed: false },
  ];
  const now = h('div', 'dm-k', 'day 0');
  const list = h('div', 'dm-stagelist');
  const rows = items.map((it) => {
    const r = h('div');
    const bar = h('span', 'r', '');
    r.append(h('span', 'n', '◆'), h('span', '', it.t), bar);
    list.append(r);
    return r;
  });
  body.append(now, list);
  let run = 0;
  const play = async () => {
    const my = ++run;
    rows.forEach((r, i) => { r.className = 'pass'; (r.children[1] as HTMLElement).textContent = items[i].t; (r.lastChild as HTMLElement).textContent = 'live'; r.style.removeProperty('opacity'); });
    const days = 180;
    for (let d = 0; d <= days; d += reduce ? days : 6) {
      if (my !== run) return;
      now.textContent = `day ${d} of a 180-day window`;
      items.forEach((it, i) => {
        const r = rows[i];
        if (it.removed && d >= 3 && r.className !== 'fail') {
          r.className = 'fail';
          (r.children[1] as HTMLElement).textContent = 'Poof. (id and reason kept; body deleted after the 72-hour contest window)';
          (r.lastChild as HTMLElement).textContent = 'tombstone';
        }
        if (!it.removed && d / days >= it.life && r.className === 'pass') {
          r.className = '';
          (r.lastChild as HTMLElement).textContent = 'expired · deleted';
          if (!reduce) animate(r, { opacity: [1, 0.35], duration: 500 });
          else r.style.setProperty('opacity', '0.35');
        }
      });
      if (!reduce) await sleep(110);
    }
  };
  whenSeen(body, () => void play());
  return () => void play();
};

/* =========================================================================================
   ratelimit: an established member's posts against (1 hour, 12) and (1 day, 48)
   ========================================================================================= */
const ratelimitDemo: Demo = (body, reduce) => {
  const HOUR = 3600;
  const info = h('div', 'dm-row');
  info.append(h('span', 'tag t-sky', 'windows for posts: 12 per hour, 48 per day'), h('span', 'tag t-soft', 'newcomers: 2 per day'));
  const lane = h('div', 'dm-row');
  const meter = h('div', 'dm-k', '');
  const ctl = h('div', 'dm-row');
  const one = btn('Post now');
  const burst = btn('Post 10 at once');
  const wait = btn('Wait 20 minutes');
  ctl.append(one, burst, wait);
  body.append(info, lane, meter, ctl);
  lane.style.setProperty('margin', '14px 0 8px');
  let t = 0;
  let kept: number[] = [];
  const windowOk = (times: number[], at: number, w: number, max: number) => {
    const near = times.filter((x) => Math.abs(x - at) < w).concat(at).sort((a, b) => a - b);
    return near.filter((s) => s <= at).every((s) => near.filter((x) => x >= s && x < s + w).length <= max);
  };
  const post = () => {
    const ok = windowOk(kept, t, HOUR, 12) && windowOk(kept, t, 24 * HOUR, 48);
    if (ok) kept.push(t);
    const c = h('span', 'dm-cell ' + (ok ? 'on' : 'bad'), ok ? '✓' : '✗');
    c.title = ok ? 'accepted' : 'dropped: over the hourly window, the key gets a strike';
    lane.append(c);
    while (lane.children.length > 28) lane.firstChild?.remove();
    if (!reduce) animate(c, { scale: [0, 1], duration: 300, ease: 'outBack(3)' });
    t += 20;
    const inHour = kept.filter((x) => t - x < HOUR).length;
    meter.textContent = `in the last hour: ${inHour} / 12 · in the last day: ${kept.filter((x) => t - x < 24 * HOUR).length} / 48`;
  };
  one.addEventListener('click', post);
  burst.addEventListener('click', async () => { for (let i = 0; i < 10; i++) { post(); if (!reduce) await sleep(70); } });
  wait.addEventListener('click', () => { t += 20 * 60; meter.textContent = `+20 minutes · in the last hour: ${kept.filter((x) => t - x < HOUR).length} / 12`; });
  const reset = () => { t = 0; kept = []; lane.replaceChildren(); meter.textContent = 'in the last hour: 0 / 12'; };
  reset();
  return reset;
};

const DEMOS: Record<string, Demo> = {
  hash: hashDemo, stamp: stampDemo, pipeline: pipelineDemo, reconcile: reconcileDemo, gossip: gossipDemo,
  shamir: shamirDemo, scatter: scatterDemo, ladder: ladderDemo, expiry: expiryDemo, ratelimit: ratelimitDemo,
};

export function mount(stage: HTMLElement, reduce: boolean): void {
  const name = stage.dataset.demo ?? '';
  const demo = DEMOS[name];
  if (!demo) return;
  const title = stage.dataset.title ?? name;
  const head = h('div', 'wk-stage-h');
  const t = h('span', 't');
  t.append(h('b', '', 'live · '), title);
  head.append(t);
  const body = h('div', 'wk-stage-body');
  const cap = stage.querySelector('.wk-stage-cap');
  stage.replaceChildren(head, body);
  if (cap) stage.append(cap);
  const replay = demo(body, reduce, stage);
  if (replay) {
    const b = btn('↻ Replay');
    b.addEventListener('click', () => replay());
    head.append(b);
  }
}
