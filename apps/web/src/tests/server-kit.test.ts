import { afterAll, describe, expect, it } from 'vitest';
import { noticeJson } from '../lib/notice';
import { PUBLICATION } from '../lib/preview';
import {
  ADJECTIVES,
  NOUNS,
  blankKit,
  cloudInit,
  configText,
  hostOf,
  installFlags,
  installerUrlFor,
  kitArchive,
  kitFiles,
  kitNotice,
  kitProblems,
  noticeFileName,
  oneLiner,
  randomBelow,
  randomName,
  readme,
  registryUrlOf,
  shellQuote,
  tar,
  type KitOptions,
} from '../lib/server-kit';
import { cleanup, configWith, copyRelease, installerPath, installerWords, isExecutable, linux, read, readInstaller, release, run, unix, unpack, writeTemp } from './fixtures/install-sh.mjs';

afterAll(() => cleanup());

const goodNotice = { ...blankKit('x').notice, operator: 'Lena', contact: 'lena at example dot org', country: 'XX', provider: 'Example Hosting', updated: '2026-11-03' };
const kit = (patch: Partial<KitOptions> = {}): KitOptions => ({ ...blankKit('sleepy-walrus-48213'), domain: 'example.org', notice: goodNotice, ...patch });
/** The ExecStart line of the systemd unit a dry run prints. */
const execStart = (out: string) => /^ExecStart=.*$/m.exec(out)?.[0] ?? '';

describe('names', () => {
  it('rolls from the same words as install.sh, every word as likely', () => {
    expect(ADJECTIVES).toEqual(installerWords('ADJECTIVES'));
    expect(NOUNS).toEqual(installerWords('NOUNS'));
    expect(ADJECTIVES).toHaveLength(64);
    expect(NOUNS).toHaveLength(64);
    for (let i = 0; i < 50; i++) expect(randomName()).toMatch(/^[a-z]+-[a-z]+-[1-9][0-9]{4}$/);
  });

  it('throws away the numbers that would favour some words', () => {
    // 2^32 is not a multiple of 90000: the top of the range is rolled again, not folded over.
    const words = [2 ** 32 - 1, 2 ** 32 - 2, 7];
    const random = (out: Uint32Array<ArrayBuffer>) => { out[0] = words.shift()!; };
    expect(randomBelow(90000, random)).toBe(7);
    expect(words).toHaveLength(0);
  });

  it('makes host names the way install.sh does', { skip: !unix }, () => {
    for (const name of ["Lena's Pi 2", 'sleepy-walrus-48213', '--Big  Node--', 'node.example']) {
      const made = configWith(['public', '--name', name]);
      expect(made.status, made.out).toBe(0);
      expect(made.text).toContain(`hostname=${hostOf(name)}\n`);
    }
  });
});

describe('what install.sh would refuse', () => {
  it('needs an address for a registry, and a domain or https:// address for it', () => {
    expect(kitProblems(kit())).toEqual([]);
    expect(kitProblems(kit({ role: 'public', domain: '' }))).toEqual([]);
    expect(kitProblems(kit({ domain: '' })).map((p) => p.field)).toEqual(['domain']);
    expect(kitProblems(kit({ domain: 'not a domain' })).map((p) => p.field)).toEqual(['domain']);
    expect(kitProblems(kit({ registryUrl: 'http://names.example.org' })).map((p) => p.field)).toEqual(['registryUrl']);
    expect(kitProblems(kit({ registryUrl: 'https://names.example.org' }))).toEqual([]);
  });

  it('keeps Caddy to a host name of its own', () => {
    expect(kitProblems(kit({ registryUrl: 'https://203.0.113.10' }))[0]?.message).toMatch(/domain name/);
    expect(kitProblems(kit({ registryUrl: 'https://example.org/registry' }))[0]?.message).toMatch(/root of its own host name/);
    expect(kitProblems(kit({ registryUrl: 'https://example.org/registry', tls: false }))).toEqual([]);
  });

  it('checks names, registries to announce to and the cap', () => {
    expect(kitProblems(kit({ name: '' }))[0]?.field).toBe('name');
    expect(kitProblems(kit({ name: '---' }))[0]?.field).toBe('name');
    expect(kitProblems(kit({ indexes: ['https://names.example.net', 'ftp://x'] })).map((p) => p.field)).toEqual(['indexes']);
    expect(kitProblems(kit({ dailyCapGb: 'lots' })).map((p) => p.field)).toEqual(['dailyCapGb']);
    expect(kitProblems(kit({ dailyCapGb: '2.5' }))).toEqual([]);
  });
});

