<script lang="ts">
  // Where an observed IP is listed, found on this device (#152). Opening this card looks nothing up: the
  // country tables load only after a check the person chose, or after they press Show country.
  import { addressScope } from '../connection-ip';
  import { regionName } from '../geo/tz';
  import { exposureStatus, type Observation } from '../ip-exposure';
  import { deviceZone, ipLocation, locate, observation, worldMap } from '../ip-location.svelte';
  import { toast } from '../toast.svelte';
  import { copyText } from '../util';
  import DotMap from './DotMap.svelte';
  import StatusIcon from './StatusIcon.svelte';

  let { address, observer, via, checkedAt, also = [], compact = false }: {
    address: string; observer: string; via: string; checkedAt?: Date; also?: Observation[]; compact?: boolean;
  } = $props();

  const own = $derived(observation(address, observer));
  const zone = $derived(deviceZone());
  const status = $derived(exposureStatus({ pending: false, observations: [own, ...also], zone: zone.zone, zoneCode: zone.code, name: regionName }));
  const map = $derived(worldMap());
  const code = $derived(own.lookup.state === 'ready' ? own.lookup.code : null);
  const located = $derived(status.codes.length > 0);
  const family = $derived(address.includes(':') ? 'IPv6' : 'IPv4');
  const title = $derived(status.kind === 'routes' ? 'Routes differ' : code ? regionName(code) : status.kind === 'local' ? 'Local address'
    : status.kind === 'locating' ? 'Finding country…' : status.kind === 'unlocated' ? 'Country not shown yet' : status.short);
  const stamp = $derived(({ same: 'Same country as your time zone', other: 'Not your time zone', locating: 'Finding country…', unlocated: 'Not looked up' } as Record<string, string>)[status.kind] ?? status.short);
  let busy = $state(false);

  async function show() {
    busy = true;
    try { await locate(address); } finally { busy = false; }
  }
  async function copy() {
    toast(await copyText(address) ? 'Address copied.' : 'Copy did not work here.');
  }
</script>

