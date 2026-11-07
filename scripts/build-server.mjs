#!/usr/bin/env node
// Build a Linux server locally and pack the existing service installer beside it.
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, copyFileSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { BUNDLE_FILES } from '../deploy/server/bundle/deploy.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const targets = { x64: { platform: 'linux/amd64', machine: 62, uname: 'x86_64' }, arm64: { platform: 'linux/arm64', machine: 183, uname: 'aarch64' } };
const help = `Usage: pnpm build:server [--arch x64|arm64] [--binary PATH] [--out DIR]

Default: build the current checkout with Docker Buildx for a Linux x64 VPS.
--binary PATH  Package an existing Linux hoi-node or hoi-node .tar.gz instead.
--out DIR      Output directory (default: dist/server).

Creates hoi-server-linux-ARCH/ and hoi-server-linux-ARCH.tar.gz.
Run pnpm deploy:server USER@HOST to upload and install the x64 bundle over SSH.`;

export function parseBuildArgs(args) {
  const options = { arch: 'x64', out: join(root, 'dist/server') };
  for (let i = 0; i < args.length; i++) {
    const flag = args[i];
    if (flag === '--help' || flag === '-h') return { help: true };
    if (!['--arch', '--binary', '--out'].includes(flag)) throw new Error(`Unknown option: ${flag}`);
    const value = args[++i];
    if (!value || value.startsWith('--')) throw new Error(`${flag} needs a value`);
    options[flag.slice(2)] = value;
  }
  if (!Object.hasOwn(targets, options.arch)) throw new Error('--arch must be x64 or arm64');
  return options;
}

function run(command, args, cwd = root) {
  const result = spawnSync(command, args, { cwd, stdio: 'inherit' });
  if (result.error) throw new Error(`Cannot run ${command}: ${result.error.message}`);
  if (result.status !== 0) throw new Error(`${command} failed (exit ${result.status})`);
}

export function validateBinary(bytes, arch) {
  if (bytes.length < 64 || !bytes.subarray(0, 4).equals(Buffer.from([127, 69, 76, 70])) || bytes[4] !== 2 || bytes[5] !== 1) {
    throw new Error('Expected a 64-bit Linux ELF binary, not a Windows or macOS app');
  }
  if (bytes.readUInt16LE(18) !== targets[arch]?.machine) throw new Error(`The binary does not match --arch ${arch}`);
}

export function buildBundle(options) {
  const { arch, binary } = options;
  if (!Object.hasOwn(targets, arch)) throw new Error('--arch must be x64 or arm64');
  const target = targets[arch];
  // Fail before creating output when Docker isn't ready; a supplied binary needs no Docker.
  if (!binary) {
    const ready = spawnSync('docker', ['info'], { stdio: 'ignore' });
    if (ready.status !== 0) throw new Error('Start Docker with Linux containers, then run pnpm build:server again; or use --binary PATH');
  }
  const out = resolve(options.out);
  mkdirSync(out, { recursive: true });
  const work = mkdtempSync(join(out, '.build-'));
  const name = `hoi-server-linux-${arch}`;
  const bundle = join(work, name);
  const destination = join(out, name);
  mkdirSync(bundle);
  try {
    const executable = join(bundle, 'hoi-node');
    if (binary) {
      const source = resolve(binary);
      if (/\.(tar\.gz|tgz)$/i.test(source)) run('tar', ['-xzf', source, '-C', bundle, 'hoi-node']);
      else copyFileSync(source, executable);
    } else {
      console.log(`Building hoi-node for ${target.platform}. The first build can take a while.`);
      // Quote the CSV field so spaces and commas in the checkout path survive Buildx parsing.
      const output = `type=local,"dest=${bundle.replaceAll('"', '""')}"`;
      run('docker', ['buildx', 'build', '--platform', target.platform, '-f', 'core/crates/hoi-node/Dockerfile', '--target', 'binary', '--output', output, '.']);
    }
    if (!lstatSync(executable).isFile()) throw new Error('hoi-node must be a regular file, not a symbolic link');
    validateBinary(readFileSync(executable), arch);
    chmodSync(executable, 0o755);
    const write = (name, text) => writeFileSync(join(bundle, name), text.replaceAll('\r\n', '\n'));
    write('node-installer.sh', readFileSync(join(root, 'deploy/install/install.sh'), 'utf8'));
    write('install.sh', readFileSync(join(root, 'deploy/server/bundle/install.sh'), 'utf8').replaceAll('@ARCH@', target.uname));
    write('deploy.mjs', readFileSync(join(root, 'deploy/server/bundle/deploy.mjs'), 'utf8'));
    write('README.md', readFileSync(join(root, 'deploy/server/bundle/README.md'), 'utf8'));
    // Keep the operator's configuration when rebuilding this architecture.
    write('server.conf', existsSync(join(destination, 'server.conf'))
      ? readFileSync(join(destination, 'server.conf'), 'utf8')
      : '# Read as settings, never executed. See README.md for registry settings.\nrole=public\n');
    write('SHA256SUMS', ['hoi-node', 'node-installer.sh'].map(file =>
      `${createHash('sha256').update(readFileSync(join(bundle, file))).digest('hex')}  ${file}\n`).join(''));
    for (const script of ['install.sh', 'node-installer.sh']) chmodSync(join(bundle, script), 0o755);
    const archive = join(out, `${name}.tar.gz`);
    run('tar', ['-czf', join(work, 'bundle.tar.gz'), '-C', work, ...BUNDLE_FILES.map(file => `${name}/${file}`)]);
    mkdirSync(destination, { recursive: true });
    for (const file of BUNDLE_FILES) copyFileSync(join(bundle, file), join(destination, file));
    renameSync(join(work, 'bundle.tar.gz'), archive);
    return { directory: destination, archive };
  } finally {
    // Only remove the temporary directory this invocation created under the output directory.
    if (dirname(work) !== out || !basename(work).startsWith('.build-')) throw new Error('Unsafe build cleanup path');
    rmSync(work, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const options = parseBuildArgs(process.argv.slice(2));
    if (options.help) console.log(help);
    else {
      const result = buildBundle(options);
      console.log(`\nBundle: ${result.archive}\nFolder: ${result.directory}\n\nUpload the archive and run bash install.sh after extracting it, or use:\n  node "${join(result.directory, 'deploy.mjs')}" USER@HOST`);
    }
  } catch (error) {
    console.error(`build-server: ${error.message}`);
    process.exitCode = 1;
  }
}
