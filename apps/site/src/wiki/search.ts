// The wiki's search: a dialog over an index bundled into the page (virtual:wiki-index), so a
// search makes no request. Opens with the sidebar button, "/" or Ctrl/⌘+K.
import index, { type WikiIndexPage } from 'virtual:wiki-index';
import { animate, stagger } from 'animejs';

interface Hit { url: string; title: string; where: string; text: string; color: string; score: number }

const norm = (s: string) => s.toLowerCase().normalize('NFKD').replace(/[̀-ͯ]/g, '');

function search(q: string): Hit[] {
  const words = norm(q).split(/\s+/).filter((w) => w.length > 1 || /\d/.test(w));
  if (!words.length) return [];
  const hits: Hit[] = [];
  const scoreOf = (hay: string, weight: number) => {
    let s = 0;
    const h = norm(hay);
    for (const w of words) {
      const i = h.indexOf(w);
      if (i < 0) return 0;
      s += weight * (i === 0 || /\W/.test(h[i - 1]) ? 2 : 1);
    }
    return s;
  };
  for (const p of index as WikiIndexPage[]) {
    const ps = scoreOf(p.t, 10) + scoreOf(p.t + ' ' + p.s + ' ' + p.l, 3);
    if (ps) hits.push({ url: p.u, title: p.t, where: p.g, text: p.s, color: p.c, score: ps + 5 });
    for (const h of p.h) {
      const s = scoreOf(h.t, 6) + scoreOf(h.t + ' ' + h.x, 2);
      if (s) hits.push({ url: `${p.u}#${h.id}`, title: h.t, where: p.t, text: h.x, color: p.c, score: s });
    }
  }
  return hits.sort((a, b) => b.score - a.score).slice(0, 40);
}

function marked(text: string, q: string): DocumentFragment {
  const frag = document.createDocumentFragment();
  const words = norm(q).split(/\s+/).filter((w) => w.length > 1);
  const n = norm(text);
  let i = 0;
  const ranges: [number, number][] = [];
  for (const w of words) {
    let k = n.indexOf(w);
    while (k >= 0) { ranges.push([k, k + w.length]); k = n.indexOf(w, k + w.length); }
  }
  ranges.sort((a, b) => a[0] - b[0]);
  for (const [a, b] of ranges) {
    if (a < i) continue;
    frag.append(text.slice(i, a));
    const m = document.createElement('mark');
    m.textContent = text.slice(a, b);
    frag.append(m);
    i = b;
  }
  frag.append(text.slice(i));
  return frag;
}

export function setupSearch(reduce: boolean): void {
  const openers = Array.from(document.querySelectorAll<HTMLButtonElement>('[data-search-open]'));
  if (!openers.length || typeof HTMLDialogElement === 'undefined') return;
  const dlg = document.createElement('dialog');
  dlg.className = 'wk-dlg';
  dlg.setAttribute('aria-label', 'Search the wiki');
  const inner = document.createElement('div');
  inner.className = 'wk-dlg-in';
  const bar = document.createElement('div');
  bar.className = 'wk-dlg-q';
  const icon = document.createElement('span');
  icon.className = 'wk-search-i';
  icon.setAttribute('aria-hidden', 'true');
  const input = document.createElement('input');
  input.type = 'search';
  input.placeholder = 'Search records, sync, MLS, hoi-node…';
  input.setAttribute('aria-label', 'Search the wiki');
  input.setAttribute('role', 'combobox');
  input.setAttribute('aria-expanded', 'true');
  input.setAttribute('aria-controls', 'wk-res');
  input.autocomplete = 'off';
  input.spellcheck = false;
  const close = document.createElement('button');
  close.type = 'button';
  close.className = 'wk-btn';
  close.textContent = 'Esc';
  close.setAttribute('aria-label', 'Close search');
  bar.append(icon, input, close);
  const list = document.createElement('ul');
  list.className = 'wk-dlg-res';
  list.id = 'wk-res';
  list.setAttribute('role', 'listbox');
  const foot = document.createElement('div');
  foot.className = 'wk-dlg-foot';
  foot.textContent = '↑ ↓ to move · Enter to open · Esc to close · searches every page of the wiki, here in your browser';
  inner.append(bar, list, foot);
  dlg.append(inner);
  document.body.append(dlg);

  let sel = 0;
  let links: HTMLAnchorElement[] = [];
  const select = (i: number) => {
    if (!links.length) return;
    sel = (i + links.length) % links.length;
    links.forEach((a, k) => a.setAttribute('aria-selected', String(k === sel)));
    links[sel].scrollIntoView({ block: 'nearest' });
    input.setAttribute('aria-activedescendant', links[sel].id);
  };
  const render = () => {
    const q = input.value.trim();
    list.replaceChildren();
    links = [];
    if (!q) {
      const li = document.createElement('li');
      li.className = 'wk-dlg-empty';
      li.textContent = `${(index as WikiIndexPage[]).length} pages, ${(index as WikiIndexPage[]).reduce((n, p) => n + p.h.length, 0)} sections. Type to search.`;
      list.append(li);
      return;
    }
    const hits = search(q);
    if (!hits.length) {
      const li = document.createElement('li');
      li.className = 'wk-dlg-empty';
      li.textContent = `Nothing for “${q}”.`;
      list.append(li);
      return;
    }
    hits.forEach((h, k) => {
      const li = document.createElement('li');
      const a = document.createElement('a');
      a.href = h.url;
      a.id = `wk-hit-${k}`;
      a.setAttribute('role', 'option');
      a.className = `g-${h.color}`;
      const i = document.createElement('i');
      const b = document.createElement('b');
      b.append(marked(h.title, q));
      const small = document.createElement('small');
      small.textContent = h.where;
      b.append(small);
      const t = document.createElement('span');
      t.append(marked(h.text.length > 150 ? h.text.slice(0, 150) + '…' : h.text, q));
      a.append(i, b, t);
      a.addEventListener('click', () => dlg.close());
      li.append(a);
      list.append(li);
      links.push(a);
    });
    select(0);
    if (!reduce) animate(links.slice(0, 10), { opacity: [0, 1], x: [-6, 0], delay: stagger(18), duration: 260, ease: 'outQuad' });
  };
  input.addEventListener('input', render);
  input.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown') { e.preventDefault(); select(sel + 1); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); select(sel - 1); }
    else if (e.key === 'Enter' && links[sel]) { e.preventDefault(); links[sel].click(); location.href = links[sel].href; }
  });
  close.addEventListener('click', () => dlg.close());
  dlg.addEventListener('click', (e) => { if (e.target === dlg) dlg.close(); });
  let opener: HTMLElement | null = null;
  const open = () => {
    opener = document.activeElement as HTMLElement | null;
    if (!dlg.open) dlg.showModal();
    input.select();
    render();
    if (!reduce) animate(dlg, { opacity: [0, 1], y: [-12, 0], scale: [0.98, 1], duration: 280, ease: 'outExpo' });
  };
  dlg.addEventListener('close', () => opener?.focus());
  openers.forEach((b) => b.addEventListener('click', open));
  document.addEventListener('keydown', (e) => {
    const t = e.target as HTMLElement;
    const typing = t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable);
    if ((e.key === '/' && !typing) || ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k')) {
      e.preventDefault();
      open();
    }
  });
}
