// Uses user-assigned codes (XA, XB) and made-up zones only.
import { describe, expect, it } from 'vitest';
import { exposureStatus, NO_VPN_CLAIM, type ExposureInput, type Lookup } from '../lib/ip-exposure';

const name = (code: string) => `Country ${code}`;
const obs = (address: string, lookup: Lookup, observer = 'ipify.example') => ({ address, observer, lookup });
const status = (input: Partial<ExposureInput>) => exposureStatus({ pending: false, observations: [], zone: 'Test/Alpha', zoneCode: 'XA', name, ...input });
const ready = (code: string | null): Lookup => ({ state: 'ready', code });

describe('the IP exposure status', () => {
  it('starts unchecked and shows a check in progress', () => {
    expect(status({}).kind).toBe('unchecked');
    expect(status({ pending: true }).kind).toBe('checking');
  });

  it('compares the listed country with the device time zone', () => {
    const same = status({ observations: [obs('192.0.2.10', ready('XA'))] });
    expect(same).toMatchObject({ kind: 'same', tone: 'peach', short: 'Country XA', note: 'your time zone', codes: ['XA'] });
    const other = status({ observations: [obs('192.0.2.10', ready('XB'))] });
    expect(other).toMatchObject({ kind: 'other', tone: 'sky', short: 'Country XB', note: 'not your time zone' });
    expect(other.long).toContain('Country XA');
    const noZone = status({ observations: [obs('192.0.2.10', ready('XB'))], zone: 'UTC', zoneCode: null });
    expect(noZone).toMatchObject({ kind: 'no-zone', tone: 'sky' });
    expect(noZone.long).toContain('(UTC) is not tied to a country');
  });

  it('says when two observers see different countries', () => {
    const routes = status({ observations: [obs('192.0.2.10', ready('XB')), obs('2001:db8::25', ready('XA'), 'Node one')] });
    expect(routes).toMatchObject({ kind: 'routes', tone: 'sun', short: 'Routes differ', note: 'XB · XA', codes: ['XB', 'XA'] });
    expect(routes.long).toContain('Node one saw 2001:db8::25 in Country XA');
    expect(status({ observations: [obs('192.0.2.10', ready('XA')), obs('192.0.2.11', ready('XA'))] }).kind).toBe('same');
  });

  it('handles local, unlisted, loading, idle and failed lookups', () => {
    expect(status({ observations: [obs('192.168.1.20', { state: 'local' }, 'Home node')] })).toMatchObject({ kind: 'local', tone: 'lilac', address: '192.168.1.20' });
    expect(status({ observations: [obs('192.0.2.10', ready(null))] })).toMatchObject({ kind: 'unlisted', short: 'Country unknown' });
    expect(status({ observations: [obs('192.0.2.10', { state: 'loading' })] })).toMatchObject({ kind: 'locating', short: '192.0.2.10' });
    expect(status({ observations: [obs('192.0.2.10', { state: 'idle' })] })).toMatchObject({ kind: 'unlocated', short: '192.0.2.10' });
    expect(status({ observations: [obs('192.0.2.10', { state: 'failed' })] })).toMatchObject({ kind: 'unavailable', tone: 'dashed' });
    expect(status({ observations: [obs('192.168.1.20', { state: 'local' }), obs('192.0.2.10', ready('XB'))] }).kind).toBe('other');
  });

  it('never claims a VPN state', () => {
    const all = [
      status({}), status({ pending: true }),
      status({ observations: [obs('192.0.2.10', ready('XA'))] }), status({ observations: [obs('192.0.2.10', ready('XB'))] }),
      status({ observations: [obs('192.0.2.10', ready('XB')), obs('192.0.2.11', ready('XA'))] }),
      status({ observations: [obs('192.168.1.20', { state: 'local' })] }), status({ observations: [obs('192.0.2.10', { state: 'failed' })] }),
    ];
    for (const s of all) {
      const text = `${s.short} ${s.note} ${s.long}`.replace(NO_VPN_CLAIM, '');
      expect(text).not.toMatch(/\bVPN\b|\bsafe\b|protected|hidden/i);
    }
  });
});
