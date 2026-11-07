<script lang="ts">
  import type { CommunityView } from '@hoi/core-api';
  import { app } from '../app.svelte';
  import { REGISTRY_URL, RegistryConflict, lookupName, publicNameValid, publishName, canJoinPublic, type RegistryEntry } from '../registry';
  import { to } from '../router';
  import { errorText, toast } from '../toast.svelte';
  import CopyField from './CopyField.svelte';

  let { community }: { community: CommunityView } = $props();
  let entry = $state<RegistryEntry | null>(null);
  let loading = $state(false);
  let registering = $state(false);
  let checked = $state(false);
  let error = $state('');
  let retry = $state(0);
  const name = $derived(community.name.trim().toLowerCase());
  const valid = $derived(/^[\x00-\x7f]+$/.test(community.name) && publicNameValid(name));
  const registered = $derived(entry?.community === community.id);
  const conflict = $derived(!!entry && !registered);

  $effect(() => {
    const requestedName = name;
    void community.id;
    void retry;
    entry = null;
    checked = false;
    error = '';
    if (!REGISTRY_URL || !valid) return;
    let active = true;
    loading = true;
    void lookupName(requestedName).then((value) => {
      if (!active) return;
      entry = value;
      checked = true;
    }).catch((e) => {
      if (active) error = errorText(e);
    }).finally(() => {
      if (active) loading = false;
    });
    return () => { active = false; };
  });

  async function register() {
    if (registering || loading) return;
    const id = community.id;
    registering = true;
    checked = false;
    error = '';
    try {
      const proof = await app.core.communityPublication(id);
      const value = await publishName(proof);
      if (community.id !== id) return;
      entry = value;
      checked = true;
      toast(`c/${value.name} is registered to this community.`);
    } catch (e) {
      if (community.id !== id) return;
      if (e instanceof RegistryConflict) {
        entry = e.entry;
        checked = true;
      }
      error = errorText(e);
    } finally {
      registering = false;
    }
  }
</script>

<div class="public-name" aria-label="Community identity">
  <div role="status">
    {#if !REGISTRY_URL}
      <span class="tag tag-dashed">Local name · public reservation unavailable</span>
      <p class="help">Share this community's invite to join the same community. Making another with the same name creates a separate place.</p>
    {:else if !valid}
      <span class="tag tag-dashed">Local name · cannot be registered</span>
      <p class="help">Public names use 1–32 lowercase letters, digits, dots, dashes or underscores, starting with a letter or digit. This community keeps its existing name and ID.</p>
    {:else if loading}
      <span class="tag tag-soft">Checking public name…</span>
    {:else if registered}
      <span class="tag tag-mint">Registered public name · c/{name}</span>
      {#if entry && canJoinPublic(entry)}
        <p class="help">People can find this community by name. <a href={to.public(community.id, name, entry.protocolVersion!)}>Public join page</a></p>
      {:else}<p class="help">This name remains reserved to this key but is not listed in this app's compatible index. Use existing invites and compatible software; index updates do not migrate community data.</p>{/if}
    {:else if conflict}
      <span class="tag tag-peach">Name registered to another community</span>
      <p class="help">This community still has its own ID. Matching names do not merge posts or membership. Join the registered community, or keep sharing this one's invite.</p>
    {:else}
      <span class="tag tag-dashed">Local name · {checked ? 'not registered' : 'public reservation unconfirmed'}</span>
      <p class="help">{checked ? 'This community is saved locally. Its public name is only reserved after registration succeeds.' : 'The public registry has not confirmed this name. Your local community is still saved.'}</p>
    {/if}
    {#if error}<p class="help problem">{error}</p>{/if}
  </div>
  {#if REGISTRY_URL && valid && !registered}
    <div class="btnrow">
      {#if community.role === 'owner' && community.limits.admission === 'open' && !conflict}
        <button type="button" class="pill pill-sm" disabled={registering || loading} onclick={register}>{registering ? 'Registering…' : 'Register public name'}</button>
      {/if}
      <button type="button" class="act" disabled={registering || loading} onclick={() => retry++}>{loading ? 'Checking…' : 'Check registry again'}</button>
    </div>
  {/if}
  <details>
    <summary>Community ID and public name</summary>
    <CopyField label="This community's full ID" value={community.id} what="Community ID copied" />
    {#if conflict && entry}<CopyField label="Registered community's ID" value={entry.community} what="Registered community ID copied" />{/if}
    <p class="help">Every member needs the same community ID to share posts. A name reservation identifies a community; an online node must also hold its posts for others to receive them.</p>
  </details>
</div>

<style>
  .public-name { display: flex; flex-direction: column; gap: 10px; max-width: 620px; min-width: 0; }
  .help { margin-top: 6px; }
  .problem { font-weight: 700; }
  details { font-size: 13px; min-width: 0; }
  summary { cursor: pointer; font-weight: 700; }
  details :global(.field) { margin-top: 12px; }
  .tag { white-space: normal; }
</style>
