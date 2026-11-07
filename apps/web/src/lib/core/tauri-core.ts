// Desktop backend: native hoi-core inside the Tauri shell (apps/desktop). One command carries
// every API call to the core's JSON dispatcher; stamps are mined on all cores natively; events
// arrive on the `hoi://event` channel.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { CoreEvent, HoiCore } from '@hoi/core-api';
import { createRpcCore, toBase64Url } from './rpc-core';

export async function createTauriCore(): Promise<HoiCore> {
  const call = (method: string, args: unknown[]) => invoke('hoi_call', { method, args });
  const subscribers: ((e: CoreEvent) => void)[] = [];
  await listen<CoreEvent>('hoi://event', (e) => subscribers.forEach((s) => s(e.payload)));
  let rate: Promise<number> | null = null;
  return createRpcCore(
    { call, subscribe: (fn) => void subscribers.push(fn) },
    {
      rate: () => (rate ??= invoke<{ hashesPerSecond: number }>('hoi_benchmark').then((r) => r.hashesPerSecond)),
      mine: (prefix, suffix, bits) => invoke<number>('hoi_mine', { prefix: toBase64Url(prefix), suffix: toBase64Url(suffix), bits }),
      // On every core natively, about a second; too quick (and too far away) to report tries.
      mineIdentity: (rootKey, bits) => call('mineIdentity', [rootKey, bits]) as Promise<number>,
      // More work can take minutes: off the node, read twice a second, stopped on abort.
      mineMore: async (rootKey, bits, onProgress, signal) => {
        const stop = () => void invoke('hoi_identity_stop');
        signal?.addEventListener('abort', stop, { once: true });
        const timer = setInterval(() => {
          void invoke<{ tries: number; best: number } | null>('hoi_identity_progress').then((p) => p && onProgress(p));
        }, 500);
        try {
          return await invoke<number | null>('hoi_mine_identity', { rootKey, bits });
        } finally {
          clearInterval(timer);
          signal?.removeEventListener('abort', stop);
        }
      },
    },
  );
}
