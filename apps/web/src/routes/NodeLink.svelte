<script lang="ts">
  // A public node's link: someone shared the node they run, so this device has somewhere to look
  // for communities. Nothing in the app names a node; links like this one, invites and the nodes
  // they lead to are how a browser finds the rest (hoi-core `nodes`). Before anything is dialed,
  // the core reads the link: where it points and, when the operator put their notice in it,
  // the notice, checked against the node the link names. Accepting it connects; a link
  // without a notice connects only after an extra word about that.
  import { onMount } from 'svelte';
  import type { NodeLinkPreview } from '@hoi/core-api';
  import { app, setTitle } from '../lib/app.svelte';
  import { router } from '../lib/router.svelte';
  import { to } from '../lib/router';
  import { enterView } from '../lib/motion';
  import { toast, toastError } from '../lib/toast.svelte';
  import NoticeText from '../lib/ui/NoticeText.svelte';

  let { query }: { query: string } = $props();

  const params = $derived(new URLSearchParams(query));
  const link = $derived(`hoi://node?${query}`);
  // `code` packs the addresses out of sight (hoi_types::NodeLink); `w`/`p` are older links in the clear
  const valid = $derived(!!params.get('code') || !!params.get('w') || !!params.get('p'));
  let preview = $state.raw<NodeLinkPreview | null>(null);
  let previewError = $state('');
  let busy = $state(false);
  let ackNoNotice = $state(false);

  onMount(() => setTitle('A public node'));

  $effect(() => {
    const l = link;
    preview = null;
    previewError = '';
    if (!valid) return;
    app.core.previewNodeLink(l).then(
      (p) => (preview = p),
      (e) => (previewError = e instanceof Error ? e.message : String(e)),
    );
  });

  const notice = $derived(preview?.notice ?? null);
  const accepted = $derived(notice?.state === 'accepted');
  const canConnect = $derived(!!preview && !preview.problem && (notice ? true : ackNoNotice));

  async function connect() {
    if (!preview || busy) return;
    busy = true;
    try {
      if (notice && !accepted) await app.core.acceptNotice(notice.node, notice.hash);
      app.net = { ...(app.net ?? (await app.core.net())), directory: await app.core.openNodeLink(link) };
      toast('Connecting to that node. The communities it carries show up here.');
      router.go(to.communities());
    } catch (err) {
      toastError(err);
    }
    busy = false;
  }

  async function decline() {
    if (!notice || busy) return;
    busy = true;
    try {
      await app.core.declineNotice(notice.node);
      toast('Declined. That node is not contacted.');
      router.go(to.communities());
    } catch (err) {
      toastError(err);
    }
    busy = false;
  }
</script>

<section class="view nodeview" use:enterView aria-labelledby="node-h">
  <span class="sticker anim">a place to look</span>
  <h1 id="node-h" class="anim">A public node.</h1>
  {#if !valid}
    <p class="callout anim">This link is missing its address. Ask whoever shared it for the link their node shows.</p>
  {:else if previewError}
    <p class="callout anim">{previewError}</p>
  {:else if !preview}
    <p class="lead anim">Reading the link…</p>
  {:else}
    <p class="lead anim">
      Someone shared the node they run. Connect, and you see the open communities it carries, can join one, and hear of the other public
      nodes it knows. Nothing is joined until you choose a community.
    </p>
    <p class="frame where anim">
      {#if preview.scope === 'device'}It points at <b>this device</b> (<span class="mono">{preview.host}</span>).
      {:else if preview.scope === 'network'}It points at a computer <b>on your network</b> (<span class="mono">{preview.host}</span>).
      {:else}It points at a computer <b>on the internet</b> (<span class="mono">{preview.host}</span>).{/if}
      Like any node you connect to, it sees this {app.platformName}'s address and which of its communities you're in too. It never learns your
      name or your other communities, and it can't sign anything for you.
    </p>
    {#if preview.problem}
      <p class="callout anim" role="alert">{preview.problem} The link cannot be opened.</p>
    {:else if notice}
      <p class="help anim">The operator's notice, signed by the node this link names{#if accepted}{' '}(accepted on this {app.platformName} before){/if}:</p>
      <div class="anim"><NoticeText title={notice.title} sections={notice.sections} footer={notice.footer} /></div>
    {:else}
      <div class="frame nonotice anim">
        <p><b>This node has not published an operator notice.</b> Whoever runs it has not said who they are, where the server stands or what it keeps. They are still responsible for the data their node processes; you cannot ask them about it from here.</p>
        <label class="ack"><input id="node-ack-nonotice" type="checkbox" bind:checked={ackNoNotice} /> <span>I understand, and want to connect to this node anyway.</span></label>
      </div>
    {/if}
    <div class="btnrow anim">
      <button type="button" class="pill pill-ink pill-lg" onclick={connect} disabled={busy || !canConnect}>{busy ? 'Connecting…' : notice && !accepted ? 'Accept and connect' : 'Connect and look around'}</button>
      {#if notice && !accepted}
        <button type="button" class="pill pill-lg" onclick={decline} disabled={busy}>Decline</button>
      {:else}
        <a class="pill pill-lg" href={to.communities()}>Not now</a>
      {/if}
    </div>
  {/if}
</section>

<style>
  .nodeview { display: flex; flex-direction: column; gap: 18px; align-items: flex-start; max-width: 720px; }
  h1 { font-size: clamp(36px, 7vw, 64px); line-height: 1; }
  .lead { font-size: var(--type-lead-size); line-height: var(--type-lead-lh); color: var(--mute); }
  .where { padding: 14px 16px; line-height: 1.5; overflow-wrap: anywhere; }
  .nonotice { padding: 14px 16px; display: flex; flex-direction: column; gap: 12px; line-height: 1.5; border-style: dashed; }
  .ack { display: flex; gap: 10px; align-items: flex-start; font-weight: 600; cursor: pointer; }
  .ack input { width: 20px; height: 20px; margin-top: 2px; flex: none; accent-color: var(--ink); }
</style>
