import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { address, readCommits, report, signedOff } from './check-signoff.mjs';

const script = fileURLToPath(new URL('./check-signoff.mjs', import.meta.url));
const ADA = 'Ada <ada@example.com>';

test('a sign-off counts when it carries the author email, in any case', () => {
  assert.equal(address(ADA), 'ada@example.com');
  assert.equal(address('Ada <Ada@Example.com>  '), 'ada@example.com');
  assert.equal(address('Ada'), null);
  assert.equal(address('Ada <ada@example.com> trailing'), null);
  assert.ok(signedOff({ email: 'ada@example.com', signoffs: [ADA] }));
  assert.ok(signedOff({ email: 'ADA@example.com', signoffs: ['Someone Else <x@example.com>', ADA] }));
  assert.ok(!signedOff({ email: 'ada@example.com', signoffs: [] }));
  assert.ok(!signedOff({ email: 'ada@example.com', signoffs: ['Ada <ada@example.org>'] }));
  assert.ok(!signedOff({ email: 'ada@example.com', signoffs: ['ada@example.com'] }));
});

test('GitHub App commits need no sign-off; a person with a bot-like name does', () => {
  assert.ok(signedOff({ email: '49699333+dependabot[bot]@users.noreply.github.com', signoffs: [] }));
  assert.ok(!signedOff({ email: 'dependabot[bot]@example.com', signoffs: [] }));
  assert.ok(!signedOff({ email: '1+ada@users.noreply.github.com', signoffs: [] }));
});

test('the report names each unsigned commit and how to fix it', () => {
  const text = report([{ sha: 'a'.repeat(40), subject: 'fix(web): one', email: 'ada@example.com' }], 'b'.repeat(12));
  assert.match(text, /1 commit\(s\)/);
  assert.match(text, /aaaaaaaaaaaa fix\(web\): one \(author ada@example\.com\)/);
  assert.match(text, /git rebase --signoff bbbbbbbbbbbb/);
});

/** A repository with a base branch and a pull request branch merged into it, like GitHub's merge ref. */
function repository() {
  const dir = mkdtempSync(join(tmpdir(), 'hoi-check-signoff-'));
  // Only this repository's settings: no global or system config (a signing key, hooks, a template).
  const env = { ...process.env, GIT_CONFIG_GLOBAL: join(dir, 'no-global-config'), GIT_CONFIG_NOSYSTEM: '1',
    GIT_AUTHOR_DATE: '2026-01-01T00:00:00Z', GIT_COMMITTER_DATE: '2026-01-01T00:00:00Z' };
  const git = (...args) => execFileSync('git', args, { cwd: dir, env, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
  const commit = (name, email, message) => {
    writeFileSync(join(dir, 'file.txt'), message);
    git('add', 'file.txt');
    git('-c', `user.name=${name}`, '-c', `user.email=${email}`, 'commit', '-q', '--no-verify', '-m', message);
  };
  git('init', '-q', '-b', 'main');
  commit('Ada', 'ada@example.com', 'chore: base');
  git('checkout', '-q', '-b', 'change');
  commit('Ada', 'ada@example.com', `fix: signed\n\nSigned-off-by: ${ADA}`);
  commit('Ada', 'ada@example.com', 'fix: unsigned');
  commit('Ada', 'ada@example.com', `fix: sign-off mid-message\n\nSigned-off-by: ${ADA}\n\nA later paragraph.`);
  commit('Ada', 'ada@example.com', 'fix: someone else signed\n\nSigned-off-by: Bob <bob@example.com>');
  commit('dependabot[bot]', '49699333+dependabot[bot]@users.noreply.github.com', 'chore(deps): bump');
  // Update branch: main moves on and is merged into the pull request without a sign-off.
  git('checkout', '-q', 'main');
  writeFileSync(join(dir, 'other.txt'), 'main moved');
  git('add', 'other.txt');
  git('-c', 'user.name=Bob', '-c', 'user.email=bob@example.com', 'commit', '-q', '--no-verify', '-m', 'docs: on main');
  git('checkout', '-q', 'change');
  git('-c', 'user.name=Ada', '-c', 'user.email=ada@example.com', 'merge', '-q', '--no-ff', '--no-edit', 'main');
  // GitHub's merge ref for the pull request: first parent the base, second the head.
  git('checkout', '-q', 'main');
  git('-c', 'user.name=GitHub', '-c', 'user.email=noreply@example.com', 'merge', '-q', '--no-ff', '--no-edit', 'change');
  return { dir, env, git };
}

test('reads the pull request commits and their trailers, leaving out merges and base commits', () => {
  const { dir } = repository();
  try {
    const commits = readCommits('HEAD^1..HEAD^2', dir);
    assert.deepEqual(commits.map((commit) => commit.subject),
      ['fix: signed', 'fix: unsigned', 'fix: sign-off mid-message', 'fix: someone else signed', 'chore(deps): bump']);
    assert.deepEqual(commits[0].signoffs, [ADA]);
    assert.deepEqual(commits[1].signoffs, []);
    assert.deepEqual(commits[2].signoffs, [], 'a sign-off outside the last paragraph is not a trailer');
    assert.equal(commits[0].email, 'ada@example.com');
    assert.match(commits[0].sha, /^[0-9a-f]{40}$/);
    assert.deepEqual(commits.filter((commit) => !signedOff(commit)).map((commit) => commit.subject),
      ['fix: unsigned', 'fix: sign-off mid-message', 'fix: someone else signed']);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('the command fails on unsigned commits and passes on signed ones', () => {
  const { dir, env, git } = repository();
  try {
    const run = (...args) => spawnSync(process.execPath, [script, ...args], { cwd: dir, env, encoding: 'utf8' });
    const failed = run();
    assert.equal(failed.status, 1);
    assert.match(failed.stderr, /3 commit\(s\)/);
    assert.match(failed.stderr, /fix: unsigned \(author ada@example\.com\)/);
    // The branch merged main, so main's tip is where the unsigned commits are rebased onto.
    const base = git('rev-parse', 'HEAD^1').trim().slice(0, 12);
    assert.match(failed.stderr, new RegExp(`git rebase --signoff ${base}\n`));
    const signed = git('rev-parse', 'HEAD^2~5').trim();
    const passed = run(`HEAD^1^1..${signed}`);
    assert.equal(passed.status, 0, passed.stderr);
    assert.match(passed.stdout, /1 commit\(s\) checked/);
    assert.notEqual(run('--output=x').status, 0, 'a range is never an option');
    assert.ok(!existsSync(join(dir, 'x')));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
