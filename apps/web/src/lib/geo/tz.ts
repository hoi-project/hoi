// The device's time zone is read on the device and never sent anywhere. It is compared with an IP's
// country only to say whether the two match.
export type ZoneTable = Record<string, string>;

export function deviceTimeZone(): string | null {
  try { return Intl.DateTimeFormat().resolvedOptions().timeZone || null; }
  catch { return null; }
}

export function zoneCountry(table: ZoneTable | null, zone: string | null): string | null {
  if (!table || !zone || !Object.hasOwn(table, zone)) return null;
  return table[zone];
}

export function parseZoneTable(value: unknown): ZoneTable {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('The time zone table is damaged.');
  for (const [zone, code] of Object.entries(value)) {
    if (typeof code !== 'string' || !/^[A-Z]{2}$/.test(code) || !zone) throw new Error('The time zone table is damaged.');
  }
  return value as ZoneTable;
}

/** The country's name in the interface language, or its code when the browser has no name for it. */
export function regionName(code: string, locale = typeof document === 'undefined' ? 'en' : document.documentElement.lang || 'en'): string {
  try { return new Intl.DisplayNames([locale, 'en'], { type: 'region', fallback: 'code' }).of(code) || code; }
  catch { return code; }
}
