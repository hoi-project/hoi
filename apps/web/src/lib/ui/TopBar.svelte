<script lang="ts">
  import { app } from '../app.svelte';
  import { router } from '../router.svelte';
  import { to, type SearchScope } from '../router';
  import Avatar from './Avatar.svelte';
  import { pulse } from '../motion';
  import Logo from './Logo.svelte';
  import { startPost } from '../compose';
  import { toastError } from '../toast.svelte';
  import { RELAY_URLS } from '../private-relay';
  import { privateLine } from '../private-status';
  let locking = $state(false);
  async function lock() {
    locking = true;
    try { await app.lock(); } catch (e) { toastError(e); locking = false; }
  }

  let q = $state('');
  let searchScope = $state<SearchScope>('posts');
  $effect(() => {
    if (router.route.name === 'search') { q = router.route.q; searchScope = router.route.scope ?? 'posts'; }
    else if (router.route.name === 'registry') { q = router.route.q; searchScope = 'communities'; }
  });
  const route = $derived(router.route);
  const privateGroup = $derived(route.name === 'community' || route.name === 'ledger' ? app.privateGroups.find(g => g.community === route.id) : undefined);
  const line = $derived(privateGroup ? privateLine(privateGroup, app.privateSync[privateGroup.community], RELAY_URLS.length, app.platformName) : undefined);
  const connected = $derived(line ? line.connected : !!app.net && app.net.peers > 0);
  const trouble = $derived(line ? line.trouble : !!app.net?.transport?.error || (!!app.net?.homeNode && !app.net.homeNode.connected));
  const statusLabel = $derived(line ? line.label : route.name === 'private' ? 'Private communities' : app.net?.peers ? app.net.label : 'Public peers offline');

  function search(e: SubmitEvent) {
    e.preventDefault();
    router.go(to.search(q.trim(), searchScope));
  }

</script>

<header class="top">
  <div class="bar gutter">
    <a href="#/" class="logo" aria-label="hoi.sh, home"><Logo /><span>hoi<span class="sh">.sh</span></span></a>
    <form class="search" role="search" aria-label="Search everywhere" onsubmit={search}>
      <label class="sr-only" for="q">Search</label>
      <label class="sr-only" for="search-scope">Search in</label>
      <select id="search-scope" bind:value={searchScope}><option value="posts">Posts</option><option value="communities">Communities</option></select>
      <input id="q" type="search" bind:value={q} placeholder={searchScope === 'communities' ? 'Search communities' : 'Search synced posts'} autocomplete="off" />
    </form>
    <a class="iconbtn searchlink" href={to.search('', searchScope)} aria-label="Search posts or communities">
      <svg width="20" height="20" viewBox="0 0 24 24" aria-hidden="true"><circle cx="10.5" cy="10.5" r="6.5" fill="none" stroke="currentColor" stroke-width="2.4" /><line x1="15.5" y1="15.5" x2="21" y2="21" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" /></svg>
    </a>
    <a href={privateGroup || route.name === 'private' ? '#/private' : '#/you'} class={['relay', connected && 'on', trouble && 'trouble']} title={statusLabel}>
      <i use:pulse={connected}></i><span class="rl">{statusLabel}</span>
    </a>
    <button type="button" class="pill pill-ink newpost" onclick={() => startPost()} disabled={privateGroup && !privateGroup.active}>+ Post</button>
    <a href="#/you" class="me" aria-label={`You · ${app.me?.name ?? ''}`} title={`Signed in as ${app.me?.name ?? ''} · ${app.me?.rootKey.slice(0, 12) ?? ''}`}><Avatar name={app.me?.name ?? ''} keyHex={app.me?.rootKey} avatar={app.me?.avatar} avatarColor={app.me?.avatarColor} accountBadge /></a>
    <button type="button" class="pill lock" onclick={lock} disabled={locking}>{locking ? 'Locking…' : 'Lock'}</button>
  </div>
</header>

<style>
  .top { position: sticky; top: env(safe-area-inset-top, 0px); z-index: 9; background: var(--card); border-bottom: 2px solid var(--ink); }
  .bar { display: flex; align-items: center; gap: 16px; height: 66px; max-width: 1240px; margin: 0 auto; }
  .logo { display: inline-flex; align-items: center; gap: 9px; font-weight: 900; font-size: 19px; letter-spacing: -0.02em; flex: 0 0 auto; }
  .sh { color: var(--mute); font-weight: 600; }
  .search { flex: 1 1 180px; max-width: 420px; display: flex; align-items: center; background: var(--soft); border-radius: 999px; height: 40px; padding: 0 14px; }
  .search input { flex: 1; min-width: 0; background: transparent; border: 0; outline: 0; font-size: 14px; font-weight: 500; color: var(--ink); }
  .search select { max-width: 115px; min-height: 36px; border: 0; border-right: 1px solid var(--line); background: transparent; color: var(--ink); font-size: 12px; font-weight: 700; margin-right: 9px; padding-right: 4px; }
  .search:focus-within { outline: 3px solid var(--cobalt); outline-offset: 2px; }
  .iconbtn { display: none; width: 44px; height: 44px; border-radius: 50%; align-items: center; justify-content: center; flex: 0 0 auto; }
  .iconbtn:hover { background: var(--soft); }
  .relay {
    display: inline-flex; align-items: center; gap: 7px; background: var(--soft); color: var(--ink); border-radius: 999px; padding: 5px 11px;
    font-weight: 700; font-size: 13px; white-space: nowrap; margin-left: auto; min-width: 0; box-shadow: inset 0 0 0 2px var(--line);
  }
  .relay.on { background: var(--mint); color: var(--tagink); box-shadow: none; }
  .relay.trouble { background: var(--peach); color: var(--tagink); box-shadow: none; }
  .relay i { width: 8px; height: 8px; border-radius: 50%; background: currentColor; display: inline-block; flex: 0 0 8px; }
  .relay:not(.on) i { background: var(--mute); }
  .rl { overflow: hidden; text-overflow: ellipsis; min-width: 0; }
  .me {
    width: 44px; height: 44px; border-radius: 50%; display: inline-flex; align-items: center;
    justify-content: center; font-weight: 800; font-size: 12px; color: var(--tagink); flex: 0 0 auto;
  }
  @media (max-width: 760px) {
    .search { display: none; }
    .iconbtn { display: inline-flex; margin-left: auto; }
    .relay { margin-left: 0; }
    .bar { gap: 8px; }
  }
  @media (max-width: 640px) {
    .newpost { display: none; }
    .me { width: 44px; height: 44px; }
  }
  @media (max-width: 360px) {
    .sh { display: none; }
    .relay { padding: 5px 8px; font-size: 12px; }
  }
</style>
