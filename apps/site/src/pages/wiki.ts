// Every wiki page: the shared chrome, then the wiki's own behaviour. All of it is optional:
// without scripts every page reads the same, and with reduced motion nothing moves.
import '../common';
import '../styles/wiki.css';
import { animate, stagger, svg, utils } from 'animejs';
import { $, $$, copyFrom, reduce } from '../common';
import { highlight } from '../wiki/highlight';
import { setupSearch } from '../wiki/search';
import { setupAssistant } from '../wiki/assistant';

/* ---------- the title, letter by letter ---------- */
const h1 = $('.wk-doc h1');
if (h1 && !reduce) {
  const chars: HTMLElement[] = [];
  const splitInto = (text: string, into: Node, extra = '') => {
    text.split(/(\s+)/).forEach((part) => {
      if (!part) return;
      if (/^\s+$/.test(part)) { into.appendChild(document.createTextNode(' ')); return; }
      const wd = document.createElement('span');
      wd.className = 'wd' + extra;
      for (const c of part) {
        const s = document.createElement('span');
        s.className = 'ch';
        s.textContent = c;
        wd.appendChild(s);
        chars.push(s);
      }
      into.appendChild(wd);
    });
  };
  const label = h1.textContent ?? '';
  const out = document.createDocumentFragment();
  h1.childNodes.forEach((n) => {
    if (n.nodeType === Node.TEXT_NODE) splitInto(n.textContent ?? '', out);
    else if (n instanceof HTMLElement) {
      const span = document.createElement('span');
      span.className = n.className;
      splitInto(n.textContent ?? '', span);
      out.appendChild(span);
    }
  });
  h1.setAttribute('aria-label', label.trim());
  h1.replaceChildren(out);
  chars.forEach((c) => c.setAttribute('aria-hidden', 'true'));
  animate(chars, {
    y: ['0.6em', 0],
    opacity: [0, 1],
    rotate: [() => utils.random(-12, 12), 0],
    delay: stagger(18, { start: 80 }),
    duration: 900,
    ease: 'outElastic(1, .7)',
  });
  animate('.wk-crumbs li, .wk-lead, .wk-src-list, .wk-head-rule', { opacity: [0, 1], y: [10, 0], delay: stagger(60, { start: 200 }), duration: 600, ease: 'outExpo' });
}

/* ---------- code: colours and copy ---------- */
$$<HTMLElement>('.wk-code').forEach((box) => {
  const code = box.querySelector<HTMLElement>('pre code') ?? box.querySelector<HTMLElement>('pre');
  if (!code) return;
  highlight(code, box.dataset.lang ?? '');
  const btn = box.querySelector<HTMLButtonElement>('.wk-copy');
  btn?.addEventListener('click', () => void copyFrom(btn));
});

/* ---------- on this page: follow the reader ---------- */
const tocLinks = $$<HTMLAnchorElement>('.wk-toc a');
if (tocLinks.length && 'IntersectionObserver' in window) {
  const byId = new Map(tocLinks.map((a) => [decodeURIComponent(a.hash.slice(1)), a]));
  const targets = Array.from(byId.keys()).map((id) => document.getElementById(id)).filter((e): e is HTMLElement => !!e);
  const visible = new Set<string>();
  const pick = () => {
    let best: string | null = null;
    for (const t of targets) {
      if (t.getBoundingClientRect().top < window.innerHeight * 0.35) best = t.id;
    }
    if (!best && targets[0]) best = targets[0].id;
    tocLinks.forEach((a) => a.classList.toggle('on', decodeURIComponent(a.hash.slice(1)) === best));
  };
  const io = new IntersectionObserver((entries) => {
    for (const e of entries) e.isIntersecting ? visible.add(e.target.id) : visible.delete(e.target.id);
    pick();
  }, { rootMargin: '-80px 0px -55% 0px' });
  targets.forEach((t) => io.observe(t));
  window.addEventListener('scroll', () => window.requestAnimationFrame(pick), { passive: true });
  pick();
}

/* ---------- the sidebar opens at the current page ---------- */
const current = $<HTMLElement>('.wk-side a[aria-current="page"]');
const side = $<HTMLElement>('.wk-side');
if (current && side) {
  const top = current.offsetTop - side.clientHeight / 3;
  if (top > 0) side.scrollTop = top;
}

/* ---------- things arriving as they scroll into view ----------
   Nothing is hidden ahead of time: an element animates in only once it is actually on screen,
   so content stays readable for find-in-page, printing and anything that never scrolls. */
