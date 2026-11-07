// A node for a server, made on this device: a random name, the file `install.sh
// --config` reads, and a kit with everything the server needs: that file, the installer, a
// README, a cloud-init file for a server not ordered yet and, for a registry behind your own
// proxy, a Caddyfile. It is `install.sh config` (deploy/install) in the app: the same words, the
// same name and host name rules, the same checks, the same file. Nothing here touches the
// network; the kit is handed to the person as a file and goes nowhere else.

import { PUBLICATION } from './preview';
import { type NodeNotice, noticeByline, noticeJson, noticeTemplate, validateNotice } from './notice';

export type KitRole = 'public' | 'registry';

export interface KitOptions {
  role: KitRole;
  /** The node's label, and the server's host name unless `keepHostname`. */
  name: string;
  /** Registry: its address becomes https://<host name>.<domain>. */
  domain: string;
  /** Registry: an address of your own instead of a domain (takes precedence when set). */
  registryUrl: string;
  /** Registry: Caddy on the server serves its https:// address, certificate included. */
  tls: boolean;
  /** Leave the server's host name as it is. */
  keepHostname: boolean;
  /** Registries the node announces itself to, besides its own. */
  indexes: string[];
  /** Gigabytes a day it may relay for others; empty for hoi-node's default. */
  dailyCapGb: string;
  /** Gigabytes of encrypted pieces and private communities' mailboxes it keeps for others
   * (`--hold-pieces-gb`); empty or 0 for none. The node can read none of it; its notice says so. */
  holdPiecesGb: string;
  /** The operator's join notice: the node carries it, people read it before it serves them. */
  notice: NodeNotice;
}

export interface KitFile {
  name: string;
  /** Unix permissions: 0o755 for the installer, 0o644 for the rest. */
  mode: number;
  text: string;
  /** What it is, in a few words, for the page and the README. */
  about: string;
}

// The same 64 adjectives and 64 nouns as install.sh (tests/server-kit.test.ts holds them equal), so
// every word is as likely, and a five-digit number: about 368 million names. A label, not a
// secret: the node's key is made on the server and is what tells nodes apart.
export const ADJECTIVES = [
  'bashful', 'bouncy', 'breezy', 'bubbly', 'cheeky', 'chirpy', 'comfy', 'cosmic', 'cozy', 'crispy', 'crumbly', 'curious',
  'dapper', 'dizzy', 'dozy', 'dreamy', 'fizzy', 'fluffy', 'frosty', 'fuzzy', 'gentle', 'giddy', 'gleeful', 'goofy', 'groovy', 'grumpy',
  'hasty', 'hazy', 'humble', 'hungry', 'jazzy', 'jolly', 'jumpy', 'lanky', 'lucky', 'mellow', 'mighty', 'misty', 'moody', 'nimble', 'nosy',
  'peppy', 'perky', 'plucky', 'polite', 'puzzled', 'quiet', 'quirky', 'rowdy', 'rusty', 'salty', 'sleepy', 'sneaky', 'snoozy', 'snug',
  'soggy', 'spicy', 'sturdy', 'sunny', 'tiny', 'toasty', 'wobbly', 'wiggly', 'zesty',
];
export const NOUNS = [
  'alpaca', 'badger', 'bagel', 'banjo', 'beaver', 'biscuit', 'capybara', 'crumpet', 'dumpling', 'ferret', 'gecko', 'gnome', 'goose',
  'hedgehog', 'kazoo', 'kettle', 'koala', 'lantern', 'lemur', 'llama', 'lobster', 'manatee', 'marmot', 'mitten', 'mongoose', 'moose',
  'muffin', 'narwhal', 'newt', 'noodle', 'octopus', 'otter', 'owl', 'pancake', 'panda', 'parsnip', 'pelican', 'penguin', 'pickle',
  'pigeon', 'platypus', 'potato', 'pretzel', 'puffin', 'quokka', 'raccoon', 'radish', 'sardine', 'sloth', 'sock', 'spatula', 'squid',
  'teacup', 'teapot', 'toaster', 'trombone', 'turnip', 'ukulele', 'waffle', 'walrus', 'wombat', 'yak', 'yeti', 'zucchini',
];

