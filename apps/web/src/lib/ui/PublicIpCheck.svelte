<script lang="ts">
  import { browserIp, checkBrowserIp } from '../browser-ip.svelte';
  import { BROWSER_EGRESS_PROVIDER, BROWSER_EGRESS_URL } from '../core/browser-egress';
  import { isDesktopShell, openInBrowser } from '../host';
  import { errorText } from '../toast.svelte';
  import IpLocationCard from './IpLocationCard.svelte';

  const desktop = isDesktopShell();
  let openError = $state('');

  async function openProvider() {
    try {
      openError = '';
      if (desktop) await openInBrowser(BROWSER_EGRESS_URL);
      else window.open(BROWSER_EGRESS_URL, '_blank', 'noopener,noreferrer');
    } catch (error) { openError = errorText(error); }
  }
</script>

<section class="public-check" aria-label="Browser public IP">
  <h3>Browser public IP</h3>
  {#if desktop}
    <p class="help">Open {BROWSER_EGRESS_PROVIDER} in your default browser to see the HTTPS exit IP it sees. hoi’s native connections can use a different route.</p>
    <button type="button" class="pill" onclick={openProvider}>Open public IP check in browser</button>
  {:else}
    <p class="help">Checking contacts <strong>{BROWSER_EGRESS_PROVIDER}</strong> over HTTPS and shares your IP with that service. hoi sends no profile or invitation. hoi then finds the IP’s country on this device. Peer connections can use a different route.</p>
    <button type="button" class="pill" aria-disabled={browserIp.pending} onclick={checkBrowserIp}>{browserIp.pending ? 'Checking public IP…' : browserIp.sample ? 'Check public IP again' : 'Check public IP'}</button>
    {#if browserIp.sample}
      <div class="sample" role="status">
        <strong>Browser HTTPS exit IP</strong>
        <output>{browserIp.sample.address}</output>
        <span class="help">Seen by {BROWSER_EGRESS_PROVIDER} at {browserIp.sample.checkedAt.toLocaleTimeString()}. Check again after changing VPNs.</span>
      </div>
      <IpLocationCard address={browserIp.sample.address} observer={BROWSER_EGRESS_PROVIDER} via="Browser HTTPS exit" checkedAt={browserIp.sample.checkedAt} />
    {/if}
    {#if browserIp.error}
      <p role="alert">{browserIp.error}</p>
      <button type="button" class="pill" onclick={openProvider}>Open {BROWSER_EGRESS_PROVIDER} in a new tab</button>
    {/if}
  {/if}
  {#if openError}<p role="alert">{openError}</p>{/if}
</section>

<style>
  .public-check { display: grid; gap: var(--space-2); min-width: 0; text-align: start; }
  h3 { font-size: var(--type-body-size); }
  button { justify-self: start; }
  .sample { display: grid; gap: var(--space-1); }
  output { font-variant-numeric: tabular-nums; overflow-wrap: anywhere; }
</style>
