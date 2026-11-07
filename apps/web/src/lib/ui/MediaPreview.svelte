<script lang="ts">
  import type { MediaView } from '@hoi/core-api';
  import { mediaUrls } from '../media-urls.svelte';

  // A post's picture in a feed row, scaled down, linking to the post. The bytes open once the
  // row nears the screen, so a long feed doesn't decode every picture up front. The box has the
  // picture's stated shape from the start, so the row doesn't jump when it opens, and takes the
  // shape it is shown in once open (a photo turned by its EXIF orientation).
  let { item, href, more = 0 }: { item: MediaView; href: string; more?: number } = $props();

  const MAX_W = 560;
  const MAX_H = 340;
  const MIN_W = 120;
  const files = mediaUrls();
  const url = $derived(files.url(item.hash));
  let shown = $state<{ w: number; h: number } | null>(null);
  const size = $derived(shown ?? (item.w && item.h ? { w: item.w, h: item.h } : null));
  const ratio = $derived(size ? size.w / size.h : 4 / 3);
  const width = $derived(Math.round(Math.max(MIN_W, Math.min(MAX_W, size?.w ?? MAX_W, MAX_H * ratio))));
  let el: HTMLElement | undefined = $state();
  let near = $state(false);

  $effect(() => {
    if (!el || near) return;
    if (typeof IntersectionObserver === 'undefined') {
      near = true;
      return;
    }
    const io = new IntersectionObserver(
      (es) => {
        if (es.some((e) => e.isIntersecting)) near = true;
      },
      { rootMargin: '600px 0px' },
    );
    io.observe(el);
    return () => io.disconnect();
  });
  $effect(() => {
    if (near) files.open([item]);
  });
</script>

<a class="preview" {href} tabindex="-1" bind:this={el} style:width={`${width}px`} style:aspect-ratio={ratio}>
  {#if url}
    <img
      src={url}
      alt={item.alt ?? ''}
      decoding="async"
      onload={(e) => {
        const i = e.currentTarget as HTMLImageElement;
        if (i.naturalWidth && i.naturalHeight) shown = { w: i.naturalWidth, h: i.naturalHeight };
      }}
    />
  {:else}
    <span class="ph">{item.available ? 'Opening…' : 'On its way from a peer…'}</span>
  {/if}
  {#if more > 0}<span class="more">+{more}</span>{/if}
</a>

<style>
  .preview {
    position: relative; display: flex; align-items: center; justify-content: center; max-width: 100%; min-height: 48px;
    border: 2px solid var(--ink); border-radius: 14px; background: var(--soft); overflow: hidden;
  }
  img { display: block; width: 100%; height: 100%; object-fit: scale-down; }
  .ph { padding: 0 12px; color: var(--mute); font-weight: 600; font-size: 13px; text-align: center; }
  .more {
    position: absolute; top: 8px; right: 8px; padding: 2px 10px; border-radius: 999px;
    background: var(--ink); color: var(--onink); font-weight: 700; font-size: 13px;
  }
</style>
