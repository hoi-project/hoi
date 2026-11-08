import assert from 'node:assert/strict';
import { test } from 'node:test';
import { findVerifiedResume, resumeRequest, validateSigningLog, verifyResume } from './release-resume.mjs';

const source = 'a'.repeat(40);
const env = { GH_REPO: 'example/hoi', GH_TOKEN: 'test-token', RESUME_RUN_ID: '123',
  SOURCE_SHA: source, TAG: 'v1.0.0-preview.2', DESKTOP: 'true' };
const run = { id: 123, status: 'completed', conclusion: 'failure', event: 'workflow_dispatch',
  path: '.github/workflows/release.yml', head_sha: source, head_branch: 'main',
  repository: { full_name: env.GH_REPO }, head_repository: { full_name: env.GH_REPO } };
const signing = { id: 456, run_id: 123, name: 'signing secrets, tag and versions', status: 'completed', conclusion: 'success' };
const log = [
  '##[group]Run actions/checkout@' + 'b'.repeat(40),
  'with:', `  ref: ${source}`, 'env:', `  TAG: ${env.TAG}`, '  DESKTOP: true',
  '##[endgroup]', `  TAG: ${env.TAG}`, '  DESKTOP: true',
].map(line => `2026-11-08T21:39:13.8911261Z ${line}`).join('\n');
const response = data => new Response(JSON.stringify(data), { status: 200 });

function fixture(patch = {}) {
  const calls = [];
  const fetcher = async (url, options) => {
    calls.push({ url, options });
    if (url.endsWith('/actions/runs/123')) return response({ ...run, ...patch.run });
    if (url.includes('/jobs?')) {
      const page = Number(new URL(url).searchParams.get('page'));
      const pages = patch.pages ?? [[{ ...signing, ...patch.signing }]];
      return response({ total_count: pages.flat().length, jobs: pages[page - 1] ?? [] });
    }
    if (url.endsWith('/actions/jobs/456/logs')) {
      if (patch.logError) throw new Error('sensitive network details');
      if (patch.logStatus) return new Response('', { status: patch.logStatus });
      if (patch.redirect) return new Response('', { status: 302, headers: { location: patch.redirect } });
      return new Response(patch.log ?? log);
    }
    if (url === 'https://logs.example.com/signed-download') return new Response(patch.log ?? log);
    throw new Error(`Unexpected URL ${url}`);
  };
  return { fetcher, calls };
}

test('a failed release can reuse its successful signing gate for exactly the same source and scope', async () => {
  const { fetcher, calls } = fixture();
  assert.deepEqual(await verifyResume(env, fetcher), { reuse_checks: true });
  assert.equal(calls.length, 3);
  assert.match(calls[1].url, /filter=latest&per_page=100&page=1$/);
  for (const { options } of calls) assert.equal(options.headers.Authorization, 'Bearer test-token');
  assert.deepEqual(await verifyResume(env, fixture({ run: { event: 'push', head_branch: env.TAG } }).fetcher), { reuse_checks: true });
  assert.deepEqual(await verifyResume(env, fixture({ run: { head_branch: env.TAG } }).fetcher), { reuse_checks: true });
  assert.deepEqual(await verifyResume(env, fixture({ run: { head_sha: 'c'.repeat(40) } }).fetcher), { reuse_checks: true });
});

test('incomplete, unrelated or malformed runs cannot supply release checks', async () => {
  for (const patch of [
    { id: 124 }, { status: 'in_progress' }, { event: 'pull_request' }, { event: 'pull_request_target' },
    { head_branch: 'topic' }, { head_branch: 'v9.0.0' }, { event: 'push', head_branch: 'main' },
    { path: '.github/workflows/ci.yml' }, { head_sha: 'abc' },
    { repository: { full_name: 'other/hoi' } }, { head_repository: { full_name: 'other/hoi' } },
    { head_repository: null },
  ]) await assert.rejects(verifyResume(env, fixture({ run: patch }).fetcher));
});

test('only a unique completed successful signing job from the same run is accepted', async () => {
  for (const patch of [
    { name: 'another signing job' }, { status: 'queued' }, { conclusion: 'failure' },
    { conclusion: 'cancelled' }, { conclusion: 'skipped' }, { run_id: 124 }, { id: 'invalid' },
  ]) await assert.rejects(verifyResume(env, fixture({ signing: patch }).fetcher), /successful signing gate/);
  await assert.rejects(verifyResume(env, fixture({ pages: [[signing, signing]] }).fetcher), /successful signing gate/);
  await assert.rejects(verifyResume(env, fixture({ pages: [[]] }).fetcher), /successful signing gate/);
});

