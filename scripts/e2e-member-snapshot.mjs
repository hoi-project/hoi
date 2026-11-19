// Actual WASM, encrypted browser vaults and the normal snapshot forms. Each member has a
// separate clean browser profile. Private ciphertext is delivered explicitly in memory;
// no relay, project service or external address is contacted.
// Build current bindings, then: CHROMIUM_PATH=/usr/bin/chromium node scripts/e2e-member-snapshot.mjs
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { chromium } from 'playwright';
import { createServer } from '../apps/web/node_modules/vite/dist/node/index.js';
import { consentAccepted, unlockBrowser } from './browser-vault.mjs';
import { rawStore, storeHelpers } from './browser-store.mjs';

const root = resolve(import.meta.dirname, '..');
process.env.VITE_HOI_BACKEND = 'wasm';
process.env.VITE_HOI_REGISTRY_URL = '';
process.env.VITE_HOI_RELAY_URLS = '[]';
process.env.HOI_SANDBOX = '0';
const server = await createServer({ root:resolve(root,'apps/web'), logLevel:'error', server:{host:'127.0.0.1',port:0,hmr:false} });
await server.listen();
const origin = `http://127.0.0.1:${server.httpServer.address().port}`;
let browser;
const contexts = [], errors = [], unexpectedRequests = [];
const password = 'member snapshot browser test password';
const digest = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');

async function call(page, method, ...args) {
  return page.evaluate(async ({method,args}) => {
    const { app } = await import('/src/lib/app.svelte.ts');
    return app.core[method](...args);
  }, {method,args});
}

async function cleanPage(name) {
  const context = await browser.newContext({ viewport:{width:1100,height:850}, reducedMotion:'reduce', acceptDownloads:true, serviceWorkers:'block' });
  contexts.push(context);
  await consentAccepted(context);
  await storeHelpers(context);
  await context.route('**/*', route => {
    const url = new URL(route.request().url());
    if (url.origin === origin) return route.continue();
    // The ordinary preview banner may ask for the static release list. Keep the test offline.
    if (url.pathname === '/releases/manifest.json') return route.fulfill({ contentType:'application/json', body:JSON.stringify({schema:1,latest:null,releases:[]}) });
    unexpectedRequests.push(url.origin);
    return route.abort();
  });
  const page = await context.newPage();
  page.setDefaultTimeout(60_000);
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(origin);
  await unlockBrowser(page);
  await page.getByRole('heading',{name:'Restore my identity',exact:true}).waitFor();
  if (name) await page.evaluate(async name => {
    const { app } = await import('/src/lib/app.svelte.ts');
    await app.identityChanged(await app.core.createIdentity(name));
  }, name);
  return {page,context};
}

/** Each body is a real MLS ciphertext, not a plaintext API shortcut. */
async function deliver(from, to, community) {
  const pending = await call(from,'privateOutbox',community);
  for (const item of pending) await call(to,'privateReceive',community,item.body);
  if (pending.length) await call(from,'privateAck',community,pending.map(item => item.id));
  return pending.length;
}
async function clearOutbox(page, community) {
  const pending = await call(page,'privateOutbox',community);
  if (pending.length) await call(page,'privateAck',community,pending.map(item => item.id));
}

