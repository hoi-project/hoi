<script lang="ts">
  import { app } from '../app.svelte';
  import { toast, toastError } from '../toast.svelte';
  import { saveFile } from '../util';
  import CopyField from './CopyField.svelte';
  import { RELAY_URLS } from '../private-relay';
  import { privateHelp, privateLine } from '../private-status';

  let { community }: { community: string } = $props();
  const group = $derived(app.privateGroups.find(g => g.community === community));
  const sync = $derived(app.privateSync[community]);
  const line = $derived(group ? privateLine(group, sync, RELAY_URLS.length, app.platformName) : undefined);
  let request = $state('');
  let invitation = $state('');
  let recipient = $state<{ root: string; device: string } | null>(null);
  let busy = $state(false);
  let removing = $state('');
  let leaving = $state(false);
  let passing = $state('');

  /** Where membership follows the records, any member hands a request on; the owner's device adds them once the records admit them. */
  async function pass(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    try {
      await app.core.privatePassRequest(community, passing.trim());
      passing = '';
      await app.refresh();
      toast('Passed on. Their invitation appears here once the community admits them.');
    } catch (error) { toastError(error); }
    finally { busy = false; }
  }

  async function leave() {
    busy = true;
    try {
      await app.core.leave(community);
      await app.refresh();
      leaving = false;
      toast('Private keys removed from this device. Ask the owner to remove its membership too.');
    } catch (error) { toastError(error); }
    finally { busy = false; }
  }

  /** A relay group must deliver what waits first; a mailbox group only orders it ahead of the change. */
  async function readyForChange() {
    const now = await app.core.privateGroup(community);
    if (now.pending && now.transport !== 'mailboxes') throw new Error('Finish sending saved updates before changing membership.');
  }

  async function check(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    recipient = null;
    try { recipient = await app.core.privatePreviewRequest(request.trim()); }
    catch (error) { toastError(error); }
    finally { busy = false; }
  }

  async function approve() {
    if (!recipient) return;
    busy = true;
    try {
      await app.syncPrivate(community);
      await readyForChange();
      const code = await app.core.privateInvite(community, request.trim());
      await app.refresh();
      const result = await app.syncPrivate(community);
      invitation = code;
      recipient = null;
      request = '';
      toast(result.error ? 'Invitation saved. Keep this device open and retry syncing before your friend joins.' : 'Invitation ready. Send it directly to your friend.');
    } catch (error) { toastError(error); }
    finally { busy = false; }
  }

  async function remove(device: string) {
    busy = true;
    try {
      await app.syncPrivate(community);
      await readyForChange();
      await app.core.privateRemove(community, device);
      removing = '';
      await app.refresh();
      const result = await app.syncPrivate(community);
      toast(result.error ? 'Removal saved locally. Retry syncing to deliver the new membership keys.' : 'Device removed. New messages use new keys.');
    } catch (error) { toastError(error); }
    finally { busy = false; }
  }
</script>

