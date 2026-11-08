#!/usr/bin/env node
// Reuse release checks only when the successful signing gate proves the same source and scope.
import { appendFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const signingName = 'signing secrets, tag and versions';
const shaPattern = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/;
const positiveId = value => /^[1-9][0-9]*$/.test(String(value));
class UnusableRun extends Error {}

export function resumeRequest(env, requireRun = true) {
  const request = { repository: env.GH_REPO, runId: env.RESUME_RUN_ID,
    source: env.SOURCE_SHA, tag: env.TAG, desktop: env.DESKTOP };
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(request.repository ?? '')) throw new Error('Invalid release repository.');
  if (requireRun && !positiveId(request.runId)) throw new Error('A positive release run ID is required.');
  if (!shaPattern.test(request.source ?? '')) throw new Error('A full source commit SHA is required.');
  if (!/^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$/.test(request.tag ?? '')) throw new Error('A version tag is required.');
  if (!['true', 'false'].includes(request.desktop)) throw new Error('Desktop scope must be true or false.');
  return request;
}

export function validateRun(run, request) {
  if (String(run.id) !== String(request.runId)) throw new UnusableRun('Release run ID does not match.');
  if (run.status !== 'completed') throw new UnusableRun('The previous release run must be completed.');
  if (!['workflow_dispatch', 'push'].includes(run.event)) throw new UnusableRun('Only manual or tag release runs can supply checks.');
  if (run.head_branch !== request.tag && (run.event !== 'workflow_dispatch' || run.head_branch !== 'main')) {
    throw new UnusableRun('The previous release run must come from main or the requested release tag.');
  }
  if (run.path !== '.github/workflows/release.yml') throw new UnusableRun('The previous run must use the Release workflow.');
  if (run.repository?.full_name !== request.repository || run.head_repository?.full_name !== request.repository) {
    throw new UnusableRun('The previous run must come from this repository.');
  }
  // A manual run on main uses main's workflow revision while checking out an older release tag.
  // The signing log, not the workflow head SHA, proves which product source passed the checks.
  if (!shaPattern.test(run.head_sha ?? '')) throw new UnusableRun('The previous run has an invalid workflow revision.');
}

