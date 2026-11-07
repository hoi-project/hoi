import { describe, expect, it } from 'vitest';
import type { LedgerEntryView, Person } from '@hoi/core-api';
import { DEFAULT_LIMITS } from '@hoi/core-api';
import { ledgerSigned, ledgerTag, ledgerWho, limitsLine, rulesVersions } from '../lib/ledger';

const person = (name: string, short: string): Person => ({ key: short.replace('·', '') + '0'.repeat(56), short, name, trust: 'member', muted: false, keyAgeDays: 10 });
const entry = (over: Partial<LedgerEntryView>): LedgerEntryView => ({
  id: Math.random().toString(16).slice(2), seq: 1, at: 0, action: 'remove', by: [], auto: false, evidence: 0, expiresAt: null,
  vetoes: 0, vetoesVoice: 0, vetoesNeeded: 2, support: 0, overturned: false, vetoable: true, ...over,
});

describe('ledger wording', () => {
  it('names actions like the prototype', () => {
    expect(ledgerTag(entry({ action: 'remove' })).text).toBe('removed');
    expect(ledgerTag(entry({ action: 'retire' })).text).toBe('left for good');
    expect(ledgerTag(entry({ action: 'ban', auto: true }))).toEqual({ text: 'bounced', color: 'peach' });
    expect(ledgerTag(entry({ action: 'ban' })).text).toBe('banned');
    expect(ledgerTag(entry({ action: 'mod-add' })).text).toBe('new mod');
    expect(ledgerTag(entry({ action: 'rules' }), 7).text).toBe('rules v7');
  });

  it('says who signed and how', () => {
    const jonas = person('jonas.w', '88c1·0b3e');
    const mira = person('mira', 'a7f2·91c0');
    expect(ledgerWho(entry({ auto: true }))).toBe('spam shield');
    expect(ledgerSigned(entry({ auto: true }))).toBe('auto');
    expect(ledgerSigned(entry({ by: [jonas] }))).toBe('88c1');
    expect(ledgerSigned(entry({ by: [mira, jonas] }))).toBe('2 sigs');
    expect(ledgerWho(entry({ by: [mira, jonas] }))).toBe('mira, jonas.w');
  });

  it('counts rules versions down from the current one', () => {
    const a = entry({ action: 'rules', seq: 3 });
    const b = entry({ action: 'rules', seq: 1 });
    const v = rulesVersions([a, entry({ action: 'ban', seq: 2 }), b], 7);
    expect(v.get(a.id)).toBe(7);
    expect(v.get(b.id)).toBe(6);
  });

  it('summarises limits in plain words', () => {
    expect(limitsLine(DEFAULT_LIMITS)).toContain('posts fade after 180 days');
  });
});
