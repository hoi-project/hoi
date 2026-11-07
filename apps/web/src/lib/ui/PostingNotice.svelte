<script lang="ts">
  // Asked once, on a browser or phone with no home node of its own, before its first post, reply,
  // vote or vouch. Nobody can reach a browser, so what it signs is readable by others only once a
  // home node that carries the community has it: someone's who keeps one running, or your own.
  import { app } from '../app.svelte';
  import { plural } from '../util';
  import Modal from './Modal.svelte';

  const notice = $derived(app.postingNotice);
</script>

<Modal open={!!notice} title="Nobody can read posts from this {app.platformName}" onclose={() => notice && app.answerPostingNotice(false)}>
  {#if notice}
    <div class="notice">
      <p>
        Nothing can connect to a {app.platformName}, so what you post here reaches others only through a home node that carries this community{notice.carriers
          ? ` (${plural(notice.carriers, 'node')} connected now)`
          : ''}. Once one has it, it keeps it for the community after you close this {app.platformName === 'browser' ? 'tab' : 'app'}.
      </p>
      <p>If you close it before any has your post, nobody else sees it until you're back. Votes and vouches travel the same way.</p>
      <p class="help">Your own home node keeps what you post available while you're away, and is a place where others find communities too.</p>
      <div class="btnrow">
        <button type="button" class="pill pill-ink" onclick={() => app.answerPostingNotice(true)}>Post through them</button>
        <a class="pill" href="https://hoi.sh/download/" target="_blank" rel="noopener" onclick={() => app.answerPostingNotice(false)}>Get the home node</a>
        <button type="button" class="textlink" onclick={() => app.answerPostingNotice(false)}>Not now</button>
      </div>
    </div>
  {/if}
</Modal>

<style>
  .notice { display: flex; flex-direction: column; gap: 12px; }
  .notice p { line-height: 1.5; }
</style>
