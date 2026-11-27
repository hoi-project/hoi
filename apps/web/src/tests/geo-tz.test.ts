// Made-up zones and user-assigned codes only.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { deviceTimeZone, parseZoneTable, regionName, zoneCountry } from '../lib/geo/tz';

afterEach(() => { vi.restoreAllMocks(); });

describe('device time zone', () => {
  it('maps a zone through the table and nothing else', () => {
    const table = parseZoneTable({ 'Test/Alpha': 'XA', 'Test/Old_Alpha': 'XA' });
    expect(zoneCountry(table, 'Test/Old_Alpha')).toBe('XA');
    expect(zoneCountry(table, 'UTC')).toBeNull();
    expect(zoneCountry(table, 'constructor')).toBeNull();
    expect(zoneCountry(null, 'Test/Alpha')).toBeNull();
    expect(zoneCountry(table, null)).toBeNull();
  });

  it('rejects a damaged table', () => {
    expect(() => parseZoneTable([])).toThrow(/damaged/);
    expect(() => parseZoneTable({ 'Test/Alpha': 'xa' })).toThrow(/damaged/);
    expect(() => parseZoneTable({ 'Test/Alpha': 1 })).toThrow(/damaged/);
  });

  it('reads the zone from Intl and survives an Intl failure', () => {
    vi.spyOn(Intl, 'DateTimeFormat').mockReturnValue({ resolvedOptions: () => ({ timeZone: 'Test/Alpha' }) } as unknown as Intl.DateTimeFormat);
    expect(deviceTimeZone()).toBe('Test/Alpha');
    vi.spyOn(Intl, 'DateTimeFormat').mockImplementation(() => { throw new RangeError('no zone'); });
    expect(deviceTimeZone()).toBeNull();
  });

  it('falls back to the code when there is no name', () => {
    vi.spyOn(Intl, 'DisplayNames').mockImplementation(() => { throw new RangeError('unsupported'); });
    expect(regionName('XA', 'en')).toBe('XA');
  });
});
