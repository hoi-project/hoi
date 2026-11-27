<script lang="ts">
  import PublicIpCheck from './PublicIpCheck.svelte';
  import { browserIp } from '../browser-ip.svelte';
  import { app } from '../app.svelte';
  import { BROWSER_EGRESS_PROVIDER } from '../core/browser-egress';
  import { rtcAddresses } from '../core/rtc-addresses';
  import { addressScope, connectionIps, connectionIpSummary } from '../connection-ip';
  import { regionName } from '../geo/tz';
  import { exposureStatus } from '../ip-exposure';
  import { deviceZone, locate, lookupOf, observation } from '../ip-location.svelte';
  import { copyText } from '../util';
  import { toast } from '../toast.svelte';
  import IpStatus from './IpStatus.svelte';
  import Modal from './Modal.svelte';

  let open = $state(false);
  const rows = $derived(connectionIps(app.net, $rtcAddresses));
  const summary = $derived(connectionIpSummary(app.net, $rtcAddresses, browserIp.sample?.address));
  const observations = $derived([
    ...(browserIp.sample ? [observation(browserIp.sample.address, BROWSER_EGRESS_PROVIDER)] : []),
    ...rows.map(row => observation(row.address, row.observer)),
  ]);
  const zone = $derived(deviceZone());
  const status = $derived(exposureStatus({ pending: browserIp.pending, observations, zone: zone.zone, zoneCode: zone.code, name: regionName }));
  // Announced only once a check the person started has a country, never when a sample is cleared.
  const announcement = $derived(['same', 'other', 'no-zone', 'routes', 'unlisted', 'unavailable'].includes(status.kind) ? status.long : '');
  const unlocated = $derived(rows.filter(row => lookupOf(row.address).state === 'idle'));
  let locating = $state(false);
  async function showCountries() {
    locating = true;
    try { await Promise.all(unlocated.map(row => locate(row.address))); } finally { locating = false; }
  }
  function countryOf(address: string): string {
    const found = lookupOf(address);
    if (found.state === 'ready') return found.code ? `Listed in ${regionName(found.code)} (${found.code}).` : 'Not listed in hoi’s country table.';
    return found.state === 'loading' ? 'Finding its country…' : found.state === 'failed' ? 'Its country could not be looked up.' : '';
  }
  const home = $derived(app.net?.homeNode);
  const relayed = $derived(app.net?.connections?.some(c => c.connected && c.route === 'relay'));
  const vpn = $derived(app.net?.vpn);
  async function copy(address: string) {
    toast(await copyText(address) ? 'Address copied.' : 'Copy did not work here.');
  }
</script>

<div class="connection-ip gutter" aria-label="Connection IP">
  <IpStatus {status} onclick={() => (open = true)} />
  {#if !observations.length}<span class="label">{summary}</span>{/if}
  <span class="sr-only" role="status">{announcement}</span>
  {#if vpn}<span class="vpn">{vpn.up ? `VPN route: ${vpn.interface}` : `VPN required: ${vpn.interface} unavailable`}</span>{/if}
</div>

<Modal bind:open title="Your connection IP" wide>
  <PublicIpCheck />
  <h3>Connected peers</h3>
  <p>What an existing connection reports about its current path. Other connections can use different addresses.</p>
  {#if home}<h3>This device to its home node and other connections</h3>{/if}
  {#if rows.length}
    <ul class="reports">
      {#each rows as row (row.id)}
        <li>
          <b>{row.observer}</b>
          <span class="address">{row.address}<button type="button" class="pill pill-sm" onclick={() => copy(row.address)} aria-label={`Copy IP address reported for ${row.observer}`}>Copy</button></span>
          <span class="help">{row.source === 'node' ? 'This node reports seeing you as this address.' : 'WebRTC reports this source address.'} {row.path}. {addressScope(row.address)}. {countryOf(row.address)}</span>
        </li>
      {/each}
    </ul>
    {#if unlocated.length}
      <button type="button" class="pill pill-sm" onclick={showCountries} aria-disabled={locating}>{locating ? 'Finding countries…' : 'Show country'}</button>
      <p class="help">This loads hoi’s country table from this site. The addresses stay on this device.</p>
    {/if}
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
  .label, .vpn { color: var(--mute); }
  .reports { display: grid; gap: var(--space-3); padding: 0; list-style: none; }
  .reports li { display: grid; gap: var(--space-2); min-width: 0; }
  .address { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); overflow-wrap: anywhere; }
</style>
