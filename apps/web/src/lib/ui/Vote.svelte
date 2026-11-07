<script lang="ts">
  import type { Hex } from '@hoi/core-api';
  import { app, signedAndSent } from '../app.svelte';
  import { animate, bounce, reduced, utils } from '../motion';
  import { toast, toastError } from '../toast.svelte';

  let {
    target,
    community = undefined,
    score,
    myVote,
    horizontal = false,
    pinned = false,
    disabled = false,
    onvoted,
  }: {
    target: Hex;
    community?: Hex;
    score: number;
    myVote: -1 | 0 | 1;
    horizontal?: boolean;
    pinned?: boolean;
    disabled?: boolean;
    onvoted?: (score: number, myVote: -1 | 0 | 1) => void;
  } = $props();

  // Local copies that follow the props but can be changed optimistically.
  let mine = $derived(myVote);
  let count = $derived(score);
  let shown = $derived(score);
  let busy = $state(false);
  let box: HTMLElement | undefined = $state();

  function roll(from: number, to: number) {
    if (reduced() || from === to) {
      shown = to;
      return;
    }
    const o = { v: from };
    animate(o, { v: to, duration: 350, ease: 'outQuad', modifier: utils.round(0), onUpdate: () => (shown = o.v) });
  }

  async function vote(dir: 1 | -1, e: MouseEvent) {
    if (busy || disabled) return;
    const at = e.currentTarget as Element;
    if (!(await app.confirmPosting(community))) return;
    const next: -1 | 0 | 1 = mine === dir ? 0 : dir;
    const before = { mine, count };
    const optimistic = count - mine + next;
    mine = next;
    count = optimistic;
    roll(before.count, optimistic);
    bounce(box ?? null, dir, at);
    busy = true;
    try {
      const real = await app.core.vote(target, next);
      if (real !== count) roll(count, real);
      count = real;
      onvoted?.(real, next);
      toast(next === 0 ? 'Vote taken back. Signed, too.' : signedAndSent(next > 0 ? 'Upvote' : 'Downvote', community));
    } catch (err) {
      mine = before.mine;
      count = before.count;
      shown = before.count;
      toastError(err);
    } finally {
      busy = false;
    }
  }
</script>

<div
  bind:this={box}
  class={[horizontal ? 'hvote' : 'vote', mine === 1 && 'up', mine === -1 && 'down', pinned && mine === 0 && 'pinned']}
  role="group"
  aria-label="Votes"
>
  {#if pinned && !horizontal}<span class="pin" aria-label="Pinned by the mods">PIN</span>{/if}
  <button type="button" aria-label="Upvote" aria-pressed={mine === 1} disabled={disabled} onclick={(e) => vote(1, e)}>▲</button>
  <b aria-live="polite"><span class="sr-only">Score </span>{shown}</b>
  <button type="button" aria-label="Downvote" aria-pressed={mine === -1} disabled={disabled} onclick={(e) => vote(-1, e)}>▼</button>
</div>
