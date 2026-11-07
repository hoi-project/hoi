import { beforeEach, describe, expect, it, vi } from 'vitest';
import { claim, pair, otherKey } from './fixtures/registry-claims.mjs';
import { IDBFactory } from 'fake-indexeddb';
import { RegistryConflict, publicNameValid, registryBase, registryClient, canJoinPublic, registryQuery } from '../lib/registry';
import { registryLogPins, registryPins, verifyRegistryEntry } from '../../../../registry/public/verify.js';
import { logEntryText } from '../../../../registry/public/log.js';
import { answerLog, nameLog } from '../../../../registry/log.mjs';
import { RELEASE } from '../lib/preview';

function listing(proof: ReturnType<typeof claim>, indexed = false) {
  const version = proof.protocolVersion ? 2 : 1;
  const signature = version === 2 ? { version: 2 as const, signature: proof.signature, protocolVersion: proof.protocolVersion!, indexVersion: proof.indexVersion! } : { version: 1 as const, signature: proof.signature };
  return { name: proof.name, community: proof.community, proof: signature, ...(indexed ? { protocolVersion: proof.protocolVersion ?? null, indexVersion: proof.indexVersion ?? null, compatible: version === 2, indexed: version === 2 } : {}) };
}
const proof = claim();
const response = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status });
beforeEach(() => {
  vi.stubGlobal('indexedDB', new IDBFactory());
  vi.stubGlobal('location', { href: 'https://hoi.test/app/' });
});

