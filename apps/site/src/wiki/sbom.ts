// Filtering the SBOM's package table: by text, program, ecosystem and "shipped only".
// The table is complete without this; filters only hide rows.
import { animate } from 'animejs';

export function setupSbom(reduce: boolean): void {
  const q = document.querySelector<HTMLInputElement>('[data-sb-q]');
  const sel = document.querySelector<HTMLSelectElement>('[data-sb-p]');
  const count = document.querySelector<HTMLElement>('[data-sb-count]');
  const rows = Array.from(document.querySelectorAll<HTMLTableRowElement>('.sb-all tbody tr'));
  if (!q || !sel || !count || !rows.length) return;
  const ecoBtns = Array.from(document.querySelectorAll<HTMLButtonElement>('[data-sb-e]'));
  const shipBtn = document.querySelector<HTMLButtonElement>('[data-sb-s]');
  let eco = '';
  let shipped = false;
  const total = rows.length;
  const text = rows.map((r) => (r.textContent ?? '').toLowerCase());
  let last = total;
  const apply = () => {
    const words = q.value.toLowerCase().split(/\s+/).filter(Boolean);
    const prog = sel.value;
    let n = 0;
    rows.forEach((r, i) => {
      const ok =
        (!eco || r.dataset.e === eco) &&
        (!shipped || r.dataset.s === '1') &&
        (!prog || (r.dataset.p ?? '').split(' ').includes(prog)) &&
        words.every((w) => text[i].includes(w));
      r.classList.toggle('sb-hide', !ok);
      if (ok) n++;
    });
    count.textContent = n === total ? `${total} packages` : `${n} of ${total} packages`;
    if (!reduce && n !== last) animate(count, { opacity: [0.3, 1], duration: 300 });
    last = n;
  };
  q.addEventListener('input', apply);
  sel.addEventListener('change', apply);
  ecoBtns.forEach((b) => b.addEventListener('click', () => {
    eco = b.dataset.sbE ?? '';
    ecoBtns.forEach((x) => { x.classList.toggle('on', x === b); x.setAttribute('aria-pressed', String(x === b)); });
    apply();
  }));
  shipBtn?.addEventListener('click', () => {
    shipped = !shipped;
    shipBtn.classList.toggle('on', shipped);
    shipBtn.setAttribute('aria-pressed', String(shipped));
    apply();
  });
  // A link to a package row (from a program's direct list) clears filters that would hide it.
  document.querySelectorAll<HTMLAnchorElement>('.sb-direct a').forEach((a) => a.addEventListener('click', () => {
    const row = document.getElementById(a.hash.slice(1));
    if (row?.classList.contains('sb-hide')) {
      q.value = '';
      sel.value = '';
      eco = '';
      shipped = false;
      ecoBtns.forEach((x, i) => { x.classList.toggle('on', i === 0); x.setAttribute('aria-pressed', String(i === 0)); });
      shipBtn?.classList.remove('on');
      shipBtn?.setAttribute('aria-pressed', 'false');
      apply();
    }
  }));
}