export function validateSigningLog(log, request) {
  const values = { ref: new Set(), TAG: new Set(), DESKTOP: new Set() };
  let checkout = false;
  for (const raw of log.split(/\r?\n/)) {
    const line = raw.replace(/\x1b\[[0-9;]*m/g, '')
      .replace(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z /, '');
    if (line.startsWith('##[group]')) checkout = line.startsWith('##[group]Run actions/checkout@');
    if (line === '##[endgroup]') checkout = false;
    // These are runner-written inputs/environment entries, not arbitrary mentions in commands.
    const entry = /^ {2}(ref|TAG|DESKTOP): (.*)$/.exec(line);
    if (entry && (entry[1] !== 'ref' || checkout)) values[entry[1]].add(entry[2]);
  }
  for (const [field, expected] of Object.entries({ ref: request.source, TAG: request.tag, DESKTOP: request.desktop })) {
    if (values[field].size !== 1 || !values[field].has(expected)) {
      throw new UnusableRun(`The successful signing log does not prove the requested ${field}.`);
    }
  }
}

function apiClient(env, request, fetcher) {
  if (!env.GH_TOKEN) throw new Error('A GitHub token is required to verify previous release checks.');
  const base = `https://api.github.com/repos/${request.repository}`;
  const headers = { Authorization: `Bearer ${env.GH_TOKEN}`, Accept: 'application/vnd.github+json',
    'X-GitHub-Api-Version': '2022-11-28' };
  const fetchResponse = async (url, options, label) => {
    try { return await fetcher(url, { ...options, signal: AbortSignal.timeout(30_000) }); }
    catch { throw new Error(`Could not fetch ${label}.`); }
  };
  const json = async (path, label) => {
    const response = await fetchResponse(`${base}/${path}`, { headers, redirect: 'error' }, label);
    if (!response.ok) throw new Error(`Could not fetch ${label} (HTTP ${response.status}).`);
    try { return await response.json(); }
    catch { throw new Error(`Invalid ${label} response.`); }
  };
  return { base, headers, fetchResponse, json };
}

export async function verifyResume(env, fetcher = fetch) {
  const request = resumeRequest(env);
  const { base, headers, fetchResponse, json } = apiClient(env, request, fetcher);
  const run = await json(`actions/runs/${request.runId}`, 'release run');
  validateRun(run, request);
  const jobs = [];
  for (let page = 1; ; page++) {
    if (page > 100) throw new Error('Too many release job pages to verify.');
    const result = await json(`actions/runs/${request.runId}/jobs?filter=latest&per_page=100&page=${page}`, 'release jobs');
    if (!Array.isArray(result.jobs) || !Number.isSafeInteger(result.total_count) || result.total_count < 0) {
      throw new Error('Invalid release jobs response.');
    }
    jobs.push(...result.jobs);
    if (jobs.length >= result.total_count) break;
    if (!result.jobs.length) throw new Error('The release jobs response is incomplete.');
  }
  const signing = jobs.filter(job => job.name === signingName);
  if (signing.length !== 1 || signing[0].status !== 'completed' || signing[0].conclusion !== 'success' ||
      !positiveId(signing[0].id) || String(signing[0].run_id) !== String(request.runId)) {
    throw new UnusableRun('The previous run needs one successful signing gate.');
  }
  let response = await fetchResponse(`${base}/actions/jobs/${signing[0].id}/logs`,
    { headers, redirect: 'manual' }, 'signing log');
  for (let redirects = 0; response.status >= 300 && response.status < 400; redirects++) {
    if (redirects >= 3) throw new Error('Too many signing log redirects.');
    let location;
    try { location = new URL(response.headers.get('location')); }
    catch { throw new Error('Invalid signing log redirect.'); }
    if (location.protocol !== 'https:' || location.username || location.password) throw new Error('Invalid signing log redirect.');
    // The log download URL is signed. Never forward the repository token to its host.
    response = await fetchResponse(location.href, { redirect: 'manual' }, 'signing log download');
  }
  if (!response.ok) throw new Error(`Could not fetch signing log (HTTP ${response.status}).`);
  let log;
  try { log = await response.text(); }
  catch { throw new Error('Could not read the signing log.'); }
  validateSigningLog(log, request);
  return { reuse_checks: true };
}

export async function findVerifiedResume(env, fetcher = fetch) {
  const request = resumeRequest(env, false);
  const { json } = apiClient(env, request, fetcher);
  const seen = new Set();
  // GitHub caps filtered run searches at 1,000 results. Every candidate must prove its source.
  for (let page = 1; page <= 10; page++) {
    const result = await json(`actions/workflows/release.yml/runs?status=completed&per_page=100&page=${page}`, 'release runs');
    if (!Array.isArray(result.workflow_runs) || !Number.isSafeInteger(result.total_count) || result.total_count < 0) {
      throw new Error('Invalid release runs response.');
    }
    for (const run of result.workflow_runs) {
      if (!positiveId(run.id)) throw new Error('Invalid release run ID in search results.');
      if (seen.has(String(run.id))) continue;
      seen.add(String(run.id));
      try {
        validateRun(run, { ...request, runId: String(run.id) });
        await verifyResume({ ...env, RESUME_RUN_ID: String(run.id) }, fetcher);
        return { reuse_checks: true, run_id: String(run.id),
          run_url: `https://github.com/${request.repository}/actions/runs/${run.id}` };
      } catch (error) {
        // Mismatched evidence can lead to an older valid run. API failures must remain visible.
        if (!(error instanceof UnusableRun)) throw error;
      }
    }
    if (seen.size >= result.total_count) break;
    if (!result.workflow_runs.length) throw new Error('The release runs response is incomplete.');
  }
  throw new Error(`No passed checks found for ${request.tag}. Choose Build.`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const result = await findVerifiedResume(process.env);
    if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, 'reuse_checks=true\n');
    const summary = `Reusing verified checks from [run ${result.run_id}](${result.run_url}).`;
    if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${summary}\n`);
    console.log(`Reusing verified checks from ${result.run_url}`);
  } catch (error) { console.error(`release-resume: ${error.message}`); process.exitCode = 1; }
}
