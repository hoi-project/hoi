<script lang="ts">
  // "Delete known posts and comments" (RFC-0002) and clearing afterwards (RFC-0003). The core
  // keeps and resumes every list: this screen only reads its status and starts what the person
  // confirms. Reopening it never starts a second list, and closing it cancels nothing.
  import type { BulkDeletionPreview, BulkDeletionStatus, CleanupReadiness, DeletionProgress } from '@hoi/core-api';
  import { now } from '@hoi/core-api';
  import { onMount } from 'svelte';
  import { app } from '../app.svelte';
  import { router } from '../router.svelte';
  import { errorText, toast } from '../toast.svelte';
  import { batchLines, gapLines, readinessWords, summarize } from '../deletion-status';
  import { plural } from '../util';

  let { ondone }: { ondone: () => void } = $props();

  let batches = $state<BulkDeletionStatus[]>([]);
  let progress = $state<DeletionProgress[]>([]);
  /** `null` when this core cannot say: the combined clear is then never offered. */
  let readiness = $state<CleanupReadiness | null>(null);
  let loaded = $state(false);
  let preview = $state<BulkDeletionPreview | null>(null);
  let step = $state<'status' | 'review' | 'confirm' | 'clear'>('status');
  let busy = $state(false);
  let error = $state('');

  const device = $derived(app.platformName);
  const started = $derived(batches.length > 0 || progress.length > 0);
  const words = $derived(readinessWords(readiness, now(), device));
  const day = (t: number) => new Date(t * 1000).toLocaleDateString('en', { day: 'numeric', month: 'short', year: 'numeric' });

  async function load() {
    const [b, p, r] = await Promise.all([
      app.core.bulkDeletions(),
      app.core.deletionProgress(),
      app.core.cleanupReadiness().catch(() => null),
    ]);
    batches = b;
    progress = p;
    readiness = r;
    loaded = true;
  }

  onMount(() => {
    load().catch((e) => (error = errorText(e)));
    const timer = setInterval(() => void load().catch(() => undefined), 15_000);
    return () => clearInterval(timer);
  });

  async function review() {
    busy = true;
    error = '';
    try {
      preview = await app.core.previewBulkDeletion();
      step = 'review';
    } catch (e) { error = errorText(e); }
    busy = false;
  }

  async function confirm() {
    if (!preview) return;
    busy = true;
    error = '';
    try {
      await app.core.confirmBulkDeletion(preview.snapshot);
      preview = null;
      step = 'status';
      toast(`Saved on this ${device}. Deleting continues after a restart.`);
    } catch (e) {
      // Not saved, so nothing was signed. The same list can be confirmed again.
      error = errorText(e);
    }
    busy = false;
    await load().catch(() => undefined);
  }

  async function stop(snapshot: string) {
    busy = true;
    error = '';
    try { await app.core.cancelBulkDeletion(snapshot); }
    catch (e) { error = errorText(e); }
    busy = false;
    await load().catch(() => undefined);
  }

  async function clear() {
    if (!readiness?.ready) return;
    busy = true;
    error = '';
    try {
      // The core checks again, under the same lock as its deletion work, before any key goes.
      await app.core.clearAfterDeletion(readiness.inventory, readiness.deletions);
      await app.wiped();
      busy = false;
      ondone();
      router.go('#/');
      toast(`Cleared. Nothing of yours is left on this ${device}.`);
      return;
    } catch (e) {
      error = errorText(e);
      step = 'status';
    }
    busy = false;
    await load().catch(() => undefined);
  }

  function back() {
    step = 'status';
    preview = null;
    error = '';
  }
</script>

