#!/usr/bin/env node
// Standalone SSH uploader; this file is also copied into the generated bundle.
import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createInterface } from 'node:readline/promises';

export const BUNDLE_FILES = ['hoi-node', 'node-installer.sh', 'install.sh', 'deploy.mjs', 'README.md', 'server.conf', 'SHA256SUMS'];
const actions = ['deploy', 'check', 'services', 'status', 'logs', 'start', 'stop', 'restart', 'disable', 'enable'];
const roles = ['public', 'registry', 'home', 'private', 'config'];
const help = `Usage: node deploy.mjs USER@HOST [OPTIONS] [-- INSTALL_OPTIONS]

With no action, opens a menu. Use flags for unattended commands:
  --action deploy|check|services|status|logs|start|stop|restart|disable|enable
  --role public|registry|home|private|config
    registry = public node + registry in one hoi-node service
    config = use server.conf; explicit roles ignore that file
  --registry-url https://names.example.org
  --tls caddy|external     install HTTPS with Caddy, or use your own proxy
  --service node|registry|caddy|all|UNIT  service controls (default: node)
    all = installed hoi-node and Caddy; Caddy controls affect every site it serves
  --notice FILE            the operator's notice (JSON from the app's "A node on a server" page
                           or the setup app): uploaded with the bundle and carried by the node,
                           so people read it before it serves them. Without one, a public node
                           says so when it starts (the menu asks for it).
  --reset-data             start the selected node role with empty data
  --confirm-reset NAME     hoi-public for public/registry, hoi-node for home/private
  --port PORT --identity KEY --bundle DIR

Stop preserves data; disable also prevents startup after reboot. Enable starts the service
and enables startup after reboot. Services/status/logs do not change the VPS.
Service controls need only Node 22+ and SSH locally, with systemd and root/sudo on the VPS.
Deploy/check additionally need a built bundle, scp and tar. Your SSH settings remain in use:
one connection is shared for the whole deployment on Linux and macOS, so a password login
is asked once (Windows' OpenSSH cannot share, so it asks again for the upload; a key avoids it).
Whoever runs a public node answers for other people's data: the menu says what that means
and asks you to acknowledge it (hoi.sh/download/#operator).

Examples:
  node deploy.mjs root@203.0.113.10
  node deploy.mjs my-vps --role registry --registry-url https://names.example.org --tls caddy
  node deploy.mjs my-vps --action stop
  node deploy.mjs my-vps --action status --service all
  node deploy.mjs my-vps --role public --reset-data --confirm-reset hoi-public

The uploaded files stay in a private temporary folder on the VPS, printed below.
Data is kept unless you explicitly request and confirm a reset. No website or Worker is deployed.`;

export function parseDeployArgs(args) {
  const options = { bundle: dirname(fileURLToPath(import.meta.url)), installArgs: [] };
  for (let i = 0; i < args.length; i++) {
    const flag = args[i];
    if (flag === '--help' || flag === '-h') return { help: true };
    if (flag === '--') { options.installArgs = args.slice(i + 1); break; }
    if (flag === '--reset-data') { options.resetData = true; continue; }
    if (['--port', '--identity', '--bundle', '--action', '--role', '--service', '--registry-url', '--tls', '--confirm-reset', '--notice'].includes(flag)) {
      const value = args[++i];
      if (!value || value.startsWith('--')) throw new Error(`${flag} needs a value`);
      const key = { '--registry-url': 'registryUrl', '--confirm-reset': 'confirmReset' }[flag] ?? flag.slice(2);
      options[key] = value;
    } else if (!options.host && !flag.startsWith('-')) options.host = flag;
    else throw new Error(`Unexpected argument: ${flag}. Installer options go after --`);
  }
  if (!/^(?:[A-Za-z_][A-Za-z0-9_.-]*@)?(?:[A-Za-z0-9][A-Za-z0-9._-]*|\[[0-9a-fA-F:]+\])$/.test(options.host ?? '')) {
    throw new Error('Give an SSH destination such as root@203.0.113.10 or an SSH config alias');
  }
  if (options.port && (!/^\d+$/.test(options.port) || +options.port < 1 || +options.port > 65535)) throw new Error('--port must be 1–65535');
  if (options.identity) options.identity = resolve(options.identity);
  if (options.notice) options.notice = resolve(options.notice);
  options.bundle = resolve(options.bundle);
  // Keep the previous "-- check" and "-- registry ..." syntax working.
  if (options.installArgs[0] === 'check') { options.action ??= 'check'; options.installArgs.shift(); options.role ??= 'config'; }
  if (roles.includes(options.installArgs[0])) { options.role ??= options.installArgs.shift(); }
  if (options.role || options.registryUrl || options.tls || options.resetData || options.installArgs.length) options.action ??= 'deploy';
  validateOptions(options);
  return options;
}

