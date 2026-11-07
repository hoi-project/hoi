// The communities list as the person arranged it on this device: favorites, their own order,
// and private communities kept out of sight until asked for, like a private browsing window.
// Favorites and order live in the core's local settings (encrypted with everything else, never
// sent, never in a backup); whether private ones show is forgotten when the app closes.
import type { CommunitySummary } from '@hoi/core-api';
import { app } from './app.svelte';
import { arrange, move } from './arrange';

class Nav {
  /** Private communities show in the list. Off at every start. */
  privateShown = $state(false);

  /** Joined public communities, as the person arranged them. */
  get arranged() {
    return arrange(this.publicJoined, app.settings?.favorites, app.settings?.communityOrder);
  }

  get publicJoined(): CommunitySummary[] {
    return app.joined.filter((c) => !this.isPrivate(c.id));
  }

  get privateJoined(): CommunitySummary[] {
    return app.joined.filter((c) => this.isPrivate(c.id));
  }

  isPrivate(id: string): boolean {
    return app.privateGroups.some((g) => g.community === id);
  }

  isFavorite(id: string): boolean {
    return !!app.settings?.favorites?.includes(id);
  }

  async toggleFavorite(id: string): Promise<void> {
    const favs = app.settings?.favorites ?? [];
    await this.#save({ favorites: favs.includes(id) ? favs.filter((x) => x !== id) : [...favs, id] });
  }

  /** Moves a community within its group (favorites or the rest). */
  async move(id: string, to: { by: number } | { before: string | null }): Promise<void> {
    const { favorites, rest } = this.arranged;
    if (favorites.some((c) => c.id === id)) await this.#save({ favorites: move(favorites.map((c) => c.id), id, to) });
    else await this.#save({ communityOrder: move(rest.map((c) => c.id), id, to) });
  }

  async #save(patch: { favorites?: string[]; communityOrder?: string[] }): Promise<void> {
    // Shown at once; the core's answer settles it (or puts it back, with a toast).
    const before = app.settings;
    if (before) app.settings = { ...before, ...patch };
    if (!(await app.setSettings(patch)) && before) app.settings = before;
  }
}

export const nav = new Nav();
