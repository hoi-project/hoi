<script lang="ts">
  // An operator's notice as people read it: the fixed titles and sentences with the operator's
  // words inside (hoi-core `NoticeView`, or the app's own `noticeSections` for a preview). The
  // same component shows it before a node link is opened, when a node first says hello, on the
  // kit page's live preview and in the setup app.
  import type { NoticeSection } from '@hoi/core-api';

  let { title, sections, footer, compact = false }: { title: string; sections: NoticeSection[]; footer: string; compact?: boolean } = $props();
</script>

<article class={['notice', compact && 'compact']} aria-label={title}>
  <h3>{title}</h3>
  {#each sections as s (s.title)}
    <p><b>{s.title}.</b> {s.text}</p>
  {/each}
  <p class="foot">{footer}</p>
</article>

<style>
  .notice { display: flex; flex-direction: column; gap: 10px; padding: 16px 18px; border: 2px solid var(--ink); border-radius: var(--radius-card); background: var(--card); }
  .notice.compact { padding: 12px 14px; gap: 8px; font-size: 14px; }
  h3 { font-size: 20px; letter-spacing: -0.03em; }
  .compact h3 { font-size: 17px; }
  p { line-height: 1.5; color: var(--ink); overflow-wrap: anywhere; white-space: pre-line; }
  b { font-weight: 800; }
  .foot { font-size: 13px; color: var(--mute); font-family: var(--mono); }
</style>
