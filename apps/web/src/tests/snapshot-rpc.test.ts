import { describe, expect, it, vi } from 'vitest';
import { createRpcCore, type Stamper } from '../lib/core/rpc-core';
import { createMockCore } from '../lib/core/mock-core';

const stamper: Stamper = { rate: async () => 1, mine: async () => 0, mineIdentity: async () => 0 };

describe('member snapshot API boundary', () => {
  it('carries only encrypted archive bytes across the UI RPC boundary and scopes rejoin requests', async () => {
    const call = vi.fn(async (method: string) => method === 'exportSnapshot' ? 'AAH_' : method === 'snapshotRejoins' ? [['community', 'Private group']] : { name: 'Restored member' });
    const core = createRpcCore({ call, subscribe: () => {} }, stamper);
    expect(await core.exportSnapshot('a long snapshot password')).toEqual(new Uint8Array([0, 1, 255]));
    await core.importSnapshot(new Uint8Array([0, 1, 255]), 'a long snapshot password');
    expect(await core.snapshotRejoins()).toEqual([['community', 'Private group']]);
    await core.snapshotRejoinRequest('community');
    expect(call.mock.calls).toEqual([
      ['exportSnapshot', ['a long snapshot password']],
      ['importSnapshot', ['AAH_', 'a long snapshot password']],
      ['snapshotRejoins', []],
      ['snapshotRejoinRequest', ['community']],
    ]);
  });

  it('preserves authenticated import errors instead of falling back to identity import or reset', async () => {
    const call = vi.fn(async () => { throw { code: 'bad-passphrase', message: 'This password did not open the snapshot.' }; });
    const core = createRpcCore({ call, subscribe: () => {} }, stamper);
    await expect(core.importSnapshot(new Uint8Array([1]), 'incorrect')).rejects.toMatchObject({ code: 'bad-passphrase' });
    expect(call).toHaveBeenCalledExactlyOnceWith('importSnapshot', ['AQ', 'incorrect']);
  });

  it('does not simulate secure member archives in the development mock', async () => {
    const core = createMockCore();
    await core.boot();
    await expect(core.importSnapshot(new Uint8Array([1]), 'not a real archive')).rejects.toThrow('secure core');
    expect(await core.me()).toBeNull();
    expect(await core.snapshotRejoins()).toEqual([]);
  });
});
