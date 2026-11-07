// The playground's world: a floor plan of a simulated hoi network drawn like the pipes of a
// building. A few devices stand for thousands (the tanks hold the rest); pipes are connections
// and the dots flowing through them are messages. Everything here is made up except the shapes
// of what flows, which come from scenarios.ts.
import type { Fam, Pipe } from './scenarios';

export const W = 1600;
export const H = 1000;

export type Kind = 'you' | 'public' | 'member' | 'browser' | 'home' | 'device' | 'tank' | 'dht' | 'service';
export interface Actor {
  id: string;
  kind: Kind;
  label: string;
  sub: string;
  x: number;
  y: number;
  r: number;
  /** Drawn scale (animated by anime.js), 0 = hidden. */
  s: number;
  /** Pulse 0..1 (animated). */
  p: number;
  /** Shown only when a scenario uses it. */
  optional?: boolean;
  dim?: boolean;
  count?: number;
  info: string[];
}
export interface PipeLink {
  a: Actor;
  b: Actor;
  kind: Pipe | 'bg' | 'tank';
  /** Curvature offset in world units. */
  bend: number;
  /** 0..1 how much of it is built (animated). */
  grow: number;
  /** Highlighted by the scenario. */
  hot: number;
  length: number;
}
export interface Dot { link: PipeLink; t: number; dir: 1 | -1; speed: number; fam: Fam; label?: string; size: number; done?: () => void; ambient: boolean }

export const FAM_COLOR: Record<Fam, string> = {
  hello: '#FFE16A', disc: '#9FD0FF', sync: '#BFF0D0', pair: '#8AA0FF', carry: '#FFD2C2', rtc: '#FF8A70', priv: '#DCD2FF', local: '#F4F2EA',
};
export const FAM_LABEL: Record<Fam, string> = {
  hello: 'hello · bind', disc: 'catalog · nodes · find · dht', sync: 'rng · want · recs', pair: 'pairing · link', carry: 'carry · pieces', rtc: 'STUN · DTLS · QUIC · sig', priv: 'private · MLS · relay', local: 'local',
};
export const PIPE_LABEL: Record<PipeLink['kind'], string> = {
  rtc: 'WebRTC', quic: 'iroh QUIC', ws: 'WebSocket', udp: 'UDP', https: 'HTTPS', oob: 'out of band', direct: 'WebRTC direct', bg: 'link', tank: 'many links',
};

export interface Region { x: number; y: number; w: number; h: number; label: string; color: string }
export const REGIONS: Region[] = [
  { x: 470, y: 40, w: 660, h: 400, label: 'PUBLIC NODES · MAINLINE DHT AMONG THEM', color: '#FFE16A' },
  { x: 1170, y: 40, w: 400, h: 600, label: "MEMBERS' NODES · c/privacy", color: '#BFF0D0' },
  { x: 860, y: 680, w: 710, h: 290, label: 'BROWSERS AND PHONES', color: '#9FD0FF' },
  { x: 30, y: 520, w: 560, h: 450, label: 'YOUR HOME NETWORK', color: '#DCD2FF' },
  { x: 30, y: 40, w: 400, h: 440, label: 'OPTIONAL SERVICES', color: '#FFD2C2' },
];

const A = (id: string, kind: Kind, label: string, sub: string, x: number, y: number, r: number, info: string[], extra: Partial<Actor> = {}): Actor =>
  ({ id, kind, label, sub, x, y, r, s: 0, p: 0, info, ...extra });