function validateOptions(options) {
  if (options.action && !actions.includes(options.action)) throw new Error(`--action must be ${actions.join(', ')}`);
  if (options.role && !roles.includes(options.role)) throw new Error(`--role must be ${roles.join(', ')}`);
  if (options.tls && !['caddy', 'external'].includes(options.tls)) throw new Error('--tls must be caddy or external');
  if (options.registryUrl && !/^https:\/\/[A-Za-z0-9.-]+(:[0-9]+)?\/?$/.test(options.registryUrl)) throw new Error('--registry-url needs an HTTPS origin such as https://names.example.org');
  if (options.confirmReset && !options.resetData) throw new Error('--confirm-reset requires --reset-data');
  if (options.notice && options.action && !['deploy', 'check'].includes(options.action)) throw new Error('--notice goes with a deployment');
  if (options.service) serviceUnits(options.service);
  if (options.action && !['deploy', 'check'].includes(options.action) &&
      (options.role || options.registryUrl || options.tls || options.resetData || options.installArgs.length)) {
    throw new Error('Service controls cannot be combined with deployment or reset options');
  }
  if (['deploy', 'check'].includes(options.action) && options.service) throw new Error('--service selects a service to control; use --role for deployment');
  if (options.action === 'check' && options.resetData) throw new Error('A check cannot reset data');
}

const OPERATOR_TEXT = `Before you run a public node
  You become responsible: a public node processes other people's IP addresses and connection data.
  Under the data protection rules that apply where you, your server and those people are, whoever
  runs the node is usually the one responsible for that data. The developers are not. A node only
  your family and close friends use may count as personal use, which many such rules exempt; a
  community open to people you do not know personally generally does not.
  Write a join notice (what the node keeps, for how long): everyone reads it before the node serves
  them. Your name, your contact and the server's country are optional in it, but you should give
  them; whatever you fill in is shown to everyone who connects. Keep the server secure and answer
  people's requests about their data. This is not legal advice (hoi.sh/download/#operator has the
  whole text).`;

async function choose(ask, title, choices, fallback) {
  const answer = (await ask(`${title}\n${choices.map(([value, label], i) => `  ${i + 1}) ${label} (${value})`).join('\n')}\nChoice${fallback ? ` [${fallback}]` : ''}: `)).trim();
  const value = answer || fallback;
  const choice = choices.find(([name], i) => value === name || value === String(i + 1));
  if (!choice) throw new Error('No valid choice selected; nothing was changed');
  return choice[0];
}

