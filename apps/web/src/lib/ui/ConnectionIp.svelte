<script lang="ts">
  import { app } from '../app.svelte';
  import { rtcAddresses } from '../core/rtc-addresses';
  import { addressScope, connectionIps, connectionIpSummary } from '../connection-ip';
  import { copyText } from '../util';
  import { toast } from '../toast.svelte';
  import Modal from './Modal.svelte';

  let open = $state(false);
  const rows = $derived(connectionIps(app.net, $rtcAddresses));
  const summary = $derived(connectionIpSummary(app.net, $rtcAddresses));
  const home = $derived(app.net?.homeNode);
  const relayed = $derived(app.net?.connections?.some(c => c.connected && c.route === 'relay'));
  const vpn = $derived(app.net?.vpn);
  async function copy(address: string) {
    toast(await copyText(address) ? 'Address copied.' : 'Copy did not work here.');
  }
</script>

<div class="connection-ip gutter" aria-label="Connection IP">
  <button type="button" class="readout" onclick={() => (open = true)} aria-haspopup="dialog">
    <span class="label">Connection IP</span><strong>{summary}</strong><span class="details">Details</span>
  </button>
  <span class="vpn">{vpn ? vpn.up ? `VPN route: ${vpn.interface}` : `VPN required: ${vpn.interface} unavailable` : 'VPN not verified'}</span>
</div>

<Modal bind:open title="Your connection IP">
  <p>What an existing connection reports about its current path. Other connections can use different addresses.</p>
  {#if home}<h3>This device to its home node and other connections</h3>{/if}
  {#if rows.length}
    <ul class="reports">
      {#each rows as row (row.id)}
        <li>
          <b>{row.observer}</b>
          <span class="address">{row.address}<button type="button" class="pill pill-sm" onclick={() => copy(row.address)} aria-label={`Copy IP address reported for ${row.observer}`}>Copy</button></span>
          <span class="help">{row.source === 'node' ? 'This node reports seeing you as this address.' : 'WebRTC reports this source address.'} {row.path}. {addressScope(row.address)}.</span>
        </li>
      {/each}
    </ul>
  {:else}<p role="status">{summary}. A node can still see your source address when it connects.</p>{/if}
  {#if home}
    <h3>Home node to other nodes</h3>
    {#if home.connected && home.onward?.length}
      <ul class="reports" aria-label="Home node onward addresses">
        {#each home.onward as path}
          <li><b>{path.transport}</b><span class="address">{path.address}</span><span class="help">{addressScope(path.address)}.</span></li>
        {/each}
      </ul>
    {:else}<p role="status">The home node has no current onward address to report.</p>{/if}
  {/if}
  {#if relayed}<p class="help">A connection uses a packet relay. Its destination has no direct observation of your IP on that route.</p>{/if}
  {#if vpn}
    <p class="help">{vpn.up ? `hoi restricts its internet connections to ${vpn.interface}. Its interface addresses are not its public exit address.` : `${vpn.interface} ${vpn.detail || 'is unavailable'}. hoi’s VPN policy pauses internet connections until it can use that interface.`}</p>
  {/if}
  <p class="help">An address report does not verify a VPN or anonymity. A node can report an incorrect address.</p>
  <p class="help">These reports are held in memory. A home node shares its own onward addresses only with its paired devices. Opening this panel makes no IP lookup or new connection.</p>
  <a class="textlink" href="#/you" onclick={() => (open = false)}>Connection settings and advertised addresses</a>
</Modal>

<style>
  .connection-ip { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-1) var(--space-3); max-width: 1240px; margin: 0 auto; padding-bottom: var(--space-2); font-size: var(--type-meta-size); }
  .readout { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); min-height: var(--space-8); min-width: 0; max-width: 100%; text-align: left; }
  .readout strong { overflow-wrap: anywhere; min-width: 0; font-variant-numeric: tabular-nums; }
  .label, .vpn { color: var(--mute); }
  .details { color: var(--cobalt); text-decoration: underline; text-underline-offset: 3px; }
  .reports { display: grid; gap: var(--space-3); padding: 0; list-style: none; }
  .reports li { display: grid; gap: var(--space-2); min-width: 0; }
  .address { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); overflow-wrap: anywhere; }
</style>
