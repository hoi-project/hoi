// Snapshot UI acceptance with an explicit stubbed secure-core boundary. Cryptographic archive,
// atomic restoration and private quarantine semantics are tested by the real core suites.
import assert from 'node:assert/strict';
import { chromium } from 'playwright';
import { startComponentServer } from './e2e-component-server.mjs';

process.env.VITE_HOI_BACKEND = 'mock';
const { server, origin } = await startComponentServer('/snapshot-ui-test', `<!doctype html><html lang="en"><title>Member snapshots</title>
  <main><div id="backup" class="gutter"></div><div id="first"></div><div id="private" class="gutter"></div></main>
  <script type="module">
    import { mount } from 'svelte';
    import FirstRun from '/src/routes/FirstRun.svelte';
    import KeyBackup from '/src/lib/ui/KeyBackup.svelte';
    import PrivateCommunities from '/src/routes/PrivateCommunities.svelte';
    import { createMockCore } from '/src/lib/core/mock-core.ts';
    import { app } from '/src/lib/app.svelte.ts';
    import '@hoi/tokens/fonts.css'; import '@hoi/tokens/tokens.css'; import '/src/app.css';
    await app.start();
    const donor = createMockCore(); await donor.boot(); const me = await donor.createIdentity('Example');
    const fixture = window.snapshotFixture = { app, me, exports: [], imports: [], requests: [], imported: false, pending: false, joined: 0 };
    app.refresh = async () => {}; app.refreshWarmth = async () => {};
    app.identityChanged = async restored => { app.me = restored; app.rev++; };
    app.syncPrivate = async () => ({ busy:false, replicas:0, total:0 });
    app.core.exportSnapshot = async password => { fixture.exports.push(password); return new Uint8Array([1,2,3]); };
    app.core.importSnapshot = async (bytes, password) => {
      fixture.imports.push({ bytes: Array.from(bytes), password });
      if (password !== 'the correct snapshot password') throw new Error('This password did not open the snapshot. No data was replaced.');
      fixture.imported = fixture.pending = true; return me;
    };
    app.core.snapshotRejoins = async () => fixture.pending ? [['a'.repeat(64), 'Saved private group']] : [];
    app.core.snapshotRejoinRequest = async id => { fixture.requests.push(id); return 'hoi-request-fixture'; };
    app.core.privateJoin = async invitation => {
      if (invitation !== 'approved invitation') throw new Error('An authorized invitation is required.');
      fixture.joined++; fixture.pending = false; app.rev++; return { id:'a'.repeat(64) };
    };
    mount(KeyBackup, { target:document.querySelector('#backup'), props:{ me } });
    mount(FirstRun, { target:document.querySelector('#first'), props:{ route:{name:'home'} } });
    mount(PrivateCommunities, { target:document.querySelector('#private') });
  </script></html>`);
