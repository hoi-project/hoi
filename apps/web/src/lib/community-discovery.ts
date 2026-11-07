import type { CommunitySummary, DiscoveredCommunity } from '@hoi/core-api';
import { normalizeCommunityName } from './util';

/** Preserve distinct keys even when their names match, and prefer local membership state. */
export function matchingCommunities(
  name: string,
  local: CommunitySummary[],
  discovered: DiscoveredCommunity[],
): (Pick<DiscoveredCommunity, 'id' | 'name' | 'joined'> & { invite?: string })[] {
  const clean = normalizeCommunityName(name);
  if (!clean) return [];
  const byId = new Map(discovered.map((c) => [c.id, c]));
  for (const c of local) byId.set(c.id, { ...byId.get(c.id), ...c, invite: byId.get(c.id)?.invite ?? '' });
  return [...byId.values()].filter((c) => normalizeCommunityName(c.name) === clean)
    .sort((a, b) => a.id.localeCompare(b.id));
}
