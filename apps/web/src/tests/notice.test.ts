import { describe, expect, it } from 'vitest';
import vector from '../../../../core/vectors/node-notice.json';
import { COUNTRIES, MAX_NOTICE_BYTES, PIECES, canonicalNotice, noticeJson, noticeOmissions, noticeSections, noticeTemplate, noticeText, noticeTitle, validateNotice, type NodeNotice } from '../lib/notice';

const hex = (b: Uint8Array) => [...b].map((x) => x.toString(16).padStart(2, '0')).join('');

function fromVector(): NodeNotice {
  return vector.json as NodeNotice;
}

describe('operator notices (the same words and bytes as hoi_types::notice)', () => {
  it('writes the canonical bytes the Rust core hashes, byte for byte', () => {
    expect(hex(canonicalNotice(fromVector()))).toBe(vector.canonical);
    expect(canonicalNotice(fromVector()).length).toBeLessThan(MAX_NOTICE_BYTES);
  });

  it('renders the notice as the core prints it, up to the hash only the node knows', () => {
    const text = noticeText(fromVector(), vector.hash);
    expect(text).toBe(vector.text);
    expect(noticeTitle(fromVector())).toBe('Join notice – sleepy-walrus-48213');
    const sections = noticeSections(fromVector());
    expect(sections.map((s) => s.title)).toEqual(['Operator', 'Server location', 'What this node processes', 'Logging and retention', 'Purpose', 'Other participants', 'Beyond this node', 'Your rights']);
    expect(sections[3].text).toBe('IP addresses are logged for 7 days for security and abuse prevention, then deleted.');
  });

  it('says in a fixed sentence that a node keeps encrypted pieces, with the bytes and words of the core', () => {
    const n: NodeNotice = { ...fromVector(), pieces: true };
    expect(hex(canonicalNotice(n))).toBe(vector.pieces.canonical);
    expect(noticeText(n, vector.hash).replace(/Notice version \w+/, '')).toBe(vector.pieces.text.replace(/Notice version \w+/, ''));
    const sections = noticeSections(n);
    expect(sections[3]).toEqual({ title: 'Encrypted pieces', text: PIECES });
    expect(JSON.parse(noticeJson(n)).pieces).toBe(true);
    expect('pieces' in JSON.parse(noticeJson(fromVector()))).toBe(false);
  });

  it('asks only for what is required, with the words the installer shows: who and where may be left out', () => {
    expect(validateNotice(noticeTemplate(''))).toEqual([]);
    const blank: NodeNotice = { ...noticeTemplate(''), processes: '', updated: '' };
    expect(validateNotice(blank).map((p) => p.field)).toEqual(['processes', 'updated']);
    const n: NodeNotice = { ...fromVector(), ipLogging: true, retentionDays: 0 };
    expect(validateNotice(n)[0]).toEqual({ field: 'retentionDays', message: 'Say for how many days IP addresses are kept.' });
    expect(validateNotice({ ...fromVector(), country: 'xx' })[0].field).toBe('country');
    expect(validateNotice({ ...fromVector(), country: 'Somewhere' })[0].message).toBe("Give the server's country as its two-letter ISO code, or leave it empty.");
    expect(validateNotice({ ...fromVector(), updated: '3.11.2026' })[0].message).toBe('Date the notice as YYYY-MM-DD.');
    expect(validateNotice({ ...fromVector(), operator: 'x'.repeat(121) })[0].message).toBe('At most 120 characters.');
    expect(validateNotice({ ...fromVector(), operator: 'Le\u0007na' })[0].message).toBe('No control characters.');
    expect(validateNotice({ ...fromVector(), processes: 'two\nlines' })).toEqual([]);
    expect(validateNotice({ ...fromVector(), v: 2 })[0].field).toBe('v');
  });

  it('keeps a link scannable with a byte budget', () => {
    const long: NodeNotice = { ...fromVector(), processes: 'x'.repeat(600), purpose: 'y'.repeat(400), communityRules: 'z'.repeat(600) };
    const problems = validateNotice(long);
    expect(problems).toHaveLength(1);
    expect(problems[0].field).toBe('notice');
    expect(problems[0].message).toMatch(/^Too long for a node link: shorten it by \d+ bytes/);
  });

  it('trims and drops empty optionals in the file and the bytes', () => {
    const padded: NodeNotice = { ...fromVector(), operator: '  Lena ', extra: '   ' };
    expect(hex(canonicalNotice(padded))).toBe(vector.canonical);
    const json = JSON.parse(noticeJson(padded));
    expect(json.operator).toBe('Lena');
    expect('extra' in json).toBe(false);
    expect(json.provider).toBe('Example Hosting');
    expect(json.ipLogging).toBe(true);
  });

  it('says plainly what an operator left out, the same words as the core', () => {
    const at = (n: NodeNotice, title: string) => noticeSections(n).find((s) => s.title === title)?.text;
    const anonymous: NodeNotice = { ...fromVector(), operator: '', contact: '', country: '', provider: undefined };
    expect(validateNotice(anonymous)).toEqual([]);
    expect(at(anonymous, 'Operator')).toBe('The operator of this node chose not to say who they are or how to reach them.');
    expect(at(anonymous, 'Server location')).toBe('The operator did not say where the server is located or who hosts it.');
    expect(at(anonymous, 'Your rights')).toMatch(/^The operator gave no contact, so this notice offers no way to ask them/);
    expect(at({ ...fromVector(), contact: '' }, 'Operator')).toBe('This node is operated by Lena, who gave no contact.');
    expect(at({ ...fromVector(), operator: '' }, 'Operator')).toBe('The operator of this node gave no name. Contact: lena@example.org.');
    expect(at({ ...fromVector(), country: '' }, 'Server location')).toBe('The server is hosted by Example Hosting. The operator did not say in which country.');
    expect(noticeOmissions(anonymous)).toHaveLength(3);
    expect(noticeOmissions(fromVector())).toEqual([]);
    // Empty, they still travel, so a reader from before they became optional reads the notice.
    const bytes = canonicalNotice(anonymous);
    expect(hex(bytes)).toContain('616f60'); // "o": ""
  });

  it('offers every country in alphabetical order, none first', () => {
    expect(COUNTRIES).toHaveLength(249);
    const names = COUNTRIES.map(([, name]) => name);
    expect([...names].sort((a, b) => a.localeCompare(b, 'en'))).toEqual(names);
    expect(COUNTRIES.every(([code]) => /^[A-Z]{2}$/.test(code))).toBe(true);
  });
});
