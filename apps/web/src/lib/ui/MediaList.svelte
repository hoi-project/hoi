<script lang="ts">
  import type { MediaView } from '@hoi/core-api';
  import { mediaUrls } from '../media-urls.svelte';

  // Media a post carries, from the bytes the core checked (see media-urls).
  let { media }: { media: MediaView[] } = $props();
  const files = mediaUrls();

  $effect(() => files.open(media));
</script>

{#if media.length}
  <div class="media">
    {#each media as m (m.hash)}
      {@const url = files.url(m.hash)}
      {#if !url}
        <div class="ph" style:aspect-ratio={m.w && m.h ? `${m.w} / ${m.h}` : undefined}>{m.available ? 'Opening…' : 'On its way from a peer…'}<span class="mono">{m.mime}</span></div>
      {:else if m.mime.startsWith('image/')}
        <img src={url} alt={m.alt ?? ''} width={m.w} height={m.h} loading="lazy" decoding="async" />
      {:else if m.mime.startsWith('audio/')}
        <audio controls preload="metadata" src={url} aria-label={m.alt ?? 'audio'}></audio>
      {:else}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video controls preload="metadata" src={url} aria-label={m.alt ?? 'video'}></video>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .media { display: grid; gap: 10px; max-width: 640px; margin: 4px 0 16px; }
  img, video { width: 100%; height: auto; border: 2px solid var(--ink); border-radius: 14px; background: var(--soft); display: block; }
  audio { width: 100%; }
  .ph { display: flex; gap: 8px; align-items: center; justify-content: center; min-height: 120px; border: 2px dashed var(--line); border-radius: 14px; color: var(--mute); font-weight: 600; font-size: 14px; }
</style>
