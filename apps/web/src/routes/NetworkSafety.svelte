<script lang="ts">
  // The fixed notice every connection calls for: shown before the app starts anything
  // that connects, and again when its version changes. Both boxes, then Continue.
  import Logo from '../lib/ui/Logo.svelte';
  import { NETWORK_SAFETY } from '../lib/legal/texts';
  import { networkConsent } from '../lib/consent.svelte';
  import { to } from '../lib/router';

  let ip = $state(false);
  let vpn = $state(false);
  const ready = $derived(ip && vpn);

  function submit(event: SubmitEvent) {
    event.preventDefault();
    if (ready) networkConsent.accept();
  }
</script>

<main class="safety gutter" id="main" aria-labelledby="safety-h">
  <div class="top">
    <Logo size={48} />
    <span class="sticker">before anything connects</span>
  </div>
  <h1 id="safety-h">{NETWORK_SAFETY.title}</h1>
  {#if networkConsent.changed}
    <p class="callout" role="status">This notice changed since you last read it (you acknowledged version {networkConsent.record?.version} on {networkConsent.record?.date}). Please read it again.</p>
  {/if}
  <div class="text">
    {#each NETWORK_SAFETY.sections as s (s.title)}
      <section>
        <h2>{s.title}</h2>
        <p>{s.text}</p>
        {#if s.bullets}<ul>{#each s.bullets as b (b)}<li>{b}</li>{/each}</ul>{/if}
      </section>
    {/each}
  </div>
  <form onsubmit={submit}>
    <label class="ack"><input id="safety-ack-ip" type="checkbox" bind:checked={ip} /> <span>{NETWORK_SAFETY.checks.ip}</span></label>
    <label class="ack"><input id="safety-ack-vpn" type="checkbox" bind:checked={vpn} /> <span>{NETWORK_SAFETY.checks.vpn}</span></label>
    <button type="submit" class="pill pill-ink pill-lg" disabled={!ready}>{NETWORK_SAFETY.cta}</button>
    <p class="help">Nothing has connected yet. Read the <a class="textlink" href={to.privacy()}>privacy notice</a> first if you like. Network &amp; Safety version {NETWORK_SAFETY.version}.</p>
  </form>
</main>

<style>
  .safety { min-height: 100dvh; display: flex; flex-direction: column; align-items: flex-start; gap: 18px; padding-block: 40px 64px; max-width: 720px; margin-inline: auto; }
  .top { display: flex; align-items: center; gap: 14px; flex-wrap: wrap; }
  h1 { font-size: clamp(36px, 7vw, 56px); line-height: 1; }
  .text { display: flex; flex-direction: column; gap: 16px; }
  section h2 { font-size: 18px; letter-spacing: -0.02em; margin-bottom: 4px; }
  section p, section li { color: var(--mute); line-height: 1.5; font-weight: 500; }
  section ul { margin: 6px 0 0 20px; display: flex; flex-direction: column; gap: 4px; }
  form { display: flex; flex-direction: column; gap: 12px; width: 100%; padding-top: 6px; border-top: 2px dotted var(--line); }
  .ack { display: flex; gap: 10px; align-items: flex-start; font-weight: 600; line-height: 1.45; cursor: pointer; }
  .ack input { width: 20px; height: 20px; margin-top: 2px; flex: none; accent-color: var(--ink); }
  button { align-self: flex-start; }
  .help { font-size: 13px; }
</style>
