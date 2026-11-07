<script lang="ts">
  // Two backup choices: the recovery key, one line from which every root you will ever
  // sign with is derived, for a password manager (which is encrypted) or a drawer; and
  // the .hoikey file, this root under its own passphrase, for a disk or a cloud. The device keeps
  // the line until you say it is saved, then forgets it, so a stolen device can't replace your
  // root. The page holds it only in this form until the modal closes.
  import type { Me } from '@hoi/core-api';
  import { onDestroy } from 'svelte';
  import { app } from '../app.svelte';
  import { isDesktopShell } from '../host';
  import { backupLogin, canStoreCredential, credentialValue, recoveryLogin, sameRecoveryKey, storeCredential } from '../password-manager';
  import { errorText, toast } from '../toast.svelte';
  import { copyText, saveFile, slug } from '../util';

  let { me: given, ondone }: { me?: Me; ondone?: () => void } = $props();
  const me = $derived(given ?? app.me!);
  const desktop = isDesktopShell();
  const browserSaves = !desktop && canStoreCredential();
  /** What the password manager lists the key under: unique per key, readable at a glance. */
  const login = $derived(recoveryLogin(me.name, me.short));
  let entryLabel = $state('');
  $effect(() => { entryLabel = login; });
  const fileId = crypto.randomUUID().slice(0, 8);
  const filename = $derived(`${slug(me.name)}-${fileId}.hoikey`);
  let method = $state<'recovery' | 'file'>('recovery');
  let checking = $state(false);
  let verified = $state(false);
  let checkKey = $state('');
  let error = $state('');
  let showFilePassword = $state(false);

  let key = $state('');
  let shown = $state(false);
  let busy = $state(false);
  let keyEl: HTMLInputElement | undefined = $state();

  async function reveal() {
    if (busy) return;
    error = '';
    busy = true;
    try {
      key = await app.core.recoveryKey();
      shown = false;
    } catch (e) {
      error = errorText(e);
    }
    busy = false;
  }

  /** The line is saved: this device forgets it for good. */
  async function forget() {
    if (busy || !verified) return;
    busy = true;
    error = '';
    try {
      if (!sameRecoveryKey(key, await app.core.recoveryKey())) throw new Error('Your recovery key changed. Close this dialog and back up the current key.');
      await app.core.forgetRecoveryKey();
      key = '';
      await app.refreshMe();
      toast('This device forgot your recovery key. Keep the line you saved safe: it is how you come back.');
      ondone?.();
    } catch (e) {
      error = errorText(e);
    }
    busy = false;
  }

  async function copy() {
    if (await copyText(key)) toast('Recovery key copied. Paste it into your password manager, then clear your clipboard.');
    else toast('Copy did not work here. Show the key and copy it yourself.');
  }

  /** What came of asking the browser: nothing yet, it showed its own save prompt, or it said no. */
  let asked = $state<'' | 'stored' | 'declined'>('');

  /** The form submit is what password manager extensions watch for; browsers that can be asked
   * outright (Chrome, Edge) are. Others give no sign either way, so the modal stays open and
   * says how to add it by hand instead of claiming a prompt that may never have come. */
  async function remember(e: SubmitEvent) {
    e.preventDefault();
    if (!key || busy) return;
    entryLabel = credentialValue(e.currentTarget as HTMLFormElement, 'username').trim();
    if (!entryLabel) { error = 'Give this recovery key an entry name in your password manager.'; return; }
    if (!browserSaves) { checking = true; return; }
    busy = true;
    const result = await storeCredential(entryLabel, key, 'hoi recovery key');
    busy = false;
    asked = result === 'stored' ? 'stored' : 'declined';
  }

  function beginVerification() {
    if (keyEl?.form) entryLabel = credentialValue(keyEl.form, 'username');
    checking = true;
    shown = false;
    error = '';
  }

  function verify(e: SubmitEvent) {
    e.preventDefault();
    checkKey = credentialValue(e.currentTarget as HTMLFormElement);
    verified = sameRecoveryKey(checkKey, key);
    checkKey = '';
    error = verified ? '' : 'That is not this identity’s recovery key. Select the matching hoi key entry in your manager and try again.';
  }

  function choose(next: 'recovery' | 'file') {
    method = next;
    key = checkKey = pass1 = pass2 = error = '';
    checking = verified = shown = showFilePassword = false;
    asked = '';
  }

  // ---------------------------------------------------------- the encrypted file
  let pass1 = $state('');
  let pass2 = $state('');
  async function exportKey(e: SubmitEvent) {
    e.preventDefault();
    if (busy) return;
    error = '';
    const form = e.currentTarget as HTMLFormElement;
    pass1 = credentialValue(form);
    pass2 = credentialValue(form, 'password-confirmation');
    if ([...pass1].length < 15) { error = 'Use at least 15 characters. Your password manager can generate this password.'; return; }
    if (pass1 !== pass2) { error = "The two passwords don't match."; return; }
    busy = true;
    try {
      const data = await app.core.exportIdentity(pass1);
      await saveFile(filename, data);
      toast(desktop ? 'Backup file saved. Keep its password separately.' : 'Backup download started. Check Downloads and keep the file’s password separately.');
      ondone?.();
    } catch (err) {
      error = errorText(err);
    } finally {
      pass1 = pass2 = '';
      busy = false;
    }
  }

  onDestroy(() => {
    key = '';
    checkKey = '';
    pass1 = pass2 = '';
  });
