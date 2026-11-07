<script lang="ts">
  // You → Connection: the network as this device sees it, drawn. This device in the middle, the
  // links it has now around it (what each one is, whether its copy outlives it), the public
  // nodes it knows but isn't connected to on the rim, and, for each public community, where a
  // post goes from here: pick one and the links that carry it light up. Everything comes from
  // this device's own links (NetStatus); it is a picture of connections, never a delivery
  // receipt (Warmth, PROTOCOL.md §20).
  import type { CommunityWarmth, NetStatus } from '@hoi/core-api';
  import { ago } from '@hoi/core-api';
  import { app } from '../app.svelte';
  import { plural } from '../util';
  import { warmthWords } from '../warmth';
  import Warmth from './Warmth.svelte';

  type Link = NonNullable<NetStatus['connections']>[number];
  type Kind = NonNullable<Link['kind']>;

  const KIND: Record<Kind, { word: string; short: string; glyph: string; color: string }> = {
    home: { word: 'Your home node', short: 'Home node', glyph: 'H', color: 'var(--sun)' },
    device: { word: 'One of your devices', short: 'Your device', glyph: 'D', color: 'var(--lilac)' },
    public: { word: 'A public node', short: 'Public node', glyph: 'P', color: 'var(--sky)' },
    node: { word: "A member's computer", short: "Member's node", glyph: 'N', color: 'var(--mint)' },
    browser: { word: "A member's browser or phone", short: 'Browser', glyph: 'B', color: 'var(--card)' },
  };
  /** At most this many links are drawn; the rest are counted. */
  const DRAWN = 8;

  const net = $derived(app.net);
  const kindOf = (l: Link): Kind => l.kind ?? (l.role === 'home' ? 'home' : l.role === 'client' ? 'device' : 'node');
  const links = $derived((net?.connections ?? []).filter((l) => l.connected));
  // Home first, then public nodes, members' nodes, devices, browsers: what lasts, first.
  const ORDER: Kind[] = ['home', 'public', 'node', 'device', 'browser'];
  const sorted = $derived([...links].sort((a, b) => ORDER.indexOf(kindOf(a)) - ORDER.indexOf(kindOf(b))));
  const drawn = $derived(sorted.slice(0, DRAWN));
  const hidden = $derived(sorted.length - drawn.length);
  const publicCommunities = $derived(app.joined.filter((c) => !app.privateGroups.some((g) => g.community === c.id)));
  const warmthOf = (id: string): CommunityWarmth | undefined => app.warmth?.communities.find((w) => w.community === id);
  const dir = $derived(net?.directory);
  const rim = $derived(Math.min(12, Math.max(0, (dir?.known ?? 0) - (dir?.connected ?? 0))));
  // Nothing linked and nothing known: just this device, drawn small instead of alone in a big empty map.
  const alone = $derived(drawn.length === 0 && rim === 0);

  // The community whose path is lit; the first one with somewhere to go, until the person picks.
  let picked = $state<string | null>(null);
  const selected = $derived(picked ?? publicCommunities.find((c) => links.some((l) => l.communities?.includes(c.id)))?.id ?? publicCommunities[0]?.id ?? null);
  const carriers = (id: string | null) => (id ? links.filter((l) => l.communities?.includes(id)) : []);
  const lit = $derived(new Set(carriers(selected).map((l) => l.peer)));

  // Geometry: a 360 × 300 drawing, the device in the middle, links on a circle around it.
  const W = 360, H = 320, CX = 180, CY = 160, R = 104, RIM = 138;
  const at = (i: number, n: number, r: number) => {
    const a = -Math.PI / 2 + (2 * Math.PI * i) / Math.max(n, 1) + (n === 2 ? Math.PI / 2 : 0);
    return { x: CX + r * Math.cos(a), y: CY + r * Math.sin(a) };
  };

  const counts = $derived(ORDER.map((k) => ({ k, n: links.filter((l) => kindOf(l) === k).length })).filter((x) => x.n));
  const lasting = $derived(links.filter((l) => l.keeps).length);
  const summary = $derived(
    links.length
      ? `This ${app.platformName} is connected to ${plural(links.length, 'device')} now: ${counts.map(({ k, n }) => (n === 1 ? KIND[k].word.toLowerCase() : `${n} × ${KIND[k].short.toLowerCase()}`)).join(', ')}. ${lasting === 1 ? 'One of them keeps what it gets after the link closes.' : lasting ? `${lasting} of them keep what they get after the link closes.` : 'None of them keeps a copy after the link closes.'}`
      : `This ${app.platformName} is connected to nobody right now. What you post waits here until a node that carries the community connects.`,
  );
  const pickedName = $derived(publicCommunities.find((c) => c.id === selected)?.name);
  const path = $derived.by(() => {
    if (!selected) return '';
    const c = carriers(selected);
    const keep = c.filter((l) => l.keeps).length;
    if (!c.length) return `A post in c/${pickedName} is saved here and waits: no link carries it right now.`;
    const one = 'keeps it, hands it to every member who connects there and passes it to the nodes it reaches';
    const many = 'keep it, hand it to every member who connects there and pass it to the nodes they reach';
    const kept = keep === 1 ? `${c.length === 1 ? 'it' : 'one of them'} ${one}` : `${keep === c.length ? 'all' : `${keep} of them`} ${many}`;
    return `A post in c/${pickedName} goes from here to ${plural(c.length, 'link')}${keep ? `; ${kept}` : ', none of which keeps it after it closes'}.`;
  });
