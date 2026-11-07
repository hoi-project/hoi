<script lang="ts">
  // "Say hoi": making a key, acted out. The game shows what really happens and gives
  // you something to do during the few seconds of work a key needs anyway; your score doesn't
  // make the key safe, the system's randomness does.
  //   1. Stir the swarm: the field is a touch pad; your moves and their timing are folded into extra
  //      randomness (lib/sayhoi/stir.ts) that the core mixes into its own. Skippable: the key is
  //      safe without it.
  //   2. Earn your stamp: the key's identity work (Argon2id, PROTOCOL.md §4.1), mined by the
  //      page's workers; every try flies to the bouncer, the one with enough zero bits gets through.
  //   3. Meet your key: its did:key, fingerprint and picture, then an optional rehearsal of
  //      getting back in with 3 of 5 friends.
  import type { IdentityDraft, IdentityTry, Me } from '@hoi/core-api';
  import { onMount, tick } from 'svelte';
  import { app } from '../app.svelte';
  import { animate, EASE, reduced } from '../motion';
  import { Stir } from '../sayhoi/stir';
  import { Swarm, type Arrival } from '../sayhoi/swarm';
  import { toastError } from '../toast.svelte';
  import { copyText } from '../util';
  import Identicon from './Identicon.svelte';
  import KeyBackup from './KeyBackup.svelte';
  import Modal from './Modal.svelte';
  import Shards from './Shards.svelte';

  let { name, next, ondone, oncancel }: { name: string; next: string; ondone: (me: Me) => void; oncancel: () => void } = $props();

  type Act = 'stir' | 'work' | 'meet' | 'shards';
  const still = reduced();
  const stir = new Stir();
  const BITS_SHOWN = 16;

  let act = $state<Act>('stir');
  let filled = $state(0);
  let draft = $state.raw<IdentityDraft | null>(null);
  let me = $state.raw<Me | null>(null);
  let tries = $state(0);
  let best = $state(0);
  let latest = $state.raw<Arrival | null>(null);
  /** The miners report their tries (the browser does; the desktop mines natively, too fast to show). */
  let seen = $state(false);
  let stamped = $state(false);
  let need = $state(10);
  let live = $state('');
  /** The backup modal, offered as soon as the key exists: the best moment to save it. */
  let backupOpen = $state(false);

  let dlg: HTMLDialogElement | undefined = $state();
  let canvas: HTMLCanvasElement | undefined = $state();
  let heading: HTMLElement | undefined = $state();
  let stampEl: HTMLElement | undefined = $state();
  let meetBtn: HTMLButtonElement | undefined = $state();
  let swarm: Swarm | null = null;
  let cursorAt: { x: number; y: number } | null = null;
  let winnerLanded = false;
  let advancing = false;

  const platform = $derived(app.platformName);

  onMount(() => {
    dlg?.showModal();
    if (canvas) swarm = new Swarm(canvas, { reduced: still, onArrive });
    // Without SHA-256 (an insecure context) there is nothing to stir with: straight to the work.
    if (!Stir.supported()) void begin(false);
    return () => {
      swarm?.destroy();
      stir.wipe();
      cancelAnimationFrame(flushRaf);
    };
  });

  // ------------------------------------------------------------------ act 1: stir

  /** A light buzz on phones that have one, for the moments that land. */
  function buzz(ms: number) {
    if (still) return;
    try {
      navigator.vibrate?.(ms);
    } catch {
      /* not here */
    }
  }

  function at(e: PointerEvent) {
    const r = canvas!.getBoundingClientRect();
    const hand = e.pointerType === 'touch' || e.pointerType === 'pen' ? e.pointerType : 'mouse';
    return { x: e.clientX - r.left, y: e.clientY - r.top, pressed: e.buttons > 0 || hand === 'touch', hand } as const;
  }

  function pointerMove(e: PointerEvent) {
    swarm?.touch(at(e));
    if (act !== 'stir') return;
    const r = canvas!.getBoundingClientRect();
    let got = 0;
    for (const c of e.getCoalescedEvents?.() ?? [e]) got += stir.move(c.clientX - r.left, c.clientY - r.top, c.pressure, c.timeStamp || performance.now());
    feed(got);
  }

  function pointerDown(e: PointerEvent) {
    const p = at(e);
    swarm?.touch(p);
    swarm?.poke(p.x, p.y);
    if (act === 'stir') feed(stir.down(p.x, p.y, e.pressure));
  }

  function pointerLeave() {
    swarm?.touch(null);
  }

  const MOVES: Record<string, [number, number]> = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };

  function key(e: KeyboardEvent) {
    if (!swarm || e.key === 'Tab' || e.key === 'Escape' || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key.length > 1 && !(e.key in MOVES) && e.key !== 'Enter') return;
    e.preventDefault();
    const { w, h } = swarm.size;
    const c = (cursorAt ??= { x: w / 2, y: h / 2 });
    const step = e.shiftKey ? 44 : 22;
    const m = MOVES[e.key];
    if (m) {
      c.x = Math.min(w - 8, Math.max(8, c.x + m[0] * step));
      c.y = Math.min(h - 8, Math.max(8, c.y + m[1] * step));
    } else swarm.poke(c.x, c.y);
    swarm.cursor({ ...c });
    // which key and exactly when go into the hash; nothing typed is kept
    if (act === 'stir') feed(stir.key(e.key.charCodeAt(0) ^ (e.key.length << 8), e.repeat));
  }

  function fieldFocus(e: FocusEvent) {
    const el = e.currentTarget as HTMLElement;
    if (!swarm || !el.matches(':focus-visible')) return;
    const { w, h } = swarm.size;
    cursorAt ??= { x: w / 2, y: h / 2 };
    swarm.cursor({ ...cursorAt });
  }

  /** The ring fills as you move. Full, the key is made. */
  function feed(points: number) {
    if (!points || act !== 'stir') return;
    const f = stir.filled;
    const quarter = Math.floor(f * 4);
    if (quarter > Math.floor(filled * 4) && quarter < 4) live = `Stirred ${quarter * 25}%.`;
    filled = f;
    if (f >= 1 && !advancing) {
      advancing = true;
      live = 'Stirred. Making your key…';
      buzz(20);
      setTimeout(() => void begin(true), still ? 0 : 450);
    }
  }

  // ------------------------------------------------------------------ act 2: the work

  let flushRaf = 0;
  let pending: { tries: number; best: number; latest: Arrival | null } = { tries: 0, best: 0, latest: null };
  /** Tries arrive hundreds a second; the screen takes them once a frame. */
  function flush() {
    if (flushRaf) return;
    flushRaf = requestAnimationFrame(() => {
      flushRaf = 0;
      tries = pending.tries;
      best = pending.best;
      if (pending.latest) latest = pending.latest;
    });
  }

  function onTry(t: IdentityTry) {
    seen = true;
    pending.tries++;
    pending.best = Math.max(pending.best, t.bits);
    flush();
    swarm?.launch(t);
  }

  function onArrive(a: Arrival) {
    // once the winner is through, its hash stays on the strip; stragglers still bounce
    if (winnerLanded && !a.passed) return;
    pending.latest = a;
    flush();
    if (a.passed) {
      winnerLanded = true;
      land();
    }
  }

  async function begin(useStir: boolean) {
    if (act !== 'stir' || left) return;
    act = 'work';
    let extra: Uint8Array | undefined;
    if (useStir) {
      try {
        extra = await stir.take();
      } catch {
        extra = undefined; // the key is just as safe from the system's randomness alone
      }
    }
    stir.wipe();
    swarm?.setMode('work', need);
    live = `Making your key. Your ${platform} is doing a few seconds of work, once.`;
    await tick();
    dlg?.scrollTo({ top: 0 });
    heading?.focus({ preventScroll: true });
    try {
      me = await app.core.createIdentity(name, {
        stir: extra,
        onKey: (d) => {
          draft = d;
          need = d.bits;
          swarm?.setMode('work', d.bits);
        },
        onTry,
      });
      // No flight to wait for (mined natively, or reduced motion): stamp now. Otherwise the
      // winning try is still flying; give it a moment, but never wait on it for good.
      if (!seen || winnerLanded) land();
      else setTimeout(land, 1500);
    } catch (e) {
      toastError(e);
      leave();
    } finally {
      extra?.fill(0);
    }
  }

  function land() {
    if (stamped || !me) return;
    stamped = true;
    buzz(30);
    live = `Stamped: ${me.identityBits} bits of identity work. Your key is ready.`;
    if (!seen) swarm?.launch({ work: 0, hash: new Uint8Array(32), bits: need });
    void tick().then(() => {
      if (stampEl && !still) animate(stampEl, { scale: [3, 1], rotate: [-30, -8], opacity: [0, 1], duration: 650, ease: EASE.stamp });
      meetBtn?.focus({ preventScroll: true });
    });
  }

  /** The first bits of the latest try that reached the bouncer: its leading zeros light up, the
   * first one bit (where it fell short) is ringed. */
  const cells = $derived.by(() =>
    Array.from({ length: BITS_SHOWN }, (_, i) => ({ zero: !!latest && i < latest.bits, miss: !!latest && i === latest.bits })),
  );
  const hex = $derived(latest ? Array.from(latest.hash.slice(0, 4), (b) => b.toString(16).padStart(2, '0')).join('') + '…' : '');

  // ------------------------------------------------------------------ act 3: you

  async function meet() {
    act = 'meet';
    swarm?.setMode('done', need);
    await showHeading();
  }

  /** Brings the new act's heading to the top (on phones the field sits above it) and focuses it. */
  async function showHeading() {
    await tick();
    heading?.scrollIntoView({ block: 'start' });
    heading?.focus({ preventScroll: true });
  }

  /** The did:key settles out of noise, left to right. */
  function resolve(node: HTMLElement, text: string) {
    if (still) {
      node.textContent = text;
      return;
    }
    const B58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
    const t0 = performance.now();
    let raf = 0;
    const step = () => {
      const k = Math.min(1, (performance.now() - t0) / 800);
      const n = Math.floor(text.length * k);
      node.textContent = text.slice(0, n) + Array.from(text.slice(n), (c, i) => (i === 0 || c === ':' ? c : B58[(Math.random() * 58) | 0])).join('');
      if (k < 1) raf = requestAnimationFrame(step);
    };
    step();
    return { destroy: () => cancelAnimationFrame(raf) };
  }

  function pop(node: Element) {
    if (!still) animate(node, { scale: [0.3, 1], rotate: [-25, 0], opacity: [0, 1], duration: 700, ease: 'outBack(2)' });
  }

  // Toasts sit under this dialog, so the button answers for itself.
  let copied = $state(false);
  async function copyDid() {
    if (!me) return;
    copied = await copyText(me.root);
    live = copied ? 'Copied your key.' : 'Copy did not work here. Select the key and copy it yourself.';
    if (copied) setTimeout(() => (copied = false), 1600);
  }

  async function rehearse() {
    act = 'shards';
    await showHeading();
  }

  // ------------------------------------------------------------------ leaving

  let left = false;

  function finish() {
    if (!me || left) return;
    left = true;
    dlg?.close();
    ondone(me);
  }

  function leave() {
    if (left) return;
    left = true;
    stir.wipe();
    dlg?.close();
    oncancel();
  }

  /** Browsers may close a dialog on a second Esc without asking: finish what can be finished. */
  function closed() {
    if (left) return;
    if (act === 'stir') leave();
    else if (me) finish();
    else dlg?.showModal(); // the work is half done: stay
  }

  function onEsc(e: Event) {
    // Act 1: not now. During the work: it's a few seconds, and the key is half made. After: the key is yours either way.
    e.preventDefault();
    if (act === 'stir') leave();
    else if (me && stamped) finish();
  }
