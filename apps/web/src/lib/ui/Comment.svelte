<script lang="ts">
  import type { CommentView, PostView } from '@hoi/core-api';
  import { ago } from '@hoi/core-api';
  import { app, attempt, signedAndSent } from '../app.svelte';
  import { to } from '../router';
  import { bounce, dropIn } from '../motion';
  import { toast, toastError } from '../toast.svelte';
  import Avatar from './Avatar.svelte';
  import TrustTag from './TrustTag.svelte';
  import ReplyBox from './ReplyBox.svelte';
  import FlagPicker from './FlagPicker.svelte';
  import ReviewPicker from './ReviewPicker.svelte';
  import { napWho } from '../util';
  import Self from './Comment.svelte';

  let {
    c,
    post,
    canMod,
    canReview = false,
    depth = 0,
    freshId = null,
    rules = null,
    onreplied,
  }: {
    c: CommentView;
    post: PostView;
    canMod: boolean;
    canReview?: boolean;
    depth?: number;
    freshId?: string | null;
    rules?: string | null;
    onreplied: (c: CommentView) => void;
  } = $props();

  let peek = $state(false);
  let replying = $state(false);
  let editing = $state(false);
  let flagging = $state(false);
  let hiding = $state(false);
  let removing = $state(false);
  let confirmDelete = $state(false);
  let draft = $state('');
  let reason = $state('');
  let score = $derived(c.score);
  let mine = $derived(c.myVote);
  let el: HTMLElement | undefined = $state();
  let upBtn: HTMLElement | undefined = $state();

  const isOp = $derived(c.author.key === post.author.key);
  const short4 = $derived(c.author.short.slice(0, 4));

  $effect(() => {
    if (freshId === c.id && el) dropIn(el);
  });

  async function up() {
    if (!(await app.confirmPosting(post.community.id))) return;
    const next = mine === 1 ? 0 : 1;
    const before = { score, mine };
    score = score - mine + next;
    mine = next;
    bounce(upBtn ?? null, 1);
    try {
      score = await app.core.vote(c.id, next);
      toast(next ? signedAndSent('Upvote', post.community.id) : 'Vote taken back. Signed, too.');
    } catch (e) {
      score = before.score;
      mine = before.mine;
      toastError(e);
    }
  }

  async function saveEdit(e: SubmitEvent) {
    e.preventDefault();
    const ok = await attempt(() => app.core.edit(c.id, draft), 'Edit signed. Everyone sees the new version.');
    if (ok !== undefined) editing = false;
  }

  async function del() {
    await attempt(() => app.core.deleteOwn(c.id), 'Deleted. Peers drop the body and keep a tombstone.');
    confirmDelete = false;
  }

  async function remove(e: SubmitEvent) {
    e.preventDefault();
    const ok = await attempt(() => app.core.remove(c.id, reason), 'Removed. The reason is signed into the ledger.');
    if (ok !== undefined) removing = false;
  }
</script>

