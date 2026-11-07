<script lang="ts">
  // Link a browser, desktop app or phone to your key. With a home node it is one scan: the
  // QR pairs the new device with the home node, the new device asks, this one shows four digits
  // to compare (LinkAskPrompt). Without one, or without a camera, the three codes still work:
  // the new device asks (linkRequest), a device with your root key blesses it (linkApprove),
  // the new device finishes (linkComplete). No server carries anything either way.
  import { app } from '../app.svelte';
  import { toast, toastError } from '../toast.svelte';
  import CopyField from './CopyField.svelte';
  import LinkWaiting from './LinkWaiting.svelte';
  import QR from './QR.svelte';

  let { newOnly = false, ondone }: { newOnly?: boolean; ondone?: () => void } = $props();

  // svelte-ignore state_referenced_locally
  let side = $state<'approve' | 'new'>(newOnly || !app.me?.hasRoot ? 'new' : 'approve');
  let label = $state(defaultLabel());
  let request = $state('');
  let approvalIn = $state('');
  let requestIn = $state('');
  let approval = $state('');
  let busy = $state(false);
  let codes = $state(false);
  let asking = $state(false);

  /** The link a phone's camera opens: it pairs with the same home node, then asks to be linked. */
  const pairingWeb = $derived(app.net?.serving?.pairingWeb || app.net?.homeNode?.pairingWeb || '');
  const homeName = $derived(app.net?.homeNode?.name ?? (app.net?.serving ? 'this computer' : 'your home node'));
  const paired = $derived(!!app.net?.homeNode);
  /** A computer serving only on loopback, with no network route: a phone can't reach it. */
  const reach = $derived(app.net?.serving?.pairingWeb ? app.net.serving.phoneCanReach : app.net?.homeNode?.phoneCanReach);
  const noLan = $derived(!!pairingWeb && reach === false);

  function defaultLabel() {
    const p = app.boot?.platform;
    if (p === 'desktop') return 'Desktop';
    if (p === 'mobile' || /Android|iPhone|iPad/i.test(navigator.userAgent)) return 'Phone';
    return 'Browser';
  }

  // The new device was linked through the home node (a `linked` event set app.me).
  $effect(() => {
    if (asking && app.me) {
      asking = false;
      ondone?.();
    }
  });

  /** Keep no signing key in this browser: the computer with your key makes it, keeps it and
   * signs for this browser. */
  let keyless = $state(false);

  async function askHome(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    try {
      app.linkChecks = [];
      await (keyless ? app.core.linkHomeRemote : app.core.linkHome)(label.trim() || defaultLabel());
      asking = true;
    } catch (err) {
      toastError(err);
    }
    busy = false;
  }

  async function cancelAsk() {
    asking = false;
    app.linkChecks = [];
    await app.core.linkCancel().catch(() => undefined);
  }

  async function makeRequest(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    try {
      request = await app.core.linkRequest(label.trim() || defaultLabel());
    } catch (err) {
      toastError(err);
    }
    busy = false;
  }

  async function complete(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    try {
      const me = await app.core.linkComplete(approvalIn.trim());
      await app.identityChanged(me);
      toast(`Linked. This ${app.platformName} signs as ${me.name} now.`);
      ondone?.();
    } catch (err) {
      toastError(err);
    }
    busy = false;
  }

  async function approve(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    try {
      approval = await app.core.linkApprove(requestIn.trim());
      await app.refreshMe();
      toast('Blessed by your root key. Now finish on the new device.');
    } catch (err) {
      toastError(err);
    }
    busy = false;
  }
</script>

