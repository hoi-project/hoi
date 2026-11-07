<script lang="ts">
  import { onMount } from 'svelte';
  import { disableOsUnlock, enableOsUnlock, osUnlockStatus, type OsUnlockStatus } from '../core';
  import { isDesktopShell } from '../host';
  import { credentialValue, vaultLoginLabel } from '../password-manager';
  import { errorText } from '../toast.svelte';

  const shown = isDesktopShell();
  let status = $state<OsUnlockStatus | null>(null);
  let busy = $state(false);
  let password = $state('');
  let visible = $state(false);
  let message = $state('');
  let error = $state('');

  async function refresh() {
    try { status = await osUnlockStatus(); }
    catch (e) { error = errorText(e); }
  }
  onMount(() => { if (shown) void refresh(); });

  async function enable(e: SubmitEvent) {
    e.preventDefault();
    if (busy) return;
    password = credentialValue(e.currentTarget as HTMLFormElement);
    busy = true;
    error = message = '';
    try {
      await enableOsUnlock(password);
      message = 'OS unlock is ready. Your vault password still works. Keep your identity backup too.';
    } catch (e) { error = errorText(e); }
    finally { password = ''; await refresh(); busy = false; }
  }

  async function disable() {
    if (busy) return;
    busy = true;
    error = message = '';
    try {
      await disableOsUnlock();
      message = 'OS unlock was removed. Use your vault password next time.';
    } catch (e) { error = errorText(e); }
    finally { await refresh(); busy = false; }
  }
</script>

{#if shown}
  <section class="os-unlock" aria-labelledby="os-unlock-heading" aria-busy={busy}>
    <h3 id="os-unlock-heading">Unlock with this OS account</h3>
    <p class="help">Keep an unlock key in macOS Keychain, Windows Credential Manager or Linux Secret Service on this computer. Protection depends on your OS account and keystore; it may not ask for a fingerprint or PIN each time. This does not back up your identity or sync the vault.</p>
    {#if error}<p class="help" role="alert">{error}</p>{/if}
    {#if message}<p class="help" role="status">{message}</p>{/if}
    {#if status?.enabled}
      <p class="help">Enabled. After locking, choose OS unlock or your vault password.</p>
    {:else if status}
      <form id="enable-os-unlock" onsubmit={enable} autocomplete="on">
        <label class="field" for="os-login"><span>Password-manager entry</span><input class="input" id="os-login" name="username" autocomplete="username" value={vaultLoginLabel(true, false)} disabled={busy} spellcheck="false" autocapitalize="none" /></label>
        <label class="field" for="os-password"><span>Current vault password</span><input class="input" id="os-password" name="password" type={visible ? 'text' : 'password'} autocomplete="current-password" bind:value={password} disabled={busy} required /></label>
        <label class="show"><input type="checkbox" bind:checked={visible} disabled={busy} /> Show vault password</label>
        <p class="help">Confirm your vault password to enable this option. Your OS may ask to open its keystore. If unavailable, keep using your password.</p>
        <button class="pill" type="submit" disabled={busy}>{busy ? 'Checking OS unlock…' : 'Enable OS unlock'}</button>
      </form>
    {:else if !error}<p class="help" role="status">Checking OS unlock settings…</p>{/if}
    {#if status?.configured}<button class="pill" type="button" onclick={disable} disabled={busy}>Remove OS unlock</button>{/if}
    {#if !status && error}<button class="pill" type="button" onclick={refresh} disabled={busy}>Retry</button>{/if}
  </section>
{/if}

<style>
  .os-unlock, form { display: flex; flex-direction: column; gap: 12px; }
  .os-unlock { padding-block: 16px; border-top: 1px solid var(--line); }
  h3 { font-size: 17px; }
  .help { font-size: 14px; line-height: 1.5; color: var(--mute); }
  button { align-self: start; }
  .show { display: flex; align-items: center; gap: 8px; min-height: 44px; font-size: 14px; }
</style>
