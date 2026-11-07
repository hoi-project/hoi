// Motion helpers on Anime.js 4 (tokens.motion). Every animation teaches something;
// the UI is complete at rest, so with reduced motion nothing here runs and nothing stays hidden.
import { animate, createSpring, createTimeline, stagger, utils } from 'animejs';
import type { Action } from 'svelte/action';

export const DUR = { micro: 300, ui: 420, enter: 520, hero: 900 } as const;
export const STAGGER = { rows: 45, words: 55 } as const;
export const EASE = {
  enter: 'outExpo',
  pop: 'outBack(2)',
  stamp: 'outElastic(1, .55)',
  ambient: 'inOutSine',
} as const;

export { animate, createTimeline, createSpring, stagger, utils };

/** True when motion should be skipped: OS setting or the app's own "reduced" setting. */
export function reduced(): boolean {
  if (typeof window === 'undefined') return true;
  if (document.documentElement.dataset.motion === 'reduced') return true;
  return window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;
}

type AnimParams = Parameters<typeof animate>[1];

/**
 * Run an appear-type animation (opacity/translate/scale) and guarantee the element ends at rest:
 * inline styles are cleared on completion, on a fallback timer (Anime's engine pauses in hidden tabs,
 * a screenshot or re-render may land mid-flight) and on destroy. Nothing can be left at opacity 0.
 */
function appear(targets: HTMLElement[], params: AnimParams, budgetMs: number): () => void {
  if (targets.length === 0) return () => {};
  let done = false;
  const clear = () => {
    if (done) return;
    done = true;
    for (const el of targets) {
      el.style.removeProperty('opacity');
      el.style.removeProperty('transform');
    }
  };
  const a = animate(targets, { ...params, onComplete: clear });
  const t = setTimeout(() => {
    a.pause();
    clear();
  }, budgetMs + 250);
  return () => {
    clearTimeout(t);
    a.pause();
    clear();
  };
}

/** Stagger a view's on-screen `.anim` children in: translateY 18 → 0, opacity 0 → 1. Off-screen ones are simply there. */
export const enterView: Action<HTMLElement> = (node) => {
  if (reduced()) return;
  const limit = window.innerHeight * 1.1;
  const items = Array.from(node.querySelectorAll<HTMLElement>(':scope .anim'));
  const targets = (items.length ? items : [node]).filter((el) => el.getBoundingClientRect().top < limit).slice(0, 12);
  const stop = appear(
    targets,
    { opacity: [0, 1], translateY: [18, 0], delay: stagger(STAGGER.rows), duration: DUR.enter, ease: EASE.enter },
    DUR.enter + STAGGER.rows * targets.length,
  );
  return { destroy: stop };
};

/** Stickers pop in with an elastic ease when they enter the viewport. */
export const popIn: Action<HTMLElement, number | undefined> = (node, rotate = -2) => {
  if (reduced() || typeof IntersectionObserver === 'undefined') return;
  let stop = () => {};
  const io = new IntersectionObserver((entries) => {
    for (const e of entries) {
      if (!e.isIntersecting) continue;
      io.disconnect();
      stop = appear([node], { scale: [0.4, 1], rotate: [rotate - 10, rotate], opacity: [0, 1], duration: 700, ease: 'outElastic(1, .6)' }, 700);
    }
  });
  io.observe(node);
  return {
    destroy: () => {
      io.disconnect();
      stop();
    },
  };
};

/** The logo's three dots bob on an ambient loop. */
export const bob: Action<SVGElement> = (node) => {
  if (reduced()) return;
  const a = animate(node.querySelectorAll('.ld'), {
    translateY: [0, -3, 0],
    delay: stagger(140),
    duration: 1600,
    loop: true,
    ease: EASE.ambient,
  });
  return { destroy: () => a.revert() };
};

/** The relay pill's dot pulses while connected. */
export const pulse: Action<HTMLElement, boolean> = (node, on) => {
  let a: ReturnType<typeof animate> | null = null;
  const set = (v: boolean | undefined) => {
    a?.revert();
    a = null;
    if (v && !reduced()) a = animate(node, { scale: [1, 1.5, 1], opacity: [1, 0.5, 1], duration: 1800, loop: true, ease: EASE.ambient });
  };
  set(on);
  return { update: set, destroy: () => a?.revert() };
};