test('job pagination includes the successful signing gate on a later page', async () => {
  const other = { ...signing, id: 1, name: 'another job' };
  const { fetcher, calls } = fixture({ pages: [Array.from({ length: 100 }, () => other), [signing]] });
  assert.deepEqual(await verifyResume(env, fetcher), { reuse_checks: true });
  assert.ok(calls.some(call => call.url.endsWith('page=2')));
});

test('the signing log must uniquely prove checkout source, tag and desktop scope', async () => {
  const request = resumeRequest(env);
  validateSigningLog(log, request);
  validateSigningLog(log.replace(/^\S+ /gm, ''), request);
  for (const text of [
    '', log.replaceAll(source, 'c'.repeat(40)), log.replaceAll(env.TAG, 'v1.0.0-preview.3'),
    log.replaceAll('DESKTOP: true', 'DESKTOP: false'),
    log.replaceAll('  ref:', ' ref:'), log.replaceAll('  TAG:', 'TAG:'),
    log.replace('Run actions/checkout@', 'Run another/action@'),
    log + '\n  TAG: v2.0.0', log + '\n  DESKTOP: false',
    log + `\n##[group]Run actions/checkout@abc\n  ref: ${'c'.repeat(40)}\n##[endgroup]`,
  ]) await assert.rejects(verifyResume(env, fixture({ log: text }).fetcher), /does not prove/);
  await assert.rejects(verifyResume({ ...env, DESKTOP: 'false' }, fixture().fetcher), /DESKTOP/);
});

test('signed log download redirects never receive the GitHub token', async () => {
  const { fetcher, calls } = fixture({ redirect: 'https://logs.example.com/signed-download' });
  assert.deepEqual(await verifyResume(env, fetcher), { reuse_checks: true });
  assert.equal(calls.at(-1).options.headers, undefined);
  assert.equal(calls.at(-1).options.redirect, 'manual');
  for (const redirect of ['http://logs.example.com/signed-download', '/relative', 'https://user:password@logs.example.com/file']) {
    await assert.rejects(verifyResume(env, fixture({ redirect }).fetcher), /Invalid signing log redirect/);
  }
});

test('unavailable logs and API failures fail closed without printing server details', async () => {
  await assert.rejects(verifyResume(env, fixture({ logStatus: 404 }).fetcher), /HTTP 404/);
  await assert.rejects(verifyResume(env, fixture({ logError: true }).fetcher), /^Error: Could not fetch signing log\.$/);
  await assert.rejects(verifyResume(env, async () => new Response('private details', { status: 403 })), /HTTP 403/);
  await assert.rejects(verifyResume(env, async () => new Response('not json')), /Invalid release run response/);
  await assert.rejects(verifyResume({ ...env, GH_TOKEN: '' }, fixture().fetcher), /GitHub token/);
});

test('malformed release requests are rejected before API access', async () => {
  for (const patch of [
    { GH_REPO: 'https://example.com' }, { RESUME_RUN_ID: '0' }, { RESUME_RUN_ID: '../1' },
    { SOURCE_SHA: 'abc' }, { TAG: 'main' }, { DESKTOP: '' },
  ]) await assert.rejects(verifyResume({ ...env, ...patch }, () => assert.fail('No request expected')));
});

function discoveryFixture({ pages = [[{ ...run, id: 124 }, run]], evidence = {}, failAt, failure = 403 } = {}) {
  const calls = [];
  const runs = pages.flat();
  const fetcher = async (url, options) => {
    calls.push({ url, options });
    if (failAt?.test(url)) return new Response('private details', { status: failure });
    if (url.includes('/actions/workflows/release.yml/runs?')) {
      const page = Number(new URL(url).searchParams.get('page'));
      return response({ total_count: runs.length, workflow_runs: pages[page - 1] ?? [] });
    }
    const runId = /\/actions\/runs\/(\d+)/.exec(url)?.[1];
    if (runId) {
      const candidate = runs.find(item => String(item.id) === runId);
      if (url.includes('/jobs?')) return response({ total_count: 1, jobs: [{ ...signing,
        id: Number(runId) * 10, run_id: Number(runId), ...evidence[runId]?.signing }] });
      return response({ ...candidate, ...evidence[runId]?.run });
    }
    const jobId = /\/actions\/jobs\/(\d+)\/logs$/.exec(url)?.[1];
    if (jobId) return new Response(evidence[Number(jobId) / 10]?.log ?? log);
    throw new Error(`Unexpected URL ${url}`);
  };
  return { fetcher, calls };
}

