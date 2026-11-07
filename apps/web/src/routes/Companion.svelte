<script lang="ts">
  // The desktop companion: the hoi app without the forum. It keeps your communities reachable
  // while this computer is on, lets your browser and phone in without copying anything, and shows
  // what passes through: amounts and times, never content. No profile needed.
  import { onMount } from 'svelte';
  import { app, setTitle } from '../lib/app.svelte';
  import { router } from '../lib/router.svelte';
  import { autostart, openInBrowser, windowFace } from '../lib/host';
  import { enterView } from '../lib/motion';
  import { toast, toastError } from '../lib/toast.svelte';
  import Logo from '../lib/ui/Logo.svelte';
  import QR from '../lib/ui/QR.svelte';
  import Switch from '../lib/ui/Switch.svelte';
  import Traffic from '../lib/ui/Traffic.svelte';
  import Warmth from '../lib/ui/Warmth.svelte';
  import OsUnlock from '../lib/ui/OsUnlock.svelte';
  import Updates from '../lib/ui/Updates.svelte';
  import { REGISTRY_URL } from '../lib/registry';
  import { directoryWarmth, linkWarmthWords } from '../lib/warmth';
  import { routerWords } from '../lib/public-node';

  setTitle('Companion');

  const net = $derived(app.net);
  const s = $derived(app.settings);
  const serving = $derived(net?.serving);
  const reachable = $derived(!!net?.rtc.length);
  const routed = $derived(routerWords(net?.portMapping));
  // The coldest community first: that is the one this computer should keep an eye on.
  const coldest = $derived(app.warmth?.communities[0]);
  const directory = $derived(directoryWarmth({
    registryConfigured: !!REGISTRY_URL,
    registryReachable: app.directory.registryReachable && Date.now() - app.directory.checkedAt < 10 * 60_000,
    announcedNodes: app.directory.announcedNodes,
    catalogNodes: app.warmth?.directory.catalogNodes ?? 0,
    discoverable: app.warmth?.directory.discoverable ?? 0,
    carryNodes: app.warmth?.directory.carryNodes ?? 0,
  }));
  let starts = $state<boolean | null>(null);
  let locking = $state(false);
  async function lock() {
    locking = true;
    try { await app.lock(); } catch (e) { toastError(e); locking = false; }
  }

  onMount(() => {
    void windowFace(false).catch(() => undefined);
    void autostart().then((v) => (starts = v), () => (starts = null));
  });

  async function openBrowser() {
    if (!serving) return;
    try {
      await openInBrowser(serving.pairingWeb);
      toast('Opened in your browser. Tap Connect there; Chrome may ask to let hoi.sh reach apps on this device: allow it.');
    } catch (err) {
      toastError(err);
    }
  }

  async function setStarts(on: boolean) {
    try {
      starts = await autostart(on);
      toast(on ? 'hoi starts with this computer now, in the corner, without a window.' : 'hoi waits until you open it.');
    } catch (err) {
      toastError(err);
    }
  }

  async function relay(on: boolean) {
    const r = await app.setSettings({ relay: on, relayNoticeSeen: true });
    if (r) toast(on ? 'Relaying again. Members can catch up through this computer.' : 'Relaying paused. Only your own devices connect here.');
  }

  // Opens the full app for now; hoi still opens as the companion unless You → Settings says otherwise.
  async function fullApp() {
    await windowFace(true).catch(() => undefined);
    router.go('#/');
  }
</script>