</script>

<section class="netmap" aria-labelledby="nm-h">
  <div class="head">
    <h3 id="nm-h">Your network</h3>
    <p class="help">{summary}</p>
  </div>

  <div class={['body', alone && 'alone']}>
    <svg viewBox={alone ? `${CX - 40} ${CY - 40} 80 80` : `0 0 ${W} ${H}`} role="img" aria-label={`${summary} ${path}`}>
      <!-- Public nodes this device knows but isn't connected to: places it can reach next. -->
      {#each Array.from({ length: rim }) as _, i (i)}
        {@const p = at(i + 0.5, Math.max(rim, 6), RIM)}
        <circle class="known" cx={p.x} cy={p.y} r="5" />
      {/each}
      {#each drawn as l, i (l.peer)}
        {@const p = at(i, drawn.length, R)}
        {@const on = lit.has(l.peer)}
        {@const d = Math.hypot(p.x - CX, p.y - CY) || 1}
        <!-- From the rim of this device's circle to the rim of the link's. -->
        <line class={['edge', !l.keeps && 'brief', on && 'on']} x1={CX + ((p.x - CX) * 32) / d} y1={CY + ((p.y - CY) * 32) / d} x2={p.x - ((p.x - CX) * 24) / d} y2={p.y - ((p.y - CY) * 24) / d} />
      {/each}
      {#each drawn as l, i (l.peer)}
        {@const p = at(i, drawn.length, R)}
        {@const k = KIND[kindOf(l)]}
        {@const on = lit.has(l.peer)}
        <!-- Labels go away from the middle: above a link drawn above it, below the others. -->
        {@const up = p.y < CY - 20}
        <g class={['peer', selected && !on && 'dim']}>
          <circle cx={p.x} cy={p.y} r="22" fill={k.color} class={['dot', !l.keeps && 'brief']} />
          <text x={p.x} y={p.y + 5} class={['glyph', kindOf(l) === 'browser' && 'oncard']}>{k.glyph}</text>
          <text x={p.x} y={up ? p.y - 44 : p.y + 38} class="lbl">{k.short}</text>
          <text x={p.x} y={up ? p.y - 30 : p.y + 52} class="sub">{plural(l.communities?.length ?? 0, 'community', 'communities')}</text>
        </g>
      {/each}
      <g class="me">
        <circle cx={CX} cy={CY} r="30" />
        <text x={CX} y={CY - 2} class="glyph mine">you</text>
        <text x={CX} y={CY + 13} class="sub mine">{app.platformName}</text>
      </g>
      {#if hidden > 0}<text x={W - 8} y={H - 8} class="sub" text-anchor="end">+{hidden} more</text>{/if}
    </svg>

    <div class="side">
      <dl class="totals">
        <div><dt>Connected now</dt><dd>{links.length}</dd></div>
        <div><dt>Keep a copy</dt><dd>{lasting}</dd></div>
        <div><dt>Public nodes known</dt><dd>{dir?.known ?? 0}{dir?.waiting ? ` · ${dir.waiting} to try` : ''}</dd></div>
        <div><dt>Communities heard of</dt><dd>{dir?.listings ?? 0}</dd></div>
        <div><dt>Last catch-up</dt><dd>{net?.lastSync ? ago(net.lastSync) : 'not yet'}</dd></div>
      </dl>
      <ul class="legend" aria-label="What the circles are">
        {#each ORDER as k (k)}<li><i style:background={KIND[k].color} class={[k === 'browser' && 'brief']}></i>{KIND[k].short}</li>{/each}
        <li><i class="ring"></i>Known, not connected</li>
        <li><i class="dash"></i>Copy closes with the link</li>
      </ul>
    </div>
  </div>

  {#if publicCommunities.length}
    <h4 class="sub-h">Where a post goes</h4>
    <p class="help">Pick a community: the links that carry it light up. Signed here, your post goes to those links; the ones that keep a copy hand it to every member who connects to them, and pass it to the nodes they reach. A connection is not a receipt; Warmth counts the copies this {app.platformName} can see.</p>
    <ul class="routes">
      {#each publicCommunities as c (c.id)}
        {@const via = carriers(c.id)}
        {@const w = warmthOf(c.id)}
        <li>
          <button type="button" class={['route', selected === c.id && 'on']} aria-pressed={selected === c.id} onclick={() => (picked = c.id)}>
            <span class="cname"><span class="cdot bg-{c.color}" aria-hidden="true"></span>c/{c.name}</span>
            <span class="via">
              {#if via.length}
                {#each via as l (l.peer)}<span class={['chip', kindOf(l) === 'browser' && 'oncard']} style:background={KIND[kindOf(l)].color}>{KIND[kindOf(l)].short}</span>{/each}
              {:else}
                <span class="none">saved here, waiting for a link</span>
              {/if}
            </span>
            {#if w}<span class="wt" title={warmthWords(w, { platformName: app.platformName }).summary}><Warmth {w} compact /></span>{/if}
          </button>
        </li>
      {/each}
    </ul>
    {#if path}<p class="path" role="status">{path}</p>{/if}
  {/if}
</section>

<style>
  .netmap { display: flex; flex-direction: column; gap: 12px; padding: 18px; border: 2px solid var(--ink); border-radius: var(--radius-card); background: var(--card); margin-bottom: 18px; }
  .head h3, .sub-h { font-size: 20px; font-weight: 800; letter-spacing: -0.02em; }
  .sub-h { font-size: 16px; margin-top: 6px; }
  .head .help { max-width: 72ch; margin-top: 4px; }
  .body { display: grid; grid-template-columns: minmax(0, 420px) minmax(0, 1fr); gap: 18px 28px; align-items: center; }
  svg { width: 100%; height: auto; display: block; overflow: visible; }
  .body.alone { grid-template-columns: auto minmax(0, 1fr); }
  .alone svg { width: 80px; }
  .edge { stroke: var(--line); stroke-width: 3; }
  .edge.brief { stroke-dasharray: 5 6; }
  /* A lit link: dots running outward, the way a post goes. */
  .edge.on { stroke: var(--ink); stroke-width: 5; stroke-linecap: round; stroke-dasharray: 0.1 11; animation: flow 0.8s linear infinite; }
  @keyframes flow { to { stroke-dashoffset: -11.1; } }
  .dot { stroke: var(--ink); stroke-width: 2.5; }
  .dot.brief { stroke-dasharray: 4 4; }
  .peer { transition: opacity .2s; }
  .peer.dim { opacity: .38; }
  .glyph { font-weight: 900; font-size: 15px; text-anchor: middle; fill: var(--tagink); }
  .lbl { font-weight: 800; font-size: 12px; text-anchor: middle; fill: var(--ink); }
  .sub { font-weight: 600; font-size: 11px; text-anchor: middle; fill: var(--mute); }
  .me circle { fill: var(--ink); }
  .mine { fill: var(--card); }
  .glyph.oncard { fill: var(--ink); }
  .known { fill: none; stroke: var(--mute); stroke-width: 2; }
  .totals { margin: 0; display: grid; grid-template-columns: repeat(auto-fit, minmax(130px, 1fr)); gap: 10px 16px; }
  .totals div { display: flex; flex-direction: column; gap: 2px; }
  .totals dt { font-size: 12px; color: var(--mute); font-weight: 600; }
  .totals dd { margin: 0; font-size: 20px; font-weight: 900; font-variant-numeric: tabular-nums; }
  .legend { list-style: none; margin: 16px 0 0; padding: 0; display: flex; flex-wrap: wrap; gap: 6px 14px; font-size: 12px; font-weight: 600; color: var(--mute); }
  .legend li { display: inline-flex; align-items: center; gap: 6px; }
  .legend i { width: 12px; height: 12px; border-radius: 50%; border: 2px solid var(--ink); display: inline-block; }
  .legend i.brief { border-style: dashed; }
  .legend i.ring { border-color: var(--mute); background: none; }
  .legend i.dash { width: 18px; height: 0; border-radius: 0; border-width: 0 0 3px; border-style: dashed; border-color: var(--line); }
  .routes { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .route { width: 100%; display: grid; grid-template-columns: minmax(0, 180px) minmax(0, 1fr) auto; gap: 8px 14px; align-items: center; text-align: left; padding: 10px 12px; border: 2px solid var(--line); border-radius: var(--radius-input); background: var(--card); min-height: 48px; }
  .route:hover { border-color: var(--ink); }
  .route.on { border-color: var(--ink); box-shadow: 3px 3px 0 var(--ink); }
  .cname { display: inline-flex; align-items: center; gap: 8px; font-weight: 800; overflow-wrap: anywhere; min-width: 0; }
  .cdot { width: 14px; height: 14px; border-radius: 5px; border: 2px solid var(--ink); flex: 0 0 14px; }
  .via { display: flex; flex-wrap: wrap; gap: 4px 6px; min-width: 0; }
  .chip { font-size: 12px; font-weight: 700; padding: 2px 9px; border-radius: 999px; border: 1.5px solid var(--ink); color: var(--tagink); white-space: nowrap; }
  .chip.oncard { color: var(--ink); border-style: dashed; }
  .none { font-size: 13px; color: var(--mute); font-weight: 600; }
  .path { font-size: 14px; font-weight: 600; line-height: 1.5; max-width: 72ch; }
  @media (max-width: 860px) {
    .body { grid-template-columns: 1fr; }
    svg { max-width: 420px; margin: 0 auto; }
    .body.alone { grid-template-columns: auto minmax(0, 1fr); align-items: start; }
    .alone svg { margin: 0; }
  }
  @media (max-width: 640px) {
    .netmap { padding: 14px; }
    .route { grid-template-columns: minmax(0, 1fr) auto; }
    .via { grid-column: 1 / -1; grid-row: 2; }
    .body.alone { grid-template-columns: 1fr; }
  }
  @media (prefers-reduced-motion: reduce) {
    .edge.on { animation: none; }
  }
  :global(html[data-motion='reduced']) .edge.on { animation: none; }
</style>
