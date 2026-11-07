<script lang="ts">
  // A name that is a domain: verified when the domain names this key, flagged when it
  // doesn't, so "nytimes.com" on a stranger's key reads as what it is.
  import type { Person } from '@hoi/core-api';
  let { person }: { person: Person } = $props();
</script>

{#if person.handle?.verified}
  <span class="tag tag-trust" title={`${person.handle.domain} says this key is theirs`}>✓ {person.handle.domain}</span>
{:else if person.handle && !person.handle.checking}
  <span class="tag tag-dashed" title={`Their name looks like a domain, but ${person.handle.domain} doesn't name this key`}>not their domain</span>
{/if}
