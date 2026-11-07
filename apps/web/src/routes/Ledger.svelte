<script lang="ts">
  import { app, setTitle } from '../lib/app.svelte';
  import { resource } from '../lib/resource.svelte';
  import { enterView, popIn } from '../lib/motion';
  import { to } from '../lib/router';
  import { saveFile, slug } from '../lib/util';
  import { toast, toastError } from '../lib/toast.svelte';
  import LedgerTable from '../lib/ui/LedgerTable.svelte';

  let { id }: { id: string } = $props();
  const view = resource(() => {
    void app.rev;
    return app.core.community(id);
  });
  const entries = resource(() => {
    void app.rev;
    return app.core.ledger(id);
  });
  const c = $derived(view.value);
  $effect(() => setTitle(c ? `c/${c.name} ledger` : 'Ledger'));

  async function exportLog() {
    if (!c) return;
    try {
      const data = await app.core.exportBundle(c.id);
      await saveFile(`c-${slug(c.name)}-ledger-${new Date().toISOString().slice(0, 10)}.hoibundle`, data);
      toast(`Signed log exported (${Math.max(1, Math.round(data.length / 1024))} KB). Anyone with the community key can check it.`);
    } catch (e) {
      toastError(e);
    }
  }
</script>

{#if (view.error || entries.error) && !c}
  <div class="empty"><h2>No ledger for that here.</h2><p>{view.error || entries.error}</p><a class="pill" href="#/">Home</a></div>
{:else if !c || !entries.value}
  <p class="loading" role="status">Reading the ledger…</p>
{:else}
  <section class="view" use:enterView aria-labelledby="l-h">
    <a class="act back anim" href={to.community(c.id)}>← c/{c.name}</a>
    <div class="vh anim">
      <h1 id="l-h">The ledger</h1>
      <span class="sub">c/{c.name} · {entries.value.length.toLocaleString('en')} {entries.value.length === 1 ? 'action' : 'actions'} · append-only · anyone can check it</span>
      <button type="button" class="pill exp" onclick={exportLog}>Export signed log</button>
    </div>
    <div class="lgrid">
      <div class="anim tblwrap"><LedgerTable entries={entries.value} rules={c.rules} owners={c.owners.length} canVeto={!!c.me?.capabilities.includes('veto')} /></div>
      <div class="frame note" use:popIn={-1.5}>
        <b>Yours, not ours.</b>
        <p>Everything here is signed by mod keys and copied to every member. Nobody outside the community, including the people who make the app, can touch it.</p>
        <p><b>Nothing gets deleted by hand.</b> Mods label and hide; time removes.</p>
      </div>
    </div>
  </section>
{/if}

<style>
  .back { margin-left: -10px; margin-bottom: 8px; }
  .sub { font-size: 14px; color: var(--mute); font-weight: 500; }
  .exp { margin-left: auto; }
  .lgrid { display: grid; grid-template-columns: minmax(0, 1fr) 240px; gap: 28px; align-items: start; }
  .tblwrap { min-width: 0; }
  /* The table needs 640px beside the 240px note; with the index beside the view that is 1200px. */
  @media (max-width: 1200px) {
    .lgrid { grid-template-columns: 1fr; }
    .note { max-width: 420px; }
  }
  @media (max-width: 640px) {
    .exp { margin-left: 0; }
  }
</style>
