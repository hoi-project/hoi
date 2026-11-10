<script lang="ts">
  import { onMount } from 'svelte';
  import { RELEASE, latestRelease } from '../preview';
  import { networkConsent } from '../consent.svelte';
  import { noticeForBuild, type ReleaseConfig } from '../../../../../scripts/release-schema.mjs';

  let remote = $state.raw<ReleaseConfig | null>(null);
  const notice = $derived(noticeForBuild(RELEASE, remote));

  onMount(() => {
    let active = true;
    const refresh = async () => {
      if (document.visibilityState === 'hidden') return;
      if (__HOI_DESKTOP__ || !networkConsent.accepted) return;
      try { const next = await latestRelease(); if (active && next && next.revision >= (remote?.revision ?? RELEASE.revision)) remote = next; }
      catch { /* Keep the truthful bundled warning offline or on an invalid response. */ }
    };
    void refresh();
    const timer = setInterval(() => void refresh(), 60 * 60_000);
    document.addEventListener('visibilitychange', refresh);
    return () => { active = false; clearInterval(timer); document.removeEventListener('visibilitychange', refresh); };
  });
</script>

<div class="preview" aria-label="Release status" role="status">
  <span class="dot" aria-hidden="true"></span>
  <div class="body">
    <strong>{notice.title}</strong> <span>{notice.message}</span>
    {#if notice.update}<span class="update">{notice.update}</span>{/if}
  </div>
</div>

<style>
  .preview { display: flex; align-items: flex-start; gap: 10px; padding: 10px 16px; background: var(--sun); color: var(--tagink, #1a1a1a); border-bottom: 2px solid var(--ink); font-weight: 600; font-size: 13px; line-height: 1.45; }
  .body { flex: 1; min-width: 0; overflow-wrap: anywhere; }
  strong { font-weight: 900; }
  .update { display: block; margin-top: 3px; font-size: 12px; }
  .dot { width: 9px; height: 9px; margin-top: 5px; border-radius: 50%; background: var(--coral); border: 2px solid var(--ink); flex: none; }
</style>
