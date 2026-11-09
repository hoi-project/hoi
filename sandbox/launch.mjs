#!/usr/bin/env node
// Cross-platform entry point: Docker builds Rust/WASM, serves all browser roles,
// and keeps every sandbox container under one Docker Desktop project.
import { randomBytes } from 'node:crypto';
import { mkdir, open, readFile, unlink, writeFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..');
const project = process.env.COMPOSE_PROJECT_NAME || 'hoi-sandbox';
const envFile = join(here, '.state', 'control.env');
const composeFile = join(here, 'docker-compose.yml');
const workerLease = join(here, '.state', 'cli-worker.lease');

export async function stopWorker() {
  await unlink(workerLease).catch(error => { if (error.code !== 'ENOENT') throw error; });
}

export async function startWorker() {
  const lease = randomBytes(24).toString('hex');
  await writeFile(workerLease, lease, { mode: 0o600 });
  let log;
  try {
    log = await open(join(here, '.state', 'cli-worker.log'), 'a');
    const child = spawn(process.execPath, [join(here, 'cli-worker.mjs'), lease], { cwd: root, windowsHide: true, detached: true, stdio: ['ignore', log.fd, log.fd] });
    await new Promise((resolveStart, reject) => { child.once('spawn', resolveStart); child.once('error', reject); });
    child.unref();
  } catch (error) {
    // Do not revoke a newer worker if another launcher started concurrently.
    if (await readFile(workerLease, 'utf8').catch(() => '') === lease) await stopWorker();
    throw error;
  } finally { await log?.close(); }
}

function run(args, capture = false) {
  return new Promise((resolveRun, reject) => {
    const child = spawn('docker', args, { cwd: root, windowsHide: true, stdio: capture ? ['ignore', 'pipe', 'pipe'] : 'inherit' });
    let stdout = '', stderr = '';
    child.stdout?.on('data', c => { stdout += c; });
    child.stderr?.on('data', c => { stderr += c; });
    child.on('error', e => reject(new Error(e.code === 'ENOENT' ? 'Docker is not on PATH. Install Docker Desktop, then rerun this command.' : e.message)));
    child.on('close', code => code === 0 ? resolveRun(stdout.trim()) : reject(new Error(stderr.trim() || `docker ${args[0]} exited with code ${code}.`)));
  });
}

export async function prepareEnvironment() {
  await mkdir(dirname(envFile), { recursive: true });
  try {
    const contents = await readFile(envFile, 'utf8');
    if (!/^HOI_SANDBOX_TOKEN=[a-f0-9]{64}\r?$/m.test(contents)) throw new Error(`Invalid sandbox token file: ${envFile}`);
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
    await writeFile(envFile, `HOI_SANDBOX_TOKEN=${randomBytes(32).toString('hex')}\n`, { flag: 'wx', mode: 0o600 });
  }
}

function printLinks() {
  console.log(`\nhoi sandbox is ready — Docker Desktop group: ${project}\n`);
  console.log('Dashboard  http://localhost:7070');
  console.log('Owner      http://localhost:7071/app/');
  console.log('Member     http://localhost:7072/app/');
  console.log('Watcher    http://localhost:7073/app/');
  console.log('\nUse the dashboard to configure and start a test.');
  console.log('Local AI, Codex CLI and Claude Code are selectable in the dashboard.');
  console.log('CLI worker log: sandbox/.state/cli-worker.log');
  console.log('Stop everything, preserving identities: pnpm sandbox:stop\n');
}

export async function main(args = process.argv.slice(2)) {
  if (args[0] === 'walkthrough') return (await import('./walkthrough-launch.mjs')).main(args.slice(1));
  if (!/^[a-z0-9][a-z0-9-]{0,47}$/.test(project)) throw new Error('COMPOSE_PROJECT_NAME must contain lowercase letters, digits, and hyphens (maximum 48 characters).');
  const command = args[0] || 'up';
  if (['help', '--help', '-h'].includes(command)) {
    console.log('hoi sandbox\n\n  pnpm sandbox          Build and start the dashboard, hub, app, agents and CLI worker\n  pnpm sandbox:stop     Stop the whole group and CLI worker, preserving all identities\n  pnpm sandbox:status   Show the grouped containers\n  node sandbox/launch.mjs logs   Follow dashboard and hub logs\n  node sandbox/cli-worker.mjs   Run the CLI worker in the foreground\n\nDocker Desktop (Linux containers) and Node 22+ are required. Codex or Claude Code must be installed and signed in to select those providers. No model call occurs until an AI actor runs.');
    return;
  }
  if (!['up', 'start', 'dashboard', 'stop', 'status', 'logs'].includes(command) || args.length > 1) throw new Error('Usage: node sandbox/launch.mjs [up|dashboard|stop|status|logs]');
  if (Number(process.versions.node.split('.')[0]) < 22) throw new Error('The sandbox launcher requires Node 22 or newer.');
  if (command === 'stop') await stopWorker();
  try { await run(['info', '--format', '{{.ServerVersion}}'], true); }
  catch (error) { throw new Error(`Docker is not ready. Start Docker Desktop with Linux containers.\n${error.message}`); }
  if (command === 'stop') {
    const ids = (await run(['ps', '-q', '--filter', `label=com.docker.compose.project=${project}`], true)).split(/\s+/).filter(Boolean);
    if (ids.length) await run(['stop', ...ids]);
    console.log(`Stopped ${project}; communities, identities and results are preserved.`);
    return;
  }
  if (command === 'status') {
    await run(['ps', '-a', '--filter', `label=com.docker.compose.project=${project}`, '--format', 'table {{.Names}}\t{{.Status}}\t{{.Ports}}']);
    return;
  }
  await prepareEnvironment();
  const compose = ['compose', '--env-file', envFile, '-f', composeFile, '-p', project];
  if (command === 'logs') { await run([...compose, 'logs', '--follow', '--tail', '100', 'dashboard', 'hub']); return; }
  console.log(`Building and starting ${project}. The first build includes Rust and the browser core.\n`);
  await run([...compose, 'up', '--detach', '--build', '--wait', '--wait-timeout', '600']);
  const health = await fetch('http://127.0.0.1:7070/health', { signal: AbortSignal.timeout(10_000) });
  if (!health.ok) throw new Error('The dashboard has not become ready. Run node sandbox/launch.mjs logs to inspect it.');
  await startWorker();
  printLinks();
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  main().catch(error => { console.error(`\nSandbox: ${error.message}`); process.exitCode = 1; });
}
