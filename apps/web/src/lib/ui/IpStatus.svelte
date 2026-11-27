<script lang="ts">
  // The status pill under the top bar (#152): what a checked IP tells the nodes you connect to.
  import type { Exposure } from '../ip-exposure';
  import { nudge, pulse } from '../motion';
  import StatusIcon from './StatusIcon.svelte';

  let { status, onclick }: { status: Exposure; onclick: () => void } = $props();
  const located = $derived(['same', 'other', 'no-zone'].includes(status.kind));
  const label = $derived(`${status.short}${status.note ? `, ${status.note}` : ''}${status.address && status.short !== status.address ? `, ${status.address}` : ''}. Details`);
  let pill: HTMLButtonElement | undefined = $state();
  let previous = '';
  $effect(() => {
    const kind = status.kind;
    if (previous && kind !== previous) nudge(pill ?? null);
    previous = kind;
  });
</script>

<button type="button" class="ipill" data-tone={status.tone} bind:this={pill} {onclick} aria-haspopup="dialog" aria-label={label}>
  {#if status.kind === 'checking' || status.kind === 'locating'}<i class="pdot" use:pulse={true}></i>{:else}<StatusIcon icon={status.icon} />{/if}
  {#if located && status.codes[0]}<span class="cc">{status.codes[0]}</span>{/if}
  <span class="txt">{status.short}{#if status.note}<span class="sub">{` · ${status.note}`}</span>{/if}</span>
  {#if status.address && status.short !== status.address}<span class="ip">{status.address}</span>{/if}
</button>

<style>
  .ipill {
    display: inline-flex; align-items: center; gap: var(--space-2); min-height: 36px; max-width: 100%; min-width: 0;
    padding: 4px 14px 4px 10px; border-radius: var(--radius-pill); font-weight: 800; font-size: var(--type-tag-size); line-height: 1.2;
    background: var(--card); color: var(--ink); border: 2px solid var(--ink); box-shadow: 3px 3px 0 var(--ink); text-align: start;
    transition: transform .15s ease, box-shadow .15s ease;
  }
  .ipill:hover { transform: translate(-1px, -1px); box-shadow: 4px 4px 0 var(--ink); }
  .ipill[data-tone='peach'] { background: var(--peach); color: var(--tagink); }
  .ipill[data-tone='sky'] { background: var(--sky); color: var(--tagink); }
  .ipill[data-tone='sun'] { background: var(--sun); color: var(--tagink); }
  .ipill[data-tone='lilac'] { background: var(--lilac); color: var(--tagink); }
  .ipill[data-tone='idle'] { background: var(--soft); border-color: transparent; box-shadow: inset 0 0 0 2px var(--line); }
  .ipill[data-tone='dashed'] { background: transparent; border: 2px dashed var(--line); box-shadow: none; }
  .ipill[data-tone='idle']:hover, .ipill[data-tone='dashed']:hover { transform: translateY(-1px); box-shadow: inset 0 0 0 2px var(--line); }
  .ipill[data-tone='dashed']:hover { box-shadow: none; }
  .pdot { width: 8px; height: 8px; margin: 4px; border-radius: 50%; background: currentColor; flex: 0 0 8px; }
  .txt { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { font-weight: 600; }
  .cc, .ip { font-family: var(--mono); font-weight: 500; }
  .cc { font-size: 11px; padding: 0 5px; border: 1.5px solid currentColor; border-radius: 6px; flex: 0 0 auto; }
  .ip { font-size: var(--type-mono-size); opacity: .85; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  @media (max-width: 640px), (pointer: coarse) { .ipill { min-height: 44px; } }
  @media (max-width: 420px) { .ip, .sub { display: none; } }
  @media (prefers-reduced-motion: reduce) { .ipill { transition: none; } .ipill:hover { transform: none; } }
  :global(html[data-motion='reduced']) .ipill { transition: none; }
  :global(html[data-motion='reduced']) .ipill:hover { transform: none; }
</style>
