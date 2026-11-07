<script lang="ts">
  // A pairing link opened here, usually a phone's camera scanning the QR code on your computer.
  // One tap pairs this device with that home node; without a key yet, one more asks your device
  // that holds it to link this one (four digits to compare, once it answers). Shown with or
  // without a key.
  import { onMount } from 'svelte';
  import type { PairPreview } from '@hoi/core-api';
  import { app, setTitle } from '../lib/app.svelte';
  import { router } from '../lib/router.svelte';
  import { enterView } from '../lib/motion';
  import { toast, toastError } from '../lib/toast.svelte';
  import { describeHome } from '../lib/pairing';
  import LinkWaiting from '../lib/ui/LinkWaiting.svelte';
  import Logo from '../lib/ui/Logo.svelte';

  let { query }: { query: string } = $props();

  const params = $derived(new URLSearchParams(query));
  const link = $derived(`hoi://pair?${query}`);
  /** The link is masked (`code=`, no address in it): the core reads it. Older links spelled it out. */
  let preview = $state.raw<PairPreview | null>(null);
  let read = $state(false);
  $effect(() => {
    const l = link;
    preview = null;
    read = !params.get('code') && !params.get('t');
    if (read) return;
    app.core.previewPair(l).then(
      (p) => { preview = p; read = true; },
      () => { preview = null; read = true; },
    );
  });
  const name = $derived(preview?.name || 'your computer');
  const valid = $derived(!!preview && (!!preview.url || preview.routes.length > 0));
  /** Where the link really points, so a name in it ("lena's laptop") can't stand in for the address. */
  const where = $derived.by(() => describeHome(preview?.url || null, preview?.routes.join(',') || null));
  const home = $derived(app.net?.homeNode);
  /** Paired a moment ago on this screen (the token is gone from the address bar by then). */
  let justPaired = $state(false);
  /** Already paired with this home node, and this is its current link (a spent one stays paired: the core keeps the credential).
   * With no token in the address it was taken out after pairing: the screen may have mounted
   * again since (the app's frame appears once the device has a key), and shows where it stands. */
  const pairedHere = $derived(justPaired || (!!home && ((!params.get('t') && !params.get('code')) || !!preview?.pairedHere)));

  let busy = $state<'' | 'pair' | 'link'>('');
  let asking = $state(false);
  let label = $state(/iPhone|Android.*Mobile/i.test(navigator.userAgent) ? 'Phone' : /iPad|Android/i.test(navigator.userAgent) ? 'Tablet' : 'Browser');

  onMount(() => setTitle('Connect'));

  async function connect() {
    busy = 'pair';
    try {
      app.net = await app.core.pair(link);
      toast(`Paired with ${name}. Connecting…`);
      // The pairing token is a bearer secret: once used, it leaves the address bar and the
      // browser's history (synced histories included). The screen stays, from a clean route.
      justPaired = true;
      router.replace('#/pair');
    } catch (err) {
      toastError(err);
    }
    busy = '';
  }

  async function linkThis() {
    busy = 'link';
    try {
      app.linkChecks = [];
      await app.core.linkHome(label.trim() || 'Phone');
      asking = true;
    } catch (err) {
      toastError(err);
    }
    busy = '';
  }

  async function stop() {
    asking = false;
    app.linkChecks = [];
    await app.core.linkCancel().catch(() => undefined);
  }

  const via = $derived(home?.via === 'webrtc' ? 'over WebRTC, encrypted end to end' : home?.via === 'websocket' ? 'on this device' : '');
</script>