/** Ask only for unspecified choices. Tests supply an answer function instead of a terminal. */
export async function configureOptions(input, ask) {
  const options = { ...input, installArgs: [...input.installArgs] };
  const menu = !options.action;
  if (!options.action) {
    if (!ask) throw new Error('Choose --action (for example --action stop), or --role for deployment; run in a terminal for the menu');
    options.action = await choose(ask, 'What should this VPS do?', [
      ['deploy', 'Deploy or update'], ['check', 'Check before installing'], ['services', 'List existing services'],
      ['status', 'Service status'], ['logs', 'Recent service logs'], ['start', 'Start a service'],
      ['stop', 'Stop a service'], ['restart', 'Restart a service'], ['disable', 'Stop and disable startup'], ['enable', 'Enable startup and start'],
    ]);
  }
  if (['deploy', 'check'].includes(options.action)) {
    if (!options.role) {
      if (!ask) throw new Error('Choose --role public|registry|home|private|config');
      options.role = await choose(ask, 'What should be deployed? One node role runs per host.', [
        ['public', 'Public node'], ['registry', 'Public node and registry'], ['home', 'Home node and public relay'],
        ['private', 'Private home node'], ['config', 'Use the saved server.conf settings'],
      ]);
    }
    if (options.role === 'registry') {
      if (!options.registryUrl && !options.installArgs.includes('--registry-url')) {
        if (!ask) throw new Error('A registry needs --registry-url https://names.example.org');
        options.registryUrl = (await ask('Registry HTTPS URL: ')).trim();
        if (!options.registryUrl) throw new Error('A registry HTTPS URL is required');
      }
      if (!options.tls && !options.installArgs.includes('--tls') && ask) {
        options.tls = await choose(ask, 'HTTPS for the registry:', [['caddy', 'Install/configure Caddy'], ['external', 'Use my existing proxy']], 'caddy');
      }
    } else if (options.registryUrl || options.tls) {
      throw new Error('--registry-url and --tls go with --role registry');
    }
    if (menu && options.action === 'deploy' && ['public', 'registry'].includes(options.role)) {
      // Whoever runs a public node answers for other people's data: said once, here.
      const reply = (await ask(`${OPERATOR_TEXT}\nI understand that I am responsible for the data processed on my node [y/N]: `)).trim();
      if (!/^y(es)?$/i.test(reply)) throw new Error('Nothing was changed. Read hoi.sh/download/#operator and come back when you are ready');
      if (!options.notice && !options.installArgs.includes('--notice-file')) {
        const file = (await ask('Your operator notice, a JSON file from the app (You → A node on a server) or the setup app; empty installs without one, and the node says so: ')).trim();
        if (file) options.notice = resolve(file);
      }
    }
    if (menu && options.action === 'deploy' && !options.resetData) {
      options.resetData = await choose(ask, 'Existing node data:', [['keep', 'Keep existing data'], ['reset', 'Erase data and start fresh']], 'keep') === 'reset';
    }
    if (options.resetData) {
      const state = ['home', 'private'].includes(options.role) ? 'hoi-node' : 'hoi-public';
      if (!options.confirmReset && ask) {
        options.confirmReset = (await ask(`Permanently erase /var/lib/${state} on ${options.host}, including keys, records and any registry database. Type ${state} to confirm: `)).trim();
      }
      if (options.confirmReset !== state) throw new Error(`Data reset requires --confirm-reset ${state}; nothing was changed`);
    }
  } else if (menu && options.action !== 'services' && !options.service) {
    options.service = await choose(ask, 'Which service?', [
      ['node', 'hoi-node (including its registry, if enabled)'], ['caddy', 'Caddy HTTPS (all hosted sites)'],
      ['all', 'Both installed hoi-node and Caddy services'], ['custom', 'An existing systemd service by name'],
    ], 'node');
    if (options.service === 'custom') options.service = (await ask('Exact systemd service name: ')).trim();
    if (!options.service) throw new Error('A service name is required');
  }
  validateOptions(options);
  return options;
}

export function serviceUnits(service = 'node') {
  if (service === 'all') return ['hoi-node.service', 'caddy.service'];
  const name = { node: 'hoi-node', registry: 'hoi-node', caddy: 'caddy' }[service] ?? service;
  if (!/^[A-Za-z0-9][A-Za-z0-9_.@-]*$/.test(name) || name.length > 200 || /\.(socket|target|timer|mount|slice|path)$/.test(name)) {
    throw new Error('--service needs an exact systemd service name, not a path or pattern');
  }
  return [name.endsWith('.service') ? name : `${name}.service`];
}

