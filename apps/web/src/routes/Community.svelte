<script lang="ts">
  import type { PostSummary } from '@hoi/core-api';
  import { ago, duration } from '@hoi/core-api';
  import { onDestroy } from 'svelte';
  import { app, attempt, setTitle } from '../lib/app.svelte';
  import { resource } from '../lib/resource.svelte';
  import { enterView, nudge } from '../lib/motion';
  import { to, type CommunityTab } from '../lib/router';
  import { router } from '../lib/router.svelte';
  import { admissionLabel, copyText, formatMs, monthYear, plural, voiceText } from '../lib/util';
  import { toast } from '../lib/toast.svelte';
  import PostRow from '../lib/ui/PostRow.svelte';
  import Composer from '../lib/ui/Composer.svelte';
  import { startPost } from '../lib/compose';
  import StandingCard from '../lib/ui/StandingCard.svelte';
  import ProposalCard from '../lib/ui/ProposalCard.svelte';
  import InviteModal from '../lib/ui/InviteModal.svelte';
  import JoinWarning from '../lib/ui/JoinWarning.svelte';
  import Modal from '../lib/ui/Modal.svelte';
  import PublicName from '../lib/ui/PublicName.svelte';
  import PrivateGroup from '../lib/ui/PrivateGroup.svelte';
  import { openPiecesLine } from '../lib/private-status';
  import Warmth from '../lib/ui/Warmth.svelte';
  import Manage from './Manage.svelte';
  import Wiki from './Wiki.svelte';
  import AgeProof from '../lib/ui/AgeProof.svelte';

  let { id, tab, page }: { id: string; tab: CommunityTab; page?: string } = $props();

  const view = resource(() => {
    void app.rev;
    return app.core.community(id);
  });
  const feed = resource(() => {
    void app.rev;
    return app.core.feed({ community: id, sort: 'hot', limit: 60 });
  });
  const c = $derived(view.value);
  const privateGroup = $derived(app.privateGroups.find(g => g.community === id));
  // whether you share whom you trust here
  // a desktop app or hoi-node may count the votes here for others
  const counting = resource(() => {
    void app.rev;
    return app.boot?.platform === 'web' ? Promise.resolve(null) : app.core.aggregating(id);
  });
  async function toggleCounting() {
    if (!c || counting.value === null || counting.value === undefined) return;
    const next = !counting.value;
    await attempt(() => app.core.setAggregating(c.id, next), next ? `Counting c/${c.name}'s votes every hour. People who follow your counts see them.` : 'Not counting here any more.');
  }
  const sharing = resource(() => {
    void app.rev;
    return app.core.trustSharing(id);
  });
  const queue = resource(() => {
    void app.rev;
    return tab === 'ladder' ? app.core.reviewQueue(id) : Promise.resolve([]);
  });
  let inviteOpen = $state(false);
  let inviteFresh = $state(false);
  let joinWarning = $state(false);
  let joining = $state(false);
  let fresh = $state.raw<PostSummary[]>([]);
  const top = $derived(fresh.map((f) => feed.value?.find((p) => p.id === f.id) ?? f));
  const rest = $derived((feed.value ?? []).filter((p) => !fresh.some((f) => f.id === p.id)));
  const canManage = $derived(!!c && c.role !== 'member');
  const composing = $derived(app.composer.open && app.composer.community === id && (!privateGroup || privateGroup.active));
  const todo = $derived(c ? c.pending.length + c.flags.length : 0);
  // proposals waiting for your answer, and whether you still need someone to let you in
  const asked = $derived(c ? c.proposals.filter((p) => p.canAnswer && !p.myAnswer) : []);
  const atTheDoor = $derived(!!c?.me && c.me.level === 'newcomer' && !c.me.blocked && (c.limits.admission === 'invite' || c.limits.admission === 'approve' || c.limits.admission === 'age-proof'));
  // Warmth: who else holds this community's records, from this device's own links.
  const warmth = $derived(app.warmth?.communities.find((w) => w.community === id));
  const carryNodes = $derived(app.warmth?.directory.carryNodes ?? 0);
  const canAskCarry = $derived(!!c && c.joined && !privateGroup && c.limits.admission === 'open');
  const NO_CARRY_NODE = 'No connected node takes requests right now.';
  let asking = $state(false);
  // Handing a community to public nodes is a choice to make once, with its consequences in view.
  let carryConfirm = $state(false);
  let copying = $state(false);
  /** S4: an owner of a members-only community makes a private copy whose membership follows its own records. */
  async function makePrivateCopy() {
    if (!c || copying) return;
    copying = true;
    const copy = await attempt(() => app.core.privateCopy(c.id), 'Private copy made. People join it by request; its members’ devices check every change against its records.');
    copying = false;
    if (copy) {
      await app.refresh();
      router.go(to.community(copy.id));
    }
  }

  async function askCarry() {
    if (!c || asking) return;
    carryConfirm = false;
    asking = true;
    const r = await attempt(() => app.core.requestCarry(c.id));
    asking = false;
    if (!r) return;
    if (r.asked) toast(`Asked ${plural(r.asked, 'public node')}. Their answers show up as notices; Warmth shows what arrived.`);
    else if (r.carrying) toast('Connected public nodes already keep it.');
    else toast(NO_CARRY_NODE);
    void app.refreshWarmth();
  }

  async function copyAsk() {
    if (!c || !app.me) return;
    const inv = await attempt(() => app.core.invite(c.id));
    if (!inv) return;
    const link = `${inv.web.split('#')[0]}#/u/${app.me.rootKey}?c=${c.id}`;
    toast((await copyText(link))
      ? 'Link copied. Send it to someone in the community: it opens your page there, with a Vouch button.'
      : `Copy this link and send it to a member: ${link}`);
  }

  $effect(() => setTitle(c ? `c/${c.name}` : 'Community'));
  $effect(() => {
    if (c && !privateGroup && app.inviteFor === c.id) {
      app.inviteFor = null;
      inviteFresh = true;
      inviteOpen = true;
    }
  });
  onDestroy(() => {
    if (app.composer.open && app.composer.community === id) app.closeComposer();
  });

  async function toggleJoin(e: MouseEvent) {
    if (!c || joining) return;
    const btn = e.currentTarget as HTMLElement;
    if (c.joined) {
      await attempt(() => app.core.leave(c.id), `Left. Your copy of c/${c.name} fades over the next ${duration(c.limits.maxRetention)}.`);
    } else {
      joinWarning = true;
      return;
    }
    nudge(btn);
  }

  async function join() {
    if (!c || c.joined || joining) return;
    const community = c;
    joining = true;
    try {
      await attempt(async () => {
        const inv = await app.core.invite(community.id);
        return app.core.join(inv.uri);
      }, `Joined c/${community.name}. Your ${app.platformName} keeps it in sync now.`);
    } finally { joining = false; }
  }

  async function toggleShareTrust() {
    if (!c || !sharing.value) return;
    const next = !sharing.value.shared;
    await attempt(
      () => app.core.shareTrust(c.id, next),
      next ? `Shared: everyone in c/${c.name} can see the friends you trust here.` : 'Taken back. Your empty statement replaces it.',
    );
  }

  async function togglePin() {
    if (!c) return;
    const next = !c.pinning;
    const v = await attempt(
      () => app.core.setPinning(c.id, next),
      next ? `Pinning c/${c.name}: newcomers can catch up from your ${app.platformName}.` : 'Not pinning. You keep what you read.',
    );
    if (v) view.set(v);
  }

  function rulesLines(text: string) {
    return text.split('\n').map((line) => {
      const m = line.match(/^\s*(\d{1,2})[.)]\s+(.+)$/);
      return m ? { n: m[1], text: m[2] } : { n: '', text: line };
    }).filter((l) => l.text.trim());
  }
