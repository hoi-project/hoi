// The network playground: pick a device and a way in, press Join, and watch every frame your
// device sends and receives, step by step, through a simulated network drawn like a building's
// pipes. Canvas for the network (one draw per frame, hundreds of dots), anime.js for everything
// that moves in the interface and for the devices and pipes themselves (it animates the plain
// objects the canvas draws). Nothing connects anywhere: it all runs in this tab.
import './wiki';
import '../styles/playground.css';
import { animate, createTimeline, stagger, utils } from 'animejs';
import { $, reduce } from '../common';
import { highlight } from '../wiki/highlight';
import { DEVICES, FOR_DEVICE, SCENARIOS, type Counters, type Fam, type Scenario, type Step } from '../playground/scenarios';
import { buildWorld, draw, FAM_COLOR, FAM_LABEL, H, W, YOU_AT, type Actor, type Dot, type PipeLink, type View } from '../playground/world';

const root = $<HTMLElement>('#pg');
const canvas = $<HTMLCanvasElement>('#pg-canvas');
if (root && canvas) start(root, canvas);

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls = '', text = ''): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text) e.textContent = text;
  return e;
}

function start(root: HTMLElement, canvas: HTMLCanvasElement): void {
  const repo = root.dataset.repo ?? '';
  const ctx = canvas.getContext('2d')!;
  const stage = canvas.parentElement!;
  const { actors, links } = buildWorld();
  const you = actors.get('you')!;
  let dots: Dot[] = [];
  let hover: Actor | null = null;
  const used = new Set<string>();

  /* ---------- a viewport workspace, with one panel at a time on phones ---------- */
  const layout = $<HTMLElement>('.pg', root)!;
  const guide = $<HTMLElement>('#pg-guide')!;
  const title = $<HTMLElement>('.pg-intro h1')!;
  $<HTMLElement>('[data-pg-title]', root)!.append(title);
  guide.prepend(...document.querySelectorAll('.pg-intro .wk-lead, .pg-intro .wk-src-list'));
  layout.append(guide);
  document.documentElement.classList.add('pg-active');
  const compact = window.matchMedia('(max-width: 900px)');
  const panels = Array.from(root.querySelectorAll<HTMLElement>('[data-pg-view]'));
  const panelButtons = Array.from(root.querySelectorAll<HTMLButtonElement>('[data-pg-panel]'));
  let panel = compact.matches ? 'network' : 'setup';
  const showPanel = (name: string, focusButton = false) => {
    panel = !compact.matches && name === 'network' ? 'setup' : name;
    for (const p of panels) p.hidden = p.dataset.pgView !== panel && !(p === stage && !compact.matches);
    for (const b of panelButtons) b.setAttribute('aria-pressed', String(b.dataset.pgPanel === panel));
    if (focusButton) panelButtons.find((b) => b.dataset.pgPanel === panel)?.focus({ preventScroll: true });
  };
  for (const b of panelButtons) b.addEventListener('click', () => showPanel(b.dataset.pgPanel!));
  compact.addEventListener('change', () => showPanel(panel));
  showPanel(panel);
  // Existing wiki search results and bookmarks can point straight into the guide.
  const openGuideHash = () => {
    const target = Array.from(guide.querySelectorAll<HTMLElement>('[id]')).find((e) => `#${e.id}` === location.hash);
    if (!target) return;
    showPanel('guide');
    target.scrollIntoView({ block: 'start' });
  };
  window.addEventListener('hashchange', openGuideHash);
  openGuideHash();

  /* ---------- view: fit, pan, zoom ---------- */
  const view: View = { s: 1, ox: 0, oy: 0, dpr: Math.min(2, window.devicePixelRatio || 1) };
  let cw = 0, ch = 0;
  // Fit the floor plan between the numbers on top and the legend below.
  const fit = () => {
    const pad = 14;
    const top = (stage.querySelector('.pg-hud')?.getBoundingClientRect().height ?? 0) + 18;
    const legendEl = stage.querySelector<HTMLElement>('.pg-legend');
    const bottom = Math.max(44, legendEl && legendEl.offsetParent ? legendEl.getBoundingClientRect().height : 0) + 76;
    const s = Math.max(0.05, Math.min((cw - pad * 2) / W, (ch - top - bottom) / H));
    return { s, ox: (cw - W * s) / 2, oy: top + (ch - top - bottom - H * s) / 2 };
  };
  // On a phone the whole plan is small, so the camera follows each step's devices instead,
  // until the person pans or zooms themselves (Fit hands it back).
  let follow = false;
  let userMoved = false;
  let focus: string[] = [];
  let camera: ReturnType<typeof animate> | null = null;
  const camTo = (t: { s: number; ox: number; oy: number }) => {
    camera?.pause();
    if (reduce) Object.assign(view, t);
    else camera = animate(view, { ...t, duration: 750, ease: 'outExpo' });
  };
  const focusView = () => {
    const as = focus.map((id) => actors.get(id)).filter((a): a is Actor => !!a);
    if (!follow || userMoved || !as.length) return fit();
    const m = 90;
    const x0 = Math.min(...as.map((a) => a.x)) - m, x1 = Math.max(...as.map((a) => a.x)) + m;
    const y0 = Math.min(...as.map((a) => a.y)) - m, y1 = Math.max(...as.map((a) => a.y)) + m;
    const top = (stage.querySelector('.pg-hud')?.getBoundingClientRect().height ?? 0) + 18;
    const f = fit();
    const height = Math.max(40, ch - top - 120);
    const sc = Math.max(f.s, Math.min((cw - 24) / (x1 - x0), height / (y1 - y0), f.s * 3));
    return { s: sc, ox: cw / 2 - ((x0 + x1) / 2) * sc, oy: top + height / 2 - ((y0 + y1) / 2) * sc };
  };
  const resize = () => {
    const r = stage.getBoundingClientRect();
    // Hidden mobile panels have no size; keep the camera until the network is shown again.
    if (!r.width || !r.height) return;
    camera?.pause();
    const dx = (r.width - cw) / 2, dy = (r.height - ch) / 2;
    cw = r.width;
    ch = r.height;
    canvas.width = Math.round(cw * view.dpr);
    canvas.height = Math.round(ch * view.dpr);
    follow = cw < 700;
    if (userMoved) { view.ox += dx; view.oy += dy; }
    else Object.assign(view, focusView());
  };
  resize();
  new ResizeObserver(resize).observe(stage);
  const toWorld = (x: number, y: number) => [(x - view.ox) / view.s, (y - view.oy) / view.s];
  const zoomAt = (f: number, x: number, y: number) => {
    camera?.pause();
    const [wx, wy] = toWorld(x, y);
    view.s = Math.max(Math.min(0.2, fit().s / 2), Math.min(4, view.s * f));
    view.ox = x - wx * view.s;
    view.oy = y - wy * view.s;
  };
  const actorAt = (x: number, y: number) => {
    const [wx, wy] = toWorld(x, y);
    let best: Actor | null = null, distance = Infinity;
    for (const a of actors.values()) {
      if (a.s < 0.5 || (a.optional && !used.has(a.id))) continue;
      const d = Math.hypot(a.x - wx, a.y - wy);
      if (d < Math.max(a.r + 8, 22 / view.s) && d < distance) { best = a; distance = d; }
    }
    return best;
  };
  const pointers = new Map<number, { x: number; y: number }>();
  let drag: { x: number; y: number; ox: number; oy: number } | null = null;
  let pinch: { distance: number; x: number; y: number } | null = null;
  let moved = false;
  const beginGesture = () => {
    const [a, b] = Array.from(pointers.values());
    drag = a ? { ...a, ox: view.ox, oy: view.oy } : null;
    pinch = a && b ? { distance: Math.hypot(a.x - b.x, a.y - b.y), x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 } : null;
    canvas.classList.toggle('drag', !!a);
  };
  canvas.addEventListener('pointerdown', (e) => {
    if (e.pointerType === 'mouse' && e.button !== 0) return;
    camera?.pause();
    if (!pointers.size) moved = false;
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pointers.size > 1) moved = true;
    canvas.setPointerCapture(e.pointerId);
    showTip(null, 0, 0);
    beginGesture();
  });
  const endGesture = (e: PointerEvent) => {
    if (!pointers.has(e.pointerId)) return;
    const r = canvas.getBoundingClientRect();
    const tapped = e.type === 'pointerup' && !moved && drag && Math.hypot(e.clientX - drag.x, e.clientY - drag.y) < 6;
    pointers.delete(e.pointerId);
    beginGesture();
    if (tapped) {
      const actor = actorAt(e.clientX - r.left, e.clientY - r.top);
      if (actor) { inspectActor(actor); showPanel('details', true); }
    }
  };
  canvas.addEventListener('pointerup', endGesture);
  canvas.addEventListener('pointercancel', endGesture);
  canvas.addEventListener('lostpointercapture', endGesture);
  canvas.addEventListener('pointermove', (e) => {
    const r = canvas.getBoundingClientRect();
    if (pointers.has(e.pointerId) && drag) {
      pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
      const [a, b] = Array.from(pointers.values());
      if (pinch && b) {
        const distance = Math.hypot(a.x - b.x, a.y - b.y);
        const x = (a.x + b.x) / 2, y = (a.y + b.y) / 2;
        zoomAt(distance / Math.max(1, pinch.distance), pinch.x - r.left, pinch.y - r.top);
        view.ox += x - pinch.x;
        view.oy += y - pinch.y;
        pinch = { distance, x, y };
        userMoved = true;
      } else {
        if (Math.hypot(e.clientX - drag.x, e.clientY - drag.y) > 5) { userMoved = true; moved = true; }
        view.ox = drag.ox + e.clientX - drag.x;
        view.oy = drag.oy + e.clientY - drag.y;
      }
      return;
    }
    const best = actorAt(e.clientX - r.left, e.clientY - r.top);
    if (best !== hover) { hover = best; showTip(best, e.clientX - r.left, e.clientY - r.top); }
    else if (best) moveTip(e.clientX - r.left, e.clientY - r.top);
  });
  canvas.addEventListener('pointerleave', () => { hover = null; showTip(null, 0, 0); });
  canvas.addEventListener('wheel', (e) => {
    e.preventDefault();
    userMoved = true;
    const r = canvas.getBoundingClientRect();
    zoomAt(Math.exp(-e.deltaY * 0.0015), e.clientX - r.left, e.clientY - r.top);
  }, { passive: false });

  /* ---------- the interface around the canvas ---------- */
  const tip = $<HTMLElement>('.pg-tip', root)!;
  const showTip = (a: Actor | null, x: number, y: number) => {
    if (!a) { tip.style.opacity = '0'; return; }
    tip.replaceChildren(el('b', '', a.kind === 'tank' ? `${a.label} ${a.sub}` : `${a.label} · ${a.sub}`), ...a.info.map((t) => el('span', '', t)));
    tip.style.opacity = '1';
    moveTip(x, y);
  };
  const moveTip = (x: number, y: number) => {
    tip.style.left = Math.max(8, Math.min(cw - tip.offsetWidth - 8, x + 14)) + 'px';
    tip.style.top = Math.max(8, Math.min(ch - tip.offsetHeight - 8, y + 14)) + 'px';
  };

  const hud = $<HTMLElement>('.pg-hud', root)!;
  const sim = { public: 1286, home: 8402, browsers: 51930, communities: 4211, rate: 0 };
  const hudNums: Record<string, HTMLElement> = {};
  const pill = (key: string, label: string, color: string) => {
    const p = el('span', 'pg-pill');
    const i = el('i');
    i.style.setProperty('--c', color);
    const b = el('b', '', '0');
    hudNums[key] = b;
    p.append(i, b, document.createTextNode(' ' + label));
    hud.append(p);
  };
  hud.append(el('span', 'pg-pill', 'simulated network'));
  pill('public', 'public nodes', '#FFE16A');
  pill('home', 'home nodes', '#DCD2FF');
  pill('browsers', 'browsers', '#9FD0FF');
  pill('communities', 'communities', '#BFF0D0');
  pill('rate', 'msg/s', '#FF8A70');
  const setHud = (first = false) => {
    for (const k of Object.keys(hudNums) as (keyof typeof sim)[]) {
      const target = sim[k];
      const obj = { v: Number(hudNums[k].dataset.v ?? 0) };
      hudNums[k].dataset.v = String(target);
      if (reduce) { hudNums[k].textContent = Math.round(target).toLocaleString('en-US'); continue; }
      animate(obj, { v: target, duration: first ? 1400 : 900, ease: 'outExpo', onUpdate: () => { hudNums[k].textContent = Math.round(obj.v).toLocaleString('en-US'); } });
    }
  };

  const legend = $<HTMLElement>('.pg-legend', root)!;
  (Object.keys(FAM_LABEL) as Fam[]).filter((f) => f !== 'local').forEach((f) => {
    const p = el('span', 'pg-pill');
    const i = el('i');
    i.style.setProperty('--c', FAM_COLOR[f]);
    p.append(i, document.createTextNode(FAM_LABEL[f]));
    legend.append(p);
  });

  const devBox = $<HTMLElement>('[data-pg-devices]', root)!;
  const scenBox = $<HTMLElement>('[data-pg-scenarios]', root)!;
  const stepsBox = $<HTMLOListElement>('[data-pg-steps]', root)!;
  const insp = $<HTMLElement>('[data-pg-insp]', root)!;
  const playBtn = $<HTMLButtonElement>('[data-pg-play]', root)!;
  const stepBtn = $<HTMLButtonElement>('[data-pg-step]', root)!;
  const resetBtn = $<HTMLButtonElement>('[data-pg-reset]', root)!;
  const currentBtn = $<HTMLButtonElement>('[data-pg-current]', root)!;
  const fitBtn = $<HTMLButtonElement>('[data-pg-fit]', root);
  const zinBtn = $<HTMLButtonElement>('[data-pg-zin]', root);
  const zoutBtn = $<HTMLButtonElement>('[data-pg-zout]', root);
  const speedIn = $<HTMLInputElement>('[data-pg-speed]', root)!;
  const speedOut = $<HTMLElement>('[data-pg-speed-v]', root)!;
  const logBox = $<HTMLOListElement>('[data-pg-log]');
  const counterEls: Partial<Record<keyof Counters, HTMLElement>> = {};
  document.querySelectorAll<HTMLElement>('[data-pg-c]').forEach((e) => { counterEls[e.dataset.pgC as keyof Counters] = e; });

  fitBtn?.addEventListener('click', () => { userMoved = false; focus = []; camTo(fit()); });
  zinBtn?.addEventListener('click', () => { userMoved = true; zoomAt(1.25, cw / 2, ch / 2); });
  zoutBtn?.addEventListener('click', () => { userMoved = true; zoomAt(0.8, cw / 2, ch / 2); });
  canvas.tabIndex = 0;
  canvas.addEventListener('keydown', (e) => {
    if (e.key === '0' || e.key === 'Home') { e.preventDefault(); fitBtn?.click(); return; }
    if (e.key === '+' || e.key === '=') { e.preventDefault(); zinBtn?.click(); return; }
    if (e.key === '-') { e.preventDefault(); zoutBtn?.click(); return; }
    const delta: Record<string, [number, number]> = { ArrowLeft: [40, 0], ArrowRight: [-40, 0], ArrowUp: [0, 40], ArrowDown: [0, -40] };
    if (!delta[e.key]) return;
    e.preventDefault();
    camera?.pause();
    userMoved = true;
    view.ox += delta[e.key][0];
    view.oy += delta[e.key][1];
  });

  let speed = 1;
  speedIn.addEventListener('input', () => { speed = Number(speedIn.value); speedOut.textContent = `${speed}×`; });

  /* ---------- the run ---------- */
  let device: Scenario['device'] = 'browser';
  let scen: Scenario = SCENARIOS[0];
  let stepI = -1;
  let running = false;
  let paused = true;
  let stepOnce = false;
  let stepping = false;
  let runId = 0;
  let simT = 0;
  const counters: Counters = { links: 0, known: 0, candidates: 0, listings: 0, records: 0, communities: 0 };
  const scenLinks: PipeLink[] = [];

  const setCounters = (c: Partial<Counters>) => {
    for (const [k, v] of Object.entries(c) as [keyof Counters, number][]) {
      const e = counterEls[k];
      const from = counters[k];
      counters[k] = v;
      if (!e) continue;
      if (reduce || from === v) { e.textContent = v.toLocaleString('en-US'); continue; }
      const o = { v: from };
      animate(o, { v, duration: 800, ease: 'outExpo', onUpdate: () => { e.textContent = Math.round(o.v).toLocaleString('en-US'); } });
      animate(e, { scale: [1.35, 1], duration: 500, ease: 'outBack(3)' });
    }
  };

  const log = (from: string, to: string, name: string, fam: Fam) => {
    if (!logBox) return;
    const li = el('li');
    li.style.setProperty('--c', FAM_COLOR[fam]);
    const who = (id: string) => (id === 'you' ? 'you' : actors.get(id)?.label ?? id);
    li.append(el('span', 'ts', `+${(simT / 1000).toFixed(2)}s`), el('i'), el('span', '', `${who(from)} → ${who(to)}  ${name}`));
    logBox.prepend(li);
    while (logBox.children.length > 120) logBox.lastChild?.remove();
  };

  const renderDevices = () => {
    devBox.replaceChildren();
    for (const d of DEVICES) {
      const b = el('button', 'wk-btn' + (d.id === device ? ' on' : ''), d.label);
      b.type = 'button';
      b.setAttribute('aria-pressed', String(d.id === device));
      b.addEventListener('click', () => {
        device = d.id;
        renderDevices();
        choose(SCENARIOS.find((s) => s.id === FOR_DEVICE[d.id][0])!);
        devBox.querySelector<HTMLButtonElement>('[aria-pressed="true"]')?.focus({ preventScroll: true });
      });
      devBox.append(b);
    }
  };
  const renderScenarios = () => {
    scenBox.replaceChildren();
    for (const id of FOR_DEVICE[device]) {
      const s = SCENARIOS.find((x) => x.id === id)!;
      const b = el('button', 'wk-btn' + (s === scen ? ' on' : ''), s.title);
      b.type = 'button';
      b.setAttribute('aria-pressed', String(s === scen));
      b.addEventListener('click', () => {
        choose(s);
        scenBox.querySelector<HTMLButtonElement>('[aria-pressed="true"]')?.focus({ preventScroll: true });
      });
      scenBox.append(b);
    }
  };
  const renderSteps = () => {
    stepsBox.replaceChildren();
    scen.steps.forEach((st, i) => {
      const li = el('li');
      li.tabIndex = 0;
      li.setAttribute('role', 'button');
      li.setAttribute('aria-label', `Step ${i + 1}: ${st.title}. Show its details.`);
      li.append(el('span', 'n', String(i + 1)), el('b', '', st.title), el('span', 'via', st.via));
      const select = () => { inspect(st, i); showPanel('details', true); };
      li.addEventListener('click', select);
      li.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); select(); } });
      stepsBox.append(li);
    });
    if (!reduce) animate(Array.from(stepsBox.children), { opacity: [0, 1], x: [-12, 0], delay: stagger(35), duration: 420, ease: 'outExpo' });
  };
  const markSteps = () => {
    currentBtn.setAttribute('aria-controls', stepI < 0 ? 'pg-setup' : 'pg-details');
    currentBtn.textContent = stepI < 0 ? `${scen.title} · Setup` : stepI >= scen.steps.length ? 'Joined · View the final step' : `${stepI + 1}/${scen.steps.length} · ${scen.steps[stepI].title}`;
    Array.from(stepsBox.children).forEach((li, i) => {
      li.classList.toggle('done', i < stepI);
      li.classList.toggle('now', i === stepI);
    });
    const now = stepsBox.children[stepI] as HTMLElement | undefined;
    const setup = stepsBox.parentElement!;
    if (now && !setup.hidden) {
      const top = setup.scrollTop + now.getBoundingClientRect().top - setup.getBoundingClientRect().top - setup.clientHeight / 3;
      setup.scrollTo({ top: Math.max(0, top), behavior: reduce ? 'auto' : 'smooth' });
    }
  };

  const srcLink = (s: string) => {
    const m = /^([^:]+)(?::(\d+)(?:-(\d+))?)?$/.exec(s);
    const a = el('a', '', s);
    if (m && repo) a.href = `${repo}/blob/main/${m[1]}${m[2] ? `#L${m[2]}${m[3] ? `-L${m[3]}` : ''}` : ''}`;
    a.rel = 'noopener';
    return a;
  };
  const inspect = (st: Step, i: number) => {
    const h = el('h3');
    h.append(el('span', '', `${i + 1}. ${st.title}`));
    const tag = el('span', 'tag t-sky', st.via);
    h.append(tag);
    const p = el('p', '', st.text);
    const pre = el('pre');
    const code = el('code', '', st.data);
    pre.append(code);
    highlight(code, 'ts');
    const parts: HTMLElement[] = [h, p, pre];
    if (st.bytes) parts.push(el('div', 'bytes', st.bytes));
    if (st.caps?.length) {
      const ul = el('ul', 'pg-caps');
      st.caps.forEach((c) => ul.append(el('li', '', c)));
      parts.push(ul);
    }
    const src = el('div', 'bytes');
    src.append(document.createTextNode('code: '));
    st.src.forEach((s, k) => { if (k) src.append(document.createTextNode(' · ')); src.append(srcLink(s)); });
    parts.push(src);
    insp.replaceChildren(...parts);
    if (!reduce) animate(parts, { opacity: [0, 1], y: [8, 0], delay: stagger(40), duration: 380, ease: 'outQuad' });
  };
  const inspectActor = (a: Actor) => {
    const h = el('h3');
    h.append(el('span', '', a.kind === 'tank' ? `${a.label} ${a.sub}` : `${a.label} · ${a.sub}`), el('span', 'tag t-sun', a.kind));
    const ul = el('ul', 'pg-caps');
    a.info.forEach((t) => ul.append(el('li', '', t)));
    insp.replaceChildren(h, ul);
    if (!reduce) animate([h, ul], { opacity: [0, 1], y: [8, 0], delay: stagger(40), duration: 380 });
  };
  currentBtn.addEventListener('click', () => {
    if (stepI < 0) { showPanel('setup', true); return; }
    const i = Math.min(stepI, scen.steps.length - 1);
    inspect(scen.steps[i], i);
    showPanel('details', true);
  });

  const linkBetween = (a: string, b: string): PipeLink | undefined =>
    scenLinks.find((l) => (l.a.id === a && l.b.id === b) || (l.a.id === b && l.b.id === a)) ??
    links.find((l) => (l.a.id === a && l.b.id === b) || (l.a.id === b && l.b.id === a));

  const wait = (ms: number, my: number) => new Promise<boolean>((res) => {
    const until = simT + ms;
    const check = () => { if (my !== runId) return res(false); if (simT >= until) return res(true); requestAnimationFrame(check); };
    check();
  });

  const send = (from: string, to: string, name: string, fam: Fam, my: number) => new Promise<void>((res) => {
    let l = linkBetween(from, to);
    if (!l) {
      // A message along no pipe (out of band): draw a temporary one.
      const A1 = actors.get(from)!, B1 = actors.get(to)!;
      l = { a: A1, b: B1, kind: 'oob', bend: 30, grow: 1, hot: 1, length: Math.hypot(A1.x - B1.x, A1.y - B1.y) };
      scenLinks.push(l);
    }
    const dir: 1 | -1 = l.a.id === from ? 1 : -1;
    log(from, to, name, fam);
    if (reduce) { res(); return; }
    dots.push({ link: l, t: 0, dir, speed: 1 / (0.5 + l.length / 1500), fam, label: name, size: 4.5, ambient: false, done: () => { if (my === runId) pulse(actors.get(to)!); res(); } });
  });

  const pulse = (a: Actor) => { if (!reduce) animate(a, { p: [0, 1], duration: 700, ease: 'outQuad' }); };

  const openPipe = async (a: string, b: string, kind: PipeLink['kind'], my: number) => {
    let l = scenLinks.find((x) => (x.a.id === a && x.b.id === b) || (x.a.id === b && x.b.id === a));
    if (l && l.kind === kind) return l;
    const A1 = actors.get(a)!, B1 = actors.get(b)!;
    l = { a: A1, b: B1, kind, bend: kind === 'direct' ? -60 : 24, grow: 0, hot: 1, length: Math.hypot(A1.x - B1.x, A1.y - B1.y) };
    scenLinks.push(l);
    if (reduce) { l.grow = 1; return l; }
    const o = l;
    const dur = 700;
    const t0 = simT;
    await new Promise<void>((res) => {
      const tick = () => { if (my !== runId) return res(); o.grow = Math.min(1, (simT - t0) / dur); if (o.grow >= 1) return res(); requestAnimationFrame(tick); };
      tick();
    });
    return l;
  };

  const runStep = async (i: number, my: number) => {
    const st = scen.steps[i];
    stepI = i;
    markSteps();
    inspect(st, i);
    focus = [...new Set(['you', st.local, ...(st.open ?? []).slice(0, 2), ...st.msgs.flatMap((m) => [m.from, m.to])].filter((x): x is string => !!x))];
    if (follow && !userMoved) camTo(focusView());
    scenLinks.forEach((l) => { l.hot = Math.max(0.15, l.hot * 0.4); });
    if (st.local) {
      const a = actors.get(st.local)!;
      for (let k = 0; k < 2; k++) { pulse(a); if (!(await wait(380, my))) return false; }
    }
    if (st.open) {
      const l = await openPipe(st.open[0], st.open[1], st.open[2], my);
      if (my !== runId) return false;
      l.hot = 1;
    }
    for (const m of st.msgs) {
      const l = linkBetween(m.from, m.to);
      if (l) l.hot = 1;
      const n = m.n ?? 1;
      const flights: Promise<void>[] = [];
      for (let k = 0; k < n; k++) {
        flights.push(send(m.from, m.to, k === 0 ? (n > 1 ? `${m.name} ×${n}` : m.name) : '', m.fam, my));
        if (!(await wait(160, my))) return false;
      }
      await Promise.all(flights);
      if (my !== runId) return false;
      if (!(await wait(120, my))) return false;
    }
    if (st.set) setCounters(st.set);
    if (!(await wait(st.msgs.length ? 450 : 700, my))) return false;
    return true;
  };

  const run = async () => {
    const my = runId;
    running = true;
    for (let i = stepI + 1; i < scen.steps.length; i++) {
      while (paused && !stepOnce) {
        await new Promise((r) => requestAnimationFrame(r));
        if (my !== runId) return;
      }
      const once = stepOnce;
      stepOnce = false;
      stepping = once;
      const ok = await runStep(i, my);
      stepping = false;
      if (!ok) return;
      if (once) { paused = true; setPlayLabel(); }
    }
    if (my !== runId) return;
    running = false;
    paused = true;
    stepI = scen.steps.length;
    markSteps();
    setPlayLabel(true);
    // Joined: the new pipes carry the network's water from now on.
    scenLinks.forEach((l) => { l.hot = 0.5; });
  };

  const setPlayLabel = (done = false) => {
    playBtn.textContent = done ? '↻ Again' : paused ? (stepI < 0 ? '▶ Join' : '▶ Continue') : '❚❚ Pause';
    playBtn.classList.toggle('on', !paused || stepI < 0 || done);
  };

  const reset = () => {
    runId++;
    focus = [];
    userMoved = false;
    if (cw) camTo(fit());
    running = false;
    paused = true;
    stepOnce = false;
    stepI = -1;
    dots = dots.filter((d) => d.ambient);
    scenLinks.length = 0;
    used.clear();
    for (const st of scen.steps) {
      if (st.local) used.add(st.local);
      if (st.open) { used.add(st.open[0]); used.add(st.open[1]); }
      st.msgs.forEach((m) => { used.add(m.from); used.add(m.to); });
    }
    const [x, y, sub] = YOU_AT[scen.you];
    you.sub = sub;
    if (reduce) { you.x = x; you.y = y; } else animate(you, { x, y, duration: 900, ease: 'outExpo' });
    setCounters({ links: 0, known: 0, candidates: 0, listings: 0, records: 0, communities: 0 });
    if (logBox) logBox.replaceChildren();
    stepsBox.parentElement!.scrollTop = 0;
    markSteps();
    setPlayLabel();
    insp.replaceChildren(el('h3', '', scen.title), el('p', '', scen.intro), el('p', '', 'Press Join to play it, or Step to go one frame at a time. Click any step to read what it sends; click a device to see what it is.'));
    if (!reduce) {
      for (const a of actors.values()) if (a.optional && used.has(a.id) && a.s < 1) animate(a, { s: [0, 1], duration: 700, ease: 'outBack(2)' });
    } else for (const a of actors.values()) a.s = 1;
  };

  const choose = (s: Scenario) => {
    scen = s;
    renderScenarios();
    renderSteps();
    reset();
  };

  playBtn.addEventListener('click', () => {
    if (compact.matches && paused) showPanel('network');
    if (stepI >= scen.steps.length) { reset(); paused = false; setPlayLabel(); void run(); return; }
    paused = !paused;
    setPlayLabel();
    if (!running) void run();
  });
  stepBtn.addEventListener('click', () => {
    if (compact.matches) showPanel('network');
    if (stepI >= scen.steps.length) reset();
    stepOnce = true;
    paused = true;
    setPlayLabel();
    if (!running) void run();
  });
  resetBtn.addEventListener('click', reset);

  /* ---------- ambient flow: the rest of the simulated network at work ---------- */
  const ambientLinks = () => links.concat(scenLinks.filter((l) => l.grow >= 1 && l.kind !== 'oob'));
  const FAMS: Fam[] = ['sync', 'sync', 'sync', 'disc', 'hello', 'sync', 'disc', 'carry'];
  let emitAcc = 0;
  let rateWin = 0, rateCount = 0;
  const emit = (dt: number) => {
    if (reduce) return;
    emitAcc += dt * 0.03 * speed;
    while (emitAcc > 1) {
      emitAcc -= 1;
      const pool = ambientLinks().filter((l) => (!l.a.optional || used.has(l.a.id)) && (!l.b.optional || used.has(l.b.id)));
      const l = pool[Math.floor(Math.random() * pool.length)];
      if (!l) return;
      dots.push({ link: l, t: 0, dir: Math.random() < 0.5 ? 1 : -1, speed: (0.7 + Math.random() * 0.6) / (0.9 + l.length / 260), fam: FAMS[Math.floor(Math.random() * FAMS.length)], size: l.kind === 'tank' ? 2.6 : 2, ambient: true });
      rateCount++;
    }
  };
  // The numbers drift like a live network would; each ambient dot stands for ~4,000 messages.
  window.setInterval(() => {
    if (document.visibilityState !== 'visible') return;
    sim.public += utils.random(-3, 4);
    sim.home += utils.random(-8, 10);
    sim.browsers += utils.random(-60, 70);
    sim.communities += utils.random(-1, 2);
    sim.rate = reduce ? 186_000 + utils.random(-4000, 4000) : Math.round((rateCount / Math.max(0.5, rateWin / 1000)) * 4000);
    rateCount = 0;
    rateWin = 0;
    setHud();
  }, 2400);

  /* ---------- the loop ---------- */
  let last = performance.now();
  const frame = (now: number) => {
    // Schedule the next frame first: one bad frame must never stop the picture.
    requestAnimationFrame(frame);
    const dt = Math.min(64, now - last);
    last = now;
    rateWin += dt;
    if (document.visibilityState === 'visible') {
      // The scenario's clock stops when paused; the rest of the network keeps flowing.
      const advancing = !paused || stepping;
      if (advancing) simT += dt * speed;
      emit(dt);
      for (const d of dots) if (d.ambient || advancing) d.t += (dt / 1000) * d.speed * speed;
      const arrived = dots.filter((d) => d.t >= 1);
      if (arrived.length) {
        dots = dots.filter((d) => d.t < 1);
        arrived.forEach((d) => d.done?.());
      }
      draw(ctx, view, cw, ch, actors, links.concat(scenLinks), dots, now, hover, used);
    }
  };

  /* ---------- go ---------- */
  renderDevices();
  choose(scen);
  setHud(true);
  resize();
  sim.rate = 186_000;
  const intro = Array.from(actors.values()).filter((a) => !a.optional);
  if (reduce) intro.forEach((a) => { a.s = 1; });
  else {
    createTimeline()
      .add(intro.filter((a) => a.kind === 'tank'), { s: [0, 1], duration: 900, ease: 'outElastic(1, .7)' })
      .add(intro.filter((a) => a.kind !== 'tank'), { s: [0, 1], delay: stagger(28, { from: 'center' }), duration: 700, ease: 'outBack(2.2)' }, '-=650');
    links.forEach((l) => { l.grow = 0; });
    animate(links, { grow: [0, 1], delay: stagger(14, { start: 300 }), duration: 900, ease: 'inOutQuad' });
  }
  document.fonts?.ready.then(() => { last = performance.now(); });
  requestAnimationFrame(frame);
  // Make the canvas describe itself for screen readers as the scenario changes.
  canvas.setAttribute('aria-label', 'A simulated hoi network drawn as pipes between devices. Drag to pan or pinch to zoom. Use arrow keys to pan, plus and minus to zoom, and Home to fit. Open Setup for the steps and Details to read their frames.');
}

