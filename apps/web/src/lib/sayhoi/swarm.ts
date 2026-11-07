// The first-run field (every animation teaches something), drawn like the site's live
// network: people are 30px dots with their initials and dashed links, yours is the lilac one,
// the coral one is a post. In act 1 the field is a touch pad: people scatter from your fingertip
// and spring back to their place, like the site's community bubbles. In act 2 each identity try
// flies from you to the bouncer (the site's sun shield: every device's check); tries short of the
// work bounce off, the one that carries it goes through with a thump, and everyone gets a ✓ as
// they check it. Canvas 2D on springs, no engine.
//
// With reduced motion nothing flies, springs, ripples or sparks: dots shimmer where you touch,
// tries land at once.
import type { IdentityTry } from '@hoi/core-api';

export type Mode = 'stir' | 'work' | 'done';
export type Hand = 'mouse' | 'pen' | 'touch';

export interface Arrival extends IdentityTry {
  passed: boolean;
}

interface Peer {
  hx: number;
  hy: number;
  x: number;
  y: number;
  vx: number;
  vy: number;
  name: string;
  phase: number;
  /** 0..1, where you last touched (reduced motion) */
  shimmer: number;
  /** when this peer checks the stamp (ms, performance.now()), 0 = not yet */
  checkAt: number;
}

interface Flight {
  t: IdentityTry;
  passed: boolean;
  t0: number;
  dur: number;
  sx: number;
  sy: number;
  ex: number;
  ey: number;
  arc: number;
  /** after it reached the bouncer */
  x: number;
  y: number;
  vx: number;
  vy: number;
  arrived: number;
}

interface Spark {
  x: number;
  y: number;
  vx: number;
  vy: number;
  t0: number;
  life: number;
  color: string;
}

interface Colors {
  bg: string;
  card: string;
  ink: string;
  line: string;
  mute: string;
  soft: string;
  coral: string;
  cobalt: string;
  lilac: string;
  sun: string;
  mint: string;
}

const K = 60; // spring stiffness (1/s²): bouncy, like the site's chips
const DAMP = 7; // damping (1/s)
const REACH = 92; // how far your finger pushes (px)
const PUSH = 2600; // px/s² at the finger
const MAX_FLIGHTS = 10;
/** People, you, posts: the site's 30px, 40px and 14px dots. */
const PEER_R = 15;
const YOU_R = 20;
const POST_R = 7;
const NAMES = ['mi', 'jw', 'sk', 'ra', 'pt', 'lh', 'tk', 'an', 'mo', 'kai', 'ida', 'rob', 'emi', 'nur', 'jo', 'el', 'sam', 'lu'];
/** The progress chip in act 1's top left corner (px). */
const CHIP = { w: 190, h: 72 };
const LAUNCH_GAP = 70; // ms between flights; extra tries are counted, not all drawn
/** Room kept at the bottom for the hash strip after act 1. */
const STRIP = 92;

const ease = (x: number) => 0.5 - Math.cos(Math.PI * x) / 2;

export class Swarm {
  readonly #c: HTMLCanvasElement;
  readonly #ctx: CanvasRenderingContext2D;
  readonly #reduced: boolean;
  readonly #onArrive: (a: Arrival) => void;
  #colors: Colors;
  #w = 0;
  #h = 0;
  #dpr = 1;
  #peers: Peer[] = [];
  #links: [number, number][] = [];
  #touch: { x: number; y: number; pressed: boolean; hand: Hand } | null = null;
  #cursor: { x: number; y: number } | null = null;
  #flights: Flight[] = [];
  #lastLaunch = 0;
  #mode: Mode = 'stir';
  #you = { x: 0, y: 0, tx: 0, ty: 0 };
  /** the bouncer: the site's sun shield */
  #gate = { x: 0, y: 0, r: 40, shake: 0, open: 0 };
  #ground: CanvasGradient | null = null;
  #need = 10;
  #raf = 0;
  #last = 0;
  #ro: ResizeObserver;

  #ripples: { x: number; y: number; t: number }[] = [];
  #sparks: Spark[] = [];
  #shakeAt = 0;

