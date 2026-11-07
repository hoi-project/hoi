<script lang="ts">
  // One line above a feed: what the connection is and, off the community page, Warmth. The
  // sentences behind it open on request; × hides it until the reading changes (a new problem
  // always shows again). Everything here is also on the You page, under Connection.
  import { app } from '../app.svelte';
  import { canPublishPublic, postsThroughOthers, syncStatus } from '../network-status';
  import { RELAY_URLS } from '../private-relay';
  import { privateLine } from '../private-status';
  import { REGISTRY_URL } from '../registry';
  import { to } from '../router';
  import { syncNotice } from '../sync-notice.svelte';
  import Warmth from './Warmth.svelte';

  let { community = undefined, showWarmth = true }: { community?: string; showWarmth?: boolean } = $props();
  const status = $derived(syncStatus(app.net, community));
  const warmth = $derived(community && showWarmth ? app.warmth?.communities.find(w => w.community === community) : undefined);
  const privateGroup = $derived(community ? app.privateGroups.find(g => g.community === community) : undefined);
  const privateStatus = $derived(community ? app.privateSync[community] : undefined);
  const line = $derived(privateGroup ? privateLine(privateGroup, privateStatus, RELAY_URLS.length, app.platformName) : undefined);
  const hasPublicCommunity = $derived(app.joined.some(c => (!community || c.id === community) && !app.privateGroups.some(g => g.community === c.id)));
  const needsPublicConnection = $derived(community ? !canPublishPublic(app.net, community) : !app.joined.some(c => canPublishPublic(app.net, c.id)));
  const hidden = $derived(!privateGroup && syncNotice.isHidden(status));
  const uid = $props.id();
  const panel = `sync-more-${uid}`;
</script>

{#if privateGroup}
<div class={['sync-notice', line?.connected && 'connected', line?.trouble && 'degraded']} aria-label="Sync status" role="status">
  <div class="line"><span class="dot" aria-hidden="true"></span><strong class="title">{privateGroup.active ? 'Private community' : 'Former private member'}</strong></div>
  <p class="detail">{line?.text} {#if privateGroup.active && line?.connected}Your {app.platformName} checks every update it receives.{/if}</p>
</div>
{:else if !hidden}
<div class={['sync-notice', status.state]} aria-label="Sync status" role="status">
  <div class="line">
    <span class="dot" aria-hidden="true"></span>
    <strong class="title">{status.title}</strong>
    {#if warmth}<span class="chip"><Warmth w={warmth} compact /></span>{/if}
    <span class="ctl">
      <button type="button" class="more-btn" aria-expanded={syncNotice.open} aria-controls={panel} onclick={() => (syncNotice.open = !syncNotice.open)}>{syncNotice.open ? 'Less' : 'Details'}</button>
      <button type="button" class="x" aria-label="Hide this notice until the connection changes" title="Hide until the connection changes. It stays on the You page." onclick={() => syncNotice.hide(status)}>×</button>
    </span>
  </div>
  {#if syncNotice.open}
    <div class="more" id={panel}>
      <p class="detail">{status.detail}</p>
      {#if needsPublicConnection && app.publicConnection.message}<p class="detail">{app.publicConnection.message}</p>{/if}
      {#if hasPublicCommunity && postsThroughOthers(app.net, app.boot?.platform ?? 'web')}<p class="detail">Nobody can read what you post from this {app.platformName} until a home node carrying the community has it. <a class="textlink" href="https://hoi.sh/download/" target="_blank" rel="noopener">Your own</a> keeps your posts available while you're away.</p>{/if}
      {#if app.privateGroups.length && !community}<p class="detail">Private communities sync on their own, through encrypted mailboxes on public nodes that can’t read them.</p>{/if}
      <div class="sync-actions">
        {#if needsPublicConnection && hasPublicCommunity && REGISTRY_URL}<button type="button" class="pill pill-sm" disabled={app.publicConnection.busy} onclick={() => app.connectPublic(community, true)}>{app.publicConnection.busy ? 'Finding public relays…' : 'Retry public relays'}</button>{/if}
        <a class="textlink" href={to.registry()}>Find communities</a>
        <a class="textlink" href={to.you()}>{status.action}</a>
      </div>
    </div>
  {/if}
</div>
{/if}

<style>
  .sync-notice { border: 2px solid var(--line); border-radius: 14px; padding: 6px 6px 6px 14px; margin-bottom: 18px; background: var(--soft); font-size: 13px; }
  .sync-notice.offline, .sync-notice.degraded { border-color: var(--ink); background: var(--peach); color: var(--tagink); }
  .line { display: flex; align-items: center; gap: 6px 10px; min-height: 36px; }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--mute); flex: 0 0 8px; }
  .connected .dot { background: var(--ink); }
  .offline .dot, .degraded .dot { background: var(--coral); box-shadow: 0 0 0 2px var(--ink); }
  .title { font-size: 14px; flex: 1 1 auto; min-width: 0; line-height: 1.3; overflow-wrap: anywhere; }
  .chip { flex: 0 0 auto; display: inline-flex; }
  .ctl { display: inline-flex; align-items: center; gap: 2px; flex: 0 0 auto; margin-left: auto; }
  .more-btn { min-height: 36px; padding: 0 10px; border-radius: 999px; font-weight: 700; color: var(--cobalt); }
  .offline .more-btn, .degraded .more-btn { color: inherit; text-decoration: underline; text-underline-offset: 3px; }
  .x { width: 36px; height: 36px; border-radius: 50%; font-size: 20px; line-height: 1; font-weight: 700; color: inherit; display: inline-flex; align-items: center; justify-content: center; }
  .more-btn:hover, .x:hover { background: color-mix(in srgb, var(--ink) 8%, transparent); }
  .more { padding: 2px 8px 8px 0; }
  .detail { margin: 6px 0 0; line-height: 1.45; overflow-wrap: anywhere; max-width: 72ch; }
  .sync-notice > .detail { padding: 0 8px 6px 0; }
  .sync-actions { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 16px; margin-top: 10px; }
  @media (max-width: 640px) {
    .sync-notice { margin-bottom: 14px; }
    .more-btn, .x { min-height: 40px; }
    .x { width: 40px; }
  }
  @media (max-width: 420px) {
    /* Warmth drops under the title rather than squeezing it. */
    .line { flex-wrap: wrap; }
    .title { flex-basis: calc(100% - 140px); }
    .chip { order: 3; margin-left: 18px; }
  }
</style>
