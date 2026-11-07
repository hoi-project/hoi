<script lang="ts">
  // You → Connection details: for troubleshooting. Three plain sections: when this device last
  // caught up, who it is connected to and how, and the addresses other nodes dial to reach it.
  // Everything here stays on this device.
  import { app } from '../app.svelte';
  import CopyField from './CopyField.svelte';

  const net = $derived(app.net);
  const who = (role: string) => (role === 'home' ? 'Your home node' : role === 'client' ? 'One of your devices' : 'Another node');
  const lock = (e: string) => (e.startsWith('encrypted') ? 'tag-mint' : e.startsWith('stays') ? 'tag-sky' : 'tag-peach');
</script>

<details class="diagnostics">
  <summary>
    <b>Connection details</b>
    <span>For troubleshooting: last catch-up, who's connected and how, and this {app.platformName}'s addresses</span>
  </summary>
  <div class="body">
    <section aria-labelledby="cd-sync">
      <div class="head"><h3 id="cd-sync">Last catch-up</h3></div>
      <p class="help">When this {app.platformName} last swapped records with another device. Not proof that every post or picture arrived.</p>
      <p class="val">{net?.lastSync ? new Date(net.lastSync * 1000).toLocaleString() : 'Not yet'}</p>
    </section>

    <section aria-labelledby="cd-peers">
      <div class="head"><h3 id="cd-peers">Connected now</h3><span class="tag tag-soft">{net?.connections?.length ?? 0}</span></div>
      <p class="help">A connection means another device answered. Posts made here stay here until one does.</p>
      {#if net?.connections?.length}
        <ul class="peers">
          {#each net.connections as c (c.peer)}
            <li>
              <span class="who"><b>{who(c.role)}</b><span class={['tag', c.connected ? 'tag-mint' : 'tag-soft']}>{c.connected ? '● connected' : 'connecting…'}</span></span>
              <dl>
                <div><dt>Route</dt><dd>{c.route === 'unknown' ? 'not reported' : c.route}</dd></div>
                <div><dt>Protection</dt><dd><span class={['tag', lock(c.encryption)]}>{c.encryption}</span></dd></div>
                <div><dt>ID</dt><dd class="mono" title={c.peer}>{c.peer}</dd></div>
              </dl>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="val">Nobody yet.</p>
      {/if}
    </section>

    {#if net?.transport || net?.rtc.length}
      <section aria-labelledby="cd-addr">
        <div class="head"><h3 id="cd-addr">How others reach this {app.platformName}</h3></div>
        <p class="help">What other nodes dial. Invites carry these hints.</p>
        {#if net?.transport}
          <CopyField label="Endpoint ID (computers)" value={net.transport.endpointId} what="Endpoint copied" />
          <p class="meta"><span><b>{net.transport.peers.length}</b> address hints in invites</span><span><b>{net.transport.relays.length}</b> relays</span></p>
          {#if net.transport.relays.length}
            <h4>Relays</h4>
            <ul class="addresses">{#each net.transport.relays as relay (relay)}<li class="mono">{relay}</li>{/each}</ul>
          {/if}
          {#if net.transport.error}<p class="callout" role="status">{net.transport.error}</p>{/if}
        {/if}
        {#if net?.rtc.length}
          <h4>Browsers and phones</h4>
          <ul class="addresses hidden" title="Point at one to see it">{#each net.rtc as route (route)}<li class="mono">{route}</li>{/each}</ul>
        {/if}
      </section>
    {/if}
    <p class="help foot">These details stay on this {app.platformName}.</p>
  </div>
</details>

<style>
  .diagnostics { margin-top: 18px; border: 2px solid var(--ink); border-radius: var(--radius-card); padding: 4px 18px; background: var(--card); }
  summary { cursor: pointer; padding: 12px 0; min-height: 44px; display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px 10px; }
  summary b { font-weight: 800; font-size: 17px; }
  summary span { color: var(--mute); font-size: 13px; font-weight: 500; }
  .body { padding-bottom: 14px; display: flex; flex-direction: column; }
  section { border-top: 2px dotted var(--line); padding-block: 14px; display: flex; flex-direction: column; gap: 8px; min-width: 0; }
  .head { display: flex; align-items: center; gap: 8px; }
  h3 { font-size: 16px; font-weight: 800; letter-spacing: -0.01em; }
  h4 { font-size: 13px; font-weight: 800; margin: 4px 0 0; }
  .val { font-weight: 700; font-size: 15px; }
  .meta { gap: 14px; }
  .meta b { color: var(--ink); }
  ul { list-style: none; margin: 0; padding: 0; }
  .peers li { padding: 10px 12px; border-radius: 12px; background: var(--soft); }
  .peers li + li { margin-top: 8px; }
  .who { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; font-size: 14px; }
  dl { margin: 8px 0 0; display: flex; flex-direction: column; gap: 6px; }
  dl div { display: grid; grid-template-columns: 90px minmax(0, 1fr); gap: 12px; font-size: 13px; align-items: center; }
  dt { color: var(--mute); font-weight: 600; }
  dd { margin: 0; overflow-wrap: anywhere; }
  dd .tag { white-space: normal; }
  .addresses li { overflow-wrap: anywhere; font-size: 12px; padding: 2px 0; }
  /* Like Your addresses: an IP shows only while pointed at. */
  .hidden li { filter: blur(4px); transition: filter .2s ease; }
  .hidden li:hover { filter: none; }
  @media (prefers-reduced-motion: reduce) { .hidden li { transition: none; } }
  .foot { border-top: 2px dotted var(--line); padding-top: 12px; }
  @media (max-width: 640px) { dl div { grid-template-columns: 76px minmax(0, 1fr); } }
</style>
