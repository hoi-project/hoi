<script lang="ts">
  import type { CommunityRef } from '@hoi/core-api';
  import { to } from '../router';
  let { c, link = true, big = false }: { c: CommunityRef; link?: boolean; big?: boolean } = $props();
</script>

{#if link}
  <a href={to.community(c.id)} class={['tag', `tag-${c.color}`, big && 'big']}>c/{c.name}</a>
{:else}
  <span class={['tag', `tag-${c.color}`, big && 'big']}>c/{c.name}</span>
{/if}
