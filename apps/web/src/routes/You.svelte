<script lang="ts">
  import Labelers from '../lib/ui/Labelers.svelte';
  import ListKeepers from '../lib/ui/ListKeepers.svelte';
  import type { Settings, VpnInterface } from '@hoi/core-api';
  import { onMount } from 'svelte';
  import { bytes, fadesIn } from '@hoi/core-api';
  import { app, attempt, setTitle } from '../lib/app.svelte';
  import { enterView, popIn } from '../lib/motion';
  import { router } from '../lib/router.svelte';
  import { toast, toastError } from '../lib/toast.svelte';
  import Switch from '../lib/ui/Switch.svelte';
  import Modal from '../lib/ui/Modal.svelte';
  import LinkDevice from '../lib/ui/LinkDevice.svelte';
  import KeyBackup from '../lib/ui/KeyBackup.svelte';
  import RecoveryFriends from '../lib/ui/RecoveryFriends.svelte';
  import PasskeyUnlock from '../lib/ui/PasskeyUnlock.svelte';
  import OsUnlock from '../lib/ui/OsUnlock.svelte';
  import Updates from '../lib/ui/Updates.svelte';
  import QR from '../lib/ui/QR.svelte';
  import CopyField from '../lib/ui/CopyField.svelte';
  import Traffic from '../lib/ui/Traffic.svelte';
  import ConnectionDetails from '../lib/ui/ConnectionDetails.svelte';
  import Avatar from '../lib/ui/Avatar.svelte';
  import ProfilePicture from '../lib/ui/ProfilePicture.svelte';
  import Warmth from '../lib/ui/Warmth.svelte';
  import KeyWork from '../lib/ui/KeyWork.svelte';
  import Devices from '../lib/ui/Devices.svelte';
  import Addresses from '../lib/ui/Addresses.svelte';
  import BrowserAddresses from '../lib/ui/BrowserAddresses.svelte';
  import NetworkMap from '../lib/ui/NetworkMap.svelte';
  import { nudge } from '../lib/motion';
  import { openInBrowser } from '../lib/host';
  import { REGISTRY_URL } from '../lib/registry';
  import { directoryWarmth, linkWarmthWords } from '../lib/warmth';
  import { routerWords, whoSees } from '../lib/public-node';
  import { to } from '../lib/router';
  import { networkConsent } from '../lib/consent.svelte';
  import type { NoticeView } from '@hoi/core-api';
  import { copyText, plural } from '../lib/util';
  import { reduced } from '../lib/motion';
  import { syncStatus } from '../lib/network-status';
  import { syncNotice } from '../lib/sync-notice.svelte';

  setTitle('You');

  // A long page: a row of its sections stays in reach while scrolling, on a phone too.
  const sections = $derived([
    { id: 'y-sec-key', label: 'Key' },
    { id: 'y-sec-conn', label: 'Network' },
    { id: 'y-sec-warmth', label: 'Warmth' },
    { id: 'y-sec-traffic', label: 'Traffic' },
    { id: 'y-sec-trouble', label: 'Troubleshoot' },
    ...(app.settings ? [{ id: 'y-sec-privacy', label: 'Privacy' }, { id: 'y-sec-looks', label: 'Looks' }] : []),
    { id: 'y-sec-device', label: `This ${app.platformName}` },
  ]);
  function jump(id: string) {
    const el = document.getElementById(id);
    if (!el) return;
    // Troubleshooting is folded away: going there opens it.
    const fold = el.matches('details') ? (el as HTMLDetailsElement) : el.querySelector('details');
    if (id === 'y-sec-trouble' && fold) fold.open = true;
    el.scrollIntoView({ block: 'start', behavior: reduced() ? 'auto' : 'smooth' });
    const heading = el.querySelector<HTMLElement>('h1, h2, summary');
    heading?.focus({ preventScroll: true });
  }
  /** What the sync notice above the feeds says, all of it, also once its × hid it. */
  const sync = $derived(syncStatus(app.net));

  const me = $derived(app.me!);
  const s = $derived(app.settings);
  const web = $derived(app.boot?.platform === 'web');
  const net = $derived(app.net);
  // Warmth: communities coldest first, and how findable communities are from here.
  const warmth = $derived(app.warmth);
  const directory = $derived(directoryWarmth({
    registryConfigured: !!REGISTRY_URL,
    registryReachable: app.directory.registryReachable && Date.now() - app.directory.checkedAt < 10 * 60_000,
    announcedNodes: app.directory.announcedNodes,
    catalogNodes: warmth?.directory.catalogNodes ?? 0,
    discoverable: warmth?.directory.discoverable ?? 0,
    carryNodes: warmth?.directory.carryNodes ?? 0,
  }));

  // The warmth of this device's own connections: places and checked links, in plain words.
  const links = $derived(warmth?.links ? linkWarmthWords(warmth.links, app.platformName) : null);

  onMount(() => { void app.refresh(); void app.refreshWarmth(); });

  async function loadBlocklist(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const f = input.files?.[0];
    if (!f) return;
    if (f.size > 5 * 1024 * 1024) {
      toast('Choose a text fingerprint list smaller than 5 MB. Pictures cannot be imported here.');
      input.value = '';
      return;
    }
    const n = await attempt(async () => app.core.importBlocklist(await f.text()));
    if (n !== undefined) {
      toast(n ? `Loaded ${n} picture ${n === 1 ? 'fingerprint' : 'fingerprints'}. Matching pictures are refused on this ${app.platformName} from now on.` : 'Nothing new in that list.');
      void app.refresh();
    }
    input.value = '';
  }

  // ---------------------------------------------------------- name
  let editingName = $state(false);
  let nameDraft = $state('');
  async function saveName(e: SubmitEvent) {
    e.preventDefault();
    const m = await attempt(() => app.core.setName(nameDraft), (v) => `Hi, ${v.name}. New posts carry the new name.`);
    if (m) {
      app.me = m;
      editingName = false;
    }
  }

  // ---------------------------------------------------------- fingerprint and burning
  async function copyFingerprint(e: MouseEvent) {
    if (await copyText(me.fingerprint)) {
      nudge(e.currentTarget as Element);
      toast('Fingerprint copied.');
    } else toast('Copy did not work here. Select it and copy it yourself.');
  }
  let burnOpen = $state(false);
  let burnConfirm = $state('');
  async function burn() {
    busy = true;
    const ok = await attempt(() => app.core.burnIdentity(burnConfirm), 'Burned. Every device that hears of it drops what this key signed.');
    if (ok !== undefined) {
      burnOpen = false;
      burnConfirm = '';
      app.me = await app.core.me();
    }
    busy = false;
  }

  // ---------------------------------------------------------- devices
  let linkOpen = $state(false);

  // ---------------------------------------------------------- key backup
  let backupOpen = $state(false);
  let friendsOpen = $state(false);
  let importOpen = $state(false);
  let busy = $state(false);

  // ---------------------------------------------------------- home node
  let pairLink = $state('');
  let pairing = $state(false);
  async function pair(e: SubmitEvent) {
    e.preventDefault();
    pairing = true;
    try {
      const n = await app.core.pair(pairLink.trim());
      app.net = n;
      pairLink = '';
      // The socket may still be coming up; the relay pill follows the `net` events from here.
      const name = n.homeNode?.name || 'your home node';
      toast(n.homeNode?.connected ? `Connected to ${name}. Checking for missing posts…` : `Paired. Connecting to ${name}…`);
    } catch (err) {
      toastError(err);
    }
    pairing = false;
  }
  async function unpair() {
    const n = await attempt(() => app.core.unpair(), 'Home node unpaired.');
    if (n) app.net = n;
  }
  async function rotate() {
    const n = await attempt(() => app.core.rotatePairing(), 'New pairing link made. Devices paired with the old one pair again.');
    if (n) app.net = n;
  }

  // ---------------------------------------------------------- settings
  let relayNotice = $state(false);
  /** The core's own default (Settings::default). */
  const DEFAULT_CAP_GB = 5;
  async function set(patch: Partial<Settings>, msg?: string) {
    const r = await app.setSettings(patch);
    if (r && msg) toast(msg);
  }
  function relaySwitch(next: boolean) {
    if (next) relayNotice = true;
    else void set({ relay: false, relayNoticeSeen: true }, 'Relaying paused. Only your own devices connect here now.');
  }

  // ---------------------------------------------------------- public node
  // Turning it on asks the router first, without publishing, so the person sees the very
  // address that will go into invites and the node link before saying yes.
  let publicAsk = $state(false);
  const routed = $derived(routerWords(net?.portMapping));
  const seen = $derived(whoSees(net));
  const unreportedAddress = $derived(net?.connections?.some((connection) =>
    connection.connected && connection.encryption !== 'stays on this device' && connection.transport !== 'webrtc') ?? false);
  async function publicSwitch(next: boolean) {
    if (!next) {
      await set({ publicNode: false }, `Not a public node any more. Your router's forwards are removed, and this ${app.platformName} publishes no address of its own.`);
      return;
    }
    publicAsk = true;
    const n = await attempt(() => app.core.previewPublicNode(true));
    if (n) app.net = n;
  }
  async function publicCancel() {
    publicAsk = false;
    const n = await attempt(() => app.core.previewPublicNode(false));
    if (n) app.net = n;
  }
  // ---------------------------------------------------------- mailboxes for others (SCATTER.md §5.7)
  const HOLD_SIZES = [1, 2, 5, 10, 20];
  const heldText = $derived(
    app.stats?.heldBudget
      ? `Keeps ${(app.stats.heldPieces ?? 0).toLocaleString('en')} ${app.stats.heldPieces === 1 ? 'message or piece' : 'messages and pieces'} for others · ${bytes(app.stats.heldBytes ?? 0)} of ${bytes(app.stats.heldBudget)}. None of it readable here.`
      : 'Opening the mailboxes…',
  );
  async function holdSwitch(next: boolean) {
    if (next) await set({ holdPiecesGb: 2 }, `This ${app.platformName} can keep mailboxes for others now, up to 2 GB.`);
    else await set({ holdPiecesGb: 0 }, `No mailboxes for others any more. What this ${app.platformName} kept for them is deleted.`);
  }
  async function holdSize(gb: number) {
    await set({ holdPiecesGb: gb }, `Up to ${gb} GB for others' mailboxes.`);
  }
  // ---------------------------------------------------------- only through my VPN
  let vpnPick = $state(false);
  let vpnList = $state<VpnInterface[]>([]);
  let vpnChoice = $state('');
  const vpnText = $derived(
    net?.vpn
      ? net.vpn.up
        ? `${net.vpn.interface} is up (${net.vpn.addresses.join(', ')}): every connection to the internet goes through it.`
        : `${net.vpn.interface} ${net.vpn.detail ?? 'is down'}. Nothing goes to the internet until it does.`
      : 'Looking at the VPN…',
  );
  async function vpnSwitch(next: boolean) {
    if (!next) {
      vpnPick = false;
      await set({ vpn: '' }, `Every network again. Your VPN app's own kill switch still works, if it has one.`);
      return;
    }
    const list = await attempt(() => app.core.vpnInterfaces());
    if (!list) return;
    vpnList = list;
    vpnChoice = list.find((i) => i.tunnel)?.name ?? '';
    vpnPick = true;
  }
  async function pickVpn() {
    if (!vpnChoice) return;
    vpnPick = false;
    await set({ vpn: vpnChoice }, `Only through ${vpnChoice} from now on.`);
  }
  async function publicConfirm() {
    publicAsk = false;
    await set({ publicNode: true, relayNoticeSeen: true }, `Public. People outside your home can reach this ${app.platformName} now.`);
  }

  // ---------------------------------------------------------- operators' notices
  let notices = $state.raw<NoticeView[]>([]);
  async function loadNotices() {
    try { notices = await app.core.nodeNotices(); }
    catch { notices = []; }
  }
  $effect(() => {
    void app.net;
    void loadNotices();
  });
  async function answerNotice(node: string, hash: string, accept: boolean) {
    try {
      if (accept) await app.core.acceptNotice(node, hash);
      else await app.core.declineNotice(node);
      await loadNotices();
      toast(accept ? 'Accepted. The node is contacted again.' : 'Declined. The node is left alone.');
    } catch (e) { toastError(e); }
  }
  async function forgetNotice(node: string) {
    try {
      await app.core.forgetNotice(node);
      await loadNotices();
      toast('Forgotten. It asks again the next time it is reached.');
    } catch (e) { toastError(e); }
  }

  // ---------------------------------------------------------- wipe
  let wipeStep = $state(0);
  async function wipe() {
    busy = true;
    try {
      await app.core.wipe();
      wipeStep = 0;
      await app.wiped();
      router.go('#/');
      toast(`Wiped. Nothing of yours is left on this ${app.platformName}.`);
    } catch (err) {
      toastError(err);
    }
    busy = false;
  }
  // A name that is a domain can be verified; the same rule as the core's.
  const domainName = $derived.by(() => {
    const d = (me.name ?? '').trim().replace(/\.$/, '').toLowerCase();
    const labels = d.split('.');
    const ok = d.length <= 253 && labels.length >= 2 && labels.every((l) => /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(l));
    const tld = labels[labels.length - 1] ?? '';
    return ok && (/^[a-z]{2,}$/.test(tld) || tld.startsWith('xn--')) ? d : '';
  });
