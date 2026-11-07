<script lang="ts">
  import { bob } from '../motion';
  let { size = 26 }: { size?: number } = $props();
</script>

<!-- The mark: three dots (coral, cobalt, sun) joined by ink lines. The dots bob gently. -->
<svg width={size} height={size} viewBox="0 0 30 30" aria-hidden="true" use:bob>
  <line x1="8" y1="10" x2="22" y2="9" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
  <line x1="8" y1="10" x2="15" y2="22" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
  <line x1="22" y1="9" x2="15" y2="22" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
  <circle class="ld coral" cx="8" cy="10" r="5" />
  <circle class="ld cobalt" cx="22" cy="9" r="5" />
  <circle class="ld sun" cx="15" cy="22" r="5" stroke="currentColor" stroke-width="2" />
</svg>

<style>
  .coral { fill: var(--coral); }
  .cobalt { fill: var(--cobalt); }
  .sun { fill: var(--sun); }
</style>
