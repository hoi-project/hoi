<script lang="ts">
  import { rtcAddresses } from '../core/rtc-addresses';
  import { copyText } from '../util';
  import { toast } from '../toast.svelte';

  let { connected, unreported }: { connected: number; unreported: boolean } = $props();
  const active = $derived($rtcAddresses.filter((row) => row.connected));
  const known = $derived(active.filter((row) => row.address));
  // Core and RTC observations arrive separately: equal counts do not prove every link is covered.
  const unavailable = $derived(unreported || active.some((row) => !row.address) || connected > active.length);
  const context = (kind: 'node' | 'browser', id: number) => `${kind === 'node' ? 'Node' : 'Browser'} connection ${id}`;

  async function copy(address: string) {
    toast(await copyText(address) ? 'Address copied.' : 'Copy did not work here. Select the address and copy it yourself.');
  }
</script>

<div class="browser-addresses">
  {#if !connected && !active.length}
    <p class="state" role="status">No active connections.</p>
  {:else}
    {#if known.length}
      <ul aria-label="Your connection IP addresses">
        {#each known as row (row.id)}
          <li>
            <span class="help">{context(row.kind, row.id)}</span>
            <div class="address">
              <span class="ip">{row.address}</span>
              <button type="button" class="pill pill-sm" aria-label={`Copy IP address for ${context(row.kind, row.id).toLowerCase()}`} onclick={() => copy(row.address!)}>Copy</button>
            </div>
            <span class="help">WebRTC reports this source IP for this connection.</span>
          </li>
        {/each}
      </ul>
      <p class="help">Shown only here. Other connections may use a different address.</p>
    {/if}
    {#if unavailable || !known.length}
      <div class="unavailable" role="status">
        <p class="state">Connection IP unavailable.</p>
        <p class="help">hoi cannot determine the source IP for {known.length ? 'every connection' : 'this connection'}. The other device can still see it.</p>
      </div>
    {/if}
  {/if}
  <p class="help">hoi cannot verify whether your VPN protects these connections.</p>
</div>

<style>
  .browser-addresses { display: grid; gap: var(--space-3); width: 100%; min-width: 0; }
  ul { display: grid; gap: var(--space-2); list-style: none; margin: 0; padding: 0; }
  li { display: grid; gap: var(--space-2); padding: var(--space-3); border-radius: var(--radius-input); background: var(--soft); min-width: 0; }
  .address { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2) var(--space-3); }
  .ip { font-size: var(--type-body-size); font-variant-numeric: tabular-nums; overflow-wrap: anywhere; min-width: 0; }
  .unavailable { display: grid; gap: var(--space-1); }
  .state { font-size: var(--type-meta-size); }
</style>
