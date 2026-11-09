import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, isAbsolute, resolve, win32 } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');

export function dockerExecutable({ platform = process.platform, env = process.env, exists = existsSync } = {}) {
  const absolute = platform === 'win32' ? win32.isAbsolute : isAbsolute;
  if (env.HOI_DOCKER_BIN) {
    if (!absolute(env.HOI_DOCKER_BIN) || !exists(env.HOI_DOCKER_BIN)) throw new Error('HOI_DOCKER_BIN must name an installed Docker executable by absolute path.');
    return env.HOI_DOCKER_BIN;
  }
  // Do not search inherited PATH: a project-local executable must not replace Docker.
  const candidates = platform === 'win32'
    ? [win32.join(env.ProgramFiles || 'C:\\Program Files', 'Docker', 'Docker', 'resources', 'bin', 'docker.exe')]
    : platform === 'darwin'
      ? ['/Applications/Docker.app/Contents/Resources/bin/docker', '/usr/local/bin/docker', '/opt/homebrew/bin/docker']
      : ['/usr/bin/docker', '/usr/local/bin/docker'];
  const executable = candidates.find(candidate => absolute(candidate) && exists(candidate));
  if (!executable) throw new Error('Install Docker Desktop, or set HOI_DOCKER_BIN to its executable’s absolute path.');
  return executable;
}

export function walkthroughCommand(command = 'up') {
  const base = ['compose', '-f', resolve(ROOT, 'sandbox/walkthrough-compose.yml'), '-p', 'hoi-walkthrough'];
  if (command === 'install') return [...base, 'build'];
  // Installation is explicit. Starting an installed walkthrough never downloads or builds.
  if (command === 'up' || command === 'start') return [...base, 'up', '--detach', '--no-build', '--pull', 'never', '--wait', '--wait-timeout', '90'];
  if (command === 'stop') return [...base, 'stop'];
  if (command === 'status') return [...base, 'ps', '-a'];
  if (command === 'logs') return [...base, 'logs', '--follow', '--tail', '60'];
  throw new Error('Usage: node sandbox/launch.mjs walkthrough [install|up|stop|status|logs]');
}

async function run(args) {
  return new Promise((done, reject) => {
    const child = spawn(dockerExecutable(), args, { cwd: ROOT, stdio: 'inherit', windowsHide: true });
    child.once('error', error => reject(new Error(error.code === 'ENOENT' ? 'Install Docker Desktop and start its Linux engine.' : error.message)));
    child.once('exit', code => code === 0 ? done() : reject(new Error('Docker did not finish. Start Docker Desktop; run walkthrough install before the first start.')));
  });
}

export async function main(args = process.argv.slice(2), execute = run) {
  if (args.length > 1) throw new Error('Choose one walkthrough command.');
  const command = args[0] || 'up';
  if (['help', '--help', '-h'].includes(command)) {
    console.log('Local network walkthrough\n\n  node sandbox/launch.mjs walkthrough install   Build once (requires internet)\n  node sandbox/launch.mjs walkthrough           Start offline at http://localhost:7080\n  node sandbox/launch.mjs walkthrough stop      Stop the walkthrough\n  node sandbox/launch.mjs walkthrough status    Show its container\n  node sandbox/launch.mjs walkthrough logs      Read startup status\n\nNo AI provider or host CLI worker is used. Each start/reset creates new demonstration identities.');
    return;
  }
  await execute(walkthroughCommand(command));
  if (command === 'install') console.log('Installed. Start with: node sandbox/launch.mjs walkthrough');
  if (command === 'up' || command === 'start') console.log('Network walkthrough: http://localhost:7080\nThe screen shows preparation and observed delivery. Stop: node sandbox/launch.mjs walkthrough stop');
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) main().catch(error => { console.error(error.message); process.exitCode = 1; });
