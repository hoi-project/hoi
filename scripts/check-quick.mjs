// Bounded checks: JavaScript dependencies installed (pnpm install), but no native compilation,
// service, browser or full test suite.
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { checkVersion } from './release-version.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
checkVersion();
for (const args of [
  ['packages/tokens/scripts/generate.mjs', '--check'],
  ['scripts/check-dependency-security.mjs'],
  ['scripts/export-public.mjs', '--check'],
  // The wiki: every page's structure and links, and the SBOM against every lockfile.
  ['apps/site/scripts/wiki-check.mjs'],
  ['apps/site/scripts/sbom.mjs', '--check'],
  // Scattered delivery's sizing vectors against the spec's reference arithmetic.
  ['docs/spec/scatter-params.mjs', '--check', 'core/vectors/scatter.json'],
  ['--test', 'scripts/release-tier.test.mjs', 'scripts/export-public.test.mjs',
    'scripts/release-config.test.mjs', 'scripts/release-local.test.mjs',
    'scripts/release-version.test.mjs', 'scripts/release-workflow.test.mjs',
    'scripts/release-plan.test.mjs', 'scripts/release-manifest.test.mjs', 'scripts/build-all.test.mjs',
    'scripts/installer-art.test.mjs',
    'scripts/static-boundaries.test.mjs', 'scripts/public-copy.test.mjs', 'scripts/wiki.test.mjs',
    'scripts/labels.test.mjs', 'scripts/ci-changes.test.mjs'],
]) {
  const result = spawnSync(process.execPath, args, { cwd: root, stdio: 'inherit', timeout: 90_000 });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
