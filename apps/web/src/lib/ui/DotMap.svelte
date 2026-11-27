<script lang="ts">
  // A dotted world map with the listed country filled and a pin at its middle (#152). The pin marks the
  // country, never a location. Data: Natural Earth via scripts/geo-assets.mjs.
  import { dotsPath, mapDots, type WorldMap } from '../geo/world-map';
  import { regionName } from '../geo/tz';
  import { drawArc, dropPin, lightDots } from '../motion';
  import type { Tone } from '../ip-exposure';

  let { map, codes, zoneCode = null, tone = 'peach', date = null }: { map: WorldMap; codes: string[]; zoneCode?: string | null; tone?: Tone; date?: string | null } = $props();

  const dots = $derived(mapDots(map));
  const land = $derived(dotsPath(dots));
  const shown = $derived(codes.filter(code => map.countries[code]).slice(0, 2));
  const lit = $derived(dots.filter(d => shown.includes(d.code)));
  const pins = $derived(shown.map(code => ({ code, at: map.countries[code].p })));
  const zonePoint = $derived(zoneCode && !shown.includes(zoneCode) ? map.countries[zoneCode]?.p ?? null : null);
  const arc = $derived.by(() => {
    if (!zonePoint || pins.length !== 1) return null;
    const [[x1, y1], [x2, y2]] = [zonePoint, pins[0].at];
    const lift = Math.hypot(x2 - x1, y2 - y1) * 0.32;
    return `M${x1} ${y1}Q${(x1 + x2) / 2} ${Math.min(y1, y2) - lift} ${x2} ${y2}`;
  });
  const stickers = $derived(shown.filter(code => map.countries[code].d && map.countries[code].b));
  // Stickers sit in a lower corner; pick the one with fewer pins or zone markers in it.
  const east = $derived.by(() => {
    const marks = [...pins.map(p => p.at), ...(zonePoint ? [zonePoint] : [])];
    const low = marks.filter(([, y]) => y > map.h * 0.4);
    const left = low.filter(([x]) => x < map.w / 2).length, right = low.length - left;
    return left > right || (left === right && marks.some(([x]) => x < map.w * 0.45));
  });
  const pct = (n: number, of: number) => `${(n / of) * 100}%`;
  const label = $derived(shown.length
    ? `World map. ${shown.map(code => regionName(code)).join(' and ')} highlighted, with a pin at the middle of the country.${zonePoint ? ' A line joins it to your device’s time zone.' : ''}`
    : 'World map. The country of this IP has no place on the map.');

  function box(code: string) {
    const [x0, y0, x1, y1] = map.countries[code].b!;
    const size = Math.max(x1 - x0, y1 - y0, 6);
    const pad = size * 0.12;
    const cx = (x0 + x1) / 2, cy = (y0 + y1) / 2, w = Math.max(x1 - x0, size * 1.3) + pad * 2, h = Math.max(y1 - y0, size * 0.75) + pad * 2;
    return { view: `${cx - w / 2} ${cy - h / 2} ${w} ${h}`, r: size * 0.07 };
  }

  let litGroup: SVGGElement | undefined = $state();
  let arcPath: SVGPathElement | undefined = $state();
  $effect(() => {
    void shown;
    const circles = litGroup ? [...litGroup.querySelectorAll('circle')] : [];
    const stopDots = lightDots(circles, map.step * 0.36, pins[0]?.at ?? [0, 0]);
    const stopArc = drawArc(arcPath ?? null);
    return () => { stopDots(); stopArc(); };
  });
</script>

