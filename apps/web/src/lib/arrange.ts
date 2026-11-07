// The person's own arrangement of their communities, on this device only (Settings.favorites,
// Settings.communityOrder): favorites first in the order they put them, then the rest in theirs,
// with communities they haven't placed yet at the end in the order the core lists them.

export interface Arranged<T> {
  favorites: T[];
  rest: T[];
}

export function arrange<T extends { id: string }>(list: T[], favorites: string[] = [], order: string[] = []): Arranged<T> {
  const byId = new Map(list.map((c) => [c.id, c]));
  const fav = favorites.filter((id) => byId.has(id));
  const isFav = new Set(fav);
  const placed = order.filter((id) => byId.has(id) && !isFav.has(id));
  const seen = new Set(placed);
  const unplaced = list.filter((c) => !isFav.has(c.id) && !seen.has(c.id)).map((c) => c.id);
  return { favorites: fav.map((id) => byId.get(id)!), rest: [...placed, ...unplaced].map((id) => byId.get(id)!) };
}

/** `ids` with `id` moved `by` places (−1 up, +1 down), or put just before `before`. */
export function move(ids: string[], id: string, to: { by: number } | { before: string | null }): string[] {
  const from = ids.indexOf(id);
  if (from < 0) return ids;
  const out = ids.filter((x) => x !== id);
  let at: number;
  if ('by' in to) at = Math.max(0, Math.min(out.length, from + to.by));
  else at = to.before === null ? out.length : Math.max(0, out.indexOf(to.before));
  out.splice(at, 0, id);
  return out;
}
