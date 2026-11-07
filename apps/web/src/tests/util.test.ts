import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  base58Decode,
  base58Encode,
  didFromKey,
  formatMs,
  initials,
  inviteQuery,
  joinLink,
  normalizeCommunityName,
  paragraphs,
  parseKey,
  reasonLabel,
  ruleOptions,
  safeHttpUrl,
  saveFile,
} from '../lib/util';

const KEY = 'a7f291c0' + '3'.repeat(56);

afterEach(() => vi.unstubAllGlobals());

describe('native backup saves', () => {
  it('does not report success when the native save dialog was cancelled', async () => {
    const invoke = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true);
    vi.stubGlobal('window', { __TAURI_INTERNALS__: { invoke } });
    await expect(saveFile('identity.hoikey', new Uint8Array([1, 2, 3]))).rejects.toThrow('No file was saved');
    await expect(saveFile('identity.hoikey', new Uint8Array([1, 2, 3]))).resolves.toBeUndefined();
    expect(invoke).toHaveBeenCalledWith('hoi_save_file', { name: 'identity.hoikey', data: 'AQID' }, undefined);
  });
});

describe('names and text', () => {
  it('makes two initials', () => {
    expect(initials('jonas.w')).toBe('JW');
    expect(initials('mira')).toBe('MI');
    expect(initials('p.tran')).toBe('PT');
    expect(initials('')).toBe('··');
  });

  it('normalises community names', () => {
    expect(normalizeCommunityName('  c/Bikes Repair ')).toBe('bikes-repair');
  });

  it('splits paragraphs on blank lines only', () => {
    expect(paragraphs('one\ntwo\n\n\nthree\r\n\r\nfour')).toEqual(['one\ntwo', 'three', 'four']);
    expect(paragraphs('   ')).toEqual([]);
  });

  it('finds numbered rules for the flag picker', () => {
    expect(ruleOptions('1. Be kind.\nnot a rule\n12) No spam')).toEqual([
      { n: 1, label: 'Be kind.' },
      { n: 12, label: 'No spam' },
    ]);
    expect(reasonLabel('rule:4')).toBe('rule 4');
    expect(reasonLabel('spam')).toBe('spam');
  });

  it('only lets http(s) links be clickable', () => {
    expect(safeHttpUrl('https://notes.example/x')?.host).toBe('notes.example');
    expect(safeHttpUrl('javascript:alert(1)')).toBeNull();
    expect(safeHttpUrl('not a url')).toBeNull();
  });

  it('formats stamp times', () => {
    expect(formatMs(12)).toBe('12 ms');
    expect(formatMs(1234)).toBe('1.2 s');
    expect(formatMs(64_000)).toBe('1 min 4 s');
  });
});

describe('keys', () => {
  it('round-trips base58', () => {
    const bytes = Uint8Array.from([0, 0, 1, 2, 250, 99]);
    expect(base58Decode(base58Encode(bytes))).toEqual(bytes);
  });

  it('makes did:keys that start with z6Mk and parse back', () => {
    const did = didFromKey(KEY);
    expect(did.startsWith('did:key:z6Mk')).toBe(true);
    expect(parseKey(did)).toBe(KEY);
  });

  it('accepts hex in any case and refuses junk', () => {
    expect(parseKey(KEY.toUpperCase())).toBe(KEY);
    expect(parseKey(' ' + KEY + ' ')).toBe(KEY);
    expect(parseKey('a7f2·91c0')).toBeNull();
    expect(parseKey('did:key:zNotBase58!')).toBeNull();
    expect(parseKey('did:key:z' + base58Encode(Uint8Array.from([1, 2, 3])))).toBeNull();
  });
});

describe('invite links', () => {
  const q = 'c=' + KEY + '&g=eyJuIjoiYmlrZXMifQ';
  it('pulls the query out of every shape a person might paste', () => {
    expect(inviteQuery('hoi://join?' + q)).toBe(q);
    expect(inviteQuery('https://hoi.sh/app/#/join?' + q)).toBe(q);
    expect(inviteQuery('#/join?' + q)).toBe(q);
    expect(inviteQuery('?' + q)).toBe(q);
    expect(inviteQuery('  hoi://join?' + q + '  ')).toBe(q);
  });
  it('refuses things that are not invites', () => {
    expect(inviteQuery('https://example.com')).toBeNull();
    expect(inviteQuery('hoi://join?g=only')).toBeNull();
    expect(inviteQuery('')).toBeNull();
    expect(inviteQuery('hoi://join?code=')).toBeNull();
  });
  it('accepts encoded invites in pasted, web and native links without decoding them in the UI', () => {
    const code = 'code=pGFjWCAFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBWFwgGFygGF2AQ';
    for (const prefix of ['', '?', '#/join?', 'https://hoi.sh/app/#/join?', 'hoi://join?', 'web+hoi://join?']) {
      expect(inviteQuery(`  ${prefix}${code}  `)).toBe(code);
    }
    expect(joinLink(code)).toBe(`hoi://join?${code}`);
  });
  it('hands the core a hoi:// link', () => {
    expect(joinLink(q)).toBe('hoi://join?' + q);
  });
});
