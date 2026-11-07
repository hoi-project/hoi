// Communities this device can see without a registry: what connected nodes list in their
// catalogs and what this device already holds. A registry is one source among
// others; when it is gone, names are still findable here, and when it answers, a same-name
// community under another key is shown as the parallel community it is, never merged.
import type { DiscoveredCommunity } from '@hoi/core-api';
import { registryQuery } from './registry';

export type NearbyLabel = 'not registered' | 'same name, other key' | 'from a connected node';

export interface NearbyCommunity {
  id: string;
  name: string;
  description: string;
  invite: string;
  joined: boolean;
  label: NearbyLabel;
}

/** The canonical public spelling of a display name, when it has one. */
const canonical = (name: string): string => registryQuery(name);

/**
 * Listings whose canonical name starts with `prefix`, labelled against what the registry said:
 * `registered` is a set of keys the registry listed; `null` means it did not answer (or is not
 * configured), so nothing can be called unregistered.
 */
export function nearbyCommunities(prefix: string, discovered: DiscoveredCommunity[], registered: Map<string, string> | null): NearbyCommunity[] {
  const want = registryQuery(prefix);
  const seen = new Set<string>();
  const out: NearbyCommunity[] = [];
  for (const d of discovered) {
    const name = canonical(d.name);
    if (!name.startsWith(want) || seen.has(d.id)) continue;
    seen.add(d.id);
    if (registered?.get(d.id) !== undefined) continue; // the registry lists this very key: shown there
    let label: NearbyLabel;
    if (registered === null) label = 'from a connected node';
    else if ([...registered.values()].includes(name)) label = 'same name, other key';
    else label = 'not registered';
    out.push({ id: d.id, name: d.name, description: d.description, invite: d.invite, joined: d.joined, label });
  }
  return out.sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id));
}

/** How a listing is matched by `searchDirectory`, best first. */
const rank = (d: DiscoveredCommunity, words: string[]): number => {
  const name = canonical(d.name);
  const text = `${name} ${d.description.toLowerCase()}`;
  if (!words.every((w) => text.includes(w))) return -1;
  if (words.length && name.startsWith(words[0])) return 3;
  if (words.every((w) => name.includes(w))) return 2;
  return 1;
};

/**
 * The communities nodes told this device about (live and remembered), matched by name and
 * description: every word must appear, the name's own start counts most. Without a query, all
 * of them: the ones a connected node lists now first, then the most recently seen. Same names
 * under other keys stay separate entries.
 */
export function searchDirectory(query: string, discovered: DiscoveredCommunity[]): DiscoveredCommunity[] {
  const words = registryQuery(query).split(/\s+/).filter(Boolean);
  const seen = new Set<string>();
  const out: { d: DiscoveredCommunity; r: number }[] = [];
  for (const d of discovered) {
    if (seen.has(d.id)) continue;
    seen.add(d.id);
    const r = rank(d, words);
    if (r >= 0) out.push({ d, r });
  }
  return out
    .sort((a, b) => b.r - a.r || Number(!!b.d.live) - Number(!!a.d.live) || (b.d.seen ?? 0) - (a.d.seen ?? 0) || a.d.name.localeCompare(b.d.name) || a.d.id.localeCompare(b.d.id))
    .map((x) => x.d);
}
