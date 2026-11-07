<script lang="ts">
  import type { FeedSort, PostSummary } from '@hoi/core-api';
  import { onDestroy } from 'svelte';
  import { app, setTitle } from '../lib/app.svelte';
  import { resource } from '../lib/resource.svelte';
  import { enterView } from '../lib/motion';
  import { to } from '../lib/router';
  import PostRow from '../lib/ui/PostRow.svelte';
  import Composer from '../lib/ui/Composer.svelte';
  import { startPost } from '../lib/compose';

  const SORTS: { id: FeedSort; label: string }[] = [
    { id: 'hot', label: 'Hot' },
    { id: 'fresh', label: 'Fresh' },
    { id: 'loudest', label: 'Loudest' },
    { id: 'friends', label: 'Friends' },
    { id: 'counted', label: 'Counted' },
  ];
  let sort = $state<FeedSort>(remembered());
  // Posts you sign during this visit stay on top, where you saw them land.
  let fresh = $state.raw<PostSummary[]>([]);

  const feed = resource(() => {
    void app.rev;
    return app.core.feed({ sort, limit: 60 });
  });
  const top = $derived(fresh.map((f) => feed.value?.find((p) => p.id === f.id) ?? f));
  const rest = $derived((feed.value ?? []).filter((p) => !fresh.some((f) => f.id === p.id)));
  const composing = $derived(app.composer.open && !app.composer.community);

  $effect(() => setTitle(app.me ? `Hey ${app.me.name}` : undefined));
  onDestroy(() => {
    if (app.composer.open && !app.composer.community) app.closeComposer();
  });

  function remembered(): FeedSort {
    try {
      const s = sessionStorage.getItem('hoi:sort');
      if (s === 'hot' || s === 'fresh' || s === 'loudest' || s === 'friends' || s === 'counted') return s;
    } catch {
      /* ignore */
    }
    return 'hot';
  }
  function pick(s: FeedSort) {
    sort = s;
    try {
      sessionStorage.setItem('hoi:sort', s);
    } catch {
      /* ignore */
    }
  }
</script>

<section class="view" use:enterView aria-labelledby="home-h">
  <div class="vh anim">
    <h1 id="home-h">Hey {app.me?.name}.</h1>
    <div class="tabs" role="group" aria-label="Sort the feed">
      {#each SORTS as s (s.id)}
        <button type="button" class={['tab', sort === s.id && 'on']} aria-pressed={sort === s.id} onclick={() => pick(s.id)}>{s.label}</button>
      {/each}
    </div>
    <span class="hint">ranked right here, weighted by people you trust</span>
  </div>

  {#if composing}
    {#key app.composer.nonce}
      <Composer onposted={(p) => (fresh = [p, ...fresh])} onclose={() => app.closeComposer()} />
    {/key}
  {/if}

  {#if app.joined.length === 0}
    <div class="empty anim">
      <span class="sticker">nothing here yet</span>
      <h2>hoi has no front page. That's the point.</h2>
      <p>
        Posts come from communities you're in, straight from the people in them, and get ranked on this {app.platformName}.
        Start one for a few people you know, or open an invite someone sent you.
      </p>
      <div class="btnrow">
        <a class="pill pill-ink" href="#/new">Start a community</a>
        <a class="pill" href={to.join()}>Open an invite</a>
      </div>
    </div>
  {:else if feed.value && top.length + rest.length === 0}
    <div class="empty anim">
      {#if sort === 'friends'}
        <h2>No friends posted lately.</h2>
        <p>Trust someone from their post or their page and their posts show up here. Trust stays on this device.</p>
      {:else}
        <h2>Quiet in here.</h2>
        <p>
          Posts arrive from the people in your communities.
          {#if !app.net?.peers}New updates need a home node carrying this community: a member's, a public one, or your own. You can <a class="textlink" href={to.communities()}>find communities</a>, open a node link or a member's invite.{/if}
          Use <b>+ Post</b> to write; sending becomes available when this community connects.
        </p>
      {/if}
      <div class="btnrow"><button type="button" class="pill pill-ink" onclick={() => startPost()}>+ Post</button></div>
    </div>
  {:else if feed.error && !feed.value}
    <p class="loading">{feed.error}</p>
  {:else if !feed.value}
    <p class="loading" role="status">Ranking what's on this {app.platformName}…</p>
  {:else}
    <div class="rows" id="feed">
      {#each top as p (p.id)}<PostRow post={p} fresh />{/each}
      {#each rest as p (p.id)}<PostRow post={p} />{/each}
    </div>
  {/if}
</section>
