<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { app } from '../app.svelte';
  import { canCheckRoute, checkConnection } from '../core/connection-check';
  import { addressScope, connectionTarget } from '../connection-ip';
  import { errorText } from '../toast.svelte';

  let { routes, check = checkConnection }: { routes: string[]; check?: typeof checkConnection } = $props();
  const targets = $derived([...new Set(routes)].filter(canCheckRoute));
  let selected = $state('');
  const target = $derived(targets.includes(selected) ? selected : targets[0] || '');
  let busy = $state(false);
  let result = $state<{ address: string; transport: string } | null>(null);
  let error = $state('');
  let controller: AbortController | undefined;
  let generation = 0;
  const vpnRoute = $derived(JSON.stringify(app.net?.vpn ?? null));
  function reset() {
    ++generation;
    controller?.abort();
    result = null; error = ''; busy = false;
  }
  $effect(() => {
    void target; void vpnRoute;
    reset();
  });
  onMount(() => {
    const network = (navigator as Navigator & { connection?: EventTarget }).connection;
    window.addEventListener('online', reset);
    window.addEventListener('offline', reset);
    network?.addEventListener('change', reset);
    return () => {
      window.removeEventListener('online', reset);
      window.removeEventListener('offline', reset);
      network?.removeEventListener('change', reset);
    };
  });
  onDestroy(() => { ++generation; controller?.abort(); });

  async function run() {
    if (!target || busy) return;
    const route = target;
    const current = ++generation;
    controller?.abort();
    controller = new AbortController();
    busy = true; result = null; error = '';
    try {
      const next = await check(route, controller.signal);
      if (current === generation) result = next;
    } catch (e) {
      if (current === generation) error = errorText(e);
    } finally { if (current === generation) busy = false; }
  }
</script>

<section class="connection-check" aria-label="Check connection before joining">
  <h2>Check your connection first</h2>
  {#if target}
    {#if targets.length > 1}
      <label class="field"><span>Node to check</span><select class="input" value={target} onchange={(e) => (selected = e.currentTarget.value)} disabled={busy}>
        {#each targets as route (route)}<option value={route}>{connectionTarget(route)}</option>{/each}
      </select></label>
    {/if}
    <p class="help">Checking contacts <b>{connectionTarget(target)}</b> and reveals your source IP to that node. It sends no profile or community membership and does not join. Only check a node you are willing to contact.</p>
    <button type="button" class="pill" onclick={run} disabled={busy}>{busy ? 'Checking connection…' : 'Check connection'}</button>
    {#if result}
      <div class="result" role="status"><p><b>This node sees you as {result.address}</b></p><p class="help">Reported by {connectionTarget(target)} over {result.transport} for this check. {addressScope(result.address)}. Other connections and your home node’s onward traffic can use different addresses.</p></div>
      <p class="help">Change your VPN or network? Check again before deciding to join. No membership was changed.</p>
    {/if}
    {#if error}<p role="alert">Connection IP unavailable. {error} No community was joined. You can retry or choose whether to continue.</p>{/if}
  {:else}
    <p class="help">This invitation has no route this device can check before joining. Your outgoing IP is unknown. Existing connection reports are in the main Connection IP panel.</p>
    <button type="button" class="pill" disabled>Check connection unavailable</button>
  {/if}
</section>

<style>
  .connection-check { display: grid; gap: var(--space-3); padding: var(--space-4); border: 2px solid var(--line); border-radius: var(--radius-card); min-width: 0; }
  h2 { font-size: var(--type-body-size); font-weight: 800; }
  .connection-check p, .connection-check select { overflow-wrap: anywhere; max-width: 100%; }
  .connection-check button { justify-self: start; }
  .result { display: grid; gap: var(--space-2); min-width: 0; }
  .result b { overflow-wrap: anywhere; }
</style>