// These tests are about entries and pins; the log check has its own tests below.
const withoutLog = (base: string, fetcher?: typeof fetch, pins?: Parameters<typeof registryClient>[2]) => registryClient(base, fetcher, pins, { log: { check: async () => {} } });
describe('public-name registry client', () => {
  it('accepts familiar community labels but refuses to send arbitrary text searches', async () => {
    expect(registryQuery(' C/Bikes-Repair ')).toBe('bikes-repair');
    const fetcher = vi.fn();
    const client = withoutLog('https://registry.test', fetcher);
    expect(await client.searchIndexed(registryQuery('a private sentence'))).toEqual([]);
    expect(await client.searchIndexed(registryQuery('https://untrusted.test/'))).toEqual([]);
    expect(fetcher).not.toHaveBeenCalled();
  });
  it('never treats network or server failure as name availability', async () => {
    const offline = withoutLog('https://registry.test', vi.fn().mockRejectedValue(new TypeError('offline')));
    await expect(offline.lookup('bikes')).rejects.toThrow('unreachable');
    const failed = withoutLog('https://registry.test', vi.fn().mockResolvedValue(response({}, 503)));
    await expect(failed.lookup('bikes')).rejects.toThrow('Availability is unknown');
  });
  it('only accepts an exact 404 for a name never verified on this device', async () => {
    const client = withoutLog('https://registry.test', vi.fn().mockResolvedValue(response({}, 404)));
    expect(await client.lookup('bikes')).toBeNull();
    const wrong = withoutLog('https://registry.test', vi.fn().mockResolvedValue(response(listing(claim('other')))));
    await expect(wrong.lookup('bikes')).rejects.toThrow('different name');
  });
  it('verifies the winning key on an atomic reservation conflict', async () => {
    const winner = listing(claim('bikes', otherKey()));
    const client = withoutLog('https://registry.test', vi.fn().mockImplementation(async () => response(winner, 409)));
    await expect(client.reserve(proof)).rejects.toMatchObject({ entry: winner });
    await expect(client.reserve(proof)).rejects.toBeInstanceOf(RegistryConflict);
  });
  it('retries the identical proof after a lost response and only accepts its own mapping', async () => {
    const fetcher = vi.fn().mockRejectedValueOnce(new Error('lost response')).mockResolvedValueOnce(response(listing(proof)));
    const client = withoutLog('https://registry.test', fetcher);
    await expect(client.reserve(proof)).rejects.toThrow();
    expect(await client.reserve(proof)).toEqual(listing(proof));
    expect(fetcher.mock.calls[0][1].body).toBe(fetcher.mock.calls[1][1].body);
    expect(fetcher.mock.calls[0][1]).toMatchObject({ credentials: 'omit', referrerPolicy: 'no-referrer', cache: 'no-store', redirect: 'error' });
    const wrong = withoutLog('https://registry.test', vi.fn().mockResolvedValue(response(listing(claim('bikes', otherKey())))));
    await expect(wrong.reserve(proof)).rejects.toThrow('did not confirm');
  });
  it('validates public names and HTTPS registry addresses', () => {
    expect(publicNameValid('bikes-repair')).toBe(true);
    for (const name of ['Bikes', 'аbc', '', '-bikes', 'a'.repeat(33)]) expect(publicNameValid(name)).toBe(false);
    expect(registryBase('http://127.0.0.1:7845/')).toBe('http://127.0.0.1:7845');
    expect(() => registryBase('http://registry.test')).toThrow('HTTPS');
    expect(() => registryBase('https://user:secret@registry.test')).toThrow('credentials');
  });
  it('supports an explicitly selected same-origin registry without credentials', async () => {
    const fetcher = vi.fn().mockResolvedValue(response(listing(proof)));
    await withoutLog('/registry/', fetcher).reserve(proof);
    expect(fetcher.mock.calls[0][0]).toBe('/registry/v1/communities');
    expect(fetcher.mock.calls[0][1]).toMatchObject({ credentials: 'omit', redirect: 'error', cache: 'no-store' });
    expect(() => registryBase('//registry.test')).toThrow();
    expect(() => registryBase('/registry/../other')).toThrow();
  });
  it('bounds directory count, response bytes and unexpected names', async () => {
    const client = withoutLog('https://registry.test', vi.fn().mockResolvedValue(response(Array(51).fill(listing(proof)))));
    await expect(client.search('')).rejects.toThrow('invalid directory');
    const large = withoutLog('https://registry.test', vi.fn().mockResolvedValue(response('x'.repeat(128 * 1024))));
    await expect(large.lookup('bikes')).rejects.toThrow('could not be verified');
    const other = withoutLog('https://registry.test', vi.fn().mockResolvedValue(response([listing(claim('other'))])));
    await expect(other.search('bikes')).rejects.toThrow('different name');
  });
  it('rejects unsigned mappings, modified claims, forged signatures and mismatched proofs', async () => {
    const valid = listing(proof);
    for (const item of [{ name: proof.name, community: proof.community }, { ...valid, name: 'other' },
      { ...valid, community: 'a'.repeat(64) }, { ...valid, proof: { ...valid.proof, signature: '0'.repeat(128) } },
      { ...valid, proof: { ...valid.proof, version: 99 } }]) {
      await expect(verifyRegistryEntry(item)).rejects.toThrow('could not be verified');
    }
  });
  it('pins names persistently across client instances and blocks validly signed replacements', async () => {
    const first = withoutLog('https://registry.test/', vi.fn().mockResolvedValue(response(listing(proof))));
    await first.lookup('bikes');
    const replacement = listing(claim('bikes', otherKey()));
    const other = withoutLog('https://registry.test', vi.fn().mockResolvedValue(response(replacement)));
    await expect(other.lookup('bikes')).rejects.toThrow('changed since it was verified');
    const missing = withoutLog('https://registry.test', vi.fn().mockResolvedValue(response({}, 404)));
    await expect(missing.lookup('bikes')).rejects.toThrow('previously verified');
    const separate = withoutLog('https://different.test', vi.fn().mockResolvedValue(response(replacement)));
    expect((await separate.lookup('bikes'))?.community).toBe(replacement.community);
  });
  it('fails closed when persistent checks are unavailable', async () => {
    vi.stubGlobal('indexedDB', undefined);
    const client = withoutLog('https://registry.test', vi.fn().mockResolvedValue(response(listing(proof))));
    await expect(client.lookup('bikes')).rejects.toThrow('checks are unavailable');
    const missing = withoutLog('https://registry.test', vi.fn().mockResolvedValue(response({}, 404)));
    await expect(missing.lookup('bikes')).rejects.toThrow('checks are unavailable');
  });
  it('does not fall back when saved storage cannot open or has an unexpected schema', async () => {
    const denied = { open() { throw new Error('private browser detail'); } } as unknown as IDBFactory;
    await expect(registryPins('https://registry.test', denied).accept([await verifyRegistryEntry(listing(proof))])).rejects.toThrow('checks are unavailable');
    await new Promise<void>((resolve, reject) => {
      const request = indexedDB.open('hoi-registry-trust-v1', 2);
      request.onupgradeneeded = () => request.result.createObjectStore('wrong-store');
      request.onsuccess = () => { request.result.close(); resolve(); };
      request.onerror = () => reject(request.error);
    });
    const saved = registryPins('https://registry.test');
    await expect(saved.accept([await verifyRegistryEntry(listing(proof))])).rejects.toThrow('checks are unavailable');
    await expect(saved.checkMissing('bikes')).rejects.toThrow('checks are unavailable');
  });
  it('atomically arbitrates concurrent first-use results and never partially pins a conflicting batch', async () => {
    const pins = registryPins('https://registry.test');
    const a = await verifyRegistryEntry(listing(proof));
    const b = await verifyRegistryEntry(listing(claim('bikes', otherKey())));
    const outcomes = await Promise.allSettled([pins.accept([a]), registryPins('https://registry.test').accept([b])]);
    expect(outcomes.filter(outcome => outcome.status === 'fulfilled')).toHaveLength(1);
    expect(outcomes.filter(outcome => outcome.status === 'rejected')).toHaveLength(1);
    const winner = outcomes[0].status === 'fulfilled' ? a : b;
    const loser = winner === a ? b : a;
    const unrelated = await verifyRegistryEntry(listing(claim('unrelated')));
    await expect(pins.accept([unrelated, loser])).rejects.toThrow('changed');
    await expect(pins.checkMissing('unrelated')).resolves.toBeUndefined();
    await expect(pins.accept([winner, loser])).rejects.toThrow('could not be verified');
    await pins.accept([winner]);
  });
});