export function buildWorld(): { actors: Map<string, Actor>; links: PipeLink[] } {
  const actors = new Map<string, Actor>();
  const add = (a: Actor) => actors.set(a.id, a);
  // Public nodes on a ring, the DHT drawn around them.
  const cx = 800, cy = 235, rx = 250, ry = 140;
  const PUBLIC_INFO = ['hoi-node --public', 'iroh QUIC hoi/sync/1 · WebRTC 7844', 'fx: carry find dht signal', 'carries open communities, signs nothing'];
  for (let i = 0; i < 8; i++) {
    const a = (i / 8) * Math.PI * 2 - Math.PI / 2;
    add(A(`p${i}`, 'public', i === 0 ? 'N' : `P${i}`, i === 0 ? 'public node from the link' : 'public node', cx + Math.cos(a) * rx, cy + Math.sin(a) * ry, 15, PUBLIC_INFO));
  }
  add(A('pubTank', 'tank', '1,278', 'more public nodes', cx, cy, 52, ['Simulated: the rest of the public nodes', 'each knows ≤ 32 others, dials 3', 'nodes lists pass on only what they reached'], { count: 1278 }));
  add(A('dht', 'dht', 'DHT', 'Mainline, no outside bootstrap', cx + rx + 40, cy + ry - 10, 16, ['Mainline DHT (BEP 5) among public nodes', 'UDP 7846, answers on 7847', 'info_hash = BLAKE3("hoi dht node v1" ‖ key)[..20]']));
  // Members of the community.
  const MEMBER_INFO = ['a member\'s home node or desktop', 'iroh QUIC + WebRTC listener', 'keeps the community\'s live window'];
  const mpos: [number, number][] = [[1260, 140], [1450, 175], [1300, 300], [1490, 340], [1250, 460], [1440, 520]];
  mpos.forEach(([x, y], i) => add(A(`m${i}`, 'member', `M${i + 1}`, 'member node', x, y, 14, MEMBER_INFO)));
  add(A('memTank', 'tank', '412', 'more member nodes', 1370, 420, 40, ['Simulated: other members\' nodes of c/privacy', 'each browser keeps ≤ 2 member links'], { count: 412 }));
  add(A('founder', 'member', 'Mod', 'can admit people', 1500, 600, 14, ['a member who can admit people', 'commits newcomers into the MLS group']));
  // Browsers.
  const bpos: [number, number][] = [[960, 760], [1100, 820], [1250, 760], [1390, 840], [1500, 760], [1020, 900]];
  bpos.forEach(([x, y], i) => add(A(`b${i}`, 'browser', i === 0 ? 'B2' : `B${i + 1}`, 'browser', x, y, 12, ['the web app in a browser', 'dials WebRTC only; accepts nothing', 'never publishes an address'])));
  add(A('brTank', 'tank', '48,120', 'more browsers', 1300, 900, 44, ['Simulated: browsers and phones', 'they reach the network only through nodes'], { count: 48120 }));
  // Home network.
  add(A('home', 'home', 'Home', 'your home node', 420, 650, 17, ['your desktop or hoi-node at home', 'serves your devices on ws://127.0.0.1:7842', 'and over WebRTC on the LAN'], { optional: true }));
  add(A('root', 'device', 'Root', 'your laptop (root key)', 150, 640, 13, ['the device holding your root key', 'approves new devices'], { optional: true }));
  add(A('lan', 'home', 'Node', 'a node on the same Wi-Fi', 470, 880, 15, ['a member\'s hoi-node on the same network', 'beacons on 239.255.70.74:7848', 'relaying: accepts incoming links'], { optional: true }));
  // Services.
  add(A('registry', 'service', 'Registry', 'none configured', 140, 150, 18, ['optional; builds name none', 'names → community keys, node announcements'], { dim: true }));
  add(A('relay', 'service', 'Relay', 'encrypted relay', 300, 300, 18, ['optional; for private communities', 'stores padded ciphertext ≤ 31 days', 'sees room ids, sizes, times'], { dim: true }));
  // You.
  add(A('you', 'you', 'You', 'your device', 760, 800, 21, ['your device']));

  const links: PipeLink[] = [];
  const link = (a: string, b: string, kind: PipeLink['kind'], bend = 0) => {
    const A1 = actors.get(a)!, B1 = actors.get(b)!;
    links.push({ a: A1, b: B1, kind, bend, grow: 1, hot: 0, length: Math.hypot(A1.x - B1.x, A1.y - B1.y) });
  };
  for (let i = 0; i < 8; i++) { link(`p${i}`, `p${(i + 1) % 8}`, 'bg', 14); link(`p${i}`, 'pubTank', 'tank'); }
  link('p1', 'p5', 'bg', 40); link('p3', 'p7', 'bg', -40);
  link('p2', 'dht', 'bg', 20); link('p3', 'dht', 'bg', -20);
  link('m0', 'p1', 'bg', 30); link('m2', 'p2', 'bg', -20); link('m4', 'p3', 'bg', 30); link('m1', 'm3', 'bg', 10); link('m0', 'm2', 'bg', -10);
  for (let i = 0; i < 6; i++) link(`m${i}`, 'memTank', 'tank');
  link('m5', 'founder', 'bg', 10); link('m3', 'm5', 'bg', -10);
  link('b0', 'p0', 'bg', 30); link('b1', 'm4', 'bg', -30); link('b2', 'm4', 'bg', 20); link('b3', 'm5', 'bg', -10); link('b4', 'm5', 'bg', 20); link('b5', 'p4', 'bg', -40);
  for (let i = 0; i < 6; i++) link(`b${i}`, 'brTank', 'tank');
  link('memTank', 'pubTank', 'tank', 30); link('brTank', 'memTank', 'tank', -40); link('brTank', 'pubTank', 'tank', 60);
  return { actors, links };
}

