<script lang="ts">
  // First run, drawn like the website hero: pick a name, then start / open an invite / bring your key.
  // Making a key opens "say hoi" (SayHoi.svelte), the three acts of what making it really does.
  import type { InvitePreview, Me } from '@hoi/core-api';
  import { looksLikeRecoveryKey } from '@hoi/core-api';
  import { onMount } from 'svelte';
  import { app, setTitle } from '../lib/app.svelte';
  import type { Route } from '../lib/router';
  import { router } from '../lib/router.svelte';
  import { to } from '../lib/router';
  import { MAX_KEY_FILE, inviteQuery, joinLink, readFile } from '../lib/util';
  import { enterView, popIn } from '../lib/motion';
  import { errorText, toast } from '../lib/toast.svelte';
  import { backupLogin, credentialValue } from '../lib/password-manager';
  import Logo from '../lib/ui/Logo.svelte';
  import Modal from '../lib/ui/Modal.svelte';
  import LinkDevice from '../lib/ui/LinkDevice.svelte';
  import AskFriends from '../lib/ui/AskFriends.svelte';
  import SayHoi from '../lib/ui/SayHoi.svelte';

  let { route }: { route: Route } = $props();

  let name = $state('');
  let link = $state('');
  let pass = $state('');
  let file = $state<File | null>(null);
  let recoveryKey = $state('');
  let restoreName = $state('');
  let restoreMethod = $state<'recovery' | 'file'>('recovery');
  let recoveryEntry = $state('');
  let fileEntry = $state('');
  let restoreError = $state('');
  let showSecret = $state(false);
  let busy = $state<'' | 'start' | 'invite' | 'key' | 'restore' | 'look' | 'join'>('');
  let nameEl: HTMLInputElement | undefined = $state();
  let linkOpen = $state(false);
  let askOpen = $state(false);
  let preview = $state.raw<InvitePreview | null>(null);
  let game = $state<{ name: string; next: string; done: (ok: boolean) => void } | null>(null);

  const pendingJoin = $derived(route.name === 'join' && route.query ? route.query : '');
  const pendingPublic = $derived(route.name === 'public' && !!route.query);
  const pendingNode = $derived(route.name === 'node' && !!route.query);

  onMount(() => setTitle('Say hoi'));

  $effect(() => {
    const q = pendingJoin;
    preview = null;
    if (!q) return;
    app.core.previewInvite(joinLink(q)).then(
      (p) => (preview = p),
      () => (preview = null),
    );
  });

  function needName(): string | null {
    const n = name.trim();
    if (n) return n;
    toast('Pick a name first. Any name.');
    nameEl?.focus();
    return null;
  }

  /** Plays "say hoi"; resolves once the key is made and met (true) or the game was left (false). */
  function makeKey(n: string, next: string): Promise<boolean> {
    return new Promise((done) => (game = { name: n, next, done }));
  }

  async function keyMade(me: Me) {
    const g = game;
    game = null;
    await app.identityChanged(me);
    toast(`Key made. Hi, ${me.name}.`);
    g?.done(true);
  }

  function gameLeft() {
    const g = game;
    game = null;
    g?.done(false);
  }

  async function start() {
    const n = needName();
    if (!n) return;
    busy = 'start';
    if (await makeKey(n, 'Start my community →')) router.go('#/new');
    busy = '';
  }

  async function openInvite(e: SubmitEvent) {
    e.preventDefault();
    const q = inviteQuery(link);
    if (!q) {
      toast("That isn't an invite link. They start with hoi://join? or end in #/join?…");
      return;
    }
    const n = needName();
    if (!n) return;
    busy = 'invite';
    if (await makeKey(n, 'Open the invite →')) router.go(to.join(q));
    busy = '';
  }

  async function joinPending() {
    const n = needName();
    if (!n) return;
    busy = 'join';
    await makeKey(n, `See c/${preview?.community.name ?? 'the community'} →`); // the route is already #/join?…, so the join preview shows next
    busy = '';
  }

  async function nodePending() {
    const n = needName();
    if (!n) return;
    busy = 'join';
    await makeKey(n, 'Look at this node →'); // the route is already #/node?…, so the node page shows next
    busy = '';
  }

  async function back(me: Me, method?: 'recovery' | 'file') {
    await app.identityChanged(me);
    toast(method === 'recovery' ? `Identity restored, ${me.name}. Standing returns as your communities sync. Private communities may need a new invitation.`
      : method === 'file' ? `Backup restored, ${me.name}. Your saved profile and settings are back. Posts need syncing; private communities may need a new invitation.`
      : `Welcome back, ${me.name}.`);
    if (!pendingJoin && !pendingPublic && !pendingNode) router.go('#/');
  }

  async function bringKey(e: SubmitEvent) {
    e.preventDefault();
    if (busy) return;
    restoreError = '';
    const form = e.currentTarget as HTMLFormElement;
    const selected = new FormData(form).get('key-file');
    file = selected instanceof File && selected.size ? selected : null;
    pass = credentialValue(form);
    if (!file) { restoreError = 'Choose your .hoikey backup file first.'; return; }
    if (file.size > MAX_KEY_FILE) { restoreError = 'This file is too large to be a hoi key backup.'; return; }
    busy = 'key';
    try {
      await back(await app.core.importIdentity(await readFile(file), pass), 'file');
    } catch (err) {
      restoreError = errorText(err);
    }
    pass = '';
    busy = '';
  }

  /** A recovery key from a password manager or a drawer: you come back under a new root that takes
   * over from the old one, with a fresh device key here; your standing stays. */
  async function restore(e: SubmitEvent) {
    e.preventDefault();
    if (busy) return;
    restoreError = '';
    const form = e.currentTarget as HTMLFormElement;
    recoveryKey = credentialValue(form);
    restoreName = credentialValue(form, 'display-name');
    const key = recoveryKey.trim();
    if (!looksLikeRecoveryKey(key)) { restoreError = 'Choose your recovery-key entry, not a vault or backup-file password. The key starts with hoi-key-1-.'; return; }
    const n = (restoreName || name).trim();
    if (!n) { restoreError = 'Enter the name you want people to see. It can be the name you used before.'; return; }
    busy = 'restore';
    try {
      await back(await app.core.restoreIdentity(key, n), 'recovery');
    } catch (err) {
      restoreError = errorText(err);
    }
    recoveryKey = '';
    busy = '';
  }

  function chooseRestore(method: 'recovery' | 'file') {
    restoreMethod = method;
    restoreError = '';
    recoveryKey = pass = '';
    file = null;
    showSecret = false;
  }

  async function lookAround() {
    const n = needName();
    if (!n) return;
    busy = 'look';
    if ((await makeKey(n, 'Look around →')) && !pendingJoin && !pendingPublic && !pendingNode) router.go('#/');
    busy = '';
  }