<section class={['card', compact && 'compact']} data-tone={status.tone} aria-label={`Where ${address} is listed`}>
  <div class="head">
    <span class="stamp" data-tone={status.tone}><StatusIcon icon={status.icon} size={14} />{stamp}</span>
    {#if located}<span class="tag tag-line">Approximate · country level</span>{/if}
  </div>
  {#if !compact}
    <h4 class="country">{title}{#if code}<span class="cc">{code}</span>{/if}</h4>
    <p class="ip"><b class="addr">{address}</b><span class="fam">{family}</span><button type="button" class="pill pill-sm" onclick={copy} aria-label={`Copy ${address}`}>Copy</button></p>
  {/if}
  <p class="say">{status.long}</p>
  {#if own.lookup.state === 'idle' || own.lookup.state === 'failed'}
    <button type="button" class="pill pill-sm" onclick={show} aria-disabled={busy}>{busy ? 'Finding country…' : own.lookup.state === 'failed' ? 'Try again' : 'Show country'}</button>
    {#if own.lookup.state === 'idle'}<p class="help">This loads hoi’s country table from this site. Your IP stays on this device.</p>{/if}
    {#if ipLocation.error && own.lookup.state === 'failed'}<p class="help" role="alert">{ipLocation.error}</p>{/if}
  {/if}
  {#if located && map && !compact}
    <DotMap {map} codes={status.codes} zoneCode={status.kind === 'other' ? zone.code : null} tone={status.tone} date={ipLocation.date} />
  {/if}
  {#if !compact}
    <dl class="facts">
      <div><dt>Seen by</dt><dd>{observer}<small>{via}{checkedAt ? ` · ${checkedAt.toLocaleTimeString()}` : ''}</small></dd></div>
      <div><dt>Address</dt><dd>{family}<small>{addressScope(address)}</small></dd></div>
      <div><dt>Listed in</dt><dd>{code ? regionName(code) : own.lookup.state === 'ready' ? 'Not listed' : own.lookup.state === 'local' ? 'No country for this address' : own.lookup.state === 'loading' ? 'Finding…' : 'Not looked up'}{#if code}<small>Country level, from public routing data. Not your exact location, and it can be wrong.</small>{/if}</dd></div>
      <div><dt>Your device</dt><dd>{zone.zone ? `Time zone ${zone.zone}` : 'Time zone unknown'}<small>{zone.code ? `${regionName(zone.code)} · ` : ''}Read on this device, never sent</small></dd></div>
      {#if ipLocation.date}<div><dt>Country table</dt><dd>Built into hoi<small>{ipLocation.date}</small></dd></div>{/if}
    </dl>
  {/if}
</section>

<style>
  .card { --tint: 45%; display: grid; gap: var(--space-3); min-width: 0; padding: var(--space-4); border: 2px solid var(--ink); border-radius: var(--radius-card); background: var(--card); box-shadow: 4px 4px 0 var(--ink); }
  .card.compact { box-shadow: none; border-color: var(--line); }
  .head { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); }
  .stamp {
    display: inline-flex; align-items: center; gap: 6px; padding: 4px 12px; border-radius: var(--radius-pill); border: 2px solid var(--ink);
    font-weight: 800; font-size: var(--type-tag-size); color: var(--tagink); background: var(--soft); transform: rotate(-2deg); transform-origin: left center;
  }
  .stamp[data-tone='peach'] { background: var(--peach); } .stamp[data-tone='sky'] { background: var(--sky); }
  .stamp[data-tone='sun'] { background: var(--sun); } .stamp[data-tone='lilac'] { background: var(--lilac); }
  .stamp[data-tone='idle'], .stamp[data-tone='dashed'] { color: var(--ink); border-color: var(--line); }
  .country, .ip, .say, .help { margin: 0; }
  .country { display: flex; flex-wrap: wrap; align-items: baseline; gap: var(--space-2); font-size: var(--type-title-size); font-weight: 900; letter-spacing: -0.03em; overflow-wrap: anywhere; }
  .cc, .fam { font-family: var(--mono); font-weight: 500; font-size: var(--type-mono-size); letter-spacing: 0; }
  .cc { padding: 0 5px; border: 1.5px solid var(--ink); border-radius: 6px; }
  .fam { color: var(--mute); }
  .ip { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2) var(--space-3); min-width: 0; }
  .ip .addr { font-size: 22px; font-weight: 900; letter-spacing: -0.02em; font-variant-numeric: tabular-nums; overflow-wrap: anywhere; min-width: 0; }
  .say { padding: var(--space-3) var(--space-4); border-radius: var(--radius-input); background: var(--soft); font-weight: 600; font-size: var(--type-meta-size); line-height: 1.45; overflow-wrap: anywhere; }
  [data-tone='peach'] > .say { background: color-mix(in srgb, var(--peach) var(--tint), var(--card)); }
  [data-tone='sky'] > .say { background: color-mix(in srgb, var(--sky) var(--tint), var(--card)); }
  [data-tone='sun'] > .say { background: color-mix(in srgb, var(--sun) var(--tint), var(--card)); }
  [data-tone='lilac'] > .say { background: color-mix(in srgb, var(--lilac) var(--tint), var(--card)); }
  [data-tone='dashed'] > .say { background: transparent; border: 2px dashed var(--line); }
  button { justify-self: start; }
  .facts { margin: 0; display: grid; border-top: 2px solid var(--ink); }
  .facts div { display: grid; grid-template-columns: 112px minmax(0, 1fr); gap: var(--space-3); padding: 10px 0; border-bottom: 2px dotted var(--line); }
  .facts dt { color: var(--mute); font-size: 13px; font-weight: 700; }
  .facts dd { margin: 0; min-width: 0; font-weight: 600; overflow-wrap: anywhere; }
  .facts small { display: block; color: var(--mute); font-weight: 500; font-size: 13px; }
  @media (max-width: 480px) { .facts div { grid-template-columns: minmax(0, 1fr); gap: 2px; } }
  /* A lighter tint in dark themes keeps the text readable. */
  @media (prefers-color-scheme: dark) { :global(html:not([data-theme='light'])) .card { --tint: 18%; } }
  :global(html[data-theme='dark']) .card { --tint: 18%; }
</style>
