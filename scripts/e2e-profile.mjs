// Real browser/WASM coverage for signed profile records, encrypted persistence and removal.
import assert from 'node:assert/strict';
import { resolve } from 'node:path';
import { chromium } from 'playwright';
import { createServer } from '../apps/web/node_modules/vite/dist/node/index.js';
import { unlockBrowser, consentAccepted } from './browser-vault.mjs';

const root = resolve(import.meta.dirname, '..');
process.env.VITE_HOI_BACKEND = 'wasm';
process.env.VITE_HOI_REGISTRY_URL = '';
process.env.VITE_HOI_RELAY_URLS = '[]';
process.env.HOI_SANDBOX = '0';
const server = await createServer({ root: resolve(root, 'apps/web'), logLevel: 'error', server: { host: '127.0.0.1', port: 0, hmr: false } });
await server.listen();
const origin = `http://127.0.0.1:${server.httpServer.address().port}`;
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
const errors = [];
try {
  const authorContext = await browser.newContext({ reducedMotion: 'reduce' });
  await consentAccepted(authorContext);
  const readerContext = await browser.newContext({ reducedMotion: 'reduce' });
  await consentAccepted(readerContext);
  const author = await authorContext.newPage();
  const reader = await readerContext.newPage();
  for (const page of [author, reader]) {
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(origin);
    await page.getByRole('heading', { name: 'Protect this device', exact: true }).waitFor();
    await unlockBrowser(page);
  }
  const published = await author.evaluate(async () => {
    const { app } = await import('/src/lib/app.svelte.ts');
    await app.identityChanged(await app.core.createIdentity('Profile author'));
    const community = await app.core.createCommunity({ name: 'profile-check', description: 'Profile browser regression', color: 'mint', limits: { powBits: 12 } });
    const canvas = document.createElement('canvas'); canvas.width = canvas.height = 96;
    const ctx = canvas.getContext('2d'); ctx.fillStyle = '#e0573a'; ctx.fillRect(0, 0, 96, 96);
    ctx.fillStyle = '#ffd24d'; ctx.fillRect(20, 20, 56, 56);
    const blob = await new Promise(resolve => canvas.toBlob(resolve, 'image/jpeg', 0.7));
    const me = await app.core.setAvatar(new Uint8Array(await blob.arrayBuffer()));
    await app.core.setAvatarColor('peach');
    await app.refreshMe();
    const post = await app.core.createPost({ community: community.id, title: 'My signed profile', text: 'A small picture belongs to this public key.' });
    let rejected = false;
    try { await app.core.setAvatar(new Uint8Array(4097)); } catch { rejected = true; }
    if (!rejected || (await app.core.me()).avatar !== me.avatar) throw new Error('Invalid photo replaced the profile');
    let colorRejected = false;
    try { await app.core.setAvatarColor('ultraviolet'); } catch { colorRejected = true; }
    if (!colorRejected || (await app.core.me()).avatarColor !== 'peach') throw new Error('Invalid color replaced the profile');
    return { community: community.id, post: post.id, avatar: me.avatar, avatarColor: 'peach', bundle: [...await app.core.exportBundle(community.id)] };
  });
  assert(published.avatar);
  const received = await reader.evaluate(async data => {
    const { app } = await import('/src/lib/app.svelte.ts');
    await app.identityChanged(await app.core.createIdentity('Profile reader'));
    await app.core.importBundle(new Uint8Array(data.bundle));
    const post = await app.core.post(data.post);
    return { avatar: post.author.avatar, color: post.author.avatarColor };
  }, published);
  assert.equal(received.avatar, published.avatar, 'A reader receives the signed photo alongside the post');
  assert.equal(received.color, published.avatarColor, 'A reader receives the signed color alongside the post');
  const hidden = await reader.evaluate(async post => {
    const { app } = await import('/src/lib/app.svelte.ts');
    await app.setSettings({ autoMedia: false });
    const person = (await app.core.post(post)).author;
    return { avatar: person.avatar, color: person.avatarColor };
  }, published.post);
  assert(!hidden.avatar, 'Media-off keeps received profile pictures hidden');
  assert.equal(hidden.color, published.avatarColor, 'Color remains visible without loading media');
  await author.reload();
  await unlockBrowser(author);
  assert.equal(await author.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.me.avatar), published.avatar, 'Encrypted storage restores the profile on reopen');
  assert.equal(await author.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.me.avatarColor), published.avatarColor, 'Encrypted storage restores the chosen color');
  const removal = await author.evaluate(async data => {
    const { app } = await import('/src/lib/app.svelte.ts');
    await app.core.setAvatar(null);
    await app.core.setAvatarColor(null);
    await app.core.reply(data.post, 'Use my key picture again.');
    return [...await app.core.exportBundle(data.community)];
  }, published);
  const afterRemoval = await reader.evaluate(async data => {
    const { app } = await import('/src/lib/app.svelte.ts');
    await app.setSettings({ autoMedia: true });
    await app.core.importBundle(new Uint8Array(data.bundle));
    const person = (await app.core.post(data.post)).author;
    return { avatar: person.avatar, color: person.avatarColor };
  }, { bundle: removal, post: published.post });
  assert(!afterRemoval.avatar, 'A signed removal restores the identicon on existing posts');
  assert(!afterRemoval.color, 'A signed reset restores the key color on existing posts');
  assert.deepEqual(errors, []);
  console.log('PASS: real WASM profile photo/color publish, read, limits, media preference, encrypted reopen and reset');
} finally {
  await browser.close();
  await server.close();
}