</script>

<div class="backup" aria-busy={busy}>
  <p class="lead">Back up your identity so you can restore it on a new device. Your vault password only unlocks this installation.</p>
  <div class="btnrow" role="group" aria-label="Choose an identity backup">
    <button type="button" class="pill" aria-pressed={method === 'recovery'} onclick={() => choose('recovery')} disabled={busy}>Recovery key</button>
    <button type="button" class="pill" aria-pressed={method === 'file'} onclick={() => choose('file')} disabled={busy}>Encrypted file</button>
  </div>
  {#if error}<p class="pmnote warn" role="alert">{error}</p>{/if}

  {#if method === 'recovery'}
  <section class="way" aria-labelledby="kb-pm">
    <h3 id="kb-pm">Recovery key <span class="for">for your password manager</span></h3>
    <p class="help">One line that brings you back on any device, with your standing, even if a device with your key is stolen. Anyone who reads it can be you: never paste it into a post, a chat or a website.</p>
    {#if !me.holdsLine}
      <p class="help">This {app.platformName} no longer keeps your recovery key. It's in your password manager, on paper, or with your friends.</p>
    {:else if !key}
      <button type="button" class="pill pill-ink" onclick={reveal} disabled={busy}>{busy ? 'One moment…' : 'Show my recovery key'}</button>
    {:else if checking}
      {#if verified}
        <p class="pmnote ok" role="status">The saved recovery key matches this identity. You can use it under Restore my identity on a fresh device.</p>
        <p class="help">You may now remove this device's copy of the recovery key. Keep the saved entry: OS unlock and a vault password cannot replace it.</p>
        <div class="btnrow">
          {#if !me.restoring}<button type="button" class="pill" onclick={forget} disabled={busy}>Forget the recovery key here</button>{/if}
          <button type="button" class="pill pill-ink" onclick={() => ondone?.()} disabled={busy}>Done</button>
        </div>
      {:else}
        <form id="verify-recovery" class="pmform" onsubmit={verify} autocomplete="on">
          <p>Open the entry you saved in your manager. Fill or paste its recovery key below to check that you can retrieve it. This check stays on this device and does not restore or rotate your identity.</p>
          <label class="field" for="kb-check-login"><span>Password-manager entry</span><input id="kb-check-login" class="input" name="username" autocomplete="username" value={entryLabel} spellcheck="false" autocapitalize="none" /></label>
          <label class="field" for="kb-check-key"><span>Saved recovery key</span><input id="kb-check-key" class="input mono-in" name="password" type="password" autocomplete="current-password" bind:value={checkKey} required spellcheck="false" autocapitalize="none" /></label>
          <div class="btnrow"><button type="submit" class="pill pill-ink">Check saved key</button><button type="button" class="pill" onclick={() => { checking = false; error = ''; checkKey = ''; }}>Back</button></div>
        </form>
      {/if}
    {:else}
      <form id="save-recovery" class="pmform" onsubmit={remember} autocomplete="on">
        <label class="field" for="kb-login"><span>Password-manager entry</span><input id="kb-login" class="input" type="text" name="username" autocomplete="username" bind:value={entryLabel} required spellcheck="false" autocapitalize="none" /></label>
        <label class="field" for="kb-key"><span>Recovery key <span class="pw">(the password)</span></span>
          <span class="keyrow">
            <input id="kb-key" class="input mono-in" name="password" type={shown ? 'text' : 'password'} autocomplete="new-password" value={key} readonly spellcheck="false" autocapitalize="none" bind:this={keyEl} onfocus={(e) => e.currentTarget.select()} />
            <button type="button" class="pill pill-sm" onclick={() => (shown = !shown)} aria-pressed={shown}>{shown ? 'Hide' : 'Show'}</button>
            <button type="button" class="pill pill-sm" onclick={copy}>Copy</button>
          </span>
        </label>
        {#if asked === 'stored'}
          <p class="pmnote ok" role="status">The save request was sent to your browser. Check your manager for the entry, then verify the saved key below.</p>
        {:else if asked === 'declined'}
          <p class="pmnote warn" role="status">Your browser didn't save it. Press <b>Copy</b> and add a login by hand.</p>
        {:else}
          <p class="help">
            Use these two fields for an entry in Dashlane, 1Password or your browser's manager. {#if browserSaves}You can ask this browser to save it.{:else}Press <b>Copy</b>, then add the entry in your manager.{/if} Keep it separate from your vault password.
          </p>
        {/if}
        <div class="btnrow">
          {#if browserSaves}<button type="submit" class="pill pill-ink" disabled={busy}>Ask browser to save</button>{/if}
          <button type="button" class="pill" onclick={beginVerification} disabled={busy}>Verify saved key</button>
        </div>
      </form>
    {/if}
  </section>

  {:else}
  <section class="way" aria-labelledby="kb-file">
    <h3 id="kb-file">Encrypted file <span class="for">for a disk or a cloud</span></h3>
    <dl class="inside">
      <div><dt>Inside</dt><dd class="tags">
        <span class="tag tag-coral">your key</span>
        <span class="tag tag-sky">name & picture</span>
        <span class="tag tag-mint">communities you're in</span>
        <span class="tag tag-lilac">friends & petnames</span>
        <span class="tag tag-sun">proofs for public names you made</span>
      </dd></div>
      <div><dt>Not inside</dt><dd class="tags"><span class="tag tag-dashed">private communities</span></dd></div>
      <div><dt>Locked with</dt><dd>its own password, 15 characters or more</dd></div>
      <div><dt>Keep</dt><dd>the file and the password <b>in different places</b>: you need both</dd></div>
    </dl>
    <form id="export-backup" class="mform" onsubmit={exportKey} autocomplete="on">
      <label class="field" for="kb-file-login"><span>Password-manager entry</span><input id="kb-file-login" class="input" type="text" name="username" autocomplete="username" value={backupLogin(filename)} spellcheck="false" autocapitalize="none" disabled={busy} /></label>
      <p class="help">Save the password for <b>{filename}</b> in this entry. A new backup gets its own filename, so older file passwords remain usable.</p>
      <label class="field" for="kb-pass1"><span>New backup-file password</span><input id="kb-pass1" name="password" class="input" type={showFilePassword ? 'text' : 'password'} bind:value={pass1} minlength="15" autocomplete="new-password" required disabled={busy} /></label>
      <label class="field" for="kb-pass2"><span>Repeat backup-file password</span><input id="kb-pass2" name="password-confirmation" class="input" type={showFilePassword ? 'text' : 'password'} bind:value={pass2} minlength="15" autocomplete="new-password" required disabled={busy} /></label>
      <label class="show"><input type="checkbox" bind:checked={showFilePassword} disabled={busy} /> Show backup-file password</label>
      {#if pass2 && pass1 !== pass2}<p class="help bad">Those don't match yet.</p>{/if}
      <div class="btnrow"><button class="pill" type="submit" disabled={busy}>{busy ? 'Encrypting…' : 'Download .hoikey'}</button></div>
    </form>
  </section>
  {/if}

  <p class="help">On a fresh device, protect its local vault, then choose <b>Restore my identity</b>. Select your recovery-key entry, or your file and its saved password. Lost a device? Revoke it under <b>Your devices</b>.</p>
</div>

<style>
  .backup { display: flex; flex-direction: column; gap: 18px; }
  .show { display: flex; align-items: center; gap: 8px; min-height: 44px; font-size: 14px; }
  .lead { line-height: 1.5; font-weight: 500; }
  .way { border: 2px solid var(--line); border-radius: 16px; padding: 16px 18px; display: flex; flex-direction: column; gap: 10px; }
  h3 { font-size: 19px; letter-spacing: -0.02em; display: flex; flex-wrap: wrap; gap: 8px; align-items: baseline; }
  .for { font-size: 14px; font-weight: 600; color: var(--mute); }
  .help { color: var(--mute); font-size: 14px; line-height: 1.5; }
  .help b { color: var(--ink); }
  .pmform, .mform { display: flex; flex-direction: column; gap: 10px; }
  .keyrow { display: flex; gap: 8px; align-items: center; }
  .keyrow .input { flex: 1; min-width: 0; text-overflow: ellipsis; }
  .bad { color: var(--ink); }
  .pw { font-weight: 600; color: var(--mute); }
  .pmnote { font-size: 14px; line-height: 1.45; padding: 10px 12px; border-radius: 12px; color: var(--tagink); }
  .pmnote.ok { background: var(--mint); }
  .pmnote.warn { background: var(--peach); }
  .inside { margin: 0; display: flex; flex-direction: column; }
  .inside div { display: grid; grid-template-columns: 96px minmax(0, 1fr); gap: 12px; padding: 8px 0; border-top: 1px solid var(--line); align-items: baseline; }
  .inside div:first-child { border-top: 0; }
  .inside dt { font-size: 13px; font-weight: 800; }
  .inside dd { margin: 0; font-size: 14px; color: var(--mute); line-height: 1.45; }
  .inside dd.tags { display: flex; flex-wrap: wrap; gap: 6px; }
  .inside dd b { color: var(--ink); }
  @media (max-width: 480px) {
    .inside div { grid-template-columns: 1fr; gap: 4px; }
    .keyrow { flex-wrap: wrap; }
    .keyrow .input { flex-basis: 100%; }
  }
</style>
