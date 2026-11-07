import { describe, expect, it, vi } from 'vitest';
import { indexedDB } from 'fake-indexeddb';
import { apply, snapshot, decodeSnapshot, migrate, openStore, type Change } from '../lib/core/idb';
import { testVault } from './fixtures/vault.mjs';
import { createDurableSettler } from '../lib/core/durable-settle';

describe('browser persistence', () => {
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
