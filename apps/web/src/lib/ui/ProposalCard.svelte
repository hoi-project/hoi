<script lang="ts">
  import type { ProposalView } from '@hoi/core-api';
  import { ago, fadesIn } from '@hoi/core-api';
  import { app, attempt } from '../app.svelte';
  import { toast } from '../toast.svelte';
  import { to } from '../router';
  import { changeText, plural, voices } from '../util';

  // A proposed change to a community's rules or limits, and where the people deciding it stand
  // (PROTOCOL.md §6.1). Everyone established when it was proposed decides; people who are away
  // don't hold it up; one retention window later, silence counts as yes.
  let { p, community, compact = false }: { p: ProposalView; community: string; compact?: boolean } = $props();
  let objecting = $state(false);
  let why = $state('');

  const STATUS: Record<ProposalView['status'], { text: string; color: string }> = {
    open: { text: 'deciding', color: 'sun' },
    'in-effect': { text: 'in effect', color: 'mint' },
    passed: { text: 'agreed', color: 'mint' },
    lapsed: { text: 'didn’t pass', color: 'soft' },
  };
  const what = $derived(p.kind === 'rules' ? 'new rules' : plural(p.changes.length, 'change'));
  const who = $derived(p.by.map((x) => (x.trust === 'me' ? 'you' : x.name)).join(', '));

  // Owners may say an away elector left for good: after a week, their silence stops
  // holding the change up. Anything they sign here voids it; members can veto it.
  async function retire(key: string, name: string) {
    const r = await attempt(() => app.core.retire(community, key));
    if (r?.status === 'published') toast(`Signed: ${name} left for good. In a week their silence stops counting, unless they sign something here first.`);
    else if (r) toast(`Proposed. It needs ${r.need - r.have} more owner signature${r.need - r.have === 1 ? '' : 's'}.`);
  }

  async function answer(verdict: 'ok' | 'veto' | 'none') {
    const msg = verdict === 'ok'
      ? 'You agreed. Signed and public; it applies once everyone has.'
      : verdict === 'veto'
        ? 'You objected, with your reason. Signed and public: it waits while you do.'
        : 'Answer taken back. Signed, too.';
    const ok = await attempt(() => app.core.review(p.id, verdict, verdict === 'veto' ? why.trim() : undefined), msg);
    if (ok !== undefined) {
      objecting = false;
      why = '';
    }
  }
</script>

