<script lang="ts">
  // The client's privacy notice, readable at any time, locked or not: who made the
  // software, what the network shares, what stays on the device, and the Network & Safety
  // notice with when it was acknowledged here.
  import { onMount } from 'svelte';
  import { app, setTitle } from '../lib/app.svelte';
  import { NETWORK_SAFETY, PRIVACY_NOTICE } from '../lib/legal/texts';
  import { networkConsent } from '../lib/consent.svelte';
  import { router } from '../lib/router.svelte';
  import Logo from '../lib/ui/Logo.svelte';

  const device = $derived(app.platformName);
  onMount(() => setTitle('Privacy'));

  function readAgain() {
    networkConsent.reset();
    router.go('#/');
  }
</script>

<main class="privacy gutter" id="main" aria-labelledby="privacy-h">
  <div class="top">
    <Logo size={40} />
    <a class="textlink" href="#/">← Back to hoi</a>
  </div>
  <h1 id="privacy-h">{PRIVACY_NOTICE.title}</h1>
  <p class="help">Version {PRIVACY_NOTICE.version}, {PRIVACY_NOTICE.date}. This is the software's notice; each public node has its operator's own, shown before it serves you.</p>
  <div class="text">
    {#each PRIVACY_NOTICE.sections as s (s.title)}
      <section>
        <h2>{s.title}</h2>
        {#if s.text}<p>{s.text.replaceAll('{device}', device)}</p>{/if}
        {#if s.bullets}<ul>{#each s.bullets as b (b)}<li>{b}</li>{/each}</ul>{/if}
        {#if s.after}<p>{s.after}</p>{/if}
      </section>
    {/each}
  </div>
  <div class="frame safety">
    <h2>{NETWORK_SAFETY.title}</h2>
    {#if networkConsent.accepted}
      <p>Acknowledged on this {device} on {networkConsent.record?.date} (version {networkConsent.record?.version}).</p>
    {:else}
      <p>Not acknowledged on this {device} yet: it is shown before anything connects.</p>
    {/if}
    {#each NETWORK_SAFETY.sections as s (s.title)}
      <h3>{s.title}</h3>
      <p>{s.text}</p>
      {#if s.bullets}<ul>{#each s.bullets as b (b)}<li>{b}</li>{/each}</ul>{/if}
    {/each}
    {#if networkConsent.accepted}
      <button type="button" class="pill" onclick={readAgain}>Read and acknowledge it again</button>
    {/if}
  </div>
</main>

<style>
  .privacy { display: flex; flex-direction: column; align-items: flex-start; gap: 16px; padding-block: 32px 72px; max-width: 760px; margin-inline: auto; }
  .top { display: flex; align-items: center; gap: 16px; flex-wrap: wrap; width: 100%; justify-content: space-between; }
  h1 { font-size: clamp(30px, 6vw, 48px); line-height: 1.05; letter-spacing: -0.03em; }
  .text { display: flex; flex-direction: column; gap: 14px; }
  section h2 { font-size: 17px; letter-spacing: -0.02em; margin-bottom: 4px; }
  p, li { color: var(--mute); line-height: 1.5; font-weight: 500; overflow-wrap: anywhere; }
  ul { margin: 6px 0 6px 20px; display: flex; flex-direction: column; gap: 4px; }
  .safety { padding: 18px 20px; display: flex; flex-direction: column; gap: 8px; width: 100%; }
  .safety h2 { font-size: 22px; letter-spacing: -0.03em; }
  .safety h3 { font-size: 15px; margin-top: 8px; }
  .safety button { align-self: flex-start; margin-top: 8px; }
</style>
