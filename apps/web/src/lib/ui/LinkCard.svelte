<script lang="ts">
  // A post's link: with previews on, the page's own title, description and picture,
  // fetched by a desktop app or hoi-node and kept on this device; else the plain address.
  import { app } from '../app.svelte';
  import { resource } from '../resource.svelte';

  let { url, full = false }: { url: URL; full?: boolean } = $props();
  const on = $derived(!!app.settings?.linkPreviews);
  const preview = resource(() => {
    void app.previewRev;
    return on ? app.core.linkPreview(url.href) : Promise.resolve(null);
  });
  const p = $derived(preview.value);
  const where = $derived(`${url.host}${full && url.pathname !== '/' ? url.pathname : ''}`);
</script>

{#if on && p && (p.title || p.description || p.image)}
  <a class="lcard" href={url.href} target="_blank" rel="noopener noreferrer nofollow">
    {#if p.image}<img src={p.image} alt="" loading="lazy" />{/if}
    <span class="ltext">
      <span class="lsite">↗ {p.site}</span>
      {#if p.title}<span class="ltitle">{p.title}</span>{/if}
      {#if p.description}<span class="ldesc">{p.description}</span>{/if}
    </span>
  </a>
{:else}
  <a class="linkchip" href={url.href} target="_blank" rel="noopener noreferrer nofollow">
    ↗ {where}{#if !on}<span class="muted">· preview off</span>{:else if preview.error}<span class="muted" title={preview.error}>· previews need a home node</span>{/if}
  </a>
{/if}

<style>
  .lcard {
    display: flex; gap: 12px; align-items: stretch; border: 2px solid var(--ink); border-radius: 12px; overflow: hidden;
    background: var(--card); max-width: 560px; text-decoration: none; color: inherit;
  }
  .lcard:hover { background: var(--soft); }
  .lcard img { width: 112px; min-height: 84px; object-fit: cover; flex: 0 0 auto; border-right: 2px solid var(--ink); }
  .ltext { display: flex; flex-direction: column; gap: 4px; padding: 10px 12px 10px 0; min-width: 0; }
  .lcard:not(:has(img)) .ltext { padding-left: 12px; }
  .lsite { font-size: 13px; font-weight: 600; color: var(--mute); }
  .ltitle { font-weight: 700; overflow-wrap: anywhere; }
  .ldesc { font-size: 14px; color: var(--mute); overflow: hidden; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; }
  .muted { font-weight: 500; }
  @media (max-width: 480px) {
    .lcard img { width: 84px; }
  }
</style>