describe('versioned public directory', () => {
  const publication = claim('bikes', pair, 2) as Required<ReturnType<typeof claim>>;
  const entry = listing(publication, true);
  it('requires version-bound publication and a confirmed compatible mapping', async () => {
    const fetcher = vi.fn().mockResolvedValue(response(entry));
    expect(await withoutLog('https://registry.test', fetcher).publish(publication)).toEqual(entry);
    expect(fetcher.mock.calls[0][0]).toBe('https://registry.test/v2/communities');
    expect(JSON.parse(fetcher.mock.calls[0][1].body)).toEqual(publication);
    const hidden = withoutLog('https://registry.test', vi.fn().mockResolvedValue(response({ ...entry, indexed: false })));
    await expect(hidden.publish(publication)).rejects.toThrow('did not confirm');
  });
  it('accepts signed reservations without advertising them and refuses old unsigned reservations', async () => {
    const legacy = listing(proof, true);
    const client = withoutLog('https://registry.test', vi.fn().mockResolvedValue(response(legacy)));
    expect(await client.lookupIndexed(proof.name)).toEqual(legacy);
    expect(canJoinPublic(legacy)).toBe(false);
    const unsigned = withoutLog('https://registry.test', vi.fn().mockResolvedValue(response({ ...legacy, proof: null })));
    await expect(unsigned.lookupIndexed(proof.name)).rejects.toThrow('could not be verified');
    expect(canJoinPublic({ ...entry, protocolVersion: 'future', compatible: true })).toBe(false);
    expect(canJoinPublic({ ...entry, indexVersion: RELEASE.indexVersion + 1 })).toBe(false);
    expect(canJoinPublic(entry)).toBe(true);
  });
  it('verifies versioned conflicts and rejects altered publication metadata', async () => {
    const winner = listing(claim('bikes', otherKey(), 2), true);
    const client = withoutLog('https://registry.test', vi.fn().mockResolvedValue(response(winner, 409)));
    await expect(client.publish(publication)).rejects.toMatchObject({ entry: winner });
    for (const value of [{ ...entry, indexVersion: RELEASE.indexVersion + 1 }, { ...entry, protocolVersion: 'future' },
      { ...entry, proof: { ...entry.proof, indexVersion: RELEASE.indexVersion + 1 } }]) {
      await expect(verifyRegistryEntry(value, RELEASE)).rejects.toThrow('could not be verified');
    }
    const future = listing(claim('future', pair, 2, 'future'), true);
    expect((await verifyRegistryEntry({ ...future, compatible: true, indexed: true }, RELEASE)).compatible).toBe(false);
  });
  it('allows one signed publication upgrade but never a signed downgrade or changed immutable metadata', async () => {
    const pins = registryPins('https://registry.test');
    await pins.accept([await verifyRegistryEntry(listing(proof))]);
    await pins.accept([await verifyRegistryEntry(entry, RELEASE)]);
    await expect(pins.accept([await verifyRegistryEntry(listing(proof))])).rejects.toThrow('changed');
    const changed = await verifyRegistryEntry(listing(claim('bikes', pair, 2, 'future'), true), RELEASE);
    await expect(pins.accept([changed])).rejects.toThrow('changed');
  });
  it('bounds metadata and omits credentials for public bootstrap requests', async () => {
    const meta = { release: RELEASE, publicNodes: [], appUrl: 'https://hoi.test/app/', registry: { apiVersion: 2, schemaVersion: 3, indexVersion: RELEASE.indexVersion } };
    const fetcher = vi.fn().mockResolvedValue(response(meta));
    expect(await withoutLog('https://registry.test', fetcher).metadata()).toEqual(meta);
    expect(fetcher.mock.calls[0][0]).toBe('https://registry.test/v1/meta');
    expect(fetcher.mock.calls[0][1].credentials).toBe('omit');
    const excessive = withoutLog('https://registry.test', vi.fn().mockResolvedValue(response({ ...meta, publicNodes: Array(17).fill('x') })));
    await expect(excessive.metadata()).rejects.toThrow('invalid service');
  });
});