test('Resume finds an older successful signing gate without a run ID input', async () => {
  const automatic = { ...env, RESUME_RUN_ID: undefined };
  const { fetcher, calls } = discoveryFixture({ evidence: { 124: { signing: { conclusion: 'failure' } } } });
  assert.deepEqual(await findVerifiedResume(automatic, fetcher), { reuse_checks: true, run_id: '123',
    run_url: 'https://github.com/example/hoi/actions/runs/123' });
  const query = new URL(calls[0].url);
  assert.equal(query.pathname, '/repos/example/hoi/actions/workflows/release.yml/runs');
  assert.equal(query.searchParams.has('head_sha'), false, 'workflow revision need not equal the checked tag');
  assert.equal(query.searchParams.get('status'), 'completed');
  assert.equal(query.searchParams.get('per_page'), '100');
  assert.ok(calls.some(({ url }) => url.endsWith('/actions/runs/123')));
});

test('discovery verifies candidate logs and does not trust only the run-list metadata', async () => {
  for (const evidence of [
    { 124: { log: log.replaceAll(env.TAG, 'v9.0.0') } },
    { 124: { log: log.replaceAll('DESKTOP: true', 'DESKTOP: false') } },
    { 124: { run: { head_sha: 'abc' } } },
    { 124: { log: log.replaceAll(source, 'c'.repeat(40)) } },
  ]) assert.equal((await findVerifiedResume(env, discoveryFixture({ evidence }).fetcher)).run_id, '123');
});

test('a Check run from newer main can supply proof for the same older tagged source', async () => {
  const options = { pages: [[{ ...run, id: 124, head_sha: 'c'.repeat(40) }]] };
  assert.equal((await findVerifiedResume(env, discoveryFixture(options).fetcher)).run_id, '124');
  await assert.rejects(findVerifiedResume(env, discoveryFixture({ ...options,
    evidence: { 124: { log: log.replaceAll(source, 'c'.repeat(40)) } },
  }).fetcher), /No passed checks found/);
});

test('discovery follows run pagination while ignoring unrelated branch evidence', async () => {
  const unrelated = Array.from({ length: 100 }, (_, index) => ({ ...run, id: 1000 + index, head_branch: 'topic' }));
  const { fetcher, calls } = discoveryFixture({ pages: [unrelated, [run]] });
  assert.equal((await findVerifiedResume(env, fetcher)).run_id, '123');
  assert.ok(calls.some(({ url }) => url.endsWith('page=2')));
  assert.equal(calls.filter(({ url }) => /\/actions\/runs\//.test(url)).length, 2, 'only the eligible run and its jobs are fetched');
});

test('Resume stops with a Build instruction when no matching evidence exists', async () => {
  for (const options of [
    { pages: [[]] },
    { pages: [[{ ...run, head_branch: 'topic' }]] },
    { evidence: { 124: { signing: { conclusion: 'failure' } }, 123: { log: '' } } },
  ]) await assert.rejects(findVerifiedResume(env, discoveryFixture(options).fetcher),
    /^Error: No passed checks found for v1\.0\.0-preview\.2\. Choose Build\.$/);
});

test('discovery does not hide permission, network or log-download failures by trying another run', async () => {
  for (const [failAt, failure] of [
    [/\/workflows\//, 403], [/\/runs\/124\/jobs\?/, 500], [/\/jobs\/1240\/logs$/, 404],
  ]) {
    const { fetcher, calls } = discoveryFixture({ failAt, failure });
    await assert.rejects(findVerifiedResume(env, fetcher), new RegExp(`HTTP ${failure}`));
    assert.ok(!calls.some(({ url }) => url.includes('/actions/runs/123')));
  }
  await assert.rejects(findVerifiedResume(env, async () => { throw new Error('private details'); }),
    /^Error: Could not fetch release runs\.$/);
  await assert.rejects(findVerifiedResume(env, async () => response({ total_count: 1 })), /Invalid release runs response/);
});
