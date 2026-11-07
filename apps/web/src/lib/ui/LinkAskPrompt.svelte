<script lang="ts">
  // A new device of yours asks, through your home node, to sign as you. Shown on whichever
  // screen you're on, on devices that hold your root key. Four digits to compare, one answer.
  import { app } from '../app.svelte';
  import { toast, toastError } from '../toast.svelte';
  import Modal from './Modal.svelte';

  const ask = $derived(app.linkAsks[0] ?? null);
  let busy = $state(false);

  async function answer(yes: boolean) {
    if (!ask) return;
    const a = ask;
    busy = true;
    try {
      await app.core.linkAnswer(a.id, yes);
      toast(yes ? `Blessed “${a.label}” with your key. It signs as you now.` : `Said no. “${a.label}” stays unlinked.`);
      if (yes) await app.refreshMe();
    } catch (err) {
      toastError(err);
    }
    app.dropAsk(a.id);
    busy = false;
  }
</script>

<Modal open={!!ask} title="Link a new device?" onclose={() => ask && !busy && app.dropAsk(ask.id)}>
  {#if ask}
    <div class="ask">
      <p><b>“{ask.label}”</b> asks to sign as you. It shows four digits right now:</p>
      <p class="digits num" aria-label={`Check digits ${ask.check.split('').join(' ')}`}>{ask.check.slice(0, 2)}<span> </span>{ask.check.slice(2)}</p>
      <p class="help">Say yes only if they match and it's your device. It gets its own key, blessed by yours; you can revoke it on this page any time.</p>
      <div class="btnrow">
        <button type="button" class="pill pill-ink" onclick={() => answer(true)} disabled={busy}>{busy ? 'Blessing…' : 'Yes, link it'}</button>
        <button type="button" class="pill" onclick={() => answer(false)} disabled={busy}>No</button>
      </div>
    </div>
  {/if}
</Modal>

<style>
  .ask { display: flex; flex-direction: column; gap: 14px; }
  .digits {
    font-size: 44px; font-weight: 900; letter-spacing: 0.08em; background: var(--sun); color: var(--tagink); border: 2px solid var(--ink);
    border-radius: 18px; padding: 8px 18px; align-self: flex-start;
  }
  .digits span { display: inline-block; width: 0.4em; }
</style>
