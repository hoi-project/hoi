// "+ Post", wherever it is pressed: the composer opens where the post will land (Home, or the
// community on screen) and the title field has the cursor within the same tap. Phones raise
// their keyboard only for a focus made during the tap itself, so the screen and the composer
// are drawn synchronously first (the router changes its route at once, `flushSync` draws it).
import type { Hex } from '@hoi/core-api';
import { flushSync } from 'svelte';
import { app } from './app.svelte';
import { reduced } from './motion';
import { router } from './router.svelte';
import { to } from './router';

/** Below this width the composer is a sheet over the whole screen. */
export const SHEET_QUERY = '(max-width: 640px)';

export function composerIsSheet(): boolean {
  return typeof window !== 'undefined' && !!window.matchMedia?.(SHEET_QUERY).matches;
}

/** Opens the composer for the screen in view, or for `community`, and focuses its title. */
export function startPost(community?: Hex): void {
  const r = router.route;
  const target = community ?? (r.name === 'community' ? r.id : undefined);
  if (target) {
    if (r.name !== 'community' || r.id !== target || r.tab !== 'posts') router.go(to.community(target));
  } else if (r.name !== 'home') {
    router.go(to.home());
  }
  // Already open for the same place: keep what was typed, just go back to it.
  if (!(app.composer.open && app.composer.community === target)) app.openComposer(target);
  flushSync();
  focusComposer();
}

/** The open composer's title (or whichever of its fields has text), in view and focused. */
export function focusComposer(): void {
  const title = document.getElementById('ctitle') as HTMLInputElement | null;
  if (!title) return;
  const body = document.getElementById('ctext') as HTMLTextAreaElement | null;
  const field = title.value && body ? body : title;
  field.focus({ preventScroll: true });
  if (!composerIsSheet()) title.closest('.composer')?.scrollIntoView({ block: 'start', behavior: reduced() ? 'auto' : 'smooth' });
}
