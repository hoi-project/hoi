<script lang="ts">
  // "If you lose everything: 3 of 5 friends", rehearsed. Your key's picture is split
  // into five pieces with real Shamir maths (lib/sayhoi/shamir.ts), but over the PUBLIC key, so
  // nothing secret is in play: hand the pieces out, then ask any three back and watch the
  // picture come together. Fewer than three rebuild a stranger's. Drag a piece onto a friend, or
  // tap a piece and then a friend.
  import { createSpring } from 'animejs';
  import { animate, reduced } from '../motion';
  import { combine, split, type Share } from '../sayhoi/shamir';
  import { hexToBytes } from '../util';
  import Identicon from './Identicon.svelte';

  let { keyHex, platform }: { keyHex: string; platform: string } = $props();

  const FRIENDS = ['mira', 'jonas', 'sami', 'ida', 'kai'];
  const COLORS = ['mint', 'sky', 'peach', 'sun', 'lilac'];
  const key = $derived(hexToBytes(keyHex));
  const shares: Share[] = $derived(split(key, 5, 3));

  /** friend index → piece index */
  let given = $state<(number | null)[]>([null, null, null, null, null]);
  let picked = $state<number | null>(null);
  let asked = $state<boolean[]>([false, false, false, false, false]);
  let live = $state('');

  const handedOut = $derived(given.every((g) => g !== null));
  const loose = $derived([0, 1, 2, 3, 4].filter((p) => !given.includes(p)));
  const back = $derived(asked.flatMap((a, f) => (a && given[f] !== null ? [shares[given[f]!]] : [])));
  const rebuilt = $derived(back.length ? combine(back) : null);
  const whole = $derived(!!rebuilt && rebuilt.every((b, i) => b === key[i]));

  function give(piece: number, friend: number) {
    if (given[friend] !== null) return;
    given[friend] = piece;
    picked = null;
    live = `${FRIENDS[friend]} keeps piece ${piece + 1}.`;
    if (given.every((g) => g !== null)) live += ` All five handed out. Now say this ${platform} is gone: ask any three friends for their piece.`;
  }

  function handOut() {
    const left = [0, 1, 2, 3, 4].filter((p) => !given.includes(p));
    for (let f = 0; f < 5; f++) if (given[f] === null) give(left.shift()!, f);
  }

  function friendClick(f: number) {
    if (!handedOut) {
      if (picked !== null) give(picked, f);
      return;
    }
    asked[f] = !asked[f];
    const n = asked.filter(Boolean).length;
    live = n >= 3 ? `${n} pieces: that's your key again.` : n ? `${n} ${n === 1 ? 'piece' : 'pieces'}: a stranger's picture, nothing of yours.` : '';
  }

  // ------------------------------------------------------------------ dragging a piece

  let drag: { piece: number; el: HTMLElement; id: number; x0: number; y0: number; moved: boolean } | null = null;
  /** A drag ends in a click too; that one isn't a pick. */
  let droppedAt = 0;

  function down(e: PointerEvent, piece: number) {
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    drag = { piece, el, id: e.pointerId, x0: e.clientX, y0: e.clientY, moved: false };
  }

  function move(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.id) return;
    const dx = e.clientX - drag.x0;
    const dy = e.clientY - drag.y0;
    if (Math.hypot(dx, dy) > 6) drag.moved = true;
    if (drag.moved) drag.el.style.transform = `translate(${dx}px, ${dy}px) rotate(${dx / 20}deg)`;
  }

  function up(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.id) return;
    const d = drag;
    drag = null;
    if (!d.moved) return;
    droppedAt = performance.now();
    const over = document.elementsFromPoint(e.clientX, e.clientY).find((n) => n instanceof HTMLElement && n.dataset.friend);
    const f = over instanceof HTMLElement ? Number(over.dataset.friend) : -1;
    if (f >= 0 && given[f] === null) {
      d.el.style.transform = '';
      give(d.piece, f);
      return;
    }
    // not on a free friend: it springs back
    if (reduced()) d.el.style.transform = '';
    else animate(d.el, { translateX: 0, translateY: 0, rotate: 0, ease: createSpring({ stiffness: 140, damping: 10 }), onComplete: () => (d.el.style.transform = '') });
  }

  function pick(piece: number) {
    if (performance.now() - droppedAt < 400) return;
    picked = picked === piece ? null : piece;
  }
</script>