<div class={['pairview', !app.me && 'alone']} use:enterView>
  {#if !app.me}
    <header class="ptop gutter">
      <span class="logo"><Logo size={30} /><span>hoi<span class="sh">.sh</span></span></span>
    </header>
  {/if}
  <section class={['wrap', !app.me && 'gutter']} aria-labelledby="pair-h">
    <span class="sticker anim">one scan, no codes</span>
    <h1 id="pair-h" class="anim">Connect to {name}.</h1>
    {#if !read && !pairedHere}
      <p class="lead anim">Reading the link…</p>
    {:else if !valid && !pairedHere}
      <p class="callout anim">This link is missing its address or token. Scan the code on your computer again.</p>
    {:else if !pairedHere}
      <p class="lead anim">
        This {app.platformName} then keeps your communities in sync through {name} while it's on. They talk over an encrypted connection,
        straight to each other: nothing in between, no account.
      </p>
      <p class={['frame', 'where', 'anim', where.scope === 'internet' && 'far']}>
        {#if where.scope === 'device'}This link points at <b>this device</b> (<span class="mono">{where.host}</span>).
        {:else if where.scope === 'network'}This link points at a computer <b>on your network</b> (<span class="mono">{where.host}</span>).
        {:else}This link points at a computer <b>on the internet</b> (<span class="mono">{where.host}</span>). Pair only if you run that node or know who does: everything you post and every community you're in would pass through it.{/if}
      </p>
      <div class="btnrow anim">
        <button type="button" class="pill pill-ink pill-lg" onclick={connect} disabled={!!busy}>{busy === 'pair' ? 'Connecting…' : where.scope === 'internet' ? `Yes, connect to ${where.host}` : 'Connect'}</button>
        <a class="pill pill-lg" href={app.me ? '#/you' : '#/'}>Not now</a>
      </div>
      <p class="help anim">Only connect to your own computer or home node. This link pairs one device, then it is spent; the name in it is whatever the link says.</p>
    {:else}
      <div class="frame status anim" role="status">
        <span class={['tag', home?.connected ? 'tag-mint' : 'tag-peach']}>{home?.connected ? '● connected' : '● connecting…'}</span>
        <span>{home?.connected ? `Through ${name}${via ? `, ${via}` : ''}.` : home?.error ? home.error : `Looking for ${name} on your network…`}</span>
      </div>

      {#if !app.me}
        {#if asking}
          <div class="anim"><LinkWaiting home={name} oncancel={stop} /></div>
        {:else}
          <h2 class="anim">Use the key you already have?</h2>
          <p class="lead anim">Your other device blesses this one with your key: same name, same communities, nothing to type.</p>
          <label class="field anim namef">
            <span>Call this device</span>
            <input class="input" bind:value={label} maxlength="32" />
          </label>
          <div class="btnrow anim">
            <button type="button" class="pill pill-ink pill-lg" onclick={linkThis} disabled={!!busy || !home?.connected}>
              {busy === 'link' ? 'Asking…' : `Link this ${app.platformName}`}
            </button>
            <a class="textlink" href="#/">or make a new key instead</a>
          </div>
          {#if !home?.connected}<p class="help">The button wakes up once {name} answers.</p>{/if}
        {/if}
      {:else}
        <p class="lead anim">You're set. Posts made here travel through {name} to your communities, and back.</p>
        <div class="btnrow anim">
          <button type="button" class="pill pill-ink" onclick={() => router.go('#/')}>Open hoi</button>
          <a class="textlink" href="#/you">See the connection</a>
        </div>
      {/if}
    {/if}
  </section>
</div>

<style>
  .pairview.alone { min-height: 100dvh; padding-top: env(safe-area-inset-top, 0px); padding-bottom: max(40px, env(safe-area-inset-bottom, 0px)); }
  .ptop { display: flex; align-items: center; height: 72px; max-width: 1120px; margin: 0 auto; border-bottom: 2px solid var(--ink); }
  .logo { display: inline-flex; align-items: center; gap: 10px; font-weight: 900; font-size: 21px; letter-spacing: -0.02em; }
  .sh { color: var(--mute); font-weight: 600; }
  .wrap { max-width: 720px; margin: 0 auto; padding-block: clamp(28px, 6vw, 72px) 24px; display: flex; flex-direction: column; gap: 18px; align-items: flex-start; }
  .pairview:not(.alone) .wrap { margin: 0; padding-top: 8px; }
  h1 { font-size: clamp(36px, 7vw, 64px); line-height: 1; max-width: 12em; }
  h2 { font-size: 26px; letter-spacing: -0.03em; font-weight: 800; padding-top: 10px; }
  .lead { font-size: 18px; line-height: 1.45; color: var(--mute); max-width: 34em; font-weight: 500; }
  .status { padding: 12px 14px; display: flex; gap: 10px; align-items: center; flex-wrap: wrap; font-weight: 600; width: 100%; }
  .where { padding: 12px 14px; line-height: 1.5; max-width: 34em; }
  .where.far { background: var(--peach); }
  .namef { width: min(320px, 100%); }
  .btnrow .textlink { padding: 0 6px; }
</style>
