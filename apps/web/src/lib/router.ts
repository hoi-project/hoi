// A tiny hash router. Pure parsing and building here; the reactive part is in router.svelte.ts.
// Hash routes keep the app working under any base path (/app/ on the static site, tauri:// on desktop).

export type CommunityTab = 'posts' | 'rules' | 'wiki' | 'ladder' | 'manage';
export type SearchScope = 'posts' | 'communities';

export type Route =
  | { name: 'home' }
  | { name: 'communities' }
  | { name: 'registry'; q: string }
  /** `page`: a wiki page, on the wiki tab. */
  | { name: 'community'; id: string; tab: CommunityTab; page?: string }
  | { name: 'ledger'; id: string }
  /** Start a community: public by default, `kind: 'private'` for an invite-only one. */
  | { name: 'new'; kind?: 'private' }
  | { name: 'private' }
  | { name: 'join'; query: string }
  | { name: 'public'; query: string }
  | { name: 'post'; id: string }
  | { name: 'you' }
  /** A pairing link opened on this device (a phone's camera scanned it): `query` is after `?`. */
  | { name: 'pair'; query: string }
  /** A public node's link: an address to look for communities at, never a credential. */
  | { name: 'node'; query: string }
  /** The desktop companion: status, traffic and pairing, without the forum. */
  | { name: 'companion' }
  /** A public node (and registry) for a server: its files and commands, made on this device. */
  | { name: 'server' }
  /** The client's privacy notice, readable before the vault is unlocked. */
  | { name: 'privacy' }
  | { name: 'saved' }
  | { name: 'search'; q: string; scope?: 'communities' }
  | { name: 'person'; key: string; community?: string }
  | { name: 'notfound'; path: string };

const ID = /^[0-9a-zA-Z_-]{1,128}$/;
/** A wiki page's name, as the core takes it: 1 to 64 of a-z, 0-9 and -. */
const WIKI_PAGE = /^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/;

/** '#/c/ab12/manage' → { name: 'community', id: 'ab12', tab: 'manage' } */
export function parseHash(hash: string): Route {
  let h = hash.startsWith('#') ? hash.slice(1) : hash;
  if (h === '' || h === '/') return { name: 'home' };
  if (!h.startsWith('/')) h = '/' + h;
  const qi = h.indexOf('?');
  const path = qi >= 0 ? h.slice(0, qi) : h;
  const query = qi >= 0 ? h.slice(qi + 1) : '';
  const parts = path.split('/').filter(Boolean).map(safeDecode);
  const [a, b, c, d] = parts;

  switch (a) {
    case undefined:
      return { name: 'home' };
    case 'communities':
      return parts.length === 1 ? { name: 'communities' } : notFound(path);
    case 'registry':
      return parts.length === 1 ? { name: 'registry', q: new URLSearchParams(query).get('q') ?? '' } : notFound(path);
    case 'c':
      if (!b || !ID.test(b) || parts.length > (c === 'wiki' ? 4 : 3)) return notFound(path);
      if (c === 'wiki') return !d ? { name: 'community', id: b, tab: 'wiki' } : WIKI_PAGE.test(d) ? { name: 'community', id: b, tab: 'wiki', page: d } : notFound(path);
      if (!c || c === 'posts') return { name: 'community', id: b, tab: 'posts' };
      if (c === 'rules' || c === 'ladder' || c === 'manage') return { name: 'community', id: b, tab: c };
      if (c === 'ledger') return { name: 'ledger', id: b };
      return notFound(path);
    case 'new':
      if (parts.length === 1) return { name: 'new' };
      return parts.length === 2 && b === 'private' ? { name: 'new', kind: 'private' } : notFound(path);
    case 'private':
      return parts.length === 1 ? { name: 'private' } : notFound(path);
    case 'join':
      return parts.length === 1 ? { name: 'join', query } : notFound(path);
    case 'public':
      return parts.length === 1 ? { name: 'public', query } : notFound(path);
    case 'p':
      return b && ID.test(b) && parts.length === 2 ? { name: 'post', id: b } : notFound(path);
    case 'u': {
      if (!b || !ID.test(b) || parts.length !== 2) return notFound(path);
      // `?c=<community>`: their place in that community, e.g. from an "ask to be let in" link
      const c = new URLSearchParams(query).get('c');
      return c && ID.test(c) ? { name: 'person', key: b, community: c } : { name: 'person', key: b };
    }
    case 'you':
      return parts.length === 1 ? { name: 'you' } : notFound(path);
    case 'pair':
      return parts.length === 1 ? { name: 'pair', query } : notFound(path);
    case 'node':
      return parts.length === 1 ? { name: 'node', query } : notFound(path);
    case 'companion':
      return parts.length === 1 ? { name: 'companion' } : notFound(path);
    case 'server':
      return parts.length === 1 ? { name: 'server' } : notFound(path);
    case 'privacy':
      return parts.length === 1 ? { name: 'privacy' } : notFound(path);
    case 'saved':
      return parts.length === 1 ? { name: 'saved' } : notFound(path);
    case 'search': {
      const params = new URLSearchParams(query);
      return parts.length === 1 ? { name: 'search', q: params.get('q') ?? '', ...(params.get('scope') === 'communities' ? { scope: 'communities' as const } : {}) } : notFound(path);
    }
    default:
      return notFound(path);
  }
}

