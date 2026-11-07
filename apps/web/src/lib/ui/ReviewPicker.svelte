<script lang="ts">
  import type { Hex } from '@hoi/core-api';
  import { app, attempt } from '../app.svelte';
  import { ruleOptions } from '../util';

  // Hide a post or comment for the community (a signed `review` record). Reasons come from a
  // closed list: disagreeing isn't one (PROTOCOL.md §16). Enough established members hiding,
  // outnumbering those who say it's fine two to one, put it to sleep for everyone.
  let { target, rules, ondone }: { target: Hex; rules?: string | null; ondone: () => void } = $props();
  let reason = $state('spam');
  let busy = $state(false);
  const options = $derived(ruleOptions(rules));
  const name = `hide-${Math.random().toString(36).slice(2, 8)}`;

  async function send(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    const ok = await attempt(() => app.core.review(target, 'hide', reason), 'Hidden for you, and signed. It naps for everyone once enough established members agree.');
    busy = false;
    if (ok !== undefined) ondone();
  }
</script>

<form class="inset hide" onsubmit={send}>
  <fieldset>
    <legend class="label">Why should it nap for everyone?</legend>
    <label class="opt"><input type="radio" {name} value="spam" bind:group={reason} /> Spam</label>
    <label class="opt"><input type="radio" {name} value="abuse" bind:group={reason} /> Abuse or harassment</label>
    <label class="opt"><input type="radio" {name} value="off-topic" bind:group={reason} /> Off-topic here</label>
    {#each options as r (r.n)}
      <label class="opt"><input type="radio" {name} value={`rule:${r.n}`} bind:group={reason} /> Breaks rule {r.n}: {r.label}</label>
    {/each}
    <label class="opt"><input type="radio" {name} value="illegal" bind:group={reason} /> Illegal</label>
  </fieldset>
  {#if reason === 'illegal'}
    <p class="callout">Your device drops any media it carries right away, and blocks the picture from coming back.</p>
  {/if}
  <p class="help">Disagreeing isn't a reason: that's what downvotes are for. Your hide is signed and public, and hides the community defends count against you. Hidden posts nap; anyone can still peek.</p>
  <div class="btnrow">
    <button type="submit" class="pill pill-ink pill-sm" disabled={busy}>Sign &amp; hide</button>
    <button type="button" class="pill pill-sm" onclick={ondone}>Cancel</button>
  </div>
</form>

<style>
  .hide { margin-top: 10px; max-width: 640px; }
  fieldset { border: 0; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 4px; }
  legend { padding: 0 0 6px; }
  .opt { display: flex; gap: 10px; align-items: center; min-height: 36px; font-weight: 500; font-size: 15px; }
  .opt input { width: 18px; height: 18px; accent-color: var(--ink); flex: 0 0 auto; }
</style>
