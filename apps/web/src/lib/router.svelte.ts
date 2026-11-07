// Reactive hash router: `router.route` follows location.hash.
import { parseHash, type Route } from './router';

class Router {
  route: Route = $state(parseHash(typeof location === 'undefined' ? '' : location.hash));

  constructor() {
    if (typeof window === 'undefined') return;
    window.addEventListener('hashchange', () => this.#show(parseHash(location.hash)));
  }

  /** Navigate. The route changes at once, not on the next `hashchange`, so whatever the same
   * click opens on the new screen (the composer behind "+ Post") is there to take focus. */
  go(hash: string): void {
    if (location.hash === hash) return;
    location.hash = hash;
    this.#show(parseHash(hash));
  }

  /** Replace the current entry (no new history step). */
  replace(hash: string): void {
    history.replaceState(history.state, '', hash);
    this.route = parseHash(hash);
  }

  #show(next: Route): void {
    const prev = this.route;
    // `go` already showed it; the `hashchange` that follows changes nothing.
    if (JSON.stringify(prev) === JSON.stringify(next)) return;
    this.route = next;
    // New screen → start at the top. Tab switches inside a screen keep the scroll.
    if (!sameScreen(prev, next)) window.scrollTo({ top: 0, behavior: 'instant' });
  }
}

function sameScreen(a: Route, b: Route): boolean {
  if (a.name === 'community' && b.name === 'community') return a.id === b.id;
  return false;
}

export const router = new Router();
