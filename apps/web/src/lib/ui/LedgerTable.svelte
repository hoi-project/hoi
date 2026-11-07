<script lang="ts">
  import type { LedgerEntryView, RulesView } from '@hoi/core-api';
  import { ago, duration, fadesIn, now } from '@hoi/core-api';
  import { ledgerSigned, ledgerTag, ledgerWho, limitsLine, proposalNote, rulesVersions, truncate } from '../ledger';
  import { voices } from '../util';
  import { to } from '../router';
  import { toast } from '../toast.svelte';
  import { app, attempt } from '../app.svelte';

  // `canVeto`: you are established here, so your veto counts (PROTOCOL.md §16.3).
  let { entries, rules, owners, canVeto = false }: { entries: LedgerEntryView[]; rules: RulesView | null; owners: number; canVeto?: boolean } = $props();
  const versions = $derived(rulesVersions(entries, rules?.version));
  let open = $state<Record<string, boolean>>({});
  let vetoing = $state<string | null>(null);
  let why = $state('');

  async function veto(e: LedgerEntryView, ev: SubmitEvent) {
    ev.preventDefault();
    const ok = await attempt(() => app.core.review(e.id, 'veto', why.trim()), `Veto signed. It takes ${voices(e.vetoesNeeded)} ${e.vetoesNeeded === 1 ? 'voice' : 'voices'} of vetoes to overturn it.`);
    if (ok !== undefined) {
      vetoing = null;
      why = '';
    }
  }

  function evidence(e: LedgerEntryView) {
    toast(`${e.evidence} signed flags and rate-limit receipts. All of them verify on this device.`);
  }
</script>

