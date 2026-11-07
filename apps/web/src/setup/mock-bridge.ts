// A server that isn't there, for the wizard in a plain browser: a host key to pin, a check with
// its rows, an install that prints what the real one prints, and a node link with a notice in it.
// This computer has a hoi-node of its own built (pnpm build:all), which the wizard uploads.
import type { Bridge, LogLine, NodeFile, Profile, UploadProgress } from './bridge';

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
const SOURCE_ROW = '  ok     source      hoi 9.9.9, hoi-node_9.9.9_x86_64-unknown-linux-gnu.tar.gz';
/** A real node link's code (core/vectors/node-notice.json): routes on 203.0.113.9 and a signed
 * notice, packed and masked as hoi-node writes it, so no address shows in the link. */
const NODE_CODE =
  'SE9OAV1m6-1etSPtkXN7LS0qm2OTNFkZoLVKbr_WavppSXRLV5XvR_WeqoiiQys3TcvbFBM6WrmYv-WW7ADwomqC55J8iE4-jacwxNLSi3Y-d3oYP-zL213OieHQWbHxxf8yy7nrgjcOt_GZnZPrWbdKaNwCYj2PxDBizCOWj7soEByqW6Eonf82YwdQ5vTEPbL0tBKPXOP9dKVW6iS6zpLTH6k5nDKixiC2wgmu_bqLm2RErSGntutjmq5L4sUxJ_rXdniCvV_33-9xEvjbmH6jBJcYPW3K7ZJI6yOUCz0v-rltVK3KAxmXgf0bI0vXiGhwWlHeQ2XriDKJ6CwxnTn3lw0oc5fynVzeoNo_5aRWgRg3eUOs901T5NT09nsvtQhfeJTx1ufd-gkgy0fZ3KjV6dqGhHZxF75glJCI_nhJ2O38djeO4PRLQsJ8PGicPRteD3HNR_L5vATBkiIg9l-F0UJUN0TvP4mJ-3w3gdmaGnB13tXClKrXgic3MyEYd9LfAZu05wSIrYsh50le1YxZsr_MXXoBVbWgxN6DQooRoex88jd2IG1hdMH3QlV1OEkYZFTTcfPi4CQl8bWdH9nmSsJvlhKYPYO-T4c93GeP0Q';

const BUILT = (target: string, bytes: number): NodeFile => ({
  path: `/home/lena/hoi/builds/hoi-9.9.9/server/hoi-node_9.9.9_${target}.tar.gz`,
  name: `hoi-node_9.9.9_${target}.tar.gz`,
  version: '9.9.9',
  target,
  bytes,
});

