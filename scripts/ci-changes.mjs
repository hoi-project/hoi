// Which CI and Security jobs a run needs, from the files it changes (the `changes` job in ci.yml
// and security.yml). A pull request compares its merge commit with the first parent (the base it
// was merged into), so only the pull request's own changes count; a push compares `before` with
// the pushed commit. Anything else (a manual or scheduled run, a new branch, a `before` that is
// gone) runs everything. Skipped jobs count as passed for required checks, which is why CI gates
// jobs here instead of with workflow-level `paths:` filters (those leave required checks pending).
//
//   node scripts/ci-changes.mjs            writes key=value lines to $GITHUB_OUTPUT (or stdout)
import { execFileSync } from 'node:child_process';
import { appendFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const any = (paths, patterns) => paths.some((path) => patterns.some((pattern) => pattern.test(path)));
const RUST = [/^core\//, /^vendor\//, /^\.github\/workflows\/ci\.yml$/, /^scripts\/ci-changes\.mjs$/];
const WEB = [/^apps\//, /^packages\//, /^design\/tokens\//, /^vendor\//, /^package\.json$/, /^pnpm-(lock|workspace)\.yaml$/, /^release\.json$/, /^publication\.json$/, /^\.github\/workflows\/ci\.yml$/, /^scripts\/ci-changes\.mjs$/];
const DEPS = [/(^|\/)Cargo\.(toml|lock)$/, /(^|\/)deny\.toml$/, /^core\/supply-chain\//, /^vendor\//, /^package\.json$/, /(^|\/)package\.json$/, /^pnpm-(lock|workspace)\.yaml$/, /^scripts\/check-dependency-security/, /^\.github\/workflows\/security\.yml$/];
const WORKFLOWS = [/^\.github\//];

/** The job switches for a list of changed paths, or `null` (run everything). */
export function classify(paths) {
  if (paths === null) return { rust: true, web: true, deps: true, workflows: true };
  return { rust: any(paths, RUST), web: any(paths, WEB), deps: any(paths, DEPS), workflows: any(paths, WORKFLOWS) };
}

function git(...args) {
  return execFileSync('git', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
}

/** Changed paths for this run, or null when the run should check everything. */
export function changedPaths(env = process.env) {
  try {
    if (env.EVENT === 'pull_request') return git('diff', '--name-only', 'HEAD^1', 'HEAD').split('\n').filter(Boolean);
    if (env.EVENT === 'push' && env.BEFORE && !/^0+$/.test(env.BEFORE)) {
      git('cat-file', '-e', `${env.BEFORE}^{commit}`);
      return git('diff', '--name-only', env.BEFORE, 'HEAD').split('\n').filter(Boolean);
    }
  } catch {
    // history not available: check everything
  }
  return null;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const paths = changedPaths();
  const result = classify(paths);
  const lines = Object.entries(result).map(([key, value]) => `${key}=${value}`);
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `${lines.join('\n')}\n`);
  console.log(paths === null ? 'Checking everything (no comparable base).' : `${paths.length} changed path(s).`);
  console.log(lines.join('\n'));
}
