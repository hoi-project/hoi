<script lang="ts">
  // On a device without a key: ask the friends who keep shares of your recovery line.
  // Send them the request, read them the digits, paste back what three of them answer.
  import type { Me, RecoveryAsk } from '@hoi/core-api';
  import { onMount } from 'svelte';
  import { app } from '../app.svelte';
  import { toast, toastError } from '../toast.svelte';
  import { copyText } from '../util';

  let { name, ondone }: { name: string; ondone: (me: Me) => void } = $props();

  let ask = $state<RecoveryAsk | null>(null);
  let answer = $state('');
  let answers = $state(0);
  let busy = $state(false);

  onMount(async () => {
    try {
      ask = await app.core.askFriends();
    } catch (e) {
      toastError(e);
    }
  });

  async function copy() {
    if (ask && (await copyText(ask.code))) toast('Request copied. Send it to your friends.');
  }

  async function take(e: SubmitEvent) {
    e.preventDefault();
    if (!answer.trim()) return;
    if (!name.trim()) return toast('Pick a name first. Any name; people see your key next to it.');
    busy = true;
    try {
      const got = await app.core.takeAnswer(answer.trim(), name.trim());
      answers = got.answers;
      answer = '';
      if (got.me) return ondone(got.me);
      toast(`${got.answers} of 3 answers in.`);
    } catch (err) {
      toastError(err);
    }
    busy = false;
  }
</script>

<div class="ask">
  {#if !ask}
    <p class="help">One moment…</p>
  {:else}
    <p class="help">Send this request to the friends who keep a share of your recovery line. Then call each of them: they should see the same digits.</p>
    <div class="keyrow"><input class="input mono-in" readonly value={ask.code} onfocus={(e) => e.currentTarget.select()} aria-label="Your request" /><button type="button" class="pill pill-sm" onclick={copy}>Copy</button></div>
    <p class="digits">Your digits: <b class="mono">{ask.digits}</b></p>
    <form class="take" onsubmit={take}>
      <label class="field" for="af-answer"><span>A friend's answer ({answers} of 3)</span><input id="af-answer" class="input mono-in" bind:value={answer} placeholder="hoi-share-1-…" spellcheck="false" autocapitalize="none" disabled={busy} /></label>
      <button type="submit" class="pill pill-ink" disabled={busy || !answer.trim()}>{busy ? 'Opening…' : 'Add this answer'}</button>
    </form>
    <p class="help">Keep this window open until three answers are in: they open only on this device, for this request.</p>
  {/if}
</div>

<style>
  .ask { display: flex; flex-direction: column; gap: 12px; }
  .help { color: var(--mute); font-size: 14px; line-height: 1.5; }
  .digits { font-size: 16px; }
  .take { display: flex; flex-direction: column; gap: 10px; }
  .keyrow { display: flex; gap: 8px; align-items: center; }
  .keyrow .input { flex: 1; min-width: 0; text-overflow: ellipsis; }
  @media (max-width: 480px) {
    .keyrow { flex-wrap: wrap; }
    .keyrow .input { flex-basis: 100%; }
  }
</style>