<div class="tbl">
  <table>
    <caption class="sr-only">Signed moderation actions, newest first</caption>
    <thead>
      <tr>
        <th scope="col" class="c-when">When</th>
        <th scope="col" class="c-what">What</th>
        <th scope="col" class="c-who">Who</th>
        <th scope="col">Target · reason</th>
        <th scope="col" class="c-signed">Signed</th>
      </tr>
    </thead>
    <tbody>
      {#each entries as e (e.id)}
        {@const tag = ledgerTag(e, versions.get(e.id))}
        <tr class={['anim', e.overturned && 'over']}>
          <td class="muted when" title={new Date(e.at * 1000).toLocaleString()}>{ago(e.at)}</td>
          <td><span class={['tag', tag.color === 'soft' ? 'tag-soft' : `tag-${tag.color}`]}>{tag.text}</span>{#if e.overturned}<span class="tag tag-peach overt">overturned</span>{/if}</td>
          <td>{ledgerWho(e)}</td>
          <td class="what">
            {#if e.target?.kind === 'record'}
              {#if e.target.author}<a class="plain" href={to.person(e.target.author.key)}>{e.target.author.name === e.target.author.short ? '' : e.target.author.name + ' '}<span class="mono">{e.target.author.short}</span></a>{:else}<span class="mono">{e.target.short}</span>{/if}
              {#if e.target.title}
                on {#if e.target.post}<a class="plain" href={to.post(e.target.post)}>“{truncate(e.target.title, 44)}”</a>{:else}“{truncate(e.target.title, 44)}”{/if}
              {:else if e.target.post && e.target.recordKind === 'comment'}
                · <a class="plain" href={to.post(e.target.post)}>in this thread</a>
              {/if}
            {:else if e.target?.kind === 'key'}
              <a class="plain" href={to.person(e.target.person.key)}>{e.target.person.name === e.target.person.short ? '' : e.target.person.name + ' '}<span class="mono">{e.target.person.short}</span></a>
            {/if}
            {#if e.action === 'limits' && e.limits}
              {limitsLine(e.limits)}
            {/if}
            {#if e.reason}{e.target || e.action === 'limits' ? ' · ' : ''}{e.reason}{/if}
            {#if e.action === 'rules' || e.action === 'limits' || e.action === 'mod-add' || e.action === 'mod-remove'}
              · {e.by.length} of {owners} signed
            {/if}
            {#if e.proposal}
              · <span class={['tag', e.proposal === 'lapsed' ? 'tag-soft' : e.proposal === 'open' ? 'tag-sun' : 'tag-mint']}>{proposalNote(e.proposal)}</span>
            {/if}
            <div class="sub">
              <span class="mono">{e.expiresAt === null ? 'kept until replaced' : fadesIn(e.expiresAt)}</span>
              {#if e.evidence > 0}<button type="button" class="lnk" onclick={() => evidence(e)}>See the evidence</button>{/if}
              {#if e.action === 'rules' && e.text}
                <button type="button" class="lnk" aria-expanded={!!open[e.id]} onclick={() => (open[e.id] = !open[e.id])}>{open[e.id] ? 'Hide' : 'Read them'}</button>
              {/if}
              {#if e.pendingUntil && !e.overturned}<span class="mono">text deleted in {duration(Math.max(0, e.pendingUntil - now()))} unless vetoed</span>{/if}
              {#if e.vetoable && (e.vetoes || e.overturned)}<span class="mono">{e.overturned ? 'overturned' : 'vetoes'}: {e.vetoes} {e.vetoes === 1 ? 'person' : 'people'}, {voices(e.vetoesVoice)} of {voices(e.vetoesNeeded)} {e.vetoesNeeded === 1 ? 'voice' : 'voices'}{e.support ? ` · ${e.support} support it` : ''}</span>{/if}
              {#if e.vetoable && canVeto && !e.overturned}
                {#if e.myReview === 'veto'}
                  <button type="button" class="lnk" onclick={() => attempt(() => app.core.review(e.id, 'none'), 'Veto taken back. Signed, too.')}>Take back your veto</button>
                {:else}
                  <button type="button" class="lnk" aria-expanded={vetoing === e.id} onclick={() => (vetoing = vetoing === e.id ? null : e.id)}>Veto</button>
                {/if}
              {/if}
            </div>
            {#if open[e.id] && e.text}<pre class="rulestext">{e.text}</pre>{/if}
            {#if vetoing === e.id}
              <form class="vetof" onsubmit={(ev) => veto(e, ev)}>
                <label class="field">
                  <span>Why was this wrong? Your veto is signed and public. Enough of them undo it, and a moderator overturned three times is suspended.</span>
                  <input class="input" bind:value={why} maxlength="200" placeholder="the post broke no rule" />
                </label>
                <div class="btnrow"><button class="pill pill-danger pill-sm" type="submit" disabled={!why.trim()}>Sign the veto</button><button class="pill pill-sm" type="button" onclick={() => (vetoing = null)}>Cancel</button></div>
              </form>
            {/if}
          </td>
          <td class="signed">{ledgerSigned(e)} ✓</td>
        </tr>
      {:else}
        <tr><td colspan="5" class="muted none">Nothing signed yet. The first mod action will show up here, for everyone.</td></tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  .c-when { width: 76px; }
  .c-what { width: 120px; }
  .c-who { width: 140px; }
  .c-signed { width: 96px; text-align: right; }
  .muted { color: var(--mute); }
  .over td { opacity: 0.75; }
  .overt { margin-left: 4px; }
  .vetof { display: flex; flex-direction: column; gap: 8px; margin-top: 10px; max-width: 520px; }
  .when { white-space: nowrap; }
  .what { line-height: 1.5; overflow-wrap: anywhere; }
  .plain:hover { color: var(--cobalt); }
  .sub { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; margin-top: 8px; }
  .signed { text-align: right; color: var(--cobalt); font-weight: 700; white-space: nowrap; }
  /* Phones: each entry becomes a stacked ledger row instead of a sideways-scrolling table. */
  @media (max-width: 640px) {
    .tbl table { min-width: 0; }
    thead { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); }
    tr {
      display: grid; grid-template-columns: auto minmax(0, 1fr) auto; gap: 6px 10px; padding: 14px 16px;
      grid-template-areas: 'what when signed' 'who who who' 'target target target'; border-top: 2px dotted var(--line);
    }
    tbody tr:first-child { border-top: 0; }
    td { padding: 0 !important; border: 0 !important; }
    td:nth-child(1) { grid-area: when; align-self: center; }
    td:nth-child(2) { grid-area: what; }
    td:nth-child(3) { grid-area: who; font-weight: 700; }
    td:nth-child(4) { grid-area: target; }
    td:nth-child(5) { grid-area: signed; align-self: center; }
    td.none { grid-column: 1 / -1; grid-row: 1; }
  }
  .rulestext { white-space: pre-wrap; font-family: var(--display); font-size: 14px; background: var(--soft); border-radius: 12px; padding: 10px 12px; margin: 10px 0 0; }
</style>
