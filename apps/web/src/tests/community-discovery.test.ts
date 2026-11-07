import { describe, expect, it } from 'vitest';
import type { CommunitySummary, DiscoveredCommunity } from '@hoi/core-api';
import { matchingCommunities } from '../lib/community-discovery';

const listing = (id: string, name: string): DiscoveredCommunity => ({
  id, name, color: 'mint', description: '', invite: `hoi://join?c=${id}`, joined: false,
});

describe('community name conflicts', () => {
  it('keeps separate community keys with the same normalized name', () => {
    expect(matchingCommunities(' c/Bikes Repair ', [], [listing('b', 'bikes-repair'), listing('a', 'BIKES-REPAIR')]).map((c) => c.id)).toEqual(['a', 'b']);
  });
  it('deduplicates advertisements by key and uses current local membership', () => {
    const local: CommunitySummary = { ...listing('a', 'bikes'), joined: true, createdAt: 0, pinning: false, role: 'member', posts: 0, lastActivity: 0 };
    expect(matchingCommunities('bikes', [local], [listing('a', 'bikes')])).toMatchObject([{ id: 'a', joined: true, invite: 'hoi://join?c=a' }]);
  });
  it('does not infer a match from an empty or unrelated name', () => {
    expect(matchingCommunities('', [], [listing('a', 'bikes')])).toEqual([]);
    expect(matchingCommunities('trains', [], [listing('a', 'bikes')])).toEqual([]);
  });
});