describe('public node discovery per community', () => {
  const key = 'c'.repeat(64);
  const rtc = `rtc:${'q'.repeat(43)}@203.0.113.10:7844`;
  it('asks the registry which nodes carry a community and keeps the answer bounded', async () => {
    const fetcher = vi.fn().mockResolvedValue(response({ publicNodes: [rtc] }));
    const client = withoutLog('https://registry.test', fetcher);
    expect(await client.nodesFor(key)).toEqual([rtc]);
    expect(fetcher.mock.calls[0][0]).toBe(`https://registry.test/v2/nodes?community=${key}`);
    expect(fetcher.mock.calls[0][1]).toMatchObject({ credentials: 'omit', cache: 'no-store' });
    await expect(client.nodesFor('c/garden')).rejects.toThrow('Invalid community key');
    expect(fetcher).toHaveBeenCalledTimes(1);
  });
  it('refuses oversized, malformed or failed answers instead of guessing routes', async () => {
    for (const bad of [{ publicNodes: Array.from({ length: 17 }, () => rtc) }, { publicNodes: [42] }, { publicNodes: ['x'.repeat(129)] }, { nodes: [rtc] }]) {
      await expect(withoutLog('https://registry.test', vi.fn().mockResolvedValue(response(bad))).nodesFor(key)).rejects.toThrow('invalid public node');
    }
    await expect(withoutLog('https://registry.test', vi.fn().mockResolvedValue(response({}, 503))).nodesFor(key)).rejects.toThrow('unavailable (503)');
    await expect(withoutLog('https://registry.test', vi.fn().mockRejectedValue(new TypeError('offline'))).nodesFor(key)).rejects.toThrow('unreachable');
  });
});

describe('the registry\'s public log of names', () => {
  /** A registry with a real log over `entries`, answering lookups of `bikes` with `served`. */
  function registry(entries: ReturnType<typeof listing>[], served: ReturnType<typeof listing>, secret = '11'.repeat(32)) {
    const texts = entries.map((entry) => logEntryText(entry));
    const log = nameLog({ logSize: () => texts.length, logEntries: (start: number, end: number) => texts.slice(start, end).map((entry) => ({ entry, withheld: false })), logSecret: () => secret });
    const fetcher = vi.fn(async (input: string) => {
      const url = new URL(input);
      if (url.pathname.startsWith('/v2/log/')) {
        const answer = answerLog(log, url.pathname, url.searchParams, 1_800_000_000);
        return response(answer.body, answer.status);
      }
      return response(served);
    });
    return { texts, fetcher, client: registryClient('https://registry.test', fetcher as unknown as typeof fetch) };
  }
  const bikes = listing(proof);
  const others = [listing(claim('apples')), listing(claim('pears'))];

  it('takes a name that is in the log, and keeps the head it checked', async () => {
    const { client, fetcher } = registry([...others, bikes], bikes);
    expect(await client.lookup('bikes')).toEqual(bikes);
    const paths = fetcher.mock.calls.map(([url]) => new URL(url).pathname);
    expect(paths).toEqual(['/v1/communities', '/v2/log/head', '/v2/log/proof']);
    expect((await registryLogPins('https://registry.test').get())?.size).toBe(3);
  });
  it('refuses a name the registry does not show in its log', async () => {
    const { client } = registry(others, bikes);
    await expect(client.lookup('bikes')).rejects.toThrow('did not show this name in its public log');
  });
  it('checks that the log only grew since this device last saw it', async () => {
    const first = registry([bikes], bikes);
    expect(await first.client.lookup('bikes')).toEqual(bikes);
    const grown = registry([bikes, ...others], bikes);
    expect(await grown.client.lookup('bikes')).toEqual(bikes);
    expect(grown.fetcher.mock.calls.map(([url]) => new URL(url).pathname)).toContain('/v2/log/consistency');
    expect((await registryLogPins('https://registry.test').get())?.size).toBe(3);
    // The same registry later shows a log in which the name sits elsewhere: rewritten.
    const rewritten = registry([others[0], bikes, others[1], listing(claim('plums'))], bikes);
    await expect(rewritten.client.lookup('bikes')).rejects.toThrow('no longer matches');
    const shrunk = registry([bikes], bikes);
    await expect(shrunk.client.lookup('bikes')).rejects.toThrow('no longer matches');
    const rekeyed = registry([bikes, ...others, listing(claim('plums'))], bikes, '22'.repeat(32));
    await expect(rekeyed.client.lookup('bikes')).rejects.toThrow('no longer matches');
  });
  it('a registry without a log is not trusted with names', async () => {
    const client = registryClient('https://registry.test', vi.fn(async (input: string) => new URL(input).pathname.startsWith('/v2/log/') ? response({ error: 'not-found' }, 404) : response(bikes)) as unknown as typeof fetch);
    await expect(client.lookup('bikes')).rejects.toThrow('did not show this name in its public log');
  });
});
