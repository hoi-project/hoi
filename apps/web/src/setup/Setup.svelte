<script lang="ts">
  // The hoi node setup app's one screen: a wizard that installs a public node on a
  // server over SSH, or on this computer, and a Manage view for the servers set up here. The
  // files the server gets are the kit page's (lib/server-kit.ts); the words people read in the
  // notice are the client's (lib/notice.ts); the server is reached through the Rust side
  // (setup/bridge.ts). Every command runs in the open: it is printed before it runs.
  import { onMount } from 'svelte';
  import { type Bridge, type HostKey, type NodeFile, type Platform, type Profile, type Auth, getBridge, inTauri } from './bridge';
  import { MANAGE, asRoot, installCommand, localNodeLinkCommand, megabytes, nodeFileFor, nodeLinkIn, nodeTarget, parsePreflight, mailboxConfig, piecesIn, piecesSummary, profileId, remoteDirIn, retentionDropIn, type Preflight } from './steps';
  import { type KitRole, DEFAULT_PIECES_GB, cleanName, configFileName, configText, hostOf, kitFiles, kitNotice, kitProblems, noticeFileName, piecesGb, randomName, registryUrlOf, unixLines } from '../lib/server-kit';
  import { MAX_NOTICE_BYTES, canonicalNotice, isoDate, noticeFooter, noticeJson, noticeSections, noticeTemplate, noticeTitle, type NodeNotice } from '../lib/notice';
  import { OPERATOR_RESPONSIBILITY } from '../lib/legal/texts';
  import NoticeEditor from '../lib/ui/NoticeEditor.svelte';
  import NoticeText from '../lib/ui/NoticeText.svelte';
  import QR from '../lib/ui/QR.svelte';
  import Logo from '../lib/ui/Logo.svelte';
  import installerRaw from '../../../../deploy/install/install.sh?raw';
  import installerPs1 from '../../../../deploy/install/install.ps1?raw';
  import { PUBLICATION } from '../lib/preview';

  const INSTALLER_URL = `${PUBLICATION.site || 'https://hoi.sh'}/install.sh`;
  const releaseConfigured = !!PUBLICATION.downloads && !!PUBLICATION.manifestKey;
  /** The installer as the server needs it: LF line ends, however this copy of the app was built. */
  const installerText = unixLines(installerRaw);
  const STEPS = ['Responsibility', 'Server', 'Host key', 'The node', 'Join notice', 'Check', 'Install', 'Done'] as const;
  /** Each step's headline, the way the website sets them: the first part in ink, the second in blue. */
  const HEADS: Record<number, [string, string]> = {
    1: ['Before you run', 'a public node.'],
    2: ['Where does it run?', 'A server or this computer.'],
    3: ['Is this your server?', 'Check its key.'],
    4: ['What it runs.', 'And what it keeps.'],
    5: ['Your join notice.', 'In your own words.'],
    6: ['Before it installs.', 'Nothing has changed yet.'],
    8: ['Your node is up.', 'Share its link.'],
  };

  let bridge = $state<Bridge | null>(null);
  let platform = $state<Platform>({ os: 'linux', arch: 'x86_64', user: '', home: '' });
  let view = $state<'wizard' | 'servers' | 'manage'>('wizard');
  let step = $state(1);
  let busy = $state('');
  let error = $state('');
  let profiles = $state<Profile[]>([]);

  // ---- 1 responsibility
  let acknowledged = $state(false);

  // ---- 2 the server
  let targetKind = $state<'ssh' | 'local'>('ssh');
  let host = $state('');
  let port = $state('22');
  let user = $state('root');
  let authKind = $state<'agent' | 'key' | 'password'>('agent');
  let keyPath = $state('');
  let passphrase = $state('');
  let password = $state('');
  const target = $derived({ host: host.trim(), port: Number.parseInt(port, 10) || 22, user: user.trim() || 'root' });
  const auth = $derived<Auth>(authKind === 'agent' ? { kind: 'agent' } : authKind === 'key' ? { kind: 'key', path: keyPath.trim(), passphrase: passphrase || undefined } : { kind: 'password', password });

  // ---- 3 the host key
  let hostKey = $state<HostKey | null>(null);
  let knownKey = $state<string | null>(null);
  let session = $state<string | null>(null);

  // ---- 4 the node
  let role = $state<KitRole>('public');
  let name = $state(randomName());
  let domain = $state('');
  let ownAddress = $state(false);
  let registryUrl = $state('');
  let tls = $state(true);
  let keepHostname = $state(false);
  let indexText = $state('');
  let cap = $state('');
  let advertise = $state('');
  let holdPieces = $state(true);
  let piecesText = $state(DEFAULT_PIECES_GB);

  // ---- 4 the program the server gets: a hoi-node built on this computer (pnpm build:all), so no
  // published release is needed, or the one from the signed release list.
  let nodeFiles = $state<NodeFile[]>([]);
  let nodeSource = $state<'file' | 'release'>('release');
  let nodeFile = $state<NodeFile | null>(null);
  let serverMachine = $state('');
  const wantTarget = $derived(targetKind === 'local' ? nodeTarget(platform.os, platform.arch) : nodeTarget('linux', serverMachine));
  const useFile = $derived(nodeSource === 'file' && !!nodeFile && nodeFile.target === wantTarget);

  // ---- 5 the notice
  let notice = $state<NodeNotice>(noticeTemplate('', isoDate(new Date())));
  let retentionText = $state('7');

  const opts = $derived({
    role, name, domain, registryUrl: ownAddress ? registryUrl : '', tls, keepHostname,
    indexes: indexText.split(/\s+/).filter(Boolean), dailyCapGb: cap, holdPiecesGb: holdPieces ? piecesText : '',
    notice: { ...notice, retentionDays: notice.ipLogging ? Number.parseInt(retentionText, 10) || 0 : 0, country: notice.country.trim().toUpperCase() },
  });
  const problems = $derived(kitProblems(opts));
  const problem = (field: string) => problems.find((p) => p.field === field)?.message;
  const nodeProblems = $derived(problems.filter((p) => !p.field.startsWith('notice.')));
  const noticeProblems = $derived(problems.filter((p) => p.field.startsWith('notice.')));
  const shown = $derived(kitNotice(opts));
  const noticeBytes = $derived(canonicalNotice(shown).length);
  const conf = $derived(configFileName(opts));
  const noticeFile = $derived(noticeFileName(opts));
  const hostName = $derived(hostOf(cleanName(name.trim())));
  const registry = $derived(registryUrlOf(opts));

  // ---- 6 the check, 7 the install
  let remoteDir = $state('');
  let localDir = $state('');
  let preflight = $state<Preflight | null>(null);
  /** The hoi-node archive uploaded beside the kit, as install.sh --binary is given it; '' for the release list. */
  let remoteBinary = $state('');
  let uploadPct = $state<number | null>(null);
  let sudoPassword = $state('');
  let log = $state<{ run: string; stream: 'out' | 'err' | 'cmd'; text: string }[]>([]);
  const runs = new Map<string, string[]>();
  let runN = 1;
  let logEl: HTMLElement | undefined = $state();

  // ---- 8 done
  let nodeLink = $state('');
  let noticeHash = $state('');

  // ---- manage
  let current = $state<Profile | null>(null);
  let status = $state('');
  let logsText = $state('');
  let manageNotice = $state<NodeNotice | null>(null);
  let manageRetention = $state('7');
  let retentionDays = $state('30');
  let eraseText = $state('');
  let managePassword = $state('');
  let piecesUse = $state('');

  onMount(async () => {
    bridge = await getBridge();
    platform = await bridge.platform();
    profiles = await bridge.profiles();
    nodeFiles = await bridge.nodeFiles();
    await bridge.onUpload((p) => { uploadPct = p.total ? Math.round((p.sent / p.total) * 100) : null; });
    await bridge.onLog((line) => {
      log.push(line);
      runs.get(line.run)?.push(line.text);
      queueMicrotask(() => logEl?.scrollTo({ top: logEl.scrollHeight }));
    });
  });

  function fail(e: unknown): string {
    const m = (e as { message?: string })?.message ?? String(e);
    error = m;
    return m;
  }

  /** Runs one command, printed first, and returns what it printed and its exit code. */
  async function run(command: string, pty: boolean, answer?: string): Promise<{ code: number; text: string }> {
    if (!bridge || !session) throw new Error('not connected');
    const id = `r${runN++}`;
    runs.set(id, []);
    log.push({ run: id, stream: 'cmd', text: `${target.user}@${target.host}$ ${command}` });
    const result = await bridge.run(session, id, command, pty, answer);
    return { code: result.code, text: runs.get(id)?.join('\n') ?? '' };
  }

  async function runLocal(program: string, args: string[], stdin?: string): Promise<{ code: number; text: string }> {
    if (!bridge) throw new Error('no bridge');
    const id = `l${runN++}`;
    runs.set(id, []);
    log.push({ run: id, stream: 'cmd', text: `$ ${program} ${args.join(' ')}` });
    const result = await bridge.localRun(id, program, args, stdin);
    return { code: result.code, text: runs.get(id)?.join('\n') ?? '' };
  }

  // ---------------------------------------------------------------- steps

  function next() {
    error = '';
    if (step === 4 && !useFile && !releaseConfigured) { error = 'Pick a local hoi-node archive. Published downloads are not configured in this build.'; return; }
    step = Math.min(step + 1, STEPS.length);
  }

  async function probe() {
    if (!bridge) return;
    error = '';
    if (targetKind === 'local') {
      chooseNode();
      step = 4;
      return;
    }
    if (!target.host) { error = 'Where is the server? A host name or an address.'; return; }
    if (authKind === 'key' && !keyPath.trim()) { error = 'Which key file?'; return; }
    busy = 'probe';
    try {
      hostKey = await bridge.probe(target);
      const known = profiles.find((p) => p.id === profileId(target.host, target.port, target.user))?.hostKey ?? null;
      knownKey = known;
      step = 3;
    } catch (e) {
      fail(e);
    }
    busy = '';
  }

  async function pinAndConnect() {
    if (!bridge || !hostKey) return;
    if (knownKey && knownKey !== hostKey.fingerprint) { error = `This server's host key differs from the one pinned here before (${knownKey}). Stop unless you know why; forget the server under Your servers to start over.`; return; }
    busy = 'connect';
    error = '';
    try {
      session = await bridge.connect(target, auth, hostKey.fingerprint);
      serverMachine = (await run('uname -m', false)).text.trim();
      chooseNode();
      step = 4;
    } catch (e) {
      fail(e);
    }
    busy = '';
  }

  /** The hoi-node from this computer that fits the server, when there is one; else the release list. */
  function chooseNode() {
    if (nodeFile && nodeFile.target === wantTarget) return;
    nodeFile = nodeFileFor(nodeFiles, wantTarget);
    nodeSource = nodeFile ? 'file' : 'release';
  }

  async function pickNode() {
    if (!bridge) return;
    error = '';
    try {
      const picked = await bridge.pickNode();
      if (!picked) return;
      const where = targetKind === 'local' ? 'this computer' : 'this server';
      if (wantTarget && picked.target !== wantTarget) { error = `${picked.name} is for ${picked.target}; ${where} needs ${wantTarget}.`; return; }
      nodeFile = picked;
      nodeSource = 'file';
      if (!nodeFiles.some((f) => f.path === picked.path)) nodeFiles = [picked, ...nodeFiles];
    } catch (e) {
      fail(e);
    }
  }

  /** Uploads a hoi-node archive beside the kit; returns what install.sh --binary is given. */
  async function uploadNode(dir: string, file: NodeFile): Promise<string> {
    if (!bridge || !session) throw new Error('not connected');
    log.push({ run: 'up', stream: 'cmd', text: `upload ${file.name} (${megabytes(file.bytes)}) → ${dir}/${file.name}` });
    uploadPct = 0;
    try {
      const up = await bridge.uploadNode(session, `u${runN++}`, file.path, dir);
      log.push({ run: 'up', stream: 'out', text: `arrived whole: SHA-256 ${up.sha256}` });
      return `./${up.name}`;
    } finally {
      uploadPct = null;
    }
  }

  /** install.sh / install.ps1 arguments for the hoi-node from this computer, on this computer. */
  const localBinaryArgs = () => (useFile && nodeFile ? (platform.os === 'windows' ? ['-Binary', nodeFile.path] : ['--binary', nodeFile.path]) : []);

  async function pickKey() {
    if (!bridge) return;
    const picked = await bridge.pickFile();
    if (picked) keyPath = picked;
  }

  /** Uploads the kit (configuration, notice, installer) to a private folder on the server. */
  async function uploadKit(): Promise<string> {
    if (!bridge || !session) throw new Error('not connected');
    const made = await run('umask 077 && mktemp -d /tmp/hoi-setup.XXXXXXXX', false);
    const dir = remoteDirIn(made.text);
    if (!dir) throw new Error(`the server made no upload folder: ${made.text.trim()}`);
    for (const f of kitFiles(opts, installerText, INSTALLER_URL)) {
      log.push({ run: 'up', stream: 'cmd', text: `upload ${f.name} → ${dir}/${f.name}` });
      await bridge.upload(session, `${dir}/${f.name}`, f.text, f.mode === 0o755 ? '0755' : '0644');
    }
    return dir;
  }

  async function writeKitLocally(): Promise<string> {
    if (!bridge) throw new Error('no bridge');
    let dir = '';
    for (const f of kitFiles(opts, installerText, INSTALLER_URL)) {
      const path = await bridge.writeLocal(f.name, f.text);
      dir = path.slice(0, path.length - f.name.length - 1);
    }
    if (platform.os === 'windows') await bridge.writeLocal('install.ps1', installerPs1);
    return dir;
  }

  async function check() {
    busy = 'check';
    error = '';
    preflight = null;
    try {
      if (targetKind === 'local') {
        localDir = await writeKitLocally();
        const sep = platform.os === 'windows' ? '\\' : '/';
        const result = platform.os === 'windows'
          ? await runLocal('powershell', ['-ExecutionPolicy', 'Bypass', '-File', `${localDir}${sep}install.ps1`, '-Role', role, '-Name', cleanName(name.trim()), '-NoticeFile', `${localDir}${sep}${noticeFile}`, ...(piecesGb(opts) ? ['-HoldPiecesGb', piecesGb(opts)] : []), ...localBinaryArgs(), '-DryRun', '-Yes'])
          : await runLocal('bash', [`${localDir}/install.sh`, 'check', '--config', `${localDir}/${conf}`, '--yes', ...localBinaryArgs()]);
        preflight = platform.os === 'windows' ? { rows: [{ level: result.code === 0 ? 'ok' : 'BLOCK', what: 'dry run', text: result.code === 0 ? 'install.ps1 would install it' : 'install.ps1 refused; see above' }], ready: result.code === 0, blocks: result.code === 0 ? 0 : 1, warns: 0 } : parsePreflight(result.text);
      } else {
        remoteDir = await uploadKit();
        remoteBinary = useFile && nodeFile ? await uploadNode(remoteDir, nodeFile) : '';
        const result = await run(installCommand(remoteDir, conf, true, remoteBinary || undefined), false);
        preflight = parsePreflight(result.text);
      }
      step = 6;
    } catch (e) {
      fail(e);
    }
    busy = '';
  }

  async function install() {
    if (!bridge) return;
    busy = 'install';
    error = '';
    try {
      let code: number;
      if (targetKind === 'local') {
        if (platform.os === 'windows') {
          code = (await runLocal('powershell', ['-ExecutionPolicy', 'Bypass', '-File', `${localDir}\\install.ps1`, '-Role', role, '-Name', cleanName(name.trim()), '-NoticeFile', `${localDir}\\${noticeFile}`, ...(piecesGb(opts) ? ['-HoldPiecesGb', piecesGb(opts)] : []), ...localBinaryArgs(), '-Yes'])).code;
        } else {
          if (platform.os === 'linux' && platform.user !== 'root') {
            const ok = await runLocal('sudo', ['-S', '-p', '', '-v'], sudoPassword);
            if (ok.code !== 0) throw new Error('sudo did not accept that password');
          }
          code = (await runLocal('bash', [`${localDir}/install.sh`, '--config', `${localDir}/${conf}`, '--yes', ...localBinaryArgs()])).code;
        }
      } else {
        const answer = target.user === 'root' ? undefined : (sudoPassword || (authKind === 'password' ? password : undefined));
        code = (await run(installCommand(remoteDir, conf, false, remoteBinary || undefined), true, answer)).code;
      }
      if (code !== 0) throw new Error(`the installer stopped (exit ${code}); the log above says why`);
      await afterInstall();
      step = 8;
    } catch (e) {
      fail(e);
    }
    busy = '';
  }

  async function afterInstall() {
    if (!bridge) return;
    if (targetKind === 'local') {
      const where = localNodeLinkCommand(platform.os, platform.home, role);
      nodeLink = nodeLinkIn((await runLocal(where.program, where.args)).text) ?? '';
      noticeHash = '';
    } else {
      const link = await run(asRoot(target.user, MANAGE.link), true, target.user === 'root' ? undefined : (sudoPassword || (authKind === 'password' ? password : undefined)));
      nodeLink = nodeLinkIn(link.text) ?? '';
      const hash = await run(asRoot(target.user, MANAGE.hash), true, target.user === 'root' ? undefined : (sudoPassword || (authKind === 'password' ? password : undefined)));
      noticeHash = /^[0-9a-f]{64}$/m.exec(hash.text)?.[0] ?? '';
    }
    const id = targetKind === 'local' ? 'local' : profileId(target.host, target.port, target.user);
    const profile: Profile = {
      id, label: cleanName(name.trim()), host: targetKind === 'local' ? 'local' : target.host, port: target.port, user: targetKind === 'local' ? platform.user : target.user,
      auth: authKind === 'key' ? { kind: 'key', path: keyPath.trim() } : authKind === 'password' ? { kind: 'password' } : { kind: 'agent' },
      hostKey: hostKey?.fingerprint, role, name: cleanName(name.trim()), nodeLink: nodeLink || undefined, noticeHash: noticeHash || undefined,
      config: configText(opts), notice: noticeJson(shown), created: 0, lastUsed: 0,
    };
    profiles = await bridge.saveProfile(profile);
    current = profiles.find((p) => p.id === id) ?? profile;
  }

  async function copy(text: string) {
    try { await navigator.clipboard.writeText(text); } catch { /* the field is selectable */ }
  }

  async function openInHoi() {
    if (bridge && nodeLink.startsWith('https://')) await bridge.openUrl(nodeLink);
  }

  function startOver() {
    step = 1; session = null; hostKey = null; preflight = null; remoteBinary = ''; serverMachine = ''; nodeLink = ''; noticeHash = ''; log = []; error = ''; sudoPassword = ''; password = ''; passphrase = '';
    view = 'wizard';
  }

  // ---------------------------------------------------------------- manage

  async function open(p: Profile) {
    if (!bridge) return;
    current = p;
    view = 'manage';
    error = '';
    status = '';
    logsText = '';
    manageNotice = null;
    piecesUse = '';
    log = [];
    if (p.host === 'local') { status = 'This computer: use the commands the installer printed (journalctl -u hoi-node -f) for now.'; return; }
    // Right after an install the login is still open: no second password for the same server.
    const sameServer = !!session && p.id === profileId(target.host, target.port, target.user);
    if (session && !sameServer) { void bridge.disconnect(session); session = null; }
    host = p.host; port = String(p.port); user = p.user; authKind = p.auth.kind; keyPath = p.auth.kind === 'key' ? p.auth.path : '';
    if (sameServer) { await refreshStatus(); return; }
    if (p.auth.kind === 'password' && !managePassword) { status = 'Enter the password to connect.'; return; }
    await connectCurrent();
  }

  async function connectCurrent() {
    if (!bridge || !current || !current.hostKey) return;
    busy = 'connect';
    try {
      password = managePassword;
      session = await bridge.connect({ host: current.host, port: current.port, user: current.user }, auth, current.hostKey);
      await refreshStatus();
    } catch (e) {
      fail(e);
    }
    busy = '';
  }

  const sudoAnswer = () => (target.user === 'root' ? undefined : (sudoPassword || managePassword || undefined));

  async function refreshStatus() {
    if (!session) return;
    const s = await run(asRoot(target.user, MANAGE.status), true, sudoAnswer());
    const active = /ActiveState=(\S+)/.exec(s.text)?.[1] ?? '?';
    const sub = /SubState=(\S+)/.exec(s.text)?.[1] ?? '';
    const since = /ActiveEnterTimestamp=(.*)/.exec(s.text)?.[1]?.trim() ?? '';
    const version = /hoi-node (\S+)/.exec(s.text)?.[1] ?? '';
    status = `${active}${sub ? ` (${sub})` : ''}${since ? ` since ${since}` : ''}${version ? ` · hoi-node ${version}` : ''}`;
    const link = await run(asRoot(target.user, MANAGE.link), true, sudoAnswer());
    nodeLink = nodeLinkIn(link.text) ?? current?.nodeLink ?? '';
    const hash = await run(asRoot(target.user, MANAGE.hash), true, sudoAnswer());
    noticeHash = /^[0-9a-f]{64}$/m.exec(hash.text)?.[0] ?? '';
  }

  async function act(what: keyof typeof MANAGE) {
    if (!session) return;
    busy = what;
    error = '';
    try {
      const r = await run(asRoot(target.user, MANAGE[what]), true, sudoAnswer());
      if (what === 'logs') logsText = r.text;
      else if (what === 'pieces') piecesUse = piecesSummary(r.text) ?? (/holds no pieces/.test(r.text) ? 'Nothing kept yet: the mailboxes fill as private communities choose this node.' : 'The node did not say; the log below shows its answer.');
      else await refreshStatus();
    } catch (e) {
      fail(e);
    }
    busy = '';
  }

  async function loadNotice() {
    if (!session) return;
    const r = await run(asRoot(target.user, MANAGE.notice), true, sudoAnswer());
    try {
      const parsed = JSON.parse(r.text) as NodeNotice;
      manageNotice = { ...noticeTemplate(parsed.node ?? ''), ...parsed };
      manageRetention = String(parsed.retentionDays ?? 7);
    } catch {
      error = 'The notice on the server could not be read; the log shows what it said.';
    }
  }

  async function saveNotice() {
    if (!bridge || !session || !manageNotice || !current) return;
    busy = 'notice';
    error = '';
    try {
      const n: NodeNotice = { ...manageNotice, retentionDays: manageNotice.ipLogging ? Number.parseInt(manageRetention, 10) || 0 : 0, country: manageNotice.country.trim().toUpperCase(), updated: isoDate(new Date()) };
      const text = noticeJson(n);
      await bridge.upload(session, '/tmp/hoi-notice.json', text, '0644');
      const r = await run(asRoot(target.user, 'install -m 0644 /tmp/hoi-notice.json /etc/hoi-node/notice.json && rm -f /tmp/hoi-notice.json'), true, sudoAnswer());
      if (r.code !== 0) throw new Error('the notice could not be written on the server');
      current = { ...current, notice: text };
      profiles = await bridge.saveProfile(current);
      status = 'Notice saved: the node reads it again within half a minute, and people who accepted the old words read the new ones first.';
    } catch (e) {
      fail(e);
    }
    busy = '';
  }

  async function setRetention() {
    if (!bridge || !session) return;
    const days = Number.parseInt(retentionDays, 10);
    if (!days || days < 1 || days > 3650) { error = 'Days: 1 to 3650.'; return; }
    busy = 'retention';
    error = '';
    try {
      await bridge.upload(session, '/tmp/hoi-journald.conf', retentionDropIn(days), '0644');
      const r = await run(asRoot(target.user, 'mkdir -p /etc/systemd/journald.conf.d && install -m 0644 /tmp/hoi-journald.conf /etc/systemd/journald.conf.d/hoi.conf && rm -f /tmp/hoi-journald.conf && systemctl restart systemd-journald'), true, sudoAnswer());
      if (r.code !== 0) throw new Error('journald did not take the retention');
      status = `The server keeps its logs ${days} days at most now.`;
    } catch (e) {
      fail(e);
    }
    busy = '';
  }

  let mailboxLimit = $state(DEFAULT_PIECES_GB);
  async function setMailboxes(enabled: boolean) {
    if (!current) return;
    try { await update(mailboxConfig(current.config, enabled ? mailboxLimit : '')); }
    catch (e) { fail(e); }
  }

  async function update(configuration?: string) {
    if (!bridge || !session || !current) return;
    busy = 'update';
    error = '';
    try {
      const made = await run('umask 077 && mktemp -d /tmp/hoi-setup.XXXXXXXX', false);
      const dir = remoteDirIn(made.text);
      if (!dir) throw new Error('no upload folder');
      const confName = `hoi-${hostOf(current.name)}.conf`;
      await bridge.upload(session, `${dir}/${confName}`, unixLines(configuration ?? current.config), '0644');
      await bridge.upload(session, `${dir}/hoi-${hostOf(current.name)}.notice.json`, unixLines(current.notice), '0644');
      await bridge.upload(session, `${dir}/install.sh`, installerText, '0755');
      // The same choice as New node: a hoi-node from this computer that fits the server, else the release list.
      const file = nodeFileFor(nodeFiles, nodeTarget('linux', (await run('uname -m', false)).text));
      if (!configuration && !file && !releaseConfigured) throw new Error('Pick a local hoi-node archive before updating; published downloads are not configured.');
      const binary = configuration ? '/usr/local/bin/hoi-node' : file ? await uploadNode(dir, file) : '';
      const r = await run(installCommand(dir, confName, false, binary || undefined), true, sudoAnswer());
      if (r.code !== 0) throw new Error(`the installer stopped (exit ${r.code})`);
      if (configuration) {
        current = { ...current, config: configuration };
        profiles = await bridge.saveProfile(current);
        piecesUse = '';
      }
      await refreshStatus();
      status = `${configuration ? 'Mailbox settings applied; the node restarted.' : 'Updated.'} ${status}`;
    } catch (e) {
      fail(e);
    }
    busy = '';
  }

  async function uninstall() {
    if (!bridge || !session || !current) return;
    busy = 'uninstall';
    error = '';
    try {
      const made = await run('umask 077 && mktemp -d /tmp/hoi-setup.XXXXXXXX', false);
      const dir = remoteDirIn(made.text);
      if (!dir) throw new Error('no upload folder');
      await bridge.upload(session, `${dir}/install.sh`, installerText, '0755');
      const r = await run(`cd ${dir} && bash ./install.sh uninstall --yes`, true, sudoAnswer());
      if (r.code !== 0) throw new Error(`the uninstall stopped (exit ${r.code})`);
      if (eraseText === current.name) await run(asRoot(target.user, 'rm -rf /var/lib/hoi-public /var/lib/private/hoi-public /etc/hoi-node'), true, sudoAnswer());
      profiles = await bridge.removeProfile(current.id);
      status = eraseText === current.name ? 'Removed, with its data.' : 'Removed; the data stays in /var/lib/hoi-public.';
      current = null;
      view = 'servers';
    } catch (e) {
      fail(e);
    }
    busy = '';
  }

  async function forget(p: Profile) {
    if (!bridge) return;
    profiles = await bridge.removeProfile(p.id);
  }
