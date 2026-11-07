<script lang="ts">
  // The new device's side of linking through the home node: wait until a device holding your key
  // has seen the request, then show the four digits it shows too (one set per device that
  // answered, usually one). The digits come from both devices' random numbers, so nobody can
  // make another request show the same ones.
  import { app } from '../app.svelte';

  let { home, oncancel }: { home: string; oncancel: () => void } = $props();

  const checks = $derived(app.linkChecks);
  const spaced = (c: string) => c.split('').join(' ');
</script>

<div class="waiting" role="status" aria-live="polite">
  {#if checks.length === 0}
    <p class="lead"><span class="dot" aria-hidden="true"></span>Asking your device that holds your key…</p>
    <p class="help">Open hoi there: the desktop app, or a browser connected to {home}. The question shows up on its own.</p>
  {:else}
    <p class="lead">
      {checks.length > 1
        ? 'Your devices with your key show a question. Say yes on the one showing one of these:'
        : 'Your other device shows a question with four digits. If they match these, say yes there:'}
    </p>
    <div class="sets">
      {#each checks as c (c)}
        <p class="digits num" aria-label={`Check digits ${spaced(c)}`}>{c.slice(0, 2)}<span> </span>{c.slice(2)}</p>
      {/each}
    </div>
    <p class="help">Keep this page open until it says linked. If the digits don't match, say no there: someone else is asking.</p>
  {/if}
  <button type="button" class="act" onclick={oncancel}>Stop asking</button>
</div>

<style>
  .waiting { display: flex; flex-direction: column; gap: 12px; align-items: flex-start; }
  .lead { font-size: 18px; line-height: 1.45; color: var(--mute); max-width: 34em; font-weight: 500; display: flex; align-items: center; gap: 10px; }
  .sets { display: flex; flex-wrap: wrap; gap: 12px; }
  .digits {
    font-size: clamp(40px, 11vw, 56px); font-weight: 900; letter-spacing: 0.08em; background: var(--sun); color: var(--tagink);
    border: 2px solid var(--ink); border-radius: 20px; padding: 8px 20px; line-height: 1.15;
  }
  .digits span { display: inline-block; width: 0.4em; }
  .dot { width: 12px; height: 12px; border-radius: 50%; background: var(--peach); border: 2px solid var(--ink); flex: 0 0 auto; animation: pulse 1.2s ease-in-out infinite; }
  @keyframes pulse { 50% { transform: scale(0.6); } }
  @media (prefers-reduced-motion: reduce) { .dot { animation: none; } }
</style>