{#if group}
  <section class="frame private-group" aria-label="Private membership">
    <div class="status-row"><span class="tag tag-lilac">Private · encrypted for members</span>{#if line?.relay}<button type="button" class="pill pill-small" disabled={busy || sync?.busy || !group.active} onclick={() => app.syncPrivate(community)}>{sync?.busy ? 'Syncing…' : 'Sync private relay'}</button>{/if}</div>
    {#if group.active}
      <p role="status" class={[line?.trouble && 'trouble']}><span class={['dot', line?.connected && 'on']} aria-hidden="true"></span>{line?.text}</p>
      <p class="help">{privateHelp(group)}</p>
    {:else}<p class="callout">{line?.text}</p>{/if}
    {#if group.active && !group.isOwner}
      {#if leaving}
        <p>Leave on this device and delete its private group keys? Rejoining needs a new invitation. Ask the owner to remove this device from the group as well.</p>
        <div><button type="button" class="pill" onclick={leave} disabled={busy}>Leave this device</button><button type="button" class="act" onclick={() => leaving = false}>Cancel</button></div>
      {:else}<button type="button" class="act" onclick={() => leaving = true}>Leave on this device…</button>{/if}
    {/if}
    {#if group.active && group.followsRecords}
      <p class="help">Who can read follows this community’s own records: the owner’s and moderators’ devices add people the community admits and remove those it bans, and every member’s device refuses anything else. They take turns in a fixed order, the owner first, then moderators by how long they have been ones; when the first stays silent, the next takes over after an hour.</p>
      {#if group.rank != null}<p class="help">{group.rank === 0 ? 'This device is first in line.' : `This device is number ${group.rank + 1} of ${group.committers} in line: it steps in after ${group.rank === 1 ? 'an hour' : `${group.rank} hours`} of silence from those ahead.`}</p>{/if}
      {#if !group.isOwner}
        <details>
          <summary>Invite someone</summary>
          <form onsubmit={pass}>
            <p class="help">Ask them to create an invitation request in Private communities and send it to you. Passing it on vouches for them; once the community admits them, their app picks up the invitation by itself.</p>
            <label class="field"><span>Their invitation request</span><textarea class="input" bind:value={passing} rows="3" maxlength="2097152" autocomplete="off" spellcheck="false"></textarea></label>
            <button class="pill" disabled={busy || !passing.trim()}>Pass it on</button>
          </form>
        </details>
      {/if}
      {#each group.invitations as ready (ready.device)}
        <CopyField label={`Invitation for device ${ready.device.slice(0, 12)}… — their app picks it up by itself; send it only if it hasn’t joined`} value={ready.invitation} what="Private invitation copied" />
      {/each}
    {/if}
    {#if group.active && group.isOwner}
      <details>
        <summary>Invite someone or manage devices</summary>
        <form onsubmit={check}>
          <p class="help">Ask your friend to open Private communities and create an invitation request. Verify their key through a trusted conversation before approving it.</p>
          <label class="field"><span>Their invitation request</span><textarea class="input" bind:value={request} oninput={() => { recipient = null; }} rows="3" maxlength="2097152" autocomplete="off" spellcheck="false"></textarea></label>
          <button class="pill" disabled={busy || !request.trim()}>Check invitation request</button>
        </form>
        {#if recipient}
          <CopyField label="Verified person key" value={recipient.root} what="Person key copied" />
          <CopyField label="Device to invite" value={recipient.device} what="Device key copied" />
          <button type="button" class="pill pill-ink" onclick={approve} disabled={busy}>Approve this device</button>
        {/if}
        {#if invitation}
          <CopyField label="Send this private invitation directly to your friend" value={invitation} what="Private invitation copied" />
          <button type="button" class="pill" onclick={() => saveFile('invitation.hoiprivate', new TextEncoder().encode(invitation)).catch(toastError)}>Save invitation file</button>
        {/if}
        <ul class="members">
          {#each group.members as member (member.device)}
            <li><span><b>{member.root.slice(0, 16)}…</b><br />Device {member.device.slice(0, 16)}…</span>
              {#if member.device !== app.me?.device}
                {#if removing === member.device}
                  <span>Remove access to future messages?<button type="button" class="pill" onclick={() => remove(member.device)} disabled={busy}>Remove device</button><button type="button" class="act" onclick={() => removing = ''}>Cancel</button></span>
                {:else}<button type="button" class="act" onclick={() => removing = member.device}>Remove…</button>{/if}
              {/if}
            </li>
          {/each}
        </ul>
      </details>
    {/if}
  </section>
{/if}

<style>
  .private-group { padding: 18px; display: flex; flex-direction: column; gap: 10px; font-size: 14px; margin-block: 16px; }
  .status-row { display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 10px; }
  [role='status'] { display: flex; align-items: baseline; gap: 8px; }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--mute); flex: 0 0 8px; transform: translateY(-1px); }
  .dot.on { background: var(--ink); }
  .trouble .dot { background: var(--coral); box-shadow: 0 0 0 2px var(--ink); }
  details, form { display: flex; flex-direction: column; gap: 12px; }
  details > :global(*) { margin-block: 10px; }
  summary { cursor: pointer; font-weight: 800; }
  .members { list-style: none; padding: 0; }
  li { display: flex; align-items: center; justify-content: space-between; gap: 10px; padding-block: 12px; border-top: 1px solid var(--line); overflow-wrap: anywhere; }
  li span:last-child { display: flex; flex-wrap: wrap; gap: 8px; }
  textarea { resize: vertical; }
</style>
