// The node setup app's window with its pretend server (src/setup/mock-bridge.ts): the wizard from
// the responsibility text to the node's link, the errors on the way, and Manage. No server is
// contacted and no core is loaded; the real app (apps/setup) wraps this same page.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { chromium } from 'playwright';

const web = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const port = 6300 + Math.floor(Math.random() * 100);
const base = `http://127.0.0.1:${port}`;
const server = spawn(process.execPath, [resolve(web, 'node_modules/vite/bin/vite.js'), ...(process.argv.includes('--built') ? ['preview'] : []), '--mode', 'setup', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: web,
  stdio: ['ignore', 'pipe', 'pipe'],
});
let output = '', browser;
server.stdout.on('data', b => { output += b; });
server.stderr.on('data', b => { output += b; });
try {
  let ready = false, lastResponse = '';
  for (let i = 0; i < 100; i++) {
    try {
      const response = await fetch(base);
      if (response.ok) { ready = true; break; }
      lastResponse = `HTTP ${response.status}: ${(await response.text()).slice(0, 1000)}`;
    } catch (error) { lastResponse = String(error.cause ?? error); }
    if (server.exitCode !== null) break;
    await new Promise(r => setTimeout(r, 100));
  }
  assert(ready, `${output}\n${lastResponse}`);
  const chrome = 'C:/Program Files/Google/Chrome/Application/chrome.exe';
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || (existsSync(chrome) ? chrome : undefined) });
  const shots = resolve(web, '.shots/setup');
  mkdirSync(shots, { recursive: true });

  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, reducedMotion: 'reduce' });
  const page = await context.newPage();
  page.setDefaultTimeout(15_000);
  const failures = [];
  page.on('pageerror', e => failures.push(e.message));
  page.on('console', m => { if (m.type() === 'error') failures.push(m.text()); });
  page.on('response', r => { if (r.status() >= 400) failures.push(`${r.status()} ${r.url()}`); });
  await page.goto(`${base}/`);
  await page.getByText('a pretend server answers here').waitFor();
  const button = name => page.getByRole('button', { name, exact: true });
  const alert = text => page.getByRole('alert').filter({ hasText: text });
  async function audit() {
    await page.addScriptTag({ path: createRequire(import.meta.url).resolve('axe-core/axe.min.js') });
    const findings = await page.evaluate(async () => (await axe.run(document, { runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'] } })).violations.filter(v => ['serious', 'critical'].includes(v.impact)).map(v => ({ id: v.id, targets: v.nodes.map(n => n.target) })));
    assert.deepEqual(findings, [], 'setup accessibility');
  }
  const radio = (group, value) => page.locator(`label:has(input[name="${group}"][value="${value}"])`);

  // 1 · nothing moves before the operator took responsibility.
  assert.equal(await button('Continue').isEnabled(), false);
  await audit();
  await page.locator('#st-ack').check();
  await button('Continue').click();

  // 2 · an unreachable server, then a wrong password, both said plainly.
  await page.getByRole('heading', { name: 'Where does it run?' }).waitFor();
  await page.locator('#st-host').fill('down.example');
  await button('Look at the server').click();
  await alert('could not reach down.example:22').waitFor();
  await page.locator('#st-host').fill('pi.example');
  await radio('st-auth', 'password').click();
  await page.locator('#st-password').fill('wrong');
  await button('Look at the server').click();

  // 3 · the host key is shown and pinned before any login.
  await page.getByRole('heading', { name: 'Is this your server?' }).waitFor();
  await page.getByText('SHA256:mockmockmockmockmockmockmockmockmockmockmoc').waitFor();
  await button('This is my server: pin it and log in').click();
  await alert('root@pi.example did not accept that login').waitFor();
  await button('Back').click();
  await page.locator('#st-password').fill('correct horse');
  await button('Look at the server').click();
  await button('This is my server: pin it and log in').click();

  // 4 · the node: a random name is already valid.
  await page.getByRole('heading', { name: 'What it runs' }).waitFor();
  const name = await page.locator('#st-name').inputValue();
  assert.match(name, /^[a-z]+-[a-z]+-\d+$/, name);
  // The program: the hoi-node this computer built for the server's processor (uname -m: x86_64),
  // not the ARM one beside it, so no published release is needed.
  await page.locator('.program.on').getByText('hoi-node_9.9.9_x86_64-unknown-linux-gnu.tar.gz').waitFor();
  assert.equal(await page.locator('input[name="st-node"][value="file"]').isChecked(), true);
  await page.locator('.program .tag', { hasText: 'x86_64-unknown-linux-gnu' }).waitFor();
  await button('Continue').click();

  // 5 · the notice is read as people will read it. Who and where are optional: left empty, the
  // preview says so in the words everyone who connects reads, and the operator is told.
  await page.getByRole('heading', { name: 'Your join notice' }).waitFor();
  await page.getByText('The operator of this node chose not to say who they are or how to reach them.').waitFor();
  await page.getByText('Left out, and said so in your notice').waitFor();
  assert.equal(await button('Upload and check the server').isEnabled(), true, 'a notice without who and where can be used');
  await page.locator('#st-operator').fill('Lena');
  await page.locator('#st-contact').fill('lena at example dot org');
  await page.locator('#st-country').fill('XX');
  await page.getByText('fits a QR code').waitFor();
  await page.getByText('This node is operated by Lena. Contact: lena at example dot org.').waitFor();
  assert.equal(await page.getByText('Left out, and said so in your notice').count(), 0);
  await page.screenshot({ path: resolve(shots, 'notice.png'), fullPage: true });
  await audit();
  await button('Upload and check the server').click();

  // 6 · the check rows, printed by the installer, before anything changes.
  await page.getByRole('heading', { name: 'Before it installs' }).waitFor();
  assert.equal(await page.locator('table.checks tr').count(), 12);
  // mailboxes are on by default, and the check says what that means
  await page.locator('table.checks tr.ok', { hasText: "keeps up to 2 GB of encrypted pieces and private communities' mailboxes" }).waitFor();
  await page.getByText('Ready.', { exact: true }).waitFor();
  const log = page.locator('details.logbox pre.log');
  assert.match(await log.innerText(), /upload install\.sh → \/tmp\/hoi-setup\.mock\/install\.sh/);
  assert.match(await log.innerText(), /root@pi\.example\$ cd \/tmp\/hoi-setup\.mock && bash \.\/install\.sh check --config hoi-.*\.conf --yes/);
  // The hoi-node from this computer went up beside the kit, arrived whole, and the installer takes it.
  assert.match(await log.innerText(), /upload hoi-node_9\.9\.9_x86_64-unknown-linux-gnu\.tar\.gz \(12\.8 MB\) → \/tmp\/hoi-setup\.mock\/hoi-node_9\.9\.9_x86_64-unknown-linux-gnu\.tar\.gz/);
  assert.match(await log.innerText(), /arrived whole: SHA-256 (ab){32}/);
  assert.match(await log.innerText(), /--yes --binary \.\/hoi-node_9\.9\.9_x86_64-unknown-linux-gnu\.tar\.gz/);
  await page.locator('table.checks tr.ok', { hasText: './hoi-node_9.9.9_x86_64-unknown-linux-gnu.tar.gz' }).waitFor();
  assert.equal(await page.locator('#st-sudo').count(), 0, 'root needs no sudo password');
  await button('Install').click();

  // 7, 8 · the install runs in the open; the link carries the notice.
  await page.getByRole('heading', { name: 'Your node is up.' }).waitFor();
  const link = await page.getByRole('textbox', { name: "The node's link" }).inputValue();
  assert.match(link, /^https:\/\/hoi\.sh\/app\/#\/node\?code=[A-Za-z0-9_-]+$/);
  assert.doesNotMatch(link, /203\.0\.113|@/, 'the link shows no address');
  await page.getByRole('img', { name: "QR code of the node's link" }).waitFor();
  await page.getByText(/Notice hash: c0ffee/).waitFor();
  assert.match(await log.innerText(), /hoi public node runs as a service/);
  assert.match(await log.innerText(), /hoi-node from \.\/hoi-node_9\.9\.9_x86_64/);
  await page.screenshot({ path: resolve(shots, 'done.png'), fullPage: true });

  // Manage: status, logs, the notice as the server has it, log retention; passwords are not kept.
  await button('Manage this server').click();
  await page.getByText('active (running) since Tue 2026-11-03 10:00:00 UTC · hoi-node 9.9.9').waitFor();
  await button('How much is kept').click();
  await page.getByText('1832 sealed pieces kept, 412.6 MB').waitFor();
  await button('Disable mailboxes').click();
  await page.getByText('Off on this node.', { exact: false }).waitFor();
  await page.getByLabel('Mailbox space (GB)').fill('3');
  await button('Enable mailboxes').click();
  await page.getByText('This node keeps up to 3 GB', { exact: false }).waitFor();
  assert.match(await log.innerText(), /--binary '?\/usr\/local\/bin\/hoi-node/);
  await button('Logs').click();
  await page.locator('pre.log.small').filter({ hasText: 'operator notice: by Lena' }).waitFor();
  await button('Edit the notice').click();
  assert.equal(await page.locator('#mn-operator').inputValue(), 'Lena');
  await button('Cancel').click();
  await page.getByLabel('Days to keep logs').fill('14');
  await button('Keep logs this many days').click();
  await page.getByText('The server keeps its logs 14 days at most now.').waitFor();
  assert.match(await log.innerText(), /journald\.conf\.d\/hoi\.conf/);
  await audit();
  await button('Back to your servers').click();
  await page.locator('.servers li').filter({ hasText: 'root@pi.example:22' }).filter({ hasText: 'key pinned' }).waitFor();
  await button('Forget').click();
  await page.getByText('None yet.').waitFor();
  assert.deepEqual(failures, []);
  await context.close();

  // A narrow window still fits.
  const phone = await browser.newContext({ viewport: { width: 390, height: 844 }, reducedMotion: 'reduce' });
  const small = await phone.newPage();
  await small.goto(`${base}/`);
  await small.locator('#st-ack').check();
  await small.getByRole('button', { name: 'Continue', exact: true }).click();
  await small.getByRole('heading', { name: 'Where does it run?' }).waitFor();
  assert(await small.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'the wizard must fit a narrow window');
  // This computer, with its long hoi-node file name, fits too.
  await small.locator('label:has(input[name="st-target"][value="local"])').click();
  await small.getByRole('button', { name: 'Continue', exact: true }).click();
  await small.locator('.program.on').getByText('hoi-node_9.9.9_x86_64-unknown-linux-gnu.tar.gz').waitFor();
  assert(await small.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'the program choice must fit a narrow window');
  await phone.close();
  console.log('PASS: the setup wizard pins the host key, refuses a bad login, checks before installing, prints every command, hands over the link with the notice, and manages the node.');
} finally {
  await browser?.close();
  server.kill();
}
