<script lang="ts">
  import type { Snippet } from 'svelte';
  import { settle } from '../motion';
  // A real <dialog>: focus is trapped, Esc closes, the page behind is inert.
  // `required`: no Esc, no backdrop, no ✕: the person answers with the buttons inside.
  let {
    open = $bindable(false),
    title,
    children,
    onclose,
    wide = false,
    required = false,
  }: { open?: boolean; title: string; children: Snippet; onclose?: () => void; wide?: boolean; required?: boolean } = $props();
  let dlg: HTMLDialogElement | undefined = $state();
  const id = `m-${Math.random().toString(36).slice(2, 8)}`;

  $effect(() => {
    if (!dlg) return;
    if (open && !dlg.open) {
      dlg.showModal();
      settle(dlg);
    } else if (!open && dlg.open) dlg.close();
  });
</script>

<dialog
  bind:this={dlg}
  class={['modal', wide && 'wide']}
  aria-labelledby={id}
  oncancel={(e) => {
    if (required) e.preventDefault();
  }}
  onclose={() => {
    if (required && open) {
      dlg?.showModal();
      return;
    }
    open = false;
    onclose?.();
  }}
  onclick={(e) => {
    if (e.target === dlg && !required) open = false;
  }}
>
  <div class="mhead">
    <h2 {id}>{title}</h2>
    {#if !required}<button type="button" class="x" aria-label="Close" onclick={() => (open = false)}>✕</button>{/if}
  </div>
  {#if open}
    <div class="mbody">{@render children()}</div>
  {/if}
</dialog>

<style>
  .modal {
    width: min(560px, calc(100vw - 32px));
    max-height: min(88dvh, 900px);
    overflow: auto;
    padding: 20px 22px 24px;
    border: 2px solid var(--ink);
    border-radius: var(--radius-frame);
    background: var(--card);
    color: var(--ink);
  }
  .modal.wide { width: min(720px, calc(100vw - 32px)); }
  .modal::backdrop { background: color-mix(in srgb, var(--bg) 30%, rgb(0 0 0 / 55%)); }
  .mhead { display: flex; align-items: center; gap: 12px; padding-bottom: 14px; border-bottom: 2px solid var(--ink); margin-bottom: 16px; }
  .mhead h2 { font-size: 24px; letter-spacing: -0.03em; flex: 1; min-width: 0; }
  .x { width: 40px; height: 40px; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center; font-weight: 800; }
  .x:hover { background: var(--soft); }
  .mbody { display: flex; flex-direction: column; gap: 16px; }
  @media (max-width: 640px) {
    .modal { padding: 16px; }
    .x { width: 44px; height: 44px; }
  }
</style>
