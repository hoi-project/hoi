// Offline verification of a candidate folder, against this checkout's trusted public keys.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { loadPublication } from './publication.mjs';
import { updaterTargets } from './release-manifest.mjs';
import { parseManifest, verifyMinisign } from './release-verify.mjs';

export function checkCandidate(dir, publication = loadPublication(true)) {
  const bytes = readFileSync(join(dir, 'manifest.json'));
  verifyMinisign(bytes, readFileSync(join(dir, 'manifest.json.minisig'), 'utf8'), publication.manifestKey);
  const manifest = parseManifest(bytes);
  for (const file of manifest.releases[0].files) {
    const artifact = readFileSync(join(dir, file.name));
    if (artifact.length !== file.size || createHash('sha256').update(artifact).digest('hex') !== file.sha256) throw new Error(`Artifact checksum mismatch: ${file.name}`);
  }
  if (manifest.updater?.version !== manifest.latest) throw new Error('Updater metadata does not match the release.');
  const platforms = manifest.updater.platforms;
  if (!platforms || typeof platforms !== 'object' || Array.isArray(platforms)) throw new Error('Updater platforms are missing.');
  const covered = new Set();
  for (const [target, entry] of Object.entries(platforms)) {
    const files = manifest.releases[0].files.filter(f => f.url === entry.url && !f.product);
    if (files.length !== 1) throw new Error(`Updater artifact is missing or ambiguous: ${target}`);
    const file = files[0];
    if (!updaterTargets(file).includes(target)) throw new Error(`Wrong updater product/platform: ${target}`);
    const signature = Buffer.from(entry.signature, 'base64').toString('utf8');
    verifyMinisign(readFileSync(join(dir, file.name)), signature, publication.updaterKey);
    const comment = signature.split(/\r?\n/)[2]?.slice('trusted comment: '.length);
    if (!comment?.split('\t').includes(`version:${manifest.latest}`)) throw new Error(`Updater signature lacks the matching signed version: ${file.name}`);
    covered.add(target.replace(/-(nsis|msi|deb|rpm)$/, ''));
  }
  for (const file of manifest.releases[0].files.filter(f => !f.product && f.platform !== 'node')) {
    // The Debian and RPM installers cannot install each other's files or an AppImage.
    // Each shipped Linux bundle must have its own verified updater signature.
    const targets = updaterTargets(file);
    if (file.platform === 'linux' && targets.length) {
      for (const target of targets) {
        if (!Object.hasOwn(platforms, target)) throw new Error(`Desktop release is missing its signed updater package: ${target}`);
      }
      continue;
    }
    const os = { macos: 'darwin', linux: 'linux', windows: 'windows' }[file.platform];
    for (const arch of file.arch === 'universal' ? ['x86_64', 'aarch64'] : [file.arch]) {
      if (!covered.has(`${os}-${arch}`)) throw new Error(`Desktop release is missing its signed updater package: ${os}-${arch}`);
    }
  }
  return manifest;
}
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  if (!process.argv[2]) throw new Error('Usage: node scripts/release-check.mjs CANDIDATE_DIRECTORY');
  const manifest = checkCandidate(process.argv[2]);
  console.log(`Verified ${manifest.latest}: manifest, ${manifest.releases[0].files.length} artifacts and updater metadata. Nothing published or installed.`);
}