let browser;
try {
  browser = await chromium.launch({ executablePath:process.env.CHROMIUM_PATH || undefined });
  const context = await browser.newContext({ viewport:{width:1280,height:900}, reducedMotion:'reduce', acceptDownloads:true, serviceWorkers:'block' });
  const page = await context.newPage();
  const errors = [], external = [];
  page.on('pageerror', error => errors.push(error.message));
  await context.route('**/*', route => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    external.push(route.request().url()); return route.abort();
  });
  await page.goto(`${origin}/snapshot-ui-test`);
  const backup = page.locator('#backup');
  await backup.getByRole('button', {name:'Member snapshot',exact:true}).click();
  assert.match(await backup.innerText(), /do not run both copies/);
  await backup.getByLabel('New backup-file password', {exact:true}).fill('the correct snapshot password');
  await backup.getByLabel('Repeat backup-file password', {exact:true}).fill('a different snapshot password');
  await backup.getByRole('button', {name:'Download .hoisnapshot',exact:true}).click();
  await backup.getByRole('alert').filter({hasText:"passwords don't match"}).waitFor();
  assert.equal(await page.evaluate(() => window.snapshotFixture.exports.length), 0);
  await backup.getByLabel('Repeat backup-file password', {exact:true}).fill('the correct snapshot password');
  const downloadReady = page.waitForEvent('download');
  await backup.getByRole('button', {name:'Download .hoisnapshot',exact:true}).click();
  assert.match((await downloadReady).suggestedFilename(), /\.hoisnapshot$/);
  assert.deepEqual(await page.evaluate(() => window.snapshotFixture.exports), ['the correct snapshot password']);
  assert.equal(await backup.getByLabel('New backup-file password', {exact:true}).inputValue(), '', 'Export clears the typed secret');

  const first = page.locator('#first');
  await first.getByRole('button', {name:'Use member snapshot',exact:true}).click();
  const input = first.getByLabel('Member snapshot (.hoisnapshot)', {exact:true});
  await input.setInputFiles({ name:'saved.hoisnapshot', mimeType:'application/octet-stream', buffer:Buffer.from([1,2,3]) });
  assert.equal(await first.locator('#fr-snapshot-entry').inputValue(), 'hoi member snapshot · saved.hoisnapshot', 'Snapshot credentials stay distinct from vault and identity-file passwords');
  await first.getByLabel('Snapshot password', {exact:true}).fill('incorrect snapshot password');
  await first.getByRole('button', {name:'Restore member snapshot',exact:true}).click();
  assert.equal(await page.evaluate(() => window.snapshotFixture.imports.length), 0, 'Replacement acknowledgement is required');
  await first.getByRole('checkbox', {name:/I am replacing the original device/}).check();
  await first.getByRole('button', {name:'Restore member snapshot',exact:true}).click();
  await first.getByRole('alert').filter({hasText:'No data was replaced.'}).waitFor();
  assert.equal(await page.evaluate(() => window.snapshotFixture.imported), false);
  assert.equal(await page.evaluate(() => window.snapshotFixture.app.me), null, 'A failed snapshot does not create or replace an identity');
  assert.equal(await input.evaluate(el => el.files[0].name), 'saved.hoisnapshot', 'An import error retains the file for retry');
  assert.equal(await first.getByLabel('Snapshot password', {exact:true}).inputValue(), '');
  await input.evaluate(el => Object.defineProperty(el.files[0], 'size', {value:32*1024*1024+1,configurable:true}));
  await first.getByLabel('Snapshot password', {exact:true}).fill('the correct snapshot password');
  await first.getByRole('button', {name:'Restore member snapshot',exact:true}).click();
  await first.getByRole('alert').filter({hasText:'32 MiB or smaller'}).waitFor();
  assert.equal(await page.evaluate(() => window.snapshotFixture.imports.length), 1, 'Oversized files never cross the secure-core boundary');
  await input.setInputFiles({name:'saved.hoisnapshot',mimeType:'application/octet-stream',buffer:Buffer.from([1,2,3])});
  await first.getByRole('button', {name:'Restore member snapshot',exact:true}).click();
  await page.waitForFunction(() => window.snapshotFixture.imported);

  const privatePage = page.locator('#private');
  await privatePage.getByRole('heading', {name:'Restore private access'}).waitFor();
  assert.equal(await page.evaluate(() => window.snapshotFixture.requests.length), 0, 'Import alone cannot request or approve private membership');
  await privatePage.getByRole('button', {name:'Request access to Saved private group'}).click();
  const warning = page.getByRole('dialog', {name:'Turn on your VPN before joining'});
  assert.equal(await page.evaluate(() => window.snapshotFixture.requests.length), 0, 'The explicit choice still waits for confirmation');
  await warning.getByRole('button', {name:'Not now',exact:true}).click();
  await privatePage.getByRole('button', {name:'Request access to Saved private group'}).click();
  await warning.getByRole('button', {name:'Continue joining',exact:true}).click();
  await privatePage.getByLabel('Send the request for Saved private group to an authorized member', {exact:true}).waitFor();
  assert.deepEqual(await page.evaluate(() => window.snapshotFixture.requests), ['a'.repeat(64)]);
  assert.equal(await page.evaluate(() => window.snapshotFixture.joined), 0, 'Making a request never self-approves membership');
  for (const width of [1280,320]) {
    await page.setViewportSize({width,height:900});
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `Snapshot controls fit ${width} px`);
  }
  await privatePage.getByLabel('Your private invitation',{exact:true}).fill('approved invitation');
  await privatePage.getByRole('button',{name:'Open private invitation',exact:true}).click();
  await warning.getByRole('button',{name:'Continue joining',exact:true}).click();
  await page.waitForFunction(() => window.snapshotFixture.joined === 1);
  await privatePage.getByRole('heading',{name:'Restore private access'}).waitFor({state:'hidden'});
  assert.deepEqual(errors, []);
  assert.deepEqual(external, [], 'Backup and restore UI use no account or external service');
  await context.close();
  console.log('PASS: snapshot download, password mismatch, fresh-install acknowledgement, wrong password retry, 32 MiB bound and explicit private approval at 320 px (stubbed secure-core boundary).');
} finally { await browser?.close(); await server.close(); }
