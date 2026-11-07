import { describe, expect, it } from 'vitest';
import { href, parseHash, screenKey, to, type Route } from '../lib/router';

describe('parseHash', () => {
  it('maps the empty hash and #/ to home', () => {
    expect(parseHash('')).toEqual({ name: 'home' });
    expect(parseHash('#')).toEqual({ name: 'home' });
    expect(parseHash('#/')).toEqual({ name: 'home' });
  });

  it('parses communities, tabs and the ledger', () => {
    expect(parseHash('#/c/ab12')).toEqual({ name: 'community', id: 'ab12', tab: 'posts' });
    expect(parseHash('#/c/ab12/rules')).toEqual({ name: 'community', id: 'ab12', tab: 'rules' });
    expect(parseHash('#/c/ab12/manage')).toEqual({ name: 'community', id: 'ab12', tab: 'manage' });
    expect(parseHash('#/c/ab12/ledger')).toEqual({ name: 'ledger', id: 'ab12' });
    expect(parseHash('#/c/ab12/wiki')).toEqual({ name: 'community', id: 'ab12', tab: 'wiki' });
    expect(parseHash('#/c/ab12/wiki/getting-started')).toEqual({ name: 'community', id: 'ab12', tab: 'wiki', page: 'getting-started' });
    expect(parseHash('#/c/ab12/wiki/Not A Page').name).toBe('notfound');
    expect(parseHash('#/c/ab12/rules/x').name).toBe('notfound');
    expect(parseHash('#/c/').name).toBe('notfound');
  });

  it('starts public communities at #/new and private ones at #/new/private', () => {
    expect(parseHash('#/new')).toEqual({ name: 'new' });
    expect(parseHash(to.start('private'))).toEqual({ name: 'new', kind: 'private' });
    expect(to.start()).toBe('#/new');
    expect(parseHash('#/new/secret').name).toBe('notfound');
  });

  it('keeps the whole invite query for #/join', () => {
    const r = parseHash('#/join?c=abc&g=eyJu&r=wss%3A%2F%2Fx');
    expect(r).toEqual({ name: 'join', query: 'c=abc&g=eyJu&r=wss%3A%2F%2Fx' });
    expect(parseHash('#/join')).toEqual({ name: 'join', query: '' });
  });

  it('decodes the search query', () => {
    expect(parseHash('#/search?q=relay%20costs')).toEqual({ name: 'search', q: 'relay costs' });
    expect(parseHash('#/search')).toEqual({ name: 'search', q: '' });
  });

  it('keeps post searches local unless community search is explicitly selected', () => {
    expect(parseHash(to.search('private & personal'))).toEqual({ name: 'search', q: 'private & personal' });
    expect(parseHash(to.search('c/Bikes', 'communities'))).toEqual({ name: 'search', q: 'c/Bikes', scope: 'communities' });
    expect(parseHash('#/search?scope=unknown&q=local')).toEqual({ name: 'search', q: 'local' });
    expect(parseHash('#/search?scope=communities')).toEqual({ name: 'search', q: '', scope: 'communities' });
  });

  it('supports a registry jump-off and shareable name searches', () => {
    expect(parseHash(to.registry())).toEqual({ name: 'registry', q: '' });
    expect(parseHash(to.registry('c/bikes'))).toEqual({ name: 'registry', q: 'c/bikes' });
    expect(parseHash('#/registry/extra').name).toBe('notfound');
  });

  it('parses posts, people and the simple screens', () => {
    expect(parseHash('#/p/ff00')).toEqual({ name: 'post', id: 'ff00' });
    expect(parseHash('#/u/a7f2')).toEqual({ name: 'person', key: 'a7f2' });
    expect(parseHash('#/u/a7f2?c=c0ff')).toEqual({ name: 'person', key: 'a7f2', community: 'c0ff' });
    expect(parseHash('#/u/a7f2?c=../x')).toEqual({ name: 'person', key: 'a7f2' });
    for (const name of ['you', 'saved', 'new', 'communities', 'server'] as const) expect(parseHash(`#/${name}`)).toEqual({ name });
  });

  it('rejects junk', () => {
    expect(parseHash('#/nope').name).toBe('notfound');
    expect(parseHash('#/p/<script>').name).toBe('notfound');
    expect(parseHash('#/you/extra').name).toBe('notfound');
    expect(parseHash('#/server/extra').name).toBe('notfound');
  });
});

describe('href', () => {
  it('round-trips through parseHash', () => {
    const routes: Route[] = [
      { name: 'home' },
      { name: 'community', id: 'ab', tab: 'posts' },
      { name: 'community', id: 'ab', tab: 'manage' },
      { name: 'ledger', id: 'ab' },
      { name: 'post', id: 'cd' },
      { name: 'person', key: 'ef' },
      { name: 'join', query: 'c=1&g=2' },
      { name: 'search', q: 'a & b' },
      { name: 'search', q: 'c/bikes', scope: 'communities' },
      { name: 'registry', q: 'bikes' },
      { name: 'saved' },
      { name: 'new', kind: 'private' },
      { name: 'you' },
      { name: 'pair', query: 'u=ws%3A%2F%2F127.0.0.1%3A7842&t=x&w=fp@192.168.1.2:7844' },
      { name: 'node', query: 'w=fp@203.0.113.7:7844&p=ab@203.0.113.7:7843' },
      { name: 'companion' },
      { name: 'server' },
    ];
    for (const r of routes) expect(parseHash(href(r))).toEqual(r);
  });

  it('keeps tab switches on one screen key', () => {
    expect(screenKey(parseHash('#/c/ab'))).toBe(screenKey(parseHash('#/c/ab/manage')));
    expect(screenKey(parseHash('#/c/ab'))).not.toBe(screenKey(parseHash('#/c/ab/ledger')));
  });
});
