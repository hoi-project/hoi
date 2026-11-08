#!/usr/bin/env node
// Read-only planning for the Release: version workflow. GitHub creates the tag afterward.
import { spawnSync } from 'node:child_process';
import { appendFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { checkVersion, readVersion, versionFromText } from './release-version.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const workspace = { path: 'core/Cargo.toml', kind: 'cargo', section: 'workspace.package' };

function readGit(args, base) {
  const result = spawnSync('git', args, { cwd: base, encoding: 'utf8' });
  if (result.error) throw result.error;
  if (result.status === 1 && args[0] === 'show-ref') return null;
  if (result.status !== 0) throw new Error(`git ${args.join(' ')} failed: ${result.stderr.trim()}`);
  return result.stdout.trim();
}

export function planRelease({ automatic = false, ref, before } = {}, base = root, git = args => readGit(args, base)) {
  if (automatic) {
    if (ref !== 'refs/heads/main') return { release: false, reason: 'Automatic version releases only run on main.' };
    if (!before || /^0+$/.test(before)) return { release: false, reason: 'Initial branch push; use Release: version manually for the first release.' };
    if (!/^[0-9a-f]{40,64}$/.test(before)) throw new Error('Invalid previous commit SHA.');
    const previous = versionFromText(workspace, git(['show', `${before}:${workspace.path}`]));
    if (!previous) throw new Error('Previous commit has no workspace version; run Release: version manually.');
    if (previous === readVersion(workspace, base)) return { release: false, reason: 'Product version is unchanged.' };
  }
  const details = checkVersion(undefined, base);
  const sha = git(['rev-parse', 'HEAD^{commit}']);
  const tagRef = `refs/tags/${details.tag}`;
  const exists = git(['show-ref', '--verify', '--quiet', tagRef]) !== null;
  if (exists && git(['rev-parse', `${tagRef}^{commit}`]) !== sha) {
    throw new Error(`${details.tag} already points to another commit. Use Retry unpublished in Release for a failed release, or prepare a new version.`);
  }
  return { release: true, ...details, sha, create_tag: !exists };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try {
    const plan = planRelease({ automatic: process.env.GITHUB_EVENT_NAME === 'push',
      ref: process.env.GITHUB_REF, before: process.env.RELEASE_BEFORE_SHA });
    console.log(plan.release ? `Release ${plan.tag} from ${plan.sha}; ${plan.create_tag ? 'create' : 'reuse'} its tag.` : plan.reason);
    if (process.env.GITHUB_OUTPUT) {
      appendFileSync(process.env.GITHUB_OUTPUT, Object.entries(plan).filter(([key]) => key !== 'reason')
        .map(([key, value]) => `${key}=${value}\n`).join(''));
    }
  } catch (error) { console.error(`release-plan: ${error.message}`); process.exitCode = 1; }
}
