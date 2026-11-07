<script lang="ts">
  // Joining an age-proof community: your wallet makes a presentation bound to what is
  // shown here; this device checks it and makes a code for a moderator. Only "over 18" leaves.
  import { app, attempt } from '../app.svelte';
  import { resource } from '../resource.svelte';
  import { toast } from '../toast.svelte';
  import { copyText } from '../util';

  let { community }: { community: string } = $props();
  const challenge = resource(() => app.core.ageChallenge(community));
  let presentation = $state('');
  let code = $state('');

  async function make(e: SubmitEvent) {
    e.preventDefault();
    const c = await attempt(() => app.core.ageProofCode(community, presentation.trim()));
    if (typeof c === 'string') code = c;
  }
  async function copy() {
    if (await copyText(code)) toast('Code copied. Send it to a moderator here.');
  }
</script>

<div class="frame kvbox door anim">
  <div class="kv head"><span>This community asks for a proof of age</span><span class="tag tag-sun">at the door</span></div>
  <p class="help">What you post waits on your device until a moderator saw a proof that you are over 18. Your identity wallet makes the proof; it shows your age and nothing else, and it works only for your key and this community.</p>
  {#if challenge.value}
    <p class="help">Your wallet asks for an audience and a nonce: <span class="mono">{challenge.value.aud}</span> and <span class="mono">{challenge.value.nonce}</span>.</p>
  {/if}
  {#if !code}
    <form class="ageform" onsubmit={make}>
      <label class="field" for="age-pres"><span>The wallet's presentation</span>
        <textarea id="age-pres" class="input mono-in" rows="3" bind:value={presentation} placeholder="eyJ…~…~eyJ…" spellcheck="false"></textarea>
      </label>
      <button type="submit" class="pill pill-ink pill-sm" disabled={!presentation.trim()}>Check it and make my code</button>
    </form>
  {:else}
    <p class="help">Send this code to a moderator here, however you like. It holds the proof: nobody else needs to see it.</p>
    <div class="keyrow"><input class="input mono-in" readonly value={code} onfocus={(e) => e.currentTarget.select()} aria-label="Your age proof code" /><button type="button" class="pill pill-sm" onclick={copy}>Copy</button></div>
  {/if}
</div>

<style>
  .ageform { display: flex; flex-direction: column; gap: 8px; align-items: flex-start; }
  .ageform .field { width: 100%; }
  .mono { overflow-wrap: anywhere; }
  .keyrow { display: flex; gap: 8px; align-items: center; }
  .keyrow .input { flex: 1; min-width: 0; }
</style>
