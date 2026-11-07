<script lang="ts">
  import { app } from '../app.svelte';
  import { router } from '../router.svelte';
  import { startPost } from '../compose';
  // Phones (MobileFeed artboard): four places and a coral + for posting.
  const r = $derived(router.route);
  const items = $derived([
    { href: '#/', label: 'Home', on: r.name === 'home' },
    { href: '#/communities', label: 'Communities', on: r.name === 'communities' || r.name === 'registry' || r.name === 'community' || r.name === 'ledger' || r.name === 'new' },
    { href: '#/saved', label: 'Saved', on: r.name === 'saved' },
    { href: '#/you', label: 'You', on: r.name === 'you' },
  ]);
  // The coral + belongs to feeds (MobileFeed); forms and the post page keep their own buttons.
  const showFab = $derived(
    r.name === 'home' || r.name === 'communities' || r.name === 'saved' || r.name === 'search' || (r.name === 'community' && r.tab === 'posts'),
  );
  // A sheet over the screen, the title focused within the same tap (lib/compose.ts).
  const composing = $derived(app.composer.open);
</script>

{#if showFab && !composing}<button type="button" class="fab" aria-label="New post" onclick={() => startPost()}>+</button>{/if}
<nav class="bottom" aria-label="Main">
  {#each items as it (it.href)}
    <a href={it.href} class:on={it.on} aria-current={it.on ? 'page' : undefined}>{it.label}</a>
  {/each}
</nav>

<style>
  .bottom, .fab { display: none; }
  @media (max-width: 640px) {
    .bottom {
      display: flex; position: fixed; left: 0; right: 0; bottom: 0; z-index: 8; background: var(--card); border-top: 2px solid var(--ink);
      padding: 4px max(12px, env(safe-area-inset-right, 0px)) max(8px, env(safe-area-inset-bottom, 0px)) max(12px, env(safe-area-inset-left, 0px));
    }
    .bottom a { flex: 1; min-height: 48px; display: flex; align-items: center; justify-content: center; font-weight: 700; font-size: 13px; color: var(--mute); border-radius: 12px; }
    .bottom a.on { color: var(--ink); }
    .bottom a.on { text-decoration: underline; text-decoration-thickness: 3px; text-underline-offset: 6px; text-decoration-color: var(--sun); }
    .fab {
      display: inline-flex; position: fixed; right: max(20px, env(safe-area-inset-right, 0px)); bottom: calc(76px + env(safe-area-inset-bottom, 0px)); z-index: 8;
      width: 56px; height: 56px; border-radius: 50%; background: var(--coral); border: 2px solid var(--ink); color: var(--tagink);
      align-items: center; justify-content: center; font-size: 28px; font-weight: 700; box-shadow: 4px 4px 0 var(--ink);
    }
  }
</style>
