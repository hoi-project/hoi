<script lang="ts">
  import type { PostSummary } from '@hoi/core-api';
  import { ago } from '@hoi/core-api';
  import { app, attempt } from '../app.svelte';
  import { to } from '../router';
  import { copyText, plural, safeHttpUrl } from '../util';
  import { toast, toastError } from '../toast.svelte';
  import { dropIn } from '../motion';
  import Vote from './Vote.svelte';
  import Avatar from './Avatar.svelte';
  import CommunityTag from './CommunityTag.svelte';
  import TrustTag from './TrustTag.svelte';
  import LinkCard from './LinkCard.svelte';
  import HandleTag from './HandleTag.svelte';
  import LifeBar from './LifeBar.svelte';
  import NapRow from './NapRow.svelte';
  import MediaPreview from './MediaPreview.svelte';

  let {
    post,
    showCommunity = true,
    showExcerpt = true,
    fresh = false,
  }: { post: PostSummary; showCommunity?: boolean; showExcerpt?: boolean; fresh?: boolean } = $props();

  let saved = $derived(post.saved);
  let el: HTMLElement | undefined = $state();
  const url = $derived(safeHttpUrl(post.link));
  const role = $derived(app.communities.find((c) => c.id === post.community.id)?.role ?? 'member');
  const canMod = $derived(role !== 'member');
  // the first picture shows in the row; other media is named
  const picture = $derived(post.media.find((m) => m.mime.startsWith('image/')));

  $effect(() => {
    if (fresh && el) dropIn(el);
  });

  async function share() {
    try {
      const inv = await app.core.invite(post.community.id);
      if (window.matchMedia?.('(pointer: coarse)').matches && 'share' in navigator) {
        try {
          await navigator.share({ title: post.title, text: `Join c/${post.community.name} on hoi`, url: inv.web });
          return;
        } catch {
          /* fall back to copying */
        }
      }
      if (await copyText(inv.web)) toast(`Invite link to c/${post.community.name} copied`);
      else toast('Copy did not work here. Use Invite a friend on the community page.');
    } catch (e) {
      toastError(e);
    }
  }

  async function toggleSave() {
    const next = !saved;
    saved = next;
    const ok = await attempt(() => app.core.save(post.id, next), next ? `Saved on this ${app.platformName}` : 'Unsaved');
    if (ok === undefined) saved = !next;
  }

  async function admit() {
    await attempt(() => app.core.admit(post.community.id, post.author.key), `${post.author.name} is in. Signed into the ledger.`);
  }
  // Why Hot puts it here: votes from people in your ring.
  const why = $derived([
    post.friendsUp ? plural(post.friendsUp, 'friend') : '',
    post.fofUp ? (post.fofUp === 1 ? '1 friend of a friend' : `${post.fofUp} friends of friends`) : '',
  ].filter(Boolean).join(' · '));
</script>

{#if post.napping}
  <NapRow {post} />
{:else}
  <article class="row anim" bind:this={el} aria-labelledby={`t-${post.id}`}>
    <Vote target={post.id} community={post.community.id} score={post.score} myVote={post.myVote} pinned={post.pinned} />
    <div class="rowbody">
      <div class="meta">
        {#if showCommunity}<CommunityTag c={post.community} />{/if}
        <a class="author" href={to.person(post.author.key)}><Avatar name={post.author.name} keyHex={post.author.key} avatar={post.author.avatar} avatarColor={post.author.avatarColor} size="sm" /><span class="who">{post.author.name}</span></a>
        {#if post.author.name !== post.author.short}<span class="mono">{post.author.short}</span>{/if}
        <HandleTag person={post.author} />
        <span>· {ago(post.createdAt)}</span>
        {#if post.pinned && showCommunity}<span class="tag tag-sun">pinned</span>{/if}
        {#if post.waiting}<span class="tag tag-dashed">waiting at the door</span>{:else if post.unvalidated}<span class="tag tag-dashed" title="Nobody established here has vouched for them yet">new here</span>{/if}
        {#if post.held}<span class="tag tag-peach" title={post.held}>held back</span>{/if}
        {#if why}<span class="tag tag-trust" title="Hot counts your friends' votes fully and friends of friends' by half: that is why this sits where it does.">↑ {why}</span>{/if}
        {#each post.tallies ?? [] as t (t.key)}<span class="tag tag-soft" title={`${t.by}, an aggregator you follow, counted this ${ago(t.at)}`}>{t.up} ↑ · {t.down} ↓ by {t.by}</span>{/each}
        <span class="push">
          {#if post.mine}
            <span class="tag tag-sun">yours · {post.short}</span>
          {:else}
            <TrustTag person={post.author} />
          {/if}
        </span>
      </div>
      <a class="title" id={`t-${post.id}`} href={to.post(post.id)}>{post.title}</a>
      {#if post.link}
        {#if url}
          <LinkCard {url} />
        {:else}
          <span class="linkchip">{post.link}</span>
        {/if}
      {/if}
      {#if showExcerpt && post.excerpt}<p class="excerpt clamp">{post.excerpt}</p>{/if}
      {#if picture}
        <MediaPreview item={picture} href={to.post(post.id)} more={post.media.length - 1} />
      {:else if post.media.length}
        <span class="mono">{post.media.length === 1 ? post.media[0].mime.split('/')[0] : `${post.media.length} media items`}</span>
      {/if}
      {#if post.labels.length}
        <div class="meta">{#each post.labels as l (l)}<span class="tag tag-sky">note: {l}</span>{/each}</div>
      {/if}
      <div class="acts">
        <a class="act" href={to.post(post.id)}>{plural(post.replies, 'reply', 'replies')}</a>
        {#if !app.privateGroups.some(g => g.community === post.community.id)}<button type="button" class="act" onclick={share}>Share invite</button>{/if}
        <button type="button" class={['act', saved && 'on']} aria-pressed={saved} onclick={toggleSave}>{saved ? '★ Saved' : 'Save'}</button>
        {#if post.waiting && canMod}<button type="button" class="act on" onclick={admit}>Admit {post.author.name}</button>{/if}
        <LifeBar createdAt={post.createdAt} expiresAt={post.expiresAt} />
      </div>
    </div>
  </article>
{/if}

<style>
  .author { display: inline-flex; align-items: center; gap: 7px; }
  .clamp { display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .who:hover { color: var(--cobalt); }
</style>
