<script lang="ts">
  import { app, attempt, setTitle } from '../lib/app.svelte';
  import { resource } from '../lib/resource.svelte';
  import { enterView } from '../lib/motion';
  import { didFromKeySafe, keyAge, monthYear } from '../lib/util';
  import CommunityTag from '../lib/ui/CommunityTag.svelte';
  import TrustTag from '../lib/ui/TrustTag.svelte';
  import HandleTag from '../lib/ui/HandleTag.svelte';
  import Avatar from '../lib/ui/Avatar.svelte';
  import StandingCard from '../lib/ui/StandingCard.svelte';
  import { to } from '../lib/router';

  let { key, community }: { key: string; community?: string } = $props();
  const view = resource(() => {
    void app.rev;
    return app.core.person(key, community);
  });
  // their place in one community (an "ask to be let in" link carries it), and whether you can vouch
  const here = resource(() => {
    void app.rev;
    return community ? Promise.all([app.core.community(community), app.core.standing(community, key)]) : Promise.resolve(null);
  });
  const hc = $derived(here.value?.[0]);
  const hs = $derived(here.value?.[1]);
  const pv = $derived(view.value);
  const p = $derived(pv?.person);
  const isMe = $derived(p?.trust === 'me');
  let pet = $state('');
  let editing = $state(false);
  $effect(() => setTitle(p ? p.name : 'Someone'));

  async function trust() {
    if (!p) return;
    const f = p.trust !== 'friend';
    await attempt(() => app.core.setTrust(p.key, f), f ? `You trust ${p.name} now.` : `${p.name} is no longer a friend on this device.`);
  }
  const labelers = resource(() => {
    void app.rev;
    return app.core.labelers();
  });
  const followsLabels = $derived(!!p && !!labelers.value?.some((l) => l.followed && l.person.key === p.key));
  const aggregators = resource(() => {
    void app.rev;
    return app.core.aggregators();
  });
  const followsCounts = $derived(!!p && !!aggregators.value?.some((a) => a.person.key === p.key));
  const keepers = resource(() => {
    void app.rev;
    return app.core.blocklistKeepers();
  });
  const followsList = $derived(!!p && !!keepers.value?.some((k) => k.person.key === p.key));
  async function followList() {
    if (!p) return;
    const on = !followsList;
    await attempt(() => app.core.followBlocklist(p.key, on), on ? `${p.name}'s blocklist counts here now: what it lists is refused on this device.` : `${p.name}'s blocklist no longer counts here.`);
  }
  async function followCounts() {
    if (!p) return;
    const on = !followsCounts;
    await attempt(() => app.core.followAggregator(p.key, on), on ? `${p.name}'s counts show next to posts now, and under Counted.` : `${p.name}'s counts no longer show here.`);
  }
  async function followLabels() {
    if (!p) return;
    const on = !followsLabels;
    await attempt(() => app.core.followLabeler(p.key, on), on ? `${p.name}'s labels count here now: what they mark as spam or nsfw naps for you.` : `${p.name}'s labels no longer count here.`);
  }
  async function mute() {
    if (!p) return;
    await attempt(() => app.core.setMuted(p.key, !p.muted), p.muted ? `Unmuted ${p.name}.` : `Muted ${p.name} on this device.`);
  }
  async function savePet(e: SubmitEvent) {
    e.preventDefault();
    if (!p) return;
    const ok = await attempt(() => app.core.setPetname(p.key, pet.trim() || null), pet.trim() ? 'Saved. Only you see that name.' : 'Back to the name they chose.');
    if (ok) editing = false;
  }
</script>

