// Release metadata is informational. Notice, channel, index and protocol labels never wipe
// IndexedDB or keys. Old communities remain usable with compatible software and their invites.
import fallback from '../../../../release.json';
import { validateRelease } from '../../../../scripts/release-schema.mjs';
import { fetchBounded, fetchVerifiedManifest } from '../../../../scripts/release-verify.mjs';
import publication from '../../../../publication.json';

export const RELEASE = validateRelease(typeof __HOI_RELEASE__ !== 'undefined' ? __HOI_RELEASE__ : fallback);
export const PREVIEW = RELEASE.channel === 'preview';
export const PROTOCOL = RELEASE.protocolVersion;
export const PUBLICATION = typeof __HOI_PUBLICATION__ !== 'undefined' ? __HOI_PUBLICATION__ : publication;

/** The site's own unsigned list (apps/site/public/releases/manifest.json, the same check as the
 * download page), served until the first release is published. It offers nothing, so it needs no
 * signature, and asking for one would only fail; any other list must verify. */
function isPlaceholder(bytes: Uint8Array): boolean {
  try {
    const m = JSON.parse(new TextDecoder().decode(bytes));
    return m?.schema === 1 && m.latest === null && Array.isArray(m.releases) && m.releases.length === 0;
  } catch {
    return false;
  }
}

/** The verified release list, or null while no release has been published. */
export async function signedManifest() {
  if (PUBLICATION.downloads && PUBLICATION.manifestKey) {
    const list = `${PUBLICATION.downloads.replace(/\/+$/, '')}/manifest.json`;
    if (isPlaceholder(await fetchBounded(list, 1024 * 1024))) return null;
  }
  return fetchVerifiedManifest(PUBLICATION, __HOI_VERSION__);
}

/** The newest signed release notice, or null while no release has been published. */
export async function latestRelease() {
  const manifest = await signedManifest();
  if (!manifest) return null;
  if (manifest.release.revision < RELEASE.revision) throw new Error('An older release notice was rejected.');
  return manifest.release;
}
