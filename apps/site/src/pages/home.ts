// The landing page: every section is a working demo of one promise.
// Without scripts, or with reduced motion, each demo rests.
import { animate, createDraggable, createSpring, createTimeline, onScroll, stagger, utils } from 'animejs';
import { $, $$, reduce } from '../common';
import { detectOS, OS_LABEL } from '../platform';

/* ---------- download buttons lead with the visitor's desktop OS ---------- */
{
  const os = detectOS();
  const mac = $<HTMLAnchorElement>('#get [data-os="macos"]');
  const mine = os === 'windows' || os === 'linux' ? $<HTMLAnchorElement>(`#get [data-os="${os}"]`) : null;
  if (os && mac && mine) {
    mac.textContent = 'macOS';
    mac.classList.remove('pill-coral');
    mine.textContent = `Get it for ${OS_LABEL[os]}`;
    mine.classList.add('pill-coral');
    mac.before(mine);
  }
}

/* ---------- hero words ---------- */
const h1 = $('#h1');
if (h1 && !reduce) {
  const words = (text: string, into: Node) => {
    const list = text.split(' ').filter(Boolean);
    list.forEach((w, i) => {
      const s = document.createElement('span');
      s.className = 'w';
      s.textContent = w;
      into.appendChild(s);
      if (i < list.length - 1) into.appendChild(document.createTextNode(' '));
    });
  };
  const out = document.createDocumentFragment();
  h1.childNodes.forEach((n) => {
    if (n.nodeType === Node.TEXT_NODE) words(n.textContent ?? '', out);
    else if (n instanceof HTMLElement) {
      const span = document.createElement('span');
      span.className = n.className;
      words(n.textContent ?? '', span);
      out.append(' ', span);
    }
  });
  h1.replaceChildren(out);
  animate('#h1 .w', { y: [36, 0], opacity: [0, 1], rotate: [() => utils.random(-4, 4), 0], delay: stagger(55, { start: 150 }), duration: 900, ease: 'outExpo' });
}

/* ---------- network ---------- */
interface NetNode { x: number; y: number; el: HTMLElement; nb: number[] }
const NAMES = ['mi', 'jw', 'sk', 'ra', 'pt', 'lh', 'tk', 'an', 'zz', 'mo', 'kai', 'ida', 'rob', 'emi', 'nur'];
const SVG_NS = 'http://www.w3.org/2000/svg';
const net = $('#net');
const svg = $<SVGSVGElement>('#netlines');
const netstat = $('#netstat');
let nodes: NetNode[] = [];
let generation = 0;
let delivered = 0;

function buildNet(): void {
  if (!net || !svg) return;
  generation++;
  net.querySelectorAll('.node, .pulse, .net-static').forEach((e) => e.remove());
  svg.replaceChildren();
  nodes = [];
  const links = new Set<string>();
  const W = net.clientWidth;
  const H = net.clientHeight;
  const n = W < 600 ? 9 : 16;
  const cols = W < 600 ? 3 : 6;
  const rows = Math.ceil(n / cols);
  const me = Math.floor(n / 2);
  for (let i = 0; i < n; i++) {
    const c = i % cols;
    const r = Math.floor(i / cols);
    const x = (c + 0.5) * (W / cols) + utils.random(-W / cols * 0.28, W / cols * 0.28);
    const y = (r + 0.5) * (H / rows) + utils.random(-H / rows * 0.25, H / rows * 0.25);
    const el = document.createElement('div');
    el.className = 'node' + (i === me ? ' me' : '');
    const chk = document.createElement('span');
    chk.className = 'chk';
    chk.textContent = '✓';
    el.append(i === me ? 'you' : NAMES[i % NAMES.length], chk);
    el.style.left = x + 'px';
    el.style.top = y + 'px';
    net.appendChild(el);
    nodes.push({ x, y, el, nb: [] });
  }
  nodes.forEach((a, i) => {
    const near = nodes
      .map((b, j) => ({ j, d: Math.hypot(a.x - b.x, a.y - b.y) }))
      .filter((o) => o.j !== i)
      .sort((p, q) => p.d - q.d)
      .slice(0, 2);
    near.forEach((o) => {
      if (!a.nb.includes(o.j)) a.nb.push(o.j);
      if (!nodes[o.j].nb.includes(i)) nodes[o.j].nb.push(i);
      const key = Math.min(i, o.j) + '-' + Math.max(i, o.j);
      if (!links.has(key)) {
        links.add(key);
        const l = document.createElementNS(SVG_NS, 'line');
        l.setAttribute('x1', String(a.x));
        l.setAttribute('y1', String(a.y));
        l.setAttribute('x2', String(nodes[o.j].x));
        l.setAttribute('y2', String(nodes[o.j].y));
        svg.appendChild(l);
      }
    });
  });
  if (!reduce) animate(nodes.map((nd) => nd.el), { scale: [0, 1], delay: stagger(40), duration: 600, ease: 'outBack(2)' });
}