describe('the configuration', () => {
  it('says what install.sh config says, line for line', { skip: !unix }, () => {
    const noticePath = writeTemp('hoi-sleepy-walrus-48213.notice.json', noticeJson(kitNotice(kit())));
    const made = configWith(['registry', '--name', 'sleepy-walrus-48213', '--domain', 'example.org', '--tls', 'caddy', '--index', 'https://names.example.net', '--daily-cap-gb', '3', '--hold-pieces-gb', '2', '--notice-file', noticePath]);
    expect(made.status, made.out).toBe(0);
    const settings = (text: string) => text.split('\n').filter((l) => l && !l.startsWith('#'));
    const ours = configText(kit({ tls: true, indexes: ['https://names.example.net'], dailyCapGb: '3' }));
    expect(settings(ours)).toEqual(settings(made.text));
    expect(settings(ours)).toContain('registry_url=https://sleepy-walrus-48213.example.org');
    expect(settings(ours)).toContain('tls=caddy');
    expect(settings(ours)).toContain('hold_pieces_gb=2');
  });

  it('keeps mailboxes for private communities unless told not to, and the notice says so', () => {
    expect(configText(kit())).toMatch(/^hold_pieces_gb=2$/m);
    expect(kitNotice(kit()).pieces).toBe(true);
    for (const off of ['', '0']) {
      expect(configText(kit({ holdPiecesGb: off }))).not.toMatch(/hold_pieces_gb/);
      expect(kitNotice(kit({ holdPiecesGb: off })).pieces).toBeUndefined();
      expect(installFlags(kit({ holdPiecesGb: off }))).not.toContain('--hold-pieces-gb');
    }
    // a notice that claimed it while mailboxes are off does not say so
    expect(kitNotice(kit({ holdPiecesGb: '', notice: { ...goodNotice, pieces: true } })).pieces).toBeUndefined();
    expect(installFlags(kit({ holdPiecesGb: '5' })).slice(-2)).toEqual(['--hold-pieces-gb', '5']);
    expect(kitProblems(kit({ holdPiecesGb: 'lots' })).map((p) => p.field)).toEqual(['holdPiecesGb']);
    expect(kitProblems(kit({ holdPiecesGb: '0.01' })).map((p) => p.field)).toEqual(['holdPiecesGb']);
    expect(kitProblems(kit({ holdPiecesGb: '0' }))).toEqual([]);
    expect(readme(kit(), 'https://hoi.sh/install.sh', [])).toContain('- mailboxes: keeps up to 2 GB');
  });

  it('leaves out the host name, Caddy and the registry when they are not wanted', () => {
    const text = configText(kit({ role: 'public', keepHostname: true }));
    expect(text).toContain('role=public\nname=sleepy-walrus-48213\n');
    expect(text).not.toMatch(/^(hostname|registry_url|tls)=/m);
    expect(registryUrlOf(kit({ registryUrl: 'https://names.example.org' }))).toBe('https://names.example.org');
  });

  it('carries nothing of the device that made it', () => {
    const files = kitFiles(kit(), '#!/bin/sh\n', 'https://hoi.sh/install.sh');
    for (const f of files) expect(f.text).not.toMatch(/did:key|hoi-key-|pairing|w=|rtc:/);
  });
});