<article class="frame kvbox prop" aria-label={`Proposed ${what}`}>
  <div class="kv head">
    <span><b>{who}</b> {p.status === 'passed' || p.status === 'lapsed' ? 'proposed' : p.by.length === 1 && p.by[0].trust !== 'me' ? 'proposes' : 'propose'} {what} · {ago(p.at)}</span>
    <span class="tag tag-{STATUS[p.status].color}">{STATUS[p.status].text}</span>
  </div>

  {#if p.kind === 'limits'}
    <ul class="changes">
      {#each p.changes as ch (ch.field)}
        {@const t = changeText(ch.field, ch.from, ch.to)}
        <li><span class="lbl">{t.label}</span><span class="from">{t.from}</span><span aria-hidden="true">→</span><span class="sr-only">to</span><b>{t.to}</b></li>
      {/each}
    </ul>
  {:else if p.text && !compact}
    <pre class="rules">{p.text}</pre>
  {/if}

  {#if !compact || p.status === 'open'}
    <p class="tally">
      {p.agreed.length} of {p.electors} agreed{p.objecting.length ? ` · ${p.objecting.length} object${p.objecting.length === 1 ? 's' : ''}` : ''}{p.waiting.length ? ` · ${p.waiting.length} still to answer` : ''}{p.away ? ` · ${p.away} away` : ''}{p.retired ? ` · ${p.retired} left for good` : ''}
      {#if p.objectingVoice}<span class="mono">objections weigh {voices(p.objectingVoice)} {p.objectingVoice === 1 ? 'voice' : 'voices'}; half a voice stops it</span>{/if}
    </p>
  {/if}

  {#if p.status === 'open'}
    <p class="pnote">
      {#if p.waiting.length}Waiting for {p.waiting.slice(0, 4).map((x) => (x.trust === 'me' ? 'you' : x.name)).join(', ')}{p.waiting.length > 4 ? ` and ${p.waiting.length - 4} more` : ''}.{/if}
      Answers close {fadesIn(p.deadline).replace(/^fades /, '')}; after that, silence counts as yes and only objections stop it.
    </p>
  {:else if p.status === 'in-effect'}
    <p class="pnote">Everyone still around agreed, so it applies now. {plural(p.away, 'person who is', 'people who are')} away can still object until answers close {fadesIn(p.deadline).replace(/^fades /, '')}.</p>
  {:else if p.status === 'passed'}
    <p class="pnote">{p.unanimous ? 'Everyone agreed' : 'Nobody objected in time'}{p.decidedAt ? ` · ${ago(p.decidedAt)}` : ''}. Signed answers are in the <a class="textlink" href={to.ledger(community)}>ledger</a>.</p>
  {:else}
    <p class="pnote">Objections stood when answers closed. Nothing changed.</p>
  {/if}

  {#if p.retirable?.length && !compact}
    <details class="retire">
      <summary>Someone left for good?</summary>
      <p class="pnote">These people decide this change but have signed nothing here for a month. If one of them has left for good, say so: after a week's notice their silence stops counting. Anything they sign here first undoes it, and members can veto it.</p>
      <ul class="objs">
        {#each p.retirable as x (x.key)}<li>{x.name} <button type="button" class="lnk lnk-ink" onclick={() => retire(x.key, x.name)}>Left for good</button></li>{/each}
      </ul>
    </details>
  {/if}

  {#if p.objecting.length && !compact}
    <ul class="objs">
      {#each p.objecting as o (o.person.key)}<li><b>{o.person.trust === 'me' ? 'You' : o.person.name}:</b> “{o.reason}”</li>{/each}
    </ul>
  {/if}

  {#if p.canAnswer}
    <div class="abtns">
      {#if p.myAnswer === 'agree'}
        <button type="button" class="pill pill-sm pill-ink" aria-pressed="true" onclick={() => answer('none')}>✓ You agree</button>
      {:else}
        <button type="button" class="pill pill-sm pill-ink" onclick={() => answer('ok')}>Agree</button>
      {/if}
      {#if p.myAnswer === 'object'}
        <button type="button" class="pill pill-sm" onclick={() => answer('none')}>Take back objection</button>
      {:else}
        <button type="button" class="pill pill-sm" aria-expanded={objecting} onclick={() => (objecting = !objecting)}>Object</button>
      {/if}
    </div>
    {#if objecting}
      <form class="obj" onsubmit={(e) => { e.preventDefault(); void answer('veto'); }}>
        <label class="field">
          <span>Why? Your objection is signed and public. Half a voice of objections stops the change.</span>
          <input class="input" bind:value={why} maxlength="200" placeholder="e.g. 60 days is too short for our slow threads" />
        </label>
        <div class="btnrow">
          <button class="pill pill-danger pill-sm" type="submit" disabled={!why.trim()}>Sign the objection</button>
          <button class="pill pill-sm" type="button" onclick={() => (objecting = false)}>Cancel</button>
        </div>
      </form>
    {/if}
  {:else if p.myAnswer && (p.status === 'open' || p.status === 'in-effect')}
    <p class="pnote">{p.by.some((x) => x.trust === 'me') ? 'You proposed it: your signature says yes.' : `You ${p.myAnswer === 'agree' ? 'agreed' : 'objected'}.`}</p>
  {/if}
</article>

<style>
  .prop { display: flex; flex-direction: column; gap: 8px; padding-bottom: 14px; }
  .head b { font-weight: 800; }
  .changes { list-style: none; margin: 4px 0 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .changes li { display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; font-weight: 600; }
  .lbl { min-width: 170px; color: var(--mute); font-weight: 600; }
  .from { text-decoration: line-through; text-decoration-color: var(--coral); color: var(--mute); }
  .rules { white-space: pre-wrap; font: inherit; font-weight: 600; line-height: 1.45; margin: 0; padding: 10px 12px; border: 2px dotted var(--line); border-radius: 12px; overflow-wrap: anywhere; }
  .tally { font-weight: 700; display: flex; gap: 8px; flex-wrap: wrap; align-items: baseline; }
  .pnote { font-size: 13px; color: var(--mute); line-height: 1.45; font-weight: 500; margin: 0; }
  .objs { margin: 0; padding-left: 18px; font-size: 14px; line-height: 1.45; }
  .abtns { display: flex; gap: 6px; margin-top: 4px; flex-wrap: wrap; }
  .obj { display: flex; flex-direction: column; gap: 8px; }
  .retire summary { cursor: pointer; font-weight: 700; font-size: 14px; }
  .retire[open] { display: flex; flex-direction: column; gap: 6px; }
  @media (max-width: 560px) {
    .lbl { min-width: 0; width: 100%; }
  }
</style>
