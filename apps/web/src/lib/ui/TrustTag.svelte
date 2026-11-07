<script lang="ts">
  import type { Person } from '@hoi/core-api';
  // Trust is always cobalt and always carries the ✓, so colour is never the only signal.
  let { person, compact = false }: { person: Person; compact?: boolean } = $props();
</script>

{#if person.trust === 'me'}
  <span class="tag tag-sun">yours</span>
{:else if person.trust === 'friend'}
  <span class="tag tag-trust" title="You trust this key">✓{#if compact}<span class="sr-only"> friend</span>{:else}{' '}friend{/if}</span>
{:else if person.trust === 'fof'}
  <span class="tag tag-soft" title={person.trustedBy?.length ? `Trusted by your friend${person.trustedBy.length > 1 ? 's' : ''} ${person.trustedBy.join(', ')}` : 'Trusted by a friend of yours'}>{compact ? 'fof' : person.trustedBy?.length ? `friend of ${person.trustedBy[0]}` : 'friend of a friend'}</span>
{:else if person.trust === 'newcomer'}
  <span class="tag tag-sun">{compact ? 'new' : 'new here · stamped'}</span>
{/if}
