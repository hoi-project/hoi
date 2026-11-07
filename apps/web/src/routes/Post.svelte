<script lang="ts">
  import LinkCard from '../lib/ui/LinkCard.svelte';
  import type { CommentView } from '@hoi/core-api';
  import { ago, fadesIn } from '@hoi/core-api';
  import { app, attempt, setTitle } from '../lib/app.svelte';
  import { resource } from '../lib/resource.svelte';
  import { enterView, settle } from '../lib/motion';
  import { router } from '../lib/router.svelte';
  import { to } from '../lib/router';
  import { clock, copyText, didFromKeySafe, keyAge, napWho, paragraphs, plural, safeHttpUrl, shortDid } from '../lib/util';
  import { toast, toastError } from '../lib/toast.svelte';
  import Avatar from '../lib/ui/Avatar.svelte';
  import TrustTag from '../lib/ui/TrustTag.svelte';
  import Vote from '../lib/ui/Vote.svelte';
  import ReplyBox from '../lib/ui/ReplyBox.svelte';
  import Comment from '../lib/ui/Comment.svelte';
  import FlagPicker from '../lib/ui/FlagPicker.svelte';
  import ReviewPicker from '../lib/ui/ReviewPicker.svelte';
  import MediaList from '../lib/ui/MediaList.svelte';
  import StandingCard from '../lib/ui/StandingCard.svelte';
  import SyncNotice from '../lib/ui/SyncNotice.svelte';

  let { id }: { id: string } = $props();

  const post = resource(() => {
    void app.rev;
    return app.core.post(id);
  });
  const p = $derived(post.value);
  const privateGroup = $derived(app.privateGroups.find(g => g.community === p?.community.id));
  const writable = $derived(!privateGroup || privateGroup.active);
  const community = resource(() => {
    void app.rev;
    const cid = post.value?.community.id;
    return cid ? app.core.community(cid) : Promise.resolve(null);
  });
  const cv = $derived(community.value);
  const canMod = $derived(!!cv && cv.role !== 'member');
  // established members review: their hides and "looks good" count (PROTOCOL.md §16)
  const canReview = $derived(!!cv?.me?.capabilities.includes('review'));
  const authorStanding = resource(() => {
    void app.rev;
    const v = post.value;
    return v && !v.mine ? app.core.standing(v.community.id, v.author.key) : Promise.resolve(null);
  });
  const url = $derived(safeHttpUrl(p?.link));
  const napCount = $derived(p ? countNapping(p.comments) : 0);
  const totalReplies = $derived(p ? countAll(p.comments) : 0);

  let peek = $state(false);
  let flagging = $state(false);
  let hiding = $state(false);
  let freshId = $state<string | null>(null);
  let tool = $state<'' | 'remove' | 'label' | 'tag' | 'ban' | 'edit' | 'delete'>('');
  let reason = $state('');
  let labelText = $state('');
  let editTitle = $state('');
  let editText = $state('');
  let petname = $state('');
  let editingPet = $state(false);
  let toolBox: HTMLElement | undefined = $state();

  $effect(() => setTitle(p ? (p.removed ? 'Poof.' : p.title) : 'Post'));
  $effect(() => {
    if (tool) settle(toolBox ?? null);
  });

  function countNapping(list: CommentView[]): number {
    return list.reduce((n, c) => n + (c.napping ? 1 : 0) + countNapping(c.children), 0);
  }
  function countAll(list: CommentView[]): number {
    return list.reduce((n, c) => n + (c.removed ? 0 : 1) + countAll(c.children), 0);
  }

  function replied(c: CommentView) {
    freshId = c.id;
  }

  async function share() {
    if (!p) return;
    try {
      const inv = await app.core.invite(p.community.id);
      if (await copyText(inv.web)) toast(`Invite link to c/${p.community.name} copied`);
    } catch (e) {
      toastError(e);
    }
  }
  async function toggleSave() {
    if (!p) return;
    await attempt(() => app.core.save(p.id, !p.saved), !p.saved ? `Saved on this ${app.platformName}` : 'Unsaved');
  }

  function open(t: typeof tool) {
    tool = tool === t ? '' : t;
    reason = '';
    labelText = '';
    if (t === 'edit' && p) {
      editTitle = p.title;
      editText = p.text;
    }
  }

  async function doTool(e: SubmitEvent) {
    e.preventDefault();
    if (!p) return;
    let ok: unknown;
    switch (tool) {
      case 'remove':
        ok = await attempt(() => app.core.remove(p.id, reason), 'Removed. Poof, with the reason signed into the ledger.');
        break;
      case 'label':
        ok = await attempt(() => app.core.label(p.id, labelText, reason || undefined), 'Labelled. Members see it napping, and can peek.');
        break;
      case 'tag':
        ok = await attempt(() => app.core.publishLabel(p.id, labelText, reason || undefined), 'Labelled for whoever follows your labels.');
        break;
      case 'ban':
        ok = await attempt(() => app.core.ban(p.community.id, p.author.key, reason), `Banned ${p.author.name} in c/${p.community.name}. Signed into the ledger.`);
        break;
      case 'edit':
        ok = await attempt(() => app.core.edit(p.id, editText, editTitle), 'Edit signed. Everyone sees the new version, marked edited.');
        break;
      case 'delete':
        ok = await attempt(() => app.core.deleteOwn(p.id), 'Deleted. Peers drop the body and keep a tombstone.');
        if (ok !== undefined) router.go(to.community(p.community.id));
        break;
    }
    if (ok !== undefined) tool = '';
  }

  async function pinToggle() {
    if (!p) return;
    await attempt(() => app.core.pin(p.id, !p.pinned), p.pinned ? 'Unpinned. Signed into the ledger.' : 'Pinned to the top. Signed into the ledger.');
  }

  async function trust() {
    if (!p) return;
    const friend = p.author.trust !== 'friend';
    await attempt(() => app.core.setTrust(p.author.key, friend), friend ? `You trust ${p.author.name} now. Their votes and flags count more for you.` : `${p.author.name} is no longer a friend on this device.`);
  }
  async function mute() {
    if (!p) return;
    const m = !p.author.muted;
    await attempt(() => app.core.setMuted(p.author.key, m), m ? `Muted ${p.author.name} on this device.` : `Unmuted ${p.author.name}.`);
  }
  async function savePet(e: SubmitEvent) {
    e.preventDefault();
    if (!p) return;
    const ok = await attempt(() => app.core.setPetname(p.author.key, petname.trim() || null), petname.trim() ? 'Saved. Only you see that name.' : 'Back to the name they chose.');
    if (ok) editingPet = false;
  }

  function relation(t: string): string {
    switch (t) {
      case 'me': return "that's you";
      case 'friend': return 'friends';
      case 'fof': return 'a friend of a friend';
      case 'member': return 'both members here';
      default: return 'new key';
    }
  }