</script>

<div class="first" use:enterView>
  <header class="ftop gutter">
    <span class="logo"><Logo size={30} /><span>hoi<span class="sh">.sh</span></span></span>
    <span class="tag tag-soft">nothing leaves this {app.platformName} yet</span> <a class="textlink" href="#/privacy">Privacy notice</a>
  </header>

  <main id="main" class="wrap gutter">
    <section class="hero">
      <span class="sticker" use:popIn>hoi.shhh · say hi quietly</span>
      <h1 class="anim">Say hoi. <span class="blue">Pick a name, get a key.</span></h1>
      <p class="lead anim">
        Anyone can pick any name, even one somebody else already uses. What counts is your key: it's made right here, on this
        {app.platformName}, and it signs everything you post. No email or phone number. Your vault password protects this device; your key is your identity.
      </p>

      <section class="returning frame" aria-labelledby="returning-title">
        <div class="rhead">
          <span class="label">Already use hoi?</span>
          <h2 id="returning-title">Restore my identity</h2>
          <p>Use your saved recovery key or encrypted backup file. Your new vault password protects this installation; it does not replace either backup.</p>
          <button type="button" class="pill pill-ink" onclick={() => (linkOpen = true)} disabled={!!busy}>Link a signed-in device</button>
          <div class="btnrow" role="group" aria-label="Choose a backup to restore">
            <button type="button" class="pill" aria-pressed={restoreMethod === 'recovery'} onclick={() => chooseRestore('recovery')} disabled={!!busy}>Use recovery key</button>
            <button type="button" class="pill" aria-pressed={restoreMethod === 'file'} onclick={() => chooseRestore('file')} disabled={!!busy}>Use backup file</button>
          </div>
        </div>
        {#if restoreMethod === 'recovery'}
        <form id="restore-recovery" class="restore" onsubmit={restore} autocomplete="on" aria-busy={!!busy}>
          <h3>Recovery key</h3>
          <label class="field" for="fr-recovery-entry"><span>Password-manager entry</span><input id="fr-recovery-entry" class="input" name="username" autocomplete="username" bind:value={recoveryEntry} placeholder="Select your hoi key entry" spellcheck="false" autocapitalize="none" disabled={!!busy} /></label>
          <label class="field" for="fr-recovery"><span>Recovery key</span><input id="fr-recovery" class="input mono-in" name="password" type={showSecret ? 'text' : 'password'} bind:value={recoveryKey} placeholder="hoi-key-1-…" autocomplete="current-password" spellcheck="false" autocapitalize="none" required disabled={!!busy} aria-describedby="fr-recovery-help" /></label>
          <label class="show-secret"><input type="checkbox" bind:checked={showSecret} /> Show recovery key</label>
          <label class="field" for="fr-restore-name"><span>Your name</span><input id="fr-restore-name" class="input" name="display-name" bind:value={restoreName} maxlength="32" placeholder={name || 'lena'} autocomplete="nickname" required disabled={!!busy} /></label>
          <p class="help" id="fr-recovery-help">Select the “hoi key” entry you saved in Dashlane, 1Password or your browser's manager, or paste its password here. This restores your identity with a new signing key and keeps your standing. Private communities may need a new invitation.</p>
          {#if restoreError}<p class="restore-error" role="alert">{restoreError}</p>{/if}
          {#if busy}<p role="status">Restoring your identity and preparing this device's key…</p>{/if}
          <button type="submit" class="pill pill-ink" disabled={!!busy}>{busy === 'restore' ? 'Bringing you back…' : 'Restore with recovery key'}</button>
          <button type="button" class="lnk lnk-ink" onclick={() => (askOpen = true)} disabled={!!busy}>No line? Ask the friends who keep it</button>
        </form>
        {:else}
        <form id="restore-file" class="keyfile" onsubmit={bringKey} autocomplete="on" aria-busy={!!busy}>
          <h3>Encrypted backup file</h3>
          <label class="field" for="fr-key-file"><span>Key backup (.hoikey)</span><input id="fr-key-file" name="key-file" type="file" accept=".hoikey,application/octet-stream" onchange={(e) => { file = e.currentTarget.files?.[0] ?? null; fileEntry = file ? backupLogin(file.name) : ''; }} required disabled={!!busy} /></label>
          <label class="field" for="fr-file-entry"><span>Password-manager entry</span><input id="fr-file-entry" class="input" name="username" autocomplete="username" bind:value={fileEntry} placeholder="Select the password saved for this file" disabled={!!busy} spellcheck="false" autocapitalize="none" /></label>
          <label class="field" for="fr-pass"><span>Backup-file password</span><input id="fr-pass" class="input" name="password" type={showSecret ? 'text' : 'password'} bind:value={pass} placeholder="Password chosen when exporting" autocomplete="current-password" required disabled={!!busy} aria-describedby="fr-file-help" /></label>
          <label class="show-secret"><input type="checkbox" bind:checked={showSecret} /> Show password</label>
          <p class="help" id="fr-file-help">Choose the password saved for this file. Older backups may use the entry “hoi — key backup”. The file restores your key, profile, community list and settings; it contains no private-group history. Your local vault password stays the same.</p>
          {#if restoreError}<p class="restore-error" role="alert">{restoreError}</p>{/if}
          {#if busy}<p role="status">Opening your backup and preparing this device's key…</p>{/if}
          <button type="submit" class="pill pill-ink" disabled={!!busy}>{busy === 'key' ? 'Opening…' : 'Restore backup file'}</button>
        </form>
        {/if}
      </section>

      {#if pendingJoin}
        <div class="frame invitenote anim">
          <span class="label">You're opening an invite</span>
          {#if preview}
            <div class="meta">
              <span class="tag tag-{preview.community.color} big">c/{preview.community.name}</span>
              {#if preview.verified}<span class="tag tag-trust">✓ signed details check out</span>{/if}
            </div>
            <p class="help">{preview.community.description}</p>
          {:else}
            <p class="help">Checking it…</p>
          {/if}
        </div>
      {/if}

      <label class="namefield anim">
        <span>New here? What should people call you?</span>
        <input class="input big" bind:this={nameEl} bind:value={name} maxlength="32" placeholder="lena" autocomplete="nickname" />
        <span class="help">You can change it later. People see it next to a short piece of your key, like <span class="mono">e04c·2a9f</span>.</span>
      </label>
      {#if pendingPublic}
        <div class="frame invitenote anim"><p>You're opening a public community. Make a local key to continue; no account, name or personal key is sent to the registry. The app will then verify the community through its nodes.</p></div>
        <div class="btnrow anim"><button type="button" class="pill pill-coral pill-lg" onclick={joinPending} disabled={!!busy}>Make my key & view public community →</button></div>
      {/if}
      {#if pendingNode}
        <div class="frame invitenote anim"><p>You're opening a link to a public node someone runs. Make a local key, then look at the open communities it carries; nothing is joined until you choose one.</p></div>
        <div class="btnrow anim"><button type="button" class="pill pill-coral pill-lg" onclick={nodePending} disabled={!!busy}>{busy === 'join' ? 'Making your key…' : 'Make my key & look at this node →'}</button></div>
      {/if}
      {#if pendingJoin}
        <div class="btnrow anim">
          <button type="button" class="pill pill-coral pill-lg" onclick={joinPending} disabled={!!busy}>
            {busy === 'join' ? 'Making your key…' : `Make my key & see c/${preview?.community.name ?? 'the community'} →`}
          </button>
        </div>
      {/if}
    </section>

    <section class="trio" aria-label="Ways in">
      <div class="frame card anim">
        <span class="num" aria-hidden="true">1</span>
        <h2>Start a community</h2>
        <p>A small place for people who care about the same thing. You hold its key, pick its colour and how long posts live.</p>
        <button type="button" class="pill pill-ink" onclick={start} disabled={!!busy}>{busy === 'start' ? 'Making your key…' : 'Start one →'}</button>
      </div>

      <form class="frame card anim" onsubmit={openInvite}>
        <span class="num" aria-hidden="true">2</span>
        <h2>Open an invite</h2>
        <p>Got a link from a friend? Paste it. It carries everything your {app.platformName} needs to find the others.</p>
        <label class="sr-only" for="fr-link">Invite link</label>
        <input id="fr-link" class="input mono-in" bind:value={link} placeholder="hoi://join?… or https://…#/join?…" autocomplete="off" />
        <button type="submit" class="pill pill-ink" disabled={!!busy}>{busy === 'invite' ? 'Making your key…' : 'Open it →'}</button>
      </form>

      <div class="frame card anim">
        <span class="num" aria-hidden="true">3</span>
        <h2>Look around</h2>
        <p>Make your key and explore. Each new key requires its own work and starts with no reputation. Keep and back up one key to build on it.</p>
        <button type="button" class="pill" onclick={lookAround} disabled={!!busy}>{busy === 'look' ? 'Making your key…' : 'Make my key & explore →'}</button>
      </div>
    </section>
  </main>
</div>

{#if game}
  <SayHoi name={game.name} next={game.next} ondone={keyMade} oncancel={gameLeft} />
{/if}

<Modal bind:open={linkOpen} title="Link to a device you already use">
  <LinkDevice newOnly ondone={() => (linkOpen = false)} />
</Modal>

<Modal bind:open={askOpen} title="Ask your friends">
  <AskFriends name={(restoreName || name).trim()} ondone={(me) => { askOpen = false; void back(me); }} />
</Modal>

<style>
  .first { min-height: 100dvh; padding-top: env(safe-area-inset-top, 0px); padding-bottom: max(40px, env(safe-area-inset-bottom, 0px)); }
  .ftop { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; height: 72px; max-width: 1120px; margin: 0 auto; border-bottom: 2px solid var(--ink); }
  .logo { display: inline-flex; align-items: center; gap: 10px; font-weight: 900; font-size: 21px; letter-spacing: -0.02em; }
  .sh { color: var(--mute); font-weight: 600; }
  .ftop .tag { margin-left: auto; }
  .wrap { max-width: 1120px; margin: 0 auto; }
  .hero { padding-block: clamp(40px, 7vw, 88px) 28px; display: flex; flex-direction: column; gap: 26px; align-items: flex-start; }
  h1 { font-size: var(--type-hero-size); line-height: var(--type-hero-lh); letter-spacing: var(--type-hero-tracking); max-width: 11em; }
  .blue { color: var(--cobalt); }
  .lead { font-size: var(--type-lead-size); line-height: var(--type-lead-lh); color: var(--mute); max-width: 36em; }
  .returning { width: 100%; display: grid; grid-template-columns: 1fr 1.2fr; gap: 26px; padding: 24px; }
  .show-secret { display: flex; align-items: center; gap: 8px; min-height: 44px; }
  .restore-error { padding: 12px; border-radius: 12px; background: var(--peach); color: var(--ink) !important; }
  .returning > div, .returning form { display: flex; flex-direction: column; align-items: flex-start; gap: 14px; min-width: 0; }
  .returning h2 { font-size: 26px; letter-spacing: -0.03em; }
  .returning h3 { font-size: 18px; letter-spacing: -0.02em; }
  .returning p { color: var(--mute); line-height: 1.5; }
  .returning .help { font-size: 14px; }
  .returning .field { width: 100%; }
  .returning input { max-width: 100%; }
  .namefield { display: flex; flex-direction: column; gap: 8px; width: min(520px, 100%); }
  .namefield > span:first-child { font-weight: 800; font-size: 16px; }
  .input.big { font-size: 22px; font-weight: 800; min-height: 56px; border-color: var(--ink); background: var(--card); border-radius: 18px; }
  .invitenote { padding: 14px 16px; display: flex; flex-direction: column; gap: 8px; width: min(620px, 100%); }
  .trio { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 16px; padding-block: 18px 8px; border-top: 2px dotted var(--line); }
  .card { padding: 20px; display: flex; flex-direction: column; gap: 12px; align-items: flex-start; position: relative; min-width: 0; }
  .card h2 { font-size: 26px; letter-spacing: -0.03em; }
  .card p { color: var(--mute); font-weight: 500; line-height: 1.5; }
  .card .input { width: 100%; }
  .num {
    width: 34px; height: 34px; border-radius: 50%; border: 2px solid var(--ink); display: inline-flex; align-items: center; justify-content: center;
    font-weight: 900; font-size: 14px; background: var(--sun); color: var(--tagink);
  }
  .card:nth-child(2) .num { background: var(--mint); }
  .card:nth-child(3) .num { background: var(--lilac); }
  @media (max-width: 1040px) {
    .returning { grid-template-columns: 1fr 1fr; }
    .returning .rhead { grid-column: 1 / -1; }
  }
  @media (max-width: 860px) {
    .trio { grid-template-columns: 1fr; }
    .returning { grid-template-columns: 1fr; }
  }
  @media (max-width: 480px) {
    .ftop .tag { display: none; }
  }
</style>
