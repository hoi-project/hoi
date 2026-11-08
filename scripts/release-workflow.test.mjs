import assert from 'node:assert/strict';
import { cpSync, mkdtempSync, readdirSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { spawnSync } from 'node:child_process';
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
    github: { event_name: event, repository_owner: owner, repository: `${owner}/hoi`, event: { repository: { private: false } } },
    vars: { HOI_RELEASES_ENABLED: releases },
    inputs: { desktop: false },
    needs: Object.fromEntries(['plan', 'checks', 'long', 'security', 'npm-audit', 'signing', 'draft', 'node', 'desktop', 'setup']
      .map(job => [job, { result: ['desktop', 'setup'].includes(job) || (tier !== 'major' && ['long', 'security', 'npm-audit'].includes(job)) ? 'skipped' : 'success', outputs: { tier } }])),
  };
}

const job = (workflow, name) => read(workflow).split(`\n  ${name}:\n`)[1]?.split(/\n  [\w-]+:\n/)[0];

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
    ['release', 'signing', "The tree carries the tag's version", 'TAG'],
  ]) {
    await t.test(`${workflow} / ${name}`, () => {
      const section = job(workflow, name)?.split(`      - name: ${step}\n`)[1]?.split('\n      - ')[0];
      const run = section?.match(/^        run: (.+)$/m)?.[1];
      assert.ok(run, step);
      const script = run === '|' ? section.split('        run: |\n')[1].replace(/^          /gm, '') : run;
      const check = tag => spawnSync('bash', ['-e', '-c', script], {
        cwd: base, encoding: 'utf8', env: { PATH: process.env.PATH, [variable]: tag },
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
  for (const name of ['ios', 'installers']) assert.match(job('release-major', name), /if: \$\{\{ !inputs\.linux_only \}\}/, name);
  assert.match(job('release-major', 'rust'), /inputs\.linux_only && '\["ubuntu-latest"\]'/);
  assert.match(job('release-major', 'desktop'), /inputs\.linux_only && '\["ubuntu-22\.04"\]'/);
  assert.match(job('release-major', 'webrtc-browsers'), /inputs\.linux_only && '\[\{"browser":"firefox"/);
  assert.match(job('weekly', 'suites'), /linux_only: \$\{\{ github\.event_name == 'workflow_dispatch' && inputs\.linux_only \}\}/);
});

test('major release browser jobs use Ubuntu supported by the pinned Playwright', () => {
  assert.match(job('release-major', 'browser-e2e'), /^    runs-on: ubuntu-24\.04$/m);
  const webrtc = job('release-major', 'webrtc-browsers');
  const firefoxRunners = [...webrtc.matchAll(/"browser":"firefox","os":"([^"]+)"/g)].map(([, os]) => os);
  assert.deepEqual(firefoxRunners, ['ubuntu-24.04', 'ubuntu-24.04']);
});

test('major release browser builds do not fetch the live release feed', () => {
  const fixture = 'scripts/fixtures/publication-browser-e2e.json';
  for (const name of ['browser-e2e', 'webrtc-browsers']) {
    assert.ok(job('release-major', name).includes('HOI_PUBLICATION_FILE: ${{ github.workspace }}/' + fixture), name);
  }
  const publication = JSON.parse(readFileSync(new URL('../' + fixture, import.meta.url), 'utf8'));
  assert.equal(publication.downloads, '');
  assert.equal(publication.manifestKey, '');
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
  assert.match(workflow, /gh release create[^\n]*--prerelease="\$PRERELEASE"/);
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
  assert.match(workflow, /gh workflow run release.yml --ref "\$TAG" -f "tag=\$TAG" -f "desktop=\$DESKTOP"/);
  assert.doesNotMatch(workflow, /--force|--method PATCH/);
});
