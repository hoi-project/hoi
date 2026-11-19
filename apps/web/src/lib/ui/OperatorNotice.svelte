<script lang="ts">
  // A public node this device met asks the person to read its operator's notice before it
  // serves them. The core ended that link and dials the node again once accepted;
  // declined nodes are left alone. One at a time, and nothing else is clickable until answered.
  import type { NoticeView } from '@hoi/core-api';
  import { tick, untrack } from 'svelte';
  import { app } from '../app.svelte';
  import { toastError } from '../toast.svelte';
  import Modal from './Modal.svelte';
  import NoticeText from './NoticeText.svelte';

  const pending = $derived(app.net?.pendingNotices ?? []);
  let current = $state.raw<NoticeView | null>(null);
  const latest = $derived(pending.find(notice => notice.node === current?.node));
  const changed = $derived(!!current && !!latest && current.hash !== latest.hash);
  let content = $state<HTMLDivElement>();
  let busy = $state(false);

  // A second connection can reorder the queue while a person is reading. Keep the
  // displayed revision until they answer, open its replacement or the core withdraws it.
  $effect(() => {
    const notices = pending;
    const shown = untrack(() => current);
    if (shown && notices.some(notice => notice.node === shown.node)) return;
    const next = notices[0] ?? null;
    current = next;
    if (shown && next) void focusHeading();
  });

  async function focusHeading() {
    await tick();
    const heading = content?.querySelector('h3');
    if (heading) {
      heading.tabIndex = -1;
      heading.focus();
    }
  }

  function reviewUpdate() {
    if (!latest || !changed || busy) return;
    current = latest;
    void focusHeading();
  }

  async function answer(accept: boolean) {
    if (!current || !latest || changed || busy) return;
    busy = true;
    try {
      if (accept) await app.core.acceptNotice(current.node, current.hash);
      else await app.core.declineNotice(current.node);
      app.net = await app.core.net();
    } catch (e) {
      toastError(e);
    } finally {
      busy = false;
    }
  }
</script>

{#if current}
  <Modal open={true} required title="Before this node serves you" wide>
    <p class="help">A public node your device reached asks you to read its operator's notice first. Nothing of yours was sent to it; like any connection, it saw this {app.platformName}'s address.</p>
    <div bind:this={content}><NoticeText title={current.title} sections={current.sections} footer={current.footer} /></div>
    {#if current.host}<p class="help">Reached at <span class="mono">{current.host}</span>.</p>{/if}
    {#if changed}
      <p role="alert">This node changed its notice or configuration while you were reading. Review the update before choosing whether to connect.</p>
    {/if}
    <div class="btnrow">
      <button type="button" class="pill" onclick={() => answer(false)} aria-disabled={busy || changed}>Decline</button>
      <button type="button" class="pill pill-ink" onclick={() => answer(true)} aria-disabled={busy || changed}>Accept and join</button>
      {#if changed}<button type="button" class="pill" onclick={reviewUpdate} disabled={busy}>Review updated notice</button>{/if}
    </div>
    <p class="help">Declined nodes are not contacted again. Both answers can be changed under You → Privacy.{#if pending.length > 1}{' '}{pending.length - 1} more node{pending.length > 2 ? 's' : ''} waiting after this one.{/if}</p>
  </Modal>
{/if}

<style>
  .pill[aria-disabled="true"] { cursor: not-allowed; opacity: .55; transform: none; }
</style>
