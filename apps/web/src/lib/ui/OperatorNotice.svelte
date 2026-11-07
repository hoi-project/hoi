<script lang="ts">
  // A public node this device met asks the person to read its operator's notice before it
  // serves them. The core ended that link and dials the node again once accepted;
  // declined nodes are left alone. One at a time, and nothing else is clickable until answered.
  import { app } from '../app.svelte';
  import { toastError } from '../toast.svelte';
  import Modal from './Modal.svelte';
  import NoticeText from './NoticeText.svelte';

  const pending = $derived(app.net?.pendingNotices ?? []);
  const current = $derived(pending[0] ?? null);
  let busy = $state(false);

  async function answer(accept: boolean) {
    if (!current || busy) return;
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
    <NoticeText title={current.title} sections={current.sections} footer={current.footer} />
    {#if current.host}<p class="help">Reached at <span class="mono">{current.host}</span>.</p>{/if}
    <div class="btnrow">
      <button type="button" class="pill" onclick={() => answer(false)} disabled={busy}>Decline</button>
      <button type="button" class="pill pill-ink" onclick={() => answer(true)} disabled={busy}>Accept and join</button>
    </div>
    <p class="help">Declined nodes are not contacted again. Both answers can be changed under You → Privacy.{#if pending.length > 1}{' '}{pending.length - 1} more node{pending.length > 2 ? 's' : ''} waiting after this one.{/if}</p>
  </Modal>
{/if}
