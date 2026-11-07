// Object URLs for media a post carries. The bytes come from the core, which checked them (type
// sniffed from the bytes, metadata stripped, structure strict); they play from blob: URLs and
// nothing here ever loads media from the network.
import type { MediaView } from '@hoi/core-api';
import { onDestroy } from 'svelte';
import { app } from './app.svelte';

export interface MediaUrls {
  /** The blob: URL of an item whose bytes are open. */
  url(hash: string): string | undefined;
  /** Opens the bytes of items that are on this device, each once. */
  open(items: MediaView[]): void;
}

/** Call while a component sets up: its URLs are revoked when it goes. */
export function mediaUrls(): MediaUrls {
  const made = new Map<string, string>();
  const urls = $state<Record<string, string>>({});
  let gone = false;
  onDestroy(() => {
    gone = true;
    made.forEach((u) => u && URL.revokeObjectURL(u));
  });
  return {
    url: (hash) => urls[hash],
    open(items) {
      for (const m of items) {
        if (!m.available || made.has(m.hash)) continue;
        made.set(m.hash, '');
        app.core
          .media(m.hash)
          .then((b) => {
            if (gone) return;
            const u = URL.createObjectURL(new Blob([b.data as BlobPart], { type: b.mime }));
            made.set(m.hash, u);
            urls[m.hash] = u;
          })
          .catch(() => made.delete(m.hash));
      }
    },
  };
}
