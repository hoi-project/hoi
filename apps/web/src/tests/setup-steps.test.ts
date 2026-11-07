import { describe, expect, it } from 'vitest';
import { MANAGE, asRoot, installCommand, localNodeLinkCommand, megabytes, nodeFileFor, nodeLinkIn, nodeTarget, parsePreflight, piecesIn, piecesSummary, profileId, remoteDirIn, retentionDropIn } from '../setup/steps';

describe('the setup wizard reads what the installer prints', () => {
  it('parses the check rows the way install.sh prints them, and the verdict', () => {
    const text = [
      'hoi-node check on pi.example',
      '  ok     system     Debian 12, aarch64',
      '  info   ports      7843 and 7844 are free',
      '  warn   notice     no join notice: people will be told the operator gave none',
      '  BLOCK  systemd    systemctl is not here',
      '',
      'Not ready: 1 thing to fix first.',
    ].join('\n');
    const result = parsePreflight(text);
    expect(result.rows.map((r) => [r.level, r.what])).toEqual([['ok', 'system'], ['info', 'ports'], ['warn', 'notice'], ['BLOCK', 'systemd']]);
    expect(result.rows[2].text).toBe('no join notice: people will be told the operator gave none');
    expect(result).toMatchObject({ ready: false, blocks: 1, warns: 1 });
    expect(parsePreflight('  ok   system  fine\nReady to install.\n')).toMatchObject({ ready: true, blocks: 0, warns: 0 });
    expect(parsePreflight('bash: install.sh: No such file').ready).toBeNull();
  });

  it('finds the node link the node printed, preferring the web link', () => {
    const uri = 'hoi://node?w=abc@203.0.113.7:7844&p=def@203.0.113.7:7843';
    const web = 'https://hoi.sh/app/#/node?w=abc@203.0.113.7:7844&p=def@203.0.113.7:7843';
    expect(nodeLinkIn(`node link:\n${uri}\n${web}\n`)).toBe(web);
    expect(nodeLinkIn(`share ${uri} with people`)).toBe(uri);
    expect(nodeLinkIn('nothing yet')).toBeNull();
    // what hoi-node prints now: the addresses packed into a code, and what it holds below it
    const coded = 'https://hoi.sh/app/#/node?code=SE9OAV1m6-1etSPtkXN7LS0qm2OTNFkZ_x';
    const printed = `node link (anyone who opens it finds…):\n\n  ${coded}\n\n  it holds 1 WebRTC route and 1 native route, and the operator's notice, signed; the addresses are packed into the code\n`;
    expect(nodeLinkIn(printed)).toBe(coded);
    expect(nodeLinkIn('hoi://node?code=SE9OAV1m6\n')).toBe('hoi://node?code=SE9OAV1m6');
  });

  it('accepts only the folder mktemp made', () => {
    expect(remoteDirIn('/tmp/hoi-setup.Ab12Cd34\n')).toBe('/tmp/hoi-setup.Ab12Cd34');
    expect(remoteDirIn('mktemp: failed')).toBeNull();
    expect(remoteDirIn('/tmp/hoi-setup.x; rm -rf /')).toBeNull();
  });
});

