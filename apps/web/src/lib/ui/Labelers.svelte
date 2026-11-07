<script lang="ts">
  // Labelers whose labels count on this device: the ones you follow, and the ones
  // your communities name by default. Follow someone from their page, or by key here.
  import { app, attempt } from '../app.svelte';
  import { resource } from '../resource.svelte';
  import { to } from '../router';

  const list = resource(() => {
    void app.rev;
    return app.core.labelers();
  });
  const counted = resource(() => {
    void app.rev;
    return app.core.aggregators();
  });
  let key = $state('');

  async function follow(e: SubmitEvent) {
    e.preventDefault();
    const k = key.trim();
    if (!k) return;
    const ok = await attempt(() => app.core.followLabeler(k, true), 'Following their labels: what they mark as spam or nsfw naps for you.');
    if (ok !== undefined) key = '';
  }
</script>

<div class="v labelers">
  {#if list.value?.length}
    <ul class="people" role="list">
      {#each list.value as l (l.person.key)}
        <li class="person">
          <a class="lnk lnk-ink" href={to.person(l.person.key)}>{l.person.name}</a>
          <span class="mono short">{l.person.short}</span>
          {#if l.defaultIn.length}<span class="tag tag-soft">default in {l.defaultIn.map((c) => `c/${c.name}`).join(', ')}</span>{/if}
          <span class="tag tag-soft">{l.labels} label{l.labels === 1 ? '' : 's'} here</span>
          {#if l.followed}
            <button type="button" class="lnk push" onclick={() => attempt(() => app.core.followLabeler(l.person.key, false), `${l.person.name}'s labels no longer count here.`)}>Stop following</button>
          {:else}
            <button type="button" class="lnk lnk-ink push" onclick={() => attempt(() => app.core.followLabeler(l.person.key, true), `Following ${l.person.name}'s labels everywhere.`)}>Follow everywhere</button>
          {/if}
        </li>
      {/each}
    </ul>
  {:else}
    <span class="tag tag-soft">none yet</span>
  {/if}
  {#if counted.value?.length}
    <p class="help">Aggregators whose counts show next to posts, and under Counted:</p>
    <ul class="people" role="list">
      {#each counted.value as a (a.person.key)}
        <li class="person">
          <a class="lnk lnk-ink" href={to.person(a.person.key)}>{a.person.name}</a>
          <span class="tag tag-soft">counts {a.communities} of your communities</span>
          <button type="button" class="lnk push" onclick={() => attempt(() => app.core.followAggregator(a.person.key, false), `${a.person.name}'s counts no longer show here.`)}>Stop following</button>
        </li>
      {/each}
    </ul>
  {/if}
  <form class="keyrow" onsubmit={follow}>
    <label class="sr-only" for="lb-key">A labeler's key</label>
    <input id="lb-key" class="input mono-in" bind:value={key} placeholder="their key (64 hex)" spellcheck="false" autocapitalize="none" />
    <button type="submit" class="pill pill-sm" disabled={!key.trim()}>Follow</button>
  </form>
</div>

<style>
  .labelers { display: flex; flex-direction: column; align-items: flex-start; gap: 10px; }
  .people { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; align-self: stretch; }
  .person { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; min-width: 0; }
  .short { color: var(--mute); font-size: 13px; }
  .push { margin-left: auto; }
  .keyrow { display: flex; gap: 8px; align-items: center; align-self: stretch; max-width: 480px; }
  .keyrow .input { flex: 1; min-width: 0; }
</style>