const meIndex = () => nodes.findIndex((nd) => nd.el.classList.contains('me'));

function hop(pulse: HTMLElement, from: number, hopsLeft: number, visited: number[], gen: number): void {
  if (gen !== generation) { pulse.remove(); return; }
  const cands = nodes[from].nb.filter((j) => !visited.includes(j));
  if (!cands.length || hopsLeft === 0) {
    animate(pulse, { scale: [1, 0], opacity: 0, duration: 300, onComplete: () => pulse.remove() });
    return;
  }
  const to = cands[Math.floor(Math.random() * cands.length)];
  visited.push(to);
  animate(pulse, {
    left: [nodes[from].x + 'px', nodes[to].x + 'px'],
    top: [nodes[from].y + 'px', nodes[to].y + 'px'],
    duration: 650,
    ease: 'inOutQuad',
    onComplete: () => {
      if (gen !== generation) { pulse.remove(); return; }
      const chk = nodes[to].el.querySelector('.chk');
      animate(nodes[to].el, { scale: [1, 1.25, 1], duration: 450, ease: 'outQuad' });
      if (chk) {
        animate(chk, {
          scale: [0, 1], opacity: [0, 1], duration: 350, ease: 'outBack(3)',
          onComplete: () => { animate(chk, { opacity: 0, scale: 0, delay: 1400, duration: 300 }); },
        });
      }
      delivered++;
      if (netstat) netstat.textContent = delivered + (delivered === 1 ? ' post' : ' posts') + ' delivered since you opened this page';
      hop(pulse, to, hopsLeft - 1, visited, gen);
    },
  });
}

function sendPulse(from?: number): void {
  if (!net || !nodes.length) return;
  const f = typeof from === 'number' && from >= 0 ? from : Math.floor(Math.random() * nodes.length);
  const p = document.createElement('div');
  p.className = 'pulse';
  p.style.left = nodes[f].x + 'px';
  p.style.top = nodes[f].y + 'px';
  net.appendChild(p);
  const gen = generation;
  animate(p, { opacity: [0, 1], scale: [0, 1], duration: 250, onComplete: () => hop(p, f, 3, [f], gen) });
}

if (net) {
  buildNet();
  // Rebuild on width changes only: phones fire resize while the URL bar slides in and out.
  let lastW = net.clientWidth;
  let rt = 0;
  window.addEventListener('resize', () => {
    window.clearTimeout(rt);
    rt = window.setTimeout(() => {
      if (net.clientWidth !== lastW) { lastW = net.clientWidth; buildNet(); }
    }, 200);
  });
  if (!reduce) {
    window.setTimeout(() => sendPulse(), 900);
    window.setInterval(() => { if (document.visibilityState === 'visible') sendPulse(); }, 2200);
  }
  net.addEventListener('click', () => sendPulse(meIndex()));
}

/* ---------- sign demo ---------- */
const signBtn = $<HTMLButtonElement>('#signBtn');
const resetBtn = $<HTMLButtonElement>('#resetBtn');
const hash = $('#hash');
const stamp = $('#stamp');
const workbar = $('#workbar');
const HEX = '0123456789abcdef';
const rnd = (n: number) => Array.from({ length: n }, () => HEX[Math.floor(Math.random() * 16)]).join('');

if (signBtn && resetBtn && hash && stamp && workbar) {
  const peers = $$('#note .peer');
  const oks = $$('#note .peer .ok');
  const finish = (refocus: boolean) => {
    resetBtn.hidden = false;
    if (refocus) resetBtn.focus();
    sendPulse(meIndex());
  };
  signBtn.addEventListener('click', () => {
    const refocus = document.activeElement === signBtn;
    signBtn.disabled = true;
    const final = 'b3:' + rnd(4) + '…' + rnd(4);
    if (reduce) {
      // State changes stay, motion goes: land straight on the finished state.
      hash.textContent = final;
      utils.set(workbar, { width: '100%' });
      utils.set(stamp, { scale: 1, rotate: -8, opacity: 1 });
      peers.forEach((p) => p.classList.add('lit'));
      utils.set(oks, { scale: 1, opacity: 1 });
      finish(refocus);
      return;
    }
    const scramble = window.setInterval(() => { hash.textContent = 'b3:' + rnd(4) + '…' + rnd(4); }, 60);
    createTimeline()
      .add(workbar, { width: ['0%', '100%'], duration: 1000, ease: 'inOutQuad' })
      .add(stamp, {
        scale: [3, 1], rotate: [-30, -8], opacity: [0, 1], duration: 650, ease: 'outElastic(1, .55)',
        onBegin: () => { window.clearInterval(scramble); hash.textContent = final; },
      })
      .add(peers, { translateX: [0, 6, 0], duration: 400, delay: stagger(140) }, '-=200')
      .add(oks, {
        scale: [0.6, 1], opacity: [0, 1], duration: 450, delay: stagger(140), ease: 'outBack(3)',
        onBegin: () => { peers.forEach((p, i) => window.setTimeout(() => p.classList.add('lit'), i * 140)); },
      }, '-=350')
      .call(() => finish(refocus));
  });
  resetBtn.addEventListener('click', () => {
    const refocus = document.activeElement === resetBtn;
    resetBtn.hidden = true;
    signBtn.disabled = false;
    if (refocus) signBtn.focus();
    hash.textContent = '— waiting —';
    peers.forEach((p) => p.classList.remove('lit'));
    if (reduce) {
      utils.set(oks, { opacity: 0, scale: 0.6 });
      utils.set(stamp, { opacity: 0, scale: 0 });
      utils.set(workbar, { width: '0%' });
      return;
    }
    animate(oks, { opacity: 0, scale: 0.6, duration: 250 });
    animate(stamp, { opacity: 0, scale: 0, duration: 250 });
    animate(workbar, { width: '0%', duration: 250 });
  });
}