/** The installer the site serves next to the app (assemble-static puts it at the site's root). */
export const DEFAULT_INSTALLER_URL = 'https://hoi.sh/install.sh';

/** Fills `words` with random numbers. */
type Random = (words: Uint32Array<ArrayBuffer>) => void;
const systemRandom: Random = (words) => {
  crypto.getRandomValues(words);
};

/** A number below `n` from the system's random source, without favouring any. */
export function randomBelow(n: number, random: Random = systemRandom): number {
  const limit = 2 ** 32 - (2 ** 32 % n);
  const word = new Uint32Array(1);
  for (;;) {
    random(word);
    if (word[0] < limit) return word[0] % n;
  }
}

/** sleepy-walrus-48213 */
export function randomName(random: Random = systemRandom): string {
  return `${ADJECTIVES[randomBelow(ADJECTIVES.length, random)]}-${NOUNS[randomBelow(NOUNS.length, random)]}-${10000 + randomBelow(90000, random)}`;
}

/** What install.sh keeps of a name: letters, digits, spaces and . _ ' -, at most 40. */
export function cleanName(name: string): string {
  return name.replace(/[^A-Za-z0-9 ._'-]/g, '').slice(0, 40);
}

/** The host name a name makes, as install.sh's host_of: lowercase letters, digits and dashes, at most 63. */
export function hostOf(name: string): string {
  return name.toLowerCase().replace(/'/g, '').replace(/[^a-z0-9]+/g, '-').replace(/^-+/, '').slice(0, 63).replace(/-+$/, '');
}

const HOST = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/;
const DOMAIN = /^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)+$/;
const HTTPS_URL = /^https:\/\/[A-Za-z0-9.-]+(:[0-9]+)?(\/[A-Za-z0-9._~/-]*)?$/;
const CAP = /^[0-9]+(\.[0-9]+)?$/;
/** What a new node keeps for private communities' mailboxes and pieces unless the operator says otherwise. */
export const DEFAULT_PIECES_GB = '2';

/** Gigabytes of pieces and mailboxes the kit asks for, or '' when it keeps none. */
export function piecesGb(o: Pick<KitOptions, 'holdPiecesGb'>): string {
  const v = o.holdPiecesGb.trim();
  return v && CAP.test(v) && Number(v) > 0 ? v : '';
}

export const blankKit = (name: string): KitOptions => ({
  role: 'registry', name, domain: '', registryUrl: '', tls: true, keepHostname: false, indexes: [], dailyCapGb: '',
  holdPiecesGb: DEFAULT_PIECES_GB, notice: noticeTemplate(name),
});

/** The registry's https:// address, or '' while it can't be made yet. */
export function registryUrlOf(o: KitOptions): string {
  if (o.role !== 'registry') return '';
  if (o.registryUrl.trim()) return o.registryUrl.trim();
  const domain = o.domain.trim().toLowerCase().replace(/^\.+|\.+$/g, '');
  const host = hostOf(cleanName(o.name));
  return domain && host ? `https://${host}.${domain}` : '';
}

export interface KitProblem {
  field: 'name' | 'domain' | 'registryUrl' | 'indexes' | 'dailyCapGb' | 'holdPiecesGb' | `notice.${string}`;
  message: string;
}

/** What install.sh would refuse, in the same words; empty when the kit can be made. */
export function kitProblems(o: KitOptions): KitProblem[] {
  const out: KitProblem[] = [];
  const name = cleanName(o.name.trim());
  if (!name) out.push({ field: 'name', message: 'Give it a name, or roll another.' });
  else if (!HOST.test(hostOf(name))) out.push({ field: 'name', message: 'That name makes no host name: use letters or digits.' });
  if (o.role === 'registry') {
    if (o.registryUrl.trim()) {
      const url = o.registryUrl.trim();
      if (!HTTPS_URL.test(url)) out.push({ field: 'registryUrl', message: 'The registry needs an https:// address, without credentials, query or fragment.' });
      else if (o.tls && /^https:\/\/[0-9.]+(:|\/|$)/.test(url)) out.push({ field: 'registryUrl', message: 'Caddy needs a domain name in the address, not an IP address.' });
      else if (o.tls && !/^https:\/\/[^/]+\/?$/.test(url)) out.push({ field: 'registryUrl', message: 'Caddy serves the registry at the root of its own host name: https://names.example.org, no path.' });
    } else if (!o.domain.trim()) {
      out.push({ field: 'domain', message: 'A registry needs an address: the domain it goes under, or an address of your own.' });
    } else if (!DOMAIN.test(o.domain.trim().toLowerCase().replace(/^\.+|\.+$/g, ''))) {
      out.push({ field: 'domain', message: 'A domain such as example.org: letters, digits, dashes and dots.' });
    }
  }
  for (const url of o.indexes) {
    if (!HTTPS_URL.test(url)) out.push({ field: 'indexes', message: `Not an https:// registry address: ${url}` });
  }
  if (o.dailyCapGb.trim() && !CAP.test(o.dailyCapGb.trim())) out.push({ field: 'dailyCapGb', message: 'A number of gigabytes, such as 5 or 2.5.' });
  const pieces = o.holdPiecesGb.trim();
  if (pieces && (!CAP.test(pieces) || (Number(pieces) !== 0 && (Number(pieces) < 0.1 || Number(pieces) > 10000)))) {
    out.push({ field: 'holdPiecesGb', message: 'A number of gigabytes from 0.1 to 10000, such as 2; 0 keeps none.' });
  }
  // The notice is checked the way the node checks it; its node field follows the kit's name.
  for (const p of validateNotice({ ...o.notice, node: cleanName(o.name.trim()) })) out.push({ field: `notice.${p.field}`, message: p.message });
  return out;
}

/** The notice the kit carries: the kit's name is the node's label in it, and it says whether the
 * node keeps pieces (the node sets that itself from how it runs; the preview shows the same). */
export const kitNotice = (o: KitOptions): NodeNotice => {
  const { pieces: _, ...rest } = o.notice;
  return piecesGb(o) ? { ...rest, node: cleanName(o.name.trim()), pieces: true } : { ...rest, node: cleanName(o.name.trim()) };
};

const describe = (o: KitOptions) => (o.role === 'registry' ? 'hoi public node and registry' : 'hoi public node');
const fileOf = (o: KitOptions) => `hoi-${hostOf(cleanName(o.name))}.conf`;
const noticeFileOf = (o: KitOptions) => `hoi-${hostOf(cleanName(o.name))}.notice.json`;
const folderOf = (o: KitOptions) => `hoi-${hostOf(cleanName(o.name))}`;
const tlsOn = (o: KitOptions) => o.role === 'registry' && o.tls;
/** hoi-NAME.tar.gz, or hoi-NAME.tar where this browser can't gzip. */
const archiveName = (o: KitOptions) => `${folderOf(o)}.${typeof CompressionStream === 'undefined' ? 'tar' : 'tar.gz'}`;
const hostOfUrl = (url: string) => url.replace(/^https:\/\//, '').replace(/[/:].*$/, '');

/** The lines install.sh --config reads: key=value, checked like flags there, never run. */
export function configText(o: KitOptions): string {
  const name = cleanName(o.name.trim());
  const lines = [
    `# A ${describe(o)} for a server, made in the hoi app (You → A node on a server).`,
    '# Nothing here is secret: the node makes its own key where it runs. Copy this file to the',
    '# server and install with it there; flags given there win over these lines, and without',
    '# the hostname line the server keeps its name:',
    `#   ./install.sh check --config ${fileOf(o)}     (is it installable?)`,
    `#   ./install.sh --config ${fileOf(o)}`,
    `role=${o.role}`,
    `name=${name}`,
  ];
  if (!o.keepHostname) lines.push(`hostname=${hostOf(name)}`);
  lines.push(`notice_file=${noticeFileOf(o)}`);
  const registry = registryUrlOf(o);
  if (registry) lines.push(`registry_url=${registry}`);
  for (const url of o.indexes) lines.push(`index=${url}`);
  if (o.dailyCapGb.trim()) lines.push(`daily_cap_gb=${o.dailyCapGb.trim()}`);
  if (piecesGb(o)) lines.push(`hold_pieces_gb=${piecesGb(o)}`);
  if (tlsOn(o)) lines.push('tls=caddy');
  return lines.join('\n') + '\n';
}

/** One shell word: plain words stay as they are, anything else in single quotes. */
export function shellQuote(word: string): string {
  return /^[A-Za-z0-9_./:=@%+-]+$/.test(word) ? word : `'${word.replace(/'/g, `'\\''`)}'`;
}

/** The flags that install the same node with no file at all. */
export function installFlags(o: KitOptions): string[] {
  const name = cleanName(o.name.trim());
  const flags: string[] = [o.role, '--name', name];
  if (!o.keepHostname) flags.push('--hostname', hostOf(name));
  const registry = registryUrlOf(o);
  if (registry) flags.push('--registry-url', registry);
  if (tlsOn(o)) flags.push('--tls', 'caddy');
  for (const url of o.indexes) flags.push('--index', url);
  if (o.dailyCapGb.trim()) flags.push('--daily-cap-gb', o.dailyCapGb.trim());
  if (piecesGb(o)) flags.push('--hold-pieces-gb', piecesGb(o));
  return flags;
}

/** curl … | bash -s -- registry --name … : straight on the server, nothing to copy over. */
export function oneLiner(o: KitOptions, installerUrl: string, check = false): string {
  return `curl -fsSL ${installerUrl} | bash -s -- ${check ? 'check ' : ''}${installFlags(o).map(shellQuote).join(' ')}`;
}

/** Where the site that served this app keeps its installer: `../install.sh` from `/app/` over
 * HTTPS; the project's own site everywhere else (the desktop app, a development server). */
export function installerUrlFor(href: string | undefined): string {
  if (PUBLICATION.site) return `${PUBLICATION.site.replace(/\/+$/, '')}/install.sh`;
  try {
    const url = new URL(href ?? '');
    if (url.protocol === 'https:' && /\/app\/(index\.html)?$/.test(url.pathname)) return new URL('../install.sh', url).href;
  } catch {
    // not a URL: the default below
  }
  return DEFAULT_INSTALLER_URL;
}

/** User data for a server not ordered yet: on its first boot it writes the configuration and runs
 * the installer, which checks the server first. A few hundred bytes: providers cap user data
 * (AWS at 16 KB), so the installer is fetched there, not carried. */
export function cloudInit(o: KitOptions, installerUrl: string): string {
  const conf = `/root/${fileOf(o)}`;
  const indent = (text: string) => text.trimEnd().split('\n').map((l) => `      ${l}`).join('\n');
  const lines = [
    '#cloud-config',
    `# A ${describe(o)} called ${cleanName(o.name.trim())}, made in the hoi app.`,
    "# Paste this into the user data (cloud-init) field when you create the server. On its first",
    '# boot it writes the configuration below and runs the hoi installer, which checks the server',
    '# first and stops before changing anything if something blocks it. What happened:',
    '#   sudo cat /var/log/cloud-init-output.log',
  ];
  // cloud-init keeps /etc/hosts in step with this name on images that manage it.
  if (!o.keepHostname) lines.push(`hostname: ${hostOf(cleanName(o.name.trim()))}`);
  lines.push(
    'write_files:',
    `  - path: ${conf}`,
    '    permissions: "0644"',
    '    content: |',
    indent(configText(o)),
    `  - path: /root/${noticeFileOf(o)}`,
    '    permissions: "0644"',
    '    content: |',
    indent(noticeJson(kitNotice(o))),
    'runcmd:',
    `  - curl -fsSL --proto '=https' --retry 5 -o /root/hoi-install.sh ${installerUrl}`,
    `  - bash /root/hoi-install.sh --config ${conf} --yes`,
  );
  return lines.join('\n') + '\n';
}

/** For a registry behind a Caddy you run yourself (no --tls caddy): the site to add to it. */
export function caddyfile(o: KitOptions): string {
  return [
    `# The hoi registry ${registryUrlOf(o)}, for a Caddy you run yourself. Caddy gets and renews the`,
    '# certificate by itself and keeps no access log unless told to. Add this to your Caddyfile,',
    '# or run: caddy run --config Caddyfile',
    `${hostOfUrl(registryUrlOf(o))} {`,
    '\treverse_proxy 127.0.0.1:7845',
    '}',
    '',
  ].join('\n');
}

export function readme(o: KitOptions, installerUrl: string, files: KitFile[]): string {
  const name = cleanName(o.name.trim());
  const host = hostOf(name);
  const conf = fileOf(o);
  const folder = folderOf(o);
  const registry = registryUrlOf(o);
  // UDP 7846 and 7847: the DHT among public nodes
  const udp = 'UDP 7843, 7844, 7846 and 7847';
  const ports = tlsOn(o) ? `${udp}, and TCP 80 and 443 (Caddy)` : udp;
  const out = [
    `# ${folder}: a ${describe(o)}`,
    '',
    'Made in the hoi app (You → A node on a server), on your own device: nothing in this folder was',
    'sent anywhere, and nothing in it is secret. The node makes its own key on the server.',
    '',
    `- name: ${name}${o.keepHostname ? '' : ` (also the server's host name: name it ${host} when you order it, or the installer does)`}`,
  ];
  if (registry) out.push(`- registry: ${registry}${tlsOn(o) ? ', served over HTTPS by Caddy, which the installer sets up' : ', behind your own TLS proxy'}`);
  out.push(`- operator: ${noticeByline(o.notice).replace(/^by /, '')} (your join notice is in \`${noticeFileOf(o)}\`; keep it next to the configuration; everyone who connects reads it)`);
  out.push(piecesGb(o)
    ? `- mailboxes: keeps up to ${piecesGb(o)} GB of encrypted pieces and private communities' mailboxes for other people's devices. It can read none of it, and its notice says so.`
    : "- mailboxes: off. Private communities can't keep their mailboxes here (hold_pieces_gb turns it on).");
  out.push('', '| File | What it is |', '|---|---|', ...files.filter((f) => f.name !== 'README.md').map((f) => `| \`${f.name}\` | ${f.about} |`));
  out.push(
    '',
    '## What the server needs',
    '',
    '- 64-bit Linux with systemd: Debian 12, Ubuntu 22.04 or newer, Fedora, Raspberry Pi OS (64-bit)',
    '  or the like. A Mac works too.',
    `- 512 MB of memory and 1 GB of disk at least; 1 GB and 2 GB or more are comfortable${piecesGb(o) ? `, plus ${piecesGb(o)} GB for the mailboxes` : ''}.`,
    '- A public IPv4 or IPv6 address, or a router that forwards the ports.',
    `- ${ports} open in the provider's firewall (a security group, a cloud firewall).`,
  );
  if (registry) out.push(`- DNS: an A record (and AAAA for IPv6) for ${hostOfUrl(registry)} that points at the server.`);
  if (tlsOn(o)) out.push('- Caddy comes from the system\'s packages: Debian 12, Ubuntu 24.04 and Fedora have it; elsewhere', '  install it first (https://caddyserver.com/docs/install).');
  out.push(
    '',
    '## Check it first',
    '',
    '```sh',
    `./install.sh check --config ${conf}`,
    '```',
    '',
    'One line per check: the system and its glibc, systemd, root or sudo, the release files, disk,',
    `memory, the clock, the ports, the host firewall, the server's addresses${registry ? ', DNS for the registry' : ''}${tlsOn(o) ? ', Caddy' : ''},`,
    'the host name and a node installed before. Each says ok, info, warn (it installs, but something',
    "won't work as well) or BLOCK (it can't be installed yet, and says why). The install runs the same",
    'check and stops before changing anything on a BLOCK.',
    '',
    '## Install it: three ways',
    '',
    '### 1. Copy this folder to the server',
    '',
    '```sh',
    `scp ${archiveName(o)} root@SERVER:`,
    'ssh root@SERVER',
    `tar -xf ${archiveName(o)} && cd ${folder}`,
    `./install.sh check --config ${conf}`,
    `./install.sh --config ${conf}`,
    '```',
    '',
    'As a user with sudo, the installer asks for the password when it needs it.',
    '',
    '### 2. When you order the server: cloud-init',
    '',
    'Most providers have a field called user data, cloud-init or cloud config on the page where you',
    'create a server (DigitalOcean, Hetzner, Vultr, Linode, Scaleway, AWS and others). Paste',
    '`cloud-init.yaml` into it. On its first boot the server writes the configuration and installs',
    'the node; log in afterwards and read what happened:',
    '',
    '```sh',
    'sudo cat /var/log/cloud-init-output.log',
    '```',
    '',
    '### 3. Straight on the server, no files',
    '',
    `Flags take the place of the configuration; your notice still has to be on the server: copy \`${noticeFileOf(o)}\``,
    'over first (`scp`), then add `--notice-file` and its path to the install command.',
    '',
    '```sh',
    oneLiner(o, installerUrl, true),
    `${oneLiner(o, installerUrl)} --notice-file ./${noticeFileOf(o)}`,
    '```',
    '',
    '### A server without the internet',
    '',
    'Put the release files for the server into this folder: `manifest.json`, `manifest.json.minisig`',
    'and the hoi-node archive for its processor, `hoi-node_VERSION_x86_64-unknown-linux-gnu.tar.gz`',
    '(`aarch64` for an ARM server such as a Raspberry Pi). They are on the release\'s page on GitHub',
    'and on https://hoi.sh/download/ with their checksums. The installer uses the files beside it',
    'and checks each against the release list, then the first way above works offline.',
    '',
    '## Afterwards',
    '',
    '```sh',
    'sudo cat /var/lib/hoi-public/node-link   # the node\'s link, your notice in it: anyone who opens it reads the notice, then finds what it carries',
    'sudo cat /var/lib/hoi-public/notice-hash # the hash of the notice the node carries now',
    'sudo nano /etc/hoi-node/notice.json       # change your notice: the node reads it again within half a minute',
    'journalctl -u hoi-node -f               # what it is doing',
  );
  if (registry) out.push(`curl ${registry}/v1/meta   # the registry answers`);
  out.push(
    `./install.sh --config ${conf}       # update: run it again; the data stays`,
    './install.sh uninstall                  # remove the service; the data stays',
    '```',
  );
  if (registry) {
    out.push(
      '',
      'The published app uses no registry: a copy of the app built with this one',
      `(VITE_HOI_REGISTRY_URL=${registry}) reaches it, and nodes started with --index ${registry}`,
      'announce themselves there.',
    );
  }
  return out.join('\n') + '\n';
}

/** `text` with Unix line ends. Every file of the kit goes to a Linux server, where bash reads a
 * CR at the end of a line as part of it (`set: pipefail\r: invalid option name`); a copy of the
 * app built from a Windows checkout carries the installer with CRLF. */
export function unixLines(text: string): string {
  return text.replace(/\r\n?/g, '\n');
}

/** Every file of the kit, the installer included, in the order they go into the archive. */
export function kitFiles(o: KitOptions, installer: string, installerUrl: string): KitFile[] {
  installer = unixLines(installer);
  const files: KitFile[] = [
    { name: fileOf(o), mode: 0o644, text: configText(o), about: 'the node: what install.sh --config reads (key=value lines, never run)' },
    { name: noticeFileOf(o), mode: 0o644, text: noticeJson(kitNotice(o)), about: 'your join notice: what the node keeps and, as far as you filled them in, who runs it, how to reach you and where it is; the node carries it and people read it before it serves them. On the server it lives at /etc/hoi-node/notice.json: edit it there and the node reads it again' },
    { name: 'install.sh', mode: 0o755, text: installer, about: `the hoi installer, the one ${installerUrl} serves: it checks the server, fetches hoi-node, checks its SHA-256 and sets it up as a service` },
    { name: 'cloud-init.yaml', mode: 0o644, text: cloudInit(o, installerUrl), about: 'the same install as user data, for a server you have not ordered yet' },
  ];
  if (o.role === 'registry' && !o.tls) files.push({ name: 'Caddyfile', mode: 0o644, text: caddyfile(o), about: 'the registry\'s site for a Caddy you run yourself' });
  files.unshift({ name: 'README.md', mode: 0o644, text: '', about: 'how to check and install it' });
  files[0].text = readme(o, installerUrl, files);
  for (const f of files) f.text = unixLines(f.text);
  return files;
}

// ---------------------------------------------------------------- the archive

const enc = new TextEncoder();

/** One ustar header field: text, NUL-padded to `size`. */
function field(header: Uint8Array, offset: number, size: number, text: string): void {
  const bytes = enc.encode(text);
  if (bytes.length > size) throw new Error(`tar: ${text} does not fit`);
  header.set(bytes, offset);
}
const octal = (n: number, size: number) => n.toString(8).padStart(size - 1, '0');

/** A POSIX ustar archive of `files` under one folder, the way `tar -xzf` everywhere reads it. */
export function tar(folder: string, files: KitFile[], mtime = Math.floor(Date.now() / 1000)): Uint8Array {
  const entries: { name: string; mode: number; data: Uint8Array; dir?: boolean }[] = [
    { name: `${folder}/`, mode: 0o755, data: new Uint8Array(0), dir: true },
    ...files.map((f) => ({ name: `${folder}/${f.name}`, mode: f.mode, data: enc.encode(f.text) })),
  ];
  const blocks = entries.reduce((n, e) => n + 1 + Math.ceil(e.data.length / 512), 2);
  // Whole 10 KiB records, as tar writes them.
  const out = new Uint8Array(Math.ceil((blocks * 512) / 10240) * 10240);
  let at = 0;
  for (const e of entries) {
    const h = out.subarray(at, at + 512);
    field(h, 0, 100, e.name);
    field(h, 100, 8, octal(e.mode, 8));
    field(h, 108, 8, octal(0, 8));
    field(h, 116, 8, octal(0, 8));
    field(h, 124, 12, octal(e.data.length, 12));
    field(h, 136, 12, octal(mtime, 12));
    h.fill(0x20, 148, 156); // the checksum counts its own field as spaces
    field(h, 156, 1, e.dir ? '5' : '0');
    field(h, 257, 6, 'ustar');
    field(h, 263, 2, '00');
    field(h, 265, 32, 'root');
    field(h, 297, 32, 'root');
    let sum = 0;
    for (const b of h) sum += b;
    field(h, 148, 8, `${octal(sum, 7)}\0`);
    out.set(e.data, at + 512);
    at += 512 + Math.ceil(e.data.length / 512) * 512;
  }
  return out;
}

/** gzip, where this browser has CompressionStream; null where it hasn't. */
export async function gzip(bytes: Uint8Array): Promise<Uint8Array | null> {
  if (typeof CompressionStream === 'undefined') return null;
  const stream = new Blob([bytes as BlobPart]).stream().pipeThrough(new CompressionStream('gzip'));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

/** The kit as one file: hoi-NAME.tar.gz (hoi-NAME.tar where the browser can't gzip). */
export async function kitArchive(o: KitOptions, installer: string, installerUrl: string, mtime?: number): Promise<{ name: string; bytes: Uint8Array }> {
  const plain = tar(folderOf(o), kitFiles(o, installer, installerUrl), mtime);
  const packed = await gzip(plain);
  return { name: archiveName(o), bytes: packed ?? plain };
}

export { archiveName, fileOf as configFileName, folderOf as kitFolderName, noticeFileOf as noticeFileName };
