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

  it('keeps home-placement authority with its post through a failed browser write before network dispatch', async () => {
    vi.stubGlobal('indexedDB', indexedDB);
    const db = await openStore('durable-home-placement');
    const vault = testVault();
    await migrate(db, await snapshot(db), vault);
    const journal = new TextEncoder().encode(JSON.stringify({ version: 1, origin: { request: { records: ['post'], content: 'public post', capability: null, result: null } }, placements: {} }));
    let changes: Change[] = [
      { op: 'put', id: 'post', community: 'community', expiresAt: 123, wire: new Uint8Array([7]) },
      { op: 'blob', key: 'home-scatter-journal', value: journal },
    ];
    const node = { take_changes: () => { const batch = changes; changes = []; return batch; }, take_events: () => '[]' };
    let fail = true;
    const send = vi.fn();
    const settle = createDurableSettler(node, async batch => {
      if (fail) throw new Error('quota exceeded');
      await apply(db, batch, vault);
    }, send, () => {});
    await expect(settle()).rejects.toThrow('quota exceeded');
    expect(send).not.toHaveBeenCalled();
    expect(decodeSnapshot(await snapshot(db), vault).records).toEqual([]);
    fail = false;
    await settle();
    const saved = decodeSnapshot(await snapshot(db), vault);
    expect(saved.records).toEqual([new Uint8Array([7])]);
    expect(Object.fromEntries(saved.blobs)['home-scatter-journal']).toEqual(journal);
    expect(send).toHaveBeenCalledOnce();
    // The existing vault seals the authority blob; a raw database snapshot has no plaintext.
    expect((await snapshot(db)).blobs.some(([key]) => key === 'home-scatter-journal')).toBe(false);
    db.close();
    vi.unstubAllGlobals();
  });

  it('keeps a confirmed deletion list until storage takes it, and a failed clear leaves it whole', async () => {
    vi.stubGlobal('indexedDB', indexedDB);
    const db = await openStore('durable-bulk-clear');
    const vault = testVault();
    await migrate(db, await snapshot(db), vault);
    const batches = new TextEncoder().encode(JSON.stringify({ version: 1, batches: { snapshot: { targets: ['post'] } } }));
    let pending: Change[] = [{ op: 'blob', key: 'deletion-batches', value: batches }, { op: 'blob', key: 'state', value: new Uint8Array([1]) }];
    const status = JSON.stringify([{ type: 'changed', communities: [] }]);
    const node = { take_changes: () => { const changes = pending; pending = []; return changes; }, take_events: () => status };
    const emit = vi.fn();
    let fail = true;
    const settle = createDurableSettler(node, async changes => {
      if (fail) throw new Error('quota exceeded');
      await apply(db, changes, vault);
    }, () => {}, emit);
    const saved = async () => Object.fromEntries(decodeSnapshot(await snapshot(db), vault).blobs)['deletion-batches'];

    await expect(settle()).rejects.toThrow('quota exceeded');
    expect(emit).not.toHaveBeenCalled();
    expect(await saved()).toBeUndefined();
    fail = false;
    await settle();
    expect(await saved()).toEqual(batches);
    expect(emit).toHaveBeenCalledOnce();

    // the clear's journal fails to save: the list stays, whole, until the same clear is retried
    pending = [{ op: 'wipe' }, { op: 'blob', key: 'format', value: new Uint8Array([5]) }];
    fail = true;
    await expect(settle()).rejects.toThrow('quota exceeded');
    expect(await saved()).toEqual(batches);
    fail = false;
    await settle();
    expect(await saved()).toBeUndefined();
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
