import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { IDBFactory } from 'fake-indexeddb';
import { apply, decodeSnapshot, migrate, openIdb, resetStore, snapshot, vaultHeader, type Snapshot } from '../lib/core/idb';
import { testVault } from './fixtures/vault.mjs';

const bytes = (s: string) => new TextEncoder().encode(s);
const legacy: Snapshot = {
  records: [['post', { wire: bytes('a private post'), community: 'group', expiresAt: 123 }]],
  blobs: [['keys', bytes('identity secret')], ['mls', bytes('group secret')], ['state', bytes('pairing secret')], ['format', bytes('{"version":3}')]],
};

async function seed(db: IDBDatabase, data = legacy) {
  await new Promise<void>((resolve, reject) => {
    const tx = db.transaction(['records', 'blobs'], 'readwrite');
    for (const [key, value] of data.records) tx.objectStore('records').put(value, key);
    for (const [key, value] of data.blobs) tx.objectStore('blobs').put(value, key);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
}

describe('encrypted browser storage', () => {
  beforeEach(() => vi.stubGlobal('indexedDB', new IDBFactory()));
  afterEach(() => vi.unstubAllGlobals());

  it('migrates all keys, group secrets, state and posts, preserves them on reopening, and guards old clients', async () => {
    const db = await openIdb();
    await seed(db);
    const vault = testVault();
    await migrate(db, await snapshot(db), vault);
    const saved = await snapshot(db);
    expect(vaultHeader(saved)).toEqual(vault.header());
    expect(saved.blobs.map(([key]) => key)).toEqual(['format', 'v:format', 'v:keys', 'v:mls', 'v:state', 'vault', 'vault-manifest']);
    expect(new TextDecoder().decode(saved.blobs.find(([key]) => key === 'format')![1])).toContain('4294967295');
    for (const [key, value] of legacy.blobs) expect(saved.blobs.find(([k]) => k === `v:${key}`)![1]).not.toEqual(value);
    expect(saved.records[0][1].wire).not.toEqual(legacy.records[0][1].wire);
    db.close();
    const reopened = await openIdb();
    const plain = decodeSnapshot(await snapshot(reopened), vault);
    expect(plain.records).toEqual(legacy.records.map(([, r]) => r.wire));
    expect(Object.fromEntries(plain.blobs)).toEqual(Object.fromEntries(legacy.blobs));
    reopened.close();
  });

  it('never writes a partially encrypted migration if sealing fails', async () => {
    const db = await openIdb();
    await seed(db);
    const before = await snapshot(db);
    const vault = testVault();
    const seal = vault.seal;
    vault.seal = (context, value) => { if (context === 'blob:mls') throw new Error('crypto failure'); return seal(context, value); };
    await expect(migrate(db, before, vault)).rejects.toThrow('crypto failure');
    expect(await snapshot(db)).toEqual(before);
    db.close();
  });

  it('rejects wrong keys, changed ciphertext and values moved to another field without rewriting storage', async () => {
    const db = await openIdb();
    const vault = testVault();
    await migrate(db, legacy, vault);
    const before = await snapshot(db);
    expect(() => decodeSnapshot(before, testVault())).toThrow();
    const tampered = structuredClone(before);
    tampered.records[0][1].wire[25] ^= 1;
    expect(() => decodeSnapshot(tampered, vault)).toThrow();
    const swapped = structuredClone(before);
    const keys = swapped.blobs.find(([key]) => key === 'v:keys')!;
    const mls = swapped.blobs.find(([key]) => key === 'v:mls')!;
    [keys[1], mls[1]] = [mls[1], keys[1]];
    expect(() => decodeSnapshot(swapped, vault)).toThrow();
    expect(await snapshot(db)).toEqual(before);
    db.close();
  });

  it('refuses mixed plaintext/ciphertext and missing metadata instead of treating them as a new account', async () => {
    const db = await openIdb();
    const vault = testVault();
    await migrate(db, legacy, vault);
    const saved = await snapshot(db);
    expect(() => vaultHeader({ ...saved, blobs: saved.blobs.filter(([key]) => key !== 'vault') })).toThrow('missing');
    expect(() => vaultHeader({ ...saved, blobs: saved.blobs.filter(([key]) => key !== 'format') })).toThrow('damaged');
    expect(() => vaultHeader({ ...saved, blobs: [...saved.blobs, ['keys', bytes('secret')]] })).toThrow('damaged');
    db.close();
  });

  it('keeps encryption after wipe and encrypts subsequent writes', async () => {
    const db = await openIdb();
    const vault = testVault();
    await migrate(db, legacy, vault);
    await apply(db, [{ op: 'wipe' }, { op: 'blob', key: 'keys', value: bytes('new identity') }], vault);
    const saved = await snapshot(db);
    expect(saved.records).toEqual([]);
    expect(vaultHeader(saved)).toEqual(vault.header());
    expect(Object.fromEntries(decodeSnapshot(saved, vault).blobs)).toEqual({ keys: bytes('new identity') });
    expect(saved.blobs.find(([key]) => key === 'v:keys')![1]).not.toEqual(bytes('new identity'));
    db.close();
  });

  it('clears deletion work with a wipe but keeps the vault; a locked reset removes the vault too; neither leaves the hoi database', async () => {
    const other = await openIdb('another-app');
    await seed(other);
    const untouched = await snapshot(other);
    const db = await openIdb();
    const vault = testVault();
    const work: Snapshot = {
      records: legacy.records,
      blobs: [...legacy.blobs, ['deletion-journal', bytes('{"version":1,"jobs":{}}')], ['deletion-batches', bytes('{"version":1,"batches":{}}')]],
    };
    await migrate(db, work, vault);
    // An ordinary wipe (`wipe` or `clearAfterDeletion`): the core's journal, as it sends it.
    await apply(db, [{ op: 'wipe' }, { op: 'blob', key: 'format', value: bytes('{"version":5}') }], vault);
    const wiped = await snapshot(db);
    expect(wiped.records).toEqual([]);
    expect(vaultHeader(wiped)).toEqual(vault.header());
    expect(Object.fromEntries(decodeSnapshot(wiped, vault).blobs)).toEqual({ format: bytes('{"version":5}') });
    expect(wiped.blobs.map(([key]) => key)).toEqual(['format', 'v:format', 'vault', 'vault-manifest']);
    db.close();
    // Locked recovery cannot read the vault; it removes it, so the next start sets up again.
    await resetStore('hoi', 'RESET');
    const fresh = await openIdb();
    const reset = await snapshot(fresh);
    expect(reset).toEqual({ records: [], blobs: [] });
    expect(vaultHeader(reset)).toBeNull();
    fresh.close();
    expect(await snapshot(other)).toEqual(untouched);
    other.close();
  });

  it('requires exact confirmation to reset a locked store, including damaged metadata', async () => {
    const db = await openIdb();
    await migrate(db, legacy, testVault());
    const saved = await snapshot(db);
    await expect(resetStore('hoi', '')).rejects.toThrow('Type RESET');
    await expect(resetStore('hoi', 'reset')).rejects.toThrow('Type RESET');
    expect(await snapshot(db)).toEqual(saved);
    await seed(db, { records: [], blobs: [['vault', bytes('unreadable header')]] });
    db.close();
    await resetStore('hoi', 'RESET');
    const fresh = await openIdb();
    expect(await snapshot(fresh)).toEqual({ records: [], blobs: [] });
    const replacement = testVault();
    await migrate(fresh, await snapshot(fresh), replacement);
    expect(vaultHeader(await snapshot(fresh))).toEqual(replacement.header());
    fresh.close();
  });

  it('recovers incompatible versions and missing stores without touching other databases', async () => {
    const other = await openIdb('another-app');
    await seed(other);
    const untouched = await snapshot(other);
    for (const version of [1, 2]) {
      await new Promise<void>((resolve, reject) => {
        const request = indexedDB.open('hoi', version);
        request.onupgradeneeded = () => request.result.createObjectStore('unexpected');
        request.onsuccess = () => { request.result.close(); resolve(); };
        request.onerror = () => reject(request.error);
      });
      await resetStore('hoi', 'RESET');
      const fresh = await openIdb();
      expect(await snapshot(fresh)).toEqual({ records: [], blobs: [] });
      fresh.close();
      await resetStore('hoi', 'RESET');
    }
    expect(await snapshot(other)).toEqual(untouched);
    other.close();
  });

  it('detects removed state, keys, MLS data, records and replay of older authentic ciphertext', async () => {
    const db = await openIdb();
    const vault = testVault();
    await migrate(db, legacy, vault);
    const original = await snapshot(db);
    for (const key of ['v:keys', 'v:state', 'v:mls', 'vault-manifest']) {
      expect(() => decodeSnapshot({ ...original, blobs: original.blobs.filter(([k]) => k !== key) }, vault)).toThrow();
    }
    expect(() => decodeSnapshot({ ...original, records: [] }, vault)).toThrow();
    await apply(db, [{ op: 'blob', key: 'mls', value: bytes('new group epoch') }], vault);
    const current = await snapshot(db);
    current.blobs.find(([key]) => key === 'v:mls')![1] = original.blobs.find(([key]) => key === 'v:mls')![1];
    expect(() => decodeSnapshot(current, vault)).toThrow();
    db.close();
  });
});