</script>

{#if view.error && !c}
  <section class="view" use:enterView>
    <div class="empty anim">
      <h2>That community isn't on this {app.platformName}.</h2>
      <p>{view.error} If someone sent you a link, open it as an invite.</p>
      <div class="btnrow"><a class="pill pill-ink" href={to.join()}>Open an invite</a><a class="pill" href="#/">Home</a></div>
    </div>
  </section>
{:else if !c}
  <p class="loading" role="status">Opening the community…</p>
{:else}
  <section class="view" use:enterView aria-labelledby="c-h">
    <div class="chead">
      <div class="headcol anim">
        <span class="tag big tag-{c.color} since">● c/{c.name} · since {monthYear(c.createdAt)}</span>
        <h1 id="c-h">{c.name}</h1>
        {#if c.description}<p class="desc">{c.description}</p>{/if}
        {#if privateGroup}<span class="tag tag-lilac">Private · invited devices only</span>{:else}<PublicName community={c} />{/if}
        <div class="btnrow">
          {#if !privateGroup}<button type="button" class={['pill', c.joined && 'pill-ink']} aria-pressed={c.joined} onclick={toggleJoin} disabled={joining}
            title={c.joined ? 'Click to leave' : undefined}>{joining ? 'Joining…' : c.joined ? '✓ Joined' : 'Join'}</button>
          {/if}
          {#if c.joined && !privateGroup}
            <button type="button" class="pill pinsw" role="switch" aria-checked={c.pinning} onclick={togglePin}
              title="Keep the full live window so newcomers can catch up from you">
              <span class="mini" aria-hidden="true"><i></i></span>Pinning
            </button>
            {#if typeof counting.value === 'boolean'}
              <button type="button" class="pill pinsw" role="switch" aria-checked={counting.value} onclick={toggleCounting}
                title="Publish this community's vote counts every hour, for people who follow your counts">
                <span class="mini" aria-hidden="true"><i></i></span>Count votes here
              </button>
            {/if}
            {#if sharing.value && sharing.value.friends > 0}
              <button type="button" class="pill pinsw" role="switch" aria-checked={sharing.value.shared} onclick={toggleShareTrust}
                title="Publish whom you trust here, so your friends rank by friends of friends. Everyone here can see it.">
                <span class="mini" aria-hidden="true"><i></i></span>Share whom I trust
              </button>
            {/if}
          {/if}
          {#if !privateGroup}<button type="button" class="pill" onclick={() => (inviteOpen = true)}>Invite a friend</button>{/if}
        </div>
        {#if sharing.value?.shared && !privateGroup}<p class="help">Everyone in c/{c.name} can see the {plural(sharing.value.named, 'friend')} you trust here. Friends of yours who share theirs see their friends' votes count by half.</p>{/if}
        {#if warmth && !privateGroup}
          <Warmth w={warmth} fold>
            {#if canAskCarry}
              <div class="carry">
                <button type="button" class="pill pill-sm" onclick={() => (carryConfirm = true)} disabled={asking || carryNodes === 0} title={carryNodes === 0 ? NO_CARRY_NODE : undefined}>{asking ? 'Asking…' : 'Ask public nodes to keep it available'}</button>
                <p class="help">{carryNodes === 0 ? `${NO_CARRY_NODE} ` : ''}Public nodes can keep sharing the posts and pictures after you close hoi. Anyone can read what they share, including the people running those nodes.</p>
              </div>
            {/if}
          </Warmth>
        {/if}
      </div>
      <div class="frame kvbox anim limits" aria-label="This community's limits">
        <div class="kv"><span>People seen</span><span class="num">~{c.stats.people.toLocaleString('en')}</span></div>
        <div class="kv"><span>Posts fade after</span><span>{duration(c.limits.maxRetention)}</span></div>
        <div class="kv"><span>Joining</span><span>{privateGroup ? 'Private invitation' : admissionLabel(c.limits.admission)}</span></div>
        <div class="kv"><span>Newcomers</span><span>{plural(c.limits.newcomerPostsPerDay, 'post')} / day</span></div>
        <div class="kv"><span>Stamp</span><span>{c.limits.powBits} bits · ≈ {formatMs(app.stampMs(c.limits.powBits))}</span></div>
        <div class="kv"><span>Rules</span><span>{c.rules ? `v${c.rules.version} · signed ${c.rules.signedBy.length} of ${c.owners.length}` : 'none yet'}</span></div>
        {#if c.me}<div class="kv"><span>You here</span><a class="textlink" href={to.community(c.id, 'ladder')}>{c.me.blocked ? 'blocked' : c.me.level}</a></div>{/if}
        {#if c.pieces && !privateGroup}<div class="kv"><span>Posts</span><span>kept in pieces</span></div>{/if}
      </div>
      {#if c.pieces && !privateGroup}<p class="help">{openPiecesLine(c.pieces)}</p>{/if}
    </div>

    {#if privateGroup}<PrivateGroup community={id} />{/if}
    {#if c.privateCopy}
      <div class="frame private-copy">
        <p><b>This community’s posts travel in the clear</b> to every node that keeps it. A private copy keeps the same name, rules and limits, encrypts everything for its members, and follows this community’s way of admitting people: invitations and approvals decide who can read, not one device.</p>
        <button type="button" class="pill" onclick={makePrivateCopy} disabled={copying}>{copying ? 'Making a private copy…' : 'Make a private copy'}</button>
      </div>
    {/if}
    <div class="vh anim tabsrow">
      <nav class="tabs" aria-label="Community sections">
        <a class={['tab', tab === 'posts' && 'on']} href={to.community(c.id)} aria-current={tab === 'posts' ? 'page' : undefined}>Posts</a>
        <a class={['tab', tab === 'rules' && 'on']} href={to.community(c.id, 'rules')} aria-current={tab === 'rules' ? 'page' : undefined}>
          Rules{#if asked.length}<span class="count" aria-label={`${asked.length} proposals waiting for your answer`}>{asked.length}</span>{/if}
        </a>
        <a class={['tab', tab === 'wiki' && 'on']} href={to.wiki(c.id)} aria-current={tab === 'wiki' ? 'page' : undefined}>Wiki</a>
        <a class={['tab', tab === 'ladder' && 'on']} href={to.community(c.id, 'ladder')} aria-current={tab === 'ladder' ? 'page' : undefined}>
          Ladder{#if c.reviewQueue}<span class="count" aria-label={`${c.reviewQueue} to review`}>{c.reviewQueue}</span>{/if}
        </a>
        <a class="tab" href={to.ledger(c.id)}>Ledger</a>
        {#if canManage}
          <a class={['tab', tab === 'manage' && 'on']} href={to.community(c.id, 'manage')} aria-current={tab === 'manage' ? 'page' : undefined}>
            Manage{#if todo}<span class="count" aria-label={`${todo} waiting`}>{todo}</span>{/if}
          </a>
        {/if}
      </nav>
      {#if tab === 'posts'}<span class="hint">sorted: hot · ranked on this {app.platformName}</span>{/if}
    </div>

    {#if atTheDoor && tab !== 'manage' && c.limits.admission === 'age-proof'}
      <AgeProof community={c.id} />
    {:else if atTheDoor && tab !== 'manage'}
      <div class="frame kvbox door anim">
        <div class="kv head"><span>{c.limits.admission === 'invite' ? 'This community is invite-only' : 'A mod lets newcomers in here'}</span><span class="tag tag-sun">at the door</span></div>
        <p class="help">
          {c.limits.admission === 'invite'
            ? 'What you post waits on your device until someone established here vouches for you. Send them your link: it opens your page with a Vouch button.'
            : 'Your posts show to established members until a moderator admits you or members vouch. Send someone your link to speed it up.'}
        </p>
        <div class="btnrow"><button type="button" class="pill pill-ink pill-sm" onclick={copyAsk}>Copy my link</button></div>
      </div>
    {/if}
    {#if tab === 'posts' && asked.length}
      <div class="asked anim" aria-label="Changes waiting for your answer">
        {#each asked as p (p.id)}<ProposalCard {p} community={c.id} compact />{/each}
      </div>
    {/if}
    {#if tab === 'posts'}
      {#if composing}
        {#key app.composer.nonce}
          <Composer community={c.id} onposted={(p) => (fresh = [p, ...fresh])} onclose={() => app.closeComposer()} />
        {/key}
      {/if}
      {#if feed.value && top.length + rest.length === 0}
        <div class="empty anim">
          <h2>Nothing here yet.</h2>
          <p>
            {#if privateGroup}Messages sync privately between invited devices.{:else if c.joined}Posts arrive as your {app.platformName} syncs with the other members.{#if !app.net?.peers}{' '}Connect with another member to receive their posts.{/if} Or start it off yourself.{:else}Join to sync this community's posts.{/if}
          </p>
          {#if c.joined && (!privateGroup || privateGroup.active)}<div class="btnrow"><button type="button" class="pill pill-ink" onclick={() => startPost(c.id)}>+ Post in c/{c.name}</button></div>{/if}
        </div>
      {:else if feed.value}
        <div class="rows">
          {#each top as p (p.id)}<PostRow post={p} showCommunity={false} showExcerpt={false} fresh />{/each}
          {#each rest as p (p.id)}<PostRow post={p} showCommunity={false} showExcerpt={false} />{/each}
        </div>
      {:else}
        <p class="loading" role="status">Ranking…</p>
      {/if}
    {:else if tab === 'rules'}
      <div class="rules anim">
        {#if c.proposals.length}
          <h2 class="lh">Proposed changes</h2>
          <div class="props">
            {#each c.proposals as p (p.id)}<ProposalCard {p} community={c.id} />{/each}
          </div>
        {/if}
        {#if c.rules}
          <div class="meta rulesmeta">
            <span class="tag tag-mint">v{c.rules.version}</span>
            <span>signed by {c.rules.signedBy.map((p) => p.name).join(', ')} · {c.rules.signedBy.length} of {c.owners.length} owners · {ago(c.rules.at)}</span>
          </div>
          <div class="rows">
            {#each rulesLines(c.rules.text) as l, i (i)}
              <div class="rule">
                <span class="rn" aria-hidden={!l.n}>{l.n}</span>
                <p>{l.text}</p>
              </div>
            {/each}
          </div>
          <p class="help">Owners propose rules and limits; everyone established here decides. A change applies once they all agree. People who are away (nothing of theirs left here) don't hold it up, and after {duration(c.limits.maxRetention)} silence counts as yes, so only objections stop it. Everything is signed and kept in the <a class="textlink" href={to.ledger(c.id)}>ledger</a>.</p>
        {:else}
          <div class="empty">
            <h2>No rules yet.</h2>
            <p>{c.role === 'owner' ? 'Propose some under Manage. They apply once the members here agree.' : 'Nobody has proposed rules yet. Be kind anyway.'}</p>
          </div>
        {/if}
      </div>
    {:else if tab === 'wiki'}
      <Wiki community={c.id} {page} />
    {:else if tab === 'ladder'}
      <div class="ladderwrap anim">
        <div class="lmain">
          {#if c.me?.capabilities.includes('review')}
            <h2 class="lh">Waiting for your review</h2>
            <p class="help">Newcomers' posts. Read them: what you don't hide earns them your vouch, a few minutes later{app.settings?.autoVouch === false ? ' (automatic vouching is off in your settings)' : ''}. Hide only spam, abuse, off-topic or rule-breaking posts. Disagreeing is what downvotes are for.</p>
            <div class="rows">
              {#each queue.value ?? [] as p (p.id)}<PostRow post={p} showCommunity={false} />{:else}<p class="quiet">Nobody waiting. Newcomers show up here.</p>{/each}
            </div>
          {/if}
          <h2 class="lh">How trust works in c/{c.name}</h2>
          <ol class="steps">
            <li><b>Newcomer.</b> Text only, {plural(c.limits.newcomerPostsPerDay, 'post')} a day, marked “new here”{c.limits.admission === 'approve' ? ', shown to established members until vouched for' : ''}. Every key carries {c.limits.ladder.identityBits} bits of identity work: a new key starts from nothing, here and everywhere.</li>
            <li><b>Member.</b> {plural(c.me?.vouchesNeeded ?? c.limits.ladder.admitVouches, 'established member')} read your posts and vouch. Links unlock; your votes count.</li>
            <li><b>Established.</b> {duration(c.limits.ladder.establishedAge)} as a member. Your vouches, hides and vetoes count.</li>
            <li><b>Trusted.</b> {plural(c.limits.ladder.trustedPosts, 'post or comment', 'posts or comments')} in the last 90 days and a response of {c.limits.ladder.mediaScore}+. Images and other media unlock. Stop contributing and it fades again.</li>
          </ol>
          <p class="help">
            Decisions weigh voices, not heads. Someone vouched in alone, or by several members, holds a full voice; people vouched in together share their voucher's, and nobody holds more than their strongest voucher. Inviting all your friends at once brings them in, but not as a voting bloc. Reading people's posts without hiding them vouches for them and fills their voice out.
          </p>
          <p class="help">
            {plural(c.limits.ladder.hideQuorum, 'voice')} of hides put a post to sleep for everyone (anyone can peek); {plural(c.limits.ladder.blockQuorum, 'voice')} of hiders block someone. Hides need twice the voice of those who say a post is fine.
            {plural(c.limits.ladder.vetoQuorum, 'voice')} of vetoes overturn a moderator's or owner's action; a moderator overturned three times is suspended. Removals wait {duration(c.limits.ladder.contestWindow)} before the text is deleted, unless they're for illegal content or private data.
            Quorums never exceed a third of the established voice here ({c.establishedVoice.toFixed(1)} across {c.established} people), and every decision needs at least two people.{c.me && !c.me.blocked && c.me.level !== 'newcomer' ? ` You hold ${voiceText(c.me.voice)}.` : ''}
          </p>
          <p class="help">Changing any of these numbers is a proposal: it applies when everyone established agrees (see Rules).</p>
        </div>
        {#if c.me}<div class="lside"><StandingCard s={c.me} self /></div>{/if}
      </div>
    {:else if tab === 'manage'}
      {#if canManage}
        <Manage {c} reload={() => view.reload()} />
      {:else}
        <div class="empty anim"><h2>Only owners and mods manage a community.</h2><p>Everything they do is public in the <a class="textlink" href={to.ledger(c.id)}>ledger</a>.</p></div>
      {/if}
    {/if}
  </section>
  {#if !privateGroup}
    <InviteModal bind:open={inviteOpen} community={c} fresh={inviteFresh} onclose={() => (inviteFresh = false)} />
    <JoinWarning bind:open={joinWarning} oncontinue={join} />
  {/if}
  {#if canAskCarry}
    <Modal bind:open={carryConfirm} title="Ask public nodes to keep c/{c.name} available?">
      <p>The {plural(carryNodes, 'connected public node')} that take{carryNodes === 1 ? 's' : ''} requests get this community's signed definition and nothing else: no routes, nothing about you. A node that says yes stores and serves the community's readable posts and pictures to anyone who joins by key, so newcomers no longer need an invite; its operator can read them too. A node lets a community go again when nobody else holds it for two weeks.</p>
      <p class="help">Their answers show up as notices. Warmth shows what actually arrived; a yes is not a receipt.</p>
      <div class="actions">
        <button type="button" class="pill pill-ink" onclick={askCarry}>Ask them</button>
        <button type="button" class="pill" onclick={() => (carryConfirm = false)}>Not now</button>
      </div>
    </Modal>
  {/if}
{/if}

<style>
  .chead { display: grid; grid-template-columns: minmax(0, 1fr) 300px; gap: 40px; align-items: end; padding-bottom: 22px; }
  .headcol { display: flex; flex-direction: column; gap: 12px; min-width: 0; }
  .since { align-self: flex-start; white-space: normal; }
  h1 { font-size: clamp(38px, 5.4vw, 64px); letter-spacing: -0.045em; line-height: 0.95; overflow-wrap: anywhere; }
  .desc { font-size: 17px; line-height: 1.45; color: var(--mute); max-width: 560px; font-weight: 500; }
  .limits { padding-top: 6px; }
  .carry { display: flex; flex-direction: column; gap: 6px; align-items: flex-start; }
  .carry .help { max-width: 60ch; }
  .actions { display: flex; flex-wrap: wrap; gap: 10px; margin-top: 16px; }
  .pinsw { gap: 10px; }
  .mini { width: 30px; height: 18px; border-radius: 999px; border: 2px solid var(--ink); background: var(--card); position: relative; flex: 0 0 30px; }
  .mini i { position: absolute; top: 1px; left: 1px; width: 12px; height: 12px; border-radius: 50%; background: var(--line); transition: transform .2s; }
  .pinsw[aria-checked='true'] .mini { background: var(--ink); }
  .pinsw[aria-checked='true'] .mini i { background: var(--sun); transform: translateX(12px); }
  .tabsrow { padding-bottom: 10px; }
  .ladderwrap { display: grid; grid-template-columns: minmax(0, 1fr) 300px; gap: 40px; }
  .lmain { min-width: 0; }
  .lmain .help + .help { margin-top: 10px; }
  .lh { font-size: 22px; font-weight: 800; letter-spacing: -0.02em; margin: 6px 0 8px; }
  .lmain .rows { margin: 10px 0 26px; }
  .steps { display: flex; flex-direction: column; gap: 8px; padding-left: 20px; margin: 10px 0 14px; line-height: 1.5; max-width: 640px; }
  .quiet { color: var(--mute); font-weight: 500; padding: 14px 0; border-top: 2px solid var(--ink); }
  @media (max-width: 860px) {
    .ladderwrap { grid-template-columns: 1fr; gap: 20px; }
  }
  .count { display: inline-flex; align-items: center; justify-content: center; min-width: 20px; height: 20px; padding: 0 6px; border-radius: 999px; background: var(--coral); color: var(--tagink); font-size: 12px; font-weight: 800; }
  .rules { display: flex; flex-direction: column; gap: 14px; }
  .props, .asked { display: flex; flex-direction: column; gap: 12px; }
  .asked { margin-bottom: 18px; }
  .door { margin-bottom: 18px; display: flex; flex-direction: column; gap: 8px; padding-bottom: 14px; }
  .rulesmeta { padding-bottom: 4px; }
  .rule { display: grid; grid-template-columns: 44px minmax(0, 1fr); gap: 14px; padding: 16px 0; border-top: 2px dotted var(--line); align-items: baseline; }
  .rn { font-size: 28px; font-weight: 900; letter-spacing: -0.03em; color: var(--ink); }
  .rule p { font-size: 18px; font-weight: 600; line-height: 1.4; overflow-wrap: anywhere; }
  @media (max-width: 860px) {
    .chead { grid-template-columns: 1fr; gap: 20px; }
  }
  @media (prefers-reduced-motion: reduce) {
    .mini i { transition: none; }
  }
  .private-copy { padding: 16px 18px; display: flex; flex-direction: column; gap: 10px; font-size: 14px; margin-block: 16px; align-items: flex-start; }
</style>