/** A capsule bounce for votes: the whole capsule pops, the arrow nudges. */
export function bounce(box: Element | null, dir: 1 | -1, arrow?: Element | null): void {
  if (!box || reduced()) return;
  animate(box, { scale: [1, 1.14, 1], rotate: [0, dir > 0 ? -6 : 6, 0], duration: 380, ease: 'outBack(3)' });
  if (arrow) animate(arrow, { translateY: [0, dir > 0 ? -4 : 4, 0], duration: 300, ease: 'outQuad' });
}

/** A small "I did something" pop for buttons. */
export function nudge(el: Element | null): void {
  if (!el || reduced()) return;
  animate(el, { scale: [1, 1.08, 1], duration: 320, ease: 'outBack(3)' });
}

/** Count a number from → to in an element (votes, peers). */
export function tick(el: HTMLElement | null, from: number, to: number): void {
  if (!el) return;
  if (reduced() || from === to) {
    el.textContent = String(to);
    return;
  }
  const o = { v: from };
  animate(o, {
    v: to,
    duration: 350,
    ease: 'outQuad',
    modifier: utils.round(0),
    onUpdate: () => {
      el.textContent = String(o.v);
    },
  });
}

/** Switch knob on a spring (things you touch). */
export function springKnob(knob: HTMLElement | null, on: boolean): void {
  if (!knob) return;
  if (reduced()) {
    knob.style.transform = `translateX(${on ? 20 : 0}px)`;
    return;
  }
  animate(knob, { translateX: on ? 20 : 0, ease: createSpring({ stiffness: 320, damping: 16 }) });
}

/** Unfold / fold an element's height (composer, inline forms). Resolves when done, even if the engine is paused. */
export function unfold(el: HTMLElement, open: boolean): Promise<void> {
  return new Promise((resolve) => {
    const finish = () => {
      el.style.height = open ? 'auto' : '0px';
      el.style.opacity = open ? '' : '0';
      resolve();
    };
    if (reduced()) return finish();
    let settled = false;
    const end = () => {
      if (settled) return;
      settled = true;
      clearTimeout(t);
      a.pause();
      finish();
    };
    const a = animate(el, {
      height: open ? [0, el.scrollHeight] : [el.offsetHeight, 0],
      opacity: open ? [0, 1] : [1, 0],
      duration: DUR.ui,
      ease: 'outCubic',
      onComplete: end,
    });
    const t = setTimeout(end, DUR.ui + 300);
  });
}

/** Slide a full-screen sheet (the phone composer) up into view, or back down. Resolves when
 * done, even if the engine is paused; at rest the element carries no transform. */
export function slideSheet(el: HTMLElement, open: boolean): Promise<void> {
  return new Promise((resolve) => {
    const finish = () => {
      el.style.removeProperty('transform');
      el.style.opacity = open ? '' : '0';
      resolve();
    };
    if (reduced()) return finish();
    let settled = false;
    const end = () => {
      if (settled) return;
      settled = true;
      clearTimeout(t);
      a.pause();
      finish();
    };
    const a = animate(el, {
      translateY: open ? ['100%', '0%'] : ['0%', '100%'],
      duration: open ? DUR.ui : 300,
      ease: open ? 'outCubic' : 'inCubic',
      onComplete: end,
    });
    const t = setTimeout(end, DUR.ui + 300);
  });
}

/** New rows drop in from above (the end of the sign-and-send timeline, a reply landing). */
export function dropIn(el: HTMLElement | null): void {
  if (!el || reduced()) return;
  appear([el], { opacity: [0, 1], translateY: [-24, 0], scale: [0.96, 1], duration: 600, ease: 'outBack(1.6)' }, 600);
  const life = el.querySelector<HTMLElement>('.life i');
  if (life) animate(life, { width: ['0%', life.style.width || '100%'], duration: DUR.hero, ease: 'outCubic', delay: 300 });
}

/** Revealed things (peek, inline forms, dialogs) settle in. */
export function settle(el: HTMLElement | null): void {
  if (!el || reduced()) return;
  appear([el], { opacity: [0, 1], translateY: [-8, 0], rotate: [-1.5, 0], duration: 380, ease: 'outCubic' }, 380);
}

/** Toasts rise in. */
export function rise(el: HTMLElement | null): void {
  if (!el || reduced()) return;
  appear([el], { opacity: [0, 1], translateY: [20, 0], duration: 350, ease: 'outBack(2)' }, 350);
}
