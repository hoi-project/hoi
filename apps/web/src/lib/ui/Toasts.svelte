<script lang="ts">
  import { toasts } from '../toast.svelte';
  import { rise } from '../motion';
  let el: HTMLElement | undefined = $state();
  $effect(() => {
    const t = toasts.current;
    if (t) rise(el ?? null);
  });
</script>

<div class="toasts" role="status" aria-live="polite">
  {#if toasts.current}
    {#key toasts.current.id}
      <button type="button" bind:this={el} class={['toast', toasts.current.kind === 'error' && 'err']} onclick={() => toasts.dismiss()}>
        {#if toasts.current.kind === 'error'}<span class="bang" aria-hidden="true">!</span><span class="sr-only">Problem: </span>{/if}{toasts.current.text}
      </button>
    {/key}
  {/if}
</div>

<style>
  .toasts {
    position: fixed; left: 0; right: 0; bottom: calc(22px + env(safe-area-inset-bottom, 0px)); z-index: 30; display: flex; justify-content: center;
    pointer-events: none; padding: 0 16px;
  }
  .toast {
    pointer-events: auto; background: var(--ink); color: var(--onink); border-radius: 999px; padding: 10px 18px; font-weight: 700; font-size: 14px;
    max-width: min(640px, 100%); text-align: center; display: inline-flex; gap: 10px; align-items: center; box-shadow: 3px 3px 0 color-mix(in srgb, var(--ink) 25%, transparent);
    line-height: 1.35;
  }
  .bang { width: 22px; height: 22px; border-radius: 50%; background: var(--peach); color: var(--tagink); display: inline-flex; align-items: center; justify-content: center; font-weight: 900; flex: 0 0 22px; }
  @media (max-width: 640px) {
    .toasts { bottom: calc(144px + env(safe-area-inset-bottom, 0px)); }
    .toast { border-radius: 20px; }
  }
</style>
