<script lang="ts">
  // You → Your addresses: every address this node gives out, one row per IP, blurred until
  // pointed at (screenshots and onlookers see nothing), copied with a click; where each reaches
  // and who hears it as colored tags.
  import type { Exposure } from '@hoi/core-api';
  import { app } from '../app.svelte';
  import { addressRows } from '../public-node';
  import { toast } from '../toast.svelte';
  import { copyText } from '../util';
  import { nudge } from '../motion';

  const rows = $derived(addressRows(app.net?.exposure ?? [], app.net?.rtc ?? []));

  const reach: Record<Exposure['scope'], { text: string; tag: string }> = {
    internet: { text: 'internet', tag: 'tag-coral' },
    network: { text: 'home network', tag: 'tag-sky' },
    device: { text: 'this computer only', tag: 'tag-soft' },
  };
  const heard: Record<Exposure['told'][number], { text: string; tag: string }[]> = {
    devices: [{ text: 'your devices', tag: 'tag-mint' }],
    strangers: [{ text: 'anyone who connects', tag: 'tag-peach' }, { text: 'invites', tag: 'tag-peach' }],
    'node-link': [{ text: 'node link', tag: 'tag-sun' }],
  };

  async function copy(e: MouseEvent, host: string) {
    if (await copyText(host)) {
      nudge(e.currentTarget as Element);
      toast('Address copied.');
    } else toast('Copy did not work here.');
  }
</script>

{#if rows.length}
  <ul class="addrs" aria-label="This {app.platformName}'s addresses">
    {#each rows as r (r.host)}
      <li>
        <button type="button" class="ip mono" onclick={(e) => copy(e, r.host)} title="Click to copy" aria-label={`Copy address ${r.host}`}>
          <span class="val">{r.host}</span>
        </button>
        <span class="ports">
          {#each r.ports as p (p.port)}<span class="port" title={`For ${p.who}`}><b class="num">:{p.port}</b> {p.who}</span>{/each}
        </span>
        <span class="tags">
          <span class={['tag', reach[r.scope].tag]}>{reach[r.scope].text}{r.fromRouter ? ' · via router' : ''}</span>
          <span class="arrow" aria-hidden="true">→</span><span class="sr-only">told to</span>
          {#each r.told.flatMap((t) => heard[t]) as h (h.text)}<span class={['tag', h.tag]}>{h.text}</span>{/each}
        </span>
      </li>
    {/each}
  </ul>
{/if}

<style>
  .addrs { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; border: 2px solid var(--line); border-radius: var(--radius-input); width: 100%; }
  li { display: grid; grid-template-columns: minmax(130px, auto) minmax(0, 1fr); gap: 6px 14px; align-items: center; padding: 10px 14px; }
  li + li { border-top: 1px solid var(--line); }
  .ip { justify-self: start; font-size: 14px; color: var(--ink); font-weight: 600; padding: 4px 8px; border-radius: 8px; background: var(--soft); cursor: copy; }
  .ip .val { filter: blur(6px); transition: filter .2s ease; user-select: none; }
  .ip:hover .val, .ip:focus-visible .val { filter: none; user-select: text; }
  .ports { display: flex; flex-wrap: wrap; gap: 4px 12px; font-size: 13px; color: var(--mute); font-weight: 500; }
  .port { white-space: nowrap; }
  .port b { color: var(--ink); font-family: var(--mono); font-weight: 600; }
  .tags { grid-column: 1 / -1; display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
  .arrow { color: var(--mute); font-weight: 800; }
  @media (prefers-reduced-motion: reduce) { .ip .val { transition: none; } }
  @media (max-width: 520px) { li { grid-template-columns: 1fr; } }
</style>