/** Point on a pipe (a gentle quadratic curve). */
export function pointAt(l: PipeLink, t: number): [number, number] {
  const mx = (l.a.x + l.b.x) / 2, my = (l.a.y + l.b.y) / 2;
  const dx = l.b.x - l.a.x, dy = l.b.y - l.a.y;
  const len = Math.hypot(dx, dy) || 1;
  const cx = mx - (dy / len) * l.bend, cy = my + (dx / len) * l.bend;
  const u = 1 - t;
  return [u * u * l.a.x + 2 * u * t * cx + t * t * l.b.x, u * u * l.a.y + 2 * u * t * cy + t * t * l.b.y];
}

const KIND_STROKE: Record<Kind, string> = {
  you: '#FF6B4A', public: '#FFE16A', member: '#BFF0D0', browser: '#9FD0FF', home: '#DCD2FF', device: '#DCD2FF', tank: '#34343B', dht: '#FFE16A', service: '#FFD2C2',
};

export interface View { s: number; ox: number; oy: number; dpr: number }

export function draw(ctx: CanvasRenderingContext2D, view: View, cw: number, ch: number, actors: Map<string, Actor>, links: PipeLink[], dots: Dot[], time: number, hover: Actor | null, used: Set<string>): void {
  ctx.setTransform(view.dpr, 0, 0, view.dpr, 0, 0);
  ctx.clearRect(0, 0, cw, ch);
  ctx.setTransform(view.dpr * view.s, 0, 0, view.dpr * view.s, view.dpr * view.ox, view.dpr * view.oy);
  const px = 1 / view.s;

  // Rooms of the floor plan.
  ctx.lineWidth = 1.2 * px;
  ctx.setLineDash([6 * px, 6 * px]);
  ctx.font = `500 ${11 * Math.max(1, px * 0.9)}px "Geist Mono", ui-monospace, monospace`;
  for (const r of REGIONS) {
    ctx.strokeStyle = 'rgba(244,242,234,0.12)';
    ctx.fillStyle = 'rgba(244,242,234,0.018)';
    roundRect(ctx, r.x, r.y, r.w, r.h, 22);
    ctx.fill();
    ctx.stroke();
  }
  ctx.setLineDash([]);
  // The DHT, around the public ring.
  ctx.strokeStyle = 'rgba(255,225,106,0.16)';
  ctx.setLineDash([2 * px, 7 * px]);
  ctx.lineWidth = 2 * px;
  ctx.beginPath();
  ctx.ellipse(800, 235, 300, 180, 0, 0, Math.PI * 2);
  ctx.stroke();
  ctx.setLineDash([]);

  const visible = (a: Actor) => a.s > 0.01 && (!a.optional || used.has(a.id));
  // Pipes: a dark casing and a coloured core.
  for (const l of links) {
    if (!visible(l.a) || !visible(l.b) || l.grow <= 0) continue;
    const tank = l.kind === 'tank';
    const scen = l.kind !== 'bg' && l.kind !== 'tank';
    const steps = 24;
    const end = Math.max(0.001, Math.min(1, l.grow));
    ctx.beginPath();
    for (let i = 0; i <= steps; i++) {
      const [x, y] = pointAt(l, (i / steps) * end);
      if (i) ctx.lineTo(x, y); else ctx.moveTo(x, y);
    }
    ctx.lineCap = 'round';
    ctx.strokeStyle = '#0A0A0D';
    ctx.lineWidth = (tank ? 13 : scen ? 11 : 8) * Math.max(0.7, px);
    ctx.stroke();
    ctx.strokeStyle = scen ? pipeColor(l.kind) : tank ? 'rgba(244,242,234,0.10)' : 'rgba(244,242,234,0.16)';
    ctx.globalAlpha = scen ? 0.45 + 0.55 * l.hot : 1;
    ctx.lineWidth = (tank ? 6 : scen ? 4 : 3) * Math.max(0.7, px);
    if (l.kind === 'oob') ctx.setLineDash([3 * px, 6 * px]);
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.globalAlpha = 1;
    if (scen && l.grow >= 1 && l.hot > 0.05) {
      const [x, y] = pointAt(l, 0.5);
      label(ctx, PIPE_LABEL[l.kind], x, y - 12 * px, pipeColor(l.kind), px, 0.6 + 0.4 * l.hot);
    }
  }

  // Dots: the water.
  for (const d of dots) {
    const t = d.dir === 1 ? d.t : 1 - d.t;
    const [x, y] = pointAt(d.link, t);
    const c = FAM_COLOR[d.fam];
    const r = d.size * (d.ambient ? Math.max(0.8, px) : Math.max(1, px * 0.9));
    if (!d.ambient) {
      ctx.globalAlpha = 0.25;
      ctx.fillStyle = c;
      ctx.beginPath();
      ctx.arc(x, y, r * 2.4, 0, Math.PI * 2);
      ctx.fill();
      ctx.globalAlpha = 1;
    }
    ctx.fillStyle = c;
    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
    ctx.fill();
    if (d.label) label(ctx, d.label, x, y - 16 * px, c, px, 1);
  }

  // Devices.
  for (const a of actors.values()) {
    if (!visible(a)) continue;
    const r = Math.max(0, a.r * a.s);
    if (a.kind === 'tank') {
      ctx.fillStyle = '#18181D';
      ctx.strokeStyle = 'rgba(244,242,234,0.14)';
      ctx.lineWidth = 1.5 * px;
      ctx.beginPath();
      ctx.arc(a.x, a.y, Math.max(0, r), 0, Math.PI * 2);
      ctx.fill();
      ctx.stroke();
      // the level inside the tank wobbles a little: water
      ctx.save();
      ctx.beginPath();
      ctx.arc(a.x, a.y, Math.max(0, r - 3 * px), 0, Math.PI * 2);
      ctx.clip();
      ctx.fillStyle = 'rgba(159,208,255,0.10)';
      ctx.beginPath();
      const level = a.y + r * 0.15;
      ctx.moveTo(a.x - r, level);
      for (let i = 0; i <= 20; i++) {
        const x = a.x - r + (i / 20) * 2 * r;
        ctx.lineTo(x, level + Math.sin(time / 600 + i * 0.7) * 3);
      }
      ctx.lineTo(a.x + r, a.y + r);
      ctx.lineTo(a.x - r, a.y + r);
      ctx.fill();
      ctx.restore();
      ctx.fillStyle = '#F4F2EA';
      ctx.textAlign = 'center';
      ctx.font = `900 ${Math.max(14 * px, r * 0.5)}px Gabarito, system-ui, sans-serif`;
      ctx.fillText(a.label, a.x, a.y + Math.max(14 * px, r * 0.5) * 0.36);
      const sf = 10.5 * Math.max(1, px * 0.9);
      ctx.font = `500 ${sf}px "Geist Mono", ui-monospace, monospace`;
      const tw = ctx.measureText(a.sub).width + 10 * px;
      ctx.fillStyle = 'rgba(14,14,18,0.85)';
      roundRect(ctx, a.x - tw / 2, a.y + r + 4 * px, tw, sf + 6 * px, 4 * px);
      ctx.fill();
      ctx.fillStyle = 'rgba(244,242,234,0.7)';
      ctx.fillText(a.sub, a.x, a.y + r + 4 * px + sf * 0.95);
      ctx.textAlign = 'start';
      continue;
    }
    if (a.p > 0.01) {
      ctx.strokeStyle = KIND_STROKE[a.kind];
      ctx.globalAlpha = 1 - a.p;
      ctx.lineWidth = 2 * px;
      ctx.beginPath();
      ctx.arc(a.x, a.y, r + 6 + a.p * 26, 0, Math.PI * 2);
      ctx.stroke();
      ctx.globalAlpha = 1;
    }
    if (a.kind === 'you') {
      ctx.fillStyle = 'rgba(255,107,74,0.14)';
      ctx.beginPath();
      ctx.arc(a.x, a.y, Math.max(0, r + 10 + Math.sin(time / 380) * 3), 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.globalAlpha = a.dim && !used.has(a.id) ? 0.35 : 1;
    ctx.fillStyle = a.kind === 'you' ? '#FF6B4A' : '#1E1E24';
    ctx.strokeStyle = KIND_STROKE[a.kind];
    ctx.lineWidth = (a === hover ? 3 : 2) * px;
    ctx.beginPath();
    if (a.kind === 'service' || a.kind === 'dht') roundRect(ctx, a.x - r, a.y - r, r * 2, r * 2, 6);
    else ctx.arc(a.x, a.y, Math.max(0, r), 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();
    ctx.fillStyle = a.kind === 'you' ? '#1A1A1A' : '#F4F2EA';
    ctx.textAlign = 'center';
    ctx.font = `800 ${Math.max(9, r * 0.62)}px Gabarito, system-ui, sans-serif`;
    ctx.fillText(a.label.length > 4 ? a.label.slice(0, 1) : a.label, a.x, a.y + r * 0.22);
    if (a.kind === 'you' || a.kind === 'home' || a.kind === 'device' || a.kind === 'service' || a.kind === 'dht' || a === hover) {
      ctx.font = `500 ${10.5 * Math.max(1, px * 0.9)}px "Geist Mono", ui-monospace, monospace`;
      ctx.fillStyle = a.kind === 'you' ? '#FF8A70' : 'rgba(244,242,234,0.7)';
      ctx.fillText(a.kind === 'you' ? a.sub : a.label.length > 4 ? a.label : a.sub, a.x, a.y + r + 16 * Math.max(1, px * 0.9));
    }
    ctx.textAlign = 'start';
    ctx.globalAlpha = 1;
  }

  // Room names last, on a dark backing, so nothing flows over them.
  const fs = 11 * Math.max(1, px * 0.9);
  ctx.font = `500 ${fs}px "Geist Mono", ui-monospace, monospace`;
  for (const r of REGIONS) {
    const w = ctx.measureText(r.label).width;
    if (w + 30 * Math.max(1, px * 0.9) > r.w) continue; // too small to read here: the tooltip and the table say it
    ctx.fillStyle = 'rgba(14,14,18,0.88)';
    roundRect(ctx, r.x + 10, r.y + 10, w + 14 * Math.max(1, px * 0.9), fs + 10 * Math.max(1, px * 0.9), 6);
    ctx.fill();
    ctx.fillStyle = r.color;
    ctx.fillText(r.label, r.x + 10 + 7 * Math.max(1, px * 0.9), r.y + 10 + fs * 0.95 + 2 * Math.max(1, px * 0.9));
  }
}

function pipeColor(k: PipeLink['kind']): string {
  return k === 'rtc' || k === 'direct' ? '#FF8A70' : k === 'quic' ? '#FFE16A' : k === 'ws' ? '#8AA0FF' : k === 'udp' ? '#9FD0FF' : k === 'https' ? '#DCD2FF' : k === 'oob' ? '#F4F2EA' : '#888';
}

function label(ctx: CanvasRenderingContext2D, text: string, x: number, y: number, color: string, px: number, alpha: number): void {
  const fs = 11 * Math.max(1, px * 0.9);
  ctx.font = `500 ${fs}px "Geist Mono", ui-monospace, monospace`;
  const w = ctx.measureText(text).width + 12 * Math.max(1, px * 0.9);
  const h = fs + 8 * Math.max(1, px * 0.9);
  ctx.globalAlpha = alpha;
  ctx.fillStyle = 'rgba(14,14,18,0.92)';
  ctx.strokeStyle = color;
  ctx.lineWidth = 1 * px;
  roundRect(ctx, x - w / 2, y - h / 2, w, h, h / 2);
  ctx.fill();
  ctx.stroke();
  ctx.fillStyle = color;
  ctx.textAlign = 'center';
  ctx.fillText(text, x, y + fs * 0.36);
  ctx.textAlign = 'start';
  ctx.globalAlpha = 1;
}

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number): void {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

/** Where "you" stand for each kind of start. */
export const YOU_AT: Record<string, [number, number, string]> = {
  'you-browser': [760, 800, 'your browser'],
  'you-home': [250, 770, 'your home node'],
  'you-phone': [190, 880, 'your phone'],
  'you-public': [800, 475, 'your new public node'],
};