</script>

<dialog class="sayhoi" bind:this={dlg} oncancel={onEsc} onclose={closed} aria-labelledby="sh-h" aria-describedby="sh-lead">
  <div class="wrap">
    <section class="copy">
      <span class="sticker">
        {#if act === 'stir'}1 of 3 · stir{:else if act === 'work'}2 of 3 · stamp{:else}3 of 3 · you{/if}
      </span>

      {#if act === 'stir'}
        <h2 id="sh-h" tabindex="-1" bind:this={heading}>Stir the swarm.</h2>
        <p id="sh-lead" class="lead">
          Run your finger or mouse through the dots, or tap them. Every move, and exactly when you make it, is mixed into your new key on
          top of this {platform}'s own randomness.
        </p>
        <p class="help">No mouse? Tab to the swarm, then press the arrow keys and Space. Nothing you do here is stored or sent anywhere.</p>
        <div class="btnrow">
          <button type="button" class={['pill', 'pill-lg', filled >= 1 ? 'pill-coral' : 'pill-ink']} onclick={() => begin(true)}>Make my key →</button>
          <button type="button" class="act" onclick={() => begin(false)}>Skip</button>
        </div>
        <button type="button" class="textlink back" onclick={leave}>← Not now</button>
      {:else if act === 'work'}
        <h2 id="sh-h" tabindex="-1" bind:this={heading}>Earn your stamp.</h2>
        <p id="sh-lead" class="lead">
          Your key does a few seconds of work, once. {still ? 'Each try is' : 'Each coral dot is one try:'} a memory-hungry hash of your key (Argon2id, 8 MiB a try,
          so a server farm gains little over a phone). The bouncer only lets a try through if its hash starts with {need} zero bits.
        </p>
        {#if draft}
          <p class="help">Your key: <span class="mono">{draft.short}</span> · about 1 in {(2 ** need).toLocaleString('en')} tries works.</p>
        {/if}
        <p class="help">
          Every post you write later carries a smaller stamp of the same kind: about a second of work, and every device checks it in a
          blink.
        </p>
        {#if stamped}
          <div class="btnrow">
            <button type="button" class="pill pill-coral pill-lg" bind:this={meetBtn} onclick={meet}>Meet your key →</button>
          </div>
        {/if}
      {:else if me && act === 'meet'}
        <h2 id="sh-h" tabindex="-1" bind:this={heading}>Meet your key, {me.name}.</h2>
        <div class="you">
          <span class="pic" use:pop><Identicon keyHex={me.rootKey} size={112} /></span>
          <div class="facts">
            <p class="did mono" aria-hidden="true" use:resolve={me.root}></p>
            <p class="sr-only">Your key: {me.root}</p>
            <button type="button" class="pill pill-sm" onclick={copyDid}>{copied ? 'Copied' : 'Copy'}</button>
          </div>
        </div>
        <p id="sh-lead" class="lead">
          That's you on hoi: a key made on this {platform}, with no email, no server and nobody to ask. Its picture is drawn from the key's
          own bytes. To check a key in person, compare its fingerprint:
        </p>
        <p class="fp mono">{me.fingerprint}</p>
        <p class="help">Identity work: {me.identityBits} bits, done for life. A new key would start from nothing, so keep a copy: one line for your password manager takes a few seconds.</p>
        <div class="btnrow">
          <button type="button" class="pill pill-coral pill-lg" onclick={finish}>{next}</button>
          <button type="button" class="pill pill-lg" onclick={() => (backupOpen = true)}>Save a recovery key</button>
          <button type="button" class="act" onclick={rehearse}>How do friends get me back in?</button>
        </div>
      {:else if me && act === 'shards'}
        <h2 id="sh-h" tabindex="-1" bind:this={heading}>If you lose everything: 3 of 5 friends.</h2>
        <p id="sh-lead" class="lead">
          Your key can be split into five pieces, one for each of five friends. Any three rebuild it; one or two say nothing about it. Hand
          them out: drag each piece onto a friend, or tap a piece, then a friend.
        </p>
        <Shards keyHex={me.rootKey} {platform} />
        <p class="help">This is a demonstration using your public key, not real recovery shares. Set up recovery with your chosen friends under You → Friends. Keep your own backup too: choose You → Back up my identity for a recovery key in your password manager or an encrypted file.</p>
        <div class="btnrow">
          <button type="button" class="pill pill-coral pill-lg" onclick={finish}>{next}</button>
          <button type="button" class="pill pill-lg" onclick={() => (backupOpen = true)}>Save a recovery key</button>
        </div>
      {/if}
    </section>

    <div class="stage">
      <!-- the field is dark whatever the page is: hoi's night sky (theme colour #15151A) -->
      <!-- A swarm you stir with the pointer or the arrow keys: role="application" tells screen
           readers to hand those keys to it. Svelte doesn't count it as interactive. -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <div
        class="swarm"
        data-theme="dark"
        role="application"
        aria-roledescription="swarm"
        aria-label={act === 'stir' ? 'Stir the swarm with the pointer, or the arrow keys and Space' : 'The swarm'}
        tabindex="0"
        onpointermove={pointerMove}
        onpointerdown={pointerDown}
        onpointerleave={pointerLeave}
        onpointerup={(e) => e.pointerType !== 'mouse' && pointerLeave()}
        onpointercancel={pointerLeave}
        onkeydown={key}
        onfocus={fieldFocus}
        onblur={() => swarm?.cursor(null)}
      >
        <canvas bind:this={canvas} aria-hidden="true"></canvas>

        {#if act === 'stir'}
          <div class="hud" role="progressbar" aria-label="Stirred" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(filled * 100)}>
            <svg viewBox="0 0 40 40" width="34" height="34" aria-hidden="true">
              <circle cx="20" cy="20" r="15" class="track" />
              <circle cx="20" cy="20" r="15" class="fill" pathLength="100" stroke-dasharray="{filled * 100} 100" />
            </svg>
            <span><b class="num">{Math.round(filled * 100)}%</b> stirred</span>
          </div>
        {:else}
          <div class="strip" aria-hidden="true">
            {#if seen}
              <!-- the latest try's first bits as dots: leading zeros in sun, the bit that spoilt it ringed in coral -->
              <div class="bitrow">
                {#key latest?.work}
                  <div class="bits">
                    {#each cells as c, i (i)}
                      <i class={['bit', c.zero && 'zero', c.miss && 'miss', i === need && 'cut']} style="--i:{i}"></i>
                    {/each}
                  </div>
                {/key}
                {#if hex}<span class="mono">{hex}</span>{/if}
              </div>
              <div class="tally">
                <span><b class="num">{tries.toLocaleString('en')}</b> {tries === 1 ? 'try' : 'tries'}</span>
                <span>best <b class="num">{best}</b> zeros</span>
                <span class="need">needs {need}</span>
              </div>
            {:else}
              <!-- the desktop mines natively and reports no tries; a browser's first try is a moment away -->
              <div class="tally"><span>{stamped ? 'Done on every core.' : platform === 'computer' ? 'Your computer is doing it on every core…' : 'Warming up…'}</span></div>
            {/if}
          </div>
          {#if stamped}
            <span class="stamp" bind:this={stampEl}>identity · {me?.identityBits ?? need} zero bits</span>
          {/if}
        {/if}
      </div>
    </div>
  </div>
  <p class="sr-only" role="status" aria-live="polite">{live}</p>
</dialog>

{#if me}
  <Modal bind:open={backupOpen} title="Back up my identity">
    <KeyBackup {me} ondone={() => (backupOpen = false)} />
  </Modal>
{/if}

<style>
  .sayhoi {
    width: 100vw; height: 100dvh; max-width: none; max-height: none; margin: 0; padding: 0; border: 0;
    background: var(--bg); color: var(--ink); overflow: auto; overscroll-behavior: contain;
  }
  .sayhoi::backdrop { background: var(--bg); }
  .wrap {
    max-width: 1120px; min-height: 100%; margin: 0 auto; display: grid; grid-template-columns: minmax(0, 5fr) minmax(0, 7fr); gap: 32px; align-items: center;
    padding: max(24px, env(safe-area-inset-top, 0px)) max(16px, env(safe-area-inset-right, 0px)) max(24px, env(safe-area-inset-bottom, 0px)) max(16px, env(safe-area-inset-left, 0px));
  }
  .copy { display: flex; flex-direction: column; gap: 14px; align-items: flex-start; min-width: 0; }
  h2 { font-size: var(--type-h2-size); line-height: var(--type-h2-lh); letter-spacing: var(--type-h2-tracking); outline: none; }
  .lead { font-size: var(--type-lead-size); line-height: var(--type-lead-lh); color: var(--mute); }
  .copy .help { max-width: 34em; }
  .back { margin-top: 4px; }
  .you { display: flex; gap: 16px; align-items: center; min-width: 0; width: 100%; }
  .pic { display: block; flex: 0 0 auto; }
  .facts { display: flex; flex-direction: column; gap: 8px; align-items: flex-start; min-width: 0; }
  .did { font-size: 13px; color: var(--ink); word-break: break-all; line-height: 1.4; }
  .fp { font-size: 15px; color: var(--ink); letter-spacing: .02em; }

  .stage { background: var(--ink); border: 2px solid var(--ink); border-radius: var(--radius-frame); overflow: hidden; box-shadow: 6px 6px 0 var(--ink); min-width: 0; }
  .swarm { position: relative; aspect-ratio: 4 / 3; background: var(--bg); color: var(--ink); touch-action: none; user-select: none; -webkit-user-select: none; cursor: crosshair; outline: none; }
  .swarm:focus-visible { box-shadow: inset 0 0 0 3px var(--cobalt); }
  canvas { position: absolute; inset: 0; width: 100%; height: 100%; display: block; }

  .hud {
    position: absolute; top: 12px; left: 12px; display: flex; align-items: center; gap: 8px; padding: 4px 14px 4px 4px; border-radius: 999px;
    background: var(--card); border: 2px solid var(--line); pointer-events: none; font-size: 13px; font-weight: 700; color: var(--mute);
  }
  .hud b { color: var(--ink); font-weight: 900; }
  .hud svg { transform: rotate(-90deg); flex: 0 0 auto; }
  .hud circle { fill: none; stroke-width: 5; }
  .hud .track { stroke: var(--line); }
  .hud .fill { stroke: var(--coral); stroke-linecap: round; transition: stroke-dasharray .12s linear; }

  .strip {
    position: absolute; left: 12px; right: 12px; bottom: 12px; display: flex; flex-direction: column; gap: 8px; padding: 10px 14px;
    border-radius: 18px; background: var(--card); border: 2px solid var(--line); pointer-events: none;
  }
  .bitrow { display: flex; align-items: center; gap: 10px; min-width: 0; }
  .bits { display: flex; gap: 5px; align-items: center; flex: 0 0 auto; }
  .bit { width: 13px; height: 13px; border-radius: 50%; border: 2px solid var(--line); animation: reveal .2s calc(var(--i) * 20ms) both; }
  .bit.cut { margin-left: 7px; }
  .bit.zero { background: var(--sun); border-color: var(--sun); }
  .bit.miss { border-color: var(--coral); background: color-mix(in srgb, var(--coral) 22%, transparent); }
  @keyframes reveal { from { opacity: 0; transform: scale(0.3); } }
  .bitrow .mono { margin-left: auto; white-space: nowrap; }
  .tally { display: flex; gap: 4px 16px; flex-wrap: wrap; font-size: 13px; font-weight: 700; color: var(--mute); }
  .tally b { color: var(--ink); font-weight: 900; }
  .tally .need { margin-left: auto; }

  .stamp {
    position: absolute; top: 16px; right: 16px; padding: 7px 12px; border-radius: 10px; border: 3px solid var(--cobalt); color: var(--cobalt);
    background: var(--card); font-weight: 900; font-size: 13px; letter-spacing: .06em; text-transform: uppercase; transform: rotate(-8deg); pointer-events: none;
  }

  @media (prefers-reduced-motion: reduce) { .bit { animation: none; } .hud .fill { transition: none; } }
  :global([data-motion='reduced']) .bit { animation: none; }
  @media (max-width: 860px) {
    .wrap { grid-template-columns: 1fr; align-content: start; gap: 20px; }
    .stage { order: -1; }
    .swarm { aspect-ratio: auto; height: min(48dvh, 420px); }
    h2 { font-size: clamp(30px, 8vw, 44px); }
  }
  @media (max-width: 420px) {
    .bit { width: 10px; height: 10px; }
    .bits { gap: 4px; }
    .bit.cut { margin-left: 5px; }
    .you { gap: 12px; }
  }
</style>
