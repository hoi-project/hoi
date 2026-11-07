<script lang="ts">
  import { app } from '../app.svelte';
  import { toast, toastError } from '../toast.svelte';
  import { drawCrop, encodeAvatar } from '../profile-picture';
  import type { AvatarColor } from '@hoi/core-api';
  import Modal from './Modal.svelte';
  let open = $state(false);
  let busy = $state(false);
  let error = $state('');
  let source: HTMLImageElement | undefined = $state();
  let preview: HTMLCanvasElement | undefined = $state();
  let zoom = $state(1);
  let x = $state(0);
  let y = $state(0);
  let generation = 0;
  const colors: { value: AvatarColor; label: string }[] = [
    { value: 'mint', label: 'Mint' }, { value: 'sky', label: 'Sky' },
    { value: 'lilac', label: 'Lilac' }, { value: 'peach', label: 'Peach' },
    { value: 'sun', label: 'Sun' },
  ];

  $effect(() => { if (preview && source) drawCrop(preview, source, zoom, x, y); });

  async function choose(event: Event) {
    const input = event.target as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    const current = ++generation;
    error = '';
    if (!['image/jpeg', 'image/png', 'image/webp'].includes(file.type) || file.size > 10 * 1024 * 1024) {
      error = 'Choose a JPEG, PNG or WebP picture smaller than 10 MB.';
      return;
    }
    const url = URL.createObjectURL(file);
    try {
      const img = new Image();
      img.src = url;
      await img.decode();
      if (current !== generation) return;
      if (!img.naturalWidth || !img.naturalHeight || img.naturalWidth * img.naturalHeight > 40_000_000) throw new Error('Choose a picture smaller than 40 megapixels.');
      source = img;
      zoom = 1; x = 0; y = 0;
      open = true;
    } catch (e) { if (current === generation) error = e instanceof Error ? e.message : 'This picture cannot be opened.'; }
    finally { URL.revokeObjectURL(url); }
  }

  async function save() {
    if (!source) return;
    busy = true;
    try {
      app.me = await app.core.setAvatar(await encodeAvatar(source, zoom, x, y));
      open = false;
      toast('Picture saved. It travels with your next post or reply in each community.');
    } catch (e) { toastError(e); }
    finally { busy = false; }
  }

  async function remove() {
    busy = true;
    try { app.me = await app.core.setAvatar(null); toast('Back to your key picture. Shared with your next post or reply.'); }
    catch (e) { toastError(e); }
    finally { busy = false; }
  }

  async function setColor(color: AvatarColor | null) {
    if (busy) return;
    busy = true;
    try {
      app.me = await app.core.setAvatarColor(color);
      toast('Key picture color saved. Shared with your next post or reply on devices that support it.');
    } catch (e) { toastError(e); }
    finally { busy = false; }
  }
</script>

