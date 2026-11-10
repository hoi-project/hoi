// Shared by every page: self-hosted fonts, tokens and the one stylesheet, plus the small bits of
// behaviour every page has (logo bob, stickers popping in, copy buttons). Pages work without it.
import '@hoi/tokens/fonts.css';
import '@hoi/tokens/tokens.css';
import './styles/site.css';
import { animate, onScroll, stagger } from 'animejs';
import { detectOS } from './platform';
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

// A copy counts only when the browser fires its `copy` event. Some extensions intercept copying
// and answer as if they had copied when they did not: uBlock Origin's ClickFix filter does this
// for any page that copies a command such as `irm … | iex`. The button copies the text it shows,
// as the reader would by hand, so a blocker sees exactly what is copied and a blocked copy stays
// blocked. Then the text is left selected and the button says which keys copy it.
const COPY_KEYS = /^(macos|ios)$/.test(detectOS() ?? '') ? '\u2318C' : 'Ctrl+C';

/** The text a copy button copies: its code block, or the code beside it (a checksum). */
function copyTarget(btn: HTMLElement): HTMLElement | null {
  return btn.closest('.code, .wk-code')?.querySelector<HTMLElement>('pre code, pre')
    ?? btn.parentElement?.querySelector<HTMLElement>('code') ?? null;
}

/** Select el's text and copy it. 'copied' only if the browser fired its copy event; 'blocked' if
 * the copy command ran without one; 'unsupported' if the command is missing or refused. */
function copySelection(el: HTMLElement): 'copied' | 'blocked' | 'unsupported' {
  const sel = window.getSelection();
  if (!sel) return 'unsupported';
  const range = document.createRange();
  range.selectNodeContents(el);
  sel.removeAllRanges();
  sel.addRange(range);
  // Without the final line break, so a pasted command waits for Enter.
  const text = sel.toString().replace(/\s+$/, '');
  let copied = false;
  const onCopy = (e: ClipboardEvent) => {
    copied = true;
    // Plain text, exactly what is selected: no formatting pasted into a terminal.
    if (e.clipboardData) {
      e.clipboardData.setData('text/plain', text);
      e.preventDefault();
    }
  };
  document.addEventListener('copy', onCopy, { capture: true });
  let ran = false;
  try {
    ran = document.execCommand('copy');
  } catch {
    ran = false;
  }
  document.removeEventListener('copy', onCopy, { capture: true });
  if (copied) return 'copied';
  return ran ? 'blocked' : 'unsupported';
}

const timers = new WeakMap<HTMLElement, number>();
/** Copy what btn belongs to and say how it went, on the button, beside the text and to screen readers. */
export async function copyFrom(btn: HTMLElement): Promise<void> {
  const target = copyTarget(btn);
  if (!target) return;
  const label = btn.dataset.label ?? (btn.textContent || 'Copy');
  btn.dataset.label = label;
  let result = copySelection(target);
  if (result === 'unsupported' && navigator.clipboard?.writeText) {
    // Only where the copy command itself is missing; it cannot confirm the copy, so it comes second.
    result = await navigator.clipboard.writeText((target.textContent ?? '').replace(/\s+$/, '')).then(() => 'copied' as const, () => 'blocked' as const);
  }
  const box = btn.closest('.code, .wk-code, .sha');
  box?.querySelector('.copy-note')?.remove();
  if (result === 'copied') {
    window.getSelection()?.removeAllRanges();
    btn.textContent = 'Copied';
    btn.classList.remove('manual');
    btn.classList.add('done');
    announce('Copied to the clipboard');
  } else {
    btn.textContent = `Press ${COPY_KEYS}`;
    btn.classList.remove('done');
    btn.classList.add('manual');
    if (box) {
      const note = document.createElement('p');
      note.className = 'copy-note';
      note.textContent = `Not copied: the browser or an extension blocked it. The text is selected, so press ${COPY_KEYS} to copy it.`;
      box.appendChild(note);
    }
    announce(`Not copied. The text is selected: press ${COPY_KEYS === 'Ctrl+C' ? 'Control C' : 'Command C'} to copy it.`);
  }
  window.clearTimeout(timers.get(btn));
  timers.set(btn, window.setTimeout(() => {
    btn.textContent = label;
    btn.classList.remove('done', 'manual');
  }, result === 'copied' ? 1600 : 6000));
}

/** Wire every `[data-copy]` button under root (idempotent). */
export function wireCopyButtons(root: ParentNode = document): void {
  $$<HTMLButtonElement>('button[data-copy]', root).forEach((btn) => {
    if (btn.dataset.wired) return;
    btn.dataset.wired = '1';
    btn.addEventListener('click', () => void copyFrom(btn));
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