export function mockBridge(): Bridge {
  const listeners = new Set<(line: LogLine) => void>();
  const uploads = new Set<(progress: UploadProgress) => void>();
  let profiles: Profile[] = [];
  const say = async (run: string, lines: string[], ms = 60) => {
    for (const text of lines) {
      await sleep(ms);
      for (const fn of listeners) fn({ run, stream: 'out', text });
    }
  };
  const checkRows = [
    'Checking this computer for a hoi public node called sleepy-walrus-48213:',
    '  ok     system      Linux x86_64 (Debian GNU/Linux 12 (bookworm)), glibc 2.36',
    '  ok     services    systemd 252',
    '  ok     rights      root',
    SOURCE_ROW,
    '  ok     disk        23.4 GB free in /var/lib',
    '  ok     memory      1.9 GB',
    '  ok     clock       synchronized',
    '  ok     ports       UDP 7843 and 7844 free',
    "  info   firewall    a firewall in front of the server (your provider's, a security group) has to let UDP 7843, 7844 in too",
    '  ok     address     public: 203.0.113.9',
    "  ok     pieces      keeps up to 2 GB of encrypted pieces and private communities' mailboxes for others; it can read none of them, and its notice says so",
    '  ok     notice      /tmp/hoi-setup.mock/hoi-sleepy-walrus-48213.notice.json, by Lena (checked again with hoi-node before the install)',
    'Ready to install.',
  ];
  /** With --binary the installer names the archive it was given; else the release list's. */
  const rowsFor = (command: string) => {
    const given = /--binary '?([^ ']+)/.exec(command)?.[1];
    return checkRows.map((row) => (row === SOURCE_ROW && given ? `  ok     source      ${given}` : row));
  };
  /** The pretend server's answer to a command the wizard reads, or null for one it just echoes. */
  const answerTo = (command: string): string | null => {
    if (command === 'uname -m') return 'x86_64\n';
    if (command.includes('mktemp')) return '/tmp/hoi-setup.mock\n';
    if (command.includes('node-link')) return `https://hoi.sh/app/#/node?code=${NODE_CODE}\n`;
    if (command.includes('notice-hash')) return 'c0ffee'.repeat(10) + 'c0ff\n';
    if (command.includes('systemctl show')) return 'ActiveState=active\nSubState=running\nActiveEnterTimestamp=Tue 2026-11-03 10:00:00 UTC\nhoi-node 9.9.9\n';
    if (command.includes('journalctl -u')) return 'Nov 03 10:00:00 sleepy-walrus hoi-node[1]: hoi-node 9.9.9 · /var/lib/hoi-public · public node · 3 communities kept warm\nNov 03 10:00:01 sleepy-walrus hoi-node[1]: operator notice: by Lena · contact lena at example dot org · server in XX · c0ffeec0ffee\n';
    if (command.includes('admin --data /var/lib/hoi-public pieces')) return 'pieces held: 1832 · 412.6 MB · none readable here\nexpiring within a week: 97\n';
    if (/(^|')cat \/etc\/hoi-node\/notice\.json/.test(command)) return '{"v":1,"node":"sleepy-walrus-48213","operator":"Lena","contact":"lena at example dot org","country":"XX","processes":"Nothing beyond that.","ipLogging":false,"retentionDays":0,"updated":"2026-11-03"}\n';
    return null;
  };
  return {
    async profiles() { return profiles; },
    async saveProfile(profile) { profiles = [profile, ...profiles.filter((p) => p.id !== profile.id)]; return profiles; },
    async removeProfile(id) { profiles = profiles.filter((p) => p.id !== id); return profiles; },
    async probe(target) {
      await sleep(300);
      if (target.host === 'down.example') throw { code: 'error', message: `could not reach ${target.host}:${target.port}` };
      return { algorithm: 'ssh-ed25519', fingerprint: 'SHA256:mockmockmockmockmockmockmockmockmockmockmoc' };
    },
    async connect(target, auth, hostKey) {
      await sleep(300);
      if (!hostKey.startsWith('SHA256:')) throw { code: 'error', message: 'the host key is not pinned' };
      if (auth.kind === 'password' && auth.password !== 'correct horse') throw { code: 'error', message: `${target.user}@${target.host} did not accept that login` };
      return 'mock-session';
    },
    async run(_session, run, command, pty, answer) {
      if (command.includes(' check ')) { await say(run, rowsFor(command), 30); return { code: 0, text: '' }; }
      if (command.includes('install.sh') && command.includes('--yes') && !command.includes('uninstall')) {
        if (pty && !answer) { await say(run, ['[sudo] password for lena: ']); }
        const given = /--binary '?([^ ']+)/.exec(command)?.[1];
        await say(run, [given ? `hoi-node from ${given}` : 'fetching hoi-node_9.9.9_x86_64-unknown-linux-gnu.tar.gz', '+ install -m 0644 notice /etc/hoi-node/notice.json', '+ write /etc/systemd/system/hoi-node.service', 'hoi public node runs as a service and starts with the computer.'], 80);
        return { code: 0, text: '' };
      }
      if (command.includes('uninstall')) { await say(run, ['removed the hoi-node service; its data stays']); return { code: 0, text: '' }; }
      // What the server would print for the commands Manage and the install's last steps run.
      const known = answerTo(command);
      await say(run, known === null ? [`+ ${command}`] : known.split('\n').filter(Boolean), 20);
      return { code: 0, text: '' };
    },
    async read(_session, command) {
      await sleep(80);
      return { code: 0, text: answerTo(command) ?? '' };
    },
    async upload() { await sleep(40); },
    async uploadNode(_session, run, path, dir) {
      const file = [BUILT('x86_64-unknown-linux-gnu', 13_421_772), BUILT('aarch64-unknown-linux-gnu', 12_582_912)].find((f) => f.path === path);
      if (!file) throw { code: 'error', message: `${path.split('/').at(-1)} is not a hoi-node archive` };
      for (const share of [0.25, 0.5, 0.75, 1]) {
        await sleep(60);
        for (const fn of uploads) fn({ run, sent: Math.round(file.bytes * share), total: file.bytes });
      }
      return { name: file.name, path: `${dir}/${file.name}`, sha256: 'ab'.repeat(32), bytes: file.bytes };
    },
    async nodeFiles() { return [BUILT('aarch64-unknown-linux-gnu', 12_582_912), BUILT('x86_64-unknown-linux-gnu', 13_421_772)]; },
    async pickNode() { return BUILT('x86_64-unknown-linux-gnu', 13_421_772); },
    async disconnect() {},
    async localRun(run, program, args) { await say(run, [`+ ${program} ${args.join(' ')}`, ...checkRows], 20); return { code: 0, text: '' }; },
    async writeLocal(name) { return `/tmp/hoi-setup-mock/${name}`; },
    async platform() { return { os: 'linux', arch: 'x86_64', user: 'lena', home: '/home/lena' }; },
    async pickFile() { return '/home/lena/.ssh/id_ed25519'; },
    async saveFile() { return true; },
    async openUrl(url) { window.open(url, '_blank', 'noopener'); },
    async onLog(fn) { listeners.add(fn); return () => listeners.delete(fn); },
    async onUpload(fn) { uploads.add(fn); return () => uploads.delete(fn); },
  };
}