  constructor(canvas: HTMLCanvasElement, opts: { reduced: boolean; onArrive: (a: Arrival) => void }) {
    this.#c = canvas;
    const ctx = canvas.getContext('2d');
    if (!ctx) throw new Error('no 2d canvas');
    this.#ctx = ctx;
    this.#reduced = opts.reduced;
    this.#onArrive = opts.onArrive;
    this.#colors = this.#readColors();
    this.#ro = new ResizeObserver(() => this.#layout());
    this.#ro.observe(canvas);
    this.#layout();
    this.#raf = requestAnimationFrame(this.#frame);
  }

  destroy(): void {
    cancelAnimationFrame(this.#raf);
    this.#ro.disconnect();
    this.#flights = [];
  }

  get size(): { w: number; h: number } {
    return { w: this.#w, h: this.#h };
  }

  setMode(m: Mode, need = this.#need): void {
    const moved = (m === 'stir') !== (this.#mode === 'stir');
    this.#mode = m;
    this.#need = need;
    this.#place();
    // the swarm makes room: peers spring to new places
    if (moved) this.#seed();
  }

  /** Where your finger or pointer is over the field (CSS px), or null when it left. */
  touch(p: { x: number; y: number; pressed?: boolean; hand?: Hand } | null): void {
    this.#touch = p && { x: p.x, y: p.y, pressed: !!p.pressed, hand: p.hand ?? 'mouse' };
  }

  /** The keyboard cursor, or null when the field isn't focused. */
  cursor(p: { x: number; y: number } | null): void {
    this.#cursor = p;
  }

  /** A tap: a ripple, and a burst that scatters the dots nearby. */
  poke(x: number, y: number): void {
    if (!this.#reduced) this.#ripples.push({ x, y, t: performance.now() });
    for (const p of this.#peers) {
      const dx = p.x - x;
      const dy = p.y - y;
      const d = Math.hypot(dx, dy) || 1;
      if (d > REACH * 1.6) continue;
      const f = 1 - d / (REACH * 1.6);
      if (this.#reduced) p.shimmer = Math.max(p.shimmer, f);
      else {
        p.vx += (dx / d) * f * 520;
        p.vy += (dy / d) * f * 520;
      }
    }
  }

  /** One identity try. Returns whether it got its own flight (the rest are counted, not drawn). */
  launch(t: IdentityTry): boolean {
    const passed = t.bits >= this.#need;
    const now = performance.now();
    if (this.#reduced) {
      this.#onArrive({ ...t, passed });
      if (passed) this.#stamped(now);
      return true;
    }
    const flying = this.#flights.filter((f) => !f.arrived).length;
    if (!passed && (flying >= MAX_FLIGHTS || now - this.#lastLaunch < LAUNCH_GAP)) return false;
    this.#lastLaunch = now;
    const spread = this.#gate.r * 0.55;
    this.#flights.push({
      t,
      passed,
      t0: now,
      dur: 620 + Math.random() * 260,
      sx: this.#you.x,
      sy: this.#you.y,
      ex: this.#gate.x - this.#gate.r - POST_R + 2,
      ey: this.#gate.y + (Math.random() * 2 - 1) * spread,
      arc: (Math.random() * 2 - 1) * Math.min(70, this.#floor * 0.2),
      x: 0,
      y: 0,
      vx: 0,
      vy: 0,
      arrived: 0,
    });
    return true;
  }

  /** Sparks flying out from one spot (the stamp landing). */
  #burst(x: number, y: number, n: number, colors: string[], speed: number) {
    if (this.#reduced) return;
    const now = performance.now();
    for (let i = 0; i < n; i++) {
      const a = Math.random() * Math.PI * 2;
      const v = speed * (0.4 + Math.random() * 0.8);
      this.#sparks.push({ x, y, vx: Math.cos(a) * v, vy: Math.sin(a) * v, t0: now, life: 500 + Math.random() * 500, color: colors[i % colors.length] });
    }
  }

  // ------------------------------------------------------------------ layout

  #readColors(): Colors {
    const s = getComputedStyle(this.#c);
    const v = (n: string, f: string) => s.getPropertyValue(`--${n}`).trim() || f;
    return {
      bg: v('bg', '#15151A'),
      card: v('card', '#1E1E24'),
      ink: v('ink', '#F4F2EA'),
      line: v('line', '#34343B'),
      mute: v('mute', '#A3A39A'),
      soft: v('soft', '#26262D'),
      coral: v('coral', '#FF6B4A'),
      cobalt: v('cobalt', '#8AA0FF'),
      lilac: v('lilac', '#DCD2FF'),
      sun: v('sun', '#FFE16A'),
      mint: v('mint', '#BFF0D0'),
    };
  }

  #layout() {
    const rect = this.#c.getBoundingClientRect();
    const w = Math.max(1, Math.round(rect.width));
    const h = Math.max(1, Math.round(rect.height));
    this.#dpr = Math.min(2, window.devicePixelRatio || 1);
    this.#c.width = Math.round(w * this.#dpr);
    this.#c.height = Math.round(h * this.#dpr);
    if (w === this.#w && h === this.#h && this.#peers.length) return;
    this.#w = w;
    this.#h = h;
    this.#colors = this.#readColors();
    // a soft light in the middle of the night sky
    this.#ground = this.#ctx.createRadialGradient(w / 2, h / 2, 0, w / 2, h / 2, Math.hypot(w, h) * 0.55);
    this.#ground.addColorStop(0, this.#colors.card);
    this.#ground.addColorStop(1, this.#colors.bg);
    this.#place(true);
    this.#seed();
  }

  /** The field minus the strip's room once the work starts. */
  get #floor(): number {
    return this.#mode === 'stir' ? this.#h : Math.max(this.#h * 0.5, this.#h - STRIP);
  }

  /** You in the middle while stirring, then on the left facing the bouncer. */
  #place(jump = false) {
    const you = this.#you;
    const mid = this.#floor / 2;
    you.tx = this.#mode === 'stir' ? this.#w / 2 : Math.max(48, this.#w * 0.16);
    you.ty = mid;
    this.#gate.r = Math.round(Math.min(52, Math.max(34, Math.min(this.#w, this.#floor) * 0.12)));
    this.#gate.x = this.#w - this.#gate.r - Math.max(16, this.#w * 0.05);
    this.#gate.y = mid;
    if (jump || this.#reduced) {
      you.x = you.tx;
      you.y = you.ty;
    }
  }

  /** Where a person may stand: clear of you, and of the progress chip in act 1 or of the
   * bouncer and the posts' way to it afterwards. */
  #free(x: number, y: number): boolean {
    if (Math.hypot(x - this.#you.tx, y - this.#you.ty) < YOU_R + PEER_R + 22) return false;
    if (this.#mode === 'stir') return !(x < CHIP.w + PEER_R && y < CHIP.h + PEER_R);
    if (Math.hypot(x - this.#gate.x, y - this.#gate.y) < this.#gate.r + PEER_R + 16) return false;
    // the corner the stamp lands in
    if (x > this.#w - 250 && y < 74) return false;
    // the posts' lane, from you to the bouncer
    return !(x > this.#you.tx && x < this.#gate.x && Math.abs(y - this.#gate.y) < PEER_R + 10);
  }

  /** Peers on a jittered grid, moved somewhere free when their spot isn't; each linked to its
   * two nearest. Peers already on the field keep where they are and spring to their new places. */
  #seed() {
    const w = this.#w;
    const h = this.#floor;
    // a dinner table, not a stadium: one person per ~125 × 125 px
    const n = Math.round(Math.min(18, Math.max(8, (w * this.#h) / 15500)));
    const cols = Math.max(3, Math.round(Math.sqrt((n * w) / h)));
    const rows = Math.ceil(n / cols);
    const old = this.#peers;
    const at = (lo: number, hi: number) => lo + Math.random() * Math.max(0, hi - lo);
    this.#peers = [];
    for (let i = 0; i < n; i++) {
      const c = i % cols;
      const r = Math.floor(i / cols);
      let hx = (c + 0.5) * (w / cols) + (Math.random() * 2 - 1) * (w / cols) * 0.3;
      let hy = (r + 0.5) * (h / rows) + (Math.random() * 2 - 1) * (h / rows) * 0.3;
      const edge = PEER_R + 6;
      hx = Math.min(w - edge, Math.max(edge, hx));
      hy = Math.min(h - edge, Math.max(edge, hy));
      // away from the others too, so names never overlap
      const crowded = (x: number, y: number) => this.#peers.some((q) => Math.hypot(q.hx - x, q.hy - y) < PEER_R * 2 + 14);
      for (let tries = 0; tries < 60 && (!this.#free(hx, hy) || crowded(hx, hy)); tries++) {
        hx = at(edge, w - edge);
        hy = at(edge, h - edge);
      }
      const prev = old[i];
      this.#peers.push({
        hx,
        hy,
        x: prev && !this.#reduced ? prev.x : hx,
        y: prev && !this.#reduced ? prev.y : hy,
        vx: 0,
        vy: 0,
        name: prev?.name ?? NAMES[i % NAMES.length],
        phase: Math.random() * Math.PI * 2,
        shimmer: 0,
        checkAt: prev?.checkAt ?? 0,
      });
    }
    const links = new Set<string>();
    this.#links = [];
    this.#peers.forEach((a, i) => {
      this.#peers
        .map((b, j) => ({ j, d: Math.hypot(a.hx - b.hx, a.hy - b.hy) }))
        .filter((o) => o.j !== i)
        .sort((p, q) => p.d - q.d)
        .slice(0, 2)
        .forEach(({ j }) => {
          const key = Math.min(i, j) + '-' + Math.max(i, j);
          if (links.has(key)) return;
          links.add(key);
          this.#links.push([i, j]);
        });
    });
  }

  // ------------------------------------------------------------------ the loop

  #frame = (now: number) => {
    const dt = Math.min(1 / 30, (now - (this.#last || now)) / 1000);
    this.#last = now;
    this.#step(now, dt);
    this.#draw(now);
    this.#raf = requestAnimationFrame(this.#frame);
  };

  #stamped(now: number) {
    if (this.#gate.open) return;
    this.#gate.open = 1;
    this.#shakeAt = now;
    this.#burst(this.#gate.x, this.#gate.y, 28, [this.#colors.mint, this.#colors.sun, this.#colors.coral, this.#colors.lilac], 240);
    // the record travels out: nearer peers check it first
    for (const p of this.#peers) {
      if (!p.checkAt) p.checkAt = now + (this.#reduced ? 0 : Math.hypot(p.hx - this.#gate.x, p.hy - this.#gate.y) * 1.4);
    }
  }

  #step(now: number, dt: number) {
    const hands = [this.#touch, this.#cursor].filter((p): p is { x: number; y: number } => p !== null);
    for (const p of this.#peers) {
      if (this.#reduced) {
        for (const h of hands) p.shimmer = Math.max(p.shimmer, 1 - Math.hypot(p.x - h.x, p.y - h.y) / REACH);
        p.shimmer = Math.max(0, p.shimmer - dt * 1.6);
        continue;
      }
      const hx = p.hx + Math.sin(now / 1700 + p.phase) * 3;
      const hy = p.hy + Math.cos(now / 2100 + p.phase) * 3;
      let ax = -K * (p.x - hx) - DAMP * p.vx;
      let ay = -K * (p.y - hy) - DAMP * p.vy;
      for (const h of hands) {
        const dx = p.x - h.x;
        const dy = p.y - h.y;
        const d = Math.hypot(dx, dy) || 1;
        if (d < REACH) {
          const f = (1 - d / REACH) ** 2 * PUSH;
          ax += (dx / d) * f;
          ay += (dy / d) * f;
        }
      }
      p.vx += ax * dt;
      p.vy += ay * dt;
      p.x += p.vx * dt;
      p.y += p.vy * dt;
    }
    const you = this.#you;
    if (!this.#reduced) {
      you.x += (you.tx - you.x) * Math.min(1, dt * 5);
      you.y += (you.ty - you.y) * Math.min(1, dt * 5);
    }
    this.#gate.shake = Math.max(0, this.#gate.shake - dt * 4);

    for (const f of this.#flights) {
      if (!f.arrived) {
        const k = (now - f.t0) / f.dur;
        if (k < 1) {
          const e = ease(k);
          f.x = f.sx + (f.ex - f.sx) * e;
          f.y = f.sy + (f.ey - f.sy) * e + Math.sin(Math.PI * e) * f.arc;
          continue;
        }
        f.arrived = now;
        f.x = f.ex;
        f.y = f.ey;
        if (f.passed) {
          // through: into the shield
          f.vx = (this.#gate.x - f.x) * 2.5;
          f.vy = (this.#gate.y - f.y) * 2.5;
          this.#stamped(now);
        } else {
          f.vx = -(90 + Math.random() * 140);
          f.vy = (Math.random() * 2 - 1) * 160;
          this.#gate.shake = 1;
        }
        this.#onArrive({ ...f.t, passed: f.passed });
        continue;
      }
      if (!f.passed) f.vy += 520 * dt;
      f.x += f.vx * dt;
      f.y += f.vy * dt;
    }
    this.#flights = this.#flights.filter((f) => !f.arrived || now - f.arrived < 700);

    for (const s of this.#sparks) {
      s.vy += 60 * dt;
      s.vx *= 1 - dt * 1.5;
      s.x += s.vx * dt;
      s.y += s.vy * dt;
    }
    this.#sparks = this.#sparks.filter((s) => now - s.t0 < s.life);
    this.#ripples = this.#ripples.filter((r) => now - r.t < 520);
  }

  // ------------------------------------------------------------------ drawing

  #dot(x: number, y: number, r: number, fill: string, stroke: string, width = 2) {
    const g = this.#ctx;
    g.beginPath();
    g.arc(x, y, r, 0, Math.PI * 2);
    g.fillStyle = fill;
    g.fill();
    g.lineWidth = width;
    g.strokeStyle = stroke;
    g.stroke();
  }

  #draw(now: number) {
    const g = this.#ctx;
    const C = this.#colors;
    g.setTransform(this.#dpr, 0, 0, this.#dpr, 0, 0);
    g.globalAlpha = 1;
    g.fillStyle = this.#ground ?? C.bg;
    g.fillRect(0, 0, this.#w, this.#h);
    // the stamp lands with a thump
    const shake = this.#reduced ? 0 : Math.max(0, 1 - (now - this.#shakeAt) / 380);
    if (shake > 0) g.translate((Math.random() * 2 - 1) * 5 * shake, (Math.random() * 2 - 1) * 5 * shake);

    // links: the site's dashed lines
    g.lineWidth = 2;
    g.lineCap = 'round';
    g.strokeStyle = C.line;
    g.setLineDash([4, 6]);
    g.beginPath();
    for (const [a, b] of this.#links) {
      g.moveTo(this.#peers[a].x, this.#peers[a].y);
      g.lineTo(this.#peers[b].x, this.#peers[b].y);
    }
    g.stroke();
    g.setLineDash([]);

    // a tap sends a ripple out
    for (const r of this.#ripples) {
      const k = (now - r.t) / 520;
      g.globalAlpha = 1 - k;
      g.lineWidth = 2.5;
      g.strokeStyle = C.sun;
      g.beginPath();
      g.arc(r.x, r.y, 10 + k * 46, 0, Math.PI * 2);
      g.stroke();
    }
    g.globalAlpha = 1;

    // people: card dots with their initials; a ✓ once they've checked your stamp
    g.textAlign = 'center';
    g.textBaseline = 'middle';
    for (const p of this.#peers) {
      const lit = this.#reduced && p.shimmer > 0.05;
      this.#dot(p.x, p.y, PEER_R, lit ? C.soft : C.card, lit ? C.sun : C.ink);
      g.fillStyle = C.ink;
      g.font = '800 10px Gabarito, system-ui, sans-serif';
      g.fillText(p.name, p.x, p.y + 0.5);
      if (p.checkAt && now >= p.checkAt) {
        const pop = this.#reduced ? 1 : Math.min(1, (now - p.checkAt) / 260);
        const cx = p.x + PEER_R * 0.75;
        const cy = p.y - PEER_R * 0.75;
        g.beginPath();
        g.arc(cx, cy, 8 * pop, 0, Math.PI * 2);
        g.fillStyle = C.cobalt;
        g.fill();
        if (pop > 0.6) {
          g.fillStyle = '#FFFFFF';
          g.font = '900 10px Gabarito, system-ui, sans-serif';
          g.fillText('✓', cx, cy + 0.5);
        }
      }
    }

    // the bouncer: the site's sun shield; it wobbles when a try bounces, opens when one gets through
    if (this.#mode !== 'stir') {
      const gt = this.#gate;
      const wob = this.#reduced ? 0 : gt.shake;
      g.save();
      g.translate(gt.x, gt.y);
      g.rotate(Math.sin(now / 45) * 0.08 * wob);
      g.scale(1 + 0.1 * wob, 1 + 0.1 * wob);
      this.#dot(0, 0, gt.r, gt.open ? C.mint : C.sun, C.ink, 3);
      g.fillStyle = '#1A1A1A';
      const small = `800 ${Math.round(gt.r * 0.24)}px Gabarito, system-ui, sans-serif`;
      if (gt.open) {
        g.font = `900 ${Math.round(gt.r * 0.7)}px Gabarito, system-ui, sans-serif`;
        g.fillText('✓', 0, -gt.r * 0.12);
        g.font = small;
        g.fillText('through', 0, gt.r * 0.42);
      } else {
        g.font = small;
        g.fillText('needs', 0, -gt.r * 0.45);
        g.font = `900 ${Math.round(gt.r * 0.62)}px Gabarito, system-ui, sans-serif`;
        g.fillText(String(this.#need), 0, gt.r * 0.02);
        g.font = small;
        g.fillText('zero bits', 0, gt.r * 0.47);
      }
      g.restore();
    }

    // posts: the coral dots, each one try
    for (const f of this.#flights) {
      const gone = f.arrived ? (now - f.arrived) / 700 : 0;
      g.globalAlpha = Math.max(0, 1 - gone);
      const r = f.passed && f.arrived ? POST_R * Math.max(0.2, 1 - gone * 1.5) : POST_R;
      this.#dot(f.x, f.y, r, C.coral, C.ink);
    }
    g.globalAlpha = 1;

    for (const sp of this.#sparks) {
      const k = (now - sp.t0) / sp.life;
      g.globalAlpha = 1 - k;
      g.fillStyle = sp.color;
      g.beginPath();
      g.arc(sp.x, sp.y, 2.6 * (1 - k * 0.5), 0, Math.PI * 2);
      g.fill();
    }
    g.globalAlpha = 1;

    // you, with a slow breath around you
    const you = this.#you;
    if (!this.#reduced) {
      const k = (Math.sin(now / 650) + 1) / 2;
      g.globalAlpha = 0.18 + 0.2 * (1 - k);
      g.lineWidth = 2;
      g.strokeStyle = C.lilac;
      g.beginPath();
      g.arc(you.x, you.y, YOU_R + 6 + 6 * k, 0, Math.PI * 2);
      g.stroke();
      g.globalAlpha = 1;
    }
    this.#dot(you.x, you.y, YOU_R, C.lilac, C.ink);
    g.fillStyle = '#1A1A1A';
    g.font = '800 12px Gabarito, system-ui, sans-serif';
    g.fillText('you', you.x, you.y + 0.5);

    // your fingertip on the pad, and the keyboard's cursor
    const t = this.#touch;
    if (t) {
      const pad = t.hand === 'touch' || t.pressed;
      g.beginPath();
      if (pad) g.ellipse(t.x, t.y, t.hand === 'touch' ? 17 : 13, t.hand === 'touch' ? 22 : 17, -0.25, 0, Math.PI * 2);
      else g.arc(t.x, t.y, 14, 0, Math.PI * 2);
      if (pad) {
        g.globalAlpha = 0.16;
        g.fillStyle = C.sun;
        g.fill();
        g.globalAlpha = 0.85;
      }
      g.lineWidth = 2;
      g.strokeStyle = pad ? C.sun : C.mute;
      g.stroke();
      g.globalAlpha = 1;
    }
    if (this.#cursor) {
      g.beginPath();
      g.arc(this.#cursor.x, this.#cursor.y, 26, 0, Math.PI * 2);
      g.lineWidth = 2;
      g.setLineDash([4, 4]);
      g.strokeStyle = C.cobalt;
      g.stroke();
      g.setLineDash([]);
    }
    g.textBaseline = 'alphabetic';
  }
}
