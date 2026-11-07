<script lang="ts">
  import type { RegistryEntry } from '../registry';
  import { app } from '../app.svelte';
  import { REGISTRY_URL, searchNames, canJoinPublic, registryQuery, publicNameValid } from '../registry';
  import { nearbyCommunities, searchDirectory } from '../nearby';
  import { ago } from '@hoi/core-api';
  import { plural } from '../util';
  import { resource } from '../resource.svelte';
  import { to } from '../router';
  import { errorText } from '../toast.svelte';
  import { inviteQuery } from '../util';

  let { query = '' }: { query?: string } = $props();
  const prefix = $derived(registryQuery(query));
  const valid = $derived(!prefix || publicNameValid(prefix));
  let entries = $state<RegistryEntry[]>([]);
  let loading = $state(false);
  let error = $state('');
  let retry = $state(0);
  // What connected nodes list and this device holds: findable with or without a registry.
  const discovered = resource(() => { void app.rev; void app.net; return app.core.discoverCommunities(); });
  const registryAnswered = $derived(!!REGISTRY_URL && valid && !loading && !error);
  const nearby = $derived(valid ? nearbyCommunities(prefix, discovered.value ?? [], registryAnswered ? new Map(entries.map((e) => [e.community, e.name])) : null) : []);
  // Without a registry, the directory is what the nodes this device reached told it, kept
  // between visits: searched by name and description, live listings first.
  const SHOWN = 60;
  let more = $state(false);
  const found = $derived(REGISTRY_URL ? [] : searchDirectory(query, discovered.value ?? []));
  const shown = $derived(more ? found : found.slice(0, SHOWN));
  $effect(() => {
    void retry;
    const name = prefix;
    entries = [];
    error = '';
    if (!REGISTRY_URL || !valid) { loading = false; return; }
    let active = true;
    loading = true;
    searchNames(name).then(
      (value) => { if (active) { entries = value; loading = false; } },
      (reason) => { if (active) { error = errorText(reason); loading = false; } },
    );
    return () => { active = false; };
  });
</script>

