<script lang="ts">
  // Open blocklists: keepers whose signed lists this person follows, and publishing a
  // list of one's own in a community, for whoever follows it.
  import { app, attempt } from '../app.svelte';
  import { resource } from '../resource.svelte';
  import { to } from '../router';
  import { toast } from '../toast.svelte';

  const keepers = resource(() => {
    void app.rev;
    return app.core.blocklistKeepers();
  });
  let community = $state('');
  let text = $state('');

  async function publish(e: SubmitEvent) {
    e.preventDefault();
    const n = await attempt(() => app.core.publishBlocklist(community, text));
    if (typeof n === 'number') {
      text = '';
      toast(`${n} hashes published in that community, for whoever follows your list.`);
    }
  }
</script>

{#if keepers.value?.length}
  <ul class="people" role="list">
    {#each keepers.value as k (k.person.key)}
      <li class="person">
        <a class="lnk lnk-ink" href={to.person(k.person.key)}>{k.person.name}</a>
        <span class="tag tag-soft">{k.exact} hashes · {k.pdq} fingerprints</span>
        <button type="button" class="lnk push" onclick={() => attempt(() => app.core.followBlocklist(k.person.key, false), `${k.person.name}'s list no longer counts here.`)}>Stop following</button>
      </li>
    {/each}
  </ul>
{/if}
<details class="block-import">
  <summary>Publish a list</summary>
  <form class="pub" onsubmit={publish}>
    <p class="help">For people who follow your list from your page: one hash a line, a PDQ fingerprint (64 hex) or <span class="mono">blake3:</span> and an exact media hash. Everyone in the community can read it.</p>
    <label class="field" for="lk-c"><span>Community</span>
      <select id="lk-c" class="input" bind:value={community}>
        <option value="">Choose…</option>
        {#each app.joined as c (c.id)}<option value={c.id}>c/{c.name}</option>{/each}
      </select>
    </label>
    <label class="field" for="lk-t"><span>The list</span><textarea id="lk-t" class="input mono-in" rows="4" bind:value={text} spellcheck="false"></textarea></label>
    <button type="submit" class="pill pill-sm" disabled={!community || !text.trim()}>Publish</button>
  </form>
</details>

<style>
  .people { list-style: none; margin: 0 0 8px; padding: 0; display: flex; flex-direction: column; gap: 6px; align-self: stretch; }
  .person { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; min-width: 0; }
  .push { margin-left: auto; }
  .pub { display: flex; flex-direction: column; gap: 8px; align-items: flex-start; }
  .pub .field { width: 100%; }
</style>
