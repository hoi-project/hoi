// An operator's notice as the app writes and shows it: the same fields, limits, words
// and canonical bytes as `hoi_types::notice` in Rust, so what the kit page or the setup app
// writes is what a node carries and what every client shows. The hash (BLAKE3 over the
// canonical bytes) is the node's to compute; the size budget is checked here exactly.
// Who and where (operator, contact, country, provider) are asked for, never forced: left empty,
// the fixed sentences say so; filled in, they are shown to everyone who connects to the node.

export interface NodeNotice {
  v: number;
  /** The node's label, as its operator calls it. */
  node: string;
  /** Name or pseudonym; empty when the operator keeps it to themselves. */
  operator: string;
  /** Email address, page or handle; empty when not given. */
  contact: string;
  /** ISO 3166-1 alpha-2, upper case; empty when not given. */
  country: string;
  provider?: string;
  processes: string;
  ipLogging: boolean;
  retentionDays: number;
  purpose?: string;
  communityRules?: string;
  extra?: string;
  /** YYYY-MM-DD. */
  updated: string;
  /** The node keeps encrypted pieces for others (`hoi-node --hold-pieces`); the node sets it from how it runs. */
  pieces?: boolean;
}

export interface NoticeProblem {
  field: string;
  message: string;
}

export interface NoticeSection {
  title: string;
  text: string;
}

export const NOTICE_VERSION = 1;
export const MAX_NOTICE_BYTES = 1536;
export const LIMITS = { node: 64, operator: 120, contact: 200, provider: 120, processes: 600, purpose: 400, communityRules: 600, extra: 600 } as const;
export const MAX_RETENTION_DAYS = 3650;
export const DEFAULT_PROCESSES =
  'Nothing beyond that: the node keeps the open communities it carries (their posts and media, which are public) and the addresses of the other public nodes it reached.';
export const DEFAULT_PURPOSE = 'The data is used only to operate this node and the communities it carries. It is not sold or used for advertising.';
export const OTHER_PARTICIPANTS =
  'Members of a community can see your IP address through direct peer-to-peer connections. The operator cannot control what they do with it.';
export const BEYOND_THIS_NODE =
  "This node is one way into the network, not the only one. When you join a community, open an invite or a link, read, post, reply or sync, your device may also connect to other nodes and participants anywhere on the internet that carry those communities, and which ones changes over time. Public nodes that carry a notice show you theirs before your device tells them anything about you; participants' devices, and nodes without a notice, show none.";

export const PIECES =
  "This node also keeps encrypted pieces of posts and encrypted messages of private communities for other people's devices, and hands them back when asked. It cannot read them. Each is deleted when it expires, after 400 days at most.";

/** A notice with the fixed defaults filled in, for an operator to complete. */
export function noticeTemplate(node: string, today = isoDate(new Date())): NodeNotice {
  return { v: NOTICE_VERSION, node, operator: '', contact: '', country: '', processes: DEFAULT_PROCESSES, ipLogging: false, retentionDays: 0, purpose: DEFAULT_PURPOSE, updated: today };
}

export function isoDate(d: Date): string {
  return d.toISOString().slice(0, 10);
}

const chars = (s: string) => [...s].length;
const clean = (s: string, multiline: boolean) => [...s].every((c) => !/[\p{Cc}]/u.test(c) || (multiline && c === '\n'));
const opt = (s: string | undefined) => (s?.trim() ? s.trim() : undefined);

export function validDate(s: string): boolean {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(s);
  if (!m) return false;
  const [y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])];
  return y >= 2024 && y <= 2999 && mo >= 1 && mo <= 12 && d >= 1 && d <= 31;
}

/** Everything that keeps a notice from being used, in field order, with the node's own words. */
export function validateNotice(n: NodeNotice): NoticeProblem[] {
  const out: NoticeProblem[] = [];
  if (n.v !== NOTICE_VERSION) out.push({ field: 'v', message: `This software writes notice version ${NOTICE_VERSION}.` });
  const text = (field: keyof typeof LIMITS, value: string | undefined, required: boolean, multiline: boolean, ask: string) => {
    const v = (value ?? '').trim();
    if (!v) {
      if (required) out.push({ field, message: ask });
    } else if (chars(v) > LIMITS[field]) out.push({ field, message: `At most ${LIMITS[field]} characters.` });
    else if (!clean(v, multiline)) out.push({ field, message: 'No control characters.' });
  };
  text('node', n.node, false, false, '');
  text('operator', n.operator, false, false, '');
  text('contact', n.contact, false, false, '');
  text('provider', n.provider, false, false, '');
  text('processes', n.processes, true, true, 'Say what this node processes.');
  text('purpose', n.purpose, false, true, '');
  text('communityRules', n.communityRules, false, true, '');
  text('extra', n.extra, false, true, '');
  const country = n.country.trim();
  if (country && !/^[A-Z]{2}$/.test(country)) out.push({ field: 'country', message: "Give the server's country as its two-letter ISO code, or leave it empty." });
  if (n.ipLogging && n.retentionDays === 0) out.push({ field: 'retentionDays', message: 'Say for how many days IP addresses are kept.' });
  else if (n.retentionDays > MAX_RETENTION_DAYS) out.push({ field: 'retentionDays', message: `At most ${MAX_RETENTION_DAYS} days.` });
  else if (!n.ipLogging && n.retentionDays !== 0) out.push({ field: 'retentionDays', message: 'Retention days go with IP logging.' });
  if (!validDate(n.updated)) out.push({ field: 'updated', message: 'Date the notice as YYYY-MM-DD.' });
  if (!out.length) {
    const size = canonicalNotice(n).length;
    if (size > MAX_NOTICE_BYTES) out.push({ field: 'notice', message: `Too long for a node link: shorten it by ${size - MAX_NOTICE_BYTES} bytes (at most ${MAX_NOTICE_BYTES}).` });
  }
  return out;
}

