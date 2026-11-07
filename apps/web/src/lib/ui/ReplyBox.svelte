<script lang="ts">
  import type { CommentView, Hex } from '@hoi/core-api';
  import { onMount } from 'svelte';
  import { app, sentTo } from '../app.svelte';
  import { animate, reduced } from '../motion';
  import { toast, toastError } from '../toast.svelte';
  import { formatMs } from '../util';
  import Avatar from './Avatar.svelte';
  import { canPublishPublic } from '../network-status';
  import { REGISTRY_URL } from '../registry';

  let {
    parent,
    community = undefined,
    onsent,
    oncancel,
    autofocus = false,
    compact = false,
  }: { parent: Hex; community?: Hex; onsent?: (c: CommentView) => void; oncancel?: () => void; autofocus?: boolean; compact?: boolean } = $props();

  let text = $state('');
  let busy = $state(false);
  let bar: HTMLElement | undefined = $state();
  let ta: HTMLTextAreaElement | undefined = $state();
  const id = `r-${Math.random().toString(36).slice(2, 8)}`;
  const canSend = $derived(app.privateGroups.some(group => group.community === community) || canPublishPublic(app.net, community));

  onMount(() => {
    if (autofocus) ta?.focus();
  });

  async function send(e: SubmitEvent) {
    e.preventDefault();
    if (busy) return;
    if (!text.trim()) {
      toast('Write something first');
      ta?.focus();
      return;
    }
    busy = true;
    let took = 0;
    const run: { a: ReturnType<typeof animate> | null } = { a: null };
    const off = app.onStamp((ev) => {
      if (ev.phase === 'start' && bar) {
        if (reduced()) bar.style.width = '60%';
        else run.a = animate(bar, { width: ['0%', '94%'], duration: ev.estimateMs, ease: 'inOutQuad' });
      } else if (ev.tookMs) took = ev.tookMs;
    });
    try {
      await app.requirePostingConnection(community);
      const c = await app.core.reply(parent, text.trim());
      text = '';
      toast(`Reply signed${took ? `, stamp took ${formatMs(took)}` : ''}. ${sentTo(community)}`);
      onsent?.(c);
    } catch (err) {
      toastError(err);
    } finally {
      off();
      run.a?.pause();
      if (bar) bar.style.width = '0%';
      busy = false;
    }
  }
</script>

<form class={['reply', compact && 'compact']} onsubmit={send} aria-busy={busy}>
  {#if !compact && app.me}<Avatar name={app.me.name} keyHex={app.me.rootKey} avatar={app.me.avatar} avatarColor={app.me.avatarColor} mine />{/if}
  <div class="rcol">
    <label class="sr-only" for={id}>Reply</label>
    <textarea {id} bind:this={ta} bind:value={text} rows="2" disabled={busy}
      placeholder={`Say something${app.me ? `, ${app.me.name}` : ''}. You can keep writing while waiting for a connection.`}></textarea>
    {#if !canSend}<p class="help" role="status">Reading saved posts is available offline. Sending this reply needs a home node carrying this community: a member's, a public one, or your own. Your text stays here while you reconnect.</p>{/if}
    <div class="work" aria-hidden="true"><i bind:this={bar}></i></div>
    <div class="btnrow">
      <button type="submit" class="pill pill-ink" disabled={busy || !canSend}>{busy ? 'Signing…' : 'Sign & send →'}</button>
      {#if !canSend && REGISTRY_URL}<button type="button" class="pill" disabled={busy || app.publicConnection.busy} onclick={() => app.connectPublic(community, true)}>Retry public relays</button>{/if}
      {#if oncancel}<button type="button" class="pill" onclick={oncancel} disabled={busy}>Cancel</button>{/if}
    </div>
  </div>
</form>

<style>
  .reply { display: grid; grid-template-columns: 38px minmax(0, 1fr); gap: 14px; padding: 22px 0 4px; }
  .reply.compact { grid-template-columns: minmax(0, 1fr); padding: 10px 0 4px; }
  .rcol { display: flex; flex-direction: column; gap: 10px; min-width: 0; }
  textarea {
    width: 100%; resize: vertical; border: 2px solid var(--ink); border-radius: 16px; padding: 12px 14px; font: inherit; font-size: 16px;
    background: var(--card); color: var(--ink); max-width: 640px; min-height: 64px;
  }
  .work { height: 6px; border-radius: 999px; background: var(--soft); overflow: hidden; max-width: 640px; }
  .work i { display: block; height: 100%; width: 0; background: var(--coral); border-radius: 999px; }
  @media (max-width: 640px) {
    .reply { grid-template-columns: minmax(0, 1fr); }
    .reply :global(.av) { display: none; }
  }
</style>
