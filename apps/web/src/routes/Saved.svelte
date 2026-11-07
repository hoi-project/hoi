<script lang="ts">
  import { app, setTitle } from '../lib/app.svelte';
  import { resource } from '../lib/resource.svelte';
  import { enterView } from '../lib/motion';
  import PostRow from '../lib/ui/PostRow.svelte';

  setTitle('Saved');
  const list = resource(() => {
    void app.rev;
    return app.core.saved();
  });
</script>

<section class="view" use:enterView aria-labelledby="s-h">
  <div class="vh anim"><h1 id="s-h">Saved</h1><span class="hint">kept on this {app.platformName} only · never passed along</span></div>
  {#if list.value && list.value.length === 0}
    <div class="empty anim">
      <h2>Nothing saved yet.</h2>
      <p>Hit Save on a post to keep it here. Saved posts stay on this {app.platformName}, and your copy is never shared with anyone.</p>
      <a class="pill" href="#/">Back to the feed</a>
    </div>
  {:else if list.value}
    <div class="rows">{#each list.value as p (p.id)}<PostRow post={p} />{/each}</div>
  {:else}
    <p class="loading" role="status">{list.error || 'Looking…'}</p>
  {/if}
</section>
