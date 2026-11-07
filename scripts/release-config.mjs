import { readFileSync, statSync } from 'node:fs';
import { validateRelease } from './release-schema.mjs';

export { validateRelease } from './release-schema.mjs';

const BUILD_MANIFEST = new URL('../release.json', import.meta.url);

/** Compiled UI metadata must match Rust's include_str snapshot of the repository manifest. */
export function loadBuildRelease() {
  return loadRelease(BUILD_MANIFEST);
}

/** Read every time: an operator can replace the manifest without rebuilding the registry. */
export function loadRelease(filename = process.env.HOI_RELEASE_CONFIG || BUILD_MANIFEST) {
  if (statSync(filename).size > 16_384) throw new Error('Release configuration exceeds 16 KiB.');
  return validateRelease(JSON.parse(readFileSync(filename, 'utf8')));
}
