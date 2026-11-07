<script lang="ts">
  import { settle } from '../motion';
  import CopyField from './CopyField.svelte';
  // An owner change that needs more signatures (LedgerResult 'needs-cosign').
  let { code, have, need, owners }: { code: string; have: number; need: number; owners: number } = $props();
  let el: HTMLElement | undefined = $state();
  $effect(() => settle(el ?? null));
</script>

<div class="inset proposal" bind:this={el} role="status">
  <b>Signed by you. It needs {need - have} more {need - have === 1 ? 'signature' : 'signatures'}.</b>
  <p class="help">
    Owner changes here need {need} of {owners} owner signatures; this one has {have}. Send the code to the other owners, any way
    you like. Each pastes it under <b>Cosign a proposal</b>. Nothing changes for anyone until enough have signed.
  </p>
  <CopyField label="Proposal code" value={code} what="Proposal code copied" />
</div>

<style>
  .proposal { border: 2px solid var(--ink); background: var(--sun); color: var(--tagink); }
  .proposal .help { color: var(--tagink); }
  .proposal :global(.label) { color: var(--tagink); }
</style>
