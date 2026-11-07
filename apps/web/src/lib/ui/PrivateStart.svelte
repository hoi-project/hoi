<script lang="ts">
  // Start a community → Private: invite-only, encrypted on the device. Messages wait in mailboxes
  // on public nodes the group agrees on, which can't read them. You approve every invitation; it
  // never shows up in the public directory.
  import { DEFAULT_LIMITS } from '@hoi/core-api';
  import { app } from '../app.svelte';
  import { router } from '../router.svelte';
  import { to } from '../router';
  import { toast, toastError } from '../toast.svelte';

  let name = $state('');
  let description = $state('');
  let busy = $state(false);

  async function create(event: SubmitEvent) {
    event.preventDefault();
    if (!name.trim()) return;
    busy = true;
    try {
      const c = await app.core.createPrivateCommunity({ name: name.trim(), description: description.trim(), color: 'lilac', limits: { ...DEFAULT_LIMITS, maxRetention: 30 * 86400 }, threshold: 1 });
      await app.refresh();
      void app.syncPrivate(c.id);
      router.go(to.community(c.id));
      toast('Private community created. Invite each person from its membership panel.');
    } catch (error) { toastError(error); }
    finally { busy = false; }
  }
</script>

<form class="frame box" onsubmit={create}>
  <ul class="facts">
    <li><span class="tag tag-lilac">invite only</span> only devices you approve get the keys to read it</li>
    <li><span class="tag tag-mint">encrypted</span> posts and media are encrypted on your device, before anything leaves it</li>
    <li><span class="tag tag-sky">mailboxes</span> messages wait on a few public nodes in different places, which keep them sealed and can’t read them</li>
    <li><span class="tag tag-soft">unlisted</span> never in the public directory</li>
    <li><span class="tag tag-peach">copies</span> members can keep what they read; removing someone protects only new posts</li>
  </ul>
  <p class="help">Your {app.platformName} chooses the mailboxes by itself once it reaches two public nodes that keep sealed messages. Nothing to set up.</p>
  <label class="field"><span>Name</span><input class="input" bind:value={name} maxlength="32" required placeholder="Friends and family" /></label>
  <label class="field"><span>What it’s about</span><textarea class="input" bind:value={description} maxlength="2000" rows="3"></textarea></label>
  <div class="btnrow">
    <button class="pill pill-ink pill-lg" disabled={busy || !name.trim()}>{busy ? 'Working…' : 'Create private community'}</button>
    <a class="pill pill-lg" href="#/private">I have an invitation</a>
  </div>
</form>

<style>
  .box { padding: 22px; display: flex; flex-direction: column; gap: 16px; max-width: 720px; }
  .facts { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; font-size: 14px; color: var(--mute); font-weight: 500; }
  .facts li { display: flex; align-items: baseline; gap: 10px; }
  .facts .tag { min-width: 96px; justify-content: center; flex: 0 0 auto; }
  textarea { resize: vertical; overflow-wrap: anywhere; }
  @media (max-width: 640px) { .box { padding: 16px; } .facts li { flex-direction: column; gap: 4px; } }
</style>