export function shellQuote(value) {
  if (value.includes('\0')) throw new Error('Arguments cannot contain NUL');
  return `'${value.replaceAll("'", "'\\''")}'`;
}

function run(command, args, capture = false) {
  const result = spawnSync(command, args, { encoding: 'utf8', stdio: ['inherit', capture ? 'pipe' : 'inherit', 'inherit'] });
  if (result.error) throw new Error(`Cannot run ${command}: ${result.error.message}`);
  if (result.status !== 0) throw new Error(`${command} failed (exit ${result.status})`);
  return result.stdout ?? '';
}

/** Windows' OpenSSH has no connection sharing (ControlMaster); everywhere else one connection
 * carries the whole deployment, so a password or a key's passphrase is asked once. */
export const canShareConnection = (platform = process.platform) => platform !== 'win32';

function sshArgs(options, scp = false) {
  const args = [];
  if (options.port) args.push(scp ? '-P' : '-p', options.port);
  if (options.identity) {
    if (!existsSync(options.identity)) throw new Error(`SSH key not found: ${options.identity}`);
    args.push('-i', options.identity);
  }
  if (options.control) {
    // The socket's path must stay short (104 bytes on macOS): a short temporary directory, %C a hash.
    args.push('-o', 'ControlMaster=auto', '-o', `ControlPath=${join(options.control, 'cm-%C')}`, '-o', 'ControlPersist=60');
  }
  return args;
}

/** Only named systemd services are controlled; the VPS itself is never shut down. */
export function serviceCommand(options) {
  validateOptions(options);
  if (!actions.includes(options.action) || ['deploy', 'check'].includes(options.action)) throw new Error('Choose a service action');
  let script = 'set -eu\n';
  if (options.action === 'services') script += 'systemctl list-units --type=service --all --no-pager';
  else {
    const units = serviceUnits(options.service);
    script += `set -- ${units.map(shellQuote).join(' ')}\n`;
    if (options.service === 'all') {
      script += `set --\nfor unit in hoi-node.service caddy.service; do
  if [ "$(systemctl show --property=LoadState --value "$unit")" = loaded ]; then set -- "$@" "$unit"; fi
done
[ "$#" -gt 0 ] || { echo 'Neither hoi-node nor Caddy is installed.' >&2; exit 1; }
`;
    }
    switch (options.action) {
      case 'status': script += 'systemctl show --property=Id,Description,LoadState,ActiveState,SubState,UnitFileState -- "$@"'; break;
      case 'logs': script += 'for unit do journalctl --no-pager --lines=80 --unit="$unit"; done'; break;
      case 'disable': script += 'systemctl disable --now -- "$@"'; break;
      case 'enable': script += 'systemctl enable --now -- "$@"'; break;
      default: script += `systemctl ${options.action} -- "$@"`;
    }
  }
  const command = `sh -c ${shellQuote(script)}`;
  return `if [ "$(id -u)" = 0 ]; then ${command}; else sudo ${command}; fi`;
}

export function controlService(options, execute = run) {
  const command = serviceCommand(options);
  console.log(`${options.action}: ${options.host}${options.action === 'services' ? '' : ` / ${options.service ?? 'node'}`}`);
  execute('ssh', [...sshArgs(options), '-t', options.host, command]);
}

