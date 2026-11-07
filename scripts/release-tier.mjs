import { appendFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

// Classify the numeric release, including prereleases; never trust a caller's tier label.
export function releaseTier(tag) {
  const match = /^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?$/.exec(tag);
  if (!match || match[0] !== tag) throw new Error(`Invalid release version: ${tag}; expected vX.Y.Z or vX.Y.Z-rc.1`);
  const [, major, minor, patch] = match;
  if (patch !== '0') return 'patch';
  if (minor !== '0') return 'minor';
  return major !== '0' ? 'major' : 'patch';
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const tier = releaseTier(process.argv[2] ?? '');
  console.log(tier);
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `tier=${tier}\n`);
}