function notFound(path: string): Route {
  return { name: 'notfound', path };
}

function safeDecode(s: string): string {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}

/** Build a hash href for a route: href({ name: 'post', id }) → '#/p/<id>' */
export function href(route: Route): string {
  switch (route.name) {
    case 'home': return '#/';
    case 'communities': return '#/communities';
    case 'registry': return route.q ? `#/registry?q=${encodeURIComponent(route.q)}` : '#/registry';
    case 'community': return route.tab === 'posts' ? `#/c/${route.id}` : route.tab === 'wiki' && route.page ? `#/c/${route.id}/wiki/${route.page}` : `#/c/${route.id}/${route.tab}`;
    case 'ledger': return `#/c/${route.id}/ledger`;
    case 'new': return route.kind === 'private' ? '#/new/private' : '#/new';
    case 'private': return '#/private';
    case 'join': return route.query ? `#/join?${route.query}` : '#/join';
    case 'public': return route.query ? `#/public?${route.query}` : '#/public';
    case 'post': return `#/p/${route.id}`;
    case 'you': return '#/you';
    case 'pair': return route.query ? `#/pair?${route.query}` : '#/pair';
    case 'node': return route.query ? `#/node?${route.query}` : '#/node';
    case 'companion': return '#/companion';
    case 'server': return '#/server';
    case 'privacy': return '#/privacy';
    case 'saved': return '#/saved';
    case 'search': {
      const query = new URLSearchParams();
      if (route.q) query.set('q', route.q);
      if (route.scope === 'communities') query.set('scope', route.scope);
      return `#/search${query.size ? `?${query}` : ''}`;
    }
    case 'person': return route.community ? `#/u/${route.key}?c=${route.community}` : `#/u/${route.key}`;
    case 'notfound': return '#/';
  }
}

/** Shorthands used all over the UI. */
export const to = {
  home: () => '#/',
  communities: () => '#/communities',
  registry: (q = '') => href({ name: 'registry', q }),
  community: (id: string, tab: CommunityTab = 'posts') => href({ name: 'community', id, tab }),
  wiki: (id: string, page?: string) => href({ name: 'community', id, tab: 'wiki', page }),
  ledger: (id: string) => href({ name: 'ledger', id }),
  post: (id: string) => href({ name: 'post', id }),
  person: (key: string, community?: string) => href({ name: 'person', key, community }),
  join: (query = '') => href({ name: 'join', query }),
  public: (id: string, name: string, protocol: string) => href({ name: 'public', query: new URLSearchParams({ c: id, name, p: protocol }).toString() }),
  search: (q: string, scope: SearchScope = 'posts') => href({ name: 'search', q, ...(scope === 'communities' ? { scope } : {}) }),
  you: () => href({ name: 'you' }),
  server: () => href({ name: 'server' }),
  privacy: () => href({ name: 'privacy' }),
  start: (kind?: 'private') => href({ name: 'new', ...(kind ? { kind } : {}) }),
};

/** A key that changes when the screen (not just a tab) changes, for view transitions. */
export function screenKey(r: Route): string {
  switch (r.name) {
    case 'community': return `community:${r.id}`;
    case 'ledger': return `ledger:${r.id}`;
    case 'post': return `post:${r.id}`;
    case 'person': return `person:${r.key}:${r.community ?? ''}`;
    case 'join': return `join:${r.query}`;
    case 'public': return `public:${r.query}`;
    case 'pair': return `pair:${r.query}`;
    case 'node': return `node:${r.query}`;
    case 'search': return 'search';
    default: return r.name;
  }
}
