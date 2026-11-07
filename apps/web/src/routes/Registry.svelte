<script lang="ts">
  import { setTitle } from '../lib/app.svelte';
  import { enterView } from '../lib/motion';
  import { router } from '../lib/router.svelte';
  import { to } from '../lib/router';
  import RegistryResults from '../lib/ui/RegistryResults.svelte';
  import { REGISTRY_URL } from '../lib/registry';

  let { q = '' }: { q?: string } = $props();
  let input = $state('');
  $effect(() => { input = q; setTitle(REGISTRY_URL ? 'Community registry' : 'Find communities'); });
  function search(event: SubmitEvent) { event.preventDefault(); router.replace(to.registry(input.trim())); }
</script>

<section class="view" use:enterView aria-labelledby="registry-h">
  <div class="vh anim"><h1 id="registry-h">{REGISTRY_URL ? 'Community registry' : 'Find communities'}</h1></div>
  <p class="intro">{REGISTRY_URL ? 'Find public communities by name. Review a community before you join; a member or community node must be reachable to receive its posts.' : 'Every open community the nodes you reached have listed, kept on this device and growing with each node you reach. Review one before you join; a node that carries it must be reachable to receive its posts.'}</p>
  <nav class="btnrow destinations" aria-label="Community discovery"><a class="pill pill-sm" href={to.communities()}>Your communities</a><a class="pill pill-sm" href={to.join()}>Open an invite</a></nav>
  <form class="search-form" role="search" aria-label="Search the registry" onsubmit={search}>
    <label class="field"><span>{REGISTRY_URL ? 'Community name' : 'Name or topic'}</span><input type="search" class="input" bind:value={input} maxlength={REGISTRY_URL ? 34 : 80} placeholder="bikes or c/bikes" autocomplete="off" spellcheck="false" aria-describedby="registry-help" /></label>
    <button class="pill pill-ink" type="submit">{REGISTRY_URL ? 'Search registry' : 'Search'}</button>
  </form>
  <p class="help" id="registry-help">{REGISTRY_URL ? 'Your browser contacts the registry directly for public names, signed keys and peer routes. The static app host does not handle this lookup or supply community content. The registry receives connection metadata, but no posts or private-community details.' : 'Searching happens on this device: nobody learns what you look for. Each listing is a signed community definition with the routes of the nodes that carry it.'}</p>
  <RegistryResults query={q} />
</section>

<style>
  .intro { max-width: 650px; color: var(--mute); line-height: 1.6; }
  .destinations { margin-block: 18px 24px; align-items: center; }
  .search-form { display: flex; align-items: end; gap: 10px; margin-bottom: 8px; max-width: 640px; }
  .field { flex: 1; }
  #registry-help { margin-bottom: 16px; }
  @media (max-width: 520px) { .search-form { flex-wrap: wrap; } .field { flex-basis: 100%; } }
</style>
