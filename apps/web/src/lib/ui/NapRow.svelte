<script lang="ts">
  import type { PostSummary } from '@hoi/core-api';
  import { ago } from '@hoi/core-api';
  import { to } from '../router';
  import { settle } from '../motion';
  import { napWho } from '../util';
  import CommunityTag from './CommunityTag.svelte';

  // The mods, the community, you, or a labeler you follow put this post to sleep. It naps; you can still peek.
  let { post }: { post: PostSummary } = $props();
  let open = $state(false);
  let box: HTMLElement | undefined = $state();
  $effect(() => {
    if (open) settle(box ?? null);
  });
</script>

<div class="nap anim">
  <div class="zz" aria-hidden="true">zz</div>
  <div class="napbody">
    <p>
      One post is napping. {post.napping ? napWho(post.napping) : 'A labeler labelled it'}: <b>{post.napping?.label}</b>.
      <button type="button" class="peek" aria-expanded={open} onclick={() => (open = !open)}>{open ? 'Hide it again' : 'Peek anyway'}</button>
    </p>
    {#if open}
      <div class="frame peekbox" bind:this={box}>
        <div class="meta">
          <span class="tag tag-peach">{post.napping?.label}</span>
          <CommunityTag c={post.community} />
          <span>{post.author.name}{post.author.name !== post.author.short ? ` · ${post.author.short}` : ''} · {post.author.trust === 'newcomer' ? 'new key · ' : ''}{ago(post.createdAt)}</span>
        </div>
        <a class="peektitle" href={to.post(post.id)}>{post.title}</a>
        <span class="mono">napping on this device only · it still fades on schedule</span>
      </div>
    {/if}
  </div>
</div>

<style>
  .napbody { min-width: 0; }
  .napbody b { color: var(--ink); font-weight: 700; }
  .peekbox { margin-top: 10px; padding: 12px 14px; display: flex; flex-direction: column; gap: 6px; }
  .peektitle { font-weight: 700; overflow-wrap: anywhere; }
  .peektitle:hover { color: var(--cobalt); }
</style>