<div class="profile-picture">
  <div class="pp-head">
    <div class="pp-title">
      <strong>Picture & color</strong>
      <span class="help">Your key draws your picture until you pick one. Changes go out with your next post or reply.</span>
    </div>
    <div class="btnrow">
      <label class="pill pill-sm choose" class:disabled={busy}>
        {app.me?.avatar ? 'Change picture' : 'Choose picture'}
        <input type="file" accept="image/jpeg,image/png,image/webp" onchange={choose} disabled={busy} aria-label="Choose profile picture" />
      </label>
      {#if app.me?.avatar}<button type="button" class="pill pill-sm" onclick={remove} disabled={busy}>Use key picture</button>{/if}
    </div>
  </div>
  {#if !app.me?.avatar}
    <fieldset class="color-picker" disabled={busy}>
      <legend>Key picture color</legend>
      <div class="swatches">
        <label class="color-option" class:selected={!app.me?.avatarColor} title="From key">
          <input type="radio" name="avatar-color" value="default" checked={!app.me?.avatarColor} onchange={() => setColor(null)} />
          <span class="swatch automatic" aria-hidden="true"></span><span class="sr-only">From key</span>
        </label>
        {#each colors as color (color.value)}
          <label class="color-option" class:selected={app.me?.avatarColor === color.value} title={color.label}>
            <input type="radio" name="avatar-color" value={color.value} checked={app.me?.avatarColor === color.value} onchange={() => setColor(color.value)} />
            <span class="swatch" style:background={`var(--${color.value})`} aria-hidden="true"></span><span class="sr-only">{color.label}</span>
          </label>
        {/each}
        <span class="picked" aria-hidden="true">{colors.find((c) => c.value === app.me?.avatarColor)?.label ?? 'From key'}</span>
      </div>
    </fieldset>
  {/if}
  <p class="help small">
    Pictures: a small JPEG (128 px, 4 KB, no camera or location data), shown where a community allows images · private groups and older
    apps show the key picture.
  </p>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<Modal bind:open title="Frame your picture" onclose={() => { source = undefined; }}>
  <p class="help">The circle shows exactly what others will see. Adjust the zoom and position.</p>
  <canvas bind:this={preview} width="256" height="256" class="crop" aria-label="Circular profile picture preview"></canvas>
  <label class="adjust">Zoom <input type="range" min="1" max="4" step="0.01" bind:value={zoom} disabled={busy} /></label>
  <label class="adjust">Horizontal position <input type="range" min="-1" max="1" step="0.01" bind:value={x} disabled={busy} /></label>
  <label class="adjust">Vertical position <input type="range" min="-1" max="1" step="0.01" bind:value={y} disabled={busy} /></label>
  <div class="btnrow"><button type="button" class="pill pill-ink" onclick={save} disabled={busy}>{busy ? 'Saving…' : 'Use this picture'}</button><button type="button" class="pill" onclick={() => { open = false; }} disabled={busy}>Cancel</button></div>
</Modal>

<style>
  .profile-picture { max-width: 100%; display: flex; flex-direction: column; gap: 10px; min-width: 0; }
  .pp-head { display: flex; align-items: center; justify-content: space-between; gap: 10px 16px; flex-wrap: wrap; }
  .pp-title { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1 1 260px; }
  .color-picker { min-width: 0; border: 0; margin: 0; padding: 0; display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
  .color-picker legend { float: left; font-size: 13px; font-weight: 700; margin-right: 12px; padding: 0; }
  .swatches { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .color-option { position: relative; display: inline-flex; align-items: center; justify-content: center; width: 40px; height: 40px; border: 2px solid transparent; border-radius: 50%; cursor: pointer; }
  .color-option.selected { border-color: var(--ink); background: var(--soft); }
  .color-option:focus-within { outline: 3px solid var(--cobalt); outline-offset: 2px; }
  .color-option input { position: absolute; inset: 0; opacity: 0; width: 100%; height: 100%; cursor: pointer; margin: 0; }
  .color-picker:disabled { opacity: 0.6; }
  .swatch { width: 26px; height: 26px; border: 1.5px solid var(--ink); border-radius: 50%; }
  .automatic { background: conic-gradient(var(--mint) 0deg 72deg, var(--sky) 72deg 144deg, var(--lilac) 144deg 216deg, var(--peach) 216deg 288deg, var(--sun) 288deg); }
  .picked { font-size: 13px; font-weight: 700; color: var(--mute); margin-left: 4px; }
  .choose { position: relative; overflow: hidden; cursor: pointer; }
  .choose:focus-within { outline: 3px solid var(--cobalt); outline-offset: 3px; }
  .choose input { position: absolute; inset: 0; opacity: 0; cursor: pointer; width: 100%; }
  .disabled { opacity: 0.5; }
  .small { font-size: 12px; }
  .error { color: var(--danger, #a32929); }
  .crop { width: min(256px, 100%); height: auto; aspect-ratio: 1; border-radius: 50%; align-self: center; border: 2px solid var(--ink); }
  .adjust { display: flex; flex-direction: column; gap: 6px; font-size: 14px; font-weight: 700; }
  .adjust input { width: 100%; accent-color: var(--cobalt); }
</style>
