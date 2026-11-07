<script lang="ts">
  // Community management for owners and mods. Owner-only parts stay visible to mods, disabled, with the reason.
  import type { Admission, CommunityView, FlagView, LedgerResult, Limits } from '@hoi/core-api';
  import { ago, duration } from '@hoi/core-api';
  import { app, attempt } from '../lib/app.svelte';
  import { router } from '../lib/router.svelte';
  import { to } from '../lib/router';
  import {
    NEWCOMER_AGE_PRESETS,
    RETENTION_PRESETS,
    saveFile,
    formatMs,
    copyLimits,
    limitsEqual,
    parseKey,
    readFile,
    MAX_BUNDLE_FILE,
    reasonLabel,
    slug,
  } from '../lib/util';
  import { truncate } from '../lib/ledger';
  import { toast, toastError } from '../lib/toast.svelte';
  import Avatar from '../lib/ui/Avatar.svelte';
  import ProposalBox from '../lib/ui/ProposalBox.svelte';
  import ProposalCard from '../lib/ui/ProposalCard.svelte';
  import Switch from '../lib/ui/Switch.svelte';

  let { c, reload }: { c: CommunityView; reload: () => void } = $props();

  const owner = $derived(c.role === 'owner');
  const privateGroup = $derived(app.privateGroups.find(g => g.community === c.id));
  const ownerNote = $derived(
    owner
      ? c.threshold > 1
        ? `Owners propose (${c.threshold} of ${c.owners.length} sign); everyone established here decides.`
        : 'You propose; everyone established here decides. While nobody else is established, it applies at once.'
      : `Only owners propose changes (${c.threshold} of ${c.owners.length} sign). Members decide on the Rules tab.`,
  );
  const open = $derived(c.proposals.filter((p) => p.status === 'open' || p.status === 'in-effect'));

  // ---------------------------------------------------------- limits
  // svelte-ignore state_referenced_locally
  let draft = $state<Limits>(copyLimits(c.limits));
  let lastBase = $state.raw<Limits | null>(null);
  $effect(() => {
    // When the published limits change (after a signature lands), start from them again.
    if (!lastBase || !limitsEqual(lastBase, c.limits)) {
      draft = copyLimits(c.limits);
      lastBase = c.limits;
    }
  });
  const dirty = $derived(!limitsEqual(draft, c.limits));
  let limitsResult = $state.raw<LedgerResult | null>(null);
  let rulesResult = $state.raw<LedgerResult | null>(null);
  let modsResult = $state.raw<LedgerResult | null>(null);
  let cosignResult = $state.raw<LedgerResult | null>(null);

  const ADMISSIONS: { id: Admission; label: string; later?: boolean }[] = [
    { id: 'open', label: 'Open · stamp only' },
    { id: 'approve', label: 'A mod says yes' },
    { id: 'invite', label: 'Invite: a member vouches first' },
    { id: 'age-proof', label: 'Age proof' },
  ];

  function published(r: LedgerResult, what: string) {
    if (r.status === 'needs-cosign') {
      toast(`Signed by you. ${r.need - r.have} more owner ${r.need - r.have === 1 ? 'signature' : 'signatures'} needed.`);
    } else if (r.entry.proposal === 'open') {
      toast(`${what} proposed. They apply once everyone established here agrees; it's on the Rules tab.`);
    } else if (r.entry.proposal) {
      toast(`${what} in effect: nobody else is established here yet to ask.`);
    } else {
      toast(`${what} signed and published in the ledger.`);
    }
  }

  async function saveLimits(e: SubmitEvent) {
    e.preventDefault();
    const r = await attempt(() => app.core.setLimits(c.id, { ...draft, labelers: [...draft.labelers], trackers: [...draft.trackers], ageIssuers: [...(draft.ageIssuers ?? [])] }));
    if (r) {
      limitsResult = r;
      published(r, 'New limits');
    }
  }

  // ---------------------------------------------------------- rules
  // svelte-ignore state_referenced_locally
  let rulesDraft = $state(c.rules?.text ?? '');
  const rulesDirty = $derived(rulesDraft.trim() !== (c.rules?.text ?? '').trim());
  async function saveRules(e: SubmitEvent) {
    e.preventDefault();
    const r = await attempt(() => app.core.setRules(c.id, rulesDraft));
    if (r) {
      rulesResult = r;
      published(r, `Rules v${(c.rules?.version ?? 0) + 1}`);
    }
  }

  // ---------------------------------------------------------- mods
  let modKey = $state('');
  async function addMod(e: SubmitEvent) {
    e.preventDefault();
    const key = parseKey(modKey);
    if (!key) return toast("That isn't a key. Paste 64 hex characters or a did:key:z6Mk…");
    const r = await attempt(() => app.core.addMod(c.id, key));
    if (r) {
      modsResult = r;
      modKey = '';
      published(r, 'New moderator');
    }
  }
  async function removeMod(key: string, name: string) {
    const r = await attempt(() => app.core.removeMod(c.id, key));
    if (r) {
      modsResult = r;
      published(r, `${name} stepping down`);
    }
  }

  // ---------------------------------------------------------- door, bans, flags
  async function admit(key: string, name: string) {
    await attempt(() => app.core.admit(c.id, key), `${name} is in. Their posts show up for everyone now.`);
  }

  let banKey = $state('');
  let banReason = $state('');
  async function ban(e: SubmitEvent) {
    e.preventDefault();
    const key = parseKey(banKey);
    if (!key) return toast("That isn't a key. Paste 64 hex characters or a did:key:z6Mk…");
    const r = await attempt(() => app.core.ban(c.id, key, banReason), 'Banned. Signed into the ledger; every member drops their new posts.');
    if (r) {
      banKey = '';
      banReason = '';
    }
  }

  let unbanFor = $state<string | null>(null);
  let unbanReason = $state('appeal accepted');
  async function unban(e: SubmitEvent, key: string) {
    e.preventDefault();
    const r = await attempt(() => app.core.unban(c.id, key, unbanReason), 'Unbanned. Signed into the ledger.');
    if (r) unbanFor = null;
  }

  let actOn = $state<{ i: number; kind: 'remove' | 'ban' } | null>(null);
  let actReason = $state('');
  function startAct(i: number, kind: 'remove' | 'ban', f: FlagView) {
    actOn = { i, kind };
    actReason = f.reasons.map(reasonLabel).join(', ');
  }
  function flagAuthor(f: FlagView) {
    return f.target.kind === 'key' ? f.target.person : f.target.author;
  }
  async function doAct(e: SubmitEvent, f: FlagView) {
    e.preventDefault();
    if (!actOn) return;
    if (actOn.kind === 'remove' && f.target.kind === 'record') {
      const id = f.target.id;
      const r = await attempt(() => app.core.remove(id, actReason), 'Removed. The reason is signed into the ledger.');
      if (r) actOn = null;
    } else {
      const who = flagAuthor(f);
      if (!who) return toast('No key to ban on this one.');
      const r = await attempt(() => app.core.ban(c.id, who.key, actReason), `Banned ${who.name}. Signed into the ledger.`);
      if (r) actOn = null;
    }
  }

  // ---------------------------------------------------------- cosign
  let code = $state('');
  async function cosign(e: SubmitEvent) {
    e.preventDefault();
    const r = await attempt(() => app.core.cosign(code.trim()));
    if (!r) return;
    cosignResult = r;
    code = '';
    if (r.status === 'published') toast("Cosigned. Yours was the last signature it needed: it's published.");
    else toast(`Cosigned. ${r.need - r.have} more to go: pass the new code on.`);
  }

  // ---------------------------------------------------------- bundles, leaving
  async function exportBundle() {
    try {
      const data = await app.core.exportBundle(c.id);
      await saveFile(`c-${slug(c.name)}-${new Date().toISOString().slice(0, 10)}.hoibundle`, data);
      toast(`Bundle exported (${Math.max(1, Math.round(data.length / 1024))} KB). Every record in it is signed.`);
    } catch (err) {
      toastError(err);
    }
  }
  async function importBundle(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const f = input.files?.[0];
    if (!f) return;
    if (f.size > MAX_BUNDLE_FILE) {
      toast('That bundle is bigger than any hoi export. Pick a .hoibundle file.');
      input.value = '';
      return;
    }
    try {
      const r = await app.core.importBundle(await readFile(f));
      toast(`Checked and imported ${r.accepted} · ${r.duplicates} already here · ${r.rejected} refused`);
      reload();
    } catch (err) {
      toastError(err);
    }
    input.value = '';
  }

  let confirmLeave = $state(false);
  async function leave() {
    const ok = await attempt(() => app.core.leave(c.id), `Left c/${c.name}. Your copy fades on its own schedule.`);
    if (ok !== undefined) router.go('#/');
  }

  const retentionChoices = $derived(
    RETENTION_PRESETS.some((p) => p.seconds === draft.maxRetention)
      ? RETENTION_PRESETS
      : [...RETENTION_PRESETS, { seconds: draft.maxRetention, label: `${Math.round(draft.maxRetention / 86400)} d` }],
  );
  const ageChoices = $derived(
    NEWCOMER_AGE_PRESETS.some((p) => p.seconds === draft.newcomerAge)
      ? NEWCOMER_AGE_PRESETS
      : [...NEWCOMER_AGE_PRESETS, { seconds: draft.newcomerAge, label: `${Math.round(draft.newcomerAge / 86400)} d` }],
  );
  let ageCode = $state('');
  async function admitAge(e: SubmitEvent) {
    e.preventDefault();
    const r = await attempt(() => app.core.admitByAge(ageCode.trim()), 'They are in: the proof checked, and only "age proof" went into the ledger.');
    if (r) ageCode = '';
  }