{#if !REGISTRY_URL}
  {#if discovered.error && !discovered.value}
    <div class="callout" role="alert"><p>{discovered.error}</p><button class="pill pill-sm" type="button" onclick={() => discovered.reload()}>Retry</button></div>
  {:else if !discovered.value}
    <p class="loading" role="status">Reading what the nodes listed…</p>
  {:else if !found.length}
    <div class="empty">
      <h2>{query.trim() ? `Nothing matches “${query.trim()}” yet.` : 'No communities heard of yet.'}</h2>
      <p>This {app.platformName} knows the open communities that the nodes it reached listed, and keeps them between visits. Open a node link or an invite to reach more nodes; each one tells you about the communities it knows.</p>
      <a class="textlink" href={to.communities()}>Open a node link →</a>
    </div>
  {:else}
    <p class="hint result-count">{plural(found.length, 'community', 'communities')}{query.trim() ? ` matching “${query.trim()}”` : ' heard of on the network'}</p>
    <div class="registry-results">
      {#each shown as entry (entry.id)}
        <article class="registry-card">
          <div class="heading"><h2>c/{entry.name}</h2>{#if entry.live}<span class="tag tag-mint">on a connected node</span>{:else if entry.seen}<span class="tag tag-soft">listed {ago(entry.seen)}</span>{/if}</div>
          {#if entry.description}<p class="help">{entry.description}</p>{/if}
          <details class="key"><summary>Community key · {entry.id.slice(0, 8)}…{entry.id.slice(-8)}</summary><code>{entry.id}</code></details>
          {#if entry.joined}
            <a class="pill pill-sm" href={to.community(entry.id)}>Open community</a>
          {:else}
            <a class="pill pill-sm" href={to.join(inviteQuery(entry.invite) ?? '')}>Review and join →</a>
          {/if}
        </article>
      {/each}
    </div>
    {#if found.length > shown.length}<button type="button" class="pill pill-sm showmore" onclick={() => (more = true)}>Show all {found.length}</button>{/if}
    <p class="help result-count">Names are labels, not claims: two communities can share one under different keys. Compare the key before you join.</p>
  {/if}
{:else if !valid}
  <p class="callout">Search by community name, for example “bikes” or “c/bikes”. Names use up to 32 letters, digits, dots, underscores or dashes.</p>
{:else if loading}
  <p class="loading" role="status">Checking the public registry…</p>
{:else if error}
  <div class="callout" role="alert"><p>{error}</p><button class="pill pill-sm" type="button" onclick={() => retry++}>Retry registry search</button></div>
{:else if !entries.length}
  <div class="empty"><h2>{prefix ? `No public names starting with “${prefix}”.` : 'No public communities listed yet.'}</h2><p>Try a shorter name, browse your connected nodes, or open a community invite.</p><a class="textlink" href={to.communities()}>Browse connected communities →</a></div>
{:else}
  <p class="hint result-count">{entries.length} {entries.length === 1 ? 'public community' : 'public communities'}{prefix ? ` starting with “${prefix}”` : ''}</p>
  <div class="registry-results">
    {#each entries as entry (entry.community)}
      <article class="registry-card">
        <div class="heading"><h2>c/{entry.name}</h2><span class="tag tag-trust">registered</span></div>
        <details class="key"><summary>Community key · {entry.community.slice(0, 8)}…{entry.community.slice(-8)}</summary><code>{entry.community}</code></details>
        {#if app.joined.some(c => c.id === entry.community)}
          <a class="pill pill-sm" href={to.community(entry.community)}>Open community</a>
        {:else if canJoinPublic(entry)}
          <a class="pill pill-sm" href={to.public(entry.community, entry.name, entry.protocolVersion!)}>Review and join →</a>
        {:else}
          <p class="help">This name is reserved under another version. Use a compatible app and an existing invite.</p>
        {/if}
      </article>
    {/each}
  </div>
  {#if entries.length === 50}<p class="help result-count">Showing the first 50 names. Narrow your search to see more.</p>{/if}
{/if}
{#if REGISTRY_URL && valid && nearby.length}
  <section class="nearby" aria-labelledby="nearby-h">
    <h2 id="nearby-h" class="hint result-count">{nearby.length} {nearby.length === 1 ? 'community' : 'communities'} on connected nodes{prefix ? ` starting with “${prefix}”` : ''}</h2>
    <p class="help">Listed by nodes this {app.platformName} is connected to, or held here. A name is a label, not a claim: two communities can share one under different keys.</p>
    <div class="registry-results">
      {#each nearby as entry (entry.id)}
        <article class="registry-card">
          <div class="heading"><h2>c/{entry.name}</h2><span class={['tag', entry.label === 'same name, other key' ? 'tag-peach' : 'tag-soft']}>{entry.label}</span></div>
          <details class="key"><summary>Community key · {entry.id.slice(0, 8)}…{entry.id.slice(-8)}</summary><code>{entry.id}</code></details>
          {#if entry.description}<p class="help">{entry.description}</p>{/if}
          {#if entry.joined}
            <a class="pill pill-sm" href={to.community(entry.id)}>Open community</a>
          {:else}
            <a class="pill pill-sm" href={to.join(inviteQuery(entry.invite) ?? '')}>Review invite →</a>
          {/if}
        </article>
      {/each}
    </div>
  </section>
{/if}

<style>
  .registry-results { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 250px), 1fr)); gap: 14px; }
  .registry-card { display: flex; flex-direction: column; align-items: flex-start; gap: 14px; min-width: 0; padding: 20px; background: var(--card); border: 2px solid var(--line); border-radius: 18px; }
  .heading { display: flex; flex-wrap: wrap; align-items: center; gap: 9px; min-width: 0; }
  h2 { font-size: 22px; overflow-wrap: anywhere; }
  .key { max-width: 100%; font-size: 12px; color: var(--mute); }
  .key summary { cursor: pointer; overflow-wrap: anywhere; }
  .key code { display: block; margin-top: 8px; overflow-wrap: anywhere; user-select: all; }
  .pill { margin-top: auto; }
  .callout { display: flex; flex-direction: column; align-items: flex-start; gap: 12px; }
  .result-count { padding-block: 10px 14px; }
  .nearby { display: flex; flex-direction: column; gap: 10px; margin-top: 22px; }
  .nearby h2.result-count { font-size: 14px; font-weight: 700; }
  .nearby .help { max-width: 60ch; }
  .showmore { margin-top: 14px; }
  .empty { display: flex; flex-direction: column; gap: 10px; align-items: flex-start; }
  .empty p { max-width: 60ch; }
</style>
