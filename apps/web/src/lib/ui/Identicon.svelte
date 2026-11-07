<script lang="ts">
  // A key's picture (lib/sayhoi/identicon.ts). Decorative: whatever shows it also says whose key it is.
  import { picture } from '../sayhoi/identicon';
  import { hexToBytes } from '../util';
  import type { AvatarColor } from '@hoi/core-api';

  let { keyHex = '', bytes, size = 120, dim = false, color }: { keyHex?: string; bytes?: Uint8Array; size?: number; dim?: boolean; color?: AvatarColor } = $props();
  const pic = $derived(picture(bytes ?? hexToBytes(keyHex)));
</script>

<svg class={['identicon', dim && 'dim']} width={size} height={size} viewBox="0 0 100 100" aria-hidden="true">
  <circle cx="50" cy="50" r="47" class="ground bg-{color ?? pic.ground}" />
  {#each pic.links as [a, b], i (i)}
    <line x1={a.cx} y1={a.cy} x2={b.cx} y2={b.cy} />
  {/each}
  {#each pic.dots as d, i (i)}
    <circle cx={d.cx} cy={d.cy} r={d.r} class={['dot', d.coral && 'coral']} />
  {/each}
</svg>

<style>
  .identicon { display: block; flex: 0 0 auto; overflow: visible; }
  .ground { stroke: var(--ink); stroke-width: 3; }
  .bg-mint { fill: var(--mint); } .bg-sky { fill: var(--sky); } .bg-lilac { fill: var(--lilac); }
  .bg-peach { fill: var(--peach); } .bg-sun { fill: var(--sun); }
  line { stroke: #1a1a1a; stroke-width: 2.5; stroke-linecap: round; }
  .dot { fill: #1a1a1a; }
  .dot.coral { fill: var(--coral); stroke: #1a1a1a; stroke-width: 2; }
  .dim { filter: grayscale(1); opacity: 0.55; }
</style>
