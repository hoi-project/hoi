<script lang="ts">
  import type { CommunitySummary } from '@hoi/core-api';
  import { bytes } from '@hoi/core-api';
  import { app } from '../app.svelte';
  import { nav } from '../nav.svelte';
  import { router } from '../router.svelte';
  import { to } from '../router';

  const r = $derived(router.route);
  const here = (id: string) => (r.name === 'community' || r.name === 'ledger') && r.id === id;
  const nodeTitle = $derived(`This ${app.platformName}`);
  const arranged = $derived(nav.arranged);
  const privates = $derived(nav.privateJoined);
  // A private community you're looking at shows in the list, like the tab you're in.
  const showPrivate = $derived(nav.privateShown || r.name === 'private' || privates.some((c) => here(c.id)));
  let arranging = $state(false);
  let dragging = $state<string | null>(null);

  function drop(e: DragEvent, before: string | null) {
    e.preventDefault();
    const id = dragging;
    dragging = null;
    if (id && id !== before) void nav.move(id, { before });
  }
</script>

{#snippet row(c: CommunitySummary, i: number, group: CommunitySummary[])}
  {@const fav = nav.isFavorite(c.id)}
  <div
    class={['crow', arranging && 'arranging', dragging === c.id && 'dragging']}
    draggable={arranging}
    role="listitem"
    ondragstart={(e) => { dragging = c.id; e.dataTransfer?.setData('text/plain', c.id); }}
    ondragend={() => (dragging = null)}
    ondragover={(e) => { if (dragging && group.some((g) => g.id === dragging)) e.preventDefault(); }}
    ondrop={(e) => drop(e, c.id)}
  >
    <a href={to.community(c.id)} class:on={here(c.id)} aria-current={here(c.id) ? 'page' : undefined}><span class="dot bg-{c.color}"></span><span class="cn" title={`c/${c.name}`}>c/{c.name}</span></a>
    {#if arranging}
      <span class="tools">
        <button type="button" class="tool" aria-label={`Move c/${c.name} up`} disabled={i === 0} onclick={() => nav.move(c.id, { by: -1 })}>↑</button>
        <button type="button" class="tool" aria-label={`Move c/${c.name} down`} disabled={i === group.length - 1} onclick={() => nav.move(c.id, { by: 1 })}>↓</button>
      </span>
    {/if}
    <button type="button" class={['tool', 'star', fav && 'faved']} aria-pressed={fav} aria-label={fav ? `Remove c/${c.name} from favorites` : `Add c/${c.name} to favorites`} title={fav ? 'Favorite (only on this device)' : 'Add to favorites (only on this device)'} onclick={() => nav.toggleFavorite(c.id)}>{fav ? '★' : '☆'}</button>
  </div>
{/snippet}

<aside class="side" aria-label="Communities and this device">
  <nav class="idx" aria-label="Your communities">
    <div class="pinned">
      <a href="#/" class:on={r.name === 'home'} aria-current={r.name === 'home' ? 'page' : undefined}><span class="dot bg-ink"></span>Home</a>
      <a href={to.communities()} class:on={r.name === 'communities'} aria-current={r.name === 'communities' ? 'page' : undefined}><span class="dot bg-sky"></span>Communities</a>
      <a href={to.registry()} class:on={r.name === 'registry'} aria-current={r.name === 'registry' ? 'page' : undefined}><span class="dot bg-sun"></span>Public registry</a>
    </div>

    <div class="group">
      <div class="ghead">
        <span class="lbl">Your communities</span>
        {#if arranged.favorites.length + arranged.rest.length > 1}
          <button type="button" class="arrange" aria-pressed={arranging} onclick={() => (arranging = !arranging)}>{arranging ? 'Done' : 'Arrange'}</button>
        {/if}
      </div>
      {#if arranged.favorites.length}
        <span class="sub" aria-hidden="true">★ Favorites</span>
        <div role="list" aria-label="Favorites">
          {#each arranged.favorites as c, i (c.id)}{@render row(c, i, arranged.favorites)}{/each}
        </div>
      {/if}
      {#if arranged.rest.length}
        {#if arranged.favorites.length}<span class="sub" aria-hidden="true">All</span>{/if}
        <div role="list" aria-label={arranged.favorites.length ? 'Other communities' : 'Communities you are in'}>
          {#each arranged.rest as c, i (c.id)}{@render row(c, i, arranged.rest)}{/each}
        </div>
      {:else if !arranged.favorites.length}
        <span class="none">None yet.</span>
      {/if}
      {#if arranging}<span class="none">Drag, or use the arrows. Only on this {app.platformName}.</span>{/if}
    </div>

    <a href={to.start()} class:on={r.name === 'new'} class="start"><span class="dot plusdot" aria-hidden="true">+</span>Start a community</a>

    <div class="group private">
      <button type="button" class="reveal" aria-expanded={showPrivate} onclick={() => (nav.privateShown = !showPrivate)}>
        <span class="dot bg-lilac" aria-hidden="true"></span>
        <span class="cn">Private</span>
        {#if privates.length}<span class="count num">{privates.length}</span>{/if}
        <span class="eye">{showPrivate ? 'Hide' : 'Show'}</span>
      </button>
      {#if showPrivate}
        {#each privates as c (c.id)}
          <a href={to.community(c.id)} class:on={here(c.id)} aria-current={here(c.id) ? 'page' : undefined}><span class="dot bg-lilac"></span><span class="cn">c/{c.name}</span></a>
        {/each}
        <a href="#/private" class="subtle" class:on={r.name === 'private'} aria-current={r.name === 'private' ? 'page' : undefined}>Join with an invitation →</a>
      {/if}
    </div>

    <a href="#/saved" class:on={r.name === 'saved'} aria-current={r.name === 'saved' ? 'page' : undefined}><span class="dot bg-soft"></span>Saved</a>
  </nav>
  <div class="frame node">
    <b>{nodeTitle}</b>
    <div class="nkv"><span>Stored</span><span class="num">{bytes(app.stats?.storedBytes ?? 0)}</span></div>
    <div class="nkv"><span>Records</span><span class="num">{(app.stats?.records ?? 0).toLocaleString('en')}</span></div>
    {#if app.boot?.platform === 'web'}
      <div class="nkv"><span>Public peers</span><span class="num">{app.net?.peers ?? 0}</span></div>
    {:else}
      <div class="nkv"><span>Passed along</span><span class="num">{bytes(app.stats?.passedAlongBytes ?? 0)}</span></div>
    {/if}
    <div class="nkv"><span>Spam bounced</span><span class="num">{app.stats?.bouncedKeys ?? 0} keys</span></div>
    <a href="#/you" class="textlink">Node settings →</a>
  </div>
</aside>

<style>
  .side {
    display: flex; flex-direction: column; gap: 22px; min-width: 0;
    position: sticky; top: calc(84px + env(safe-area-inset-top, 0px)); align-self: start;
    max-height: calc(100dvh - 100px - env(safe-area-inset-top, 0px)); overflow-y: auto; scrollbar-width: thin;
    /* The pills hang 10px left of the column; room for them inside the scroller, or it clips them. */
    margin-left: -10px; padding-left: 10px;
  }
  .idx { display: flex; flex-direction: column; gap: 2px; }
  /* The shadow covers that hang too, so a highlighted pill scrolling under shows no sliver beside it. */
  .pinned { position: sticky; top: 0; z-index: 1; display: flex; flex-direction: column; gap: 2px; background: var(--bg); box-shadow: -10px 0 var(--bg); padding-bottom: 10px; margin-bottom: 8px; border-bottom: 2px solid var(--ink); }
  .group { display: flex; flex-direction: column; gap: 2px; padding-bottom: 8px; }
  .ghead { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 4px 0 6px; }
  .lbl { font-size: 12px; font-weight: 800; color: var(--mute); letter-spacing: .06em; text-transform: uppercase; }
  .sub { font-size: 11px; font-weight: 800; color: var(--mute); letter-spacing: .04em; padding: 6px 0 2px; }
  .none { font-size: 13px; color: var(--mute); font-weight: 500; padding: 2px 0 4px; }
  .arrange { font-size: 12px; font-weight: 700; color: var(--cobalt); min-height: 28px; padding: 0 8px; border-radius: 999px; }
  .arrange:hover, .arrange[aria-pressed='true'] { background: var(--soft); }
  .idx a { display: flex; align-items: center; gap: 10px; min-height: 38px; font-size: 15px; font-weight: 600; color: var(--ink); border-radius: 12px; padding: 0 10px; margin-left: -10px; overflow-wrap: anywhere; }
  .idx a:hover { background: var(--soft); }
  .idx a.on { background: var(--ink); color: var(--onink); }
  .cn { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .crow { display: flex; align-items: center; gap: 2px; border-radius: 12px; }
  .crow a { flex: 1 1 auto; min-width: 0; }
  .crow.arranging { cursor: grab; }
  .crow.dragging { opacity: .45; }
  .tools { display: inline-flex; }
  .tool { width: 28px; height: 28px; flex: 0 0 28px; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center; font-size: 14px; color: var(--mute); }
  .tool:hover:not(:disabled) { background: var(--soft); color: var(--ink); }
  .tool:disabled { opacity: .3; }
  .star { opacity: 0; font-size: 16px; }
  .crow:hover .star, .star:focus-visible, .star.faved, .arranging .star { opacity: 1; }
  .star.faved { color: var(--ink); }
  .start { margin-top: 2px; border: 2px dashed var(--line); justify-content: flex-start; }
  .idx a.start:hover { border-color: var(--ink); }
  .plusdot { display: inline-flex; align-items: center; justify-content: center; border: 2px dashed var(--line); font-size: 11px; font-weight: 800; line-height: 1; color: var(--mute); }
  .idx a.on .plusdot { border-color: var(--onink); color: var(--onink); }
  .private { margin-top: 14px; padding-top: 10px; border-top: 2px dotted var(--line); }
  .reveal { display: flex; align-items: center; gap: 10px; min-height: 38px; font-size: 15px; font-weight: 600; border-radius: 12px; padding: 0 10px; margin-left: -10px; text-align: left; }
  .reveal:hover { background: var(--soft); }
  .count { font-size: 12px; font-weight: 800; background: var(--lilac); color: var(--tagink); border-radius: 999px; padding: 0 7px; }
  .eye { margin-left: auto; font-size: 12px; font-weight: 700; color: var(--cobalt); }
  .idx a.subtle { font-size: 13px; color: var(--cobalt); font-weight: 700; min-height: 32px; }
  .idx a.subtle.on { color: var(--onink); }
  .node { padding: 14px 16px; display: flex; flex-direction: column; gap: 7px; font-size: 14px; font-weight: 500; }
  .node b { font-weight: 800; font-size: 15px; }
  .nkv { display: flex; justify-content: space-between; gap: 10px; }
  .nkv span:first-child { color: var(--mute); }
  .nkv span:last-child { text-align: right; }
  .textlink { margin-top: 4px; }
  @media (max-width: 1024px) {
    .side { position: static; max-height: none; flex-direction: row; overflow-x: auto; overflow-y: visible; gap: 8px; padding-bottom: 4px; margin-left: 0; padding-left: 0; }
    .idx, .pinned, .group, .group [role='list'] { display: flex; flex-direction: row; align-items: center; gap: 6px; }
    .pinned { position: static; border: 0; padding: 0; margin: 0; box-shadow: none; }
    .crow, .crow a, .idx > a, .pinned a, .reveal { flex: 0 0 auto; }
    .group { padding: 0; }
    .private { margin: 0; padding: 0; border: 0; }
    .lbl, .ghead, .sub, .none, .star, .tools, .node { display: none; }
    .idx a, .reveal { margin: 0; white-space: nowrap; border: 2px solid var(--line); min-height: 40px; overflow-wrap: normal; }
    .idx a.on { border-color: var(--ink); }
    .start { border-style: dashed; }
  }
  @media (max-width: 640px) {
    .side { display: none; }
  }
</style>
