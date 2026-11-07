// Plain documentation data and deterministic retrieval. No model or DOM dependency.
export interface Passage {
  id: string;
  url: string;
  parent: string;
  title: string;
  page: string;
  text: string;
  links: string[];
}
export interface Corpus { version: string; passages: Passage[] }
export interface Topic { url: string; title: string; level: number }
export interface Turn { question: string; answer: string }
export interface Trail extends Turn {
  version: string;
  original: string;
  from: string;
  fromTitle: string;
  to: string;
  at: number;
}
export interface Reply {
  status: 'answered' | 'not-covered' | 'off-topic';
  answer: string;
  sources: number[];
  next: { source: number; reason: string; question: string }[];
}

const STOP = new Set(('a an and are as at be been but by can could do does for from had has have how i if in into is it its me my of on or our should so some than that the their them then there these they this those to us was we what when where which who why will with would you your explain please here elsewhere documentation section simply work works').split(' '));
const ALIASES: Record<string, string[]> = {
  recover: ['recovery', 'root', 'friend'], recovery: ['recover', 'root', 'friend'],
  delete: ['removal', 'expiry', 'tombstone'], deleted: ['removal', 'expiry', 'tombstone'],
  expire: ['expiry', 'retention'], expires: ['expiry', 'retention'],
  message: ['mls', 'relay', 'private'], encryption: ['encrypted', 'mls'],
  device: ['delegation', 'pairing'], offline: ['sync', 'storage'],
};
export function words(text: string): string[] {
  return [...new Set((text.toLowerCase().normalize('NFKD').replace(/[\u0300-\u036f]/g, '').match(/[a-z0-9][a-z0-9_-]+/g) ?? [])
    .filter((w) => !STOP.has(w)).map((w) => w.length > 4 && w.endsWith('s') ? w.slice(0, -1) : w))];
}
export function inTopic(p: Passage, topic: Topic): boolean {
  return p.url === topic.url || (topic.level === 2 && p.parent === topic.url);
}

/** Always include the current section; rank related passages without requiring every word. */
export function retrieve(corpus: Corpus, topic: Topic, question: string): Passage[] {
  const terms = words(question + ' ' + topic.title);
  const expanded = new Set(terms.flatMap((w) => [w, ...(ALIASES[w] ?? [])]));
  const local = corpus.passages.filter((p) => inTopic(p, topic));
  const linked = new Set(local.flatMap((p) => p.links));
  const frequency = new Map<string, number>();
  for (const p of corpus.passages) {
    for (const word of words(p.title + ' ' + p.text)) {
      if (expanded.has(word)) frequency.set(word, (frequency.get(word) ?? 0) + 1);
    }
  }
  const rank = (p: Passage) => {
    const title = new Set(words(p.page + ' ' + p.title));
    const body = new Set(words(p.text));
    let score = 0;
    for (const word of expanded) {
      const idf = Math.log(1 + corpus.passages.length / (1 + (frequency.get(word) ?? 0)));
      score += idf * (title.has(word) ? 4 : body.has(word) ? 1 : 0) * (terms.includes(word) ? 1 : 0.5);
    }
    if (linked.has(p.url) || linked.has(p.url.split('#')[0])) score += 4;
    return score;
  };
  const sort = (items: Passage[]) => items.map((p) => ({ p, score: rank(p) }))
    .sort((a, b) => b.score - a.score || a.p.id.localeCompare(b.p.id));
  const chosen = sort(local).slice(0, 3).map(({ p }) => p);
  // Keep destinations diverse so one long page cannot occupy every suggestion.
  const seen = new Set(chosen.map((p) => p.url));
  for (const { p, score } of sort(corpus.passages.filter((p) => !inTopic(p, topic)))) {
    if (score <= 0 || seen.has(p.url)) continue;
    chosen.push(p);
    seen.add(p.url);
    if (chosen.length >= 7) break;
  }
  return chosen;
}

export function replySchema(count: number) {
  const source = { type: 'integer', enum: Array.from({ length: count }, (_, i) => i + 1) };
  return {
    type: 'object', additionalProperties: false,
    required: ['status', 'answer', 'sources', 'next'],
    properties: {
      status: { type: 'string', enum: ['answered', 'not-covered', 'off-topic'] },
      answer: { type: 'string' },
      sources: { type: 'array', items: source, maxItems: 7 },
      next: {
        type: 'array', maxItems: 3,
        items: {
          type: 'object', additionalProperties: false, required: ['source', 'reason', 'question'],
          properties: { source, reason: { type: 'string' }, question: { type: 'string' } },
        },
      },
    },
  };
}

/** Validate again at the boundary; the UI never accepts URLs or HTML from the model. */
export function parseReply(raw: string, passages: Passage[], topic: Topic): Reply {
  if (raw.length > 30000) throw new Error('Response too large');
  const data = JSON.parse(raw) as Reply;
  if (!data || !['answered', 'not-covered', 'off-topic'].includes(data.status) ||
      typeof data.answer !== 'string' || !data.answer.trim() ||
      !Array.isArray(data.sources) || !Array.isArray(data.next)) throw new Error('Invalid response');
  const valid = (n: number) => Number.isInteger(n) && n >= 1 && n <= passages.length;
  if (data.sources.some((n) => !valid(n))) throw new Error('Unknown citation');
  if (data.status === 'answered' && !data.sources.some((n) => inTopic(passages[n - 1], topic))) {
    throw new Error('Missing section citation');
  }
  const seen = new Set<string>();
  const next = data.status === 'answered' ? data.next.filter((n) => {
    if (!n || !valid(n.source) || typeof n.reason !== 'string' || typeof n.question !== 'string' ||
        !n.reason.trim() || !n.question.trim()) return false;
    const url = passages[n.source - 1].url;
    if (url === topic.url || seen.has(url)) return false;
    seen.add(url);
    return true;
  }).slice(0, 3).map((n) => ({ ...n, reason: n.reason.slice(0, 450), question: n.question.slice(0, 1200) })) : [];
  return { status: data.status, answer: data.answer.slice(0, 7000), sources: [...new Set(data.sources)].slice(0, 7), next };
}

export const TRAIL_KEY = 'hoi.wiki.explanation';
export const TRAIL_TTL = 15 * 60 * 1000;
const wikiURL = (s: unknown): s is string => typeof s === 'string' && /^\/wiki\/(?:[a-z0-9-]+\/)?#[a-zA-Z0-9_-]+$/.test(s);
export function parseTrail(raw: string | null, now = Date.now()): Trail | null {
  if (!raw || raw.length > 12000) return null;
  try {
    const t = JSON.parse(raw) as Trail;
    if (!t || !wikiURL(t.from) || !wikiURL(t.to) || !Number.isFinite(t.at) || now - t.at > TRAIL_TTL || t.at > now + 1000) return null;
    for (const [key, max] of [['version', 64], ['original', 1200], ['question', 1200], ['answer', 4000], ['fromTitle', 300]] as const) {
      if (typeof t[key] !== 'string' || !t[key].length || t[key].length > max) return null;
    }
    return t;
  } catch { return null; }
}

export function takeTrail(url: string): Trail | null {
  try {
    const raw = sessionStorage.getItem(TRAIL_KEY);
    sessionStorage.removeItem(TRAIL_KEY);
    const trail = parseTrail(raw);
    return trail?.to === url ? trail : null;
  } catch { return null; }
}
export function saveTrail(trail: Trail): boolean {
  try { sessionStorage.setItem(TRAIL_KEY, JSON.stringify(trail)); return true; }
  catch { return false; }
}