<div class="shards">
  <ol class="friends" aria-label="Five friends">
    {#each FRIENDS as name, f (name)}
      {@const piece = given[f]}
      <li>
        <button
          type="button"
          class={['friend', piece !== null && 'has', asked[f] && 'asked', !handedOut && picked !== null && piece === null && 'open']}
          data-friend={f}
          aria-pressed={handedOut ? asked[f] : undefined}
          disabled={!handedOut && (piece !== null || picked === null)}
          onclick={() => friendClick(f)}
        >
          <span class="av bg-{COLORS[f]}" aria-hidden="true">{name.slice(0, 2).toUpperCase()}</span>
          <span class="who">{name}</span>
          {#if piece !== null}<span class="badge bg-{COLORS[piece]}" aria-hidden="true">{piece + 1}</span>{/if}
          <span class="sr-only">
            {#if !handedOut}{piece === null ? `Give ${name} the picked piece` : `${name} keeps piece ${piece + 1}`}{:else}Ask {name} for their piece{/if}
          </span>
        </button>
      </li>
    {/each}
  </ol>

  {#if !handedOut}
    <div class="pieces" role="group" aria-label="Five pieces of your key's picture">
      {#each loose as p (p)}
        <button
          type="button"
          class={['piece', `bg-${COLORS[p]}`, picked === p && 'on']}
          aria-pressed={picked === p}
          onpointerdown={(e) => down(e, p)}
          onpointermove={move}
          onpointerup={up}
          onpointercancel={up}
          onclick={() => pick(p)}
        >
          piece {p + 1}
        </button>
      {/each}
    </div>
    <div class="btnrow">
      <button type="button" class="act" onclick={handOut}>Hand them out for me</button>
    </div>
  {:else}
    <div class="rebuilt" aria-hidden="true">
      {#if !rebuilt}
        <span class="ghost">?</span>
      {:else}
        <Identicon bytes={rebuilt} size={96} dim={!whole} />
      {/if}
    </div>
    <p class={['verdict', whole && 'yes']}>
      {#if !rebuilt}
        Tap any three friends.
      {:else if whole}
        That's your key again. Any three would have done.
      {:else}
        {back.length === 1 ? 'One piece gives' : 'Two pieces give'} a stranger's picture. Fewer than three say nothing about your key.
      {/if}
    </p>
  {/if}
  <p class="sr-only" role="status" aria-live="polite">{live}</p>
</div>

<style>
  .shards { display: flex; flex-direction: column; gap: 14px; }
  .friends { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(5, minmax(0, 1fr)); gap: 8px; }
  .friend {
    position: relative; width: 100%; min-height: 76px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 4px;
    border: 2px dashed var(--line); border-radius: 18px; background: transparent; color: var(--ink); font-weight: 700; font-size: 13px; padding: 8px 4px;
  }
  .friend:disabled { opacity: 1; cursor: default; }
  .friend.open { border-color: var(--cobalt); border-style: solid; }
  .friend.has { border-style: solid; border-color: var(--ink); }
  .friend.asked { background: color-mix(in srgb, var(--cobalt) 18%, transparent); border-color: var(--cobalt); }
  .who { max-width: 100%; overflow: hidden; text-overflow: ellipsis; }
  .badge {
    position: absolute; top: -9px; right: -6px; width: 24px; height: 24px; border-radius: 50%; border: 2px solid var(--ink); color: #1a1a1a;
    display: inline-flex; align-items: center; justify-content: center; font-size: 12px; font-weight: 900;
  }
  .pieces { display: flex; gap: 10px; flex-wrap: wrap; min-height: 48px; }
  .piece {
    min-height: 44px; padding: 0 16px; border-radius: 14px 14px 14px 4px; border: 2px solid var(--ink); color: #1a1a1a; font-weight: 800;
    box-shadow: 3px 3px 0 var(--ink); touch-action: none; user-select: none; -webkit-user-select: none; cursor: grab; position: relative; z-index: 1;
  }
  .piece:active { cursor: grabbing; }
  .piece.on { outline: 3px solid var(--cobalt); outline-offset: 2px; }
  .rebuilt { display: flex; justify-content: center; min-height: 100px; align-items: center; }
  .ghost {
    width: 96px; height: 96px; border-radius: 50%; border: 3px dashed var(--line); display: inline-flex; align-items: center; justify-content: center;
    font-weight: 900; font-size: 32px; color: var(--mute);
  }
  .verdict { text-align: center; font-weight: 700; min-height: 2.9em; }
  .verdict.yes { color: var(--cobalt); }
  @media (max-width: 420px) {
    .friends { gap: 5px; }
    .friend { min-height: 70px; font-size: 12px; }
  }
</style>
