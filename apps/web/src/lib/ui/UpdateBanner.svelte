<script lang="ts">
  import { applyUpdate, sw } from '../sw.svelte';
  import { popIn } from '../motion';
</script>

{#if sw.waiting}
  <div class="wrap">
    <div class="update" role="status" use:popIn={0}>
      <span>A new version is ready</span>
      <button type="button" class="pill pill-sm pill-sun" onclick={applyUpdate}>Reload</button>
    </div>
  </div>
{/if}

<style>
  .wrap {
    position: fixed; left: 0; right: 0; top: calc(78px + env(safe-area-inset-top, 0px)); z-index: 20; display: flex; justify-content: center;
    pointer-events: none; padding: 0 16px;
  }
  .update {
    pointer-events: auto; display: inline-flex; align-items: center; gap: 12px; background: var(--ink); color: var(--onink); border-radius: 999px;
    padding: 6px 6px 6px 18px; font-weight: 700; font-size: 14px; max-width: 100%;
  }
</style>
