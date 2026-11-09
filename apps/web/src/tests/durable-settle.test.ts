import { describe, expect, it, vi } from 'vitest';
import { indexedDB } from 'fake-indexeddb';
import { apply, snapshot, decodeSnapshot, migrate, openStore, type Change } from '../lib/core/idb';
import { testVault } from './fixtures/vault.mjs';
import { createDurableSettler } from '../lib/core/durable-settle';

describe('browser persistence', () => {
  it('persists deletion authority and suppression together before dispatch, including a retry after quota failure', async () => {
    vi.stubGlobal('indexedDB', indexedDB);
    const db = await openStore('durable-deletion-retry');
    const vault = testVault();
    await migrate(db, await snapshot(db), vault);
    await apply(db, [{ op: 'put', id: 'target', community: 'community', expiresAt: 123, wire: new Uint8Array([7]) }], vault);
    const journal = new TextEncoder().encode(JSON.stringify({ version: 1, jobs: { deletion: { target: 'target', targets: ['holder-locator-capability'] } } }));
    let pending: Change[] = [
      { op: 'put', id: 'deletion', community: 'community', expiresAt: 123, wire: new Uint8Array([8]) },
      { op: 'delete', id: 'target' },
      { op: 'blob', key: 'deletion-journal', value: journal },
      { op: 'blob', key: 'state', value: new Uint8Array([9]) },
    ];
    const node = { take_changes: () => { const changes = pending; pending = []; return changes; }, take_events: () => '[]' };
    const send = vi.fn();
    let fail = true;
    const settle = createDurableSettler(node, async changes => {
      if (fail) throw new Error('quota exceeded');
      await apply(db, changes, vault);
    }, send, () => {});
    await expect(settle()).rejects.toThrow('quota exceeded');
    expect(send).not.toHaveBeenCalled();
    expect(decodeSnapshot(await snapshot(db), vault).records).toEqual([new Uint8Array([7])]);
    fail = false;
    await settle();
    const restored = decodeSnapshot(await snapshot(db), vault);
    expect(restored.records).toEqual([new Uint8Array([8])]);
    expect(Object.fromEntries(restored.blobs)['deletion-journal']).toEqual(journal);
    expect(send).toHaveBeenCalledOnce();
    db.close();
    vi.unstubAllGlobals();
  });

  it('retries failed writes in order and sends records and events only after storage commits', async () => {
    vi.stubGlobal('indexedDB', indexedDB);
    const db = await openStore('durable-settle-retry');
    const vault = testVault();
    await migrate(db, await snapshot(db), vault);
    const first: Change[] = [
      { op: 'put', id: 'post', community: 'community', expiresAt: 123, wire: new Uint8Array([1, 2]) },
      { op: 'blob', key: 'keys', value: new Uint8Array([3]) },
      { op: 'blob', key: 'state', value: new Uint8Array([4]) },
    ];
    let changes = first;
    const takeEvents = vi.fn(() => JSON.stringify([{ type: 'changed', communities: ['community'] }]));
    const node = { take_changes: () => { const out = changes; changes = []; return out; }, take_events: takeEvents };
    const send = vi.fn();
    const emit = vi.fn();
    let fail = true;
    const settle = createDurableSettler(node, async (batch) => {
      if (fail) throw new Error('quota exceeded');
      await apply(db, batch, vault);
    }, send, emit);

    await expect(settle()).rejects.toThrow('quota exceeded');
    await expect(settle()).rejects.toThrow('quota exceeded');
    expect(send).not.toHaveBeenCalled();
    expect(takeEvents).not.toHaveBeenCalled();
    expect(emit).not.toHaveBeenCalled();
    expect((await snapshot(db)).records).toEqual([]);

    changes = [{ op: 'blob', key: 'state', value: new Uint8Array([5]) }];
    fail = false;
    await settle();
    const saved = decodeSnapshot(await snapshot(db), vault);
    expect(saved.records).toEqual([new Uint8Array([1, 2])]);
    expect(Object.fromEntries(saved.blobs)).toEqual({ keys: new Uint8Array([3]), state: new Uint8Array([5]) });
    expect(send).toHaveBeenCalledOnce();
    expect(emit).toHaveBeenCalledExactlyOnceWith({ type: 'changed', communities: ['community'] });
    db.close();
    vi.unstubAllGlobals();
  });
});
