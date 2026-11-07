<script lang="ts">
  import { app } from '../lib/app.svelte';
  import { errorText } from '../lib/toast.svelte';
  import Logo from '../lib/ui/Logo.svelte';
  import VaultRecovery from '../lib/ui/VaultRecovery.svelte';
  import { isDesktopShell } from '../lib/host';
  import { onMount } from 'svelte';
  import { osUnlockStatus, vaultPasskeys } from '../lib/core';
  import { passkeyAnswer, passkeysPossible, type VaultPasskey } from '../lib/passkey';
  import { credentialValue, rememberVaultLogin, vaultLoginLabel } from '../lib/password-manager';

  const creating = $derived(app.vaultState === 'create' || app.vaultState === 'migrate');
  let passphrase = $state('');
  let confirmation = $state('');
  let busy = $state(false);
  let error = $state('');
  let visible = $state(false);
  let capsLock = $state(false);
  let loginLabel = $state(vaultLoginLabel(isDesktopShell(), app.vaultState === 'create' || app.vaultState === 'migrate'));

  /** Passkeys that open this vault, from its header: nothing is unlocked to read them. */
  let passkeys = $state<VaultPasskey[]>([]);
  let osUnlock = $state(false);
  onMount(async () => {
    if (!creating && isDesktopShell()) osUnlock = (await osUnlockStatus().catch(() => ({ enabled: false }))).enabled;
    if (!creating && passkeysPossible()) passkeys = await vaultPasskeys().catch(() => []);
  });

  async function withOs() {
    if (busy) return;
    busy = true;
    error = '';
    try { await app.unlockOs(); }
    catch (e) { error = errorText(e); }
    finally { busy = false; }
  }

  async function withPasskey() {
    if (busy) return;
    error = '';
    busy = true;
    try {
      const got = await passkeyAnswer(passkeys);
      await app.unlockPasskey(got.credential, got.prf);
    } catch (e) {
      error = (e as Error)?.name === 'NotAllowedError' ? 'The passkey was not used. Try again, or use your vault password.' : errorText(e);
    } finally {
      busy = false;
    }
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    error = '';
    const form = event.currentTarget as HTMLFormElement;
    passphrase = credentialValue(form);
    confirmation = credentialValue(form, 'password-confirmation');
    loginLabel = credentialValue(form, 'username');
    if (creating && [...passphrase].length < 15) { error = 'Use at least 15 characters. Several unrelated words work well.'; return; }
    if (new TextEncoder().encode(passphrase).length > 1024) { error = 'That password is too long.'; return; }
    if (creating && passphrase !== confirmation) { error = 'The two vault passwords do not match.'; return; }
    busy = true;
    try { await app.unlock(passphrase); rememberVaultLogin(loginLabel); }
    catch (e) { error = errorText(e); }
    finally { passphrase = ''; confirmation = ''; busy = false; }
  }
</script>

<main class="unlock gutter" id="main">
  <Logo size={56} />
  <h1>{creating ? 'Protect this device' : 'Unlock this device'}</h1>
  <p>{creating ? 'Choose a vault password to encrypt your keys and saved content here. Your password manager can generate and save it.' : 'Use this device’s saved vault password, or select it in your password manager.'}</p>
  {#if app.vaultState === 'create'}<p class="help">Already use hoi? Protect this device first, then import your key backup or link a signed-in device. This step does not create a new identity.</p>{/if}
  {#if app.vaultState === 'migrate'}<p>Your existing identity and communities will be kept and encrypted.</p>{/if}
  <form id="vault-login" onsubmit={submit} aria-busy={busy} autocomplete="on">
    <label for="vault-username">Password-manager entry</label>
    <input id="vault-username" name="username" type="text" autocomplete="username" bind:value={loginLabel} maxlength="200" disabled={busy} spellcheck="false" autocapitalize="none" />
    <label for="vault-passphrase">{creating ? 'New vault password' : 'Vault password'}</label>
    <input id="vault-passphrase" name="password" type={visible ? 'text' : 'password'} autocomplete={creating ? 'new-password' : 'current-password'} bind:value={passphrase} disabled={busy} required aria-describedby="vault-help" autocapitalize="none" spellcheck="false" onkeyup={(event) => capsLock = event.getModifierState('CapsLock')} />
    {#if creating}
      <label for="vault-confirm">Repeat vault password</label>
      <input id="vault-confirm" name="password-confirmation" type={visible ? 'text' : 'password'} autocomplete="new-password" bind:value={confirmation} disabled={busy} required autocapitalize="none" spellcheck="false" />
    {/if}
    <label class="show"><input type="checkbox" bind:checked={visible} /> Show password</label>
    {#if capsLock}<p class="help" role="status">Caps Lock is on.</p>{/if}
    <p class="help" id="vault-help">{creating ? 'Use at least 15 characters. This entry unlocks only this installation. After signing in, back up your identity with a recovery key or encrypted file too.' : 'Choose the entry labelled “vault”. Your recovery key and backup-file password belong in Restore my identity on a fresh device.'}</p>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <button type="submit" class="pill pill-ink pill-lg" disabled={busy}>{busy ? (creating ? 'Protecting your data…' : 'Unlocking…') : creating ? 'Protect and continue' : 'Unlock'}</button>
  </form>
  {#if passkeys.length}<button type="button" class="pill pill-lg" onclick={withPasskey} disabled={busy}>Unlock with a passkey</button>{/if}
  {#if osUnlock}<button type="button" class="pill pill-lg" onclick={withOs} disabled={busy}>Unlock with this OS account</button>{/if}
  {#if isDesktopShell()}<p class="help">Password-manager extensions may not fill this desktop window. Copy and paste from your manager, or enable OS unlock in You after unlocking.</p>{/if}
  {#if !creating || app.vaultState === 'migrate'}<VaultRecovery />{/if}
  <p class="help">Lock hoi when you are finished. Keep your device and browser updated. <a class="textlink" href="#/privacy">Privacy notice</a></p>
</main>

<style>
  .unlock { min-height: 100dvh; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 18px; padding-block: 40px; }
  h1 { font-size: var(--type-h1-size); text-align: center; }
  p { max-width: 34em; color: var(--mute); line-height: 1.5; text-align: center; }
  form { display: flex; flex-direction: column; gap: 12px; width: min(100%, 420px); }
  label { font-weight: 700; }
  input { width: 100%; border: 2px solid var(--ink); border-radius: 12px; padding: 12px; background: var(--card); color: var(--ink); font-size: 16px; }
  input:focus-visible { outline: 3px solid var(--cobalt); outline-offset: 2px; }
  .show { display: flex; align-items: center; gap: 8px; min-height: 44px; font-size: 14px; font-weight: 500; }
  .show input { width: auto; }
  .help { font-size: 14px; }
  .error { color: var(--ink); background: var(--peach); padding: 12px; border-radius: 12px; }
</style>