try {
  browser = await chromium.launch({executablePath:process.env.CHROMIUM_PATH || undefined});
  const {page:owner} = await cleanPage('Snapshot community owner');
  const {page:member,context:oldContext} = await cleanPage('Snapshot member');
  const identity = await call(member,'me');
  const community = await call(owner,'createPrivateCommunity',{name:'snapshot-private',description:'Local snapshot test',color:'mint',limits:{powBits:12,admission:'invite'}});
  const initial = await call(member,'privateJoinRequest');
  const welcome = await call(owner,'privateInvite',community.id,initial);
  await call(member,'privateJoin',welcome);
  // The welcome already gives the newcomer the committed epoch and authenticated bootstrap.
  await clearOutbox(owner,community.id);
  const oldPrivate = await call(owner,'createPost',{community:community.id,title:'private-snapshot-cache-marker',text:'A stale local copy must not return from this snapshot.'});
  assert.ok(await deliver(owner,member,community.id),'Real private ciphertext reaches the member before backup');
  assert.equal((await call(member,'post',oldPrivate.id)).title,oldPrivate.title);
  const publicCommunity = await call(member,'createCommunity',{name:'snapshot-public',description:'Public catch-up quarantine test',color:'sky',limits:{powBits:12}});
  const publicPost = await call(member,'createPost',{community:publicCommunity.id,title:'public-snapshot-cache-marker',text:'Restored public content waits for deletion catch-up.'});

  await member.goto(`${origin}/#/you`);
  await member.getByRole('button',{name:'Back up my identity',exact:true}).click();
  await member.getByRole('button',{name:'Member snapshot',exact:true}).click();
  await member.getByLabel('New backup-file password',{exact:true}).fill(password);
  await member.getByLabel('Repeat backup-file password',{exact:true}).fill(password);
  const downloading = member.waitForEvent('download');
  await member.getByRole('button',{name:'Download .hoisnapshot',exact:true}).click();
  const download = await downloading;
  const filename = download.suggestedFilename();
  assert.match(filename,/\.hoisnapshot$/);
  const archive = await readFile(await download.path());
  assert.ok(archive.length > 70 && archive.length <= 32*1024*1024);
  assert.equal(archive.includes(Buffer.from('private-snapshot-cache-marker')),false,'The portable file does not expose private plaintext');
  await oldContext.close(); // A snapshot replaces the original device; never run both copies.

  const {page:restored} = await cleanPage();
  await restored.getByRole('button',{name:'Use member snapshot',exact:true}).click();
  await restored.getByLabel('Member snapshot (.hoisnapshot)',{exact:true}).setInputFiles({name:filename,mimeType:'application/octet-stream',buffer:archive});
  await restored.getByRole('checkbox',{name:/I am replacing the original device/}).check();
  const unchanged = digest(await rawStore(restored));
  await restored.getByLabel('Snapshot password',{exact:true}).fill('an incorrect snapshot password');
  await restored.getByRole('button',{name:'Restore member snapshot',exact:true}).click();
  await restored.getByRole('alert').waitFor();
  await restored.waitForFunction(() => document.querySelector('#fr-snapshot-pass')?.value === '');
  assert.equal(await call(restored,'me'),null);
  assert.equal(digest(await rawStore(restored)),unchanged,'Wrong passwords cannot rewrite the fresh encrypted vault');
  const damaged = Buffer.from(archive); damaged[damaged.length-1] ^= 1;
  await restored.getByLabel('Member snapshot (.hoisnapshot)',{exact:true}).setInputFiles({name:filename,mimeType:'application/octet-stream',buffer:damaged});
  await restored.getByLabel('Snapshot password',{exact:true}).fill(password);
  await restored.getByRole('button',{name:'Restore member snapshot',exact:true}).click();
  await restored.waitForFunction(() => document.querySelector('#fr-snapshot-pass')?.value === '');
  assert.equal(await call(restored,'me'),null);
  assert.equal(digest(await rawStore(restored)),unchanged,'Tampered archives cannot partially replace storage');
  await restored.getByLabel('Member snapshot (.hoisnapshot)',{exact:true}).setInputFiles({name:filename,mimeType:'application/octet-stream',buffer:archive});
  await restored.getByLabel('Snapshot password',{exact:true}).fill(password);
  await restored.getByRole('button',{name:'Restore member snapshot',exact:true}).click();
  await restored.getByRole('heading',{name:'Restore private access',exact:true}).waitFor();
  const broughtBack = await call(restored,'me');
  assert.equal(broughtBack.rootKey,identity.rootKey);
  assert.equal(broughtBack.device,identity.device,'This is a replacement of the original member device');
  assert.equal((await call(restored,'privateGroup',community.id)).active,false,'An archive cannot restore active private membership');
  await assert.rejects(call(restored,'post',oldPrivate.id),'Stale private plaintext is discarded');
  await assert.rejects(call(restored,'post',publicPost.id),'Public records wait for peer deletion catch-up');
  assert.equal((await call(restored,'net')).homeNode,undefined,'Pairing is installation-local');

  await restored.reload();
  await unlockBrowser(restored);
  await restored.getByRole('heading',{name:'Restore private access',exact:true}).waitFor();
  assert.equal((await call(restored,'me')).device,identity.device,'Snapshot state persists through encrypted-vault reload');
  assert.equal((await call(restored,'privateGroup',community.id)).active,false);
  await restored.getByRole('button',{name:'Request access to snapshot-private',exact:true}).click();
  await restored.getByRole('dialog',{name:'Turn on your VPN before joining'}).getByRole('button',{name:'Continue joining',exact:true}).click();
  const request = await restored.getByLabel('Send the request for snapshot-private to an authorized member',{exact:true}).inputValue();
  assert.equal((await call(restored,'privateGroup',community.id)).active,false,'Making a request cannot self-approve');
  const approved = await call(owner,'privateInvite',community.id,request);
  await restored.getByLabel('Your private invitation',{exact:true}).fill(approved);
  await restored.getByRole('button',{name:'Open private invitation',exact:true}).click();
  await restored.getByRole('dialog',{name:'Turn on your VPN before joining'}).getByRole('button',{name:'Continue joining',exact:true}).click();
  await restored.waitForFunction(async community => (await (await import('/src/lib/app.svelte.ts')).app.core.privateGroup(community)).active,community.id);
  assert.deepEqual(await call(restored,'snapshotRejoins'),[]);
  await clearOutbox(owner,community.id);
  const fresh = await call(owner,'createPost',{community:community.id,title:'approved-fresh-snapshot-delivery',text:'Fresh membership accepts current ciphertext.'});
  assert.ok(await deliver(owner,restored,community.id));
  assert.equal((await call(restored,'post',fresh.id)).title,fresh.title);
  await restored.reload(); await unlockBrowser(restored);
  assert.equal((await call(restored,'privateGroup',community.id)).active,true,'Fresh membership is durable');
  assert.equal((await call(restored,'post',fresh.id)).title,fresh.title);
  assert.deepEqual(errors,[]);
  assert.deepEqual(unexpectedRequests,[]);
  console.log('PASS: real-WASM encrypted snapshot forms across clean profiles, wrong password and tamper atomicity, public/private quarantine, reload and authorized fresh private delivery.');
} finally {
  for (const context of contexts) await context.close();
  await browser?.close();
  await server.close();
}
