import { describe, expect, it, vi } from 'vitest';
import { createRpcCore, type Stamper, type Transport } from '../lib/core/rpc-core';
import { workOutlook } from '../lib/key-work.svelte';

const me = { rootKey: 'ab'.repeat(32), workKey: 'ab'.repeat(32), hasRoot: true, identityBits: 10 };

function setup(mineMore?: Stamper['mineMore'], who: typeof me = me) {
  const calls: [string, unknown[]][] = [];
  const t: Transport = {
    call: async (method, args) => {
      calls.push([method, args]);
      return method === 'me' ? who : { ...who, identityBits: 12 };
    },
    subscribe: () => undefined,
  };
  const stamper: Stamper = { rate: async () => 1, mine: async () => 0, mineIdentity: async () => 0, mineMore };
  return { core: createRpcCore(t, stamper), calls };
}

describe('more key work', () => {
  it('is mined outside the core and only the found work is handed in', async () => {
    const onProgress = vi.fn();
    const { core, calls } = setup(async (_root, _bits, report) => {
      report({ tries: 7, best: 9 });
      return 4242;
    });
    expect((await core.strengthenIdentity(12, { onProgress })).identityBits).toBe(12);
    expect(onProgress).toHaveBeenCalledWith({ tries: 7, best: 9 });
    expect(calls).toEqual([['me', []], ['strengthenIdentity', [12, 4242]]]);
  });

  it('is mined for the root that signs, not the first one, after a recovery', async () => {
    const restored = { ...me, workKey: 'cd'.repeat(32) };
    const mined: string[] = [];
    const { core, calls } = setup(async (root) => {
      mined.push(root);
      return 99;
    }, restored);
    await core.strengthenIdentity(12);
    expect(mined).toEqual([restored.workKey]);
    expect(calls).toEqual([['me', []], ['strengthenIdentity', [12, 99]]]);
  });

  it('stops on abort and leaves the key as it was', async () => {
    const stop = new AbortController();
    const { core, calls } = setup(async (_root, _bits, _report, signal) => {
      stop.abort();
      return signal?.aborted ? null : 1;
    });
    await expect(core.strengthenIdentity(12, { signal: stop.signal })).rejects.toMatchObject({ name: 'AbortError' });
    expect(calls.map(([m]) => m)).toEqual(['me']);
  });

  it('leaves the work to the core where the host cannot mine it', async () => {
    const { core, calls } = setup(async () => {
      throw new Error('no miners here');
    });
    await core.strengthenIdentity(12);
    expect(calls).toEqual([['me', []], ['strengthenIdentity', [12]]]);
  });

  it('reads tries against the usual count, and a time left from the pace', () => {
    expect(workOutlook(12, { tries: 1024, best: 9 }, 10_000)).toEqual({ usual: 4096, share: 0.25, leftMs: 30_000 });
    expect(workOutlook(12, { tries: 10, best: 3 }, 500).leftMs).toBeNull();
    expect(workOutlook(4, { tries: 40, best: 3 }, 10_000)).toMatchObject({ share: 2.5, leftMs: null });
  });
});