if (!reduce && 'IntersectionObserver' in window) {
  const plays = new Map<Element, () => void>();
  const io = new IntersectionObserver((entries) => {
    for (const e of entries) {
      if (!e.isIntersecting) continue;
      io.unobserve(e.target);
      plays.get(e.target)?.();
      plays.delete(e.target);
    }
  }, { rootMargin: '0px 0px -12% 0px' });
  const when = (el: Element, play: () => void) => { plays.set(el, play); io.observe(el); };
  // Only below the fold: what is on screen at load is already there.
  const below = (el: Element) => el.getBoundingClientRect().top > window.innerHeight;
  $$('.wk-sec > h2').filter(below).forEach((h) => when(h, () => animate(h, { x: [-18, 0], duration: 700, ease: 'outExpo' })));
  $$('.wk-seq').filter(below).forEach((seq) => when(seq, () => animate($$('.wk-seq-row', seq), { opacity: [0, 1], x: [-10, 0], delay: stagger(60), duration: 480, ease: 'outQuad' })));
  $$('.wk-bytes').filter(below).forEach((b) => when(b, () => animate($$('.wk-bytes-row > span', b), { scaleX: [0.2, 1], opacity: [0, 1], delay: stagger(50), duration: 600, ease: 'outExpo' })));
  $$('.wk-cards').filter(below).forEach((g) => when(g, () => animate($$('.wk-card', g), { opacity: [0, 1], y: [16, 0], delay: stagger(40), duration: 560, ease: 'outExpo' })));
  $$('.wk-steps').filter(below).forEach((g) => when(g, () => animate($$(':scope > li', g), { opacity: [0, 1], y: [12, 0], delay: stagger(70), duration: 560, ease: 'outExpo' })));
}

/* ---------- the grid on the wiki's home page ---------- */
const dots = $<HTMLElement>('.wk-dots');
if (dots) {
  const cols = window.innerWidth < 640 ? 16 : 32;
  const rows = 5;
  dots.style.setProperty('--cols', String(cols));
  const cls = ['a', 'b', 'c', 'd', 'e'];
  for (let i = 0; i < cols * rows; i++) {
    const d = document.createElement('i');
    if (Math.random() < 0.18) d.className = cls[i % cls.length];
    dots.appendChild(d);
  }
  if (!reduce) {
    const all = $$('.wk-dots i');
    const wave = (from: number) =>
      animate(all, {
        scale: [{ to: 0.25, duration: 380 }, { to: 1, duration: 900 }],
        rotate: [{ to: 45, duration: 380 }, { to: 0, duration: 900 }],
        delay: stagger(36, { grid: [cols, rows], from }),
        ease: 'inOutQuad',
      });
    wave(Math.floor(rows / 2) * cols + Math.floor(cols / 2));
    let t = 0;
    window.setInterval(() => { if (document.visibilityState === 'visible') wave(utils.random(0, cols * rows - 1)); }, 4200);
    dots.parentElement?.addEventListener('click', (e) => {
      const r = dots.getBoundingClientRect();
      const x = Math.floor(((e as MouseEvent).clientX - r.left) / (r.width / cols));
      const y = Math.floor(((e as MouseEvent).clientY - r.top) / (r.height / rows));
      t = Math.max(0, Math.min(rows - 1, y)) * cols + Math.max(0, Math.min(cols - 1, x));
      wave(t);
    });
  }
}

/* ---------- live demos (loaded only on pages that have one) ---------- */
const stages = $$<HTMLElement>('.wk-stage[data-demo]');
if (stages.length) {
  void import('../wiki/demos').then((m) => stages.forEach((s) => m.mount(s, reduce)));
}

setupSearch(reduce);
void setupAssistant();

/* ---------- the SBOM's filters ---------- */
if ($('[data-sb-q]')) void import('../wiki/sbom').then((m) => m.setupSbom(reduce));

/* ---------- the architecture diagram ---------- */
const arch = $<HTMLElement>('[data-arch]');
if (arch) {
  const say = $<HTMLElement>('.arch-say', arch);
  const boxes = $$<SVGAElement>('.arch-b', arch);
  const conns = $$<SVGPathElement>('.arch-c', arch);
  const ends = (c: SVGPathElement) => (c.dataset.a ?? '').split(' ');
  const light = (k: string | null, text: string | null) => {
    arch.classList.toggle('focus', !!k);
    conns.forEach((c) => c.classList.toggle('lit', !!k && ends(c).includes(k)));
    const near = new Set(conns.filter((c) => c.classList.contains('lit')).flatMap(ends));
    boxes.forEach((b) => b.classList.toggle('lit', !!k && (b.dataset.k === k || near.has(b.dataset.k ?? ''))));
    if (say) say.textContent = text ?? 'Hover or focus a box to read what it is.';
  };
  boxes.forEach((b) => {
    const on = () => light(b.dataset.k ?? null, b.dataset.d ?? null);
    b.addEventListener('mouseenter', on);
    b.addEventListener('focus', on);
    b.addEventListener('mouseleave', () => light(null, null));
    b.addEventListener('blur', () => light(null, null));
  });
  if (!reduce) {
    const go = () => {
      animate(svg.createDrawable($$<SVGPathElement>('.d-line', arch)), { draw: ['0 0', '0 1'], delay: stagger(40), duration: 900, ease: 'inOutQuad' });
      animate(boxes, { opacity: [0, 1], y: [10, 0], delay: stagger(30), duration: 600, ease: 'outExpo' });
      animate($$('.d-flow', arch), { strokeDashoffset: [0, -32], duration: 900, loop: true, ease: 'linear' });
    };
    if ('IntersectionObserver' in window) {
      const io = new IntersectionObserver((es) => { if (es.some((e) => e.isIntersecting)) { io.disconnect(); go(); } }, { threshold: 0.2 });
      io.observe(arch);
    } else go();
  }
}