describe('the commands', () => {
  it('quotes what a shell would split', () => {
    expect(shellQuote('sleepy-walrus-48213')).toBe('sleepy-walrus-48213');
    expect(shellQuote("lena's pi")).toBe(`'lena'\\''s pi'`);
    expect(oneLiner(kit(), 'https://hoi.sh/install.sh', true)).toBe(
      'curl -fsSL https://hoi.sh/install.sh | bash -s -- check registry --name sleepy-walrus-48213 --hostname sleepy-walrus-48213 --registry-url https://sleepy-walrus-48213.example.org --tls caddy --hold-pieces-gb 2',
    );
  });

  it('finds the installer beside the app that served it, over https only', () => {
    // A build for a configured site always names that site's installer, wherever it is served.
    if (PUBLICATION.site) {
      const site = `${PUBLICATION.site.replace(/\/+$/, '')}/install.sh`;
      for (const href of ['https://hoi.sh/app/', 'https://someone.github.io/hoi/app/index.html', 'http://127.0.0.1:8787/app/', undefined]) expect(installerUrlFor(href)).toBe(site);
      return;
    }
    expect(installerUrlFor('https://hoi.sh/app/')).toBe('https://hoi.sh/install.sh');
    expect(installerUrlFor('https://someone.github.io/hoi/app/index.html')).toBe('https://someone.github.io/hoi/install.sh');
    expect(installerUrlFor('http://127.0.0.1:8787/app/')).toBe('https://hoi.sh/install.sh');
    expect(installerUrlFor('tauri://localhost/')).toBe('https://hoi.sh/install.sh');
    expect(installerUrlFor(undefined)).toBe('https://hoi.sh/install.sh');
  });

  it('writes cloud-init that fits any provider and carries the configuration', () => {
    const text = cloudInit(kit(), 'https://hoi.sh/install.sh');
    expect(text.startsWith('#cloud-config\n')).toBe(true);
    expect(new TextEncoder().encode(text).length).toBeLessThan(16 * 1024);
    expect(text).toMatch(/^hostname: sleepy-walrus-48213$/m);
    expect(text).toMatch(/^ {6}role=registry$/m);
    expect(text).toMatch(/^ {6}tls=caddy$/m);
    expect(text).toMatch(/^ {2}- bash \/root\/hoi-install\.sh --config \/root\/hoi-sleepy-walrus-48213\.conf --yes$/m);
    // YAML: no tabs, and every line of the block is indented under `content: |`.
    expect(text).not.toContain('\t');
    expect(cloudInit(kit({ keepHostname: true }), 'https://hoi.sh/install.sh')).not.toMatch(/^hostname:/m);
  });
});

describe('the kit', () => {
  it('is a tar archive the system tar unpacks, with the installer executable', { skip: !unix }, async () => {
    const installer = readInstaller();
    const { name, bytes } = await kitArchive(kit(), installer, 'https://hoi.sh/install.sh', 1_700_000_000);
    expect(name).toBe('hoi-sleepy-walrus-48213.tar.gz');
    expect([bytes[0], bytes[1]]).toEqual([0x1f, 0x8b]);
    const { dir, list, listing } = unpack(name, bytes);
    expect(list.sort()).toEqual([
      'hoi-sleepy-walrus-48213/',
      'hoi-sleepy-walrus-48213/README.md',
      'hoi-sleepy-walrus-48213/cloud-init.yaml',
      'hoi-sleepy-walrus-48213/hoi-sleepy-walrus-48213.conf',
      'hoi-sleepy-walrus-48213/hoi-sleepy-walrus-48213.notice.json',
      'hoi-sleepy-walrus-48213/install.sh',
    ]);
    expect(listing).toMatch(/2023-11-1[45]/);
    const folder = `${dir}/hoi-sleepy-walrus-48213`;
    expect(read(`${folder}/install.sh`)).toBe(installer);
    expect(isExecutable(`${folder}/install.sh`)).toBe(true);
    expect(isExecutable(`${folder}/hoi-sleepy-walrus-48213.conf`)).toBe(false);
    expect(read(`${folder}/README.md`)).toContain('./install.sh check --config hoi-sleepy-walrus-48213.conf');
    // A registry behind your own proxy gets a Caddyfile to start from.
    const own = unpack('own.tar', tar('own', kitFiles(kit({ tls: false }), installer, 'https://hoi.sh/install.sh')));
    expect(read(`${own.dir}/own/Caddyfile`)).toMatch(/^sleepy-walrus-48213\.example\.org \{\n\treverse_proxy 127\.0\.0\.1:7845\n\}$/m);
  });

  it('has LF line ends, even from an app built on a Windows checkout (CRLF)', { skip: !unix }, async () => {
    const crlf = readInstaller().replace(/\n/g, '\r\n');
    const files = kitFiles(kit(), crlf, 'https://hoi.sh/install.sh');
    for (const f of files) expect(f.text, f.name).not.toContain('\r');
    expect(files.find((f) => f.name === 'install.sh')?.text).toBe(readInstaller());
    // What the setup app runs on the server; with the CRs left in, bash stops at `set -euo pipefail`.
    const { name, bytes } = await kitArchive(kit(), crlf, 'https://hoi.sh/install.sh');
    const folder = `${unpack(name, bytes).dir}/hoi-sleepy-walrus-48213`;
    const r = run(`${folder}/install.sh`, ['check', '--config', `${folder}/hoi-sleepy-walrus-48213.conf`, '--yes'], { dry: false });
    expect(r.out).not.toMatch(/invalid option name|\$'\\r'|\r/);
    expect(r.out).toMatch(/Checking this computer/);
  });

  it('installs what it describes, from the file and from the one-line command alike', { skip: !unix, timeout: 30_000 }, async () => {
    const files = release();
    const o = kit({ indexes: ['https://names.example.net'] });
    const { name, bytes } = await kitArchive(o, readInstaller(), 'https://hoi.sh/install.sh');
    const folder = `${unpack(name, bytes).dir}/hoi-sleepy-walrus-48213`;

    const fromFile = run(`${folder}/install.sh`, ['--config', `${folder}/hoi-sleepy-walrus-48213.conf`, '--from', files]);
    expect(fromFile.status, fromFile.out).toBe(0);
    expect(fromFile.out).toMatch(/Checking this computer for a hoi public node and registry called sleepy-walrus-48213/);
    for (const flag of ['"--public"', '"--registry-url" "https://sleepy-walrus-48213.example.org"', '"--index" "https://names.example.net"']) {
      if (linux) expect(execStart(fromFile.out)).toContain(flag);
    }
    if (linux) {
      expect(fromFile.out).toContain('+ write /etc/caddy/hoi-registry.caddy:\n# Written by deploy/install/install.sh (registry)');
      expect(fromFile.out).toMatch(/hostnamectl set-hostname sleepy-walrus-48213\n/);
    }

    // The flags alone cannot carry the notice: it travels as a file, from the kit.
    const fromFlags = run(installerPath, [...installFlags(o), '--notice-file', `${folder}/hoi-sleepy-walrus-48213.notice.json`, '--from', files]);
    expect(fromFlags.status, fromFlags.out).toBe(0);
    expect(execStart(fromFlags.out)).toBe(execStart(fromFile.out));

    // With the release files put beside the installer, nothing has to be fetched or named.
    copyRelease(files, folder);
    const offline = run(`${folder}/install.sh`, ['check', '--config', `${folder}/hoi-sleepy-walrus-48213.conf`], { dry: false });
    expect(offline.out).toMatch(/installing from the release files beside the installer/);
    expect(offline.out).toMatch(/ok {5}source {6}hoi 9\.9\.9, hoi-node_9\.9\.9_/);
  });
});

