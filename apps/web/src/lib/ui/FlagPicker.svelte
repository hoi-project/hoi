<script lang="ts">
  import type { Hex } from '@hoi/core-api';
  import { app, attempt } from '../app.svelte';
  import { ruleOptions } from '../util';

  // A signed flag for the mods (a `report` record): spam, a numbered rule, or illegal.
  let { target, rules, ondone }: { target: Hex; rules?: string | null; ondone: () => void } = $props();
  let reason = $state('spam');
  let busy = $state(false);
  const options = $derived(ruleOptions(rules));
  const name = `flag-${Math.random().toString(36).slice(2, 8)}`;

  async function send(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    const ok = await attempt(() => app.core.flag(target, reason), 'Flagged. The mods will see your signed flag.');
    busy = false;
    if (ok !== undefined) ondone();
  }
</script>

<form class="inset flag" onsubmit={send}>
  <fieldset>
    <legend class="label">Why should the mods look at this?</legend>
    <label class="opt"><input type="radio" {name} value="spam" bind:group={reason} /> Spam</label>
    {#each options as r (r.n)}
      <label class="opt"><input type="radio" {name} value={`rule:${r.n}`} bind:group={reason} /> Breaks rule {r.n}: {r.label}</label>
    {/each}
    <label class="opt"><input type="radio" {name} value="illegal" bind:group={reason} /> Illegal</label>
  </fieldset>
  <p class="help">Your flag is signed with your key and shared as a community record. Other members, and public peers for public communities, can read it; it is not a confidential message to moderators. Flags from people you trust weigh more.</p>
  <div class="btnrow">
    <button type="submit" class="pill pill-ink pill-sm" disabled={busy}>Sign &amp; flag</button>
    <button type="button" class="pill pill-sm" onclick={ondone}>Cancel</button>
  </div>
</form>

<style>
  .flag { margin-top: 10px; max-width: 640px; }
  fieldset { border: 0; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 4px; }
  legend { padding: 0 0 6px; }
  .opt { display: flex; gap: 10px; align-items: center; min-height: 36px; font-weight: 500; font-size: 15px; }
  .opt input { width: 18px; height: 18px; accent-color: var(--ink); flex: 0 0 auto; }
</style>
