<script lang="ts">
  import { app } from '../lib/app.svelte';
  import Logo from '../lib/ui/Logo.svelte';
  import VaultRecovery from '../lib/ui/VaultRecovery.svelte';
  let { kind }: { kind: 'booting' | 'blocked' | 'error' } = $props();
  let busy = $state(false);
  async function here() {
    busy = true;
    await app.takeOver();
    busy = false;
  }
</script>

<main class="boot gutter" id="main">
  <div class="mark"><Logo size={56} /></div>
  {#if kind === 'booting'}
    <p class="msg" role="status">Waking up your {app.platformName}…</p>
  {:else if kind === 'blocked'}
    <h1>hoi is open in another tab.</h1>
    <p class="msg">Only one tab can hold your key and records at a time, so nothing gets signed twice. Close the other one, or move hoi here.</p>
    <button type="button" class="pill pill-ink pill-lg" onclick={here} disabled={busy}>Use it here</button>
  {:else}
    <h1>hoi couldn't start.</h1>
    <p class="msg">{app.bootError}</p>
    <p class="msg small">Try reloading and check that this device has free storage. Recovery options are below if saved data cannot be opened.</p>
    <button type="button" class="pill pill-ink" onclick={() => location.reload()}>Reload</button>
    <VaultRecovery />
  {/if}
</main>

<style>
  .boot { min-height: 100dvh; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 18px; text-align: center; padding-block: 40px; }
  .mark { color: var(--ink); }
  h1 { font-size: var(--type-h1-size); max-width: 18em; }
  .msg { color: var(--mute); font-weight: 500; max-width: 34em; line-height: 1.5; }
  .small { font-size: 14px; }
</style>