{#if !newOnly}
  <div class="seg" role="radiogroup" aria-label="Which device is this?">
    <label><input type="radio" name="linkside" value="approve" bind:group={side} disabled={!app.me?.hasRoot} /><span>This one has my key</span></label>
    <label><input type="radio" name="linkside" value="new" bind:group={side} /><span>This is the new one</span></label>
  </div>
{/if}

{#if side === 'approve'}
  {#if pairingWeb}
    <div class="scan">
      <QR value={pairingWeb} label="QR code that opens hoi on your phone and asks to link it" color="lilac" />
      <ol class="steps">
        <li>Open the pairing link in your other browser, or scan the code with your phone.</li>
        <li>Protect that device with a local passphrase, then choose <b>Connect</b> and link it.</li>
        <li>The new device shows four digits. Compare them with the request here before approving.</li>
      </ol>
    </div>
    {#if noLan}
      <p class="callout">{homeName} only listens on this computer right now, so a phone can't reach it. Connect the computer to your Wi-Fi and open this again.</p>
    {:else}
      <p class="help">Your phone needs to be on the same network as {homeName}, or {homeName} needs an address it can reach. Each link pairs one device with {homeName}, then a new one shows here: show it only to your own devices.</p>
    {/if}
    <details class="more">
      <summary>Copy a link for another browser</summary>
      <CopyField label="Pairing link for your other browser or phone" value={pairingWeb} what="Pairing link copied" />
    </details>
  {:else}
    <p class="callout">
      Devices find each other through a home node: the hoi app on a computer, or hoi-node. With one running, linking is one scan.
      <a class="textlink" href="https://hoi.sh/download/" target="_blank" rel="noopener">Get the hoi app</a>
    </p>
  {/if}

  <details class="more" bind:open={codes}>
    <summary>{pairingWeb ? 'Link a desktop app or use codes instead' : 'Link browser or desktop with codes'}</summary>
    <ol class="steps">
      <li>In the new browser or desktop app, protect its local storage, then choose <b>Link a signed-in device</b> and <b>Use codes instead</b>. It shows a request code.</li>
      <li>
        <form class="stepform" onsubmit={approve}>
          <label class="field">
            <span>Paste the request code here</span>
            <textarea class="textarea mono-in" bind:value={requestIn} rows="2" placeholder="hoi:link:…" required></textarea>
          </label>
          <button class="pill pill-ink" type="submit" disabled={busy || !requestIn.trim()}>Bless it with my key</button>
        </form>
      </li>
      <li>
        Type or paste this on the new device.
        {#if approval}
          <div class="codes">
            <QR value={approval} label="QR code with the approval" color="lilac" />
            <CopyField label="Approval code" value={approval} what="Approval code copied" />
          </div>
        {/if}
      </li>
    </ol>
  </details>
{:else}
  <p class="help">You can link a browser, the desktop app or a phone. Each keeps its own local unlock passphrase. Keep the other device open while you finish here.</p>
  <label class="field namef">
    <span>Call this device</span>
    <input class="input" bind:value={label} maxlength="32" />
  </label>
  {#if paired}
    {#if asking}
      <LinkWaiting home={homeName} oncancel={cancelAsk} />
    {:else}
      <form class="stepform" onsubmit={askHome}>
        <p class="help">This {app.platformName} is paired with {homeName}. Your device with your key gets a question; you say yes there.</p>
        <label class="keyless"><input type="checkbox" bind:checked={keyless} /> Keep no key in this {app.platformName}: my computer signs for it. Posts need the computer on; moderation waits for a yes there.</label>
        <button class="pill pill-ink" type="submit" disabled={busy}>Ask my other device</button>
      </form>
    {/if}
  {:else}
    <p class="help">
      On the device that has your key, open <b>You → Your devices → Link a device</b>. Open its pairing link in this browser, scan the QR code on a phone, or use the codes below to sign in to the desktop app.
    </p>
  {/if}
  <details class="more" bind:open={codes}>
    <summary>Use codes instead</summary>
    <ol class="steps">
      <li>
        <form class="stepform" onsubmit={makeRequest}>
          <button class="pill pill-ink" type="submit" disabled={busy}>{request ? 'Make a new request' : 'Make a request code'}</button>
        </form>
        {#if request}
          <div class="codes">
            <QR value={request} label="QR code with the link request" color="lilac" />
            <CopyField label="Request code: show or send it to a device that has your key" value={request} what="Request code copied" />
          </div>
        {/if}
      </li>
      <li>
        <form class="stepform" onsubmit={complete}>
          <label class="field">
            <span>Paste the approval it gives you</span>
            <textarea class="textarea mono-in" bind:value={approvalIn} rows="2" placeholder="hoi:link:…"></textarea>
          </label>
          <button class="pill pill-ink" type="submit" disabled={busy || !approvalIn.trim() || !request}>Finish linking</button>
        </form>
      </li>
    </ol>
  </details>
{/if}

<style>
  .keyless { display: flex; gap: 8px; align-items: flex-start; font-size: 14px; line-height: 1.45; color: var(--mute); }
  .seg { margin-bottom: 16px; }
  .scan { display: grid; grid-template-columns: minmax(0, 220px) minmax(0, 1fr); gap: 18px; align-items: center; }
  .scan :global(.qr) { width: 100%; }
  .steps { margin: 0; padding-left: 22px; display: flex; flex-direction: column; gap: 12px; font-weight: 500; line-height: 1.5; }
  .steps li::marker { font-weight: 900; }
  .stepform { display: flex; flex-direction: column; gap: 10px; align-items: flex-start; margin-top: 4px; }
  .stepform .field { width: 100%; }
  .namef { margin-bottom: 12px; max-width: 320px; }
  .codes { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 14px; align-items: start; margin-top: 10px; }
  .codes :global(.qr) { width: 170px; }
  .callout, .help { margin-top: 12px; }
  .more { margin-top: 14px; border-top: 2px dotted var(--line); padding-top: 12px; }
  .more summary { cursor: pointer; font-weight: 700; color: var(--cobalt); min-height: 32px; display: flex; align-items: center; }
  .more[open] summary { margin-bottom: 12px; }
  @media (max-width: 560px) {
    .scan { grid-template-columns: 1fr; justify-items: center; }
    .scan :global(.qr) { width: min(260px, 100%); }
    .codes { grid-template-columns: 1fr; }
  }
</style>
