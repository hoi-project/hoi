// Release metadata is informational. Notice, channel, index and protocol labels never wipe
// IndexedDB or keys. Old communities remain usable with compatible software and their invites.
import fallback from '../../../../release.json';
import { validateRelease } from '../../../../scripts/release-schema.mjs';
import { fetchVerifiedManifest } from '../../../../scripts/release-verify.mjs';
import publication from '../../../../publication.json';

export const RELEASE = validateRelease(typeof __HOI_RELEASE__ !== 'undefined' ? __HOI_RELEASE__ : fallback);
export const PREVIEW = RELEASE.channel === 'preview';
export const PROTOCOL = RELEASE.protocolVersion;
export const PUBLICATION = typeof __HOI_PUBLICATION__ !== 'undefined' ? __HOI_PUBLICATION__ : publication;

export async function latestRelease() {
  const manifest = await fetchVerifiedManifest(PUBLICATION, __HOI_VERSION__);
  if (manifest.release.revision < RELEASE.revision) throw new Error('An older release notice was rejected.');
  return manifest.release;
}
