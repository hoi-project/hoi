<script lang="ts">
  // You → Your devices: every device that signs as you, named so you can tell them apart (names
  // stay on this device), and revoked from the one that holds your root key.
  import type { SignAsk } from '@hoi/core-api';
  import { onMount } from 'svelte';
  import { app, attempt } from '../app.svelte';

  let { onlink }: { onlink: () => void } = $props();
  const me = $derived(app.me!);

  let editing = $state<string | null>(null);
  let draft = $state('');
  let revokeFor = $state<string | null>(null);
  /** What the browsers this computer signs for asked to sign and waits for a yes. */
  let asks = $state<SignAsk[]>([]);
  const signsFor = $derived(me.devices.some((d) => d.signedHere));

  async function loadAsks() {
    if (signsFor) asks = await app.core.signAsks().catch(() => []);
  }
  onMount(() => void loadAsks());
  $effect(() => {
    if (app.signAskRev) void loadAsks();
  });

  async function answer(id: string, yes: boolean) {
    await attempt(() => app.core.signAnswer(id, yes), yes ? 'Signed.' : 'Refused. The browser is told.');
    await loadAsks();
  }

  function rename(key: string, label: string) {
    revokeFor = null;
    draft = label;
    editing = key;
  }

  async function save(e: SubmitEvent) {
    e.preventDefault();
    if (!editing) return;
    const m = await attempt(() => app.core.setDeviceLabel(editing!, draft), (v) => `Named “${v.devices.find((d) => d.key === editing)?.label ?? draft.trim()}” on this ${app.platformName}.`);
    if (m) {
      app.me = m;
      editing = null;
    }
  }

  async function revoke(key: string, label: string) {
    const m = await attempt(() => app.core.revokeDevice(key), `${label} revoked. It can't sign as you anymore.`);
    if (m) {
      app.me = m;
      revokeFor = null;
    }
  }
</script>

<div class="set top">
  <span class="l">Your devices</span>
  <span class="h">
    Each has its own key, approved by your root key. Name them to tell them apart (names stay on this {app.platformName}). Revoke one
    you lost; you stay you.{me.hasRoot ? '' : ' Revoking needs the device with your root key.'}
  </span>
  <div class="wide devs">
    <ul aria-label="Devices that sign as you">
      {#each me.devices as d (d.key)}
        <li class={['dev', d.revoked && 'revoked', d.thisDevice && 'here']}>
          {#if editing === d.key}
            <form class="renamef" onsubmit={save}>
              <label class="sr-only" for={`dev-${d.short}`}>Name for this device</label>
              <input id={`dev-${d.short}`} class="input" bind:value={draft} maxlength="32" autocomplete="off" />
              <button class="pill pill-ink pill-sm" type="submit" disabled={!draft.trim()}>Save</button>
              <button class="pill pill-sm" type="button" onclick={() => (editing = null)}>Cancel</button>
            </form>
          {:else}
            <span class="who">
              <b class="name">{d.label}</b>
              {#if d.thisDevice}<span class="tag tag-mint">● here</span>{/if}
              {#if d.thisDevice && me.hasRoot}<span class="tag tag-sun">root key</span>{/if}
              {#if d.revoked}<span class="tag tag-soft">revoked</span>{/if}
              {#if d.thisDevice && me.remote}<span class="tag tag-lilac">your computer signs for it</span>{/if}
              {#if d.signedHere && !d.revoked}<span class="tag tag-lilac">signed by this {app.platformName}</span>{/if}
              <span class="mono" title={d.key}>{d.short}</span>
            </span>
            <span class="acts">
              {#if !d.revoked}<button type="button" class="lnk lnk-ink" onclick={() => rename(d.key, d.label)}>Rename</button>{/if}
              {#if !d.thisDevice && !d.revoked && me.hasRoot}
                {#if revokeFor === d.key}
                  <button type="button" class="lnk lnk-danger" onclick={() => revoke(d.key, d.label)}>Yes, revoke</button>
                  <button type="button" class="lnk lnk-ink" onclick={() => (revokeFor = null)}>No</button>
                {:else}
                  <button type="button" class="lnk lnk-danger" onclick={() => (revokeFor = d.key)}>Revoke</button>
                {/if}
              {/if}
            </span>
          {/if}
        </li>
      {/each}
    </ul>
    {#if asks.length}
      <div class="asks" role="status">
        <p class="h">Your browsers ask this {app.platformName} to sign. Moderation, rules and owners wait for your yes.</p>
        {#each asks as a (a.id)}
          <div class="ask">
            <span><b>{a.device}</b>: a {a.kind === 'ledger' ? 'moderation step' : a.kind}{a.community ? ` in c/${app.communities.find((c) => c.id === a.community)?.name ?? a.community.slice(0, 8)}` : ''}</span>
            <button type="button" class="pill pill-sm pill-ink" onclick={() => answer(a.id, true)}>Sign it</button>
            <button type="button" class="pill pill-sm" onclick={() => answer(a.id, false)}>Say no</button>
          </div>
        {/each}
      </div>
    {/if}
    <button type="button" class="lnk lnk-ink solid" onclick={onlink}>+ Link a device</button>
  </div>
</div>

<style>
  .asks { margin-top: 12px; display: flex; flex-direction: column; gap: 8px; }
  .ask { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; font-size: 14px; }
  .devs { display: flex; flex-direction: column; gap: 10px; align-items: flex-start; }
  ul { list-style: none; margin: 0; padding: 0; width: 100%; display: flex; flex-direction: column; border: 2px solid var(--line); border-radius: var(--radius-input); }
  .dev { display: flex; align-items: center; justify-content: space-between; gap: 8px 14px; flex-wrap: wrap; padding: 10px 14px; min-height: 52px; }
  .dev + .dev { border-top: 1px solid var(--line); }
  .dev.here { background: color-mix(in srgb, var(--mint) 22%, transparent); }
  /* muted grey is too faint on the tint in the dark theme */
  .dev.here .mono { color: var(--ink); }
  .who { display: inline-flex; align-items: center; gap: 8px; flex-wrap: wrap; min-width: 0; }
  .name { font-weight: 800; overflow-wrap: anywhere; }
  .revoked .name { color: var(--mute); text-decoration: line-through; }
  .acts { display: inline-flex; gap: 6px; flex-wrap: wrap; }
  .renamef { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; width: 100%; }
  .renamef .input { flex: 1 1 200px; max-width: 320px; min-height: 40px; padding: 6px 12px; }
  .solid { background: var(--ink); color: var(--onink); }
</style>