describe('the join notice in the kit', () => {
  it('is always written, checked the way the node checks it, and named after the kit; who and where may be left out', () => {
    const blank = { ...kit(), notice: blankKit('sleepy-walrus-48213').notice };
    expect(kitProblems(blank)).toEqual([]);
    expect(kitFiles(blank, '#!/bin/sh\n', 'https://hoi.sh/install.sh')[0].text).toMatch(/operator: an operator who gave no name · no contact given/);
    expect(kitProblems({ ...blank, notice: { ...blank.notice, country: 'xx' } }).map((p) => p.field)).toEqual(['notice.country']);
    expect(kitProblems(kit())).toEqual([]);
    expect(kitProblems(kit({ notice: { ...goodNotice, ipLogging: true, retentionDays: 0 } }))[0]).toEqual({ field: 'notice.retentionDays', message: 'Say for how many days IP addresses are kept.' });
    expect(noticeFileName(kit())).toBe('hoi-sleepy-walrus-48213.notice.json');
    expect(kitNotice(kit({ name: 'Sleepy Walrus' })).node).toBe('Sleepy Walrus');
  });

  it('travels in the configuration, the kit and the cloud-init', () => {
    const o = kit();
    expect(configText(o)).toMatch(/^notice_file=hoi-sleepy-walrus-48213\.notice\.json$/m);
    const files = kitFiles(o, '#!/bin/sh\n', 'https://hoi.sh/install.sh');
    const file = files.find((f) => f.name === 'hoi-sleepy-walrus-48213.notice.json');
    expect(file?.mode).toBe(0o644);
    const json = JSON.parse(file!.text);
    expect(json).toMatchObject({ v: 1, node: 'sleepy-walrus-48213', operator: 'Lena', country: 'XX', provider: 'Example Hosting', ipLogging: false, retentionDays: 0, updated: '2026-11-03' });
    expect('extra' in json).toBe(false);
    expect(files[0].text).toMatch(/operator: Lena · contact lena at example dot org · server in XX/);
    expect(files[0].text).toMatch(/notice-hash/);
    const init = cloudInit(o, 'https://hoi.sh/install.sh');
    expect(init).toMatch(/- path: \/root\/hoi-sleepy-walrus-48213\.notice\.json/);
    expect(init).toMatch(/"operator": "Lena"/);
    expect(init.length).toBeLessThan(4000);
    expect(oneLiner(o, 'https://hoi.sh/install.sh')).not.toMatch(/notice/);
  });
});

