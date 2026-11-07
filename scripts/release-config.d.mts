import type { ReleaseConfig } from './release-schema.mjs';
export { validateRelease } from './release-schema.mjs';
export function loadRelease(filename?: string | URL): ReleaseConfig;
export function loadBuildRelease(): ReleaseConfig;