<div class="comp" use:enterView>
  <header class="ctop gutter">
    <span class="logo"><Logo size={26} /><span>hoi</span></span>
    <button type="button" class="pill pill-sm" onclick={lock} disabled={locking}>{locking ? 'Locking…' : 'Lock'}</button>
    <span class="sticker">companion</span>
    <span class={['tag', 'push', net?.relaying ? 'tag-mint' : 'tag-soft']}>● {net?.relaying ? 'keeping your communities reachable' : 'running'}</span>
  </header>

  <main id="main" class="wrap gutter">
    <h1 class="sr-only">hoi on this computer</h1>
    {#if s && s.relay && !s.relayNoticeSeen}
      <div class="notice frame anim">
        <p>
          <b>This computer helps your communities stay reachable.</b> While it's on, members' browsers and computers can connect to it and
          catch up, even when the person who invited them is away. They can see this computer's IP address. What passes through is encrypted on
          the way and signed by its author, so nobody can change it, and hoi keeps no log of it.
        </p>
        <div class="btnrow">
          <button type="button" class="pill pill-ink pill-sm" onclick={() => app.setSettings({ relayNoticeSeen: true })}>Good, keep it on</button>
          <button type="button" class="pill pill-sm" onclick={() => relay(false)}>Turn it off</button>
        </div>
      </div>
    {/if}

    <p class="help public anim" role="status">
      {#if s?.publicNode}<b>Public node.</b> {routed.text}{:else}<b>Not a public node:</b> this computer publishes no address of its own.{/if}
      <button type="button" class="textlink" onclick={() => fullApp().then(() => router.go('#/you'))}>{s?.publicNode ? 'Change' : 'Be one'}</button>
    </p>

    <section class="sec" aria-labelledby="c-devices">
      <h2 id="c-devices">Your browser and phone</h2>
      <div class="pair">
        <div class="frame card">
          <h3>On this computer</h3>
          <p class="help">Opens hoi in your browser and connects it here. No link to copy.</p>
          <button type="button" class="pill pill-ink" onclick={openBrowser} disabled={!serving}>Open hoi in my browser</button>
        </div>
        <div class="frame card phone">
          <div class="txt">
            <h3>Your phone</h3>
            {#if serving && reachable}
              <p class="help">Point its camera at the code. It opens hoi, connects to this computer over your Wi-Fi and can take your key with one more tap.</p>
            {:else}
              <p class="help">Connect this computer to your Wi-Fi first: a phone reaches it over your network.</p>
            {/if}
            <span class="tag tag-soft">{serving?.clients ?? 0} of your devices connected</span>
          </div>
          {#if serving && reachable}<QR value={serving.pairingWeb} label="QR code: open hoi on your phone and connect it to this computer" color="mint" />{/if}
        </div>
      </div>
    </section>

    {#if app.me}
      <section class="sec" aria-labelledby="c-warmth">
        <h2 id="c-warmth">Warmth</h2>
        <div class="frame card warmthcard">
          {#if app.warmth?.links}{@const links = linkWarmthWords(app.warmth.links, app.platformName)}<p class="dir"><span class={['tag', links.tag]}>● your connections · {links.word}</span> <span class="help">{links.summary}</span></p>{/if}
          <p class="dir"><span class={['tag', directory.tag]}>● finding communities · {directory.word}</span> <span class="help">{directory.summary}</span></p>
          {#if coldest}
            <p class="help">Coldest of your {app.warmth?.communities.length === 1 ? 'community' : `${app.warmth?.communities.length} communities`}: <a class="textlink" href={`#/c/${coldest.community}`}>c/{coldest.name || coldest.community.slice(0, 8)}</a></p>
            <Warmth w={coldest} />
          {:else}
            <p class="help">No public community on this computer yet. Warmth reads how many copies of each one this computer can see besides its own.</p>
          {/if}
        </div>
      </section>
    {/if}

    <section class="sec" aria-labelledby="c-traffic">
      <h2 id="c-traffic">What passes through this computer</h2>
      <Traffic compact />
    </section>

    {#if s}
      <section class="sec" aria-labelledby="c-settings">
        <h2 id="c-settings">Settings</h2>
        <div class="sets">
          <div class="set">
            <span class="l">Keep communities reachable</span>
            <span class="h">Members' browsers and computers may connect here and catch up. Off: only your own devices.</span>
            <span class="v"><Switch checked={s.relay} label="Keep communities reachable" onchange={relay} /></span>
          </div>
          {#if s.relay}
            <div class="set">
              <label class="l" for="c-cap">Daily cap</label>
              <span class="h">Relaying naps for the rest of the day (UTC) once members used this much.</span>
              <span class="v slider">
                <input id="c-cap" type="range" min="1" max="50" value={s.dailyCapGb} onchange={(e) => app.setSettings({ dailyCapGb: Number(e.currentTarget.value) })} />
                <span class="num">{s.dailyCapGb} GB</span>
              </span>
            </div>
          {/if}
          {#if starts !== null}
            <div class="set">
              <span class="l">Start with this computer</span>
              <span class="h">hoi waits in the corner of the screen, without a window, so your communities stay reachable.</span>
              <span class="v"><Switch checked={starts} label="Start with this computer" onchange={setStarts} /></span>
            </div>
          {/if}
          <div class="set">
            <span class="l">The whole app</span>
            <span class="h">Read and post right here instead of in the browser. {app.me ? '' : 'You make or bring a key first.'}</span>
            <span class="v"><button type="button" class="lnk lnk-ink" onclick={fullApp}>Open the full app</button></span>
          </div>
        </div>
        <OsUnlock />
        <Updates />
      </section>
    {/if}
    <p class="build mono">hoi {app.boot?.version} · companion · {app.me ? `signs as ${app.me.name}` : 'no key on this computer'}</p>
  </main>
</div>

<style>
  .comp { min-height: 100dvh; padding-bottom: 32px; }
  .ctop { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; min-height: 64px; max-width: 960px; margin: 0 auto; border-bottom: 2px solid var(--ink); padding-block: 8px; }
  .logo { display: inline-flex; align-items: center; gap: 8px; font-weight: 900; font-size: 20px; letter-spacing: -0.02em; }
  .push { margin-left: auto; white-space: normal; }
  .wrap { max-width: 960px; margin: 0 auto; display: flex; flex-direction: column; gap: 8px; }
  .sec { padding-top: 22px; display: flex; flex-direction: column; gap: 12px; }
  .sec h2 { font-size: 22px; letter-spacing: -0.03em; font-weight: 800; }
  .notice { margin-top: 18px; padding: 14px 16px; display: flex; flex-direction: column; gap: 12px; background: var(--sun); color: var(--tagink); }
  .notice p { line-height: 1.5; font-weight: 500; }
  .pair { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1.4fr); gap: 12px; }
  .card { padding: 16px; display: flex; flex-direction: column; gap: 10px; align-items: flex-start; min-width: 0; }
  .card h3 { font-size: 18px; letter-spacing: -0.02em; font-weight: 800; }
  .phone { flex-direction: row; align-items: center; gap: 16px; }
  .phone .txt { display: flex; flex-direction: column; gap: 10px; align-items: flex-start; flex: 1 1 auto; min-width: 0; }
  .phone :global(.qr) { width: 170px; }
  .slider input { width: 130px; }
  .warmthcard { gap: 8px; }
  .dir { display: flex; flex-direction: column; gap: 6px; align-items: flex-start; }
  .dir .tag { white-space: normal; }
  .build { padding-top: 22px; }
  @media (max-width: 720px) {
    .pair { grid-template-columns: 1fr; }
  }
  @media (max-width: 480px) {
    .phone { flex-direction: column; align-items: flex-start; }
    .phone :global(.qr) { width: min(240px, 100%); }
  }
</style>