</script>

<header class="bar">
  <div class="bar-in gutter">
    <span class="logo"><Logo size={30} /><b>hoi</b><span class="sh">node setup</span></span>
    <nav class="btnrow" aria-label="Views">
      <button type="button" class={['pill pill-sm', view === 'wizard' && 'pill-ink']} onclick={() => (view = 'wizard')}>New node</button>
      <button type="button" class={['pill pill-sm', view !== 'wizard' && 'pill-ink']} onclick={() => { view = 'servers'; error = ''; }}>Your servers{#if profiles.length}{` · ${profiles.length}`}{/if}</button>
    </nav>
  </div>
</header>

<main class="setup gutter">
  {#if !inTauri()}<p class="callout">This is the setup app's window in a plain browser: a pretend server answers here. The real app is on <a class="textlink" href="https://hoi.sh/download/#operator">hoi.sh/download</a>.</p>{/if}

  {#if view === 'wizard'}
    <ol class="steps" aria-label="Steps">
      {#each STEPS as s, i (s)}
        <li class={[i + 1 === step && 'now', i + 1 < step && 'done']} aria-current={i + 1 === step ? 'step' : undefined} title={s}>
          <span class="num">{i + 1 < step ? '✓' : i + 1}</span><span class="lbl">{s}</span>
        </li>
      {/each}
    </ol>

    <div class="hero">
      <span class="eyebrow">step {step} of {STEPS.length} · {STEPS[step - 1].toLowerCase()}</span>
      {#if step === 7}
        <h1>{busy === 'install' ? 'Installing…' : error ? 'The install stopped.' : 'Installed.'} {#if busy === 'install'}<span class="blue">Line by line.</span>{/if}</h1>
      {:else}
        <h1>{HEADS[step][0]} <span class="blue">{HEADS[step][1]}</span></h1>
      {/if}
    </div>

    {#if step === 1}
      <div class="ledger">
        {#each OPERATOR_RESPONSIBILITY.sections as sec (sec.title)}
          <div class="lrow">
            <b class="lk">{sec.title}</b>
            <div class="lv">
              {#if sec.text}<p>{sec.text}</p>{/if}
              {#if sec.bullets}<ul>{#each sec.bullets as b (b)}<li>{b}</li>{/each}</ul>{/if}
            </div>
          </div>
        {/each}
      </div>
      <section class="frame card">
        <label class="ack"><input id="st-ack" type="checkbox" bind:checked={acknowledged} /> <span>{OPERATOR_RESPONSIBILITY.check}</span></label>
        <div class="btnrow"><button type="button" class="pill pill-ink pill-lg" disabled={!acknowledged} onclick={next}>Continue</button></div>
      </section>

    {:else if step === 2}
      <section class="frame card">
        <div class="choice" role="radiogroup" aria-label="Where">
          <label><input type="radio" name="st-target" value="ssh" bind:group={targetKind} /><span>A server, over SSH</span></label>
          <label><input type="radio" name="st-target" value="local" bind:group={targetKind} /><span>This computer</span></label>
        </div>
        {#if targetKind === 'ssh'}
          <div class="grid2">
            <label class="field"><span>Host</span><input id="st-host" class="input mono-in" bind:value={host} placeholder="203.0.113.9 or server.example.org" autocomplete="off" spellcheck="false" /></label>
            <label class="field"><span>Port</span><input id="st-port" class="input mono-in" bind:value={port} inputmode="numeric" /></label>
            <label class="field"><span>User</span><input id="st-user" class="input mono-in" bind:value={user} autocomplete="off" spellcheck="false" /></label>
            <div class="field">
              <span>Login</span>
              <div class="choice small" role="radiogroup" aria-label="Login">
                <label><input type="radio" name="st-auth" value="agent" bind:group={authKind} /><span>ssh-agent</span></label>
                <label><input type="radio" name="st-auth" value="key" bind:group={authKind} /><span>Key file</span></label>
                <label><input type="radio" name="st-auth" value="password" bind:group={authKind} /><span>Password</span></label>
              </div>
            </div>
          </div>
          {#if authKind === 'key'}
            <div class="grid2">
              <label class="field"><span>Key file</span><span class="inrow"><input id="st-key" class="input mono-in" bind:value={keyPath} placeholder="~/.ssh/id_ed25519" spellcheck="false" /><button type="button" class="pill pill-sm" onclick={pickKey}>Pick…</button></span></label>
              <label class="field"><span>Passphrase (if the key has one)</span><input id="st-passphrase" class="input" type="password" bind:value={passphrase} autocomplete="off" /></label>
            </div>
          {:else if authKind === 'password'}
            <label class="field"><span>Password</span><input id="st-password" class="input" type="password" bind:value={password} autocomplete="off" /><span class="help">Used for this session only, never stored. A key or the agent is the better way; the installer can set one up on the server later.</span></label>
          {/if}
          {#if profiles.length}
            <p class="help">Set up here before: {#each profiles.filter((p) => p.host !== 'local') as p (p.id)}<button type="button" class="textlink" onclick={() => { host = p.host; port = String(p.port); user = p.user; authKind = p.auth.kind; keyPath = p.auth.kind === 'key' ? p.auth.path : ''; }}>{p.label} ({p.id})</button>{' '}{/each}</p>
          {/if}
        {:else}
          <p class="help">The node is installed on this {platform.os === 'darwin' || platform.os === 'macos' ? 'Mac, in your account' : platform.os === 'windows' ? 'Windows computer, for your account' : 'computer as a system service (sudo asks for your password)'}: a Raspberry Pi or a computer that stays on, with a public address or a router that forwards the node's ports.</p>
        {/if}
        <div class="btnrow"><button type="button" class="pill" onclick={() => (step = 1)}>Back</button><button type="button" class="pill pill-ink pill-lg" disabled={busy !== ''} onclick={probe}>{busy === 'probe' ? 'Looking…' : targetKind === 'ssh' ? 'Look at the server' : 'Continue'}</button></div>
      </section>

    {:else if step === 3}
      <section class="frame card">
        <p>{target.user}@{target.host}:{target.port} answered with this host key. Compare it with what your provider shows, or with <code>ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub</code> on the server. It is pinned from now on: a changed key stops everything.</p>
        {#if hostKey}
          <p class="keybox mono"><b>{hostKey.algorithm}</b><br />{hostKey.fingerprint}</p>
          {#if knownKey && knownKey === hostKey.fingerprint}<p class="help">The same key as when this server was set up here before.</p>{/if}
          {#if knownKey && knownKey !== hostKey.fingerprint}<p class="callout err" role="alert">Different from the key pinned here before: {knownKey}. Stop unless you know why (a reinstalled server, say).</p>{/if}
        {/if}
        <div class="btnrow"><button type="button" class="pill" onclick={() => (step = 2)}>Back</button><button type="button" class="pill pill-ink pill-lg" disabled={busy !== '' || !hostKey || (!!knownKey && knownKey !== hostKey.fingerprint)} onclick={pinAndConnect}>{busy === 'connect' ? 'Connecting…' : 'This is my server: pin it and log in'}</button></div>
      </section>

    {:else if step === 4}
      <section class="frame card">
        <div class="choice" role="radiogroup" aria-label="What the server runs">
          <label><input type="radio" name="st-role" value="public" bind:group={role} /><span>Public node</span></label>
          <label><input type="radio" name="st-role" value="registry" bind:group={role} /><span>Public node and registry</span></label>
        </div>
        <div class="grid2">
          <label class="field"><span>Name</span><span class="inrow"><input id="st-name" class="input mono-in" bind:value={name} maxlength="40" spellcheck="false" /><button type="button" class="pill pill-sm" onclick={() => (name = randomName())}>Another</button></span>{#if problem('name')}<span class="help bad">{problem('name')}</span>{:else if !keepHostname}<span class="help">The server's host name becomes {hostName}.</span>{/if}</label>
          <label class="check field"><input type="checkbox" bind:checked={keepHostname} /> <span>Leave the server's own host name alone</span></label>
          {#if role === 'registry'}
            {#if ownAddress}
              <label class="field"><span>Registry address</span><input id="st-url" class="input mono-in" bind:value={registryUrl} placeholder="https://names.example.org" spellcheck="false" />{#if problem('registryUrl')}<span class="help bad">{problem('registryUrl')}</span>{/if}</label>
            {:else}
              <label class="field"><span>Domain</span><span class="inrow"><span class="mono pre">{hostName || 'name'}.</span><input id="st-domain" class="input mono-in" bind:value={domain} placeholder="example.org" spellcheck="false" /></span>{#if problem('domain') && domain.trim()}<span class="help bad">{problem('domain')}</span>{:else if registry}<span class="help">The registry: {registry}. Point DNS for it at the server.</span>{/if}</label>
            {/if}
            <label class="check field"><input type="checkbox" bind:checked={ownAddress} /> <span>Use an address of my own</span></label>
            <label class="check field wide"><input type="checkbox" bind:checked={tls} /> <span>Set up Caddy on the server (HTTPS with its own certificate)</span></label>
          {/if}
        </div>

        <div class={['mailboxes', holdPieces && 'on']}>
          <label class="check big"><input id="st-pieces-on" type="checkbox" bind:checked={holdPieces} /> <span>Keep mailboxes for private communities <span class="tag tag-mint">recommended</span></span></label>
          <p>Private communities keep their messages in mailboxes on a few public nodes in different places. Their members' devices find nodes that offer them by themselves; nobody has to set anything up. Your node keeps only sealed pieces it cannot read, deletes each when it expires, and its join notice says so.</p>
          {#if holdPieces}
            <label class="field inline" for="st-pieces"><span>Up to</span><input id="st-pieces" class="input capin" bind:value={piecesText} inputmode="decimal" autocomplete="off" /><span>GB of disk</span></label>
            {#if problem('holdPiecesGb')}<span class="help bad">{problem('holdPiecesGb')}</span>{/if}
          {:else}<p class="help">Off: private communities can't keep their mailboxes on this node.</p>{/if}
        </div>

        <div class={['program', useFile && 'on']}>
          <b class="program-h">The program <span class="tag tag-soft mono">{wantTarget ?? 'hoi-node'}</span></b>
          {#if nodeFile}
            <div class="choice small" role="radiogroup" aria-label="Where hoi-node comes from">
              <label><input type="radio" name="st-node" value="file" bind:group={nodeSource} /><span>Built on this computer</span></label>
              <label><input type="radio" name="st-node" value="release" bind:group={nodeSource} disabled={!releaseConfigured} /><span>The signed release list</span></label>
            </div>
          {/if}
          {#if useFile && nodeFile}
            <p class="help"><span class="mono file">{nodeFile.name}</span> · {megabytes(nodeFile.bytes)}. Uploaded with the rest and checked when it arrives: no published release is needed.</p>
          {:else}
            <p class="help">{releaseConfigured ? 'The installer verifies the signed release list with its bundled key and checks the downloaded file. The target computer needs minisign.' : 'Published downloads are not configured in this development build. Pick a local hoi-node archive you trust.'}{#if !nodeFile} No hoi-node for {targetKind === 'local' ? 'this computer' : 'this server'} was found beside the app: build one with <code>pnpm build:all</code>, or pick the file.{/if}</p>
          {/if}
          <div class="btnrow"><button type="button" class="pill pill-sm" onclick={pickNode}>{nodeFile ? 'Another file…' : 'Pick a hoi-node file…'}</button></div>
        </div>

        <details class="more"><summary>More</summary>
          <div class="grid2">
            <label class="field"><span>Also announce to these registries</span><textarea id="st-index" class="textarea mono-in" rows="2" bind:value={indexText} placeholder="https://names.example.net"></textarea>{#if problem('indexes')}<span class="help bad">{problem('indexes')}</span>{/if}</label>
            <label class="field"><span>Daily cap (GB)</span><input id="st-cap" class="input capin" bind:value={cap} inputmode="decimal" placeholder="5" />{#if problem('dailyCapGb')}<span class="help bad">{problem('dailyCapGb')}</span>{/if}</label>
            <label class="field"><span>Public address, when NAT maps it (IP:7844)</span><input id="st-advertise" class="input mono-in" bind:value={advertise} placeholder="203.0.113.9:7844" spellcheck="false" /><span class="help">Only for a server whose public address is mapped through NAT; the check says when.</span></label>
          </div>
        </details>
        <div class="btnrow"><button type="button" class="pill" onclick={() => (step = targetKind === 'local' ? 2 : 3)}>Back</button><button type="button" class="pill pill-ink pill-lg" disabled={nodeProblems.length > 0} onclick={next}>Continue</button></div>
      </section>

    {:else if step === 5}
      <section class="frame card">
        <p class="lead">Everyone who connects reads this before the node serves them: your words in the fields, the rest the same on every node. It travels in the node's link and in its first word to every stranger. Your name, your contact and where the server is are optional, but you should give them; whatever you fill in is shown to everyone who connects to this node.</p>
        <NoticeEditor bind:notice bind:retentionText {problem} idPrefix="st" />
        <div class="preview">
          <div class="kit-h"><h2>As people read it</h2><span class="tag tag-soft mono">{noticeBytes} of {MAX_NOTICE_BYTES} bytes{noticeBytes <= 1100 ? ' · fits a QR code' : ''}</span>{#if shown.pieces}<span class="tag tag-mint">says it keeps mailboxes</span>{/if}</div>
          <NoticeText title={noticeTitle(shown)} sections={noticeSections(shown)} footer={noticeFooter(shown)} />
          {#if problem('notice.notice')}<p class="help bad" role="alert">{problem('notice.notice')}</p>{/if}
        </div>
        <div class="btnrow"><button type="button" class="pill" onclick={() => (step = 4)}>Back</button><button type="button" class="pill pill-ink pill-lg" disabled={noticeProblems.length > 0 || busy !== ''} onclick={check}>{busy === 'check' ? (uploadPct !== null ? `Uploading hoi-node… ${uploadPct}%` : 'Checking the server…') : 'Upload and check the server'}</button></div>
        {#if noticeProblems.length}<p class="help">First: {noticeProblems[0].message}</p>{/if}
      </section>

    {:else if step === 6}
      <section class="frame card">
        {#if preflight}
          <table class="checks">
            <tbody>
              {#each preflight.rows as r, i (i)}
                <tr class={r.level}><td class="lvl"><span class="tag">{r.level}</span></td><td class="what">{r.what}</td><td>{r.text}</td></tr>
              {/each}
            </tbody>
          </table>
          {#if preflight.ready === false || preflight.blocks}
            <p class="callout err" role="alert">Not ready: fix what says BLOCK, then check again. Nothing on the server changed.</p>
          {:else if preflight.ready}
            <p class="help">Ready{preflight.warns ? `, with ${preflight.warns} warning${preflight.warns === 1 ? '' : 's'} above` : ''}.</p>
          {:else}
            <p class="callout err" role="alert">The check broke off; the log below says where.</p>
          {/if}
        {/if}
        {#if (targetKind === 'ssh' && target.user !== 'root' && authKind !== 'password') || (targetKind === 'local' && platform.os === 'linux' && platform.user !== 'root')}
          <label class="field"><span>sudo password on the server</span><input id="st-sudo" class="input" type="password" bind:value={sudoPassword} autocomplete="off" /><span class="help">The installer runs as root through sudo; asked for this session only.</span></label>
        {/if}
        <div class="btnrow">
          <button type="button" class="pill" onclick={() => (step = 5)}>Back</button>
          <button type="button" class="pill" disabled={busy !== ''} onclick={check}>Check again</button>
          <button type="button" class="pill pill-ink pill-lg" disabled={busy !== '' || !preflight || preflight.ready !== true} onclick={() => { step = 7; void install(); }}>Install</button>
        </div>
      </section>

    {:else if step === 7}
      <section class="frame card">
        <p class="lead">What the installer does, as it does it: {remoteBinary || (targetKind === 'local' && useFile) ? 'the hoi-node from this computer, checked when it arrived' : 'the binary from the signed release list, checked'}; the notice next to the service; the service, hardened; the firewall.</p>
        {#if error}<div class="btnrow"><button type="button" class="pill" onclick={() => (step = 6)}>Back to the check</button><button type="button" class="pill pill-ink pill-lg" onclick={install}>Try again</button></div>{/if}
      </section>

    {:else if step === 8}
      <section class="frame card">
        {#if nodeLink}
          <div class="done">
            <div class="done-text">
              <p class="lead">Its link, with your notice in it: anyone who opens it reads the notice, then finds the communities it carries. Share it where you want people to find your node; found a community on it by opening the link in hoi yourself.</p>
              {#if piecesGb(opts)}<p class="note-sun">It keeps mailboxes for private communities too, up to {piecesGb(opts)} GB: their members' devices find it by themselves.</p>{/if}
            </div>
            <QR value={nodeLink} label="QR code of the node's link" color="mint" />
          </div>
          <div class="linkrow">
            <textarea class="textarea mono-in" rows="3" readonly value={nodeLink} aria-label="The node's link"></textarea>
            <div class="btnrow"><button type="button" class="pill pill-sm" onclick={() => copy(nodeLink)}>Copy</button><button type="button" class="pill pill-sm pill-ink" onclick={openInHoi}>Open in hoi</button></div>
          </div>
          {#if noticeHash}<p class="help mono">Notice hash: {noticeHash}</p>{/if}
        {:else}
          <p class="callout">The node has no link yet: it needs a public address, or the router has not forwarded its ports. <code>sudo cat /var/lib/hoi-public/node-link</code> on the server once it has one, or open Manage later.</p>
        {/if}
        <div class="btnrow"><button type="button" class="pill pill-ink pill-lg" onclick={() => current && open(current)}>Manage this server</button><button type="button" class="pill" onclick={startOver}>Set up another</button></div>
      </section>
    {/if}

    {#if error}<p class="callout err" role="alert">{error}</p>{/if}
    {#if log.length}
      <details class="logbox" open={step >= 6}>
        <summary>What was run on {targetKind === 'local' ? 'this computer' : target.host} ({log.length} lines)</summary>
        <!-- svelte-ignore a11y_no_noninteractive_tabindex (Scrollable logs need keyboard focus.) -->
        <pre tabindex="0" role="region" aria-label="Command log" class="log" bind:this={logEl}>{#each log as l, i (i)}<span class={l.stream}>{l.text}</span>{'\n'}{/each}</pre>
      </details>
    {/if}

  {:else if view === 'servers'}
    <div class="hero">
      <span class="eyebrow">set up here · remembered here</span>
      <h1>Your servers. <span class="blue">Never a password.</span></h1>
    </div>
    <section class="frame card">
      {#if !profiles.length}<p class="help">None yet. Set one up under New node; it is remembered here, with its host key and never with a password.</p>{/if}
      <ul class="servers">
        {#each profiles as p (p.id)}
          <li>
            <span class="srv"><b>{p.label}</b><span class="mono">{p.id}</span><span class="tags"><span class="tag tag-soft">{p.role === 'registry' ? 'node and registry' : 'public node'}</span>{#if p.hostKey}<span class="tag tag-sky">key pinned</span>{/if}{#if piecesIn(p.config)}<span class="tag tag-mint">mailboxes · {piecesIn(p.config)} GB</span>{/if}</span></span>
            <span class="btnrow"><button type="button" class="pill pill-sm pill-ink" onclick={() => open(p)}>Manage</button><button type="button" class="pill pill-sm" onclick={() => forget(p)}>Forget</button></span>
          </li>
        {/each}
      </ul>
    </section>

  {:else if view === 'manage' && current}
    <div class="hero">
      <span class="eyebrow mono-eyebrow">{current.id}</span>
      <h1>{current.label} <span class="blue">{current.role === 'registry' ? 'node and registry.' : 'public node.'}</span></h1>
    </div>
    <section class="frame card">
      {#if !session && current.host !== 'local'}
        {#if current.auth.kind === 'password'}<label class="field"><span>Password</span><input class="input" type="password" bind:value={managePassword} autocomplete="off" /></label>{/if}
        {#if current.user !== 'root'}<label class="field"><span>sudo password</span><input class="input" type="password" bind:value={sudoPassword} autocomplete="off" /></label>{/if}
        <div class="btnrow"><button type="button" class="pill pill-ink" disabled={busy !== ''} onclick={connectCurrent}>{busy === 'connect' ? 'Connecting…' : 'Connect'}</button></div>
      {/if}
      {#if status}<p class="status"><span class={['dot', /^active/.test(status) && 'on']} aria-hidden="true"></span>{status}</p>{/if}
      {#if session}
        <div class="btnrow" role="group" aria-label="The service">
          <button type="button" class="pill pill-sm" disabled={busy !== ''} onclick={refreshStatus}>Status</button>
          <button type="button" class="pill pill-sm" disabled={busy !== ''} onclick={() => act('logs')}>Logs</button>
          <button type="button" class="pill pill-sm" disabled={busy !== ''} onclick={() => act('restart')}>Restart</button>
          <button type="button" class="pill pill-sm" disabled={busy !== ''} onclick={() => act('stop')}>Stop</button>
          <button type="button" class="pill pill-sm" disabled={busy !== ''} onclick={() => act('enable')}>Start and enable</button>
          <button type="button" class="pill pill-sm" disabled={busy !== ''} onclick={() => act('disable')}>Stop and disable</button>
        </div>
        <div class="btnrow" role="group" aria-label="The node">
          <button type="button" class="pill pill-sm" disabled={busy !== ''} onclick={() => update()}>Update (reinstall)</button>
          <button type="button" class="pill pill-sm" disabled={busy !== ''} onclick={loadNotice}>Edit the notice</button>
        </div>
        <p class="help">Update reinstalls with a hoi-node from this computer when one fits the server{nodeFiles.length ? '' : ' (none found beside the app)'}, else with the one from the signed release list.</p>
        {#if nodeLink}
          <div class="linkrow"><textarea class="textarea mono-in" rows="3" readonly value={nodeLink} aria-label="The node's link"></textarea><div class="btnrow"><button type="button" class="pill pill-sm" onclick={() => copy(nodeLink)}>Copy</button><button type="button" class="pill pill-sm pill-ink" onclick={openInHoi}>Open in hoi</button></div></div>
          {#if noticeHash}<p class="help mono">Notice hash: {noticeHash}</p>{/if}
        {/if}
        {#if logsText}
          <!-- svelte-ignore a11y_no_noninteractive_tabindex (Scrollable logs need keyboard focus.) -->
          <pre tabindex="0" role="region" aria-label="Node logs" class="log small">{logsText}</pre>
        {/if}
        {#if manageNotice}
          <h2>The notice</h2>
          <NoticeEditor bind:notice={manageNotice} bind:retentionText={manageRetention} problem={() => undefined} idPrefix="mn" />
          <div class="btnrow"><button type="button" class="pill pill-ink" disabled={busy !== ''} onclick={saveNotice}>Save on the server</button><button type="button" class="pill" onclick={() => (manageNotice = null)}>Cancel</button></div>
        {/if}
        <h2>Mailboxes</h2>
        {#if piecesIn(current.config)}
          <p class="help">This node keeps up to {piecesIn(current.config)} GB of sealed pieces for private communities' mailboxes. It can read none of them; the totals are all there is to see.</p>
          <div class="btnrow"><button type="button" class="pill pill-sm" disabled={busy !== ''} onclick={() => act('pieces')}>How much is kept</button>{#if piecesUse}<span class="status">{piecesUse}</span>{/if}</div>
        {:else}
          <p class="help">Off on this node. Enable mailboxes to let private communities keep sealed pieces here.</p>
        {/if}
        <p class="help">Applying this setting restarts the node using its installed program. Disabling stops serving pieces; existing files stay on disk. Other members may need to repair their copies.</p>
        <div class="formrow">
          {#if piecesIn(current.config)}
            <button type="button" class="pill" disabled={busy !== ''} onclick={() => setMailboxes(false)}>Disable mailboxes</button>
          {:else}
            <label class="field"><span>Mailbox space (GB)</span><input class="input days" bind:value={mailboxLimit} inputmode="decimal" /></label>
            <button type="button" class="pill" disabled={busy !== ''} onclick={() => setMailboxes(true)}>Enable mailboxes</button>
          {/if}
        </div>
        <h2>Logs on the server</h2>
        <p class="help">hoi-node logs no addresses; the server's own logs (sshd, the firewall) carry them. Keep them no longer than your notice says.</p>
        <div class="formrow"><input class="input days" bind:value={retentionDays} inputmode="numeric" aria-label="Days to keep logs" /><button type="button" class="pill" disabled={busy !== ''} onclick={setRetention}>Keep logs this many days</button></div>
        <div class="btnrow"><button type="button" class="pill pill-sm" disabled={busy !== ''} onclick={() => act('deleteLogs')}>Delete the node's logs now</button></div>
        <h2>Remove</h2>
        <div class="formrow"><input class="input erase" bind:value={eraseText} placeholder={`type ${current.name} to erase its data too`} aria-label="Confirm erasing data" spellcheck="false" /><button type="button" class="pill pill-danger" disabled={busy !== ''} onclick={uninstall}>Uninstall{eraseText === current.name ? ' and erase the data' : ' (keep the data)'}</button></div>
      {/if}
      {#if error}<p class="callout err" role="alert">{error}</p>{/if}
      {#if log.length}
        <details class="logbox" open>
          <summary>What was run ({log.length} lines)</summary>
          <!-- svelte-ignore a11y_no_noninteractive_tabindex (Scrollable logs need keyboard focus.) -->
          <pre tabindex="0" role="region" aria-label="Command log" class="log" bind:this={logEl}>{#each log as l, i (i)}<span class={l.stream}>{l.text}</span>{'\n'}{/each}</pre>
        </details>
      {/if}
      <div class="btnrow"><button type="button" class="pill" onclick={() => { view = 'servers'; session = null; }}>Back to your servers</button></div>
    </section>
  {/if}
</main>

<style>
  /* The website's look (apps/site/src/styles/site.css): cream page, an ink rule under the bar, a
     yellow sticker over a two-tone headline, yellow numbered badges, ledger rows, ink frames. */
  :global(body) { background: var(--bg); }
  .bar { position: sticky; top: 0; z-index: 5; background: var(--bg); border-bottom: 2px solid var(--ink); }
  .bar-in { max-width: 980px; margin: 0 auto; min-height: 68px; display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 10px 16px; padding-block: 10px; }
  .logo { display: inline-flex; align-items: center; gap: 10px; font-weight: 900; font-size: 21px; letter-spacing: -.02em; }
  .logo .sh { color: var(--mute); font-weight: 600; }
  .setup { max-width: 980px; margin: 0 auto; padding-block: 24px 72px; display: flex; flex-direction: column; gap: 20px; }

  .steps { list-style: none; margin: 0; padding: 0; display: flex; align-items: center; flex-wrap: wrap; gap: 8px 6px; }
  .steps li { display: inline-flex; align-items: center; gap: 8px; font-weight: 800; font-size: 14px; color: var(--mute); }
  .steps li + li::before { content: ''; width: 14px; border-top: 2px dotted var(--line); margin-right: 2px; }
  .num { flex: 0 0 auto; width: 32px; height: 32px; border-radius: 11px; border: 2px solid var(--line); display: inline-flex; align-items: center; justify-content: center; font-weight: 800; font-size: 14px; background: var(--card); color: var(--mute); transition: transform .2s ease; }
  .steps .done .num { border-color: var(--ink); color: var(--ink); }
  .steps .now .num { width: 40px; height: 40px; border-radius: 13px; border-color: var(--ink); background: var(--sun); color: #1A1A1A; transform: rotate(-4deg); font-size: 16px; }
  .steps .lbl { display: none; }
  .steps .now .lbl { display: inline; color: var(--ink); }

  .hero { display: flex; flex-direction: column; gap: 16px; padding-block: 8px 4px; }
  .eyebrow { display: inline-flex; align-items: center; gap: 8px; padding: 5px 13px; border-radius: 999px; border: 2px solid var(--ink); font-weight: 800; font-size: 13px; background: var(--sun); color: #1A1A1A; transform: rotate(-2deg); transform-origin: left center; align-self: flex-start; }
  .mono-eyebrow { font-family: var(--mono); font-weight: 700; }
  .hero h1 { margin: 0; font-size: clamp(34px, 6vw, 58px); line-height: .98; letter-spacing: -.035em; font-weight: 900; text-wrap: balance; max-width: 15em; }
  .blue { color: var(--cobalt); }
  .lead { font-size: 17px; line-height: 1.55; color: var(--mute); max-width: 44em; }

  .card { padding: 24px 26px; display: flex; flex-direction: column; gap: 16px; border-radius: 24px; }
  .card h2 { font-size: 22px; letter-spacing: -.03em; margin-top: 10px; }
  .card p, .card li { line-height: 1.55; color: var(--mute); font-weight: 500; }
  .card p b { color: var(--ink); }
  .card .mono { overflow-wrap: anywhere; }

  .ledger { border-top: 2px solid var(--ink); border-bottom: 2px dotted var(--line); }
  .lrow { display: grid; grid-template-columns: minmax(0, 230px) minmax(0, 1fr); gap: 8px 32px; padding: 20px 0; border-top: 2px dotted var(--line); }
  .lv p, .lv li { color: var(--mute); line-height: 1.55; font-weight: 500; }
  .lrow:first-child { border-top: 0; }
  .lk { font-weight: 800; font-size: 17px; letter-spacing: -.015em; line-height: 1.3; color: var(--ink); }
  .lv { display: flex; flex-direction: column; gap: 8px; font-size: 15px; }
  .lv ul { margin: 0; padding-left: 1.2em; display: flex; flex-direction: column; gap: 4px; }

  .ack { display: flex; gap: 12px; align-items: flex-start; font-weight: 800; padding: 14px 16px; border: 2px solid var(--ink); border-radius: 16px; background: var(--soft); cursor: pointer; }
  .ack:has(input:checked) { background: var(--mint); color: #1A1A1A; }
  .ack input, .check input { width: 20px; height: 20px; accent-color: var(--ink); flex: none; margin-top: 2px; }
  /* the box beside its words, which wrap beside it rather than drop below it */
  .check { display: flex; flex-direction: row; gap: 10px; align-items: flex-start; font-weight: 700; line-height: 1.4; }
  .check > span { min-width: 0; padding-top: 1px; font-size: inherit; font-weight: inherit; }
  /* in a two-column grid, level with the input of the field beside it (under that field's caption) */
  .grid2 > .check { align-self: start; margin-top: 26px; min-height: 44px; align-items: center; }
  .grid2 > .check.wide { grid-column: 1 / -1; margin-top: 0; min-height: 0; }
  .check.big { font-size: 17px; font-weight: 800; color: var(--ink); flex-wrap: nowrap; align-items: flex-start; line-height: 1.35; }
  .check.big .tag { vertical-align: 2px; margin-left: 4px; }
  .choice { display: flex; flex-wrap: wrap; gap: 8px; }
  .choice label { position: relative; display: inline-flex; align-items: center; gap: 8px; min-height: 44px; padding: 0 18px; border: 2px solid var(--ink); border-radius: 999px; cursor: pointer; font-weight: 700; font-size: 15px; background: var(--card); transition: transform .15s ease; }
  .choice label:hover { transform: translateY(-1px); }
  .choice label:has(input:checked) { background: var(--ink); color: var(--onink); }
  .choice label:has(input:focus-visible) { outline: 3px solid var(--cobalt); outline-offset: 3px; }
  .choice input { position: absolute; opacity: 0; width: 0; height: 0; }
  .choice.small label { min-height: 40px; padding: 0 14px; font-size: 13px; }
  .grid2 { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 14px 18px; }
  .inrow { display: flex; gap: 8px; align-items: center; min-width: 0; }
  .inrow .input { flex: 1 1 120px; min-width: 0; }
  .inrow .pill { min-height: 44px; }
  .formrow { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  .formrow .input { min-height: 42px; padding-block: 8px; }
  .formrow .days { flex: 0 0 90px; width: 90px; }
  .formrow .erase { flex: 1 1 260px; }
  .pre { white-space: nowrap; }
  .capin { max-width: 140px; }
  .field.inline { display: flex; flex-direction: row; align-items: center; gap: 10px; }
  .field.inline .capin { max-width: 90px; }
  .bad { color: var(--ink); font-weight: 700; box-shadow: inset 3px 0 0 var(--coral); padding-left: 10px; }
  .keybox { padding: 14px 16px; border-radius: 12px; background: var(--soft); overflow-wrap: anywhere; color: var(--ink); line-height: 1.6; font-size: 13px; }

  .mailboxes { display: flex; flex-direction: column; gap: 10px; padding: 18px 20px; border: 2px dashed var(--line); border-radius: 20px; }
  .mailboxes.on { border: 2px solid var(--ink); background: color-mix(in srgb, var(--mint) 22%, var(--card)); }
  .mailboxes p { font-size: 15px; }
  .program { display: flex; flex-direction: column; gap: 10px; padding: 18px 20px; border: 2px dashed var(--line); border-radius: 20px; }
  .program.on { border: 2px solid var(--ink); background: color-mix(in srgb, var(--sky) 30%, var(--card)); }
  .program-h { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; font-size: 17px; font-weight: 800; }
  .program .file { overflow-wrap: anywhere; }

  .more summary, .logbox summary { cursor: pointer; color: var(--cobalt); font-weight: 700; min-height: 40px; display: flex; align-items: center; }
  .preview { display: flex; flex-direction: column; gap: 10px; }
  .kit-h { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }

  .checks { border-collapse: collapse; width: 100%; font-size: 14px; border-bottom: 2px dotted var(--line); }
  .checks td { padding: 10px 10px 10px 0; border-top: 2px dotted var(--line); vertical-align: top; line-height: 1.45; color: var(--mute); overflow-wrap: anywhere; }
  .checks tr:first-child td { border-top: 0; }
  .checks .lvl { width: 74px; }
  .checks .lvl .tag { font-family: var(--mono); font-size: 12px; background: var(--soft); color: var(--ink); }
  .checks .what { font-weight: 800; width: 100px; color: var(--ink); }
  .checks tr.ok .tag { background: var(--mint); color: #1A1A1A; }
  .checks tr.info .tag { background: var(--sky); color: #1A1A1A; }
  .checks tr.warn .tag { background: var(--sun); color: #1A1A1A; }
  .checks tr.BLOCK .tag { background: var(--coral); color: #1A1A1A; }

  .callout.err { border: 2px solid var(--coral); color: var(--ink); background: color-mix(in srgb, var(--peach) 45%, var(--card)); }
  .log { max-height: 360px; overflow: auto; font-family: var(--mono); font-size: 12.5px; line-height: 1.55; padding: 14px 16px; border: 2px solid var(--ink); border-radius: 16px; background: var(--card); white-space: pre-wrap; overflow-wrap: anywhere; margin: 0; }
  .log.small { max-height: 240px; }
  .log .cmd { color: var(--cobalt); font-weight: 700; }
  .log .err { color: var(--coral); }

  .done { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 24px; align-items: start; }
  .done-text { display: flex; flex-direction: column; gap: 16px; }
  .note-sun { background: var(--sun); color: #1A1A1A !important; border: 2px solid var(--ink); border-radius: 18px; padding: 14px 16px; transform: rotate(-1.2deg); font-weight: 600 !important; }
  .linkrow { display: flex; flex-direction: column; gap: 8px; }
  .linkrow .textarea { width: 100%; font-size: 12.5px; }

  .servers { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .servers li { display: flex; justify-content: space-between; align-items: center; gap: 12px; flex-wrap: wrap; padding: 14px 0; border-top: 2px dotted var(--line); }
  .servers li:first-child { border-top: 0; }
  .srv { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .srv b { font-size: 18px; letter-spacing: -.02em; color: var(--ink); }
  .tags { display: flex; flex-wrap: wrap; gap: 6px; }
  .status { color: var(--ink); font-weight: 600; display: inline-flex; align-items: center; gap: 8px; }
  .dot { width: 10px; height: 10px; border-radius: 50%; background: var(--mute); flex: 0 0 10px; }
  .dot.on { background: var(--mint); box-shadow: 0 0 0 2px var(--ink); }

  @media (max-width: 700px) {
    .grid2, .done { grid-template-columns: minmax(0, 1fr); }
    .grid2 > .check { margin-top: 0; min-height: 0; }
    .lrow { grid-template-columns: minmax(0, 1fr); gap: 6px; }
    .card { padding: 18px 16px; }
    /* every step on one line: the sticker under it names the step */
    .steps { gap: 8px; flex-wrap: nowrap; }
    .steps li + li::before { display: none; }
    .steps .now .lbl { display: none; }
    /* a check's level and subject on one line, what it found under them */
    .checks tr { display: grid; grid-template-columns: auto minmax(0, 1fr); column-gap: 10px; padding: 10px 0; border-top: 2px dotted var(--line); }
    .checks tr:first-child { border-top: 0; }
    .checks td { border-top: 0; padding: 0; }
    .checks .lvl, .checks .what { width: auto; }
    .checks td:last-child { grid-column: 1 / -1; padding-top: 4px; }
  }
  @media (prefers-reduced-motion: reduce) { .num, .choice label { transition: none; } }
</style>
