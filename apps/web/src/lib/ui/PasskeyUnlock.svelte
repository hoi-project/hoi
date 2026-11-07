<script lang="ts">
  // You → Unlock with a passkey: a passkey opens this browser's vault as the
  // passphrase does. The passphrase stays; the recovery line stays the way back to you.
  import { onMount } from 'svelte';
  import { app } from '../app.svelte';
  import { addVaultPasskey, removeVaultPasskeys, vaultPasskeys } from '../core';
  import { isDesktopShell } from '../host';
  import { makePasskey, passkeysPossible } from '../passkey';
  import { toast, toastError } from '../toast.svelte';

  const shown = !isDesktopShell() && passkeysPossible();
  let count = $state(0);
  let busy = $state(false);

  onMount(async () => {
    if (shown) count = (await vaultPasskeys().catch(() => [])).length;
  });

  async function add() {
    busy = true;
    try {
      const made = await makePasskey(`hoi · ${app.me?.name ?? 'me'} · ${app.platformName}`);
      await addVaultPasskey(made.credential, made.salt, made.prf);
      count = (await vaultPasskeys()).length;
      toast('This passkey unlocks hoi here now. Keep your passphrase too.');
    } catch (e) {
      toastError((e as Error)?.name === 'NotAllowedError' ? new Error('No passkey was added.') : e);
    }
    busy = false;
  }

  async function remove() {
    busy = true;
    try {
      await removeVaultPasskeys();
      count = 0;
      toast('Passkeys no longer unlock hoi here. Your passphrase does.');
    } catch (e) {
      toastError(e);
    }
    busy = false;
  }
</script>

{#if shown}
  <div class="set">
    <span class="l">Unlock with a passkey</span>
    <span class="h">{count ? `${count} passkey${count === 1 ? '' : 's'} unlock hoi in this ${app.platformName}, beside your passphrase.` : `Your fingerprint, face or device PIN unlocks hoi in this ${app.platformName}, beside your passphrase.`}</span>
    <span class="v">
      <button type="button" class="lnk lnk-ink" onclick={add} disabled={busy}>Add a passkey</button>
      {#if count}<button type="button" class="lnk lnk-ink" onclick={remove} disabled={busy}>Remove</button>{/if}
    </span>
  </div>
{/if}
