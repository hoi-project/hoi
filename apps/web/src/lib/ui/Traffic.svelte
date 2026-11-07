<script lang="ts">
  // What passes through this device: amounts and times, never content (the core counts bytes
  // and records per connection and per minute, in memory only). Polls while it's on screen.
  import type { Traffic } from '@hoi/core-api';
  import { ago, bytes } from '@hoi/core-api';
  import { onMount } from 'svelte';
  import { app } from '../app.svelte';

  let { compact = false }: { compact?: boolean } = $props();

  let t = $state.raw<Traffic | null>(null);
  let focus = $state<number | null>(null);
  let showTable = $state(false);
  let unavailable = $state(false);

  onMount(() => {
    let alive = true;
    const load = async () => {
      if (document.visibilityState !== 'visible') return;
      try {
        const next = await app.core.traffic();
        if (alive) { t = next; unavailable = false; }
      } catch {
        if (alive) unavailable = true;
      }
    };
    void load();
    const timer = setInterval(load, 2000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  });

  // ---------------------------------------------------------------- chart geometry
  // Drawn at its real width, so columns keep their 2px gaps and round caps on any screen.
  let width = $state(600);
  const W = $derived(Math.max(120, width));
  const H = 160;
  const MID = H / 2;
  const SLOT = $derived(W / 60);
  const BAR = $derived(Math.max(1, Math.min(SLOT - 2, 24)));

  const peak = $derived(Math.max(1, ...(t?.minutes ?? []).flatMap((m) => [m.bytesIn, m.bytesOut])));
  /** A clean top for the axis, in the units its label shows (KB, MB: powers of 1024). */
  const top = $derived.by(() => {
    const unit = 1024 ** Math.min(3, Math.max(0, Math.floor(Math.log(peak) / Math.log(1024))));
    const v = peak / unit;
    const p = 10 ** Math.floor(Math.log10(v));
    for (const f of [1, 1.5, 2, 3, 4, 5, 6, 8, 10]) if (f * p >= v) return f * p * unit;
    return 10 * p * unit;
  });
  const h = (v: number) => (v / top) * (MID - 6);
  const quiet = $derived(!!t && t.minutes.every((m) => !m.bytesIn && !m.bytesOut));

  /** A column with a 4px round cap away from the baseline, square at it. */
  function column(x: number, height: number, up: boolean): string {
    if (height <= 0) return '';
    const r = Math.min(4, height, BAR / 2);
    const y0 = MID + (up ? -1 : 1);
    const y1 = up ? y0 - height : y0 + height;
    const dir = up ? 1 : -1;
    return [
      `M${x},${y0}`,
      `V${y1 + dir * r}`,
      `Q${x},${y1} ${x + r},${y1}`,
      `H${x + BAR - r}`,
      `Q${x + BAR},${y1} ${x + BAR},${y1 + dir * r}`,
      `V${y0}`,
      'Z',
    ].join(' ');
  }

  const clock = (at: number) => new Date(at * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });

  function pick(e: PointerEvent) {
    const svg = e.currentTarget as SVGSVGElement;
    const box = svg.getBoundingClientRect();
    focus = Math.max(0, Math.min(59, Math.floor(((e.clientX - box.left) / box.width) * 60)));
  }

  function keys(e: KeyboardEvent) {
    if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
      e.preventDefault();
      focus = Math.max(0, Math.min(59, (focus ?? 59) + (e.key === 'ArrowLeft' ? -1 : 1)));
    } else if (e.key === 'Home' || e.key === 'End') {
      e.preventDefault();
      focus = e.key === 'Home' ? 0 : 59;
    }
  }

  const focused = $derived(t && focus !== null ? t.minutes[focus] : null);

  function readout(i: number): string {
    const m = t?.minutes[i];
    return m ? `${clock(m.at)}: ${bytes(m.bytesOut)} sent, ${bytes(m.bytesIn)} received` : 'no data yet';
  }

  // ---------------------------------------------------------------- connections in words
  function who(c: Traffic['connections'][number]): string {
    if (c.role === 'home') return c.name ? `Your home node · ${c.name}` : 'Your home node';
    if (c.role === 'client') return c.name ? `Your device · ${c.name}` : 'One of your devices';
    return c.transport === 'webrtc' ? "A member's browser" : "A member's computer";
  }
  const lockClass = (e: string) => (e.startsWith('encrypted') ? 'tag-mint' : e.startsWith('stays') ? 'tag-sky' : 'tag-peach');
  const relayShare = $derived(t ? Math.min(1, t.relay.todayBytes / Math.max(1, t.relay.capBytes)) : 0);
</script>

