<script lang="ts">
  import { onMount } from 'svelte';
  import type { PublicJoinStatus } from '@hoi/core-api';
  import { app, setTitle } from '../lib/app.svelte';
  import { publicTarget } from '../lib/public-join';
  import { REGISTRY_URL, registryClient, canJoinPublic, type RegistryMetadata } from '../lib/registry';
  import { errorText } from '../lib/toast.svelte';
  import { to } from '../lib/router';
  import { RELEASE } from '../lib/preview';
  import CopyField from '../lib/ui/CopyField.svelte';
  import JoinWarning from '../lib/ui/JoinWarning.svelte';
  import { publicRoutes } from '../lib/public-connections';

  let { query }: { query: string } = $props();
  const parsed = $derived.by(() => { try { return { target: publicTarget(query), error: '' }; } catch (e) { return { target: null, error: errorText(e) }; } });
  let metadata = $state<RegistryMetadata | null>(null);
  let serviceError = $state('');
  let error = $state('');
  let busy = $state(false);
  let joinWarning = $state(false);
  let status = $state<PublicJoinStatus | null>(null);
  /** Nodes that announced they carry this community come first; the registry's general list follows. */
  let carrying = $state<string[]>([]);
  const routes = $derived([...new Set([...carrying, ...(metadata?.publicNodes ?? [])])]);
  const usableRoutes = $derived(publicRoutes(routes, app.boot?.platform ?? 'web'));
  let active = true;
  setTitle('Join a public community');

  async function loadNodes() {
    if (!REGISTRY_URL) { serviceError = 'Public discovery is unavailable. Your existing connections may still reach this community.'; return; }
    try { metadata = await registryClient(REGISTRY_URL).metadata(); serviceError = ''; }
    catch (e) { serviceError = errorText(e); }
    // A missing per-community answer is not an outage: the general list still applies.
    if (parsed.target) carrying = await registryClient(REGISTRY_URL).nodesFor(parsed.target.id).catch(() => []);
  }

  async function refresh() {
    if (!parsed.target) return;
    try {
      const next = await app.core.publicJoinStatus(parsed.target.id);
      if (!active) return;
      const completed = next.status === 'joined' && status?.status !== 'joined';
      status = next;
      if (completed) await app.refresh();
    } catch (e) { if (active) error = errorText(e); }
  }

  async function join() {
    const target = parsed.target;
    if (!target?.compatible || busy || status?.status === 'joined') return;
    busy = true; error = '';
    await loadNodes();
    try {
      if (target.name) {
        const listing = await registryClient(REGISTRY_URL).lookupIndexed(target.name);
        if (!listing || listing.community !== target.id || listing.protocolVersion !== target.protocol || !canJoinPublic(listing)) {
          throw new Error('This public link does not match the verified community name. Use an invite from someone you trust.');
        }
      }
      status = await app.core.publicJoin(target.id, routes.slice(0, 16), target.name || undefined);
      await app.refresh();
    } catch (e) { error = errorText(e); }
    busy = false;
  }

  async function cancel() {
    if (!parsed.target) return;
    busy = true;
    try { await app.core.leave(parsed.target.id); status = null; await app.refresh(); }
    catch (e) { error = errorText(e); }
    busy = false;
  }

  onMount(() => {
    active = true;
    void loadNodes(); void refresh();
    const timer = setInterval(() => { if (status?.status === 'pending' && !busy) void refresh(); }, 2000);
    return () => { active = false; clearInterval(timer); };
  });
</script>

<section class="view" aria-labelledby="public-heading">
  <div class="vh"><h1 id="public-heading">Join a public community</h1></div>
  {#if parsed.error}<p class="callout" role="alert">{parsed.error}</p>
  {:else if parsed.target}
    {@const target = parsed.target}
    <div class="public-card frame">
      <h2>{target.name ? `c/${target.name}` : 'A community, by its key'}</h2>
      <p>You can join and read on your phone without your own home node. A public relay or reachable member supplies the community's signed definition and posts, which this app verifies. Once received, saved posts remain readable offline. Posting needs a connection carrying this community and follows its admission rules.</p>
      <CopyField label="Community key" value={target.id} what="Community key copied" />
      {#if !target.compatible}
        <p class="callout" role="alert">This link uses protocol {target.protocol}. This app supports {RELEASE.supportedProtocols.join(', ')}. Use a compatible app and the community's saved invite. Its name remains reserved; a new index does not migrate its data.</p>
      {:else if status?.status === 'joined'}
        <p class="callout" role="status">The signed community was received and verified. Posts sync while a home node carrying it is reachable, even when you open another page. You can read saved posts offline; nobody can read what you post until a home node carrying it has it.</p>
        <a class="pill pill-ink" href={to.community(target.id)}>Open community</a>
      {:else}
        {#if status?.status === 'pending'}
          <p class="callout" role="status">Waiting for a node with this community. The join request is saved on this device; no signed community has been received yet. Keep the app running and connect to a carrying node. The author can be offline if another online node has saved its data.</p>
        {:else if status?.status === 'rejected'}
          <p class="callout" role="alert">{status.reason}</p>
        {/if}
        {#if serviceError}<p class="callout">{serviceError} A registry outage does not disable existing communities or invites.</p>{/if}
        {#if metadata && !routes.length}<p class="callout">No public peer is available from this registry yet. Ask a member for an invite with a reachable peer. A reserved name alone cannot deliver posts; the static app host does not supply them.</p>{/if}
        {#if routes.length && !usableRoutes.length}<p class="callout">Public connections are unavailable on this device. Ask a member for an invite, or try an existing connection to this community.</p>{/if}
        {#if routes.length}<p class="help">Joining connects to public nodes listed by the registry{carrying.length ? ', those carrying this community first' : ''}. These nodes carry community data separately from the name registry; network peers can see your connection.</p>{/if}
        <div class="btnrow">
          <button type="button" class="pill pill-ink" onclick={() => (joinWarning = true)} disabled={busy}>{busy ? 'Requesting…' : status?.status === 'pending' ? 'Retry public nodes' : 'Join public community'}</button>
          {#if status?.status === 'pending' || status?.status === 'rejected'}<button type="button" class="pill" onclick={cancel} disabled={busy}>Cancel request</button>{/if}
        </div>
      {/if}
      {#if error}<p class="callout" role="alert">{error}</p>{/if}
      <p class="help">The registry helps people find this key. Removing its listing cannot revoke the community key or erase peer copies. Existing invites remain usable with compatible software and reachable peers.</p>
      <a href={to.join()}>Open a member's invite instead</a>
    </div>
  {/if}
</section>

<JoinWarning bind:open={joinWarning} oncontinue={join} />

<style>
  .public-card { display: flex; flex-direction: column; align-items: flex-start; gap: 18px; padding: 24px; max-width: 760px; min-width: 0; }
  .public-card :global(.field) { width: 100%; min-width: 0; }
  h2 { overflow-wrap: anywhere; }
  @media (max-width: 600px) { .public-card { padding: 16px; } }
</style>
