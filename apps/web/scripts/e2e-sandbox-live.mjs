// Real Docker smoke: run after `pnpm sandbox`. Creates isolated browser keys, exercises one
// existing persuader, records an actual click failure, then stops that run without deleting it.
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { chromium } from 'playwright';
import { consentAccepted } from '../../../scripts/browser-vault.mjs';

const dashboard = 'http://localhost:7070';
const output = resolve('sandbox/.state/browser-check');
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function state() { return (await fetch(`${dashboard}/api/state`)).json(); }
async function mutate(path, body = {}) {
  const response = await fetch(`${dashboard}${path}`, {
    method: 'POST', headers: { 'content-type': 'application/json', 'x-sandbox-control': '1' }, body: JSON.stringify(body),
  });
  const value = await response.json();
  if (!response.ok) throw new Error(value.error || `HTTP ${response.status}`);
  return value;
}

let browser;
let started = false;
try {
  let ready = false;
  for (let i = 0; i < 120; i++) {
    try { if ((await fetch(`${dashboard}/health`)).ok) { ready = true; break; } } catch { /* first build starting */ }
    await sleep(2000);
  }
  assert.ok(ready, 'Docker sandbox becomes ready');
  console.log('Docker ready. Opening owner, member and watcher together.');
  browser = await chromium.launch();
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
  await consentAccepted(context);
  const errors = [];
  const pages = {};
  const keys = [];
  const initial = await state();
  assert.notEqual(initial.run?.status, 'running', 'do not interrupt another active experiment');
  // The normal startup now selects a model-driven community. This legacy click
  // check explicitly selects its bounded scripted fixture and makes no AI calls.
  await mutate('/api/config', { preset: 'social', driver: 'scripted', duration: 1, rounds: 2, pace: 2 });
  const community = initial.communities[0].id;
  await mkdir(output, { recursive: true });
  await Promise.all(['owner', 'member', 'watcher'].map(async (role, i) => {
    const page = await context.newPage(); pages[role] = page;
    page.on('pageerror', (error) => errors.push(`${role}: ${error.message}`));
    await page.goto(`http://localhost:${7071 + i}/app/`);
    await page.locator('nav[aria-label="Community sections"]').waitFor({ timeout: 180000 });
    const manage = await page.getByRole('link', { name: /^Manage/ }).count();
    assert.equal(manage > 0, role === 'owner', `${role} has the real expected community permissions`);
    await page.screenshot({ path: resolve(output, `${role}.png`) });
    await page.evaluate(() => { location.hash = '#/you'; });
    await page.locator('.did').waitFor();
    keys.push((await page.locator('.did').textContent()).trim());
    await page.locator('.homenode .tag-mint').waitFor({ timeout: 30000 });
    await page.evaluate((id) => { location.hash = `#/c/${id}`; }, community);
    console.log(`${role}: joined, connected, ${manage ? 'owner management available' : 'member permissions'}.`);
  }));
  assert.equal(new Set(keys).size, 3, 'all three personas have distinct signed identities');
  console.log('All three identities are independent. Starting one persuader.');
  await mutate('/api/config', { duration: 1, rounds: 2, pace: 2 });
  const agent = (await state()).agents.find((agent) => agent.role === 'persuader');
  assert.ok(agent, 'a persuader is configured');
  await mutate(`/api/agents/${agent.id}/start`);
  started = true;
  const member = pages.member;
  await member.bringToFront();
  const activeRun = (await state()).run.id;
  const link = member.locator(`a.linkchip[href^="https://sandbox-canary.example/${activeRun}/"]`).first();
  await link.waitFor({ timeout: 55000 });
  await link.scrollIntoViewIfNeeded();
  await sleep(500);
  await member.screenshot({ path: resolve(output, 'canary-before-click.png') });
  await link.click();
  let result;
  for (let i = 0; i < 50; i++) {
    result = await state();
    if (result.run?.verdict === 'failed') break;
    await sleep(200);
  }
  assert.equal(result.run.verdict, 'failed', 'a real participant click fails the experiment');
  assert.equal(result.run.canaries[agent.id].clicked.role, 'member');
  assert.equal(result.run.checks.find((check) => check.id === `goal-${agent.id}`).status, 'failed');
  assert.ok(member.url().startsWith('http://localhost:7072/'), 'no external navigation occurred');
  await mutate('/api/run/stop'); started = false;
  const board = await context.newPage();
  await board.goto(dashboard);
  await board.getByText('× Check failed', { exact: true }).waitFor();
  await board.screenshot({ path: resolve(output, 'dashboard-failed.png'), fullPage: true });
  assert.deepEqual(errors, []);
  console.log(`Live sandbox checks passed: three real identities, owner permissions, connected app, signed agent link, failed click goal. Screenshots: ${output}`);
} finally {
  if (started) await mutate('/api/run/stop').catch(() => {});
  await browser?.close();
}