/* ---------- spam demo ---------- */
const arena = $('#arena');
const shield = $('#shield');
const count = $('#count');
const badge = $('#badge');
const floodBtn = $<HTMLButtonElement>('#floodBtn');
const SPAM = ['FREE CRYPTO!!!', 'buy the only phone that…', 'hot singles in your relay', 'CLICK NOW', 'make $$$ from home', 'u won a vpn', 'fake audit inside', 'sell ur keys', 'discount passkeys', 'relay doubler!!', 'best vpn trust me', 'ur post was deleted click'];

if (arena && shield && count && badge && floodBtn) {
  const bump = (by: number) => { count.textContent = String(parseInt(count.textContent ?? '0', 10) + by); };
  floodBtn.addEventListener('click', () => {
    const refocus = document.activeElement === floodBtn;
    const batch = 12;
    if (reduce) {
      bump(batch);
      utils.set(badge, { opacity: 1, translateY: 0 });
      return;
    }
    floodBtn.disabled = true;
    const H = arena.clientHeight;
    const W = arena.clientWidth;
    const cx = W / 2;
    let done = 0;
    for (let i = 0; i < batch; i++) {
      const s = document.createElement('span');
      s.className = 'spam';
      s.textContent = SPAM[i % SPAM.length];
      s.style.top = utils.random(20, H - 50) + 'px';
      arena.appendChild(s);
      const stopX = cx + 70 - s.offsetWidth * 0.1;
      createTimeline({ delay: i * 90 })
        .add(s, { translateX: [0, -(W - stopX)], rotate: [0, utils.random(-10, 10)], duration: 520, ease: 'inQuad' })
        .add(shield, { scale: [1, 1.12, 1], rotate: [0, utils.random(-6, 6), 0], duration: 260, ease: 'outQuad', onBegin: () => bump(1) }, '-=20')
        .add(s, {
          translateX: '+=' + utils.random(140, 260), translateY: '+=' + utils.random(-120, 120), rotate: '+=' + utils.random(-200, 200),
          opacity: [1, 0], scale: [1, 0.5], duration: 650, ease: 'outBack(1.4)',
          onComplete: () => {
            s.remove();
            done++;
            if (done === batch) {
              animate(badge, { opacity: [0, 1], translateY: [10, 0], duration: 500, ease: 'outBack(2)' });
              window.setTimeout(() => { floodBtn.disabled = false; if (refocus) floodBtn.focus(); }, 600);
            }
          },
        }, '-=260');
    }
  });
}

/* ---------- fade scrubber ---------- */
const posts = $$('#posts .fp');
if (!reduce && posts.length) {
  const ftl = createTimeline({ autoplay: onScroll({ target: '#posts', enter: 'bottom top', leave: 'center center', sync: 'inOutQuad' }) });
  posts.forEach((p, i) => {
    const life = p.querySelector('.life i');
    if (life) ftl.add(life, { scaleX: [1, 0], duration: 900, ease: 'linear' }, i * 600);
    ftl.add(p, { opacity: [1, 0.12], scale: [1, 0.94], translateX: [0, -14], duration: 500, ease: 'inOutQuad' }, i * 600 + 500);
  });
}

/* ---------- draggable chips ---------- */
const chips = $$('#zone .chip');
chips.forEach((c) => {
  createDraggable(c, { container: '#zone', releaseEase: createSpring({ stiffness: 110, damping: 9 }) });
});
if (!reduce && chips.length) {
  animate(chips, {
    scale: [0, 1],
    rotate: [() => utils.random(-20, 20), () => utils.random(-4, 4)],
    delay: stagger(70), duration: 700, ease: 'outBack(2)',
    autoplay: onScroll({ target: '#zone', enter: '90% top' }),
  });
}
