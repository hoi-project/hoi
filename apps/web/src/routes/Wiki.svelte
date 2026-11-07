<script lang="ts">
  // A community's wiki: pages its members write together. Each save is a signed
  // edit; edits made on other devices meanwhile merge in, line by line and letter by letter.
  import { ago } from '@hoi/core-api';
  import { app, attempt } from '../lib/app.svelte';
  import { resource } from '../lib/resource.svelte';
  import { router } from '../lib/router.svelte';
  import { to } from '../lib/router';
  import { paragraphs, plural } from '../lib/util';

  let { community, page }: { community: string; page?: string } = $props();
  const pages = resource(() => {
    void app.rev;
    return app.core.wikiPages(community);
  });
  const view = resource(() => {
    void app.rev;
    return page ? app.core.wikiPage(community, page) : Promise.resolve(null);
  });
  let editing = $state(false);
  let draft = $state('');
  let newName = $state('');
  let saving = $state(false);
  /** A page just started from the list: it opens in the editor. */
  let started = $state('');
  const nameOk = $derived(/^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/.test(newName.trim()));

  $effect(() => {
    editing = !!page && page === started;
    if (editing) draft = '';
  });

  function startEdit() {
    draft = view.value?.text ?? '';
    editing = true;
  }

  async function save(e: SubmitEvent) {
    e.preventDefault();
    if (!page) return;
    saving = true;
    const v = await attempt(() => app.core.editWiki(community, page!, draft), 'Saved and signed. Edits made elsewhere meanwhile merge in.');
    saving = false;
    if (v) {
      view.set(v);
      started = '';
      editing = false;
    }
  }

  function create(e: SubmitEvent) {
    e.preventDefault();
    const name = newName.trim();
    if (!nameOk) return;
    newName = '';
    started = name;
    router.go(to.wiki(community, name));
  }
</script>

<div class="wiki anim">
  {#if !page}
    <h2 class="lh">Pages</h2>
    {#if pages.value?.length}
      <ul class="pages" role="list">
        {#each pages.value as p (p.page)}
          <li><a class="lnk lnk-ink" href={to.wiki(community, p.page)}>{p.page}</a> <span class="hint">{plural(p.editors, 'editor')} · {ago(p.updatedAt)}</span></li>
        {/each}
      </ul>
    {:else if pages.value}
      <p class="help">No pages yet. Members write the wiki together: how to get started here, what people keep asking, what was decided.</p>
    {/if}
    <form class="newpage" onsubmit={create}>
      <label class="field" for="wk-new"><span>New page</span>
        <input id="wk-new" class="input" bind:value={newName} placeholder="getting-started" maxlength="64" spellcheck="false" autocapitalize="none" aria-describedby="wk-new-help" />
      </label>
      <p class="help" id="wk-new-help">Lowercase letters, digits and hyphens.</p>
      <button type="submit" class="pill pill-sm" disabled={!nameOk}>Start it</button>
    </form>
  {:else}
    <nav class="crumbs" aria-label="Wiki"><a class="lnk" href={to.wiki(community)}>All pages</a> <span aria-hidden="true">/</span> <b>{page}</b></nav>
    {#if view.value}
      {@const v = view.value}
      {#if editing}
        <form class="editor" onsubmit={save}>
          <label class="field" for="wk-text"><span>{page}</span>
            <textarea id="wk-text" class="textarea" rows="16" bind:value={draft} maxlength="65536"></textarea>
          </label>
          <p class="help">Saving signs an edit. If someone changed the page on another device meanwhile, both changes are kept.</p>
          <div class="btnrow">
            <button type="submit" class="pill pill-ink pill-sm" disabled={saving}>{saving ? 'Saving…' : 'Save'}</button>
            <button type="button" class="pill pill-sm" onclick={() => (editing = false)}>Cancel</button>
          </div>
        </form>
      {:else}
        {#if v.text}
          <article class="page">{#each paragraphs(v.text) as para, i (i)}<p>{para}</p>{/each}</article>
          <p class="hint">{plural(v.edits, 'edit')} by {v.editors.map((p) => p.name).join(', ')} · last {ago(v.updatedAt)}</p>
        {:else}
          <p class="help">Nobody wrote this page yet.</p>
        {/if}
        {#if v.canEdit}
          <div class="btnrow"><button type="button" class="pill pill-sm" onclick={startEdit}>{v.text ? 'Edit' : 'Write it'}</button></div>
        {:else}
          <p class="help">Members edit the wiki: once you're admitted here, you can too.</p>
        {/if}
      {/if}
    {/if}
  {/if}
</div>

<style>
  .wiki { display: flex; flex-direction: column; gap: 14px; max-width: 760px; }
  .pages { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .pages li { display: flex; gap: 10px; align-items: baseline; flex-wrap: wrap; }
  .newpage { display: flex; flex-direction: column; gap: 6px; align-items: flex-start; border-top: 2px solid var(--line); padding-top: 14px; }
  .crumbs { display: flex; gap: 8px; align-items: center; }
  .pages a, .crumbs a { min-height: 24px; display: inline-flex; align-items: center; }
  .page { display: flex; flex-direction: column; gap: 10px; font-size: 16px; line-height: 1.55; }
  .editor { display: flex; flex-direction: column; gap: 10px; }
  .editor .textarea { font-family: inherit; min-height: 320px; }
</style>
