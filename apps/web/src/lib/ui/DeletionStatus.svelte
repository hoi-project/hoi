<script lang="ts">
  // Your own deletion (RFC-0001), shown where the deleted post was. The core reports only the
  // person's own deletions, so an entry for this target means it is theirs.
  import { app } from '../app.svelte';
  import { deletionLines } from '../deletion-status';
  import { resource } from '../resource.svelte';
  import { to } from '../router';

  let { target }: { target: string } = $props();

  const progress = resource(() => {
    void app.rev;
    return app.core.deletionProgress();
  });
  const mine = $derived(progress.value?.find((p) => p.target === target));
</script>

{#if mine}
  <section class="ds" aria-labelledby="ds-h">
    <h2 id="ds-h">Your deletion</h2>
    <ul>{#each deletionLines(mine, app.platformName) as line (line)}<li>{line}</li>{/each}</ul>
    <p class="help">All your deletions are under <a class="textlink" href={to.you()}>You</a> → On this {app.platformName}.</p>
  </section>
{/if}

<style>
  .ds { border: 1px solid var(--line); border-radius: var(--radius-input); padding: 14px; display: grid; gap: 10px; max-width: 640px; }
  h2 { font-size: 16px; font-weight: 800; letter-spacing: normal; }
  ul { margin: 0; padding-left: 20px; display: grid; gap: 6px; font-size: 14px; line-height: 1.5; }
</style>
