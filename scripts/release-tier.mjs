import { appendFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

// Classify the numeric release, including prereleases; never trust a caller's tier label.
export function releaseTier(tag) {
  // The signed manifest reader caps canonical versions at 100 characters. Rust's
  // SemVer parser stores core components as u64; reject these mismatches before builds.
  if (typeof tag !== 'string' || tag.replace(/^v/, '').length > 100) {
    throw new Error('Invalid release version: expected a version of at most 100 characters (excluding the v tag prefix).');
  }
  const match = /^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?$/.exec(tag);
  if (!match || match[0] !== tag) throw new Error(`Invalid release version: ${tag}; expected vX.Y.Z or vX.Y.Z-rc.1`);
  if (match.slice(1, 4).some(part => BigInt(part) > 18_446_744_073_709_551_615n)) {
    throw new Error('Invalid release version: major, minor and patch must fit Rust SemVer unsigned 64-bit integers.');
  }
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
