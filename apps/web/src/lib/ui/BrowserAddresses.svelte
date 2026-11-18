<script lang="ts">
  import type { NetStatus } from '@hoi/core-api';
  import { addressScope, connectionIps, hasUnreportedConnections } from '../connection-ip';
  import { rtcAddresses } from '../core/rtc-addresses';
  import { copyText } from '../util';
  import { toast } from '../toast.svelte';

  let { net }: { net: NetStatus | null } = $props();
  const reports = $derived(connectionIps(net, $rtcAddresses));
  const unavailable = $derived(hasUnreportedConnections(net, $rtcAddresses));

  async function copy(address: string) {
    toast(await copyText(address) ? 'Address copied.' : 'Copy did not work here. Select the address and copy it yourself.');
  }
</script>

<div class="browser-addresses">
  {#if !reports.length && !unavailable}
    <p class="state" role="status">No active connections.</p>
  {:else}
    {#if reports.length}
      <ul aria-label="Your connection IP addresses">
        {#each reports as row (row.id)}
          <li>
            <span class="help">{row.observer}</span>
            <div class="address">
              <span class="ip">{row.address}</span>
              <button type="button" class="pill pill-sm" aria-label={`Copy IP address for ${row.observer}`} onclick={() => copy(row.address)}>Copy</button>
            </div>
            <span class="help">{row.source === 'node' ? 'This peer reports seeing your connection from this address.' : 'WebRTC reports this source IP for this connection.'} {row.path}. {addressScope(row.address)}.</span>
          </li>
        {/each}
      </ul>
      <p class="help">Other connections may use a different address.</p>
    {/if}
    {#if unavailable}
      <div class="unavailable" role="status">
        <p class="state">Connection IP unavailable.</p>
        <p class="help">hoi cannot determine the source IP for {reports.length ? 'every connection' : 'this connection'}. A direct peer can still see your source IP. A relay sees the incoming address on a relayed connection.</p>
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
