import { describe, expect, it } from 'vitest';
import type { DiscoveredCommunity } from '@hoi/core-api';
import { nearbyCommunities, searchDirectory } from '../lib/nearby';

const d = (id: string, name: string, joined = false): DiscoveredCommunity => ({ id, name, color: 'mint', description: '', invite: `hoi://join?c=${id}`, joined }) as DiscoveredCommunity;
const bikes = 'a'.repeat(64);
const otherBikes = 'b'.repeat(64);
const garden = 'c'.repeat(64);

describe('communities findable without a registry', () => {
  it('matches the familiar c/name spelling against what connected nodes list', () => {
    const found = nearbyCommunities('C/Bik', [d(bikes, 'Bikes Café'), d(garden, 'garden')], null);
    expect(found.map((f) => f.id)).toEqual([bikes]);
    expect(found[0].label).toBe('from a connected node');
  });

  it('hides keys the registry already lists and labels the rest against its answer', () => {
    const registered = new Map([[bikes, 'bikes']]);
    const found = nearbyCommunities('', [d(bikes, 'bikes'), d(otherBikes, 'bikes', true), d(garden, 'garden')], registered);
    expect(found.map((f) => [f.name, f.label, f.joined])).toEqual([
      ['bikes', 'same name, other key', true],
      ['garden', 'not registered', false],
    ]);
  });

  it('lists each key once, sorted by name then key', () => {
    const found = nearbyCommunities('', [d(garden, 'garden'), d(garden, 'garden'), d(bikes, 'bikes')], null);
    expect(found.map((f) => f.id)).toEqual([bikes, garden]);
  });
});

describe('searching what nodes listed, with no registry', () => {
  const listing = (id: string, name: string, description = '', live = true, seen = 0): DiscoveredCommunity =>
    ({ id, name, color: 'mint', description, invite: `hoi://join?c=${id}`, joined: false, live, seen }) as DiscoveredCommunity;
  const repair = 'd'.repeat(64);

  it('matches words in the name or the description, names starting with the query first', () => {
    const all = [listing(garden, 'garden', 'bikes welcome too'), listing(bikes, 'Bikes Café'), listing(repair, 'repair', 'fixing bikes and radios')];
    expect(searchDirectory('bik', all).map((d) => d.id)).toEqual([bikes, garden, repair]);
    expect(searchDirectory('c/BIKES radios', all).map((d) => d.id)).toEqual([repair]);
    expect(searchDirectory('boats', all)).toEqual([]);
  });

  it('lists everything without a query: live listings first, then the most recently seen', () => {
    const all = [listing(garden, 'garden', '', false, 10), listing(bikes, 'bikes', '', false, 20), listing(repair, 'repair', '', true, 1), listing(repair, 'repair')];
    expect(searchDirectory('', all).map((d) => d.id)).toEqual([repair, bikes, garden]);
  });
});