describe('the commands the wizard runs are quoted and shown as they run', () => {
  it('installs from the uploaded folder, checking first when asked', () => {
    expect(installCommand('/tmp/hoi-setup.Ab12', 'hoi-quiet-fox.conf', true)).toBe('cd /tmp/hoi-setup.Ab12 && bash ./install.sh check --config hoi-quiet-fox.conf --yes');
    expect(installCommand('/tmp/hoi-setup.Ab12', 'hoi-quiet-fox.conf', false)).toBe('cd /tmp/hoi-setup.Ab12 && bash ./install.sh --config hoi-quiet-fox.conf --yes');
    expect(installCommand("/tmp/it's here", 'a b.conf', false)).toBe(`cd '/tmp/it'\\''s here' && bash ./install.sh --config 'a b.conf' --yes`);
  });

  it('installs the hoi-node uploaded beside the kit when there is one', () => {
    expect(installCommand('/tmp/hoi-setup.Ab12', 'hoi-quiet-fox.conf', true, './hoi-node_1.0.0_x86_64-unknown-linux-gnu.tar.gz'))
      .toBe('cd /tmp/hoi-setup.Ab12 && bash ./install.sh check --config hoi-quiet-fox.conf --yes --binary ./hoi-node_1.0.0_x86_64-unknown-linux-gnu.tar.gz');
    expect(installCommand('/tmp/hoi-setup.Ab12', 'c.conf', false, "./it's.tar.gz")).toBe(`cd /tmp/hoi-setup.Ab12 && bash ./install.sh --config c.conf --yes --binary './it'\\''s.tar.gz'`);
    expect(installCommand('/tmp/hoi-setup.Ab12', 'c.conf', false, '')).toBe('cd /tmp/hoi-setup.Ab12 && bash ./install.sh --config c.conf --yes');
  });

  it('picks the hoi-node built for the server\'s processor', () => {
    expect(nodeTarget('linux', 'x86_64\n')).toBe('x86_64-unknown-linux-gnu');
    expect(nodeTarget('linux', 'aarch64')).toBe('aarch64-unknown-linux-gnu');
    expect(nodeTarget('macos', 'aarch64')).toBe('aarch64-apple-darwin');
    expect(nodeTarget('windows', 'x86_64')).toBe('x86_64-pc-windows-msvc');
    expect(nodeTarget('windows', 'aarch64')).toBeNull();
    expect(nodeTarget('linux', 'armv7l')).toBeNull();
    expect(nodeTarget('linux', '')).toBeNull();
    const files = [{ target: 'aarch64-unknown-linux-gnu', name: 'arm' }, { target: 'x86_64-unknown-linux-gnu', name: 'near' }, { target: 'x86_64-unknown-linux-gnu', name: 'far' }];
    expect(nodeFileFor(files, 'x86_64-unknown-linux-gnu')?.name).toBe('near');
    expect(nodeFileFor(files, 'aarch64-apple-darwin')).toBeNull();
    expect(nodeFileFor(files, null)).toBeNull();
    expect(megabytes(13_421_772)).toBe('12.8 MB');
    expect(megabytes(40 * 1024 * 1024)).toBe('40 MB');
  });

  it('runs as root directly or through sudo', () => {
    expect(asRoot('root', MANAGE.restart)).toBe('systemctl restart hoi-node');
    expect(asRoot('lena', MANAGE.restart)).toBe("sudo sh -c 'systemctl restart hoi-node'");
    expect(asRoot('lena', MANAGE.deleteLogs)).toBe("sudo sh -c 'journalctl --rotate && journalctl --vacuum-time=1s --unit=hoi-node'");
  });

  it('writes a journald drop-in for the retention the notice promises', () => {
    expect(retentionDropIn(7)).toBe('# Written by hoi node setup: keep this server\'s logs 7 days at most, as the operator notice says.\n[Journal]\nMaxRetentionSec=7day\n');
    expect(retentionDropIn(1)).toContain('1 day at most');
  });

  it('names a server by user, host and port', () => {
    expect(profileId('pi.example', 22, 'root')).toBe('root@pi.example:22');
  });

  it('knows where a local node writes its link on each system', () => {
    expect(localNodeLinkCommand('linux', '/home/lena', 'public')).toEqual({ program: 'sudo', args: ['-n', 'cat', '/var/lib/hoi-public/node-link'] });
    expect(localNodeLinkCommand('macos', '/Users/lena', 'public').args[0]).toBe('/Users/lena/Library/Application Support/hoi-node/public/node-link');
    expect(localNodeLinkCommand('windows', 'C:\\Users\\lena', 'registry').program).toBe('cmd');
  });
});

describe('mailboxes in Manage', () => {
  it('reads what a saved configuration keeps, and the totals the node reports, never what is in them', () => {
    expect(piecesIn('role=public\nname=x\nhold_pieces_gb=2.5\n')).toBe('2.5');
    expect(piecesIn('role=public\nname=x\n')).toBe('');
    expect(piecesSummary('pieces held: 1832 · 412.6 MB · none readable here\nexpiring within a week: 97\n')).toBe('1832 sealed pieces kept, 412.6 MB');
    expect(piecesSummary('pieces held: 1 · 0.0 MB · none readable here')).toBe('1 sealed piece kept, 0.0 MB');
    expect(piecesSummary('this node holds no pieces')).toBeNull();
    expect(MANAGE.pieces).toBe('/usr/local/bin/hoi-node admin --data /var/lib/hoi-public pieces');
  });
});
