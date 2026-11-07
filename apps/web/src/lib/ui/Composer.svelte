<script lang="ts">
  // The one orchestrated moment: work bar (the stamp) → id scramble → stamp slams in →
  // composer folds → the new row drops into the feed.
  import type { Hex, PostSummary } from '@hoi/core-api';
  import { onMount, tick as nextTick } from 'svelte';
  import { app, sentTo } from '../app.svelte';
  import { animate, EASE, reduced, slideSheet, unfold } from '../motion';
  import { composerIsSheet } from '../compose';
  import { toast, toastError } from '../toast.svelte';
  import { formatMs } from '../util';
  import { fitImage } from '../media';
  import { to } from '../router';
  import { resource } from '../resource.svelte';
  import { canPublishPublic, postsThroughOthers, syncStatus } from '../network-status';
  import { REGISTRY_URL } from '../registry';

  let {
    community = undefined,
    onposted,
    onclose,
  }: { community?: Hex; onposted: (p: PostSummary) => void; onclose: () => void } = $props();

  // svelte-ignore state_referenced_locally
  let picked = $state<Hex | undefined>(community ?? app.joined[0]?.id);
  let title = $state('');
  let text = $state('');
  let link = $state('');
  let showLink = $state(false);
  let signing = $state(false);
  // Media is earned (PROTOCOL.md §17): trusted members only. The core strips metadata
  // (location, camera, thumbnails) before anything is hashed or shared.
  let files = $state<{ name: string; data: Uint8Array; alt: string }[]>([]);
  let fileInput: HTMLInputElement | undefined = $state();
  const MEDIA_TYPES = 'image/png,image/jpeg,image/gif,image/webp,audio/ogg,video/mp4,video/webm';
  let idText = $state('— waiting —');
  let bits = $state(0);

  let wrap: HTMLElement | undefined = $state();
  let bar: HTMLElement | undefined = $state();
  let stampEl: HTMLElement | undefined = $state();
  let titleEl: HTMLInputElement | undefined = $state();

  const target = $derived(app.joined.find((c) => c.id === picked));
  const fixed = $derived(!!community);
  const standing = resource(() => {
    const id = picked;
    return id ? app.core.standing(id) : Promise.resolve(null);
  });
  const caps = $derived(standing.value?.capabilities ?? ['text', 'links']);
  const connection = $derived(syncStatus(app.net, picked));
  const privateGroup = $derived(app.privateGroups.some(group => group.community === picked));
  const canSend = $derived(privateGroup || canPublishPublic(app.net, picked));

  async function pickFiles(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    for (const f of Array.from(input.files ?? [])) {
      if (files.length >= 4) {
        toast('A post carries at most 4 media items');
        break;
      }
      if (f.size > 64 * 1024 * 1024 || (!f.type.startsWith('image/') && f.size > 16 * 1024 * 1024)) {
        toast(`${f.name} is too big to share`);
        continue;
      }
      // big photos are scaled down to fit instead of refused
      const fitted = await fitImage(f);
      if (fitted.note) toast(fitted.note);
      files = [...files, { name: f.name, data: fitted.data, alt: '' }];
    }
    input.value = '';
  }

  // On a phone the composer is a sheet over the screen; elsewhere it unfolds above the feed.
  // "+ Post" focused the title already (lib/compose.ts); focusing again after the animation
  // only helps when nothing in here has the cursor, and never takes it from the text.
  const sheet = composerIsSheet();
  const focusTitle = () => {
    if (!wrap?.contains(document.activeElement)) titleEl?.focus({ preventScroll: sheet });
  };
  onMount(() => {
    if (!wrap) return;
    if (sheet) {
      document.documentElement.classList.add('composing');
      void slideSheet(wrap, true).then(focusTitle);
    } else {
      wrap.style.height = '0px';
      void unfold(wrap, true).then(focusTitle);
    }
    return () => document.documentElement.classList.remove('composing');
  });

  const fold = () => (wrap ? (sheet ? slideSheet(wrap, false) : unfold(wrap, false)) : Promise.resolve());

  /** Enter in the title goes on to the text; Ctrl/⌘+Enter sends from either; Escape closes an
   * empty composer (one with text in it stays, so nothing typed is lost to a stray key). */
  function keys(e: KeyboardEvent) {
    const form = (e.currentTarget as HTMLElement).closest('form');
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      form?.requestSubmit();
    } else if (e.key === 'Enter' && e.target === titleEl && !e.isComposing) {
      e.preventDefault();
      document.getElementById('ctext')?.focus();
    } else if (e.key === 'Escape' && !signing && !title.trim() && !text.trim() && !files.length) {
      e.preventDefault();
      void close();
    }
  }

  const hex = '0123456789abcdef';
  const rnd = (n: number) => Array.from({ length: n }, () => hex[Math.floor(Math.random() * 16)]).join('');
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

  async function close() {
    await fold();
    onclose();
  }

  async function send(e: SubmitEvent) {
    e.preventDefault();
    if (signing) return;
    if (!picked) return toast('Pick a community first');
    if (!title.trim()) {
      toast('Give it a title first');
      titleEl?.focus();
      return;
    }
    signing = true;
    const still = reduced();
    const scramble = still ? null : setInterval(() => (idText = `b3:${rnd(4)}…${rnd(4)}`), 60);
    if (still) idText = 'stamping…';
    const run: { bar: ReturnType<typeof animate> | null } = { bar: null };
    let took = 0;
    const off = app.onStamp((ev) => {
      if (ev.phase === 'start') {
        bits = ev.bits;
        if (bar && !still) run.bar = animate(bar, { width: ['0%', '94%'], duration: ev.estimateMs, ease: 'inOutQuad' });
        else if (bar) bar.style.width = '60%';
      } else if (ev.tookMs) took = ev.tookMs;
    });
    try {
      await app.requirePostingConnection(picked);
      const media = files.length ? files.map((f) => ({ data: f.data, alt: f.alt.trim() || undefined })) : undefined;
      const post = await app.core.createPost({ community: picked, title: title.trim(), text: text.trim(), link: link.trim() || undefined, media });
      off();
      if (scramble) clearInterval(scramble);
      run.bar?.pause();
      idText = post.short;
      if (bar) {
        if (still) bar.style.width = '100%';
        else await new Promise<void>((r) => animate(bar!, { width: '100%', duration: 160, ease: 'outQuad', onComplete: () => r() }));
      }
      if (stampEl) {
        if (still) {
          stampEl.style.opacity = '1';
          stampEl.style.transform = 'rotate(-8deg)';
        } else {
          await new Promise<void>((r) =>
            animate(stampEl!, { scale: [0.7, 1], rotate: [-4, -8], opacity: [0, 1], duration: 600, ease: EASE.stamp, onComplete: () => r() }),
          );
          await sleep(450);
        }
      }
      await fold();
      onposted(post);
      toast(`${took ? `Signed. Your stamp took ${formatMs(took)}.` : 'Signed and stamped.'} ${sentTo(post.community.id)}`);
      onclose();
    } catch (err) {
      off();
      if (scramble) clearInterval(scramble);
      run.bar?.pause();
      if (bar) bar.style.width = '0%';
      idText = '— waiting —';
      signing = false;
      toastError(err);
      await nextTick();
      titleEl?.focus();
    }
  }