<figure class="dotmap" data-tone={tone}>
  <div class="mapframe">
    <div class="canvas" role="img" aria-label={label} style:aspect-ratio={`${map.w} / ${map.h}`}>
      <svg viewBox={`0 0 ${map.w} ${map.h}`} aria-hidden="true" focusable="false">
        {#each shown as code (code)}{#if map.countries[code].d}<path class="shape" d={map.countries[code].d} />{/if}{/each}
        <path class="land" d={land} stroke-width={map.step * 0.6} />
        <g class="lit" bind:this={litGroup}>{#each lit as d (`${d.x},${d.y}`)}<circle cx={d.x} cy={d.y} r={map.step * 0.36} />{/each}</g>
        {#if arc}<path class="arc" d={arc} bind:this={arcPath} />{/if}
      </svg>
      {#if zonePoint}<span class="zone" style:left={pct(zonePoint[0], map.w)} style:top={pct(zonePoint[1], map.h)}><i></i></span>{/if}
      {#each pins as pin (pin.code)}
        <span class="pin" style:left={pct(pin.at[0], map.w)} style:top={pct(pin.at[1], map.h)} use:dropPin><span class="ring"></span><span class="head"></span></span>
      {/each}
    </div>
    {#if stickers.length}
      <div class={['stickers', east && 'east']} aria-hidden="true">
        {#each stickers as code (code)}
          {@const b = box(code)}
          <div class="cstick">
            <svg viewBox={b.view}><path class="shape" d={map.countries[code].d} /><circle class="sp" cx={map.countries[code].p[0]} cy={map.countries[code].p[1]} r={b.r} /></svg>
            <span><b>{code}</b> {regionName(code)}</span>
          </div>
        {/each}
      </div>
    {/if}
  </div>
  <figcaption>
    <span class="key"><i class="kp"></i>Pin: middle of the country, not your location</span>
    {#if zonePoint}<span class="key"><i class="kz"></i>Your device’s time zone</span>{/if}
    {#if date}<span>Country table from {date}</span>{/if}
  </figcaption>
</figure>

<style>
  .dotmap { margin: 0; display: grid; gap: var(--space-2); min-width: 0; container-type: inline-size; }
  .mapframe { position: relative; min-width: 0; }
  .canvas { position: relative; width: 100%; max-width: 100%; }
  .canvas > svg { position: absolute; inset: 0; width: 100%; height: 100%; overflow: visible; }
  .land { fill: none; stroke: color-mix(in srgb, var(--mute) 40%, var(--card)); stroke-linecap: round; }
  .lit circle { fill: var(--ink); }
  .shape { fill: var(--peach); stroke: var(--tagink); stroke-width: 1.5; stroke-linejoin: round; fill-rule: evenodd; vector-effect: non-scaling-stroke; }
  [data-tone='sky'] .shape { fill: var(--sky); }
  [data-tone='sun'] .shape { fill: var(--sun); }
  .arc { fill: none; stroke: var(--ink); stroke-width: 2; stroke-linecap: round; vector-effect: non-scaling-stroke; }
  .pin, .zone { position: absolute; width: 0; height: 0; z-index: 2; }
  .head, .ring { position: absolute; left: -9px; top: -9px; width: 18px; height: 18px; border-radius: 50%; }
  .head { background: var(--coral); border: 2px solid var(--tagink); box-shadow: 2px 2px 0 var(--tagink); }
  .ring { border: 2px solid var(--coral); opacity: 0; }
  .zone i { position: absolute; left: -8px; top: -8px; width: 16px; height: 16px; border-radius: 50%; border: 2px solid var(--ink); background: var(--card); }
  .zone i::after, .kz::after { content: ''; position: absolute; inset: 3px; border-radius: 50%; background: var(--cobalt); }
  .stickers { position: absolute; left: 2%; bottom: 3%; display: flex; gap: var(--space-3); align-items: flex-end; z-index: 3; }
  .stickers.east { left: auto; right: 2%; }
  .cstick { width: 132px; display: grid; gap: 2px; padding: 6px 8px 8px; background: var(--card); border: 2px solid var(--ink); border-radius: 14px; box-shadow: 3px 3px 0 var(--ink); transform: rotate(-3deg); }
  .cstick + .cstick { transform: rotate(2.5deg); }
  .cstick svg { width: 100%; height: auto; aspect-ratio: 3 / 2; display: block; overflow: hidden; }
  .cstick .shape { stroke-width: 2; }
  .sp { fill: var(--coral); stroke: var(--tagink); stroke-width: 2; vector-effect: non-scaling-stroke; }
  .cstick span { font-size: 12px; font-weight: 800; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cstick b { font-family: var(--mono); font-weight: 500; font-size: 11px; padding: 0 4px; border: 1.5px solid var(--ink); border-radius: 5px; }
  figcaption { display: flex; flex-wrap: wrap; gap: var(--space-1) var(--space-4); font-size: 12px; font-weight: 600; color: var(--mute); }
  .key { display: inline-flex; align-items: center; gap: 6px; }
  .kp, .kz { position: relative; width: 10px; height: 10px; border-radius: 50%; flex: 0 0 10px; }
  .kp { background: var(--coral); border: 1.5px solid var(--tagink); }
  .kz { border: 1.5px solid var(--ink); background: var(--card); }
  .kz::after { inset: 2px; }
  @container (max-width: 520px) {
    .stickers { position: static; margin-top: var(--space-2); }
  }
</style>
