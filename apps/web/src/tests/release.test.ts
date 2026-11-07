import { afterEach, describe, expect, it, vi } from 'vitest';
import { latestRelease, RELEASE, PUBLICATION } from '../lib/preview';
import fixture from '../../../../scripts/fixtures/release-signed.json';

const original = { ...PUBLICATION };
afterEach(() => { vi.unstubAllGlobals(); Object.assign(PUBLICATION, original); });

describe('informational release updates', () => {
  it('reads only signed metadata without touching saved keys or databases', async () => {
    const remove = vi.fn();
    vi.stubGlobal('indexedDB', { deleteDatabase: remove });
    vi.stubGlobal('document', { baseURI: 'https://hoi.test/app/' });
    Object.assign(PUBLICATION, { downloads: 'https://releases.invalid', manifestKey: fixture.key });
    const updated = JSON.parse(fixture.manifest).release;
    const fetcher = vi.fn().mockImplementation(async (url: string) => new Response(url.endsWith('.minisig') ? fixture.signature : fixture.manifest));
    vi.stubGlobal('fetch', fetcher);
    expect(await latestRelease()).toMatchObject(updated);
    expect(await latestRelease()).not.toHaveProperty('wipe');
    expect(remove).not.toHaveBeenCalled();
    expect(fetcher.mock.calls[0][1]).toMatchObject({ cache: 'no-store', credentials: 'omit', referrerPolicy: 'no-referrer' });
  });
  it('rejects unsigned notices instead of treating them as stable', async () => {
    const buildChannel = RELEASE.channel;
    vi.stubGlobal('document', { baseURI: 'https://hoi.test/app/' });
    Object.assign(PUBLICATION, { downloads: 'https://releases.invalid', manifestKey: fixture.key });
    vi.stubGlobal('fetch', vi.fn().mockImplementation(async () => new Response(JSON.stringify({ schemaVersion: 2, channel: 'stable' }))));
    await expect(latestRelease()).rejects.toThrow('signature');
    expect(RELEASE.channel).toBe(buildChannel);
  });
});