<div class="c anim" bind:this={el} id={`c-${c.id}`}>
  {#if c.removed}
    <Avatar name="" ghost />
    <div class="cmain">
      <div class="poof">
        <div class="meta">
          <span class="who">Poof.</span>
          <span>
            {#if c.removed.by}Removed by {c.removed.by.name} · {c.removed.reason}{:else}Deleted by the author{/if} · {ago(c.removed.at)}
          </span>
        </div>
        <p class="cb">
          {#if c.removed.by}
            Receipt is in the <a class="textlink" href={to.ledger(post.community.id)}>ledger</a>. Your device tossed its copy. The author can appeal.
          {:else}
            The author signed a delete. Every device that follows the rules drops the body.
          {/if}
        </p>
      </div>
      {@render kids()}
    </div>
  {:else if c.napping && !peek}
    <span class="zz sm" aria-hidden="true">zz</span>
    <div class="cmain">
      <p class="napline">
        A reply is napping. {napWho(c.napping)}: <b>{c.napping.label}</b>.
        <button type="button" class="peek" onclick={() => (peek = true)}>Peek anyway</button>
      </p>
      {@render kids()}
    </div>
  {:else}
    <Avatar name={c.author.name} keyHex={c.author.key} avatar={c.author.avatar} avatarColor={c.author.avatarColor} mine={c.mine} />
    <div class="cmain">
      <div class="meta">
        <a class="who" href={to.person(c.author.key)}>{c.author.name}</a>
        {#if c.author.name !== c.author.short}<span class="mono">{short4}</span>{/if}
        {#if c.author.role}<span class="tag tag-sun">{c.author.role}</span>{/if}
        {#if isOp}<span class="tag tag-{post.community.color}">op</span>{/if}
        {#if c.mine}<span class="tag tag-sun">yours</span>{:else}<TrustTag person={c.author} />{/if}
        {#if c.napping}<span class="tag tag-soft">napping: {c.napping.label}</span>{/if}
        {#if c.waiting}<span class="tag tag-dashed">waiting at the door</span>{:else if c.unvalidated}<span class="tag tag-dashed" title="Nobody established here has vouched for them yet">new here</span>{/if}
        {#if c.held}<span class="tag tag-peach" title={c.held}>held back</span>{/if}
        <span>· {ago(c.createdAt)}{c.edited ? ' · edited' : ''}</span>
        <span class="push score num" aria-label={`${score} points`}>▲ {score}</span>
      </div>
      {#if editing}
        <form class="editf" onsubmit={saveEdit}>
          <label class="sr-only" for={`e-${c.id}`}>Edit your reply</label>
          <textarea id={`e-${c.id}`} class="textarea" bind:value={draft} rows="3"></textarea>
          <div class="btnrow">
            <button class="pill pill-ink pill-sm" type="submit">Sign the edit</button>
            <button class="pill pill-sm" type="button" onclick={() => (editing = false)}>Cancel</button>
          </div>
        </form>
      {:else}
        <p class="cb">{c.text}</p>
      {/if}
      <div class="ca">
        <button type="button" class={['act', mine === 1 && 'on']} bind:this={upBtn} aria-pressed={mine === 1} onclick={up}>▲ {mine === 1 ? 'Upped' : 'Up'}</button>
        <button type="button" class="act" aria-expanded={replying} onclick={() => (replying = !replying)}>Reply</button>
        {#if c.mine}
          <button type="button" class="act" onclick={() => { draft = c.text; editing = true; }}>Edit</button>
          <button type="button" class="act" onclick={() => (confirmDelete = !confirmDelete)}>Delete</button>
        {:else}
          {#if canReview}<button type="button" class="act" aria-expanded={hiding} onclick={() => (hiding = !hiding)}>Hide</button>{/if}
          <button type="button" class="act" aria-expanded={flagging} onclick={() => (flagging = !flagging)}>Flag</button>
          {#if canMod}<button type="button" class="act" aria-expanded={removing} onclick={() => (removing = !removing)}>Remove</button>{/if}
        {/if}
      </div>
      {#if confirmDelete}
        <div class="inset small">
          <span>Delete your reply? Peers drop the text and keep a tombstone with your signature.</span>
          <div class="btnrow">
            <button type="button" class="pill pill-danger pill-sm" onclick={del}>Delete it</button>
            <button type="button" class="pill pill-sm" onclick={() => (confirmDelete = false)}>Keep it</button>
          </div>
        </div>
      {/if}
      {#if removing}
        <form class="inset small" onsubmit={remove}>
          <label class="field">
            <span>Why remove it? This goes in the public ledger.</span>
            <input class="input" bind:value={reason} placeholder="rule 2, doxxing" maxlength="200" />
          </label>
          <div class="btnrow">
            <button type="submit" class="pill pill-danger pill-sm">Remove &amp; sign</button>
            <button type="button" class="pill pill-sm" onclick={() => (removing = false)}>Cancel</button>
          </div>
        </form>
      {/if}
      {#if flagging}<FlagPicker target={c.id} {rules} ondone={() => (flagging = false)} />{/if}
      {#if hiding}<ReviewPicker target={c.id} {rules} ondone={() => (hiding = false)} />{/if}
      {#if replying}
        <ReplyBox parent={c.id} community={post.community.id} compact autofocus oncancel={() => (replying = false)}
          onsent={(r) => { replying = false; onreplied(r); }} />
      {/if}
      {@render kids()}
    </div>
  {/if}
</div>

{#snippet kids()}
  {#if c.children.length}
    <div class={['nest', depth >= 3 && 'deep']}>
      {#each c.children as child (child.id)}
        <Self c={child} {post} {canMod} {canReview} depth={depth + 1} {freshId} {rules} {onreplied} />
      {/each}
    </div>
  {/if}
{/snippet}

<style>
  a.who { min-height: 24px; display: inline-flex; align-items: center; }
  .c { display: grid; grid-template-columns: 38px minmax(0, 1fr); gap: 14px; padding: 18px 0; border-top: 2px dotted var(--line); }
  .cmain { min-width: 0; }
  .cb { margin: 6px 0 0; font-size: 15px; line-height: 1.5; max-width: 620px; overflow-wrap: anywhere; white-space: pre-line; }
  .poof .cb { font-size: 14px; color: var(--mute); }
  .ca { display: flex; gap: 2px; margin: 6px 0 0 -10px; flex-wrap: wrap; }
  .score { font-weight: 700; color: var(--ink); }
  .nest { border-left: 3px solid var(--sun); padding-left: 18px; margin-top: 6px; }
  .nest > :global(.c:first-child) { border-top: 0; }
  .nest.deep { padding-left: 10px; }
  .zz.sm { width: 34px; height: 34px; font-size: 11px; }
  .napline { font-size: 14px; color: var(--mute); font-weight: 500; min-height: 34px; padding-top: 6px; }
  .napline b { color: var(--ink); }
  .inset.small { margin-top: 8px; max-width: 640px; font-size: 14px; font-weight: 500; }
  .editf { display: flex; flex-direction: column; gap: 8px; margin-top: 6px; max-width: 640px; }
  @media (max-width: 640px) {
    .c { grid-template-columns: 30px minmax(0, 1fr); gap: 10px; padding: 14px 0; }
    .c :global(.av:not(.lg)) { width: 30px; height: 30px; font-size: 10px; }
    .nest { padding-left: 12px; }
    .nest.deep { padding-left: 6px; border-left-width: 2px; }
  }
</style>
