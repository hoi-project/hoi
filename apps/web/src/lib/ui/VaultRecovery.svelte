<script lang="ts">
  import { lockVault, resetVault } from '../core';
  import { app } from '../app.svelte';
  import { errorText } from '../toast.svelte';

  let open = $state(false);
  let confirmation = $state('');
  let acknowledged = $state(false);
  let busy = $state(false);
  let error = $state('');

  async function lock() {
    busy = true;
    try { await lockVault(); }
    catch (e) { error = errorText(e); busy = false; }
  }

  async function reset(event: SubmitEvent) {
    event.preventDefault();
    if (busy || !acknowledged || confirmation !== 'RESET') return;
    busy = true;
    error = '';
    try { await resetVault(confirmation); }
    catch (e) { error = errorText(e); busy = false; }
  }
</script>

<details class="recovery" bind:open>
  <summary>Can't unlock? Recover or start over</summary>
  {#if open}
    <div class="options">
      <p><b>Check which password you saved.</b> Select this installation's “hoi — browser vault” or “hoi — desktop vault” entry in your manager. A “hoi key” entry restores your identity; a “hoi key file” entry opens an encrypted backup. Neither unlocks this vault.</p>
      <p><b>Have a recovery key, a key file or another signed-in device?</b> You can restore on a fresh browser profile or device while keeping this vault. If you choose to clear this installation instead, protect it with a new vault password, then use <b>Restore my identity</b>. Private community access may need a new invitation.</p>
      <p><b>Without either, your old key cannot be recovered.</b> You can start again with a new key. Its work, memberships and reputation start over.</p>
      {#if app.vaultState === 'ready'}
        <p>The local vault is still open. Lock it before clearing local data.</p>
        <button class="pill" type="button" onclick={lock} disabled={busy}>Lock and reload</button>
      {/if}
      <form onsubmit={reset} aria-busy={busy}>
        <h2>Clear this device's local data</h2>
        <p>Deletes the saved key, communities, posts, private community data, settings and local passphrase here. It does not delete copies on other devices or revoke your old key. This cannot be undone.</p>
        <p><b>It deletes nothing from the network.</b> Posts you sent stay until they fade. Deletions this device started stop unfinished: a locked vault can't be read to finish them or show what they were, and nothing reports them as done. Exported key backups and your other devices keep theirs. Deleting files doesn't guarantee the storage is physically erased.</p>
        <label class="acknowledge"><input type="checkbox" bind:checked={acknowledged} disabled={busy} /> I have a key backup or another signed-in device, or accept losing access to my old key.</label>
        <label for="vault-reset-confirm">Type RESET to continue</label>
        <input id="vault-reset-confirm" class="input" name="reset-confirmation" bind:value={confirmation} autocomplete="off" spellcheck="false" disabled={busy} />
        {#if error}<p class="error" role="alert">{error}</p>{/if}
        {#if busy}<p role="status">Waiting for local storage. Close other hoi tabs if this takes a moment.</p>{/if}
        <button class="pill pill-coral" type="submit" disabled={busy || !acknowledged || confirmation !== 'RESET'}>{busy ? 'Clearing local data…' : 'Clear local data'}</button>
      </form>
    </div>
  {/if}
</details>

<style>
  .recovery { width: min(100%, 560px); border-top: 2px dotted var(--line); padding-top: 18px; text-align: left; }
  summary { font-weight: 750; cursor: pointer; padding-block: 8px; }
  .options { display: grid; gap: 14px; margin-top: 12px; }
  p, .acknowledge { color: var(--mute); font-size: 14px; line-height: 1.55; }
  b { color: var(--ink); }
  form { border: 1px solid var(--line); border-radius: 16px; padding: 18px; display: grid; gap: 12px; }
  h2 { font-size: 18px; }
  label { font-weight: 600; }
  .acknowledge { display: flex; gap: 10px; align-items: flex-start; }
  .acknowledge input { flex: 0 0 auto; margin-top: 4px; }
  button { justify-self: start; }
  .error { border-radius: 10px; background: var(--peach); color: var(--ink); padding: 12px; }
</style>
