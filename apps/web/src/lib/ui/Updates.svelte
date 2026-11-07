<script lang="ts">
  import { onDestroy } from 'svelte';
  import { isDesktopShell } from '../host';
  import { PUBLICATION } from '../preview';
  import { fetchVerifiedManifest, compareVersions } from '../../../../../scripts/release-verify.mjs';
  import { sw, checkWebUpdate, applyUpdate } from '../sw.svelte';
  import { errorText } from '../toast.svelte';

  const desktop = isDesktopShell();
  const updateError = (e: unknown) => typeof e === 'string' ? e : errorText(e);
  type Result = { state: 'unavailable' | 'available' | 'current'; version?: string; message: string };
  let result = $state<Result | null>(null);
  let busy = $state('');
  let error = $state('');
  let ready = $state(false);
  let progress = $state(0);
  let unlisten: (() => void) | null = null;
  let active = true;
  onDestroy(() => { active = false; unlisten?.(); });

  async function check() {
    if (busy) return;
    error = ''; busy = 'Checking for updates…'; ready = false; result = null;
    try {
      if (desktop) {
        const { invoke } = await import('@tauri-apps/api/core');
        result = await invoke<Result>('hoi_update_check');
      } else {
        // The browser's worker keeps the last complete cache if an update fails.
        await checkWebUpdate();
        if (!PUBLICATION.manifestKey || !PUBLICATION.downloads) result = { state: 'unavailable', message: 'Signed release checks are unavailable in this development build.' };
        else {
          const m = await fetchVerifiedManifest(PUBLICATION, __HOI_VERSION__);
          result = { state: compareVersions(m.latest, __HOI_VERSION__) > 0 ? 'available' : 'current', version: m.latest, message: m.release.notice.message };
        }
      }
    } catch (e) { error = `${updateError(e)} Your installed app and local data are unchanged.`; }
    finally { busy = ''; }
  }
  async function download() {
    if (busy) return;
    error = ''; busy = 'Downloading and verifying…'; progress = 0;
    try {
      const [{ invoke }, { listen }] = await Promise.all([import('@tauri-apps/api/core'), import('@tauri-apps/api/event')]);
      const stop = await listen<{ downloaded: number; total: number }>('hoi:update-progress', e => { progress = Math.min(100, Math.round(e.payload.downloaded / e.payload.total * 100)); });
      if (!active) { stop(); return; }
      unlisten = stop;
      await invoke('hoi_update_download'); ready = true;
    } catch (e) { error = updateError(e); }
    finally { unlisten?.(); unlisten = null; busy = ''; }
  }
  async function install() {
    if (busy || !ready) return;
    busy = 'Saving local changes and installing…'; error = '';
    try { const { invoke } = await import('@tauri-apps/api/core'); await invoke('hoi_update_install'); }
    catch (e) { error = updateError(e); busy = ''; }
  }
</script>

<section class="updates" aria-label="App updates">
  <h3>App updates</h3>
  <p class="help">Installed: {__HOI_VERSION__}. {desktop ? 'Checking contacts the static release host. Downloads are verified before you choose to install.' : 'The browser keeps a complete cached app while an update downloads. Reload when your writing is saved.'}</p>
  <div class="actions">
    <button type="button" class="pill pill-sm" disabled={!!busy} onclick={check}>{error ? 'Retry update check' : 'Check for updates'}</button>
    {#if desktop && result?.state === 'available' && !ready}<button type="button" class="pill pill-sm" disabled={!!busy} onclick={download}>Download and verify {result.version}</button>{/if}
    {#if desktop && ready}<button type="button" class="pill pill-sm pill-sun" disabled={!!busy} onclick={install}>Install and restart</button>{/if}
    {#if !desktop && sw.waiting}<button type="button" class="pill pill-sm pill-sun" onclick={applyUpdate}>Reload to update</button>{/if}
  </div>
  <div role="status" aria-live="polite">
    {#if busy}<p>{busy}</p>{/if}
    {#if busy === 'Downloading and verifying…'}<progress max="100" value={progress} aria-label="Update download">{progress}%</progress>{/if}
    {#if ready && !busy}<p>Verified and ready. Installing restarts hoi; save any unfinished writing first.</p>
    {:else if result}<p>{result.state === 'current' ? 'This app is up to date. ' : result.state === 'available' ? `Version ${result.version} is available. ` : ''}{result.message}</p>{/if}
    {#if !desktop && sw.error}<p>{sw.error}</p>{/if}
    {#if !desktop && result?.state === 'available' && !sw.waiting}<p>The signed release is available, but the browser update is not cached yet. Try again after the site has been updated.</p>{/if}
  </div>
  {#if error}<p role="alert" class="error">{error}</p>{/if}
</section>

<style>
  .updates { margin-block: 20px; }
  .actions { display: flex; flex-wrap: wrap; gap: 8px; }
  .actions button { min-height: 44px; }
  p { line-height: 1.5; overflow-wrap: anywhere; }
  progress { width: min(100%, 320px); }
  .error { color: var(--ink); border-left: 4px solid var(--coral); padding-left: 10px; }
</style>
