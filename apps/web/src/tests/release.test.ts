import { afterEach, describe, expect, it, vi } from 'vitest';
import { latestRelease, signedManifest, RELEASE, PUBLICATION } from '../lib/preview';
import fixture from '../../../../scripts/fixtures/release-signed.json';
import placeholder from '../../../site/public/releases/manifest.json';

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
  it('asks for no signature while the site serves its unsigned placeholder list', async () => {
    vi.stubGlobal('document', { baseURI: 'https://hoi.test/app/' });
    Object.assign(PUBLICATION, { downloads: 'https://releases.invalid', manifestKey: fixture.key });
    const fetcher = vi.fn().mockImplementation(async (url: string) => (url.endsWith('.minisig') ? new Response('', { status: 404 }) : new Response(JSON.stringify(placeholder))));
    vi.stubGlobal('fetch', fetcher);
    expect(await latestRelease()).toBeNull();
    expect(await signedManifest()).toBeNull();
    expect(fetcher.mock.calls.map(([url]) => String(url))).toEqual(['https://releases.invalid/manifest.json', 'https://releases.invalid/manifest.json']);
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
