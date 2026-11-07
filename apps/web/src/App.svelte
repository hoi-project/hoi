<script lang="ts">
  import { onMount } from 'svelte';
  import { app } from './lib/app.svelte';
  import { router } from './lib/router.svelte';
  import { screenKey } from './lib/router';
  import TopBar from './lib/ui/TopBar.svelte';
  import Side from './lib/ui/Side.svelte';
  import BottomNav from './lib/ui/BottomNav.svelte';
  import Footer from './lib/ui/Footer.svelte';
  import Toasts from './lib/ui/Toasts.svelte';
  import UpdateBanner from './lib/ui/UpdateBanner.svelte';
  import PreviewBanner from './lib/ui/PreviewBanner.svelte';
  import SandboxBanner from './lib/ui/SandboxBanner.svelte';
  import { SANDBOX } from './lib/sandbox';
  import Boot from './routes/Boot.svelte';
  import Unlock from './routes/Unlock.svelte';
  import FirstRun from './routes/FirstRun.svelte';
  import Home from './routes/Home.svelte';
  import Communities from './routes/Communities.svelte';
  import Registry from './routes/Registry.svelte';
  import Community from './routes/Community.svelte';
  import Ledger from './routes/Ledger.svelte';
  import NewCommunity from './routes/NewCommunity.svelte';
  import PrivateCommunities from './routes/PrivateCommunities.svelte';
  import Join from './routes/Join.svelte';
  import PublicJoin from './routes/PublicJoin.svelte';
  import Post from './routes/Post.svelte';
  import You from './routes/You.svelte';
  import Saved from './routes/Saved.svelte';
  import Search from './routes/Search.svelte';
  import Person from './routes/Person.svelte';
  import Pair from './routes/Pair.svelte';
  import NodeLink from './routes/NodeLink.svelte';
  import Server from './routes/Server.svelte';
  import PostingNotice from './lib/ui/PostingNotice.svelte';
  import Companion from './routes/Companion.svelte';
  import NotFound from './routes/NotFound.svelte';
  import LinkAskPrompt from './lib/ui/LinkAskPrompt.svelte';
  import SyncNotice from './lib/ui/SyncNotice.svelte';
  import OperatorNotice from './lib/ui/OperatorNotice.svelte';
  import NetworkSafety from './routes/NetworkSafety.svelte';
  import Privacy from './routes/Privacy.svelte';
  import { networkConsent } from './lib/consent.svelte';

  const r = $derived(router.route);
  let main: HTMLElement | undefined = $state();
  let started = false;

  // Nothing that connects starts before the Network & Safety notice was read: the
  // core (and so its links to nodes) is opened only once it was acknowledged on this device.
  onMount(() => {
    $effect(() => {
      if (networkConsent.accepted && !started) {
        started = true;
        void app.start();
      }
    });
  });

  function skip() {
    main?.focus();
  }
</script>

{#if r.name === 'privacy'}
<Privacy />
{:else if !networkConsent.accepted}
<NetworkSafety />
{:else}
<PreviewBanner />
{#if (app.boot?.preservedRecords ?? 0) > 0}
  <div class="storage-notice" role="status">
    <strong>Some saved data needs compatible software.</strong>
    {app.boot?.preservedRecords} unsupported records remain stored on this device. This build cannot display or send them.
    Back up this storage before changing versions; do not reset it to recover these records.
  </div>
{/if}
{#if SANDBOX}
  <SandboxBanner />
{/if}

{#if app.phase === 'locked'}
  <Unlock />
{:else if app.phase !== 'ready'}
  <Boot kind={app.phase} />
{:else if r.name === 'companion'}
  <Companion />
{:else if !app.me && r.name === 'pair'}
  <Pair query={r.query} />
{:else if !app.me}
  <FirstRun route={r} />
{:else}
  <button type="button" class="skip pill pill-ink" onclick={skip}>Skip to the main view</button>
  <TopBar />
  <div class="app gutter">
    <Side />
    <main id="main" bind:this={main} tabindex="-1">
      {#if r.name !== 'post' && r.name !== 'private' && r.name !== 'registry' && r.name !== 'server' && r.name !== 'you' && !(r.name === 'new' && r.kind === 'private') && !(r.name === 'search' && r.scope === 'communities')}<SyncNotice community={r.name === 'community' || r.name === 'ledger' ? r.id : r.name === 'person' ? r.community : undefined} showWarmth={r.name !== 'community'} />{/if}
      {#key screenKey(r)}
        {#if r.name === 'home'}
          <Home />
        {:else if r.name === 'communities'}
          <Communities />
        {:else if r.name === 'registry'}
          <Registry q={r.q} />
        {:else if r.name === 'community'}
          <Community id={r.id} tab={r.tab} page={r.page} />
        {:else if r.name === 'ledger'}
          <Ledger id={r.id} />
        {:else if r.name === 'new'}
          <NewCommunity kind={r.kind} />
        {:else if r.name === 'private'}
          <PrivateCommunities />
        {:else if r.name === 'join'}
          <Join query={r.query} />
        {:else if r.name === 'public'}
          <PublicJoin query={r.query} />
        {:else if r.name === 'post'}
          <Post id={r.id} />
        {:else if r.name === 'you'}
          <You />
        {:else if r.name === 'saved'}
          <Saved />
        {:else if r.name === 'search'}
          <Search q={r.q} scope={r.scope} />
        {:else if r.name === 'person'}
          <Person key={r.key} community={r.community} />
        {:else if r.name === 'pair'}
          <Pair query={r.query} />
        {:else if r.name === 'node'}
          <NodeLink query={r.query} />
        {:else if r.name === 'server'}
          <Server />
        {:else if r.name === 'notfound'}
          <NotFound path={r.path} />
        {/if}
      {/key}
    </main>
  </div>
  <Footer />
  <BottomNav />
{/if}
<LinkAskPrompt />
<PostingNotice />
<OperatorNotice />
<UpdateBanner />
{/if}
<Toasts />

<style>
  .storage-notice { padding: 12px 16px; background: var(--card); border-bottom: 2px solid var(--ink); color: var(--ink); font-size: 13px; line-height: 1.5; overflow-wrap: anywhere; }
  .app { display: grid; grid-template-columns: 210px minmax(0, 1fr); gap: 48px; max-width: 1240px; margin: 0 auto; padding-block: 30px 72px; }
  main { min-width: 0; position: relative; outline: none; }
  /* The index turns into a chip strip below 1024px: beside it, the view would get under 735px,
     too narrow for the two-column views (post and side panel, settings rows) that start at 860px. */
  @media (max-width: 1024px) {
    .app { grid-template-columns: minmax(0, 1fr); gap: 20px; padding-block: 16px 72px; }
  }
  @media (max-width: 640px) {
    .app { padding-bottom: 48px; }
  }
  :global(body::before) {
    content: ''; position: fixed; inset: 0 0 auto; height: env(safe-area-inset-top, 0px); z-index: 10; background: var(--card);
  }
</style>