</script>

{#snippet wipeList()}
  <ul class="wipes" aria-label="What a wipe removes">
    <li><span class="tag tag-coral">your key</span><span>{me.hasRoot ? 'root key' : 'device key'} · {me.identityBits} bits of work</span></li>
    <li><span class="tag tag-sky">records</span><span><b class="num">{(app.stats?.records ?? 0).toLocaleString('en')}</b> in <b class="num">{app.stats?.communities ?? 0}</b> {app.stats?.communities === 1 ? 'community' : 'communities'} · {bytes(app.stats?.storedBytes ?? 0)}</span></li>
    <li><span class="tag tag-lilac">media</span><span>{bytes(app.stats?.mediaBytes ?? 0)}</span></li>
    {#if app.privateGroups.length}<li><span class="tag tag-lilac">private</span><span><b class="num">{app.privateGroups.length}</b> private {app.privateGroups.length === 1 ? 'community' : 'communities'} and their keys</span></li>{/if}
    <li><span class="tag tag-mint">only yours</span><span><b class="num">{app.stats?.saved ?? 0}</b> saved {app.stats?.saved === 1 ? 'post' : 'posts'} · reading notes on <b class="num">{app.stats?.readingNotes ?? 0}</b> {app.stats?.readingNotes === 1 ? 'person' : 'people'} · trust, mutes & petnames</span></li>
    <li><span class="tag tag-line">this {app.platformName}</span><span>settings, device names{(app.stats?.blockedPictures ?? 0) ? ` · ${app.stats?.blockedPictures} blocked picture fingerprints` : ''}{net?.homeNode ? ' · home node pairing' : ''}</span></li>
  </ul>
{/snippet}

{#snippet pairForm()}
  <form class="pairf" onsubmit={pair}>
    <label class="sr-only" for="y-pair">Pairing link</label>
    <input id="y-pair" class="input mono-in" bind:value={pairLink} placeholder="hoi://pair?… or https://…#/pair?…" autocomplete="off" />
    <button class="pill pill-ink pill-sm" type="submit" disabled={pairing || !pairLink.trim()}>{pairing ? 'Pairing…' : 'Pair'}</button>
  </form>
{/snippet}

<section class="view you" use:enterView aria-labelledby="y-h">
  <div class="vh anim idhead jumpto" id="y-sec-key">
    <Avatar name={me.name} keyHex={me.rootKey} avatar={me.avatar} avatarColor={me.avatarColor} accountBadge size="xl" />
    <div class="idtext">
      {#if editingName}
        <form class="namef" onsubmit={saveName}>
          <label class="sr-only" for="y-name">Your name</label>
          <input id="y-name" class="input" bind:value={nameDraft} maxlength="32" />
          <button class="pill pill-ink pill-sm" type="submit">Save</button>
          <button class="pill pill-sm" type="button" onclick={() => (editingName = false)}>Cancel</button>
        </form>
      {:else}
        <h1 id="y-h">Hey {me.name}. <button type="button" class="act rename" onclick={() => { nameDraft = me.name; editingName = true; }}>Change name</button></h1>
      {/if}
      <span class="mono did">{me.root}</span>
      {#if domainName}
        <details class="handlehelp">
          <summary>Show {domainName} as verified</summary>
          <p class="help">Your name is a domain. Devices that see it check whether the domain names your key, and show ✓ {domainName} when it does. Put either of these on the domain:</p>
          <p class="help">A DNS TXT record for <span class="mono">_hoi.{domainName}</span>: <span class="mono">did={me.root}</span></p>
          <p class="help">Or a file at <span class="mono">https://{domainName}/.well-known/hoi</span> holding <span class="mono">{me.root}</span>, served with <span class="mono">Access-Control-Allow-Origin: *</span>: browsers can only read it with that header, and check nothing else.</p>
        </details>
      {/if}
    </div>
    <span class="sticker tilt" use:popIn={2}>your key · your identity</span>
  </div>
  <nav class="jump" aria-label="Sections of this page">
    {#each sections as sec (sec.id)}<button type="button" class="jumpbtn" onclick={() => jump(sec.id)}>{sec.label}</button>{/each}
  </nav>

  {#if me.replaced}
    <p class="callout anim">Another device replaced the key this {app.platformName} signs with, from your recovery line. Nothing signed here counts anymore. Wipe this {app.platformName}; link it again from the device you use now.</p>
  {:else if me.restoring}
    <p class="callout anim">You're back. Join one of your communities with an invite, so this {app.platformName} finds your key's history and your standing.</p>
  {/if}
  {#if me.burned}
    <p class="callout anim">You burned this key. Nothing it signed counts anywhere anymore. Wipe this {app.platformName} and make a fresh key: you start from nothing, in every community.</p>
  {/if}
  <div class="profile-section anim"><ProfilePicture /></div>
  <div class="sets anim">
    <div class="set">
      <span class="l">Fingerprint</span>
      <span class="h">Compare it in person to be sure it's the right key. Read it out like a phone number.</span>
      <span class="v fpv">
        <button type="button" class="fp mono" onclick={copyFingerprint} title="Click to copy">{me.fingerprint}</button>
        <span class="copyhint" aria-hidden="true">click to copy</span>
      </span>
    </div>
    <KeyWork />
    <Devices onlink={() => (linkOpen = true)} />
    <div class="set">
      <span class="l">If you lose everything</span>
      <span class="h">Any 3 of the friends you choose bring you back, with your standing. Each keeps one share of your recovery line, never the line.</span>
      <span class="v"><button type="button" class="lnk lnk-ink" onclick={() => (friendsOpen = true)}>Friends</button></span>
    </div>
    <PasskeyUnlock />
    <OsUnlock />
    <div class="set">
      <span class="l">Identity backup</span>
      <span class="h">{me.hasRoot ? 'A recovery key for your password manager, or an encrypted .hoikey file. Either brings your key back on a new device.' : 'This device holds a device key, not your root key. Back up from the device that has it, or save this device key as a file.'}</span>
      <span class="v">
        <button type="button" class="lnk lnk-ink" onclick={() => (backupOpen = true)}>Back up my identity</button>
        <button type="button" class="lnk lnk-ink" onclick={() => (importOpen = true)}>Bring a key</button>
      </span>
    </div>
  </div>

  <div class="vh anim sec jumpto" id="y-sec-conn">
    <h2 tabindex="-1">Connection</h2>
    <span class={['tag', net && net.peers > 0 ? 'tag-mint' : 'tag-soft']}>● {net?.label ?? '…'}</span>
  </div>
  <div class="anim"><NetworkMap /></div>
  <div class={['syncline', 'anim', sync.state]} role="status">
    <b>{sync.title}.</b> {sync.detail}
    {#if syncNotice.hidden}<button type="button" class="textlink" onclick={() => { syncNotice.show(); toast('The sync notice shows above your feeds again.'); }}>Show the notice above feeds again</button>{/if}
  </div>

  {#if web && !net?.homeNode}
    <div class="frame getapp anim">
      <div class="gtxt">
        <h3>Get the home node</h3>
        <p>
          A browser reaches others only while this tab is open, and what you post here waits until a home node has it. The hoi app for
          your computer is a home node: it keeps your communities and posts available while you're away and lets your phone in with one
          scan. No account, two minutes.
        </p>
      </div>
      <div class="btnrow">
        <a class="pill pill-ink" href="https://hoi.sh/download/" target="_blank" rel="noopener">Get the hoi app</a>
        <span class="help">Already running it? Click <b>Open hoi in my browser</b> there.</span>
      </div>
    </div>
  {/if}

  <div class="stats anim">
    <div class="frame stat"><b class="num">{net?.peers ?? 0}</b><span>connected right now</span></div>
    <div class="frame stat"><b class="num">{(app.stats?.records ?? 0).toLocaleString('en')}</b><span>records kept here</span></div>
    <div class="frame stat"><b class="num">{bytes(app.stats?.storedBytes ?? 0)}</b><span>stored on this {app.platformName}</span></div>
    <div class="frame stat"><b class="num">{app.stats?.bouncedKeys ?? 0}</b><span>keys bounced for spam</span></div>
    <div class="frame stat sunny"><b>0</b><span>bytes logged. ever.</span></div>
  </div>

  <div class="sets anim">
    {#if net?.serving}
      <div class="set top flow">
        <span class="l">Your browser and phone</span>
        <span class="h">
          Scan the code with your phone's camera: it opens hoi and connects to this {app.platformName} over your network; one more tap brings
          your key. <b>{net.serving.clients}</b> connected now.
          {#if !net.rtc.length}This {app.platformName} has no network address right now, so only a browser on it can connect.{/if}
        </span>
        <span class="v serving">
          {#if net.rtc.length}<QR value={net.serving.pairingWeb} label="QR code: open hoi on your phone and connect it here" color="mint" />{/if}
          <button type="button" class="lnk lnk-ink" onclick={() => openInBrowser(net.serving!.pairingWeb)}>Open hoi in this computer's browser</button>
          <details class="more">
            <summary>Pairing link</summary>
            <CopyField label="Pairing link" value={net.serving.pairingWeb} what="Pairing link copied" />
            <span class="help">Anyone with it can pair with this {app.platformName}. <button type="button" class="textlink" onclick={rotate}>Make a new one</button> if it got out.</span>
          </details>
        </span>
      </div>
    {/if}
    {#if net?.nodeLink}
      <div class="set top flow">
        <span class="l">Share this node</span>
        <span class="h">
          Anyone who opens this link finds the open communities this {app.platformName} carries, and the next public nodes along. It's an
          address, not a key: it lets nobody in as you.
        </span>
        <span class="v serving">
          <QR value={net.nodeLink} label="QR code: this node's link" color="sun" />
          <details class="more">
            <summary>Node link</summary>
            <CopyField label="Node link" value={net.nodeLink} what="Node link copied" />
          </details>
        </span>
      </div>
    {/if}
    <div class="set top flow">
      <span class="l">{web ? 'Your address' : 'Your addresses'}</span>
      <span class="h">
        {#if web}
          Connected nodes and browsers can see the IP address your connection comes from. This browser does not advertise a listening
          address; direct browser links also exchange connection addresses.
        {:else}
          Every address this {app.platformName} gives out, and who hears it. Point at one to see it, click to copy. Devices that connect also
          see the address your connection comes from{seen.own + seen.others ? ` (now: ${plural(seen.others, 'other device')}, ${seen.own} of yours)` : ''}.
        {/if}
      </span>
      {#if web}
        <div class="v addrv"><BrowserAddresses connected={seen.own + seen.others} unreported={unreportedAddress} /></div>
      {:else if net?.exposure?.length}
        <div class="v addrv"><Addresses /></div>
      {/if}
    </div>
    {#if web || net?.homeNode}
      <div class="set top flow">
        <span class="l">Home node</span>
        <span class="h">
          {#if web}Your always-on computer or hoi-node. This browser keeps and checks posts itself and syncs through it, even while you're away from it.{:else}This {app.platformName} syncs through the hoi-node it's paired with, so your communities stay available while it's off.{/if}
        </span>
        <span class="v homenode">
          {#if net?.homeNode}
            <span class="hn">
              <span class={['tag', net.homeNode.connected ? 'tag-mint' : 'tag-peach']}>{net.homeNode.connected ? '● connected' : '● unreachable'}</span>
              <span>{net.homeNode.name ?? 'your home node'}{net.homeNode.via === 'webrtc' ? ' · WebRTC' : net.homeNode.via === 'websocket' ? ' · WebSocket' : ''}</span>
            </span>
            {#if net.homeNode.error && !net.homeNode.connected}<span class="help">{net.homeNode.error}</span>{/if}
            <button type="button" class="lnk lnk-ink" onclick={unpair}>Unpair</button>
          {:else}
            {@render pairForm()}
            <span class="help">Easiest: in the hoi app on your computer, click <b>Open hoi in my browser</b>, or scan its code with your phone.</span>
          {/if}
        </span>
      </div>
    {:else}
      <div class="set top flow">
        <span class="l">Always-on server</span>
        <span class="h">Optional. This {app.platformName} is already a home node. Only if you also run hoi-node on a server that never sleeps, pair with it so your communities stay up while this {app.platformName} is off.</span>
        <details class="v more">
          <summary>Pair with a hoi-node</summary>
          {@render pairForm()}
        </details>
      </div>
    {/if}
    {#if s && (web || app.boot?.platform === 'mobile')}
      <div class="set">
        <span class="l">Use less data</span>
        <span class="h">Wait for your home to rebuild posts instead of fetching pieces here. Your home can also store eligible public posts in pieces before forwarding them; it knows you sent them. If placement fails, posts may go whole. Without a home, this {app.platformName} fetches one spare piece fewer.</span>
        <span class="v"><Switch checked={!!s.lessData} label="Use less data" onchange={(v) => set({ lessData: v }, v ? 'Using less data. Your home node rebuilds posts for you.' : 'Fetching posts as usual.')} /></span>
      </div>
    {/if}
    {#if s && web}
      <div class="set">
        <span class="l">Direct links with members' browsers</span>
        <span class="h">A public node you're linked to introduces this {app.platformName} to other members' browsers in your communities that turned this on too. Posts and pictures then go straight between you, four links at most. The browsers you link with see this {app.platformName}'s internet address.</span>
        <span class="v"><Switch checked={!!s.directLinks} label="Direct links with members' browsers" onchange={(v) => set({ directLinks: v }, v ? "Direct links on. Members' browsers can link with this one." : 'Direct links off.')} /></span>
      </div>
    {/if}
    <div class="set">
      <span class="l">A node on a server</span>
      <span class="h">Run a public node, with a registry if you like, on a rented server or any computer that stays on. This {app.platformName} makes the files and the commands; nothing is sent anywhere.</span>
      <span class="v"><a class="lnk lnk-ink" href={to.server()}>Set one up</a></span>
    </div>
    {#if s}
      <div class="set">
        <span class="l">Keep communities reachable</span>
        <span class="h">
          {#if web}A browser can't take connections, so it can't relay. The hoi app on a computer can, with one switch.{:else}Members' browsers and computers may connect here and catch up, even while whoever invited them is away. What passes is encrypted and signed; nothing is logged. Off: only your own devices.{/if}
        </span>
        <span class="v"><Switch checked={!web && s.relay} label="Keep communities reachable" disabled={web} onchange={relaySwitch} /></span>
      </div>
      {#if app.boot?.platform === 'desktop'}
        <div class="set top flow">
          <span class="l">Be a public node</span>
          <span class="h">
            Help the network: carry open communities for whoever asks, tell visitors about the other public nodes this {app.platformName} reached,
            and let people outside your home reach it. Off, this {app.platformName} publishes no address of its own and reaches public nodes the
            way a browser does.
          </span>
          <span class="v public">
            <Switch checked={!!s.publicNode} label="Be a public node" onchange={publicSwitch} />
            {#if s.publicNode}<span class={['help', 'router', routed.state]} role="status">{routed.text}</span>{/if}
          </span>
        </div>
        {#if s.publicNode}
          <div class="set top flow">
            <span class="l">Keep mailboxes for others</span>
            <span class="h">
              Private communities keep their messages in mailboxes on a few public nodes in different places, and their devices pick them on
              their own. With this on, this {app.platformName} can be one: it keeps only sealed messages it can't read, up to the space you
              give it, and its notice says so. Off, what it kept is deleted.
            </span>
            <span class="v public">
              <Switch checked={(s.holdPiecesGb ?? 0) > 0} label="Keep mailboxes for others" onchange={holdSwitch} />
              {#if (s.holdPiecesGb ?? 0) > 0}
                <span class="vpnpick">
                  <label class="sr-only" for="y-hold">Space for mailboxes</label>
                  <select id="y-hold" class="input" value={String(s.holdPiecesGb)} onchange={(e) => holdSize(Number(e.currentTarget.value))}>
                    {#each HOLD_SIZES as gb (gb)}<option value={String(gb)}>Up to {gb} GB</option>{/each}
                  </select>
                </span>
                <span class="help" role="status">{heldText}</span>
              {/if}
            </span>
          </div>
        {/if}
        <div class="set">
          <span class="l">Share the holders I see</span>
          <span class="h">
            In communities where you're established, list the holders this {app.platformName} has seen online for days, so other members'
            devices can place pieces on them. The nodes this {app.platformName} links to can then tell its posts are yours.
          </span>
          <span class="v"><Switch checked={!!s.shareHolders} label="Share the holders I see" onchange={(v) => set({ shareHolders: v }, v ? 'Sharing the holders this device sees, where you are established.' : 'No longer sharing the holders this device sees.')} /></span>
        </div>
        <div class="set top flow">
          <span class="l">Only through my VPN</span>
          <span class="h">
            For added security: every connection to the internet goes through your VPN, so the devices you connect to see your VPN's address,
            not your home's. While the VPN is down, nothing goes to the internet. To be a public node through it, your VPN has to forward a
            port (some do, over NAT-PMP).
          </span>
          <span class="v public">
            <Switch checked={!!s.vpn || vpnPick} label="Only through my VPN" onchange={vpnSwitch} />
            {#if vpnPick}
              <span class="vpnpick">
                <label class="sr-only" for="y-vpn">Your VPN's network interface</label>
                <select id="y-vpn" class="input" bind:value={vpnChoice}>
                  <option value="" disabled>Choose your VPN's network interface</option>
                  {#each vpnList as i (i.name)}
                    <option value={i.name}>{i.name}{i.label ? ` (${i.label})` : ''}{i.tunnel ? ' · tunnel' : ''} · {i.addresses.join(', ') || 'no address'}</option>
                  {/each}
                </select>
                <button type="button" class="pill pill-ink pill-sm" disabled={!vpnChoice} onclick={pickVpn}>Use it</button>
                <button type="button" class="pill pill-sm" onclick={() => (vpnPick = false)}>Cancel</button>
              </span>
            {/if}
            {#if s.vpn}
              <span class={['help', 'router', net?.vpn?.up ? 'reachable' : 'blocked']} role="status">{vpnText}</span>
              <label class="check">
                <input type="checkbox" checked={s.homeNetwork !== false} onchange={(e) => set({ homeNetwork: e.currentTarget.checked })} />
                Still use my home network directly, so my phone on the Wi-Fi can connect
              </label>
            {/if}
          </span>
        </div>
      {/if}
      {#if !web && s.relay}
        <div class="set">
          <label class="l" for="y-cap">Daily cap</label>
          <span class="h">Relaying naps for the rest of the day (UTC) once members used this much.</span>
          <span class="v slider">
            <input id="y-cap" type="range" min="1" max="50" value={s.dailyCapGb} onchange={(e) => set({ dailyCapGb: Number(e.currentTarget.value) })} />
            <span class="num">{s.dailyCapGb} GB</span>
            {#if s.dailyCapGb !== DEFAULT_CAP_GB}<button type="button" class="lnk lnk-ink" onclick={() => set({ dailyCapGb: DEFAULT_CAP_GB }, `Daily cap back to ${DEFAULT_CAP_GB} GB.`)}>Reset to {DEFAULT_CAP_GB} GB</button>{/if}
          </span>
        </div>
      {/if}
      <div class="set">
        <span class="l">Keep what I moderate</span>
        <span class="h">Hold the full live window of communities you mod, so newcomers can catch up from you.</span>
        <span class="v"><Switch checked={s.keepModerated} label="Keep what I moderate" onchange={(v) => set({ keepModerated: v }, v ? 'Keeping the full window of what you moderate.' : 'Keeping only what you read.')} /></span>
      </div>
      {#if app.boot?.platform === 'desktop'}
        <div class="set">
          <span class="l">Open as companion</span>
          <span class="h">On from the start: a small window with status, traffic and pairing, without the forum. Read and post in your browser, or open the full app from the companion or the tray.</span>
          <span class="v">
            <Switch checked={s.companion} label="Open as companion" onchange={(v) => set({ companion: v }, v ? 'hoi opens as the companion next time.' : 'hoi opens as the full app.')} />
            <a class="lnk lnk-ink" href="#/companion">Show it</a>
          </span>
        </div>
      {/if}
    {/if}
  </div>

  <section class="traffic-section warmth-section anim jumpto" id="y-sec-warmth" aria-labelledby="y-warmth">
    <div class="traffic-heading">
      <h2 id="y-warmth" tabindex="-1">Warmth</h2>
      <p class="help">Who else holds your communities' posts, as this {app.platformName} sees it (like a torrent's availability): <b>cold</b> only here, <b>cool</b> one other place, <b>warm</b> two, <b>hot</b> three complete, recent copies in places of their own. Nodes in one network or under one operator count as one place, and copies over unchecked links count for less. A hint, not a delivery receipt.</p>
    </div>
    {#if links}
      <div class="directory" role="status">
        <span class={['tag', links.tag]}>● your connections · {links.word}</span>
        <p class="help">{links.summary}{#if links.advice}{' '}{links.advice}{/if}</p>
      </div>
    {/if}
    <div class="directory" role="status">
      <span class={['tag', directory.tag]}>● finding communities · {directory.word}</span>
      <p class="help">{directory.summary}</p>
    </div>
    {#if warmth?.communities.length}
      <ul class="warmths" aria-label="Your communities, coldest first">
        {#each warmth.communities as w (w.community)}
          <li>
            <a class="wname" href={`#/c/${w.community}`}>c/{w.name || w.community.slice(0, 8)}</a>
            <Warmth {w} />
          </li>
        {/each}
      </ul>
    {:else}
      <p class="help">{warmth ? 'No public community on this ' + app.platformName + ' yet.' : 'Reading…'}</p>
    {/if}
  </section>
  <section class="traffic-section anim jumpto" id="y-sec-traffic" aria-labelledby="y-traffic">
    <div class="traffic-heading">
      <h2 id="y-traffic" tabindex="-1">What passes through this {app.platformName}</h2>
      <p class="help">Activity on this device. Totals cover this run; the chart shows the last hour.</p>
    </div>
    <Traffic />
  </section>
  <div class="jumpto" id="y-sec-trouble"><ConnectionDetails /></div>

  {#if s}
    <div class="vh anim sec jumpto" id="y-sec-privacy"><h2 tabindex="-1">Privacy</h2></div>
    <div class="sets anim">
      <div class="set">
        <span class="l">Privacy notice</span>
        <span class="h">Who made this software, what every connection shares, what stays on this {app.platformName}. Network &amp; Safety was acknowledged here on {networkConsent.record?.date ?? 'this visit'}.</span>
        <span class="v"><a class="lnk lnk-ink" href={to.privacy()}>Read</a></span>
      </div>
      <div class="set top flow">
        <span class="l">Node operators</span>
        <span class="h">Public nodes this {app.platformName} met and what you answered to their operators' notices. A forgotten node asks again the next time it is reached; a declined one is not contacted until then.</span>
        <div class="v notices">
          {#if !notices.length}
            <span class="tag tag-soft">no notices yet</span>
          {:else}
            {#each notices as n (n.node)}
              <div class="noticerow">
                <span class="nn"><b>{n.name || 'a public node'}</b> · {[n.operator.trim() || 'operator not named', n.country.trim()].filter(Boolean).join(', ')} · <span class="tag tag-soft">{n.state}</span></span>
                <span class="na">
                  {#if n.state === 'declined'}<button type="button" class="lnk lnk-ink" onclick={() => answerNotice(n.node, n.hash, true)}>Accept</button>{/if}
                  {#if n.state === 'accepted'}<button type="button" class="lnk" onclick={() => answerNotice(n.node, n.hash, false)}>Decline</button>{/if}
                  <button type="button" class="lnk lnk-danger" onclick={() => forgetNotice(n.node)}>Forget</button>
                </span>
              </div>
            {/each}
          {/if}
        </div>
      </div>
      <div class="set">
        <span class="l">Fetch link previews</span>
        <span class="h">Your computer fetches the linked page's title, description and picture: this desktop app, or for a browser its home node. The site sees that computer's address, never your cookies; the preview stays on this {app.platformName} and is never posted. Off: links show as plain addresses.</span>
        <span class="v"><Switch checked={s.linkPreviews} label="Fetch link previews" onchange={(v) => set({ linkPreviews: v }, v ? 'Link previews on. Linked sites see your computer fetch them.' : 'Link previews off.')} /></span>
      </div>
      <div class="set">
        <span class="l">Vouch as I read</span>
        <span class="h">Where you're established, reading someone's posts without hiding them vouches for them after a few minutes: newcomers get in, members vouched in with many others get a fuller voice. Only the vouch leaves this {app.platformName}; what you read stays here and is forgotten within 30 days.</span>
        <span class="v"><Switch checked={s.autoVouch} label="Vouch as I read" onchange={(v) => set({ autoVouch: v }, v ? 'Reading counts as vouching again.' : 'You vouch only when you press Vouch.')} /></span>
      </div>
      <div class="set">
        <span class="l">Fetch media</span>
        <span class="h">Images and video from trusted members arrive by themselves, checked and stripped of metadata. Off: attachments are not fetched and others' profile pictures stay hidden. Small profile updates can still arrive with community records.</span>
        <span class="v"><Switch checked={s.autoMedia} label="Fetch media" onchange={(v) => set({ autoMedia: v }, v ? 'Media from trusted members comes along now.' : 'Attachments are not fetched and others’ profile pictures are hidden.')} /></span>
      </div>
      <div class="set top flow">
        <span class="l">Labelers</span>
        <span class="h">People whose labels count on this {app.platformName}: spam, nsfw and off-topic put posts to sleep, other labels show as notes. Communities can name default labelers; you can follow more, anywhere. Labels from anyone else are not taken.</span>
        <Labelers />
      </div>
      <div class="set top flow blocklist">
        <span class="l">Blocked pictures</span>
        <span class="h">Pictures marked as illegal when hiding or removing a post are blocked here automatically, including near copies. There is no need to upload a picture.</span>
        <div class="v">
          <span class="tag tag-soft">{app.stats?.blockedPictures ?? 0} saved fingerprints</span>
          <details class="block-import">
            <summary>Import a trusted fingerprint list</summary>
            <p class="help" id="y-block-help">Optional: choose a text list from a group you trust. It contains picture fingerprints (PDQ hashes), not pictures. Matching images are refused on this {app.platformName}; the list is never uploaded. A mistaken list can hide harmless images too.</p>
            <label class="field" for="y-block"><span>Fingerprint list (.txt or .csv)</span></label>
            <input id="y-block" type="file" accept=".txt,.csv,text/plain,text/csv" aria-describedby="y-block-help" onchange={loadBlocklist} />
            <p class="help">One 64-character PDQ fingerprint per line. Up to 5 MB.</p>
          </details>
          <p class="help">Or follow a list keeper from their page: their signed list counts on this {app.platformName} for as long as you follow it.</p>
          <ListKeepers />
        </div>
      </div>
    </div>

    <div class="vh anim sec jumpto" id="y-sec-looks"><h2 tabindex="-1">Looks</h2></div>
    <div class="sets anim">
      <div class="set">
        <span class="l" id="y-theme">Theme</span>
        <span class="h">Follow this {app.platformName}, or pick one.</span>
        <span class="v">
          <span class="seg" role="radiogroup" aria-labelledby="y-theme">
            {#each ['system', 'light', 'dark'] as t (t)}
              <label><input type="radio" name="y-theme" value={t} checked={s.theme === t} onchange={() => set({ theme: t as Settings['theme'] })} /><span>{t}</span></label>
            {/each}
          </span>
        </span>
      </div>
      <div class="set">
        <span class="l" id="y-motion">Motion</span>
        <span class="h">Reduced skips the bobbing, the stamping and the drops. Everything still works.</span>
        <span class="v">
          <span class="seg" role="radiogroup" aria-labelledby="y-motion">
            {#each ['system', 'reduced'] as m (m)}
              <label><input type="radio" name="y-motion" value={m} checked={s.motion === m} onchange={() => set({ motion: m as Settings['motion'] })} /><span>{m}</span></label>
            {/each}
          </span>
        </span>
      </div>
    </div>
  {/if}

  <div class="vh anim sec jumpto" id="y-sec-device"><h2 tabindex="-1">On this {app.platformName}</h2></div>
  <Updates />
  <div class="sets anim">
    <div class="privacy">
      <p class="lead">
        <b>What stays here, what you share.</b>
        {web ? 'hoi runs in this browser; the site only serves its files and never receives your records.' : `hoi runs on this ${app.platformName}.`}
        Your key and settings stay here. Posts go to their community's members and fade on its schedule; members can keep copies.
      </p>
      <dl>
        <div>
          <dt><span class="tag tag-mint">Only here, never sent</span></dt>
          <dd>
            <span class="item">trust, mutes & petnames</span>
            <span class="item"><b class="num">{app.stats?.saved ?? 0}</b> saved {app.stats?.saved === 1 ? 'post' : 'posts'}</span>
            <span class="item">reading notes on <b class="num">{app.stats?.readingNotes ?? 0}</b> {app.stats?.readingNotes === 1 ? 'person' : 'people'} <i>for vouching, gone in 30 days</i></span>
            <span class="item">settings</span>
          </dd>
        </div>
        <div>
          <dt><span class="tag tag-sky">Public in its community, then gone</span></dt>
          <dd>
            <span class="item">your posts, replies & votes</span>
            <span class="item">fade with each community's window</span>
            {#if app.stats?.fadesBy}<span class="item">last one here fades <b>{fadesIn(app.stats.fadesBy).replace(/^fades /, '')}</b></span>{/if}
          </dd>
        </div>
        <div>
          <dt><span class="tag tag-sun">Public and lasting</span></dt>
          <dd>
            <span class="item">your vouches</span>
            <span class="item">your answers to rule changes</span>
            <i class="why">signed, kept as long as what they decide</i>
          </dd>
        </div>
        <div>
          <dt><span class="tag tag-lilac">Media</span></dt>
          <dd>
            <span class="item"><b class="num">{bytes(app.stats?.mediaBytes ?? 0)}</b> kept here</span>
            <span class="item">location & camera details stripped before sending</span>
          </dd>
        </div>
      </dl>
    </div>
    <div class="set top">
      <span class="l">Everything here</span>
      <span class="h">Wipe removes all of this from this {app.platformName}. Your other devices keep theirs, and it all fades on each community's schedule anyway.</span>
      <span class="v"><button type="button" class="lnk lnk-danger" onclick={() => (wipeStep = 1)}>Wipe…</button></span>
      <div class="wide">{@render wipeList()}</div>
    </div>
    {#if me.hasRoot && !me.burned}
      <div class="set">
        <span class="l">Burn this key</span>
        <span class="h">For a key that leaked. Every device that hears of it drops everything it ever signed and refuses anything it signs later. Your place in every community goes with it.</span>
        <span class="v"><button type="button" class="lnk lnk-danger" onclick={() => (burnOpen = true)}>Burn</button></span>
      </div>
    {/if}
  </div>
  <p class="build mono">hoi {app.boot?.version} · protocol {app.boot?.protocol} · {app.boot?.platform}</p>
</section>

<Modal bind:open={linkOpen} title="Link a device">
  <LinkDevice ondone={() => (linkOpen = false)} />
</Modal>

<Modal bind:open={friendsOpen} title="Friends who bring you back">
  <RecoveryFriends ondone={() => (friendsOpen = false)} />
</Modal>

<Modal bind:open={backupOpen} title="Back up my identity">
  <KeyBackup ondone={() => (backupOpen = false)} />
</Modal>

<Modal bind:open={importOpen} title="Bring a key">
  <p>A {app.platformName} holds one key, and this one already signs as <b>{me.name}</b> (<span class="mono">{me.short}</span>).</p>
  <p class="help">To restore another identity, use a fresh browser profile or another device and choose <b>Restore my identity</b> after protecting its vault. To replace the identity here instead, back this one up first, then use the explicit wipe flow. A failed restore never replaces your current identity.</p>
  <div class="btnrow">
    <button type="button" class="pill pill-ink" onclick={() => { importOpen = false; backupOpen = true; }}>Back up this key first</button>
    <button type="button" class="pill pill-danger" onclick={() => { importOpen = false; wipeStep = 1; }}>Wipe this {app.platformName}</button>
  </div>
</Modal>

<Modal open={relayNotice} title="Before you relay" onclose={() => (relayNotice = false)}>
  <p>Members' browsers and computers will connect to this {app.platformName} and catch up on your communities through it. What passes is encrypted on the way and signed by its author, so nobody can change it, and hoi keeps no log of it. They can see your IP address. You can pause it or cap it any time.</p>
  <div class="btnrow">
    <button type="button" class="pill pill-ink" onclick={() => { relayNotice = false; void set({ relay: true, relayNoticeSeen: true }, 'Relaying. Members can catch up through this computer.'); }}>Turn it on</button>
    <button type="button" class="pill" onclick={() => (relayNotice = false)}>Not now</button>
  </div>
</Modal>

<Modal open={publicAsk} title="Before you become a public node" onclose={publicCancel}>
  <p>
    This {app.platformName} will carry open communities anyone asks it to keep (up to 256 of them, 256 MB together), serve their posts to
    whoever connects, and tell visitors about the other public nodes it reached. Your router is asked to forward its ports so people
    outside your home can reach it. You can turn it off any time; the forwards go with it.
  </p>
  <p class={['router', routed.state]} role="status"><b>{routed.text}</b></p>
  {#if routed.state === 'reachable'}
    <p>
      That address goes into your invites and your node link. Anyone who has them, anyone they pass them to, and every device that connects
      can see it. From it people can tell your rough location and internet provider, flood your connection, and link this node to you; your
      provider can name you to a court. Your {app.platformName} also holds other people's public posts and media, readable.
    </p>
  {/if}
  <div class="btnrow">
    <button type="button" class="pill pill-ink" disabled={routed.state !== 'reachable'} onclick={publicConfirm}>Make this {app.platformName} public</button>
    <button type="button" class="pill" onclick={publicCancel}>Not now</button>
  </div>
</Modal>

<Modal bind:open={burnOpen} title="Burn your key?">
  <p><b>This can't be undone.</b> Your posts, votes and standing everywhere go with it, and the key can never sign again. Type <span class="mono">{me.short}</span> to confirm.</p>
  <label class="field"><span class="sr-only">Your key's short form</span><input class="input mono" bind:value={burnConfirm} placeholder={me.short} autocomplete="off" /></label>
  <div class="btnrow">
    <button type="button" class="pill pill-danger" onclick={burn} disabled={busy || burnConfirm.trim() !== me.short}>{busy ? 'Burning…' : 'Burn it'}</button>
    <button type="button" class="pill" onclick={() => (burnOpen = false)}>Keep it</button>
  </div>
</Modal>

<Modal open={wipeStep > 0} title={wipeStep === 1 ? `Wipe this ${app.platformName}?` : 'Last chance.'} onclose={() => (wipeStep = 0)}>
  {#if wipeStep === 1}
    <p>This deletes everything below from this {app.platformName}. Communities live on with their other members.</p>
    {@render wipeList()}
    <div class="btnrow">
      <button type="button" class="pill pill-danger" onclick={() => (wipeStep = 2)}>Wipe it</button>
      <button type="button" class="pill" onclick={() => (wipeStep = 0)}>Keep everything</button>
    </div>
  {:else}
    <p><b>There's no undo.</b> Unless you exported your key or linked another device, nobody can bring it back. Not even us: we never had it.</p>
    <div class="btnrow">
      <button type="button" class="pill pill-danger" onclick={wipe} disabled={busy}>{busy ? 'Wiping…' : 'Yes, wipe everything'}</button>
      <button type="button" class="pill" onclick={() => (wipeStep = 0)}>Stop</button>
    </div>
  {/if}
</Modal>

<style>
  .idhead { gap: 18px; align-items: center; }
  .profile-section { padding: 16px 0 24px; max-width: 720px; }
  .idtext { display: flex; flex-direction: column; gap: 6px; min-width: 0; flex: 1 1 300px; }
  .idtext h1 { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .rename { font-size: 13px; letter-spacing: 0; }
  .handlehelp { margin-top: 6px; }
  .handlehelp summary { cursor: pointer; font-weight: 600; font-size: 14px; }
  .handlehelp .mono { overflow-wrap: anywhere; }
  .did { overflow-wrap: anywhere; }
  .namef { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; }
  .namef .input { flex: 1 1 200px; max-width: 320px; }
  .tilt { margin-left: auto; transform: rotate(2deg); white-space: normal; }
  .fpv { flex-direction: column; align-items: flex-end; gap: 2px; }
  .fp { font-size: 15px; color: var(--ink); padding: 4px 8px; margin-right: -8px; border-radius: 8px; cursor: copy; }
  .fp:hover { background: var(--soft); }
  .copyhint { font-size: 11px; font-weight: 600; color: var(--mute); opacity: 0; transition: opacity .15s ease; }
  .fpv:hover .copyhint, .fpv:focus-within .copyhint { opacity: 1; }
  .addrv { grid-column: 1 / -1; max-width: none !important; }
  .sec { padding-top: 36px; padding-bottom: 12px; }
  /* The section row sticks under the top bar; sections stop below both when jumped to. */
  .jump {
    position: sticky; top: calc(66px + env(safe-area-inset-top, 0px)); z-index: 5; display: flex; gap: 6px; overflow-x: auto; scrollbar-width: none;
    margin: 4px 0 8px; padding: 8px 0; background: var(--bg); box-shadow: 0 2px 0 var(--line);
  }
  .jump::-webkit-scrollbar { display: none; }
  .jumpbtn { flex: 0 0 auto; min-height: 36px; padding: 0 14px; border-radius: 999px; border: 2px solid var(--ink); background: var(--card); font-weight: 700; font-size: 13px; white-space: nowrap; }
  .jumpbtn:hover { background: var(--soft); }
  .jumpto { scroll-margin-top: calc(130px + env(safe-area-inset-top, 0px)); }
  .jumpto :global(h2:focus), .jumpto :global(summary:focus) { outline: none; }
  .syncline { padding: 10px 14px; margin-bottom: 14px; border-radius: var(--radius-input); background: var(--soft); font-size: 14px; line-height: 1.5; max-width: 820px; overflow-wrap: anywhere; }
  .syncline.offline, .syncline.degraded { background: var(--peach); color: var(--tagink); }
  .syncline .textlink { display: block; margin-top: 4px; }
  .homenode { flex-direction: column; align-items: flex-end; gap: 8px; }
  details.more { gap: 10px; }
  .hn { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; justify-content: flex-end; }
  .pairf { display: flex; gap: 8px; width: 100%; }
  .pairf .input { flex: 1; min-width: 0; }
  .serving { flex-direction: column; align-items: flex-end; }
  .serving :global(.qr) { width: 180px; }
  .getapp { padding: 16px 18px; display: flex; flex-direction: column; gap: 12px; margin-bottom: 16px; background: var(--mint); color: var(--tagink); }
  .getapp h3 { font-size: 20px; letter-spacing: -0.02em; font-weight: 800; }
  .getapp p { line-height: 1.5; font-weight: 500; max-width: 720px; }
  .getapp .help { color: var(--tagink); }
  .public { display: grid; gap: 6px; justify-items: start; }
  .vpnpick { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; max-width: 100%; }
  .vpnpick select { max-width: 100%; }
  .check { display: flex; gap: 8px; align-items: center; font-size: 14px; }
  .router.blocked { box-shadow: inset 3px 0 0 var(--coral); padding-left: 10px; }
  .getapp .pill-ink { background: var(--tagink); color: #fff; border-color: var(--tagink); }
  .more summary, .block-import summary { cursor: pointer; font-weight: 700; color: var(--cobalt); min-height: 44px; }
  .more { width: 100%; max-width: 420px; display: flex; flex-direction: column; gap: 8px; }
  .more summary { text-align: left; }
  .traffic-section { margin-top: 32px; padding: 22px; border: 2px solid var(--ink); border-radius: var(--radius-card); background: var(--card); }
  .traffic-heading { margin-bottom: 20px; display: flex; flex-direction: column; gap: 8px; }
  .traffic-heading h2 { font-size: 24px; letter-spacing: -.03em; }
  .traffic-heading .help, .directory .help { max-width: 72ch; }
  .warmth-section .traffic-heading { margin-bottom: 14px; }
  .directory { display: flex; flex-direction: column; gap: 6px; padding-bottom: 14px; border-bottom: 2px dotted var(--line); }
  .directory .tag { align-self: flex-start; white-space: normal; }
  .warmths { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .warmths li { display: flex; flex-direction: column; gap: 8px; padding: 14px 0; border-top: 2px dotted var(--line); }
  .warmths li:first-child { border-top: 0; }
  .wname { font-weight: 800; font-size: 17px; letter-spacing: -0.02em; overflow-wrap: anywhere; }
  .blocklist .v { flex-direction: column; gap: 12px; }
  .block-import { width: 100%; }
  .block-import p { margin-bottom: 12px; }
  .block-import .field { margin-bottom: 6px; }
  .block-import input { margin-bottom: 8px; }
  .slider input { width: 130px; }
  .privacy { padding: 20px 0; border-top: 2px dotted var(--line); display: flex; flex-direction: column; gap: 20px; }
  .privacy .lead { font-weight: 500; line-height: 1.45; max-width: 720px; }
  .privacy .lead b { font-weight: 800; }
  .privacy dl { margin: 0; display: flex; flex-direction: column; }
  .privacy dl div { display: grid; grid-template-columns: 240px minmax(0, 1fr); gap: 8px 20px; padding: 12px 0; border-top: 1px solid var(--line); font-size: 14px; line-height: 1.5; align-items: baseline; }
  .privacy dt .tag { white-space: normal; }
  .privacy dd { margin: 0; color: var(--mute); font-weight: 500; display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
  .item { display: inline-flex; align-items: baseline; gap: 5px; padding: 2px 10px; border: 1.5px solid var(--line); border-radius: 999px; color: var(--ink); background: var(--card); }
  .item b, .wipes b { font-weight: 800; }
  .item i, .why { font-style: normal; color: var(--mute); font-size: 13px; }
  .wipes { list-style: none; margin: 0; padding: 12px 14px; display: flex; flex-direction: column; gap: 8px; background: var(--soft); border-radius: var(--radius-input); font-size: 14px; font-weight: 500; }
  .wipes li { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .wipes .tag { min-width: 96px; justify-content: center; }
  @media (max-width: 640px) {
    .jumpbtn { min-height: 40px; }
    .privacy dl div { grid-template-columns: 1fr; gap: 2px; }
    .traffic-section { padding: 16px; }
    .pairf { flex-wrap: wrap; }
    .pairf .input { flex-basis: 200px; }
  }
  .build { padding-top: 28px; }
  @media (max-width: 860px) {
    .homenode, .serving, .fpv { align-items: flex-start; }
    .more summary { justify-content: flex-start; }
    .hn { justify-content: flex-start; }
    .fp { margin-right: 0; margin-left: -8px; }
    .tilt { margin-left: 0; }
  }
  .notices { display: flex; flex-direction: column; gap: 8px; align-items: flex-start; }
  .noticerow { display: flex; flex-wrap: wrap; gap: 6px 14px; align-items: center; font-size: 14px; }
  .noticerow .na { display: flex; gap: 12px; }
</style>