</script>

<div class="manage">
  <!-- ================================================= flags -->
  <section class="anim" aria-labelledby="m-flags">
    <div class="sh"><h2 id="m-flags">Flags</h2><span class="hint">signed by members · weighted by your trust · auto-bounce at {c.limits.autobanThreshold}</span></div>
    {#if c.flags.length === 0}
      <p class="calm">No flags. Quiet is good.</p>
    {:else}
      <div class="rows">
        {#each c.flags as f, i (i)}
          {@const who = flagAuthor(f)}
          <div class="mrow">
            <div class="mmain">
              <div class="meta">
                {#each f.reasons as r (r)}
                  <span class={['tag', r === 'illegal' ? 'tag-coral' : r === 'spam' ? 'tag-peach' : 'tag-sun']}>{reasonLabel(r)}</span>
                {/each}
                <span>{f.count} {f.count === 1 ? 'flag' : 'flags'} · last {ago(f.lastAt)}</span>
              </div>
              <p class="what">
                {#if f.target.kind === 'record'}
                  {#if f.target.recordKind === 'comment'}A comment {/if}{#if f.target.title}{#if f.target.post}<a class="plain" href={to.post(f.target.post)}>“{truncate(f.target.title, 70)}”</a>{:else}“{truncate(f.target.title, 70)}”{/if}{:else}<span class="mono">{f.target.short}</span>{/if}
                  {#if who}{' '}· by <a class="plain" href={to.person(who.key)}>{who.name}</a>{#if who.name !== who.short}<span class="mono sp">{who.short}</span>{/if}{/if}
                {:else}
                  The key <a class="plain" href={to.person(f.target.person.key)}>{f.target.person.name}</a>{#if f.target.person.name !== f.target.person.short}<span class="mono sp">{f.target.person.short}</span>{/if}
                {/if}
              </p>
              <div class="weight">
                <span class="meter" role="img" aria-label={`weight ${f.weight} of ${c.limits.autobanThreshold}`}>
                  <i style:width="{Math.min(100, (f.weight / Math.max(1, c.limits.autobanThreshold)) * 100)}%"></i>
                </span>
                <span class="mono">weight {f.weight} of {c.limits.autobanThreshold}</span>
              </div>
              {#if f.reasons.includes('illegal')}
                <p class="callout">Flagged as illegal. A removal for this reason takes effect at once.</p>
              {/if}
              {#if actOn?.i === i}
                <form class="inset inl" onsubmit={(e) => doAct(e, f)}>
                  <label class="field">
                    <span>{actOn.kind === 'remove' ? 'Why remove it?' : `Why ban ${who?.name ?? 'this key'}?`} The reason is public in the ledger.</span>
                    <input class="input" bind:value={actReason} maxlength="200" />
                  </label>
                  <div class="btnrow">
                    <button class="pill pill-danger pill-sm" type="submit">{actOn.kind === 'remove' ? 'Remove & sign' : 'Ban & sign'}</button>
                    <button class="pill pill-sm" type="button" onclick={() => (actOn = null)}>Cancel</button>
                  </div>
                </form>
              {/if}
            </div>
            <div class="mact">
              {#if f.target.kind === 'record'}<button type="button" class="lnk lnk-danger" onclick={() => startAct(i, 'remove', f)}>Remove it</button>{/if}
              {#if who}<button type="button" class="lnk lnk-ink" onclick={() => startAct(i, 'ban', f)}>Ban the author</button>{/if}
            </div>
          </div>
        {/each}
      </div>
      <p class="help">Flags can't be dismissed. They fade with time like everything else: act on them, or leave them be.</p>
    {/if}
  </section>

  <!-- ================================================= door -->
  {#if c.limits.admission === 'age-proof'}
    <section class="anim" aria-labelledby="m-age-proofs">
      <div class="sh"><h2 id="m-age-proofs">Age proofs</h2><span class="hint">a newcomer's code: checked here, never published</span></div>
      <form class="ageform" onsubmit={admitAge}>
        <label class="field" for="m-agecode"><span>Their code</span><input id="m-agecode" class="input mono-in" bind:value={ageCode} placeholder="hoi-age-1-…" spellcheck="false" autocapitalize="none" /></label>
        <button type="submit" class="pill pill-ink pill-sm" disabled={!ageCode.trim()}>Check and admit</button>
      </form>
    </section>
  {/if}
  {#if c.limits.admission === 'approve' || c.pending.length}
    <section class="anim" aria-labelledby="m-door">
      <div class="sh"><h2 id="m-door">Waiting at the door</h2><span class="hint">their posts only show for mods until someone admits them</span></div>
      {#if c.pending.length === 0}
        <p class="calm">Nobody waiting.</p>
      {:else}
        <div class="rows">
          {#each c.pending as p (p.key)}
            <div class="mrow">
              <div class="mmain who"><Avatar name={p.name} keyHex={p.key} avatar={p.avatar} avatarColor={p.avatarColor} /><span><a class="plain strong" href={to.person(p.key)}>{p.name}</a>{#if p.name !== p.short}<span class="mono sp">{p.short}</span>{/if}<br /><span class="hint">key first seen {p.keyAgeDays === 0 ? 'today' : `${p.keyAgeDays} d ago`}</span></span></div>
              <div class="mact"><button type="button" class="pill pill-ink pill-sm" onclick={() => admit(p.key, p.name)}>Admit</button></div>
            </div>
          {/each}
        </div>
      {/if}
    </section>
  {/if}

  <!-- ================================================= bans -->
  <section class="anim" aria-labelledby="m-bans">
    <div class="sh"><h2 id="m-bans">Banned keys</h2><span class="hint">members' devices drop their new records here</span></div>
    {#if c.banned.length}
      <div class="rows">
        {#each c.banned as b (b.person.key)}
          <div class="mrow">
            <div class="mmain">
              <div class="meta">
                <a class="plain strong" href={to.person(b.person.key)}>{b.person.name}</a>{#if b.person.name !== b.person.short}<span class="mono">{b.person.short}</span>{/if}
                {#if b.auto}<span class="tag tag-peach">bounced automatically</span>{/if}
                <span>· {ago(b.at)}</span>
              </div>
              <p class="what">{b.reason}</p>
              {#if unbanFor === b.person.key}
                <form class="inset inl" onsubmit={(e) => unban(e, b.person.key)}>
                  <label class="field"><span>Why unban? Public in the ledger.</span><input class="input" bind:value={unbanReason} maxlength="200" /></label>
                  <div class="btnrow">
                    <button class="pill pill-ink pill-sm" type="submit">Unban &amp; sign</button>
                    <button class="pill pill-sm" type="button" onclick={() => (unbanFor = null)}>Cancel</button>
                  </div>
                </form>
              {/if}
            </div>
            <div class="mact"><button type="button" class="lnk lnk-ink" onclick={() => (unbanFor = b.person.key)}>Unban</button></div>
          </div>
        {/each}
      </div>
    {:else}
      <p class="calm">Nobody banned.</p>
    {/if}
    <form class="inset banform" onsubmit={ban}>
      <span class="label">Ban a key</span>
      <div class="two">
        <label class="field"><span class="sr-only">Key to ban</span><input class="input mono-in" bind:value={banKey} placeholder="key (hex) or did:key:z6Mk…" autocomplete="off" /></label>
        <label class="field"><span class="sr-only">Reason</span><input class="input" bind:value={banReason} placeholder="reason, e.g. rule 2, doxxing" maxlength="200" /></label>
      </div>
      <div class="btnrow"><button class="pill pill-danger pill-sm" type="submit" disabled={!banKey.trim() || !banReason.trim()}>Ban &amp; sign</button></div>
    </form>
  </section>

  <!-- ================================================= limits -->
  <section class="anim" aria-labelledby="m-limits">
    <div class="sh"><h2 id="m-limits">Limits</h2><span class="hint">{ownerNote}</span></div>
    <form onsubmit={saveLimits}>
      <fieldset class="sets" disabled={!owner}>
        <div class="set">
          <label class="l" for="m-pow">Stamp</label>
          <span class="h">Higher is slower to post and harder to flood. Keys older than 90 days get a discount.</span>
          <span class="v slider">
            <input id="m-pow" type="range" min="12" max="26" step="1" bind:value={draft.powBits} />
            <span class="num">{draft.powBits} bits · ≈ {formatMs(app.stampMs(draft.powBits))}</span>
          </span>
        </div>
        <div class="set">
          <span class="l" id="m-ret">Posts fade after</span>
          <span class="h">Members' devices delete records older than this. Nobody can make them stay.</span>
          <span class="v">
            <span class="seg" role="radiogroup" aria-labelledby="m-ret">
              {#each retentionChoices as p (p.seconds)}
                <label><input type="radio" name="m-ret" value={p.seconds} bind:group={draft.maxRetention} /><span>{p.label}</span></label>
              {/each}
            </span>
          </span>
        </div>
        <div class="set">
          <label class="l" for="m-npd">Newcomer limit</label>
          <span class="h">Posts per day for keys younger than the newcomer age.</span>
          <span class="v">
            <select id="m-npd" class="select" bind:value={draft.newcomerPostsPerDay}>
              {#each [0, 1, 2, 3, 5, 10, 25] as n (n)}<option value={n}>{n} / day</option>{/each}
              {#if ![0, 1, 2, 3, 5, 10, 25].includes(draft.newcomerPostsPerDay)}<option value={draft.newcomerPostsPerDay}>{draft.newcomerPostsPerDay} / day</option>{/if}
            </select>
          </span>
        </div>
        <div class="set">
          <span class="l" id="m-age">Newcomer age</span>
          <span class="h">How long a key counts as new here.</span>
          <span class="v">
            <span class="seg" role="radiogroup" aria-labelledby="m-age">
              {#each ageChoices as p (p.seconds)}
                <label><input type="radio" name="m-age" value={p.seconds} bind:group={draft.newcomerAge} /><span>{p.label}</span></label>
              {/each}
            </span>
          </span>
        </div>
        <div class="set">
          <span class="l" id="m-adm">Joining</span>
          <span class="h">What a new key must do before its posts show up. With invites, a member's vouch is the invite; a member who vouches in many people at once gives each a smaller voice. With age proofs, a moderator lets people in after seeing a wallet's proof that they are over 18, and nothing else about them.</span>
          <span class="v">
            <span class="seg" role="radiogroup" aria-labelledby="m-adm">
              {#each ADMISSIONS as a (a.id)}
                <label title={a.later ? 'coming later' : undefined}>
                  <input type="radio" name="m-adm" value={a.id} bind:group={draft.admission} disabled={a.later} />
                  <span>{a.label}{a.later ? ' · later' : ''}</span>
                </label>
              {/each}
            </span>
          </span>
        </div>
        <div class="set">
          <label class="l" for="m-auto">Auto-bounce</label>
          <span class="h">Ban a key when enough trusted members flag it, without waiting for a mod. Mods can undo it.</span>
          <span class="v slider">
            <input id="m-auto" type="range" min="3" max="50" step="1" bind:value={draft.autobanThreshold} />
            <span class="num">after {draft.autobanThreshold} flags</span>
          </span>
        </div>
        {#if !privateGroup}
          <div class="set">
            <span class="l">Posts in pieces</span>
            <span class="h">Posts, replies and edits are cut into pieces kept by many independent nodes; only small sealed pointers travel whole. No node that stores them knows who posted, and no single node can stop one. Posts stay public: anyone who joins reads them.</span>
            <span class="v"><Switch checked={draft.scatterOpen} label="Posts in pieces" onchange={(v) => { draft.scatterOpen = v; }} /></span>
          </div>
        {/if}
        <div class="set">
          <label class="l" for="m-scatter">Pieces</label>
          <span class="h">Whoever runs up to this share of the places a post's pieces go to can neither put one together nor keep it from members. Higher sends more pieces per post. 10% suits most communities.</span>
          <span class="v"><select id="m-scatter" class="select" bind:value={draft.scatterResist}>{#each [10, 15, 20] as n (n)}<option value={n}>{n}% of places</option>{/each}</select></span>
        </div>
        {#if draft.admission === 'age-proof'}
          <div class="set">
            <label class="l" for="m-issuers">Trusted issuers</label>
            <span class="h">The keys of the identity wallets' issuers whose age credentials you take (hex, one a line, at most 8): P-256 (ES256) or Ed25519. Ask the issuer for its signing key.</span>
            <span class="v"><textarea id="m-issuers" class="input mono-in" rows="2" placeholder="04a1b2…" value={(draft.ageIssuers ?? []).join('\n')}
              oninput={(e) => (draft.ageIssuers = e.currentTarget.value.split('\n').map((t) => t.trim()).filter(Boolean).slice(0, 8))}></textarea></span>
          </div>
        {/if}
        {#if !privateGroup}
          <div class="set">
            <label class="l" for="m-labelers">Default labelers</label>
            <span class="h">Keys of people whose labels count for everyone here, without following them (one a line, at most 32). Spam, nsfw and off-topic labels put posts to sleep; members can still peek.</span>
            <span class="v"><textarea id="m-labelers" class="input mono-in" rows="2" placeholder="their key (64 hex)" value={draft.labelers.join('\n')}
              oninput={(e) => (draft.labelers = e.currentTarget.value.split('\n').map((t) => t.trim()).filter(Boolean).slice(0, 32))}></textarea></span>
          </div>
        {/if}
        {#if !privateGroup && draft.admission === 'open'}
          <div class="set">
            <label class="l" for="m-trackers">Trackers</label>
            <span class="h">Up to two registries (https://…, one a line) where public nodes that carry this community say so. A device asks them only when no node it knows answers, and the tracker then sees its address and this community. None is fine: nodes pass each other on.</span>
            <span class="v"><textarea id="m-trackers" class="input mono-in" rows="2" placeholder="https://tracker.example" value={draft.trackers.join('\n')}
              oninput={(e) => (draft.trackers = e.currentTarget.value.split('\n').map((t) => t.trim()).filter(Boolean).slice(0, 2))}></textarea></span>
          </div>
        {/if}
      </fieldset>
      <fieldset class="sets" disabled={!owner}>
        <legend class="sh2">The ladder <span class="hint">how newcomers earn their way up, and how members, not just mods, decide · quorums count voices, never exceed a third of the established voice ({c.establishedVoice.toFixed(1)}), and need two people at least</span></legend>
        <div class="set">
          <label class="l" for="m-idb">Key work</label>
          <span class="h">Memory-hard work every key here must carry. It makes minting many keys expensive. Keep it at or below your own key's {app.me?.identityBits ?? 0} bits.</span>
          <span class="v slider">
            <input id="m-idb" type="range" min="4" max="16" step="1" bind:value={draft.ladder.identityBits} />
            <span class="num">{draft.ladder.identityBits} bits</span>
          </span>
        </div>
        <div class="set">
          <label class="l" for="m-adv">Vouches to get in</label>
          <span class="h">Established members who vouch (reading without hiding counts) before a newcomer can share links and their votes count.</span>
          <span class="v"><select id="m-adv" class="select" bind:value={draft.ladder.admitVouches}>{#each [1, 2, 3, 4, 5] as n (n)}<option value={n}>{n}</option>{/each}</select></span>
        </div>
        <div class="set">
          <label class="l" for="m-est">Established after</label>
          <span class="h">Time as a member before someone's vouches, hides and vetoes count.</span>
          <span class="v"><select id="m-est" class="select" bind:value={draft.ladder.establishedAge}>
            {#each [0, 1, 3, 7, 14, 30, 90] as d (d)}<option value={d * 86_400}>{d === 0 ? 'right away' : `${d} days`}</option>{/each}
          </select></span>
        </div>
        <div class="set">
          <label class="l" for="m-tp">Media for</label>
          <span class="h">Established members with this many posts or comments in the last 90 days, and this much response (upvotes and "looks good", minus hides), may share images and other media.</span>
          <span class="v pair2">
            <input id="m-tp" class="input num" type="number" min="0" max="1000" bind:value={draft.ladder.trustedPosts} aria-label="posts needed" />
            <input class="input num" type="number" min="0" max="100000" bind:value={draft.ladder.mediaScore} aria-label="response needed" />
          </span>
        </div>
        <div class="set">
          <label class="l" for="m-hq">Voices to nap a post</label>
          <span class="h">Established members hiding it (with a reason, outweighing those who say it's fine 2:1) put it to sleep for everyone. Anyone can still peek.</span>
          <span class="v slider"><input id="m-hq" type="range" min="2" max="20" step="1" bind:value={draft.ladder.hideQuorum} /><span class="num">{draft.ladder.hideQuorum}</span></span>
        </div>
        <div class="set">
          <label class="l" for="m-bq">Voices to block</label>
          <span class="h">Established members hiding someone's posts or objecting to them block them from the community. It lifts as their reasons fade.</span>
          <span class="v slider"><input id="m-bq" type="range" min="2" max="30" step="1" bind:value={draft.ladder.blockQuorum} /><span class="num">{draft.ladder.blockQuorum}</span></span>
        </div>
        <div class="set">
          <label class="l" for="m-vq">Voices to overturn</label>
          <span class="h">Established members' vetoes undo a moderator's or owner's action. A moderator overturned three times is suspended.</span>
          <span class="v slider"><input id="m-vq" type="range" min="2" max="30" step="1" bind:value={draft.ladder.vetoQuorum} /><span class="num">{draft.ladder.vetoQuorum}</span></span>
        </div>
        <div class="set">
          <label class="l" for="m-cw">Removals wait</label>
          <span class="h">A removed post naps this long, open to vetoes, before its text is deleted. Removals for illegal content or private data take effect at once.</span>
          <span class="v"><select id="m-cw" class="select" bind:value={draft.ladder.contestWindow}>
            {#each [0, 3600, 86_400, 259_200, 604_800, 1_209_600] as w (w)}<option value={w}>{w === 0 ? 'no wait' : duration(w)}</option>{/each}
          </select></span>
        </div>
      </fieldset>
      {#if owner}
        <div class="btnrow savebar">
          <button class="pill pill-ink" type="submit" disabled={!dirty}>{c.threshold > 1 ? 'Sign & ask the other owners' : 'Propose new limits'}</button>
          {#if dirty}<button class="pill" type="button" onclick={() => (draft = copyLimits(c.limits))}>Undo changes</button>{/if}
        </div>
      {/if}
    </form>
    {#if limitsResult?.status === 'needs-cosign'}
      <ProposalBox code={limitsResult.proposal} have={limitsResult.have} need={limitsResult.need} owners={c.owners.length} />
    {/if}
    {#if open.length}
      <div class="openprops">
        <span class="label">Being decided</span>
        {#each open as p (p.id)}<ProposalCard {p} community={c.id} compact />{/each}
      </div>
    {/if}
  </section>

  <!-- ================================================= rules -->
  <section class="anim" aria-labelledby="m-rules">
    <div class="sh"><h2 id="m-rules">Rules</h2><span class="hint">{c.rules ? `now v${c.rules.version}` : 'none yet'} · plain text, one numbered rule per line</span></div>
    <form class="rulesform" onsubmit={saveRules}>
      <label class="sr-only" for="m-rtext">Rules</label>
      <textarea id="m-rtext" class="textarea" rows="7" bind:value={rulesDraft} disabled={!owner}
        placeholder={'1. Be kind. Argue with ideas, not people.\n2. No doxxing.\n3. Say so if you sell something.'}></textarea>
      {#if owner}
        <div class="btnrow"><button class="pill pill-ink" type="submit" disabled={!rulesDirty || !rulesDraft.trim()}>Propose rules v{(c.rules?.version ?? 0) + 1}</button></div>
      {:else}
        <p class="help">{ownerNote}</p>
      {/if}
    </form>
    {#if rulesResult?.status === 'needs-cosign'}
      <ProposalBox code={rulesResult.proposal} have={rulesResult.have} need={rulesResult.need} owners={c.owners.length} />
    {/if}
  </section>

  <!-- ================================================= people -->
  <section class="anim" aria-labelledby="m-mods">
    <div class="sh"><h2 id="m-mods">Moderators</h2><span class="hint">they remove, pin, label, ban and admit · every action is signed and public</span></div>
    <div class="rows">
      {#each c.mods as m (m.key)}
        <div class="mrow">
          <div class="mmain who">
            <Avatar name={m.name} keyHex={m.key} avatar={m.avatar} avatarColor={m.avatarColor} mine={m.trust === 'me'} />
            <span><a class="plain strong" href={to.person(m.key)}>{m.name}</a>{#if m.name !== m.short}<span class="mono sp">{m.short}</span>{/if}{#if m.trust === 'me'}<span class="tag tag-sun sp">you</span>{/if}</span>
          </div>
          <div class="mact">
            {#if owner}<button type="button" class="lnk lnk-ink" onclick={() => removeMod(m.key, m.name)}>Remove</button>{/if}
          </div>
        </div>
      {/each}
    </div>
    <form class="inset addmod" onsubmit={addMod}>
      <label class="field">
        <span>Add a moderator</span>
        <input class="input mono-in" bind:value={modKey} placeholder="their key (64 hex) or did:key:z6Mk…" disabled={!owner} autocomplete="off" />
        <span class="help">Ask them for the key on their You page. Names can be anything; keys can't be faked.</span>
      </label>
      <div class="btnrow"><button class="pill pill-ink pill-sm" type="submit" disabled={!owner || !modKey.trim()}>Sign &amp; add</button></div>
      {#if !owner}<p class="help">{ownerNote}</p>{/if}
    </form>
    {#if modsResult?.status === 'needs-cosign'}
      <ProposalBox code={modsResult.proposal} have={modsResult.have} need={modsResult.need} owners={c.owners.length} />
    {/if}

    <div class="owners">
      <span class="label">Owners</span>
      <div class="meta">
        {#each c.owners as o (o.key)}
          <a class="tag tag-soft" href={to.person(o.key)}>{o.trust === 'me' ? 'you' : o.name}</a>
        {/each}
        <span>· {c.threshold} of {c.owners.length} sign moderator changes and propose rules and limits</span>
      </div>
    </div>
  </section>

  <!-- ================================================= cosign -->
  <section class="anim" aria-labelledby="m-cosign">
    <div class="sh"><h2 id="m-cosign">Cosign a proposal</h2><span class="hint">for owners · paste a code another owner sent you</span></div>
    <form class="inset" onsubmit={cosign}>
      <label class="field">
        <span>Proposal code</span>
        <input class="input mono-in" bind:value={code} placeholder="hoi://cosign?p=…" disabled={!owner} autocomplete="off" />
        <span class="help">If yours is the last signature it needs, it publishes. Otherwise you get an updated code to pass on.</span>
      </label>
      <div class="btnrow"><button class="pill pill-ink pill-sm" type="submit" disabled={!owner || !code.trim()}>Cosign</button></div>
    </form>
    {#if cosignResult?.status === 'needs-cosign'}
      <ProposalBox code={cosignResult.proposal} have={cosignResult.have} need={cosignResult.need} owners={c.owners.length} />
    {/if}
  </section>

  <!-- ================================================= bundles -->
  {#if !app.privateGroups.some(g => g.community === c.id)}
  <section class="anim" aria-labelledby="m-bundle">
    <div class="sh"><h2 id="m-bundle">Carry it by hand</h2><span class="hint">a signed bundle of everything still live here</span></div>
    <div class="sets">
      <div class="set">
        <span class="l">Export a bundle</span>
        <span class="h">Every post, reply and ledger entry still live, signed. Hand it to a member on a stick; their device checks every record.</span>
        <span class="v"><button type="button" class="lnk lnk-ink" onclick={exportBundle}>Download .hoibundle</button></span>
      </div>
      <div class="set">
        <label class="l" for="m-imp">Import a bundle</label>
        <span class="h">Records that don't verify, or that already faded, are refused.</span>
        <span class="v"><input id="m-imp" type="file" accept=".hoibundle,application/octet-stream" onchange={importBundle} /></span>
      </div>
    </div>
  </section>

  <!-- ================================================= danger -->
  {/if}
  <section class="anim danger" aria-labelledby="m-danger">
    <div class="sh"><h2 id="m-danger">Danger zone</h2></div>
    <div class="sets">
      <div class="set">
        <span class="l">Leave c/{c.name}</span>
        <span class="h">{#if privateGroup?.isOwner}Keep this owner device available to manage invitations and renew member access.{:else if privateGroup}Leaving deletes this device's private group keys. Ask the owner to remove its membership too. Returning requires a new invitation.{:else}Your {app.platformName} stops syncing it. Your copy fades on schedule.{owner ? ' You stay an owner: your key is in its genesis.' : ''}{/if}</span>
        <span class="v">
          {#if confirmLeave}
            <button type="button" class="pill pill-danger pill-sm" onclick={leave}>Yes, leave</button>
            <button type="button" class="pill pill-sm" onclick={() => (confirmLeave = false)}>Stay</button>
          {:else}
            <button type="button" class="lnk lnk-danger" disabled={privateGroup?.isOwner} onclick={() => (confirmLeave = true)}>Leave</button>
          {/if}
        </span>
      </div>
    </div>
  </section>
</div>

<style>
  .manage { display: flex; flex-direction: column; gap: 40px; padding-top: 10px; }
  .sh { display: flex; align-items: baseline; gap: 14px; flex-wrap: wrap; padding-bottom: 12px; }
  .sh2 { display: flex; align-items: baseline; gap: 14px; flex-wrap: wrap; padding: 22px 0 10px; font-size: 18px; font-weight: 800; letter-spacing: -0.02em; }
  .pair2 { display: flex; gap: 8px; }
  .pair2 .input { width: 96px; }
  .sh h2 { font-size: 26px; font-weight: 800; letter-spacing: -0.03em; }
  .calm { color: var(--mute); font-weight: 600; padding: 14px 0; border-top: 2px solid var(--ink); }
  .mrow { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 16px; padding: 14px 0; border-top: 2px dotted var(--line); align-items: start; }
  .mmain { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
  .mmain.who { flex-direction: row; align-items: center; gap: 12px; }
  .mact { display: flex; gap: 6px; flex-wrap: wrap; justify-content: flex-end; }
  .what { font-weight: 600; overflow-wrap: anywhere; }
  .plain:hover { color: var(--cobalt); }
  .strong { font-weight: 700; }
  .sp { margin-left: 6px; }
  .weight { display: flex; align-items: center; gap: 10px; }
  .meter { width: 140px; height: 6px; border-radius: 999px; background: var(--soft); overflow: hidden; flex: 0 0 auto; }
  .meter i { display: block; height: 100%; background: var(--coral); border-radius: 999px; }
  .inl { margin-top: 6px; max-width: 560px; }
  .banform, .addmod { margin-top: 14px; }
  .two { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: 8px; }
  fieldset { border: 0; margin: 0; padding: 0; min-width: 0; }
  fieldset:disabled .set { opacity: .6; }
  fieldset .set { grid-template-columns: 170px minmax(0, 1fr) minmax(0, 1.25fr); }
  .slider { flex-wrap: nowrap; }
  .slider input { width: 150px; }
  .savebar { padding-top: 14px; }
  .rulesform { display: flex; flex-direction: column; gap: 10px; }
  .rulesform textarea { max-width: 720px; }
  .owners { display: flex; flex-direction: column; gap: 8px; margin-top: 18px; }
  .openprops { display: flex; flex-direction: column; gap: 10px; margin-top: 18px; }
  .danger .sh h2 { text-decoration: underline; text-decoration-color: var(--coral); text-decoration-thickness: 4px; text-underline-offset: 6px; }
  @media (max-width: 860px) {
    fieldset .set { grid-template-columns: 1fr; }
    .slider { flex-wrap: wrap; }
    .slider input { width: 100%; }
  }
  @media (max-width: 560px) {
    .mrow { grid-template-columns: 1fr; }
    .mact { justify-content: flex-start; }
    .two { grid-template-columns: 1fr; }
  }
</style>
