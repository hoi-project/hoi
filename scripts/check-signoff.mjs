// Every commit a pull request adds carries a Developer Certificate of Origin sign-off: a
// `Signed-off-by:` trailer with the commit author's email (CONTRIBUTING.md; OpenSSF Baseline
// OSPS-LE-01.01). The Sign-off job in ci.yml runs it on the pull request's merge commit, whose first
// parent is the base branch and whose second parent is the pull request's head.
//
//   node scripts/check-signoff.mjs [<base>..<head>]    default HEAD^1..HEAD^2
//   node scripts/check-signoff.mjs origin/main..HEAD   before pushing a branch
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

// GitHub Apps commit as `<id>+<name>[bot]@users.noreply.github.com`. A bot cannot certify the DCO
// (Dependabot signs off with a different address); the maintainer who merges its change answers for it.
const BOT = /^\d+\+[a-z\d-]+\[bot\]@users\.noreply\.github\.com$/i;

/** The lowercased address of a `Name <address>` trailer value, or null. */
export function address(value) {
  const match = /<([^<>\s]+)>\s*$/.exec(value);
  return match ? match[1].toLowerCase() : null;
}

/** Whether a commit carries its author's sign-off (or needs none: a GitHub App). */
export function signedOff({ email, signoffs }) {
  if (BOT.test(email)) return true;
  return signoffs.some((value) => address(value) === email.toLowerCase());
}

const FIELD = '\x1f';
const RECORD = '\x1e';
const VALUE = '\x1d';

function git(cwd, ...args) {
  return execFileSync('git', args, { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
}

/**
 * The commits in `range`, oldest first, with their authors and `Signed-off-by:` values. Git reads the
 * trailers, so a sign-off counts only in the message's last paragraph. Merge commits are left out:
 * they join two checked histories (Update branch on GitHub makes one) and add no change of their own.
 */
export function readCommits(range, cwd = process.cwd()) {
  const format = ['%H', '%an', '%ae', '%s', '%(trailers:key=Signed-off-by,valueonly,separator=%x1d)'].join('%x1f');
  // --end-of-options: a range is never read as an option, whatever it starts with.
  const out = git(cwd, 'log', '--no-merges', '--reverse', `--format=${format}%x1e`, '--end-of-options', range, '--');
  return out.split(RECORD).map((record) => record.replace(/^\n/, '')).filter(Boolean).map((record) => {
    const [sha, name, email, subject, trailers = ''] = record.split(FIELD);
    return { sha, name, email, subject, signoffs: trailers.split(VALUE).map((value) => value.trim()).filter(Boolean) };
  });
}

/** What to tell the author of the commits that lack a sign-off. */
export function report(missing, base) {
  return [
    `${missing.length} commit(s) have no Signed-off-by: line with their author's email.`,
    'Signing off certifies the Developer Certificate of Origin (CONTRIBUTING.md, "Sign off your commits"):',
    ...missing.map((commit) => `  ${commit.sha.slice(0, 12)} ${commit.subject} (author ${commit.email})`),
    'Sign them off with the same email as the author, then push again:',
    `  git rebase --signoff ${base}`,
    '  git push --force-with-lease',
  ].join('\n');
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const range = process.argv[2] ?? 'HEAD^1..HEAD^2';
  const commits = readCommits(range);
  const missing = commits.filter((commit) => !signedOff(commit));
  if (missing.length === 0) {
    console.log(`${commits.length} commit(s) checked; each is signed off by its author.`);
  } else {
    // Where the branch left its base: the commit to rebase the unsigned ones onto.
    const [from, to] = range.split('..');
    let base = '<the commit your branch started from>';
    try {
      if (from && to && !to.startsWith('.')) base = git(process.cwd(), 'merge-base', '--end-of-options', from, to).trim().slice(0, 12);
    } catch {
      // no common history: keep the placeholder
    }
    console.error(report(missing, base));
    process.exit(1);
  }
}
