<script lang="ts">
  import { duration } from '@hoi/core-api';
  import { app, setTitle } from '../lib/app.svelte';
  import { resource } from '../lib/resource.svelte';
  import { enterView } from '../lib/motion';
  import { router } from '../lib/router.svelte';
  import { to } from '../lib/router';
  import { admissionLabel, formatMs, inviteQuery, joinLink, plural } from '../lib/util';
  import { toast, toastError } from '../lib/toast.svelte';
  import JoinWarning from '../lib/ui/JoinWarning.svelte';

  let { query }: { query: string } = $props();
  let pasted = $state('');
  let busy = $state(false);
  let joinWarning = $state(false);

  const preview = resource(() => (query ? app.core.previewInvite(joinLink(query)) : Promise.resolve(null)));
  const p = $derived(preview.value);
  $effect(() => setTitle(p ? `Join c/${p.community.name}` : 'Open an invite'));

  function open(e: SubmitEvent) {
    e.preventDefault();
    const q = inviteQuery(pasted);
    if (!q) return toast("That isn't an invite link. They start with hoi://join? or end in #/join?…");
    router.go(to.join(q));
  }

  async function join() {
    if (!p || p.alreadyJoined || busy) return;
    busy = true;
    try {
      const c = await app.core.join(joinLink(query));
      toast(`Joined c/${c.name}. Your ${app.platformName} keeps it in sync from now on.`);
      router.go(to.community(c.id));
    } catch (e) {
      toastError(e);
    }
    busy = false;
  }
</script>

<section class="view" use:enterView aria-labelledby="j-h">
  {#if !query}
    <div class="vh anim"><h1 id="j-h">Open an invite</h1></div>
    <form class="pastebox anim" onsubmit={open}>
      <label class="field">
        <span>Paste the link someone sent you</span>
        <input class="input mono-in" bind:value={pasted} placeholder="https://…#/join?code=… or hoi://join?code=…" autocomplete="off" />
        <span class="help">An invite carries the community's key and its signed details, plus where to find other members. No server needed.</span>
      </label>
      <div class="btnrow"><button class="pill pill-ink" type="submit">Open it →</button><a class="pill" href="#/new">Or start your own</a></div>
    </form>
  {:else if preview.error}
    <div class="empty anim">
      <h2 id="j-h">That invite didn't work.</h2>
      <p>{preview.error}</p>
      <div class="btnrow"><a class="pill pill-ink" href={to.join()}>Paste another</a><a class="pill" href="#/">Home</a></div>
    </div>
  {:else if !p}
    <p class="loading" role="status">Checking the invite…</p>
  {:else}
    <span class="sticker anim">you're invited</span>
    <div class="frame card anim">
      <div class="band bg-{p.community.color}" aria-hidden="true"></div>
      <div class="inner">
        <div class="meta">
          <span class="tag big tag-{p.community.color}">● c/{p.community.name}</span>
          {#if p.verified}
            <span class="tag tag-trust">✓ signed details check out</span>
          {:else}
            <span class="tag tag-dashed">not checked yet: details arrive with the first sync</span>
          {/if}
        </div>
        <h1 id="j-h">{p.community.name}</h1>
        {#if p.community.description}<p class="desc">{p.community.description}</p>{/if}
        <div class="kvs">
          <div class="kv"><span>Posts fade after</span><span>{duration(p.community.limits.maxRetention)}</span></div>
          <div class="kv"><span>Joining</span><span>{admissionLabel(p.community.limits.admission)}</span></div>
          <div class="kv"><span>Newcomers</span><span>{plural(p.community.limits.newcomerPostsPerDay, 'post')} / day</span></div>
          <div class="kv"><span>Stamp</span><span>{p.community.limits.powBits} bits · ≈ {formatMs(app.stampMs(p.community.limits.powBits))} per post here</span></div>
          <div class="kv"><span>Owners</span><span>{p.community.owners.map((o) => o.name).join(', ') || '—'}</span></div>
          <div class="kv"><span>Finds members via</span><span>{p.relays.length || p.peers.length ? `${plural(p.relays.length, 'relay')} · ${plural(p.peers.length, 'peer')}` : 'nobody yet: pair a home node to sync'}</span></div>
          {#if p.trackers?.length}
            <div class="kv"><span>If nobody answers, asks</span><span class="mono">{p.trackers.map((t) => t.replace(/^https?:\/\//, '')).join(', ')}</span></div>
          {/if}
        </div>
        {#if p.community.limits.admission === 'approve'}
          <p class="callout">A mod says yes here. You can post right away; others see your posts once a mod admits your key.</p>
        {/if}
        <div class="btnrow">
          {#if p.alreadyJoined}
            <a class="pill pill-ink" href={to.community(p.community.id)}>You're in. Open it →</a>
          {:else}
            <button type="button" class="pill pill-coral pill-lg" onclick={() => (joinWarning = true)} disabled={busy}>{busy ? 'Joining…' : `Join c/${p.community.name}`}</button>
            <a class="pill pill-lg" href="#/">Not now</a>
          {/if}
        </div>
      </div>
    </div>
  {/if}
</section>

<JoinWarning bind:open={joinWarning} oncontinue={join} />

<style>
  .pastebox { display: flex; flex-direction: column; gap: 14px; max-width: 640px; padding-top: 16px; border-top: 2px solid var(--ink); }
  .sticker { margin-bottom: -10px; position: relative; z-index: 1; margin-left: 18px; }
  .card { overflow: hidden; max-width: 720px; }
  .band { height: 14px; border-bottom: 2px solid var(--ink); }
  .inner { padding: 22px; display: flex; flex-direction: column; gap: 14px; }
  h1 { font-size: clamp(36px, 5vw, 56px); letter-spacing: -0.045em; overflow-wrap: anywhere; }
  .desc { color: var(--mute); font-size: 17px; font-weight: 500; line-height: 1.45; }
  .kvs { border-top: 2px solid var(--ink); }
  .kvs .kv:first-child { border-top: 0; }
  @media (max-width: 640px) {
    .inner { padding: 16px; }
  }
</style>
