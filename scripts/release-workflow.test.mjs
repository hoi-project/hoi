import assert from 'node:assert/strict';
import { cpSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { execFileSync, spawnSync } from 'node:child_process';
import { applyVersion, LOCKFILES, planVersion, versionFiles } from './release-version.mjs';

const read = name => readFileSync(new URL(`../.github/workflows/${name}.yml`, import.meta.url), 'utf8').replaceAll('\r\n', '\n');

test('release preflight requires updater signing even when OS signing is explicitly skipped', () => {
  const workflow = read('release');
  const signing = workflow.split('\n  signing:\n')[1].split(/\n  [\w-]+:\n/)[0];
  assert.match(signing, /TAURI_SIGNING_PRIVATE_KEY: \$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY \}\}/);
  const script = signing.split('      - name: Refuse to build a release that would ship unsigned\n        run: |\n')[1]
    .split(/\n      (?:- |# )/)[0].trimEnd().replace(/^          /gm, '');
  assert.ok(script);
  const osKeys = ['APPLE_CERTIFICATE', 'APPLE_CERTIFICATE_PASSWORD', 'APPLE_SIGNING_IDENTITY', 'APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID', 'WINDOWS_CERTIFICATE', 'WINDOWS_CERTIFICATE_PASSWORD'];
  const run = patch => spawnSync('bash', ['-e', '-c', script], { encoding: 'utf8', env: {
    PATH: process.env.PATH, MINISIGN_SECRET_KEY: 'test-present', MINISIGN_PASSWORD: 'test-present',
    DESKTOP: 'true', ALLOW_UNSIGNED: 'true', ...patch,
  } });
  const absent = run({});
  assert.equal(absent.status, 1, absent.stderr);
  assert.match(absent.stderr, /Signed desktop updates need: TAURI_SIGNING_PRIVATE_KEY/);
  assert.equal(run({ DESKTOP: 'false' }).status, 0);
  assert.equal(run({ TAURI_SIGNING_PRIVATE_KEY: 'test-present' }).status, 0);
  assert.equal(run({ TAURI_SIGNING_PRIVATE_KEY: 'test-present', ALLOW_UNSIGNED: 'false' }).status, 1);
  const signed = run({ TAURI_SIGNING_PRIVATE_KEY: 'test-present', ALLOW_UNSIGNED: 'false', ...Object.fromEntries(osKeys.map(key => [key, 'test-present'])) });
  assert.equal(signed.status, 0, signed.stderr);
  assert.equal(run({ DESKTOP: 'false', MINISIGN_SECRET_KEY: '' }).status, 1);
});

// Exercise the actual job gates with representative GitHub contexts, including failed checks.
function gate(workflow, job, context) {
  const source = read(workflow);
  const section = source.split(`\n  ${job}:\n`)[1]?.split(/\n  [\w-]+:\n/)[0];
  assert.ok(section, `${workflow}: ${job}`);
  const value = section.match(/^    if: (.+)(?:\n((?:      [^\n]*\n)*))?/m);
  assert.ok(value, `${workflow}: ${job} condition`);
  // GitHub permits hyphens in property names; JavaScript requires bracket access.
  const expression = (value[1] === '>-' ? value[2].trim().replaceAll('\n', ' ') : value[1]).replace(/^\$\{\{\s*([\s\S]*?)\s*\}\}$/, '$1')
    .replace(/\bneeds\.([\w-]+)/g, (_, name) => `needs[${JSON.stringify(name)}]`);
  return Function('github', 'vars', 'needs', 'inputs', 'always', 'cancelled', `return (${expression});`)(
    context.github, context.vars, context.needs, context.inputs, () => true, () => context.cancelled ?? false);
}

function context(event, owner = 'hoi-project', releases = '', tier = 'major') {
  return {
    github: { event_name: event, repository_owner: owner, repository: `${owner}/hoi`, event: { created: true, repository: { private: false } } },
    vars: { HOI_RELEASES_ENABLED: releases },
    inputs: { desktop: false },
    needs: Object.fromEntries(['plan', 'checks', 'long', 'security', 'npm-audit', 'signing', 'draft', 'node', 'desktop', 'setup']
      .map(job => [job, { result: ['desktop', 'setup'].includes(job) || (tier !== 'major' && ['long', 'security', 'npm-audit'].includes(job)) ? 'skipped' : 'success', outputs: { tier } }])),
  };
}

const job = (workflow, name) => read(workflow).split(`\n  ${name}:\n`)[1]?.split(/\n  [\w-]+:\n/)[0];

test('tag retry configuration fails during planning before expensive checks', () => {
  const plan = job('release', 'plan');
  assert.match(plan, /^    environment: release$/m);
  const step = plan.split('      - name: Require tag retry configuration before checks\n')[1]?.split('\n      - ')[0];
  assert.ok(step);
  assert.match(step, /if: steps\.commit\.outputs\.previous_tag_sha != ''/);
  assert.match(step, /RELEASE_TAG_APP_CLIENT_ID: \$\{\{ vars\.HOI_RELEASE_TAG_APP_CLIENT_ID \}\}/);
  assert.match(step, /HAS_RELEASE_TAG_APP_PRIVATE_KEY: \$\{\{ secrets\.HOI_RELEASE_TAG_APP_PRIVATE_KEY != '' \}\}/);
  const script = step.split('        run: |\n')[1].trimEnd().replace(/^ {10}/gm, '');
  for (const client of ['', 'test-client']) for (const key of ['false', 'true']) {
    const result = spawnSync('bash', ['-e', '-c', script], { encoding: 'utf8', env: {
      PATH: process.env.PATH, RELEASE_TAG_APP_CLIENT_ID: client, HAS_RELEASE_TAG_APP_PRIVATE_KEY: key,
    } });
    assert.ifError(result.error);
    assert.equal(result.status, client && key === 'true' ? 0 : 1, result.stderr);
    if (!client) assert.match(result.stdout, /variable HOI_RELEASE_TAG_APP_CLIENT_ID/);
    if (key === 'false') assert.match(result.stdout, /secret HOI_RELEASE_TAG_APP_PRIVATE_KEY/);
    assert.doesNotMatch(result.stdout, /test-client/);
  }
  for (const name of ['checks', 'long', 'security', 'npm-audit']) {
    assert.match(job('release', name), /^    needs: plan$/m, `${name} waits for planning`);
  }
});

test('draft creation and recovery avoid target permissions and preserve source provenance', t => {
  const script = job('release', 'draft').split('      - id: draft\n')[1]
    .split('        run: |\n')[1].trimEnd().replace(/^ {10}/gm, '');
  const cwd = mkdtempSync(join(tmpdir(), 'hoi-draft-'));
  t.after(() => rmSync(cwd, { recursive: true, force: true }));
  const mock = `gh() {
  printf '%s\\n' "$*" >> calls
  case "$1 $2" in
    'release view') [ "$EXISTING" = true ] || return 1; echo https://api.github.com/repos/example/hoi/releases/123 ;;
    'release create') echo https://github.com/example/hoi/releases/tag/untagged-example ;;
    'api --method')
      case "$*" in *target_commitish=*) echo 'gh: Resource not accessible by integration (HTTP 403)' >&2; return 1 ;; esac
      [ "$API_FAIL" != true ] || return 1
      printf '%s\\n' "$RESPONSE_ID"
      ;;
    'api repos/example/hoi/commits/'*) printf '%s\\n' "$REMOTE_SHA" ;;
    'api https://api.github.com/repos/example/hoi/releases/123')
      printf '123\\t%s\\t%s\\t%s\\t%s\\n' "$DRAFT" "$EXISTING_PRERELEASE" "$ASSETS" "$BODY64" ;;
    'api repos/example/hoi/releases?per_page=100') echo null ;;
    *) return 99 ;;
  esac
}
`;
  const sha = 'a'.repeat(40);
  const run = (patch = {}) => {
    writeFileSync(join(cwd, 'output'), '');
    writeFileSync(join(cwd, 'calls'), '');
    const result = spawnSync('bash', ['-e', '-c', mock + script], { cwd, encoding: 'utf8', env: {
      PATH: process.env.PATH, GH_REPO: 'example/hoi', TAG: 'v1.0.0-preview.2', SOURCE_SHA: sha,
      DESKTOP: 'true', SIGNED: 'false', PRERELEASE: 'true', PREVIOUS_SHA: '',
      GITHUB_OUTPUT: 'output', REMOTE_SHA: sha, RESPONSE_ID: '123', EXISTING: 'false',
      DRAFT: 'true', EXISTING_PRERELEASE: 'true', ASSETS: '0', BODY64: '', ...patch,
    } });
    assert.ifError(result.error);
    return { ...result, output: readFileSync(join(cwd, 'output'), 'utf8'), calls: readFileSync(join(cwd, 'calls'), 'utf8') };
  };
  const created = run();
  assert.equal(created.status, 0, created.stderr);
  assert.equal(created.output, 'id=123\n');
  assert.match(created.calls, /api --method POST .*tag_name=v1\.0\.0-preview\.2/);
  assert.match(created.calls, new RegExp(`<!-- hoi-release-source: ${sha} -->`));
  assert.doesNotMatch(created.calls, /target_commitish/);
  assert.doesNotMatch(created.calls, /releases\?per_page/);
  const boundBody = `Release notes.\n\n<!-- hoi-release-source: ${sha} -->`;
  for (const body of ['', 'Release notes.\nKeep this text.']) {
    const recovered = run({ EXISTING: 'true', BODY64: Buffer.from(body).toString('base64') });
    assert.equal(recovered.status, 0, recovered.stderr);
    assert.equal(recovered.output, 'id=123\n');
    assert.match(recovered.calls, /api --method PATCH/);
    assert.ok(recovered.calls.includes(body));
    assert.match(recovered.calls, new RegExp(`<!-- hoi-release-source: ${sha} -->`));
    assert.doesNotMatch(recovered.calls, /target_commitish/);
    assert.doesNotMatch(recovered.calls, /api --method POST/);
  }
  const resumed = run({ EXISTING: 'true', BODY64: Buffer.from(boundBody).toString('base64'), ASSETS: '3' });
  assert.equal(resumed.status, 0, resumed.stderr);
  assert.equal(resumed.output, 'id=123\n');
  assert.doesNotMatch(resumed.calls, /api --method/, 'a matching draft needs no metadata writes');
  const reclassified = run({ EXISTING: 'true', BODY64: Buffer.from(boundBody).toString('base64'), EXISTING_PRERELEASE: 'false' });
  assert.equal(reclassified.status, 0, reclassified.stderr);
  assert.match(reclassified.calls, /api --method PATCH/);
  assert.doesNotMatch(reclassified.calls, /target_commitish/);
  for (const patch of [
    { REMOTE_SHA: 'b'.repeat(40) }, { API_FAIL: 'true' }, { RESPONSE_ID: 'null' },
    { EXISTING: 'true', DRAFT: 'false' },
    { EXISTING: 'true', ASSETS: '1' },
    { EXISTING: 'true', BODY64: Buffer.from(`<!-- hoi-release-source: ${'b'.repeat(40)} -->`).toString('base64') },
    { EXISTING: 'true', PREVIOUS_SHA: 'b'.repeat(40) },
  ]) {
    const rejected = run(patch);
    assert.notEqual(rejected.status, 0, JSON.stringify(patch));
    assert.equal(rejected.output, '');
  }
});

test('only verified prior checks allow signing after validation jobs are skipped', () => {
  const ctx = context('workflow_dispatch');
  const checks = ['checks', 'long', 'security', 'npm-audit'];
  for (const name of checks) ctx.needs[name].result = 'skipped';
  assert.equal(gate('release', 'signing', ctx), false);
  ctx.needs.plan.outputs.reuse_checks = 'true';
  for (const name of checks) assert.equal(gate('release', name, ctx), false, name);
  assert.equal(gate('release', 'signing', ctx), true);
  for (const name of ['plan', ...checks]) {
    const failed = structuredClone(ctx);
    failed.needs[name].result = 'failure';
    assert.equal(gate('release', 'signing', failed), false, name);
  }
  assert.equal(gate('release', 'signing', { ...ctx, cancelled: true }), false);
  assert.match(job('release', 'plan'), /reuse_checks: \$\{\{ steps\.resume\.outputs\.reuse_checks \}\}/);
  assert.match(job('release', 'plan'), /run: node release-tools\/scripts\/release-resume\.mjs/);
});

test('release controls offer tag and action menus without a run ID', () => {
  const workflow = read('release');
  const inputs = workflow.split('  workflow_dispatch:\n    inputs:\n')[1].split('\npermissions:')[0];
  assert.match(inputs, /tag:\n        description: "Tag"\n        type: choice/);
  assert.match(inputs, /action:\n        description: "Action"\n        type: choice/);
  for (const action of ['Resume', 'Build', 'Check', 'Update tag']) assert.ok(inputs.includes(`          - ${action}\n`));
  assert.doesNotMatch(inputs, /resume_checks_from|retry_unpublished|type: string/);
  const plan = job('release', 'plan');
  assert.match(plan, /RETRY_UNPUBLISHED: \$\{\{ inputs\.action == 'Update tag' \}\}/);
  assert.match(plan, /name: Find passed checks\n        if: inputs\.action == 'Resume'/);
  assert.doesNotMatch(plan, /RESUME_RUN_ID:/);
  const checkOnly = context('workflow_dispatch');
  checkOnly.inputs.action = 'Check';
  for (const name of ['checks', 'long', 'security', 'npm-audit']) assert.equal(gate('release', name, checkOnly), true, name);
  assert.equal(gate('release', 'signing', checkOnly), true, 'finish reusable validation with signing preflight');
  assert.equal(gate('release', 'draft', checkOnly), false);
  checkOnly.needs.draft.result = 'skipped';
  for (const name of ['node', 'desktop', 'setup']) assert.equal(gate('release', name, checkOnly), false, name);
});

test('tag selection uses the chosen source and resolves Current version without typing', t => {
  const step = job('release', 'plan').split('      - name: Select release source\n')[1].split('\n      - ')[0];
  const script = step.split('        run: |\n')[1].trimEnd().replace(/^ {10}/gm, '');
  const cwd = mkdtempSync(join(tmpdir(), 'hoi-release-selection-'));
  t.after(() => rmSync(cwd, { recursive: true, force: true }));
  const git = (...args) => execFileSync('git', args, { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
  const commit = () => {
    git('-c', 'user.name=Release Test', '-c', 'user.email=release@example.com', '-c', 'commit.gpgsign=false', 'commit', '--allow-empty', '-m', 'test');
    return git('rev-parse', 'HEAD');
  };
  git('init', '-b', 'main');
  const tagged = commit();
  git('tag', 'v1.0.0-preview.2');
  const main = commit();
  const run = (tag, action) => {
    git('checkout', '--detach', main);
    writeFileSync(join(cwd, 'env'), '');
    const result = spawnSync('bash', ['-e', '-c', 'node() { echo v1.0.0-preview.2; }\n' + script], {
      cwd, encoding: 'utf8', env: { PATH: process.env.PATH, REQUESTED_TAG: tag, ACTION: action,
        GITHUB_ENV: 'env', GITHUB_STEP_SUMMARY: 'summary' },
    });
    assert.ifError(result.error);
    return result;
  };
  for (const tag of ['v1.0.0-preview.2', 'Current version']) for (const action of ['Resume', 'Build', 'Check', 'Update tag']) {
    const selected = run(tag, action);
    assert.equal(selected.status, 0, selected.stderr);
    assert.equal(git('rev-parse', 'HEAD'), action === 'Update tag' ? main : tagged);
    assert.equal(readFileSync(join(cwd, 'env'), 'utf8'), 'TAG=v1.0.0-preview.2\n');
  }
  for (const [tag, action] of [['main', 'Build'], ['v9.9.9', 'Build'], ['Current version', 'Skip checks']]) {
    assert.notEqual(run(tag, action).status, 0);
  }
});

test('an unpublished retry validates a descendant on main and advances only the checked tag', t => {
  const step = (jobName, name) => {
    const section = job('release', jobName).split(`      - name: ${name}\n`)[1]?.split('\n      - ')[0];
    const script = section?.split('        run: |\n')[1]?.trimEnd().replace(/^ {10}/gm, '');
    assert.ok(script, name);
    return script;
  };
  const pin = step('plan', 'Pin all following jobs to this commit');
  const advance = step('draft', 'Advance the unpublished tag');
  const cwd = mkdtempSync(join(tmpdir(), 'hoi-release-retry-'));
  t.after(() => rmSync(cwd, { recursive: true, force: true }));
  const git = (...args) => execFileSync('git', args, { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
  const commit = message => {
    git('-c', 'user.name=Release Test', '-c', 'user.email=release@example.com', '-c', 'commit.gpgsign=false', 'commit', '--allow-empty', '-m', message);
    return git('rev-parse', 'HEAD');
  };
  git('init', '-b', 'main');
  git('init', '--bare', 'remote.git');
  git('remote', 'add', 'origin', 'remote.git');
  const base = commit('before the release');
  const previous = commit('failed release');
  const tag = 'v1.0.0-preview.2';
  const ref = `refs/tags/${tag}`;
  git('tag', tag, previous);
  git('push', 'origin', ref);
  const source = commit('fix on main');

  const run = (script, env = {}) => {
    const result = spawnSync('bash', ['-e', '-c', script], { cwd, encoding: 'utf8', env: {
      ...process.env, TAG: tag, GH_REPO: 'example/hoi', GITHUB_OUTPUT: 'github-output', GITHUB_STEP_SUMMARY: 'summary',
      GITHUB_EVENT_NAME: 'workflow_dispatch', GITHUB_REF: 'refs/heads/main', RETRY_UNPUBLISHED: 'true', ...env,
    } });
    assert.ifError(result.error);
    return result;
  };
  assert.match(run(pin, { GITHUB_REF: 'refs/heads/topic' }).stderr, /running Release on main/);
  git('checkout', '--detach', base);
  assert.match(run(pin).stderr, /must descend/);
  git('switch', 'main');
  git('-c', 'user.name=Release Test', '-c', 'user.email=release@example.com', '-c', 'tag.gpgsign=false',
    'tag', '-a', 'v1.0.0-preview.3', '-m', 'annotated', previous);
  assert.match(run(pin, { TAG: 'v1.0.0-preview.3' }).stderr, /Only lightweight/);
  const planned = run(pin);
  assert.equal(planned.status, 0, planned.stderr);
  assert.deepEqual(readFileSync(join(cwd, 'github-output'), 'utf8').trim().split(/\r?\n/),
    [`previous_tag_sha=${previous}`, `sha=${source}`, `tag=${tag}`]);
  const gh = `gh() {
  case "$2" in
    */immutable-releases) printf '%s\\n' "$MOCK_IMMUTABLE" ;;
    --paginate) printf '%s\\n' "$MOCK_RELEASES" ;;
    */commits/*) git --git-dir=remote.git rev-parse "refs/tags/$TAG" ;;
    *) return 99 ;;
  esac
}
`;
  const update = env => run(gh + advance, { PREVIOUS_SHA: previous, SOURCE_SHA: source, MOCK_IMMUTABLE: 'true', ...env });
  const remoteTag = () => git('--git-dir=remote.git', 'rev-parse', ref);
  for (const [env, error] of [
    [{ MOCK_IMMUTABLE: 'false' }, /Enable immutable releases/],
    [{ MOCK_RELEASES: 'true' }, /Discard the existing draft/],
    [{ MOCK_RELEASES: 'false' }, /already published/],
  ]) {
    const result = update(env);
    assert.equal(result.status, 1, result.stderr);
    assert.match(result.stderr, error);
    assert.equal(remoteTag(), previous);
  }
  const advanced = update();
  assert.equal(advanced.status, 0, advanced.stderr);
  assert.equal(remoteTag(), source);
  const newer = commit('second fix on main');
  const stale = update({ SOURCE_SHA: newer });
  assert.equal(stale.status, 1, stale.stderr);
  assert.match(stale.stderr, /stale info/);
  assert.equal(remoteTag(), source);
});

test('major suites run only a commit in the triggering revision’s history', t => {
  const workflow = read('release-major');
  const verify = job('release-major', 'verify-major');
  assert.match(verify, /ref: \$\{\{ github\.sha \}\}/);
  assert.match(verify, /fetch-depth: 0/);
  assert.doesNotMatch(workflow, /ref: \$\{\{ inputs\.ref \}\}/);
  for (const name of ['browser-e2e', 'webrtc-browsers']) {
    assert.match(job('release-major', name), /ref: \$\{\{ needs\.verify-major\.outputs\.revision \}\}/);
  }
  const script = verify.match(/        run: \|\n((?: {10}[^\n]*\n)+)/)?.[1].replace(/^ {10}/gm, '');
  assert.ok(script);
  const cwd = mkdtempSync(join(tmpdir(), 'hoi-release-ref-'));
  t.after(() => rmSync(cwd, { recursive: true, force: true }));
  const git = (...args) => execFileSync('git', args, { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
  const commit = message => {
    git('-c', 'user.name=Release Test', '-c', 'user.email=release@example.com', '-c', 'commit.gpgsign=false', 'commit', '--allow-empty', '-m', message);
    return git('rev-parse', 'HEAD');
  };
  git('init', '-b', 'main');
  const ancestor = commit('earlier release');
  git('switch', '-c', 'other');
  const unrelated = commit('off-branch');
  git('switch', 'main');
  const head = commit('triggering commit');
  const check = requested => spawnSync('bash', ['-e', '-c', script], {
    cwd, encoding: 'utf8', env: { ...process.env, REQUESTED_COMMIT: requested, GITHUB_OUTPUT: 'github-output' },
  });
  for (const requested of ['refs/heads/other', unrelated]) {
    const result = check(requested);
    assert.ifError(result.error);
    assert.equal(result.status, 1, result.stderr);
    assert.equal(git('rev-parse', 'HEAD'), head);
  }
  for (const requested of [head, ancestor]) {
    const result = check(requested);
    assert.ifError(result.error);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(git('rev-parse', 'HEAD'), requested);
    assert.equal(readFileSync(join(cwd, 'github-output'), 'utf8').trim().split(/\r?\n/).at(-1), `revision=${requested}`);
  }
});

test('the Pages build starts without restoring dependency caches', () => {
  const build = job('website', 'build');
  assert.match(build, /package-manager-cache: false/);
  assert.doesNotMatch(build, /cache: pnpm|uses: Swatinem\/rust-cache@/);
});

test('release workflow version checks run without installed dependencies', async t => {
  const base = mkdtempSync(join(tmpdir(), 'hoi-release-workflow-'));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  const scripts = readdirSync(new URL('./', import.meta.url))
    .filter(name => name.endsWith('.mjs')).map(name => `scripts/${name}`);
  const paths = new Set([...versionFiles().map(file => file.path), ...LOCKFILES,
    'package.json', 'release.json', ...scripts]);
  for (const path of paths) cpSync(new URL(`../${path}`, import.meta.url), join(base, path));

  // The major gate needs a major release even when the real checkout carries a different tier.
  const version = '1.0.0-preview.1';
  applyVersion(planVersion(version, {}, base), base);
  for (const [workflow, name, step, variable] of [
    ['release-major', 'verify-major', 'Refuse long suites for a non-major version', 'VERSION'],
    ['release', 'plan', 'Validate versions, channel and release checks', 'TAG'],
    ['release', 'signing', "The tree carries the tag's version", 'TAG'],
  ]) {
    await t.test(`${workflow} / ${name}`, () => {
      const section = job(workflow, name)?.split(`      - name: ${step}\n`)[1]?.split('\n      - ')[0];
      const run = section?.match(/^        run: (.+)$/m)?.[1];
      assert.ok(run, step);
      const script = run === '|' ? section.split('        run: |\n')[1].replace(/^          /gm, '') : run;
      const check = tag => spawnSync('bash', ['-e', '-c', script], {
        cwd: base, encoding: 'utf8', env: { PATH: process.env.PATH, GITHUB_OUTPUT: 'github-output', [variable]: tag },
      });
      const match = check(`v${version}`);
      assert.ifError(match.error);
      assert.equal(match.status, 0, match.stderr);
      const mismatch = check('v2.0.0-preview.1');
      assert.equal(mismatch.status, 1, mismatch.stderr);
      assert.match(mismatch.stderr, /Not version 2\.0\.0-preview\.1/);
    });
  }
});

test('routine checks start by themselves in the public hoi-project repository, anywhere by hand', () => {
  const ctx = (event, { owner = 'hoi-project', isPrivate = false, head } = {}) => ({ vars: {}, github: {
    event_name: event, repository_owner: owner, repository: `${owner}/hoi`,
    event: { repository: { private: isPrivate }, ...(head ? { pull_request: { head: { repo: { full_name: head } } } } : {}) } } });
  // One gate per workflow, on its changes job; every other job needs it.
  for (const workflow of ['ci', 'security']) {
    for (const event of ['push', 'schedule']) assert.equal(gate(workflow, 'changes', ctx(event)), true, `${workflow}: ${event}`);
    assert.equal(gate(workflow, 'changes', ctx('pull_request', { head: 'contributor/hoi' })), true, `${workflow}: fork pull request`);
    assert.equal(gate(workflow, 'changes', ctx('push', { isPrivate: true })), false, `${workflow}: hoi-project/hoi while private`);
    assert.equal(gate(workflow, 'changes', ctx('push', { owner: 'pclshm', isPrivate: true })), false, `${workflow}: private workspace push`);
    assert.equal(gate(workflow, 'changes', ctx('pull_request', { owner: 'pclshm', isPrivate: true })), false, `${workflow}: private workspace pull request`);
    assert.equal(gate(workflow, 'changes', ctx('push', { owner: 'contributor' })), false, `${workflow}: a fork's own push`);
    for (const owner of ['hoi-project', 'pclshm', 'contributor']) {
      assert.equal(gate(workflow, 'changes', ctx('workflow_dispatch', { owner, isPrivate: owner === 'pclshm' })), true, `${workflow}: by hand in ${owner}`);
    }
    const jobs = read(workflow).split('\njobs:\n')[1].split(/\n(?=  [\w-]+:\n)/);
    for (const [name, body] of jobs.map((part) => [part.trim().split(':')[0], part])) {
      if (name !== 'changes') assert.match(body, /\n    needs: (changes|\[changes)/, `${workflow}: ${name} needs changes`);
    }
  }
  const outputs = (o) => ({ needs: { changes: { result: 'success', outputs: o }, rust: { result: 'success' } }, github: { event_name: 'pull_request' } });
  assert.equal(gate('ci', 'rust', outputs({ rust: 'false' })), false);
  assert.equal(gate('ci', 'rust', outputs({ rust: 'true' })), true);
  assert.equal(gate('ci', 'web', outputs({ web: 'false' })), false);
  assert.equal(gate('ci', 'web', outputs({ web: 'true' })), true);
  assert.doesNotMatch(job('ci', 'quick'), /^    if:/m, 'quick checks run whenever CI runs');
  assert.equal(gate('ci', 'sonarcloud', outputs({})), true, 'Sonar after Rust');
  assert.equal(gate('ci', 'sonarcloud', { ...outputs({}), needs: { changes: { result: 'success' }, rust: { result: 'skipped' } } }), true, 'Sonar without Rust changes');
  assert.equal(gate('ci', 'sonarcloud', { ...outputs({}), needs: { changes: { result: 'success' }, rust: { result: 'failure' } } }), false, 'no Sonar after failed tests');
  assert.equal(gate('security', 'codeql', outputs({ codeql: '[]' })), false);
  assert.equal(gate('security', 'codeql', outputs({ codeql: '["rust"]' })), true);
  for (const name of ['rust-deps', 'vet', 'npm']) {
    assert.equal(gate('security', name, outputs({ deps: 'false' })), false, name);
    assert.equal(gate('security', name, outputs({ deps: 'true' })), true, name);
  }
  assert.equal(gate('security', 'dependency-review', outputs({ deps: 'true' })), true);
  assert.equal(gate('security', 'dependency-review', { ...outputs({ deps: 'true' }), github: { event_name: 'schedule' } }), false);
  assert.equal(gate('security', 'workflows', outputs({ workflows: 'true' })), true);
  assert.equal(gate('scorecard', 'analysis', ctx('schedule')), true);
  assert.equal(gate('scorecard', 'analysis', ctx('workflow_dispatch', { owner: 'pclshm', isPrivate: true })), false, 'Scorecard needs a public repository');
  assert.equal(gate('weekly', 'visibility', ctx('schedule')), true);
  assert.equal(gate('weekly', 'visibility', ctx('workflow_dispatch')), false);
  const weekly = (event, isPublic) => ({ github: { event_name: event }, needs: { visibility: { outputs: { public: isPublic } } } });
  assert.equal(gate('weekly', 'suites', weekly('schedule', 'false')), false);
  assert.equal(gate('weekly', 'suites', weekly('schedule', 'true')), true);
  assert.equal(gate('weekly', 'suites', weekly('workflow_dispatch', '')), true, 'by hand anywhere');
  assert.equal(gate('website', 'build', { vars: {} }), false);
  assert.equal(gate('website', 'build', { vars: { HOI_PAGES_ENABLED: 'true' } }), true);
  for (const workflow of ['ci', 'security', 'scorecard', 'weekly', 'website', 'release', 'release-version', 'release-checks', 'release-major']) {
    assert.doesNotMatch(read(workflow), /HOI_ACTIONS_ENABLED/, workflow);
    assert.doesNotMatch(read(workflow), /cargo install (cargo-audit|cargo-vet|wasm-bindgen-cli|cargo-nextest|cargo-llvm-cov)/, `${workflow}: prebuilt tools`);
  }
});

test('the private test mode keeps macOS and Windows out', () => {
  const major = read('release-major');
  assert.match(major, /linux_only:\n        description:/);
  assert.match(job('release-major', 'installers'), /if: \$\{\{ !inputs\.linux_only \}\}/);
  assert.match(job('release-major', 'rust'), /inputs\.linux_only && '\["ubuntu-latest"\]'/);
  assert.match(job('release-major', 'desktop'), /inputs\.linux_only && '\["ubuntu-22\.04"\]'/);
  assert.match(job('weekly', 'suites'), /linux_only: \$\{\{ github\.event_name == 'workflow_dispatch' && inputs\.linux_only \}\}/);
});

test('Release and Weekly only schedule mobile builds with a prepared package version', t => {
  for (const [workflow, name] of [['release', 'long'], ['weekly', 'suites']]) {
    assert.match(job(workflow, name), /uses: \.\/\.github\/workflows\/release-major\.yml/);
  }
  const verify = job('release-major', 'verify-major');
  const step = verify.split('      - name: Check mobile build readiness\n')[1]?.split('\n      - ')[0];
  assert.ok(step, 'the shared preflight reads the package settings');
  assert.ok(verify.indexOf('git checkout --detach') < verify.indexOf('      - name: Check mobile build readiness'));
  for (const platform of ['android', 'ios']) {
    assert.match(verify, new RegExp(`^      ${platform}: \\$\\{\\{ steps\\.mobile\\.outputs\\.${platform} \\}\\}$`, 'm'));
  }
  const script = step.split('        run: |\n')[1].trimEnd().replace(/^ {10}/gm, '');
  const cwd = mkdtempSync(join(tmpdir(), 'hoi-mobile-build-'));
  t.after(() => rmSync(cwd, { recursive: true, force: true }));
  const out = join(cwd, 'github-output');
  const cases = [
    [{}, { android: 'false', ios: 'false' }],
    [{ android: '{"version":null}', ios: 'not JSON' }, { android: 'false', ios: 'false' }],
    [{ android: '{"version":"0.1.0-preview.1"}', ios: '{"version":"draft"}' }, { android: 'true', ios: 'false' }],
    [{ android: '{"version":123}', ios: '{"version":"1.2.3"}' }, { android: 'false', ios: 'true' }],
    [{ android: '{"version":"0.1.0"}', ios: '{"version":"1.2.3"}' }, { android: 'true', ios: 'true' }],
  ];
  for (const [configs, expected] of cases) {
    for (const [platform, config] of Object.entries(configs)) {
      const dir = join(cwd, 'apps', 'mobile', platform);
      mkdirSync(dir, { recursive: true });
      writeFileSync(join(dir, 'build.json'), config);
    }
    writeFileSync(out, '');
    const result = spawnSync('bash', ['-e', '-c', script], {
      cwd, encoding: 'utf8', env: { ...process.env, GITHUB_OUTPUT: 'github-output' },
    });
    assert.ifError(result.error);
    assert.equal(result.status, 0, result.stderr);
    const outputs = Object.fromEntries(readFileSync(out, 'utf8').trim().split(/\r?\n/).map(line => line.split('=')));
    assert.deepEqual(outputs, expected);
    for (const weekly of [false, true]) for (const linux_only of [false, true]) {
      const context = { inputs: { weekly, linux_only }, needs: { 'verify-major': { outputs } } };
      assert.equal(gate('release-major', 'android', context), expected.android === 'true');
      assert.equal(gate('release-major', 'ios', context), expected.ios === 'true' && !linux_only);
    }
  }
});

test('major release browser jobs use Ubuntu supported by the pinned Playwright', () => {
  assert.match(job('release-major', 'browser-e2e'), /^    runs-on: ubuntu-24\.04$/m);
  const webrtc = job('release-major', 'webrtc-browsers');
  assert.match(webrtc, /^    runs-on: ubuntu-24\.04$/m);
  assert.match(webrtc, /^        browser: \[firefox, webkit\]$/m);
});

test('major release browser builds can use a tagged checkout without the offline fixture', t => {
  const cwd = mkdtempSync(join(tmpdir(), 'hoi-browser-publication-'));
  t.after(() => rmSync(cwd, { recursive: true, force: true }));
  const publication = { schema: 1, site: 'https://example.org', repository: 'https://example.org/hoi',
    contact: '', downloads: 'https://example.org/releases', manifestKey: 'manifest', updaterKey: 'updater' };
  writeFileSync(join(cwd, 'publication.json'), JSON.stringify(publication));
  for (const name of ['browser-e2e', 'webrtc-browsers']) {
    const step = job('release-major', name).split('      - name: Build the app without a live release feed\n')[1]?.split('\n      - ')[0];
    const script = step?.split('        run: |\n')[1]?.trimEnd().replace(/^ {10}/gm, '');
    assert.ok(script, name);
    const build = 'pnpm() { test "$1" = build:web && cat "$HOI_PUBLICATION_FILE"; }\n';
    const result = spawnSync('bash', ['-e', '-c', build + script], {
      cwd, encoding: 'utf8', env: { ...process.env, RUNNER_TEMP: '.' },
    });
    assert.ifError(result.error);
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(JSON.parse(result.stdout), { ...publication, downloads: '', manifestKey: '', updaterKey: '' }, name);
  }
});

test('called release suites have no gate of their own: the caller decides', () => {
  assert.doesNotMatch(job('release-checks', 'checks'), /^    if:/m);
  assert.doesNotMatch(job('release-major', 'verify-major'), /^    if:/m);
  assert.match(job('release-major', 'verify-major'), /if: \$\{\{ !inputs\.weekly \}\}/);
  assert.match(job('release', 'checks'), /uses: \.\/\.github\/workflows\/release-checks\.yml/);
  assert.match(job('release', 'long'), /uses: \.\/\.github\/workflows\/release-major\.yml/);
});

test('manual releases reach validation, signing, upload and manifest with automatic switches disabled', () => {
  for (const tier of ['patch', 'minor', 'major']) {
    const ctx = context('workflow_dispatch', '', '', tier);
    for (const job of ['plan', 'signing', 'draft', 'node', 'manifest']) assert.equal(gate('release', job, ctx), true, `${tier}: ${job}`);
    assert.equal(gate('release', 'desktop', ctx), false);
    assert.equal(gate('release', 'setup', ctx), false);
  }
});

test('automatic releases need the switch in the hoi-project repository and validate before publishing assets', () => {
  for (const owner of ['hoi-project', 'contributor']) for (const releases of ['', 'true']) {
    assert.equal(gate('release', 'plan', context('push', owner, releases)), owner === 'hoi-project' && releases === 'true');
  }
  const ctx = context('push', 'hoi-project', 'true');
  assert.equal(gate('release', 'desktop', ctx), true);
  assert.equal(gate('release', 'setup', ctx), true);
  for (const job of ['plan', 'checks', 'long', 'security', 'npm-audit']) {
    const failed = structuredClone(ctx);
    failed.needs[job].result = 'failure';
    assert.equal(gate('release', 'signing', failed), false, job);
  }
});

test('advancing an existing tag cannot trigger another release and requires successful validation', () => {
  const retag = context('push', 'hoi-project', 'true');
  retag.github.event.created = false;
  assert.equal(gate('release', 'plan', retag), false);
  const failedSigning = context('workflow_dispatch');
  failedSigning.needs.signing.result = 'failure';
  assert.equal(gate('release', 'draft', failedSigning), false);
});

test('failed or cancelled builds cannot produce a signed manifest', () => {
  for (const job of ['plan', 'node', 'desktop', 'setup']) {
    const ctx = context('workflow_dispatch');
    ctx.needs[job].result = 'failure';
    assert.equal(gate('release', 'manifest', ctx), false, job);
  }
  const ctx = { ...context('workflow_dispatch'), cancelled: true };
  for (const job of ['signing', 'draft', 'node', 'manifest']) assert.equal(gate('release', job, ctx), false, job);
});

test('the node setup app is built by the same switch as the desktop app and uploaded to the same draft', () => {
  const workflow = read('release');
  const job = name => workflow.split(`\n  ${name}:\n`)[1].split(/\n  [\w-]+:\n/)[0];
  for (const name of ['desktop', 'setup']) {
    assert.match(job(name), /inputs\.desktop/);
    assert.match(job(name), /releaseId: \$\{\{ needs\.draft\.outputs\.id \}\}/);
    assert.match(job(name), /ubuntu-22\.04-arm/);
  }
  assert.match(job('setup'), /projectPath: apps\/setup/);
  assert.match(job('desktop'), /projectPath: apps\/desktop/);
});

test('GitHub pre-release classification comes from checked release metadata', () => {
  const workflow = read('release');
  assert.match(workflow, /node scripts\/release-version\.mjs --check "\$TAG" --github-output/);
  assert.match(workflow, /prerelease: \$\{\{ steps\.tier\.outputs\.prerelease \}\}/);
  assert.match(workflow, /PRERELEASE: \$\{\{ needs\.plan\.outputs\.prerelease \}\}/);
  assert.match(workflow, /-F draft=true -F "prerelease=\$PRERELEASE"/);
});

test('version commits and manual runs use the same automatic opt-in policy', () => {
  assert.equal(gate('release-version', 'plan', context('workflow_dispatch')), true);
  for (const owner of ['hoi-project', 'contributor']) for (const releases of ['', 'true']) {
    assert.equal(gate('release-version', 'plan', context('push', owner, releases)), owner === 'hoi-project' && releases === 'true');
  }
  const ctx = context('push', 'hoi-project', 'true');
  for (const release of ['true', 'false']) {
    ctx.needs.plan.outputs.release = release;
    assert.equal(gate('release-version', 'start', ctx), release === 'true');
  }
});

test('tag creation pins the validated commit and explicitly dispatches its release workflow', () => {
  const workflow = read('release-version');
  assert.match(workflow, /branches: \[main\]/);
  assert.match(workflow, /paths: \["core\/Cargo.toml"\]/);
  assert.match(workflow, /ref: \$\{\{ github.sha \}\}/);
  assert.match(workflow, /fetch-depth: 0/);
  assert.match(workflow, /RELEASE_SHA: \$\{\{ needs.plan.outputs.sha \}\}/);
  assert.match(workflow, /gh api --method POST[^\n]*git\/refs[^\n]*-f "ref=refs\/tags\/\$TAG" -f "sha=\$RELEASE_SHA"/);
  assert.match(workflow, /commits\/\$TAG" --jq .sha\)" = "\$RELEASE_SHA"/);
  assert.match(workflow, /gh workflow run release.yml --ref "\$TAG" -f "tag=\$TAG" -f action=Build -f "desktop=\$DESKTOP"/);
  assert.doesNotMatch(workflow, /--force|--method PATCH/);
});
