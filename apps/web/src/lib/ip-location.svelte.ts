// Finds the country an IP is listed in, on this device, from tables built into the app (#152). The tables
// load only after a person checks an address; opening the app or a panel loads nothing. Results are not
// cached per address: they are derived from the observation each time and go away with it.
import v4Url from './geo/data/ip-country-v4.bin?url&no-inline';
import v6Url from './geo/data/ip-country-v6.bin?url&no-inline';
import zonesUrl from './geo/data/tz-country.json?url&no-inline';
import mapUrl from './geo/data/world-map.json?url&no-inline';
import { addressKey, decodeTable, lookup, type CountryTable } from './geo/ip-table';
import { fetchData, fetchJson, inflate, LIMITS } from './geo/load';
import { deviceTimeZone, parseZoneTable, zoneCountry, type ZoneTable } from './geo/tz';
import { parseWorldMap, type WorldMap } from './geo/world-map';
import type { Lookup, Observation } from './ip-exposure';

type Load = 'idle' | 'loading' | 'ready' | 'failed';
export const ipLocation = $state<{ v4: Load; v6: Load; extras: Load; error: string; date: string | null }>({
  v4: 'idle', v6: 'idle', extras: 'idle', error: '', date: null,
});

const URLS = { 4: v4Url, 6: v6Url } as const;
const tables: { 4?: CountryTable; 6?: CountryTable } = {};
let zones: ZoneTable | null = null;
let world: WorldMap | null = null;
const running = new Map<string, Promise<void>>();

function once(name: string, work: () => Promise<void>): Promise<void> {
  const current = running.get(name);
  if (current) return current;
  const next = work().finally(() => running.delete(name));
  running.set(name, next);
  return next;
}

function loadTable(family: 4 | 6): Promise<void> {
  const key = `v${family}` as const;
  if (ipLocation[key] === 'ready') return Promise.resolve();
  return once(key, async () => {
    ipLocation[key] = 'loading';
    try {
      const table = decodeTable(await inflate(await fetchData(URLS[family], LIMITS.table), LIMITS.inflated));
      if (table.family !== family) throw new Error('The country table is damaged (wrong address family).');
      tables[family] = table;
      ipLocation[key] = 'ready';
    } catch (error) {
      ipLocation[key] = 'failed';
      ipLocation.error = error instanceof Error ? error.message : 'The country table could not be loaded.';
    }
  });
}

// The time zone table and the map are small and shared by both address families. Without them the status
// still names the country; it only cannot compare it with the time zone or draw the map.
function loadExtras(): Promise<void> {
  if (ipLocation.extras === 'ready') return Promise.resolve();
  return once('extras', async () => {
    ipLocation.extras = 'loading';
    try {
      const [zoneData, mapData, manifest] = await Promise.all([fetchJson(zonesUrl), fetchJson(mapUrl), import('./geo/data/manifest.json')]);
      zones = parseZoneTable(zoneData);
      world = parseWorldMap(mapData);
      ipLocation.date = typeof manifest.default.date === 'string' ? manifest.default.date : null;
      ipLocation.extras = 'ready';
    } catch {
      ipLocation.extras = 'failed';
    }
  });
}

/** Loads what the address needs. Call it only from something the person chose to do. */
export async function locate(address: string): Promise<void> {
  const key = addressKey(address);
  if (!key) return;
  ipLocation.error = '';
  await Promise.all([loadTable(key.family), loadExtras()]);
}

/** Reactive: reads the load state, so callers update when a table arrives. */
export function lookupOf(address: string): Lookup {
  const key = addressKey(address);
  if (!key) return { state: 'local' };
  const state = ipLocation[`v${key.family}`];
  const table = tables[key.family];
  if (state !== 'ready' || !table) return { state: state === 'ready' ? 'idle' : state };
  return { state: 'ready', code: lookup(table, key) };
}

export function deviceZone(): { zone: string | null; code: string | null } {
  const zone = deviceTimeZone();
  return { zone, code: ipLocation.extras === 'ready' ? zoneCountry(zones, zone) : null };
}

export function worldMap(): WorldMap | null {
  return ipLocation.extras === 'ready' ? world : null;
}

export function observation(address: string, observer: string): Observation {
  return { address, observer, lookup: lookupOf(address) };
}
