<script lang="ts">
  // The operator's join notice, field by field: used by the kit page and the setup
  // app. The parent keeps the notice and shows the preview; this is the form.
  import type { NodeNotice } from '../notice';
  import { COUNTRIES, noticeOmissions } from '../notice';

  let {
    notice = $bindable(),
    retentionText = $bindable('7'),
    problem,
    idPrefix = 'nt',
  }: { notice: NodeNotice; retentionText?: string; problem: (field: string) => string | undefined; idPrefix?: string } = $props();

  // Who and where are the operator's to leave out; they see at once what people read then.
  const omissions = $derived(noticeOmissions(notice));
</script>

<div class="sets notice-sets">
  <div class="set top">
    <label class="l" for="{idPrefix}-operator">Operator <span class="opt">optional</span></label>
    <span class="h">
      Your name or a pseudonym, and how people reach you: an email address, a page, a handle. You should give them: they tell people who
      answers for their data and how to ask about it. You can leave them empty, and the notice then says you chose not to. Whatever you
      write here is shown to everyone who connects to this node, and travels in its link.
    </span>
    <span class="v col">
      <input id="{idPrefix}-operator" class="input" bind:value={notice.operator} maxlength="120" placeholder="name or pseudonym" autocomplete="off" />
      {#if problem('notice.operator')}<span class="help bad" role="alert">{problem('notice.operator')}</span>{/if}
      <input id="{idPrefix}-contact" class="input" bind:value={notice.contact} maxlength="200" placeholder="contact: an email address, a page, a handle" autocomplete="off" aria-label="Contact" />
      {#if problem('notice.contact')}<span class="help bad" role="alert">{problem('notice.contact')}</span>{/if}
    </span>
  </div>
  <div class="set top">
    <label class="l" for="{idPrefix}-country">Server <span class="opt">optional</span></label>
    <span class="h">
      The country the server is in, as its two-letter code, and who hosts it. You should give them: they tell people where their connection
      data is processed. Left empty, the notice says so. Whatever you write here is shown to everyone who connects to this node.
    </span>
    <span class="v col">
      <span class="fieldrow">
        <input id="{idPrefix}-country" class="input capin" bind:value={notice.country} list="{idPrefix}-countries" maxlength="2" placeholder="code" autocomplete="off" spellcheck="false" aria-label="Country code" style:text-transform="uppercase" />
        <datalist id="{idPrefix}-countries">{#each COUNTRIES as [code, label] (code)}<option value={code}>{label}</option>{/each}</datalist>
        <input id="{idPrefix}-provider" class="input" bind:value={notice.provider} maxlength="120" placeholder="hosting provider" autocomplete="off" aria-label="Hosting provider" />
      </span>
      {#if problem('notice.country')}<span class="help bad" role="alert">{problem('notice.country')}</span>{/if}
      {#if problem('notice.provider')}<span class="help bad" role="alert">{problem('notice.provider')}</span>{/if}
    </span>
  </div>
  {#if omissions.length}
    <p class="help left" role="status">
      Left out, and said so in your notice: {omissions.join('; ')}. That is your choice; fill them in above whenever you are ready.
    </p>
  {/if}
  <div class="set top">
    <span class="l">Logging</span>
    <span class="h">hoi-node keeps no log of who connects. Your server's own logs (sshd, a firewall) or your provider may; say so, and for how long they are kept.</span>
    <span class="v col">
      <label class="check"><input id="{idPrefix}-logging" type="checkbox" bind:checked={notice.ipLogging} /> IP addresses are logged on this server</label>
      {#if notice.ipLogging}
        <span class="fieldrow"><input id="{idPrefix}-retention" class="input capin" bind:value={retentionText} inputmode="numeric" aria-label="Days IP addresses are kept" /><span class="help">days, then deleted</span></span>
      {/if}
      {#if problem('notice.retentionDays')}<span class="help bad" role="alert">{problem('notice.retentionDays')}</span>{/if}
    </span>
  </div>
  <div class="set top">
    <label class="l" for="{idPrefix}-processes">What it processes</label>
    <span class="h">After the fixed sentence about addresses and connection data: anything else your node stores. Required.</span>
    <span class="v col">
      <textarea id="{idPrefix}-processes" class="textarea" rows="3" bind:value={notice.processes} maxlength="600"></textarea>
      {#if problem('notice.processes')}<span class="help bad" role="alert">{problem('notice.processes')}</span>{/if}
    </span>
  </div>
  <details class="set more">
    <summary class="l">Purpose, rules, more</summary>
    <div class="wide moregrid">
      <label class="field" for="{idPrefix}-purpose">
        <span>Purpose</span>
        <textarea id="{idPrefix}-purpose" class="textarea" rows="2" bind:value={notice.purpose} maxlength="400"></textarea>
        {#if problem('notice.purpose')}<span class="help bad" role="alert">{problem('notice.purpose')}</span>{/if}
      </label>
      <label class="field" for="{idPrefix}-rules">
        <span>Community rules (optional)</span>
        <textarea id="{idPrefix}-rules" class="textarea" rows="2" bind:value={notice.communityRules} maxlength="600" placeholder="moderation, age limits, what is welcome"></textarea>
        {#if problem('notice.communityRules')}<span class="help bad" role="alert">{problem('notice.communityRules')}</span>{/if}
      </label>
      <label class="field" for="{idPrefix}-extra">
        <span>More (optional)</span>
        <textarea id="{idPrefix}-extra" class="textarea" rows="2" bind:value={notice.extra} maxlength="600"></textarea>
        {#if problem('notice.extra')}<span class="help bad" role="alert">{problem('notice.extra')}</span>{/if}
      </label>
      <label class="field" for="{idPrefix}-updated">
        <span>Dated</span>
        <input id="{idPrefix}-updated" class="input capin" bind:value={notice.updated} placeholder="YYYY-MM-DD" autocomplete="off" />
        {#if problem('notice.updated')}<span class="help bad" role="alert">{problem('notice.updated')}</span>{/if}
      </label>
    </div>
  </details>
</div>

<style>
  .set { grid-template-columns: 160px minmax(0, 1fr) minmax(0, 1.3fr); }
  .set .v { justify-content: flex-start; text-align: left; }
  .col { flex-direction: column; align-items: stretch; gap: 8px; }
  .fieldrow { display: flex; gap: 8px; align-items: center; min-width: 0; }
  .fieldrow .input { flex: 1 1 160px; min-width: 0; }
  .check { display: flex; gap: 10px; align-items: center; font-weight: 700; min-height: 40px; }
  .check input { width: 20px; height: 20px; accent-color: var(--ink); flex: none; }
  .bad { color: var(--ink); font-weight: 700; box-shadow: inset 3px 0 0 var(--coral); padding-left: 10px; }
  .more { display: block; }
  .more summary { cursor: pointer; color: var(--cobalt); min-height: 44px; display: flex; align-items: center; }
  .moregrid { display: grid; grid-template-columns: minmax(0, 1.4fr) minmax(0, 1fr); gap: 16px; padding-top: 8px; }
  .capin { max-width: 140px; }
  .opt { display: block; font-weight: 600; font-size: 13px; color: var(--mute); }
  .left { margin: 0; padding: 10px 14px; box-shadow: inset 3px 0 0 var(--line); line-height: 1.5; }
  .notice-sets .textarea { width: 100%; }
  @media (max-width: 860px) { .set { grid-template-columns: minmax(0, 1fr); } .moregrid { grid-template-columns: minmax(0, 1fr); } }
</style>