<div class="dk" aria-busy={busy}>
  {#if step === 'status'}
    <p>Asks the network to delete the posts and comments of yours that this {device} knows, each with its own signed deletion. Your key stays here so the work can finish, also after a restart. Closing this stops nothing.</p>
    {#if !loaded && !error}<p role="status">Checking this {device}…</p>{/if}
    {#each batches as b (b.snapshot)}
      {@const s = summarize(b, progress)}
      <section class="batch" aria-label={`List of ${b.count} from ${day(b.confirmedAt)}`}>
        <h3>{plural(b.count, 'post or comment', 'posts and comments')}, listed {day(b.confirmedAt)}</h3>
        <ul>
          {#each batchLines(s) as line (line)}<li>{line}</li>{/each}
          {#each gapLines(b.gaps) as line (line)}<li class="gap">{line}</li>{/each}
        </ul>
        {#if s.saved && !b.cancelledAt}
          <button type="button" class="pill pill-sm" onclick={() => stop(b.snapshot)} disabled={busy}>Stop the {plural(s.saved, 'unsigned one')}</button>
        {/if}
      </section>
    {/each}
    {#if started}
      <section class="after" aria-labelledby="dk-after">
        <h3 id="dk-after">Clear this {device} afterwards</h3>
        {#if words.ready}
          <p>No deletion here needs this {device} any more.</p>
        {/if}
        <ul>{#each words.lines as line (line)}<li>{line}</li>{/each}</ul>
        {#if words.ready}
          <button type="button" class="pill pill-danger" onclick={() => (step = 'clear')} disabled={busy}>Clear local data…</button>
        {:else}
          <p class="help">You can still clear local data now with <b>Wipe</b> on the You page. That stops the work above unfinished, and nothing reports it as done.</p>
        {/if}
      </section>
    {/if}
    <div class="btnrow">
      <button type="button" class="pill pill-ink" onclick={review} disabled={busy}>{busy ? 'Looking…' : 'Review known posts and comments'}</button>
      <button type="button" class="pill" onclick={ondone}>Done</button>
    </div>
  {:else if step === 'review' && preview}
    {#if preview.count === 0}
      <p>No known posts or comments of yours are left here to delete.</p>
    {:else}
      <p>This {device} knows <b>{plural(preview.count, 'post or comment', 'posts and comments')}</b> of yours that are still up:</p>
      <ul class="communities">
        {#each preview.communities as c (c.id)}
          <li><b>{c.name || 'A community'}</b>: {plural(c.posts.length, 'post')}, {plural(c.comments.length, 'comment')}</li>
        {/each}
      </ul>
    {/if}
    <h3>Not on this list</h3>
    <ul>
      <li>Posts this {device} never saw, or that only your other devices hold. This list can't prove it's complete.</li>
      {#each gapLines(preview.gaps) as line (line)}<li class="gap">{line}</li>{/each}
    </ul>
    {#if preview.count}
      <p class="help">Each one gets its own signed deletion inside the hourly limit, so a long list takes hours or days. Members and storage holders are asked to remove them; copies someone kept can't be forced away. Replies by other people stay.</p>
    {/if}
    <div class="btnrow">
      {#if preview.count}<button type="button" class="pill pill-danger" onclick={() => (step = 'confirm')}>Delete these {preview.count}</button>{/if}
      <button type="button" class="pill" onclick={back}>Back</button>
    </div>
  {:else if step === 'confirm' && preview}
    <p><b>Delete {plural(preview.count, 'post or comment', 'posts and comments')}?</b> There's no undo. Your key stays on this {device} until the work is done; it isn't burned.</p>
    {#if error}
      <p class="error" role="alert">{error} Nothing was signed. You can try again.</p>
    {/if}
    <div class="btnrow">
      <button type="button" class="pill pill-danger" onclick={confirm} disabled={busy}>{busy ? 'Saving…' : error ? 'Try again' : 'Yes, delete them'}</button>
      {#if error}<button type="button" class="pill" onclick={review} disabled={busy}>Review again</button>{/if}
      <button type="button" class="pill" onclick={back} disabled={busy}>Stop</button>
    </div>
  {:else if step === 'clear'}
    <p><b>Clear this {device} now?</b> This removes your key, records, media, private community keys, settings and anything not sent yet from this {device}. Exported key backups and your other devices keep theirs. Deleting files doesn't guarantee the storage is physically erased.</p>
    <ul>{#each words.lines as line (line)}<li>{line}</li>{/each}</ul>
    <div class="btnrow">
      <button type="button" class="pill pill-danger" onclick={clear} disabled={busy}>{busy ? 'Clearing…' : 'Yes, clear it'}</button>
      <button type="button" class="pill" onclick={back} disabled={busy}>Stop</button>
    </div>
  {/if}
  {#if error && step !== 'confirm'}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
  .dk { display: grid; gap: 14px; }
  .dk p, .dk li { line-height: 1.5; }
  h3 { font-size: 16px; font-weight: 800; }
  ul { margin: 0; padding-left: 20px; display: grid; gap: 6px; font-size: 14px; }
  .gap { color: var(--mute); }
  .batch, .after { border: 1px solid var(--line); border-radius: var(--radius-input); padding: 14px; display: grid; gap: 10px; justify-items: start; }
  .communities li { overflow-wrap: anywhere; }
  .error { border-radius: 10px; background: var(--peach); color: var(--ink); padding: 12px; }
</style>
