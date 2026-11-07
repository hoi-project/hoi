<script lang="ts">
  import type { CommunityColor } from '@hoi/core-api';
  import { encode, renderSVG } from 'uqr';
  // Dark modules on a light pastel tile in both themes, with the quiet zone the standard asks
  // for, so any phone camera can read it. Too much data for a code a camera can read from a
  // screen says so instead of drawing nothing.
  let { value, label, color = 'sun' }: { value: string; label: string; color?: CommunityColor } = $props();

  /** Past this, modules get too small to read off a screen: say so, show the text instead. */
  const MAX_VERSION = 30;
  /** Dense codes are drawn bigger, so each module stays a few pixels wide. */
  const DENSE = 18;

  const code = $derived.by(() => {
    for (const ecc of ['M', 'L'] as const) {
      try {
        const version = encode(value, { ecc }).version;
        if (version > MAX_VERSION) continue;
        return { version, svg: renderSVG(value, { ecc, border: 4, pixelSize: 4, blackColor: 'currentColor', whiteColor: 'transparent' }) };
      } catch {
        // too long for this level
      }
    }
    return null;
  });
</script>

{#if code}
  <div class={['qr', code.version > DENSE && 'dense', `bg-${color}`]} role="img" aria-label={label}>
    <!-- uqr output is generated locally from the string above: plain rects, no script. -->
    {@html code.svg}
  </div>
{:else}
  <div class={['qr', 'toolong', `bg-${color}`]} role="note">This code is too long for a camera. Copy it instead.</div>
{/if}

<style>
  .qr { color: var(--tagink); padding: 6px; border: 2px solid var(--ink); border-radius: 16px; width: min(240px, 100%); flex: 0 0 auto; background-clip: padding-box; }
  .qr :global(svg) { width: 100%; height: auto; display: block; shape-rendering: crispEdges; }
  .qr.dense { width: min(320px, 100%) !important; }
  .toolong { aspect-ratio: 1; display: flex; align-items: center; justify-content: center; text-align: center; padding: 18px; font-weight: 700; font-size: 14px; line-height: 1.4; }
</style>
