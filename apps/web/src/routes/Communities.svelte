<script lang="ts">
  import { ago } from '@hoi/core-api';
  import { app, setTitle } from '../lib/app.svelte';
  import { enterView } from '../lib/motion';
  import { to } from '../lib/router';
  import { plural, inviteQuery } from '../lib/util';
  import { resource } from '../lib/resource.svelte';
  import { router } from '../lib/router.svelte';
  import { REGISTRY_URL } from '../lib/registry';
  import { toast } from '../lib/toast.svelte';
  import Warmth from '../lib/ui/Warmth.svelte';
  import { nav } from '../lib/nav.svelte';
  import { searchDirectory } from '../lib/nearby';

  setTitle('Communities');
  const left = $derived(app.communities.filter((c) => !c.joined));
  const arranged = $derived(nav.arranged);
  const mine = $derived([...arranged.favorites, ...arranged.rest]);
  const warmthOf = (id: string) => app.warmth?.communities.find((w) => w.community === id);
  const directory = resource(() => { void app.rev; void app.net; return app.core.discoverCommunities(); });
  const available = $derived(searchDirectory('', (directory.value ?? []).filter((c) => !app.joined.some((j) => j.id === c.id))));
  // The freshest few here; all of them, searchable, on the Find communities page.
  const FEW = 12;
  const nodes = $derived(app.net?.directory);
  let nodeLink = $state('');

  /** A node link pasted here opens the same page a shared link does: where it points, then connect. */
  function openNode(e: SubmitEvent) {
    e.preventDefault();
    const q = /(?:^hoi:\/\/|#\/)node\?(.+)$/.exec(nodeLink.trim())?.[1];
    if (!q) return toast("That isn't a node link. It starts with hoi://node? or has #/node? in it.");
    nodeLink = '';
    router.go(`#/node?${q}`);
  }
</script>

<section class="view" use:enterView aria-labelledby="cs-h">
  <div class="vh anim"><h1 id="cs-h">Communities</h1></div>
  <div class="btnrow anim top">
    <a class="pill pill-ink" href={to.start()}>+ Start a community</a>
    <a class="pill" href={to.join()}>Open an invite</a>
    <a class="pill" href={to.registry()}>{REGISTRY_URL ? 'Browse public registry' : 'Find communities'}</a>
  </div>
  {#if mine.length === 0}
    <div class="empty anim"><h2>You're not in any yet.</h2><p>Join a community below or open an invite. Creating one with the same name makes a different community.</p></div>
  {:else}
    <div class="rows">
      {#each mine as c (c.id)}
        {@const w = warmthOf(c.id)}
        {@const fav = nav.isFavorite(c.id)}
        <div class="crow anim">
          <span class="cdot bg-{c.color}" aria-hidden="true"></span>
          <a class="cbody" href={to.community(c.id)}>
            <span class="cname">c/{c.name}{#if c.role !== 'member'}<span class="tag tag-sun">{c.role}</span>{/if}{#if c.pinning}<span class="tag tag-soft">pinning</span>{/if}{#if w}<Warmth {w} compact />{/if}</span>
            {#if c.description}<span class="cdesc">{c.description}</span>{/if}
            <span class="meta">{plural(c.posts, 'post')} here · last activity {ago(c.lastActivity)}</span>
            <code class="key" title={c.id}>{c.id}</code>
          </a>
          <button type="button" class={['star', fav && 'faved']} aria-pressed={fav} aria-label={fav ? `Remove c/${c.name} from favorites` : `Add c/${c.name} to favorites`} title="Favorites come first in your list, on this device only" onclick={() => nav.toggleFavorite(c.id)}>{fav ? '★' : '☆'}</button>
        </div>
      {/each}
    </div>
  {/if}
  {#if nav.privateJoined.length}
    <p class="help privnote">{plural(nav.privateJoined.length, 'private community', 'private communities')} hidden here. <a class="textlink" href="#/private">Private communities →</a></p>
  {/if}
  <h2 class="lefth">Found on the network</h2>
  <p class="help">Open communities the nodes this {app.platformName} reached have listed: your home node, your communities' members, the public nodes they passed on, and what those heard from others. This {app.platformName} keeps the list between visits. Nothing here comes from a list built into the app. Names may repeat; compare the community key. Joining subscribes to its posts even when its page is closed.</p>
  {#if nodes}
    <p class="meta nodes" role="status">
      {plural(nodes.known, 'public node')} reached before{nodes.connected ? `, ${nodes.connected} connected now` : ''}{nodes.waiting ? ` · ${nodes.waiting} more to try` : ''}{nodes.listings ? ` · ${plural(nodes.listings, 'community', 'communities')} heard of` : ''}
    </p>
  {/if}
  {#if directory.error}<p class="callout" role="alert">{directory.error} <button type="button" class="pill pill-sm" onclick={() => directory.reload()}>Retry</button></p>
  {:else if directory.loading}<p role="status">Checking connected nodes…</p>
  {:else if !available.length}<p class="callout">{nodes?.waiting && !nodes.connected ? 'Connecting to the nodes this device was told of…' : 'No other communities are available from your current connections.'} Open a node link or an invite someone shared with you, or <a href="https://hoi.sh/download/" target="_blank" rel="noopener">get the home node</a>: a node you keep running is a place where others find communities too.</p>{/if}
  <form class="nodef" onsubmit={openNode}>
    <label class="sr-only" for="cs-node">Node link</label>
    <input id="cs-node" class="input mono-in" bind:value={nodeLink} placeholder="A node link: hoi://node?… or https://…#/node?…" autocomplete="off" />
    <button type="submit" class="pill pill-sm" disabled={!nodeLink.trim()}>Look at this node</button>
  </form>
  <div class="rows">
    {#each available.slice(0, FEW) as c (c.id)}
      <a class="crow" href={to.join(inviteQuery(c.invite) ?? '')}>
        <span class="cdot bg-{c.color}" aria-hidden="true"></span>
        <span class="cbody">
          <span class="cname">c/{c.name}{#if c.live}<span class="tag tag-mint">on a connected node</span>{:else if c.seen}<span class="tag tag-soft">listed {ago(c.seen)}</span>{/if}</span>
          <span class="cdesc">{c.description}</span><code class="key">{c.id}</code><span class="meta">Review and join →</span>
        </span>
      </a>
    {/each}
  </div>
  {#if available.length > FEW}<p class="allof"><a class="pill pill-sm" href={to.registry()}>Search all {available.length} →</a></p>{/if}
  {#if left.length}
    <h2 class="lefth anim">Left, still fading</h2>
    <div class="rows">
      {#each left as c (c.id)}
        <a class="crow anim" href={to.community(c.id)}>
          <span class="cdot bg-{c.color}" aria-hidden="true"></span>
          <span class="cbody"><span class="cname">c/{c.name}</span><span class="meta">not syncing · your copy fades on schedule</span></span>
        </a>
      {/each}
    </div>
  {/if}
</section>

<style>
  .top { padding-bottom: 18px; }
  .crow { display: grid; grid-template-columns: 22px minmax(0, 1fr) auto; gap: 14px; padding: 16px 0; border-top: 2px dotted var(--line); align-items: start; }
  .crow:hover .cname { color: var(--cobalt); }
  .star { width: 40px; height: 40px; border-radius: 50%; font-size: 20px; color: var(--mute); display: inline-flex; align-items: center; justify-content: center; }
  .star:hover { background: var(--soft); color: var(--ink); }
  .star.faved { color: var(--ink); }
  .privnote { padding-top: 10px; }
  .cdot { width: 22px; height: 22px; border-radius: 7px; border: 2px solid var(--ink); margin-top: 3px; }
  .cbody { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  /* Svelte drops the space at the start of an {#if}, so the tags get their distance from a gap. */
  .cname { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 8px; font-size: 20px; font-weight: 800; letter-spacing: -0.02em; overflow-wrap: anywhere; }
  .cdesc { color: var(--mute); font-weight: 500; }
  .lefth { font-size: 22px; font-weight: 800; letter-spacing: -0.02em; padding: 32px 0 10px; }
  .key { font-size: 12px; overflow-wrap: anywhere; color: var(--mute); }
  .nodes { padding-bottom: 8px; }
  .nodef { display: flex; gap: 10px; flex-wrap: wrap; padding-block: 12px 4px; }
  .nodef .input { flex: 1 1 260px; min-width: 0; }
  .allof { padding-top: 12px; }
</style>
