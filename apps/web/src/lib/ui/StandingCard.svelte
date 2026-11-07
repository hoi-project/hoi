<script lang="ts">
  import type { StandingView } from '@hoi/core-api';
  import { app, attempt } from '../app.svelte';
  import { voiceText } from '../util';

  // Where someone stands on a community's ladder, what it lets them do, and what's missing
  // (PROTOCOL.md §15). For someone else, established members can vouch or object here.
  let { s, self = false, canVouch = false }: { s: StandingView; self?: boolean; canVouch?: boolean } = $props();
  const RUNGS = ['newcomer', 'member', 'established', 'trusted'] as const;
  const WORDS: Record<string, string> = {
    text: 'text', links: 'links', vote: 'votes that count', review: 'reviews', vouch: 'vouches', veto: 'vetoes', media: 'images & media',
  };
  let objecting = $state(false);
  let reason = $state('spam');

  async function vouch(value: -1 | 0 | 1) {
    if (!(await app.confirmPosting(s.community))) return;
    const msg = value === 1 ? `You vouched for ${s.person.name}. Signed.` : value === -1 ? `You object to ${s.person.name}, with a reason. Signed.` : 'Taken back. Signed, too.';
    await attempt(() => app.core.vouch(s.community, s.person.key, value, value === -1 ? (reason as 'spam') : undefined), msg);
    objecting = false;
  }
</script>

<div class="frame kvbox standing">
  <div class="kv head"><span>{self ? 'Your place here' : 'Their place here'}</span><span class="tag tag-sun">{s.blocked ? 'blocked' : s.level}</span></div>
  <ol class="ladder" aria-label="The ladder">
    {#each RUNGS as r (r)}
      <li class={[r === s.level && 'here', RUNGS.indexOf(r) < RUNGS.indexOf(s.level) && 'done']}>{r}</li>
    {/each}
  </ol>
  {#if s.blocked}
    <p class="note2">The community blocked {self ? 'you' : 'them'}: enough established members hid {self ? 'your' : 'their'} posts or objected. It lifts as their reasons fade or others speak up.</p>
  {:else}
    <div class="kv"><span>Can share</span><span>{s.capabilities.map((c) => WORDS[c] ?? c).join(', ')}</span></div>
    <div class="kv"><span>Vouched for by</span><span>{s.vouchedBy.length ? s.vouchedBy.map((p) => p.name).join(', ') : 'nobody yet'}</span></div>
    {#if s.level !== 'newcomer'}
      <div class="kv"><span>Voice</span><span>{voiceText(s.voice)}</span></div>
    {/if}
    {#if s.objections}<div class="kv"><span>Objections</span><span>{s.objections}</span></div>{/if}
    <div class="kv"><span>Identity work</span><span>{s.identityBits} bits{s.identityNeeded ? ` (${s.identityNeeded} asked)` : ''}</span></div>
    {#if s.next}
      <p class="note2"><b>Next: {s.next.level}.</b> {s.next.missing.join(' · ')}</p>
    {/if}
    {#if s.level !== 'newcomer' && s.voice < 0.995 && !s.person.role}
      <p class="note2">{self ? 'You were' : 'They were'} vouched in along with others, so {self ? 'you' : 'they'} share a voucher's voice in hides, vetoes and changes. It fills out as more members read {self ? 'your' : 'their'} posts and vouch.</p>
    {/if}
    {#if self && s.vouchedThisQuarter > 1}
      <p class="note2">You vouched for {s.vouchedThisQuarter} people this quarter: each of those vouches carries 1/{s.vouchedThisQuarter} of your voice.</p>
    {/if}
    {#if s.struck}<p class="note2">Vouched for people the community later blocked: no voice in reviews and vetoes for now.</p>{/if}
  {/if}
  {#if !self && canVouch}
    <div class="abtns">
      {#if s.myVouch === 1}
        <button type="button" class="pill pill-sm pill-ink" aria-pressed="true" onclick={() => vouch(0)}>✓ You vouch</button>
      {:else}
        <button type="button" class="pill pill-sm" onclick={() => vouch(1)}>Vouch</button>
      {/if}
      {#if s.myVouch === -1}
        <button type="button" class="pill pill-sm" onclick={() => vouch(0)}>Take back objection</button>
      {:else}
        <button type="button" class="pill pill-sm" aria-expanded={objecting} onclick={() => (objecting = !objecting)}>Object</button>
      {/if}
    </div>
    {#if objecting}
      <form class="obj" onsubmit={(e) => { e.preventDefault(); vouch(-1); }}>
        <label class="field">
          <span>Why? Objections are signed and public. Disagreeing isn't a reason.</span>
          <select class="input" bind:value={reason}>
            <option value="spam">Spam</option>
            <option value="abuse">Abuse or harassment</option>
            <option value="off-topic">Keeps posting off-topic</option>
            <option value="illegal">Illegal</option>
          </select>
        </label>
        <div class="btnrow"><button class="pill pill-danger pill-sm" type="submit">Sign the objection</button><button class="pill pill-sm" type="button" onclick={() => (objecting = false)}>Cancel</button></div>
      </form>
    {/if}
  {/if}
</div>

<style>
  .standing { padding-top: 12px; }
  .ladder { display: flex; gap: 4px; list-style: none; margin: 8px 0 10px; padding: 0; flex-wrap: wrap; }
  .ladder li { font-size: 12px; font-weight: 700; padding: 3px 8px; border-radius: 999px; border: 2px dotted var(--line); color: var(--mute); }
  .ladder li.done { border-style: solid; border-color: var(--ink); color: var(--ink); }
  .ladder li.here { background: var(--sun); border: 2px solid var(--ink); color: #1a1a1a; }
  .note2 { margin: 10px 0 0; font-size: 13px; color: var(--mute); line-height: 1.45; font-weight: 500; }
  .note2 b { color: var(--ink); }
  .abtns { display: flex; gap: 6px; margin-top: 10px; flex-wrap: wrap; }
  .abtns .pill { flex: 1; }
  .obj { display: flex; flex-direction: column; gap: 8px; padding: 8px 0 0; }
</style>
