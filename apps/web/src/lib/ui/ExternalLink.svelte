<script lang="ts">
  import type { Snippet } from 'svelte';
  import { isDesktopShell, openInBrowser } from '../host';
  import { toastError } from '../toast.svelte';

  let { href, children, class: className = '' }: { href: string; children: Snippet; class?: string } = $props();
  function open(event: MouseEvent) {
    if (!isDesktopShell()) return;
    event.preventDefault();
    void openInBrowser(href).catch(toastError);
  }
</script>

<a {href} class={className} target="_blank" rel="noopener noreferrer" onclick={open}>{@render children()}</a>
