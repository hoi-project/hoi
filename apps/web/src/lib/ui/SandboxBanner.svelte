<script lang="ts">
  import { onMount } from 'svelte';
  import { getSandboxContext, trackSandboxLinks, type SandboxContext } from '../sandbox';
  import { toast, toastError } from '../toast.svelte';

  let context = $state<SandboxContext | null>(null);
  let dashboard = $state('');
  onMount(() => {
    let dispose: (() => void) | undefined;
    let closed = false;
    void getSandboxContext().then((value) => {
      if (!value || closed) return;
      context = value;
      const url = new URL(location.origin);
      url.port = '7070';
      dashboard = url.href;
      dispose = trackSandboxLinks(value, () => toast('Test link opened. The dashboard recorded this attempt.'), toastError);
    }).catch(() => { /* The boot screen presents setup failures. */ });
    return () => { closed = true; dispose?.(); };
  });
</script>

{#if context}
  <aside class="sandbox-banner" aria-label="Sandbox identity">
    <span><strong>Sandbox · {context.role}</strong> · {context.role === 'watcher' ? 'Watch with a separate test key.' : context.role === 'owner' ? 'Linked to the real community owner.' : 'A separate test key, ready to join in.'}</span>
    <a href={dashboard} target="_blank" rel="noopener noreferrer">Open dashboard ↗</a>
  </aside>
{/if}

<style>
  .sandbox-banner { display: flex; flex-wrap: wrap; justify-content: space-between; gap: 8px 20px; padding: 12px max(16px, env(safe-area-inset-left, 0px)); background: var(--sun); color: #1A1A1A; border-bottom: 2px solid #1A1A1A; font-size: 14px; }
  a { font-weight: 800; text-decoration: underline; text-underline-offset: 3px; }
</style>
