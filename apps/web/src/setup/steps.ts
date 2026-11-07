// The pure parts of the setup wizard: what the installer printed, the commands to run, what a
// server is called. Nothing here touches a server.

export interface PreflightRow {
  level: 'ok' | 'info' | 'warn' | 'BLOCK';
  what: string;
  text: string;
}

export interface Preflight {
  rows: PreflightRow[];
  /** true: "Ready to install"; false: "Not ready"; null: the installer said neither (it broke off). */
  ready: boolean | null;
  blocks: number;
  warns: number;
}

/** One line per check, the way install.sh prints them (`row`). */
export function parsePreflight(text: string): Preflight {
  const rows: PreflightRow[] = [];
  for (const line of text.split('\n')) {
    const m = /^\s{2}(ok|info|warn|BLOCK)\s+(\S+)\s+(.*)$/.exec(line);
    if (m) rows.push({ level: m[1] as PreflightRow['level'], what: m[2], text: m[3].trim() });
  }
  const ready = /^Ready to install/m.test(text) ? true : /^Not ready/m.test(text) ? false : null;
  return { rows, ready, blocks: rows.filter((r) => r.level === 'BLOCK').length, warns: rows.filter((r) => r.level === 'warn').length };
}

/** The node link the node printed or `cat node-link` gave: the last web link, else the hoi:// one. */
export function nodeLinkIn(text: string): string | null {
  const web = [...text.matchAll(/https:\/\/\S+#\/node\?\S+/g)].map((m) => m[0]);
  if (web.length) return web.at(-1)!;
  const uri = [...text.matchAll(/hoi:\/\/node\?\S+/g)].map((m) => m[0]);
  return uri.at(-1) ?? null;
}

/** `mktemp -d /tmp/hoi-setup.XXXXXXXX` answered with the folder it made. */
export function remoteDirIn(text: string): string | null {
  return /^\/tmp\/hoi-setup\.[A-Za-z0-9]+$/m.exec(text.trim())?.[0] ?? null;
}

const quote = (word: string) => (/^[A-Za-z0-9_./:=@%+-]+$/.test(word) ? word : `'${word.replace(/'/g, `'\\''`)}'`);

/** The install (or its check) in the folder the kit was uploaded to; with `binary`, the hoi-node
 * archive uploaded beside it instead of one from the release list. */
export function installCommand(dir: string, conf: string, check: boolean, binary?: string): string {
  return `cd ${quote(dir)} && bash ./install.sh ${check ? 'check ' : ''}--config ${quote(conf)} --yes${binary ? ` --binary ${quote(binary)}` : ''}`;
}

/** The Rust target of hoi-node a computer runs: `os` as the setup app or `uname -s` says it,
 * `machine` as `uname -m` or Rust's ARCH says it. Null for one hoi-node is not built for. */
export function nodeTarget(os: string, machine: string): string | null {
  const m = machine.trim().toLowerCase();
  const arch = m === 'x86_64' || m === 'amd64' ? 'x86_64' : m === 'aarch64' || m === 'arm64' ? 'aarch64' : null;
  if (!arch) return null;
  const o = os.trim().toLowerCase();
  if (o === 'linux') return `${arch}-unknown-linux-gnu`;
  if (o === 'macos' || o === 'darwin') return `${arch}-apple-darwin`;
  if (o === 'windows' && arch === 'x86_64') return 'x86_64-pc-windows-msvc';
  return null;
}

/** The first archive for `target`: the files come nearest first (beside the app, then builds/). */
export function nodeFileFor<T extends { target: string }>(files: T[], target: string | null): T | null {
  return (target && files.find((f) => f.target === target)) || null;
}

/** "12.3 MB" */
export function megabytes(bytes: number): string {
  return `${Number((bytes / (1024 * 1024)).toFixed(1))} MB`;
}

/** Commands the person may want to see before they run, per action in Manage. */
export const MANAGE = {
  status: "systemctl show --property=ActiveState,SubState,ActiveEnterTimestamp hoi-node; /usr/local/bin/hoi-node --version",
  logs: 'journalctl -u hoi-node -n 200 --no-pager',
  link: 'cat /var/lib/hoi-public/node-link 2>/dev/null || cat /var/lib/hoi-node/node-link',
  hash: 'cat /var/lib/hoi-public/notice-hash 2>/dev/null || cat /var/lib/hoi-node/notice-hash',
  notice: 'cat /etc/hoi-node/notice.json',
  restart: 'systemctl restart hoi-node',
  stop: 'systemctl stop hoi-node',
  start: 'systemctl start hoi-node',
  disable: 'systemctl disable --now hoi-node',
  enable: 'systemctl enable --now hoi-node',
  deleteLogs: 'journalctl --rotate && journalctl --vacuum-time=1s --unit=hoi-node',
  /** Totals of what the node keeps for private communities' mailboxes and pieces; never what is in them. */
  pieces: '/usr/local/bin/hoi-node admin --data /var/lib/hoi-public pieces',
} as const;

/** What a saved configuration says about mailboxes: the gigabytes it keeps for them, or ''. */
export function piecesIn(config: string): string {
  return /^hold_pieces_gb=([0-9]+(?:\.[0-9]+)?)$/m.exec(config)?.[1] ?? '';
}

/** `hoi-node admin pieces` answered: how many pieces, how many MB. */
export function piecesSummary(text: string): string | null {
  const m = /pieces held: (\d+) · ([0-9.]+) MB/.exec(text);
  return m ? `${m[1]} sealed piece${m[1] === '1' ? '' : 's'} kept, ${m[2]} MB` : null;
}

/** Runs `command` as root: as is for root, through sudo for anyone else. */
export function asRoot(user: string, command: string): string {
  return user === 'root' ? command : `sudo sh -c ${quote(command)}`;
}

/** A journald drop-in that keeps the host's logs no longer than `days`: the node logs no
 * addresses, but the server's own logs (sshd, the firewall) carry them. */
export function retentionDropIn(days: number): string {
  return `# Written by hoi node setup: keep this server's logs ${days} day${days === 1 ? '' : 's'} at most, as the operator notice says.\n[Journal]\nMaxRetentionSec=${days}day\n`;
}

/** What a saved server is called in the list. */
export function profileId(host: string, port: number, user: string): string {
  return `${user}@${host}:${port}`;
}

/** The local install's data folder, where the node writes its link. */
export function localNodeLinkCommand(os: string, home: string, role: 'public' | 'registry'): { program: string; args: string[] } {
  const state = role === 'registry' || role === 'public' ? 'hoi-public' : 'hoi-node';
  if (os === 'macos' || os === 'darwin') return { program: 'cat', args: [`${home}/Library/Application Support/hoi-node/public/node-link`] };
  if (os === 'windows') return { program: 'cmd', args: ['/c', 'type', `%LOCALAPPDATA%\\hoi-node\\public\\node-link`] };
  return { program: 'sudo', args: ['-n', 'cat', `/var/lib/${state}/node-link`] };
}

/** Change only the existing holder limit; an empty value disables serving, without deleting data. */
export function mailboxConfig(config: string, gb: string): string {
  const value = gb.trim();
  if (value && (!/^[0-9]+(?:\.[0-9]+)?$/.test(value) || Number(value) < 0.1 || Number(value) > 10000)) throw new Error('Mailbox space must be 0.1 to 10000 GB.');
  const lines = config.replaceAll('\r\n', '\n').split('\n').filter(line => !/^hold_pieces_gb=/.test(line));
  while (lines.at(-1) === '') lines.pop();
  if (value) lines.push(`hold_pieces_gb=${value}`);
  return lines.join('\n') + '\n';
}
