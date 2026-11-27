<script lang="ts">
  import type { StatusIcon } from '../ip-exposure';
  // Each status has its own shape, so colour is never the only signal.
  let { icon, size = 16 }: { icon: StatusIcon; size?: number } = $props();
</script>

<svg viewBox="0 0 16 16" width={size} height={size} aria-hidden="true" focusable="false">
  {#if icon === 'ring'}
    <circle cx="8" cy="8" r="5.5" fill="none" stroke="currentColor" stroke-width="2" stroke-dasharray="3 2.6" />
  {:else if icon === 'dot'}
    <circle cx="8" cy="8" r="4" fill="currentColor" />
  {:else if icon === 'pin'}
    <path d="M8 15s5-4.6 5-8.6A5 5 0 0 0 3 6.4C3 10.4 8 15 8 15z" fill="currentColor" />
    <circle cx="8" cy="6.4" r="1.9" class="hole" />
  {:else if icon === 'away'}
    <path d="M6.5 14.5s4-3.7 4-6.9a4 4 0 0 0-8 0c0 3.2 4 6.9 4 6.9z" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round" />
    <path d="M10.5 2.5h3.5V6M14 2.5 10.6 5.9" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
  {:else if icon === 'fork'}
    <path d="M8 15V9.5M8 9.5 3.5 4M8 9.5l4.5-5.5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
    <circle cx="3.2" cy="3.2" r="2" fill="currentColor" /><circle cx="12.8" cy="3.2" r="2" fill="currentColor" />
  {:else if icon === 'home'}
    <path d="M2.5 7.5 8 2.5l5.5 5V14h-11z" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round" />
    <rect x="6.5" y="9.5" width="3" height="4.5" fill="currentColor" />
  {:else}
    <circle cx="8" cy="8" r="5.5" fill="none" stroke="currentColor" stroke-width="2" />
    <path d="M4.2 11.8l7.6-7.6" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
  {/if}
</svg>

<style>
  svg { flex: 0 0 auto; display: block; }
  .hole { fill: var(--card); }
</style>