export function deployBundle(options, execute = run) {
  for (const file of BUNDLE_FILES) {
    if (!existsSync(join(options.bundle, file))) throw new Error(`Missing ${file} in ${options.bundle}. Run pnpm build:server first`);
  }
  // Validate/quote every installer argument before making a connection.
  const args = ['--role', options.role ?? 'config'];
  if (options.action === 'check') args.push('check');
  if (options.registryUrl) args.push('--registry-url', options.registryUrl);
  if (options.tls === 'caddy') args.push('--tls', 'caddy');
  if (options.resetData) args.push('--reset-data', '--confirm-reset', options.confirmReset);
  args.push(...options.installArgs);
  if (options.notice) {
    if (!existsSync(options.notice)) throw new Error(`Notice file not found: ${options.notice}`);
    args.push('--notice-file', './notice.json');
  }
  const flags = args.map(shellQuote).join(' ');
  const parent = resolve(tmpdir());
  const work = mkdtempSync(join(parent, 'hoi-upload-'));
  // One shared connection where OpenSSH can: its socket lives in a short directory of its own.
  const control = canShareConnection(options.platform) ? mkdtempSync(join(existsSync('/tmp') ? '/tmp' : parent, 'hoi-ssh-')) : null;
  const connected = { ...options, control };
  const ssh = sshArgs(connected), scp = sshArgs(connected, true);
  let remote;
  try {
    const archive = join(work, 'bundle.tar.gz');
    const files = ['-C', options.bundle, ...BUNDLE_FILES];
    if (options.notice) {
      copyFileSync(options.notice, join(work, 'notice.json'));
      files.push('-C', work, 'notice.json');
    }
    execute('tar', ['-czf', archive, ...files]);
    console.log(`Connecting to ${options.host}${control ? ' (your SSH password or key passphrase is asked once)' : ' (Windows asks for a password login twice: once here, once for the upload)'}…`);
    // -n: nothing typed here goes to the remote command, so an Enter at the prompt is not swallowed.
    remote = execute('ssh', [...ssh, '-n', options.host, 'umask 077 && mktemp -d /tmp/hoi-deploy.XXXXXXXXXX'], true).trim();
    if (!/^\/tmp\/hoi-deploy\.[A-Za-z0-9]+$/.test(remote)) throw new Error('SSH did not return a valid upload directory; check the remote login output');
    console.log(`Connected. Uploading the bundle to ${options.host}:${remote}…`);
    execute('scp', [...scp, archive, `${options.host}:${remote}/bundle.tar.gz`]);
    console.log(`Uploaded. Installing on ${options.host} (the server's sudo may ask for your password there)…`);
    // The TTY lets sudo ask for a password; no credentials are stored in the bundle.
    execute('ssh', [...ssh, '-t', options.host,
      `cd ${shellQuote(remote)} && tar -xzf bundle.tar.gz && bash ./install.sh ${flags}`]);
    console.log(`Done. Uploaded bundle: ${options.host}:${remote}`);
  } catch (error) {
    if (remote && /^\/tmp\/hoi-deploy\.[A-Za-z0-9]+$/.test(remote)) console.error(`Uploaded files, if any, remain at ${options.host}:${remote}`);
    throw error;
  } finally {
    if (control) {
      // Close the shared connection before its socket directory goes.
      try { execute('ssh', [...ssh, '-O', 'exit', options.host], true); } catch { /* already gone */ }
      if (basename(control).startsWith('hoi-ssh-')) rmSync(control, { recursive: true, force: true });
    }
    if (dirname(work) !== parent || !basename(work).startsWith('hoi-upload-')) throw new Error('Unsafe upload cleanup path');
    rmSync(work, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  let terminal;
  try {
    const options = parseDeployArgs(process.argv.slice(2));
    if (options.help) console.log(help);
    else {
      if (process.stdin.isTTY && process.stdout.isTTY) terminal = createInterface({ input: process.stdin, output: process.stdout });
      const configured = await configureOptions(options, terminal ? prompt => terminal.question(prompt) : undefined);
      terminal?.close();
      if (['deploy', 'check'].includes(configured.action)) deployBundle(configured);
      else controlService(configured);
    }
  } catch (error) {
    console.error(`deploy-server: ${error.message}`);
    process.exitCode = 1;
  } finally {
    terminal?.close();
  }
}