// ---- canonical DAG-CBOR, the way hoi_types::cbor writes it: shortest heads, keys sorted by
// length then bytes, text as UTF-8. Only what a notice needs.
function head(major: number, n: number, out: number[]): void {
  const m = major << 5;
  if (n < 24) out.push(m | n);
  else if (n < 0x100) out.push(m | 24, n);
  else if (n < 0x10000) out.push(m | 25, n >> 8, n & 0xff);
  else out.push(m | 26, (n >>> 24) & 0xff, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff);
}
const utf8 = new TextEncoder();
function pushText(s: string, out: number[]): void {
  const b = utf8.encode(s);
  head(3, b.length, out);
  for (const x of b) out.push(x);
}

/** The bytes a node hashes and carries: `NodeNotice::canonical` in Rust, byte for byte. Operator,
 *  contact and country are always written, empty when not given, so older readers still read it. */
export function canonicalNotice(n: NodeNotice): Uint8Array {
  const entries: [string, (o: number[]) => void][] = [
    ['v', (o) => head(0, n.v, o)],
    ['n', (o) => pushText(n.node.trim(), o)],
    ['o', (o) => pushText(n.operator.trim(), o)],
    ['c', (o) => pushText(n.contact.trim(), o)],
    ['k', (o) => pushText(n.country.trim(), o)],
    ['p', (o) => pushText(n.processes.trim(), o)],
    ['l', (o) => o.push(n.ipLogging ? 0xf5 : 0xf4)],
    ['r', (o) => head(0, n.retentionDays, o)],
    ['d', (o) => pushText(n.updated.trim(), o)],
  ];
  for (const [key, value] of [['h', n.provider], ['u', n.purpose], ['g', n.communityRules], ['x', n.extra]] as const) {
    const v = opt(value);
    if (v !== undefined) entries.push([key, (o) => pushText(v, o)]);
  }
  if (n.pieces) entries.push(['s', (o) => o.push(0xf5)]);
  entries.sort((a, b) => a[0].length - b[0].length || (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
  const out: number[] = [];
  head(5, entries.length, out);
  for (const [key, write] of entries) {
    pushText(key, out);
    write(out);
  }
  return Uint8Array.from(out);
}

/** The notice as people read it: the operator's words inside the client's fixed sentences. Where
 *  the operator left who or where out, the sentences say so. */
export function noticeSections(n: NodeNotice): NoticeSection[] {
  const operator = n.operator.trim();
  const contact = n.contact.trim();
  const country = n.country.trim();
  const provider = opt(n.provider);
  const who =
    operator && contact ? `This node is operated by ${operator}. Contact: ${contact}.`
    : operator ? `This node is operated by ${operator}, who gave no contact.`
    : contact ? `The operator of this node gave no name. Contact: ${contact}.`
    : 'The operator of this node chose not to say who they are or how to reach them.';
  const where =
    country && provider ? `The server is located in ${country} and hosted by ${provider}.`
    : country ? `The server is located in ${country}.`
    : provider ? `The server is hosted by ${provider}. The operator did not say in which country.`
    : 'The operator did not say where the server is located or who hosts it.';
  const out: NoticeSection[] = [
    { title: 'Operator', text: who },
    { title: 'Server location', text: where },
    {
      title: 'What this node processes',
      text: `To connect you with communities, this node processes your IP address, connection times and technical connection data. ${n.processes.trim()}`,
    },
    ...(n.pieces ? [{ title: 'Encrypted pieces', text: PIECES }] : []),
    {
      title: 'Logging and retention',
      text: n.ipLogging
        ? `IP addresses are logged for ${n.retentionDays} day${n.retentionDays === 1 ? '' : 's'} for security and abuse prevention, then deleted.`
        : 'IP addresses are not logged.',
    },
    { title: 'Purpose', text: opt(n.purpose) ?? DEFAULT_PURPOSE },
    { title: 'Other participants', text: OTHER_PARTICIPANTS },
    { title: 'Beyond this node', text: BEYOND_THIS_NODE },
  ];
  const rules = opt(n.communityRules);
  if (rules) out.push({ title: 'Community rules', text: rules });
  const extra = opt(n.extra);
  if (extra) out.push({ title: 'More', text: extra });
  out.push({
    title: 'Your rights',
    text: contact
      ? 'You can ask the operator what data about you this node holds and request its deletion via the contact above.'
      : 'The operator gave no contact, so this notice offers no way to ask them what data about you this node holds or to request its deletion. If that matters to you, decline and use another node.',
  });
  return out;
}

/** Who wrote a notice, in a few words: `by Lena · contact lena@example.org` (`NodeNotice::byline`). */
export function noticeByline(n: Pick<NodeNotice, 'operator' | 'contact' | 'country'>): string {
  const operator = n.operator.trim() ? `by ${n.operator.trim()}` : 'by an operator who gave no name';
  const contact = n.contact.trim() ? `contact ${n.contact.trim()}` : 'no contact given';
  return n.country.trim() ? `${operator} · ${contact} · server in ${n.country.trim()}` : `${operator} · ${contact}`;
}

/** What an operator left out of who and where, in the words the editors show next to the preview. */
export function noticeOmissions(n: Pick<NodeNotice, 'operator' | 'contact' | 'country'>): string[] {
  const out: string[] = [];
  if (!n.operator.trim()) out.push('no name: people read that you chose not to say who you are');
  if (!n.contact.trim()) out.push('no contact: people read that they cannot reach you about their data');
  if (!n.country.trim()) out.push('no country: people read that you did not say where the server is');
  return out;
}

export function noticeTitle(n: Pick<NodeNotice, 'node'>): string {
  const name = n.node.trim();
  return name ? `Join notice – ${name}` : 'Join notice';
}

/** `hash` is the hex the node reports (its first 12 characters are shown); unknown before install. */
export function noticeFooter(n: Pick<NodeNotice, 'updated'>, hash?: string): string {
  return `Notice version ${hash ? hash.slice(0, 12) : '(set when the node starts)'}, last updated ${n.updated.trim()}.`;
}

/** The whole notice as plain text, the way `hoi-node admin notice check` prints it. */
export function noticeText(n: NodeNotice, hash?: string): string {
  return `${noticeTitle(n)}\n${noticeSections(n).map((s) => `\n${s.title}. ${s.text}\n`).join('')}\n${noticeFooter(n, hash)}\n`;
}

/** The JSON file the node reads (`hoi-node --notice`), with the empty optionals left out. */
export function noticeJson(n: NodeNotice): string {
  const o: Record<string, unknown> = {
    v: n.v, node: n.node.trim(), operator: n.operator.trim(), contact: n.contact.trim(), country: n.country.trim(),
    processes: n.processes.trim(), ipLogging: n.ipLogging, retentionDays: n.retentionDays, updated: n.updated.trim(),
  };
  for (const key of ['provider', 'purpose', 'communityRules', 'extra'] as const) {
    const v = opt(n[key]);
    if (v !== undefined) o[key] = v;
  }
  if (n.pieces) o.pieces = true;
  return JSON.stringify(o, null, 2) + '\n';
}

// Every ISO 3166-1 alpha-2 code, for the picker; names come from the browser, in alphabetical
// order, so no country is put first.
const ISO_CODES =
  'AD AE AF AG AI AL AM AO AQ AR AS AT AU AW AX AZ BA BB BD BE BF BG BH BI BJ BL BM BN BO BQ BR BS BT BV BW BY BZ CA CC CD CF CG CH CI CK CL CM CN ' +
  'CO CR CU CV CW CX CY CZ DE DJ DK DM DO DZ EC EE EG EH ER ES ET FI FJ FK FM FO FR GA GB GD GE GF GG GH GI GL GM GN GP GQ GR GS GT GU GW GY HK ' +
  'HM HN HR HT HU ID IE IL IM IN IO IQ IR IS IT JE JM JO JP KE KG KH KI KM KN KP KR KW KY KZ LA LB LC LI LK LR LS LT LU LV LY MA MC MD ME MF MG ' +
  'MH MK ML MM MN MO MP MQ MR MS MT MU MV MW MX MY MZ NA NC NE NF NG NI NL NO NP NR NU NZ OM PA PE PF PG PH PK PL PM PN PR PS PT PW PY QA RE RO ' +
  'RS RU RW SA SB SC SD SE SG SH SI SJ SK SL SM SN SO SR SS ST SV SX SY SZ TC TD TF TG TH TJ TK TL TM TN TO TR TT TV TW TZ UA UG UM US UY UZ VA ' +
  'VC VE VG VI VN VU WF WS YE YT ZA ZM ZW';

function regionNames(): (code: string) => string {
  try {
    const names = new Intl.DisplayNames(['en'], { type: 'region' });
    return (code) => names.of(code) ?? code;
  } catch {
    return (code) => code;
  }
}

/** Two-letter codes with names, for a picker, sorted by name; any other valid code can be typed. */
export const COUNTRIES: [string, string][] = (() => {
  const name = regionNames();
  return ISO_CODES.split(' ')
    .map((code): [string, string] => [code, name(code)])
    .sort((a, b) => a[1].localeCompare(b[1], 'en'));
})();
