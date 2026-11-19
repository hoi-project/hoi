<script lang="ts">
  import { app, setTitle } from '../lib/app.svelte';
  import { router } from '../lib/router.svelte';
  import { to } from '../lib/router';
  import { toast, toastError } from '../lib/toast.svelte';
  import { readFile } from '../lib/util';
  import CopyField from '../lib/ui/CopyField.svelte';
  import JoinWarning from '../lib/ui/JoinWarning.svelte';
  import { resource } from '../lib/resource.svelte';

  setTitle('Private communities');
  let request = $state('');
  let invite = $state('');
  let busy = $state(false);
  let joinWarning = $state(false);
  let askWarning = $state(false);
  let restoreWarning = $state(false);
  let restoringCommunity = $state('');
  let restoreRequests = $state<Record<string, string>>({});
  const rejoins = resource(() => { void app.rev; return app.core.snapshotRejoins(); });

  async function requestRestoredAccess() {
    if (busy || !restoringCommunity) return;
    const community = restoringCommunity;
    busy = true;
    try {
      const code = await app.core.snapshotRejoinRequest(community);
      restoreRequests = { [community]: code };
      request = '';
      await app.refresh();
    } catch (error) { toastError(error); }
    finally { busy = false; }
  }

  async function ask() {
    askWarning = false;
    busy = true;
    try { request = await app.core.privateJoinRequest(); restoreRequests = {}; }
    catch (error) { toastError(error); }
    finally { busy = false; }
  }

  function openJoinWarning(event: SubmitEvent) {
    event.preventDefault();
    if (busy || !invite.trim()) return;
    joinWarning = true;
  }

  async function join() {
    if (busy || !invite.trim()) return;
    busy = true;
    try {
      const c = await app.core.privateJoin(invite.trim());
      invite = '';
      request = '';
      restoreRequests = {};
      await app.refresh();
      rejoins.reload();
      await app.syncPrivate(c.id);
      router.go(to.community(c.id));
      toast('Private invitation verified. This device is now a member.');
    } catch (error) { toastError(error); }
    finally { busy = false; }
  }

  async function loadInvite(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    if (file.size > 2_097_152) { input.value = ''; return toast('That invitation is too large.'); }
    try { invite = new TextDecoder().decode(await readFile(file)); }
    catch (error) { toastError(error); }
    input.value = '';
  }
</script>

<JoinWarning bind:open={restoreWarning} oncontinue={requestRestoredAccess} />

<section class="view private-page" aria-labelledby="private-title">
  <div class="vh"><h1 id="private-title">Private communities</h1><a class="pill pill-ink" href={to.start('private')}>+ Start a private community</a></div>
  <p class="lead">Invite only. Posts and media are encrypted on your device, and only devices you approve can read them.</p>
  <p class="help">Messages wait in mailboxes on a few public nodes in different places. Those nodes keep them sealed and can’t read them; only members can even find them. Members can keep what they read. Removing a device protects new posts, not copies already received.</p>

  {#if rejoins.error}<p class="callout" role="alert">Restored community status is unavailable. {rejoins.error}</p>{/if}
  {#if rejoins.value?.length}
    <section class="frame box" aria-labelledby="snapshot-private-title">
      <h2 id="snapshot-private-title">Restore private access</h2>
      <p>Your snapshot kept these community names, but its old private messages were discarded. Request fresh access separately for each community; an authorized member must approve it. Nothing here approves your own membership.</p>
      {#each rejoins.value as [id, name] (id)}
        <div class="restore-community">
          <h3>{name}</h3>
          <button type="button" class="pill" disabled={busy} onclick={() => { restoringCommunity = id; restoreWarning = true; }}>Request access to {name}</button>
          {#if restoreRequests[id]}<CopyField label={`Send the request for ${name} to an authorized member`} value={restoreRequests[id]} what="Restored-community request copied" />{/if}
        </div>
      {/each}
      <p class="help">Keep hoi open for the invitation, or paste the one the member sends in Join by invitation below. Make one request at a time: requesting another community replaces the earlier request. Fresh approval allows new deliveries. Private history is not restored from the snapshot; another member may supply still-valid content again.</p>
    </section>
  {/if}

  <section class="frame box" aria-labelledby="private-join-title">
    <h2 id="private-join-title">Join by invitation</h2>
    <ol class="steps">
      <li><b>Make a request</b> on this device and send it to a member.</li>
      <li><b>Keep hoi open.</b> Once the community admits you, this app picks up the invitation by itself from public nodes it named in the request: only this device can open it.</li>
    </ol>
    <p class="help">One request per invitation; make a new one for each community. If a member sends you the invitation instead, paste it below.</p>
    <button type="button" class="pill" onclick={() => askWarning = true} disabled={busy}>Create invitation request</button>
    {#if request}<CopyField label="Send this request to a member" value={request} what="Invitation request copied" />{/if}
    <form onsubmit={openJoinWarning}>
      <label class="field"><span>Your private invitation</span><textarea class="input" bind:value={invite} rows="4" maxlength="2097152" autocomplete="off" spellcheck="false" placeholder="Paste the invitation you received"></textarea></label>
      <label class="field"><span>Or open an invitation file</span><input type="file" accept=".hoiprivate,text/plain" onchange={loadInvite} /></label>
      <button class="pill pill-ink" disabled={busy || !invite.trim()}>Open private invitation</button>
    </form>
  </section>
  {#if app.privateGroups.length}
    <section class="box"><h2>Your private communities</h2>
      {#each app.privateGroups as group (group.community)}
        <a class="textlink" href={to.community(group.community)}>{app.communities.find(c => c.id === group.community)?.name ?? 'Private community'}{group.active ? '' : ' · former member'}</a>
      {/each}
    </section>
  {/if}
</section>

<JoinWarning bind:open={joinWarning} oncontinue={join} destinationNote="The encrypted invitation names no public node to check before opening. Current connection reports are available in Connection IP." />
<JoinWarning bind:open={askWarning} oncontinue={ask} destinationNote="A private join request has no single destination to check. Current connection reports are available in Connection IP." />

<style>
  .private-page { display: flex; flex-direction: column; gap: 20px; }
  .box, form { display: flex; flex-direction: column; gap: 16px; align-items: stretch; }
  .box { padding: 24px; }
  .box button { align-self: flex-start; }
  .lead { font-size: 18px; line-height: 1.6; }
  .vh { padding-bottom: 0; }
  .steps { margin: 0; padding-left: 22px; display: flex; flex-direction: column; gap: 4px; line-height: 1.5; }
  textarea { resize: vertical; overflow-wrap: anywhere; }
  .restore-community { display: grid; gap: var(--space-3); min-width: 0; }
  @media (max-width: 640px) { .box { padding: 16px; } }
</style>