<div class={['traffic', compact && 'compact']}>
  {#if unavailable}<p class="callout" role="status">Activity is temporarily unavailable. {t ? 'Showing the last received counts.' : 'Waiting for this device to respond.'} Retrying automatically.</p>{/if}
  <div class="tiles" aria-label="Activity totals since starting hoi">
    <div class="tile"><span class="lbl">Received</span><b>{t ? bytes(t.bytesIn) : '—'}</b></div>
    <div class="tile"><span class="lbl">Sent</span><b>{t ? bytes(t.bytesOut) : '—'}</b></div>
    <div class="tile"><span class="lbl">Records checked</span><b>{t ? t.recordsIn.toLocaleString('en') : '—'}</b></div>
    <div class="tile"><span class="lbl">Refused here</span><b>{t ? t.refused.toLocaleString('en') : '—'}</b></div>
  </div>

  <section class="history" aria-label="Activity during the last hour">
  <figure class="chart">
    <figcaption>
      <span class="ttl">The last hour</span>
      <span class="legend" aria-hidden="true"><i class="key out"></i>Sent <i class="key in"></i>Received</span>
    </figcaption>
    <div class="plot" bind:clientWidth={width}>
      <svg
        viewBox="0 0 {W} {H}"
        role="slider"
        aria-label="Bytes sent and received per minute over the last hour. Arrow keys read each minute."
        aria-valuemin={0}
        aria-valuemax={59}
        aria-valuenow={focus ?? 59}
        aria-valuetext={readout(focus ?? 59)}
        tabindex="0"
        onpointermove={pick}
        onpointerleave={() => (focus = null)}
        onkeydown={keys}
        onblur={() => (focus = null)}
      >
        <line class="base" x1="0" x2={W} y1={MID} y2={MID} />
        <line class="grid" x1="0" x2={W} y1={MID - h(top)} y2={MID - h(top)} />
        <line class="grid" x1="0" x2={W} y1={MID + h(top)} y2={MID + h(top)} />
        {#each t?.minutes ?? [] as m, i (m.at)}
          {@const x = i * SLOT + (SLOT - BAR) / 2}
          {#if focus === i}<rect class="hover" x={i * SLOT} y="0" width={SLOT} height={H} />{/if}
          <path class="out" d={column(x, h(m.bytesOut), true)} />
          <path class="in" d={column(x, h(m.bytesIn), false)} />
        {/each}
      </svg>
      {#if !quiet}
        <span class="axis y top">{bytes(top)} sent</span>
        <span class="axis y bottom">{bytes(top)} received</span>
      {/if}
      <span class="axis x left">60 min ago</span>
      <span class="axis x right">now</span>
      {#if focused}
        <div class="tip" style:left={`clamp(72px, ${(((focus ?? 0) + 0.5) / 60) * 100}%, calc(100% - 72px))`} role="status">
          <span class="when">{clock(focused.at)}</span>
          <span><i class="key out"></i><b>{bytes(focused.bytesOut)}</b> sent</span>
          <span><i class="key in"></i><b>{bytes(focused.bytesIn)}</b> received</span>
        </div>
      {/if}
    </div>
    {#if quiet}<p class="help">Nothing passed through this {app.platformName} in the last hour.</p>{/if}
  </figure>

  <button type="button" class="pill pill-sm pill-sun tablebtn" aria-expanded={showTable} onclick={() => (showTable = !showTable)}>{showTable ? 'Hide the numbers' : 'Show the numbers'}</button>
  {#if showTable && t}
    <div class="tbl mins">
      <table>
        <thead><tr><th>Minute</th><th>Sent</th><th>Received</th></tr></thead>
        <tbody>
          {#each t.minutes.filter((m) => m.bytesIn || m.bytesOut).reverse() as m (m.at)}
            <tr><td class="num">{clock(m.at)}</td><td class="num">{bytes(m.bytesOut)}</td><td class="num">{bytes(m.bytesIn)}</td></tr>
          {:else}
            <tr><td colspan="3">No traffic in the last hour.</td></tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
  </section>

  <section class="live" aria-label="Open connections">
    <div class="livehead"><h3>Open connections</h3><span class="tag tag-soft">{t?.connections.length ?? '…'}</span></div>
  {#if t?.connections.length}
    <ul class="conns" aria-label="Open connections">
      {#each t.connections as c (c.peer)}
        <li>
          <span class="whoami">{who(c)}</span>
          <span class={['tag', lockClass(c.encryption)]}>{c.encryption}</span>
          <span class="nums mono">↑ {bytes(c.bytesOut)} · ↓ {bytes(c.bytesIn)} · {c.recordsIn + c.recordsOut} records · since {ago(c.since)}</span>
        </li>
      {/each}
    </ul>
  {:else if t}
    <div class="idle"><p>No open connections right now.</p><p class="help">The chart above keeps the last hour of activity, even after a connection closes.</p></div>
  {:else}
    <p class="help" role="status">Loading connection activity…</p>
  {/if}
  </section>

  {#if t?.relay.possible}
    <div class="relay">
      <span class="lbl">{t.relay.on ? (t.relay.napping ? 'Relaying naps until tomorrow' : 'Relayed for members today') : 'Relaying is off'}</span>
      {#if t.relay.on}
        <div class="meter" role="meter" aria-valuemin="0" aria-valuemax={t.relay.capBytes} aria-valuenow={t.relay.todayBytes} aria-label="Relayed today against the daily cap">
          <i style:width={`${relayShare * 100}%`}></i>
        </div>
        <span class="mono">{bytes(t.relay.todayBytes)} of {bytes(t.relay.capBytes)}</span>
      {/if}
    </div>
  {/if}
  <p class="help foot">
    Activity counters stay in memory on this device and restart with hoi. They count bytes and records, never post text. The daily relay allowance is saved separately so restarting cannot bypass it.
  </p>
</div>

<style>
  .traffic { --out: #2f55e6; --in: #e0573a; display: flex; flex-direction: column; gap: 14px; min-width: 0; }
  :global(:root[data-theme='dark']) .traffic { --out: #6f86f0; }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme='light'])) .traffic { --out: #6f86f0; }
  }
  .tiles { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 1px; border: 1px solid var(--line); background: var(--line); border-radius: 12px; overflow: hidden; }
  .tile { padding: 14px; display: flex; flex-direction: column; gap: 4px; min-width: 0; background: var(--bg); }
  .tile b { font-size: 24px; font-weight: 900; letter-spacing: -0.03em; overflow-wrap: anywhere; }
  .lbl { font-size: 13px; color: var(--mute); font-weight: 600; }
  .chart { margin: 0; display: flex; flex-direction: column; gap: 8px; }
  .history { display: flex; flex-direction: column; gap: 10px; padding-block: 8px 4px; }
  .live { border-top: 2px dotted var(--line); padding-top: 18px; display: flex; flex-direction: column; gap: 12px; }
  .livehead { display: flex; align-items: center; gap: 10px; }
  h3 { font-size: 16px; font-weight: 800; }
  .idle { background: var(--soft); padding: 14px; border-radius: 12px; display: flex; flex-direction: column; gap: 6px; font-size: 14px; }
  figcaption { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
  .ttl { font-weight: 800; font-size: 15px; }
  .legend { display: inline-flex; align-items: center; gap: 6px; font-size: 13px; color: var(--mute); font-weight: 600; margin-left: auto; }
  .key { display: inline-block; width: 10px; height: 10px; border-radius: 3px; margin-right: 2px; }
  .legend .key + * { margin-right: 6px; }
  .key.out { background: var(--out); }
  .key.in { background: var(--in); }
  .plot { position: relative; padding: 18px 0 40px 0; }
  svg { width: 100%; height: 160px; display: block; touch-action: pan-y; border-radius: 12px; }
  svg:focus-visible { outline: 3px solid var(--cobalt); outline-offset: 4px; }
  .out { fill: var(--out); }
  .in { fill: var(--in); }
  .base { stroke: var(--line); stroke-width: 1; vector-effect: non-scaling-stroke; }
  .grid { stroke: var(--soft); stroke-width: 1; vector-effect: non-scaling-stroke; }
  .hover { fill: var(--soft); }
  .axis { position: absolute; font-size: 12px; color: var(--mute); font-weight: 600; font-variant-numeric: tabular-nums; pointer-events: none; }
  .axis.y { left: 0; }
  .axis.top { top: 0; }
  .axis.bottom { bottom: 20px; }
  .axis.x { bottom: 0; }
  .axis.left { left: 0; }
  .axis.right { right: 0; }
  .tip {
    position: absolute; top: 12px; transform: translateX(-50%); background: var(--card); border: 2px solid var(--ink); border-radius: 12px;
    padding: 8px 10px; display: flex; flex-direction: column; gap: 2px; font-size: 13px; color: var(--mute); pointer-events: none; white-space: nowrap; z-index: 2;
  }
  .tip b { color: var(--ink); font-weight: 800; }
  .tip .when { font-weight: 700; color: var(--ink); }
  .tip .key { width: 12px; height: 3px; border-radius: 2px; vertical-align: middle; margin-right: 6px; }
  .tablebtn { align-self: flex-start; }
  .mins table { min-width: 0; }
  .conns { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .conns li { display: flex; flex-wrap: wrap; gap: 6px 10px; align-items: center; padding: 10px 0; border-top: 2px dotted var(--line); min-width: 0; }
  .whoami { font-weight: 700; }
  .nums { flex-basis: 100%; overflow-wrap: anywhere; }
  .relay { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .meter { flex: 1 1 140px; max-width: 260px; height: 10px; border-radius: 999px; background: var(--mint); opacity: 1; overflow: hidden; border: 2px solid var(--ink); }
  .meter i { display: block; height: 100%; background: var(--tagink); border-radius: 999px; }
  .foot { border-top: 1px solid var(--line); padding-top: 14px; }
  .compact .tiles { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .compact .tile b { font-size: 20px; }
  @media (max-width: 640px) {
    .tiles { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .tile b { font-size: 20px; }
    .legend { margin-left: 0; }
  }
</style>