</script>

<div class={['composer', sheet && 'sheet']} bind:this={wrap} role={sheet ? 'dialog' : undefined} aria-modal={sheet ? 'true' : undefined} aria-label={sheet ? 'New post' : undefined}>
  <form class="frame in" onsubmit={send} aria-label="New post" aria-busy={signing}>
    {#if sheet && app.joined.length}
      <div class="sheethead">
        <button type="button" class="pill pill-sm" onclick={close} disabled={signing}>Cancel</button>
        <span class="sheettitle">New post</span>
        <button type="submit" class="pill pill-ink pill-sm" disabled={signing || !canSend}>{signing ? 'Signing…' : 'Send'}</button>
      </div>
    {/if}
    {#if app.joined.length === 0}
      <p class="label">You're not in a community yet.</p>
      <p class="help">Posts live in communities. <a class="textlink" href="#/new">Start one</a> or <a class="textlink" href={to.join()}>open an invite</a>.</p>
      <div class="btnrow"><button type="button" class="pill" onclick={close}>Close</button></div>
    {:else}
      <div class="pickrow">
        <span class="label">New post in</span>
        {#if fixed && target}
          <span class="tag tag-{target.color}">c/{target.name}</span>
        {:else}
          <div class="picker" role="radiogroup" aria-label="Community">
            {#each app.joined as c (c.id)}
              <label class={['chip', `tag-${c.color}`, picked === c.id && 'on']}>
                <input type="radio" name="community" value={c.id} bind:group={picked} disabled={signing} />
                <span>c/{c.name}</span>
              </label>
            {/each}
          </div>
        {/if}
        <span class="stamp-slot" aria-hidden="true"><span class="stamp" bind:this={stampEl}>signed · pow {bits || '…'}</span></span>
      </div>
      <label class="sr-only" for="ctitle">Title</label>
      <input id="ctitle" class="input titlein" bind:this={titleEl} bind:value={title} maxlength="300" placeholder="A title people can scan" disabled={signing} autocomplete="off" enterkeyhint="next" onkeydown={keys} />
      <label class="sr-only" for="ctext">Text</label>
      <textarea id="ctext" class="textarea" bind:value={text} rows={sheet ? 6 : 3} disabled={signing} onkeydown={keys}
        placeholder="Say something. You can keep writing while waiting for a connection."></textarea>
      <p class="help">{privateGroup ? 'Your post is saved here for encrypted delivery.' : postsThroughOthers(app.net, app.boot?.platform ?? 'web') ? `Nobody can read what you post from this ${app.platformName} until a home node carrying this community has it: a member's, a public one, or your own. Keep hoi open while the signed post syncs; a connection does not confirm delivery.` : 'Posting needs a connected node carrying this community. Keep hoi open while the signed post syncs; a connection does not confirm delivery.'}</p>
      {#if !canSend}
        <p class="help" role="status"><strong>{connection.title}.</strong> {connection.detail} Your text stays in this editor while you reconnect.</p>
        {#if target?.role === 'owner'}<p class="help">A new open community is offered to the public nodes this {app.platformName} reaches, and kept by those that take it. Pair your own home node, or open a node link, to give it a copy. A registry listing alone does not store the community's posts.</p>{/if}
        <div class="btnrow">{#if REGISTRY_URL}<button type="button" class="pill" disabled={app.publicConnection.busy || signing} onclick={() => app.connectPublic(picked, true)}>{app.publicConnection.busy ? 'Finding public relays…' : 'Retry public relays'}</button>{/if}<a class="textlink" href={to.communities()}>Open a node link</a><a class="textlink" href={to.join()}>Open a member's invite</a></div>
      {/if}
      {#if standing.value && !caps.includes('links')}
        <p class="help">You're new in c/{target?.name}: text only for now, no links. Members vouch for you by reading what you write.</p>
      {/if}
      {#if files.length}
        <ul class="files">
          {#each files as f, i (i)}
            <li>
              <span class="mono">{f.name}</span>
              <label class="sr-only" for={`alt-${i}`}>Describe {f.name}</label>
              <input id={`alt-${i}`} class="input" bind:value={f.alt} maxlength="500" placeholder="Describe it for people who can't see it" disabled={signing} />
              <button type="button" class="act" onclick={() => (files = files.filter((_, j) => j !== i))} disabled={signing}>Remove</button>
            </li>
          {/each}
        </ul>
        <p class="help">Location, camera details and thumbnails are stripped on this {app.platformName} before anything is shared.</p>
      {/if}
      {#if showLink}
        <label class="field">
          <span>Link</span>
          <input class="input" type="url" inputmode="url" bind:value={link} placeholder="https://…" disabled={signing} />
          <span class="help">Shown as a link. Nobody's device fetches it unless they turn on link previews.</span>
        </label>
      {/if}
      <div class="work" aria-hidden="true"><i bind:this={bar}></i></div>
      <div class="btnrow">
        {#if !sheet}
          <button type="submit" class="pill pill-ink" disabled={signing || !canSend}>{signing ? 'Signing…' : 'Sign & send'}</button>
          <button type="button" class="pill" onclick={close} disabled={signing}>Cancel</button>
        {/if}
        {#if !showLink && caps.includes('links')}<button type="button" class="act" onclick={() => (showLink = true)} disabled={signing}>+ Add a link</button>{/if}
        {#if caps.includes('media')}
          <button type="button" class="act" onclick={() => fileInput?.click()} disabled={signing || files.length >= 4}>+ Add images or media</button>
          <input class="sr-only" type="file" accept={MEDIA_TYPES} multiple bind:this={fileInput} onchange={pickFiles} tabindex="-1" aria-hidden="true" />
        {/if}
        <span class="mono idline" aria-live="polite">id <span>{idText}</span></span>
      </div>
    {/if}
  </form>
</div>

<style>
  .composer { overflow: hidden; scroll-margin-top: 84px; }
  /* Phones: the whole screen, Cancel and Send at the top where the keyboard never covers them. */
  .composer.sheet {
    position: fixed; inset: 0; z-index: 25; overflow-y: auto; overscroll-behavior: contain; background: var(--bg);
    padding: env(safe-area-inset-top, 0px) env(safe-area-inset-right, 0px) env(safe-area-inset-bottom, 0px) env(safe-area-inset-left, 0px);
  }
  .sheet .in { margin: 0; min-height: 100%; border: 0; border-radius: 0; box-shadow: none; padding-top: 0; }
  .sheethead {
    position: sticky; top: 0; z-index: 1; display: flex; align-items: center; justify-content: space-between; gap: 10px;
    margin: 0 -14px; padding: 10px 14px; background: var(--card); border-bottom: 2px solid var(--ink);
  }
  .sheettitle { font-weight: 800; font-size: 16px; }
  .sheet .titlein { font-size: 18px; }
  :global(html.composing) { overflow: hidden; }
  .in { padding: 18px; display: flex; flex-direction: column; gap: 12px; position: relative; margin: 14px 0 18px; }
  .pickrow { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }
  .picker { display: flex; gap: 6px; flex-wrap: wrap; }
  .chip { position: relative; display: inline-flex; align-items: center; min-height: 32px; padding: 0 12px; border-radius: 999px; font-weight: 700; font-size: 13px; color: var(--tagink); border: 2px solid transparent; cursor: pointer; }
  .chip input { position: absolute; inset: 0; opacity: 0; margin: 0; cursor: pointer; }
  .chip.on { border-color: var(--ink); box-shadow: 2px 2px 0 var(--ink); }
  .chip:has(input:focus-visible) { outline: 3px solid var(--cobalt); outline-offset: 2px; }
  .titlein { font-weight: 700; }
  .work { height: 8px; border-radius: 999px; background: var(--soft); overflow: hidden; }
  .work i { display: block; height: 100%; width: 0; background: var(--coral); border-radius: 999px; }
  .stamp {
    display: inline-block; padding: 6px 12px; border-radius: 10px; border: 3px solid var(--cobalt); color: var(--cobalt); white-space: nowrap;
    font-weight: 900; font-size: 12px; letter-spacing: .06em; text-transform: uppercase; transform: scale(0); opacity: 0; background: var(--card); pointer-events: none;
  }
  .stamp-slot { display: inline-flex; align-items: center; justify-content: center; margin-left: auto; min-height: 58px; padding: 12px; flex-shrink: 0; }
  .idline { margin-left: auto; }
  .files { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .files li { display: grid; grid-template-columns: minmax(0, 160px) minmax(0, 1fr) auto; gap: 8px; align-items: center; }
  .files .mono { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  @media (max-width: 640px) { .files li { grid-template-columns: 1fr auto; } .files .input { grid-column: 1 / -1; } }
  @media (max-width: 640px) {
    .in { padding: 14px; }
    .chip { min-height: 40px; }
    .idline { margin-left: 0; width: 100%; }
  }
</style>
