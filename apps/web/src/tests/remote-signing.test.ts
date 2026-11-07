import { describe, expect, it, vi } from 'vitest';
import type { CoreEvent } from '@hoi/core-api';
import { createRpcCore, type Stamper, type Transport } from '../lib/core/rpc-core';

/** A core whose computer signs for it: publishDraft answers `{ signing }`, and the
 * `signed` event comes later. */
function remoteCore(answer: (id: string) => CoreEvent) {
  let emit: (e: CoreEvent) => void = () => {};
  const calls: string[] = [];
  const t: Transport = {
    subscribe: (fn) => { emit = fn; },
    call: async (method, args) => {
      calls.push(method);
      if (method === 'draftPost') return { token: 1, prefix: 'AA', suffix: 'AA', bits: 4 };
      if (method === 'publishDraft') {
        setTimeout(() => emit(answer('ab'.repeat(32))), 5);
        return { signing: 'ab'.repeat(32) };
      }
      if (method === 'published') return { id: args[0], title: 'From the browser' };
      throw { code: 'invalid', message: method };
    },
  };
  const stamper: Stamper = { rate: async () => 1e6, mine: async () => 7, mineIdentity: async () => 0 } as unknown as Stamper;
  return { core: createRpcCore(t, stamper), calls };
}

describe('a browser whose computer signs for it', () => {
  it('waits for the signature, then shows what was signed', async () => {
    const { core, calls } = remoteCore((id) => ({ type: 'signed', id, ok: true, message: null }));
    const post = await core.createPost({ community: 'c'.repeat(64), title: 'From the browser', text: 'hi' } as never);
    expect(post).toMatchObject({ id: 'ab'.repeat(32), title: 'From the browser' });
    expect(calls).toEqual(['draftPost', 'publishDraft', 'published']);
  });
  it('says so when the computer said no', async () => {
    const { core } = remoteCore((id) => ({ type: 'signed', id, ok: false, message: 'You said no on your computer.' }));
    await expect(core.createPost({ community: 'c'.repeat(64), title: 'x', text: 'x' } as never)).rejects.toThrow('said no');
  });
  it('gives up when the computer never answers', async () => {
    vi.useFakeTimers();
    const { core } = remoteCore(() => ({ type: 'notice', text: 'nothing' }));
    const pending = core.createPost({ community: 'c'.repeat(64), title: 'x', text: 'x' } as never);
    const caught = pending.catch((e) => e);
    await vi.advanceTimersByTimeAsync(121_000);
    expect(String((await caught)?.message)).toContain("didn't answer");
    vi.useRealTimers();
  });
});
