<script lang="ts">
  // Who else holds a community's records, read from this device's own links (PROTOCOL.md §20):
  // a word and a colour, never the colour alone, with the numbers behind it on request. It is
  // never a receipt that any post reached anyone.
  import type { CommunityWarmth } from '@hoi/core-api';
  import type { Snippet } from 'svelte';
  import { ago } from '@hoi/core-api';
  import { app } from '../app.svelte';
  import { othersDurable, warmthWords } from '../warmth';

  // `fold`: one line (badge, meter, sentence) that opens to the reasons, the numbers and
  // whatever the page adds (`children`): the community page's way to keep its header short.
  let { w, compact = false, details = false, fold = false, announced = undefined, children = undefined }: {
    w: CommunityWarmth; compact?: boolean; details?: boolean; fold?: boolean; announced?: number; children?: Snippet;
  } = $props();
  const words = $derived(warmthWords(w, { announced, platformName: app.platformName }));
  const others = $derived(othersDurable(w));
  const pct = (v: number) => `${Math.round(v * 100)}%`;
  const copies = (v: number) => (Number.isInteger(v) ? String(v) : v.toFixed(1));
  const lastSync = $derived(w.lastSync ? ago(w.lastSync) : null);
</script>

{#snippet numbers()}
  <p class="help">Warmth estimates how available this community is on other devices. Other nodes can keep sharing when you close hoi. This estimate cannot confirm that a particular post reached someone or that a node will stay online.</p>
  <dl>
    <div><dt>Estimated copies</dt><dd>{copies(w.copies)} total, including this {app.platformName}</dd></div>
    <div><dt>Copies on other nodes</dt><dd>{copies(others)}{w.kept < others ? `, ${copies(w.kept)} once each place is counted once` : ''} — used for the warmth score</dd></div>
    {#if others > 0}<div><dt>Independent places</dt><dd>{w.places} {w.places === 1 ? 'place holds' : 'places hold'} a copy{w.places < w.holdersDurable ? '; some nodes share a network or an operator' : ''}</dd></div>{/if}
    {#if others > 0}<div><dt>Checked links</dt><dd>{pct(w.guarded)} of those copies are reached over links that are encrypted and whose other end was checked</dd></div>{/if}
    <div><dt>Connected devices</dt><dd>{w.holdersDurable} {w.holdersDurable === 1 ? 'node' : 'nodes'} · {w.holdersTransient} {w.holdersTransient === 1 ? 'browser or phone' : 'browsers or phones'}, excluding this device</dd></div>
    {#if w.networkCopies > others}<div><dt>Reported elsewhere</dt><dd>A connected node reports {copies(w.networkCopies)} copies on other nodes; unconfirmed here</dd></div>{/if}
    {#if announced}<div><dt>Public nodes listed</dt><dd>{announced} in the registry; they may be offline</dd></div>{/if}
    <div><dt>Community records</dt><dd>{w.records.toLocaleString('en')} saved · {w.missing.toLocaleString('en')} to download ({pct(w.complete)} saved)</dd></div>
    <div><dt>Pictures downloaded</dt><dd>{pct(w.media)} of those referenced by saved posts</dd></div>
    <div><dt>Last completed sync</dt><dd>{lastSync ? (lastSync === 'just now' ? 'Just now' : `${lastSync} ago`) : 'Not yet'}</dd></div>
    <div><dt>Sharing now</dt><dd>{w.live ? 'A connected device lists this community' : 'No connected device lists this community'}</dd></div>
    {#if w.remembered}<div><dt>Remembered member nodes</dt><dd>{w.remembered} seen in the last 30 days; they may be offline</dd></div>{/if}
    {#if w.carried}<div><dt>Shared from here</dt><dd>{w.carried === 'index' ? 'Because it is listed in a registry' : 'At someone’s request'}</dd></div>{/if}
  </dl>
  <details class="more counting">
    <summary>How are copies counted?</summary>
    <p class="help">These numbers come from this device’s connections, not a check of the whole network. Records include posts, replies and community details. A device with half the records counts as half a copy.</p>
    <p class="help">When another device connects to us, we count the records exchanged with it. When we connect to a node at a previously accepted address, we also count records it lists that we already have. That list is the node’s claim, not an independent check of its storage.</p>
    <p class="help">Nodes in the same network, or run by the same operator as their join notices say, count as one place: one outage or one person could take them together. Ten nodes in one place keep a community about as well as one.</p>
    <p class="help">A copy reached over a link that is not encrypted, or whose other end this device never checked, counts for less: someone in between could read or cut that link.</p>
    <p class="help">Nodes may also report copies they have seen elsewhere. We show the highest report separately, without adding reports together: they may refer to the same copies. These reports do not raise the warmth score.</p>
    <p class="help">Remembered nodes are saved connection addresses from the last 30 days, not a count of nodes online now.</p>
  </details>
{/snippet}

{#if compact}
  <span class={['tag', words.tag]} role="status" title={words.summary}>● warmth · {words.word}</span>
{:else if fold}
  <details class="warmth fold">
    <summary>
      <span class="line">
        <span class={['tag', words.tag]}>● warmth · {words.word}</span>
        <span class="meter" role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow={w.score} aria-label="Warmth, 0 to 100"><i style:width={`${Math.max(2, w.score)}%`}></i></span>
        <span class="num">{w.score}</span>
        <span class="why" aria-hidden="true">Details</span><span class="sr-only">, show why</span>
      </span>
      <span class="summary">{words.summary}</span>
    </summary>
    <div class="body">
      {#if words.reasons.length}<ul class="reasons help">{#each words.reasons as reason}<li>{reason}</li>{/each}</ul>{/if}
      {#if words.advice}<p class="help advice">{words.advice}</p>{/if}
      {@render children?.()}
      {@render numbers()}
    </div>
  </details>
{:else}
  <div class="warmth" role="status" aria-label={`Warmth: ${words.word}. ${words.summary}`}>
    <div class="line">
      <span class={['tag', words.tag]}>● warmth · {words.word}</span>
      <span class="meter" role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow={w.score} aria-label="Warmth, 0 to 100"><i style:width={`${Math.max(2, w.score)}%`}></i></span>
      <span class="num">{w.score}</span>
    </div>
    <p class="summary">{words.summary}</p>
    {#if words.reasons.length}<ul class="reasons help">{#each words.reasons as reason}<li>{reason}</li>{/each}</ul>{/if}
    {#if words.advice}<p class="help advice">{words.advice}</p>{/if}
    {#if details}
      <details class="more">
        <summary>Availability details</summary>
        {@render numbers()}
      </details>
    {/if}
  </div>
{/if}

<style>
  .warmth { display: flex; flex-direction: column; gap: 6px; min-width: 0; max-width: 65ch; }
  .line { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .meter { flex: 1 1 120px; max-width: 220px; height: 10px; border-radius: 999px; border: 2px solid var(--ink); background: var(--soft); overflow: hidden; }
  .meter i { display: block; height: 100%; background: var(--ink); border-radius: 999px; }
  .num { font-weight: 800; font-variant-numeric: tabular-nums; font-size: 13px; }
  .summary { font-size: 14px; font-weight: 600; line-height: 1.45; }
  .advice { color: var(--ink); }
  .reasons { margin: 0; padding-left: 18px; display: grid; gap: 4px; }
  .counting { margin-top: 6px; }
  .counting p + p { margin-top: 8px; }
  .more { font-size: 13px; }
  .more summary { cursor: pointer; font-weight: 700; color: var(--cobalt); min-height: 32px; }
  dl { margin: 8px 0 0; display: flex; flex-direction: column; gap: 6px; }
  dl div { display: grid; grid-template-columns: 175px minmax(0, 1fr); gap: 12px; font-size: 13px; }
  dt { color: var(--mute); }
  dd { margin: 0; overflow-wrap: anywhere; }
  .fold { gap: 0; max-width: 640px; border: 2px dashed var(--line); border-radius: var(--radius-input); }
  .fold > summary { list-style: none; cursor: pointer; display: flex; flex-direction: column; gap: 4px; padding: 10px 14px; border-radius: inherit; }
  .fold > summary::-webkit-details-marker { display: none; }
  .fold > summary:focus-visible { outline: 3px solid var(--cobalt); outline-offset: 2px; }
  .fold .summary { font-size: 13px; font-weight: 600; color: var(--mute); }
  .why { margin-left: auto; font-size: 12px; font-weight: 800; color: var(--cobalt); }
  .fold[open] .why { visibility: hidden; }
  .fold .body { display: flex; flex-direction: column; gap: 8px; padding: 0 14px 12px; font-size: 13px; }
  .fold .body dl { margin: 0; }
  @media (max-width: 520px) { dl div { grid-template-columns: 1fr; gap: 2px; } }
</style>