{#if view.error && !p}
  <div class="empty"><h2>Nobody by that key here.</h2><p>{view.error}</p><a class="pill" href="#/">Home</a></div>
{:else if !p || !pv}
  <p class="loading" role="status">Looking them up…</p>
{:else}
  <section class="view" use:enterView aria-labelledby="u-h">
    <div class="vh anim head">
      <Avatar name={p.name} keyHex={p.key} avatar={p.avatar} avatarColor={p.avatarColor} size="xl" />
      <div class="htext">
        <h1 id="u-h">{p.name}</h1>
        <span class="mono">{didFromKeySafe(p.key)}</span>
      </div>
      <span class="push"><HandleTag person={p} /> <TrustTag person={p} /></span>
    </div>
    {#if hc && hs && !isMe}
      <div class="anim ask">
        <h2 class="askh">{hs.level === 'newcomer' && !hs.blocked ? `${p.name} would like to be let into` : `${p.name} in`} <a class="textlink" href={to.community(hc.id)}>c/{hc.name}</a></h2>
        {#if hs.level === 'newcomer' && !hs.blocked}
          <p class="help">
            {hc.me?.capabilities.includes('vouch')
              ? `Vouch if you know them or have read their posts and would want them here. Your vouch is signed and public${hc.limits.admission === 'invite' ? ', and in an invite-only community it is the invite' : ''}.`
              : `You can vouch for people once you're established in c/${hc.name}.`}
          </p>
        {/if}
        <StandingCard s={hs} canVouch={!!hc.me?.capabilities.includes('vouch') && !hs.person.role} />
      </div>
    {:else if community && here.error}
      <p class="help anim">{here.error}</p>
    {/if}
    <div class="grid">
      <div class="sets anim">
        <div class="set"><span class="l">Short key</span><span class="h">What shows next to their name everywhere. Names can be copied; this can't.</span><span class="v mono strong">{p.short}</span></div>
        {#if p.nameHint && p.petname}<div class="set"><span class="l">Calls themself</span><span class="h">The name on their posts.</span><span class="v">{p.nameHint}</span></div>{/if}
        <div class="set"><span class="l">First seen</span><span class="h">When your {app.platformName} first saw a record signed by this key.</span><span class="v">{monthYear(pv.firstSeen)} · {keyAge(p.keyAgeDays)}</span></div>
        <div class="set"><span class="l">Posts here</span><span class="h">Live posts by them on this {app.platformName}.</span><span class="v num">{pv.posts}</span></div>
        <div class="set top"><span class="l">In common</span><span class="h">Communities you're both in.</span>
          <span class="v tags">{#each pv.communities as c (c.id)}<CommunityTag {c} />{:else}<span class="muted">none yet</span>{/each}</span>
        </div>
        {#if !isMe}
          <div class="set top">
            <span class="l">Your name for them</span>
            <span class="h">A petname lives only on your devices. Handy when two people both call themselves "alex".</span>
            <span class="v">
              {#if editing}
                <form class="petf" onsubmit={savePet}>
                  <label class="sr-only" for="u-pet">Petname</label>
                  <input id="u-pet" class="input" bind:value={pet} maxlength="32" />
                  <button class="pill pill-ink pill-sm" type="submit">Save</button>
                </form>
              {:else}
                <span>{p.petname ?? '—'}</span>
                <button type="button" class="lnk lnk-ink" onclick={() => { pet = p.petname ?? ''; editing = true; }}>{p.petname ? 'Change' : 'Give them one'}</button>
              {/if}
            </span>
          </div>
        {/if}
      </div>
      {#if !isMe}
        <div class="frame kvbox anim actions">
          <div class="kv head"><span>Trust</span><span>{p.trust === 'friend' ? '✓ friend' : p.trust === 'fof' ? `friend of ${p.trustedBy?.join(', ') || 'a friend'}` : 'not yet'}</span></div>
          <p class="help">Trust is local: their votes and flags weigh more for you, and their posts show under Friends. Nobody is told, unless you share whom you trust in a community.</p>
          <div class="btnrow">
            <button type="button" class={['pill', 'pill-sm', p.trust === 'friend' && 'pill-ink']} aria-pressed={p.trust === 'friend'} onclick={trust}>{p.trust === 'friend' ? '✓ Trusted' : 'Trust'}</button>
            <button type="button" class="pill pill-sm" aria-pressed={p.muted} onclick={mute}>{p.muted ? 'Unmute' : 'Mute'}</button>
            <button type="button" class={['pill', 'pill-sm', followsLabels && 'pill-ink']} aria-pressed={followsLabels} onclick={followLabels}
              title="Their labels (spam, nsfw, off-topic, notes) count on this device">{followsLabels ? '✓ Following labels' : 'Follow labels'}</button>
            <button type="button" class={['pill', 'pill-sm', followsCounts && 'pill-ink']} aria-pressed={followsCounts} onclick={followCounts}
              title="If they run an aggregator: the vote counts it publishes show next to posts">{followsCounts ? '✓ Following counts' : 'Follow counts'}</button>
            <button type="button" class={['pill', 'pill-sm', followsList && 'pill-ink']} aria-pressed={followsList} onclick={followList}
              title="If they keep a blocklist: the media and pictures it lists are refused on this device">{followsList ? '✓ Following blocklist' : 'Follow blocklist'}</button>
          </div>
        </div>
      {/if}
    </div>
  </section>
{/if}

<style>
  .head { gap: 18px; }
  .htext { display: flex; flex-direction: column; gap: 6px; min-width: 0; flex: 1 1 260px; }
  .htext .mono { overflow-wrap: anywhere; }
  .htext h1 { overflow-wrap: anywhere; }
  .push { margin-left: auto; }
  .grid { display: grid; grid-template-columns: minmax(0, 1fr) 270px; gap: 40px; align-items: start; }
  .set { grid-template-columns: 160px minmax(0, 1fr) minmax(0, auto); }
  .strong { color: var(--ink); font-size: 14px; }
  .tags { flex-wrap: wrap; }
  .muted { color: var(--mute); }
  .petf { display: flex; gap: 8px; }
  .actions { display: flex; flex-direction: column; gap: 10px; padding-bottom: 16px; }
  .ask { display: flex; flex-direction: column; gap: 10px; margin-bottom: 26px; max-width: 520px; }
  .askh { font-size: 20px; font-weight: 800; letter-spacing: -0.02em; overflow-wrap: anywhere; }
  @media (max-width: 860px) {
    .grid { grid-template-columns: 1fr; }
    .set { grid-template-columns: 1fr; }
  }
</style>
