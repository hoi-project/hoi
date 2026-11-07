// Shared by every page: self-hosted fonts, tokens and the one stylesheet, plus the small bits of
// behaviour every page has (logo bob, stickers popping in, copy buttons). Pages work without it.
import '@hoi/tokens/fonts.css';
import '@hoi/tokens/tokens.css';
import './styles/site.css';
import { animate, onScroll, stagger } from 'animejs';
// Release notices are built into these informational pages; no background polling.

export const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

export const $ = <T extends Element = HTMLElement>(sel: string, root: ParentNode = document): T | null =>
  root.querySelector<T>(sel);
export const $$ = <T extends Element = HTMLElement>(sel: string, root: ParentNode = document): T[] =>
  Array.from(root.querySelectorAll<T>(sel));

/* ---------- copy buttons ---------- */

let live: HTMLElement | null = null;
/** Politely tell screen readers what just happened ("Copied"). */
export function announce(text: string): void {
  if (!live) {
    live = document.createElement('div');
    live.className = 'sr-only';
    live.setAttribute('role', 'status');
    live.setAttribute('aria-live', 'polite');
    document.body.appendChild(live);
  }
  live.textContent = '';
  window.setTimeout(() => { if (live) live.textContent = text; }, 30);
}

function copySource(btn: HTMLElement): { text: string; node: Element | null } {
  if (btn.dataset.copyText) return { text: btn.dataset.copyText, node: null };
  const pre = btn.closest('.code')?.querySelector('pre') ?? null;
  if (!pre) return { text: '', node: null };
  // Leave out the explanatory comments (.c); copy just the commands.
  const clone = pre.cloneNode(true) as HTMLElement;
  clone.querySelectorAll('.c').forEach((c) => c.remove());
  const text = (clone.textContent ?? '')
    .split('\n')
    .map((l) => l.trimEnd())
    .filter((l) => l.trim() !== '')
    .join('\n');
  return { text, node: pre };
}

async function copy(btn: HTMLElement): Promise<void> {
  const { text, node } = copySource(btn);
  if (!text) return;
  const label = btn.dataset.label ?? (btn.textContent || 'Copy');
  btn.dataset.label = label;
  try {
    await navigator.clipboard.writeText(text);
    btn.textContent = 'Copied';
    btn.classList.add('done');
    announce('Copied to the clipboard');
  } catch {
    // No clipboard access (insecure context, permissions): select it so Ctrl+C / ⌘C works.
    const target = node ?? btn.parentElement?.querySelector('code') ?? null;
    if (target) {
      const range = document.createRange();
      range.selectNodeContents(target);
      const sel = window.getSelection();
      sel?.removeAllRanges();
      sel?.addRange(range);
    }
    btn.textContent = 'Selected';
    announce('Selected. Press Control C or Command C to copy.');
  }
  window.setTimeout(() => {
    btn.textContent = label;
    btn.classList.remove('done');
  }, 1600);
}

/** Wire every `[data-copy]` button under root (idempotent). */
export function wireCopyButtons(root: ParentNode = document): void {
  $$<HTMLButtonElement>('button[data-copy]', root).forEach((btn) => {
    if (btn.dataset.wired) return;
    btn.dataset.wired = '1';
    btn.addEventListener('click', () => void copy(btn));
  });
}

/* ---------- chrome: logo bob and stickers ---------- */

if (!reduce) {
  animate('.logo-mark .ld', { translateY: [0, -3, 0], delay: stagger(140), duration: 1600, loop: true, ease: 'inOutSine' });
  // The page's first sticker pops on load, every other one when it scrolls into view.
  $$('.hero .eyebrow, .page-hero .eyebrow').forEach((e) =>
    animate(e, { scale: [0, 1], rotate: [-14, -2], duration: 700, ease: 'outElastic(1, .6)' }),
  );
  $$('.band .eyebrow, .section .eyebrow').forEach((e) =>
    animate(e, { scale: [0, 1], rotate: [-12, -2], duration: 650, ease: 'outElastic(1, .6)', autoplay: onScroll({ target: e, enter: '95% top' }) }),
  );
}
wireCopyButtons();
