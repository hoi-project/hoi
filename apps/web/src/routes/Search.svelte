<script lang="ts">
  import { app, setTitle } from '../lib/app.svelte';
  import { resource } from '../lib/resource.svelte';
  import { enterView } from '../lib/motion';
  import { router } from '../lib/router.svelte';
  import { to, type SearchScope } from '../lib/router';
  import PostRow from '../lib/ui/PostRow.svelte';
  import RegistryResults from '../lib/ui/RegistryResults.svelte';
  import { REGISTRY_URL } from '../lib/registry';

  let { q, scope = 'posts' }: { q: string; scope?: SearchScope } = $props();
  // svelte-ignore state_referenced_locally
  let input = $state(q);
  let el: HTMLInputElement | undefined = $state();
  $effect(() => {
    input = q;
  });
  $effect(() => setTitle(q ? `“${q}”` : 'Search'));
  $effect(() => {
    if (!q) el?.focus();
  });

  const results = resource(() => {
    void app.rev;
    return scope === 'posts' && q.trim() ? app.core.search(q.trim()) : Promise.resolve([]);
  });

  function go(e: SubmitEvent) {
    e.preventDefault();
    router.replace(to.search(input.trim(), scope));
  }
</script>

<section class="view" use:enterView aria-labelledby="q-h">
  <div class="vh anim"><h1 id="q-h">Search</h1><a class="textlink" href={to.registry()}>{REGISTRY_URL ? 'Browse registry →' : 'Browse communities →'}</a></div>
  <nav class="tabs scopes" aria-label="Search in">
    <a class="tab" class:on={scope === 'posts'} aria-current={scope === 'posts' ? 'page' : undefined} href={to.search(q)}>Posts on this device</a>
    <a class="tab" class:on={scope === 'communities'} aria-current={scope === 'communities' ? 'page' : undefined} href={to.search(q, 'communities')}>Public communities</a>
  </nav>
  <form class="qform anim" role="search" aria-label="Search this page" onsubmit={go}>
    <label class="sr-only" for="q2">Search</label>
    <input id="q2" class="input" type="search" bind:this={el} bind:value={input} placeholder={scope === 'communities' ? 'Community name, e.g. bikes or c/bikes' : `Search posts on this ${app.platformName}`} aria-describedby="search-help" />
    <button class="pill pill-ink" type="submit">Search</button>
  </form>
  <p id="search-help" class="help search-help">{scope === 'communities' ? (REGISTRY_URL ? 'Community names are looked up in the public registry. Searching does not join or download posts.' : 'Communities the nodes you reached listed, searched on this device. Searching does not join or download posts.') : 'Titles, posts and replies synced here. Post searches stay on this device.'}</p>
  {#if scope === 'communities'}
    <RegistryResults query={q} />
  {:else if !q}
    <p class="loading">Titles, posts and replies you have here. Nobody else sees what you search for.</p>
  {:else if results.error}
    <p class="callout" role="alert">{results.error} <button class="pill pill-sm" type="button" onclick={() => results.reload()}>Retry</button></p>
  {:else if results.value && results.value.length === 0}
    <div class="empty anim"><h2>Nothing for “{q}”.</h2><p>Only what this {app.platformName} has synced is searchable. Posts that faded are gone for good.</p></div>
  {:else if results.value}
    <p class="hint count">{results.value.length} {results.value.length === 1 ? 'match' : 'matches'} for “{q}”</p>
    <div class="rows">{#each results.value as p (p.id)}<PostRow post={p} />{/each}</div>
  {:else}
    <p class="loading" role="status">Looking…</p>
  {/if}
</section>

<style>
  .qform { display: flex; gap: 8px; max-width: 640px; padding-bottom: 18px; }
  .qform .input { flex: 1; }
  .scopes { margin-bottom: 16px; }
  .search-help { margin-bottom: 16px; }
  .count { padding-bottom: 10px; }
</style>
