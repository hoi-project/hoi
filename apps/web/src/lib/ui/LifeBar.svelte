<script lang="ts">
  import { fadesIn, lifeLeft } from '@hoi/core-api';
  let { createdAt, expiresAt }: { createdAt: number; expiresAt: number } = $props();
  const left = $derived(lifeLeft(createdAt, expiresAt));
  const label = $derived(fadesIn(expiresAt));
</script>

<!-- Width = remaining life. Everything fades one day. -->
<span class="life" role="img" aria-label={label} title={label}><i style:width="{Math.max(2, Math.round(left * 100))}%"></i></span>