</script>

<SyncNotice community={p?.community.id} />

{#if post.error && !p}
  <div class="empty">
    <h2>That post isn't here.</h2>
    <p>{post.error} Posts fade on schedule, and your {app.platformName} only has what it synced.</p>
    <a class="pill" href="#/">Home</a>
  </div>
{:else if !p}
  <p class="loading" role="status">Opening…</p>
{:else}
  <section class="view postwrap" use:enterView>
    <article class="main" aria-labelledby="p-h">
      <div class="meta anim">
        <a href={to.community(p.community.id)} class="act back">← c/{p.community.name}</a>
        <Avatar name={p.author.name} keyHex={p.author.key} avatar={p.author.avatar} avatarColor={p.author.avatarColor} mine={p.mine} size="sm" />
        <a class="who" href={to.person(p.author.key)}>{p.author.name}</a>
        {#if p.author.name !== p.author.short}<span class="mono">{p.author.short}</span>{/if}
        <span>· {clock(p.createdAt)}{p.edited ? ' · edited' : ''}</span>
        {#if p.author.role}<span class="tag tag-sun">{p.author.role}</span>{/if}
        {#if p.mine}<span class="tag tag-sun">yours</span>{:else}<TrustTag person={p.author} />{/if}
        {#if p.pinned}<span class="tag tag-sun">pinned</span>{/if}
        {#if p.waiting}<span class="tag tag-dashed">waiting at the door</span>{:else if p.unvalidated}<span class="tag tag-dashed">new here · not vouched for yet</span>{/if}
      </div>
      {#if p.held}
        <p class="callout anim">Held back on other devices: {p.held}. You still see it here; they'll show it once that changes.</p>
      {/if}

      {#if p.removed}
        <h1 id="p-h" class="anim gone">Poof.</h1>
        <div class="poof anim big">
          <div class="meta"><span class="who">{p.removed.by ? `Removed by ${p.removed.by.name}` : 'Deleted by the author'}</span><span>· {p.removed.reason} · {ago(p.removed.at)}</span></div>
          <p class="cb">
            {#if p.removed.by}The receipt is in the <a class="textlink" href={to.ledger(p.community.id)}>ledger</a>. Your device tossed its copy of the text.{:else}The author signed a delete; every device that follows the rules drops the body.{/if}
          </p>
        </div>
      {:else}
        <h1 id="p-h" class="anim">{p.title}</h1>
        {#if p.napping && !peek}
          <div class="nap napbig anim">
            <div class="zz" aria-hidden="true">zz</div>
            <p>This post is napping. {napWho(p.napping)}: <b>{p.napping.label}</b>. <button type="button" class="peek" onclick={() => (peek = true)}>Peek anyway</button></p>
          </div>
        {:else}
          {#if p.labels.length}<div class="meta anim">{#each p.labels as l (l)}<span class="tag tag-sky">note: {l}</span>{/each}</div>{/if}
          {#if p.link}
            {#if url}
              <div class="anim linkwrap"><LinkCard {url} full /></div>
            {:else}
              <span class="linkchip anim">{p.link}</span>
            {/if}
          {/if}
          <div class="anim"><MediaList media={p.media} /></div>
          <div class="body anim">
            {#each paragraphs(p.text) as para, i (i)}<p>{para}</p>{/each}
          </div>
        {/if}
      {/if}

      <div class="postacts anim">
        <Vote target={p.id} community={p.community.id} score={p.score} myVote={p.myVote} horizontal disabled={!!p.removed || !writable} />
        <a class="act" href="#replies">{plural(totalReplies, 'reply', 'replies')}</a>
        {#if !privateGroup}<button type="button" class="act" onclick={share}>Share invite</button>{/if}
        <button type="button" class={['act', p.saved && 'on']} aria-pressed={p.saved} onclick={toggleSave}>{p.saved ? '★ Saved' : 'Save'}</button>
        {#if canReview && !p.mine && !p.removed}
          {#if p.unvalidated}<button type="button" class="act" onclick={() => attempt(() => app.core.review(p.id, 'ok'), `Signed: it belongs here. That counts toward vouching for ${p.author.name}.`)}>Looks good</button>{/if}
          <button type="button" class="act" aria-expanded={hiding} onclick={() => (hiding = !hiding)}>Hide</button>
        {/if}
        {#if !p.mine && !p.removed}<button type="button" class="act" aria-expanded={flagging} onclick={() => (flagging = !flagging)}>Flag for the mods</button>{/if}
        <span class="hint fades">{fadesIn(p.expiresAt)}</span>
      </div>
      {#if flagging}<FlagPicker target={p.id} rules={cv?.rules?.text} ondone={() => (flagging = false)} />{/if}
      {#if hiding}<ReviewPicker target={p.id} rules={cv?.rules?.text} ondone={() => (hiding = false)} />{/if}

      {#if !p.removed}
        <div class="tools anim">
          <span class="label">{p.mine ? 'Yours' : canMod ? 'Mod tools' : 'Labels'}</span>
          {#if p.mine}
            <button type="button" class={['act', tool === 'edit' && 'on']} onclick={() => open('edit')}>Edit</button>
            <button type="button" class={['act', tool === 'delete' && 'on']} onclick={() => open('delete')}>Delete</button>
          {/if}
          {#if canMod}
            <button type="button" class={['act', tool === 'remove' && 'on']} onclick={() => open('remove')}>Remove</button>
            <button type="button" class="act" onclick={pinToggle}>{p.pinned ? 'Unpin' : 'Pin'}</button>
            <button type="button" class={['act', tool === 'label' && 'on']} onclick={() => open('label')}>Label</button>
            {#if !p.mine && p.author.role !== 'owner'}<button type="button" class={['act', tool === 'ban' && 'on']} onclick={() => open('ban')}>Ban {p.author.name}</button>{/if}
            {#if p.waiting}<button type="button" class="act on" onclick={() => attempt(() => app.core.admit(p.community.id, p.author.key), `${p.author.name} is in.`)}>Admit</button>{/if}
          {/if}
          {#if !canMod && !p.mine && !privateGroup}<button type="button" class={['act', tool === 'tag' && 'on']} onclick={() => open('tag')}>Label for your followers</button>{/if}
        </div>
        {#if tool}
          <form class="inset toolform" bind:this={toolBox} onsubmit={doTool}>
            {#if tool === 'edit'}
              <label class="field"><span>Title</span><input class="input" bind:value={editTitle} maxlength="300" /></label>
              <label class="field"><span>Text</span><textarea class="textarea" rows="6" bind:value={editText}></textarea></label>
              <p class="help">An edit is a new signed record pointing at this one. Peers show the latest, marked edited.</p>
              <div class="btnrow"><button class="pill pill-ink pill-sm" type="submit">Sign the edit</button><button class="pill pill-sm" type="button" onclick={() => (tool = '')}>Cancel</button></div>
            {:else if tool === 'delete'}
              <p>Delete this post? Peers drop the text and keep a tombstone signed by you. Replies stay, under a Poof.</p>
              <div class="btnrow"><button class="pill pill-danger pill-sm" type="submit">Delete it</button><button class="pill pill-sm" type="button" onclick={() => (tool = '')}>Keep it</button></div>
            {:else if tool === 'tag'}
              <label class="field"><span>Label</span><input class="input" bind:value={labelText} placeholder="spam, nsfw, off-topic, needs a source…" maxlength="40" /></label>
              <label class="field"><span>Note (optional, public)</span><input class="input" bind:value={reason} maxlength="200" /></label>
              <p class="help">It counts for whoever follows your labels, or whose community names you as a labeler. Spam, nsfw, off-topic and the like put the post to sleep for them; anything else shows as a note.</p>
              <div class="btnrow"><button class="pill pill-ink pill-sm" type="submit" disabled={!labelText.trim()}>Label &amp; sign</button><button class="pill pill-sm" type="button" onclick={() => (tool = '')}>Cancel</button></div>
            {:else if tool === 'label'}
              <label class="field"><span>Label</span><input class="input" bind:value={labelText} placeholder="off-topic, needs a source, spoiler…" maxlength="40" /></label>
              <label class="field"><span>Note (optional, public)</span><input class="input" bind:value={reason} maxlength="200" /></label>
              <div class="btnrow"><button class="pill pill-ink pill-sm" type="submit" disabled={!labelText.trim()}>Label &amp; sign</button><button class="pill pill-sm" type="button" onclick={() => (tool = '')}>Cancel</button></div>
            {:else}
              <label class="field">
                <span>{tool === 'remove' ? 'Why remove it?' : `Why ban ${p.author.name}?`} The reason is public in the ledger.</span>
                <input class="input" bind:value={reason} placeholder={tool === 'remove' ? 'rule 4, undisclosed vendor' : 'spam, three times'} maxlength="200" />
              </label>
              <div class="btnrow"><button class="pill pill-danger pill-sm" type="submit" disabled={!reason.trim()}>{tool === 'remove' ? 'Remove & sign' : 'Ban & sign'}</button><button class="pill pill-sm" type="button" onclick={() => (tool = '')}>Cancel</button></div>
            {/if}
          </form>
        {/if}
      {/if}

      {#if !p.removed && writable}<div class="anim"><ReplyBox parent={p.id} community={p.community.id} onsent={replied} /></div>{/if}

      <div class="rhead anim" id="replies">
        <h2>{plural(totalReplies, 'reply', 'replies')}</h2>
        <span class="tag tag-soft">friends first</span>
        {#if napCount}<span class="hint">· {napCount} napping</span>{/if}
      </div>
      <div class="comments">
        {#each p.comments as c (c.id)}
          <Comment {c} post={p} {canMod} {canReview} {freshId} rules={cv?.rules?.text} onreplied={replied} />
        {:else}
          <p class="quiet">No replies yet. Yours would be the first.</p>
        {/each}
      </div>
    </article>

    <aside class="side" aria-label="About this post">
      <div class="frame kvbox anim">
        <div class="kv head"><span>This post, checked</span><span class="tag tag-trust">✓<span class="sr-only"> checked</span></span></div>
        <div class="kv"><span>Signature</span><span>{p.checks.signature} · {p.checks.signedBy === 'root' ? 'root key' : 'device key'}</span></div>
        <div class="kv"><span>Id</span><span class="mono">{p.short}</span></div>
        <div class="kv"><span>Stamp</span><span>{p.checks.stampBits} bits of work{p.checks.stampBits > p.checks.requiredBits ? ` (${p.checks.requiredBits} needed)` : ''}</span></div>
        <div class="kv"><span>{p.checks.receivedFrom ? 'Seen on' : 'Made'}</span><span>{p.checks.receivedFrom ? `${p.checks.receivedFrom} of your peers` : `here, on this ${app.platformName}`}</span></div>
        <div class="kv"><span>Fades</span><span>{fadesIn(p.checks.fadesAt).replace('fades ', '')}</span></div>
        <div class="kv"><span>Identity</span><span>{p.checks.identityBits} bits of key work</span></div>
        <div class="kv"><span>Author here</span><span>{p.checks.standing}{p.checks.vouchedBy ? ` · vouched by ${p.checks.vouchedBy}` : ''}</span></div>
        {#if p.checks.homeDelivery}
          <div class="kv" role="status"><span>Delivery through your home</span><span>
            {p.checks.homeDelivery === 'placing' ? 'Your home is storing this post in pieces.' : p.checks.homeDelivery.startsWith('pieces-') ? 'Stored in pieces.' : 'Sent as a whole post.'}
            {p.checks.homeDelivery === 'placing' || p.checks.homeDelivery.endsWith('-pending') ? ' Waiting for your home’s delivery report.' : p.checks.homeDelivery.endsWith('-injected') ? ' Your home reports delivery through an injector.' : ' Your home reports direct forwarding because protected delivery was unavailable.'}
            Your home knows you sent this post.
          </span></div>
        {/if}
        {#if p.checks.addressHidden}
          <div class="kv"><span>Your address</span><span>The node you connect through can't tell this post is yours.</span></div>
        {/if}
        <p class="note2">Your device checked all this when it arrived. No server vouched for anything; members of c/{p.community.name} did.</p>
      </div>

      <div class="frame kvbox anim author">
        <div class="ahead">
          <Avatar name={p.author.name} keyHex={p.author.key} avatar={p.author.avatar} avatarColor={p.author.avatarColor} mine={p.mine} size="lg" />
          <div class="aname">
            <a class="strong" href={to.person(p.author.key)}>{p.author.name}</a>
            <span class="mono">{shortDid(didFromKeySafe(p.author.key))}</span>
          </div>
        </div>
        <div class="kv"><span>You two</span><span class={p.author.trust === 'friend' ? 'trusty' : ''}>{relation(p.author.trust)}</span></div>
        <div class="kv"><span>Key age</span><span>{keyAge(p.author.keyAgeDays)}</span></div>
        {#if p.author.role}<div class="kv"><span>Role</span><span>{p.author.role} in c/{p.community.name}</span></div>{/if}
        {#if p.author.nameHint && p.author.petname}<div class="kv"><span>Calls themself</span><span>{p.author.nameHint}</span></div>{/if}
        {#if !p.mine}
          {#if editingPet}
            <form class="pet" onsubmit={savePet}>
              <label class="field"><span>Your name for them (only you see it)</span><input class="input" bind:value={petname} maxlength="32" /></label>
              <div class="btnrow"><button class="pill pill-ink pill-sm" type="submit">Save</button><button type="button" class="pill pill-sm" onclick={() => (editingPet = false)}>Cancel</button></div>
            </form>
          {/if}
          <div class="abtns">
            <button type="button" class={['pill', 'pill-sm', p.author.trust === 'friend' && 'pill-ink']} aria-pressed={p.author.trust === 'friend'} onclick={trust}>{p.author.trust === 'friend' ? '✓ Trusted' : 'Trust'}</button>
            <button type="button" class="pill pill-sm" aria-pressed={p.author.muted} onclick={mute}>{p.author.muted ? 'Unmute' : 'Mute'}</button>
            {#if !editingPet}<button type="button" class="pill pill-sm" onclick={() => { petname = p.author.petname ?? ''; editingPet = true; }}>Rename</button>{/if}
          </div>
        {:else}
          <p class="note2">That's you. People see your name next to <span class="mono">{p.author.short}</span>.</p>
        {/if}
      </div>
      {#if authorStanding.value}
        <div class="anim"><StandingCard s={authorStanding.value} canVouch={!!cv?.me?.capabilities.includes('vouch')} /></div>
      {/if}
    </aside>
  </section>
{/if}

<style>
  .postwrap { display: grid; grid-template-columns: minmax(0, 1fr) 270px; gap: 56px; }
  .main { min-width: 0; }
  .back { padding-left: 0; }
  h1 { margin: 14px 0 22px; font-size: clamp(28px, 4vw, 48px); max-width: 820px; letter-spacing: -0.04em; overflow-wrap: anywhere; }
  h1.gone { color: var(--mute); }
  .body { display: flex; flex-direction: column; gap: 14px; font-size: 17px; line-height: 1.55; max-width: 640px; overflow-wrap: anywhere; }
  .body p { white-space: pre-line; }
  .linkchip, .linkwrap { margin-bottom: 16px; }
  .poof.big { padding: 14px 16px; max-width: 640px; }
  .napbig { border-top: 0; grid-template-columns: 52px minmax(0, 1fr); }
  .napbig b { color: var(--ink); }
  .postacts { display: flex; align-items: center; gap: 8px; margin-top: 22px; padding: 14px 0; border-top: 2px solid var(--ink); border-bottom: 2px dotted var(--line); flex-wrap: wrap; }
  .fades { margin-left: auto; }
  .tools { display: flex; align-items: center; gap: 2px; flex-wrap: wrap; padding: 10px 0; border-bottom: 2px dotted var(--line); }
  .tools .label { margin-right: 8px; }
  .toolform { margin-top: 10px; max-width: 640px; }
  .rhead { display: flex; align-items: center; gap: 10px; margin-top: 22px; padding-bottom: 4px; flex-wrap: wrap; }
  .rhead h2 { font-size: 22px; font-weight: 800; letter-spacing: -0.02em; }
  .comments > :global(.c:first-child) { border-top: 2px solid var(--ink); }
  .comments > :global(.c:last-child) { border-bottom: 2px dotted var(--line); }
  .quiet { color: var(--mute); font-weight: 500; padding: 18px 0; border-top: 2px solid var(--ink); }
  .side { display: flex; flex-direction: column; gap: 16px; padding-top: 4px; min-width: 0; }
  .note2 { margin: 10px 0 0; font-size: 13px; color: var(--mute); line-height: 1.45; font-weight: 500; }
  .author { padding-top: 14px; }
  .ahead { display: flex; align-items: center; gap: 12px; padding-bottom: 8px; min-width: 0; }
  .aname { display: flex; flex-direction: column; min-width: 0; }
  .strong { font-weight: 800; }
  .strong:hover { color: var(--cobalt); }
  .trusty { color: var(--cobalt); font-weight: 700; }
  .abtns { display: flex; gap: 6px; margin-top: 8px; flex-wrap: wrap; }
  .abtns .pill { flex: 1; }
  .pet { display: flex; flex-direction: column; gap: 8px; padding: 8px 0; }
  @media (max-width: 860px) {
    .postwrap { grid-template-columns: 1fr; gap: 24px; }
  }
</style>
