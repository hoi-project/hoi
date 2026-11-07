<script lang="ts">
  import { PUBLICATION } from '../preview';
  import { to } from '../router';
  import { isDesktopShell } from '../host';
  import ExternalLink from './ExternalLink.svelte';

  const site = PUBLICATION.site || (isDesktopShell() ? 'https://hoi.sh/' : new URL('/', window.location.href).href);
</script>

<footer class="footer gutter">
  <div class="brand"><b>hoi</b><span>A forum with nobody in the middle.</span></div>
  <nav aria-label="Footer">
    <a href={to.home()}>Home</a>
    <a href={to.communities()}>Communities</a>
    <a href={to.registry()}>Public registry</a>
    <a href={to.privacy()}>Privacy</a>
    <ExternalLink href={site}>About hoi ↗</ExternalLink>
    {#if PUBLICATION.repository}<ExternalLink href={PUBLICATION.repository}>Source</ExternalLink>{/if}
  </nav>
</footer>

<style>
  .footer { max-width: 1240px; margin: 0 auto; padding-block: 24px 32px; border-top: 2px dotted var(--line); display: flex; justify-content: space-between; align-items: start; flex-wrap: wrap; gap: 18px 32px; color: var(--mute); font-size: 13px; }
  .brand { display: flex; flex-wrap: wrap; align-items: center; gap: 12px; }
  .brand b { font-size: 24px; color: var(--ink); letter-spacing: -.04em; }
  nav { display: flex; flex-wrap: wrap; gap: 4px 20px; }
  nav :global(a) { display: inline-flex; align-items: center; min-height: 36px; color: var(--ink); font-weight: 700; }
  nav :global(a:hover) { text-decoration: underline; text-underline-offset: 3px; }
  @media (max-width: 640px) { .footer { padding-bottom: calc(92px + env(safe-area-inset-bottom, 0px)); } nav :global(a) { min-height: 44px; } }
</style>
