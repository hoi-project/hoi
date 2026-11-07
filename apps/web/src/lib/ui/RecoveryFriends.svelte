<script lang="ts">
  // Friends who keep your recovery line in shares: any three of the people you choose
  // bring it back, none of them alone can read it. Also: answering a friend who lost theirs,
  // after comparing the digits their new device shows.
  import type { HeldShare, Person, RecoveryAnswer, RecoveryStatus } from '@hoi/core-api';
  import { onMount } from 'svelte';
  import { app } from '../app.svelte';
  import { toast, toastError } from '../toast.svelte';
  import { copyText } from '../util';

  let { ondone }: { ondone?: () => void } = $props();
  const me = $derived(app.me!);

  let status = $state<RecoveryStatus | null>(null);
  let held = $state<HeldShare[]>([]);
  let community = $state('');
  let candidates = $state<Person[]>([]);
  let picked = $state<string[]>([]);
  let busy = $state(false);

  // helping a friend
  let helping = $state<HeldShare | null>(null);
  let request = $state('');
  let digits = $state('');
  let answer = $state<RecoveryAnswer | null>(null);

  onMount(async () => {
    try {
      [status, held] = await Promise.all([app.core.recoveryStatus(), app.core.heldShares()]);
    } catch (e) {
      toastError(e);
    }
  });

  async function pickCommunity() {
    candidates = [];
    picked = [];
    if (!community) return;
    try {
      candidates = await app.core.shareCandidates(community);
    } catch (e) {
      toastError(e);
    }
  }

  function toggle(key: string) {
    picked = picked.includes(key) ? picked.filter((k) => k !== key) : picked.length < 7 ? [...picked, key] : picked;
  }

  async function give() {
    busy = true;
    try {
      status = await app.core.giveShares(community, picked);
      toast(`${picked.length} friends now keep a share each. Any three of them bring you back.`);
      ondone?.();
    } catch (e) {
      toastError(e);
    }
    busy = false;
  }

  async function check() {
    digits = '';
    answer = null;
    try {
      digits = await app.core.requestDigits(request);
    } catch (e) {
      toastError(e);
    }
  }

  async function answerIt() {
    if (!helping) return;
    busy = true;
    try {
      answer = await app.core.answerRequest(helping.person, request);
    } catch (e) {
      toastError(e);
    }
    busy = false;
  }

  async function copyAnswer() {
    if (answer && (await copyText(answer.code))) toast('Answer copied. Send it to your friend.');
  }
</script>

<div class="friends">
  <section aria-labelledby="rf-give">
    <h3 id="rf-give">Your friends</h3>
    {#if status && status.friends.length}
      <p class="help">{status.friends.length} friends keep a share of your recovery line. Any three of them bring you back, with your standing. None of them can read it alone.</p>
    {:else}
      <p class="help">Choose 3 to 7 people you trust. Each keeps one share of your recovery line, on their own devices. Any three of them bring you back. Everyone in the community can see whom you chose.</p>
    {/if}
    {#if !me.holdsLine}
      <p class="help">This {app.platformName} no longer keeps your recovery line, so it can't make new shares. Bring the line back here first if you want to choose again.</p>
    {:else}
      <label class="field" for="rf-community"><span>Community you share with them</span>
        <select id="rf-community" class="input" bind:value={community} onchange={pickCommunity}>
          <option value="">Choose…</option>
          {#each app.joined as c (c.id)}<option value={c.id}>c/{c.name}</option>{/each}
        </select>
      </label>
      {#if community}
        {#if candidates.length === 0}
          <p class="help">Nobody here can keep a share yet: people need to have posted from a device that can open one.</p>
        {:else}
          <ul class="people" role="list">
            {#each candidates as p (p.key)}
              <li><label class="pick"><input type="checkbox" checked={picked.includes(p.key)} onchange={() => toggle(p.key)} /> {p.name} <span class="mono short">{p.short}</span></label></li>
            {/each}
          </ul>
          <div class="btnrow">
            <button type="button" class="pill pill-ink" onclick={give} disabled={busy || picked.length < 3}>{picked.length < 3 ? `Pick ${3 - picked.length} more` : `Give ${picked.length} shares`}</button>
          </div>
        {/if}
      {/if}
    {/if}
  </section>

  <section aria-labelledby="rf-help">
    <h3 id="rf-help">Help a friend</h3>
    {#if held.length === 0}
      <p class="help">Nobody gave you a share yet.</p>
    {:else if !helping}
      <p class="help">You keep a share for these people. If one of them lost everything, their new device shows a request and six digits.</p>
      <ul class="people" role="list">
        {#each held as h (h.person)}
          <li><button type="button" class="lnk lnk-ink" onclick={() => (helping = h)}>{h.name}</button> <span class="mono short">{h.short}</span></li>
        {/each}
      </ul>
    {:else}
      <p class="help">Paste the request <b>{helping.name}</b> sent you. Then call them and compare the digits before you answer.</p>
      <label class="field" for="rf-request"><span>Their request</span><input id="rf-request" class="input mono-in" bind:value={request} oninput={check} placeholder="hoi-ask-1-…" spellcheck="false" autocapitalize="none" /></label>
      {#if digits}
        <p class="digits" role="status">Their new device should show <b class="mono">{digits}</b></p>
        {#if !answer}
          <div class="btnrow">
            <button type="button" class="pill pill-ink" onclick={answerIt} disabled={busy}>The digits match. Answer</button>
            <button type="button" class="pill" onclick={() => { helping = null; request = ''; digits = ''; }}>They don't</button>
          </div>
        {:else}
          <p class="help">Send this answer back to {helping.name}. It opens only on the device that asked.</p>
          <div class="keyrow"><input class="input mono-in" readonly value={answer.code} onfocus={(e) => e.currentTarget.select()} /><button type="button" class="pill pill-sm" onclick={copyAnswer}>Copy</button></div>
        {/if}
      {/if}
    {/if}
  </section>
</div>

<style>
  .friends { display: flex; flex-direction: column; gap: 18px; }
  section { border: 2px solid var(--line); border-radius: 16px; padding: 16px 18px; display: flex; flex-direction: column; gap: 10px; }
  h3 { font-size: 19px; letter-spacing: -0.02em; }
  .help { color: var(--mute); font-size: 14px; line-height: 1.5; }
  .help b { color: var(--ink); }
  .people { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .pick { display: flex; gap: 8px; align-items: center; }
  .short { color: var(--mute); font-size: 13px; }
  .digits { font-size: 16px; line-height: 1.5; }
  .keyrow { display: flex; gap: 8px; align-items: center; }
  .keyrow .input { flex: 1; min-width: 0; text-overflow: ellipsis; }
  @media (max-width: 480px) {
    .keyrow { flex-wrap: wrap; }
    .keyrow .input { flex-basis: 100%; }
  }
</style>
