// Passkey unlock with real WASM crypto and browser storage, against Chromium's
// virtual authenticator with the PRF extension: protect with a passphrase, add a passkey, lock,
// unlock with the passkey, and see a wrong answer refused. WebAuthn needs a domain, not an IP:
// the page is served as localhost.
//   CHROMIUM_PATH=/opt/pw-browsers/chromium node scripts/e2e-passkey.mjs
import assert from 'node:assert/strict';
import { resolve } from 'node:path';
import { chromium } from 'playwright';
import { createServer } from '../apps/web/node_modules/vite/dist/node/index.js';
import { consentAccepted, unlockBrowser } from './browser-vault.mjs';

const root = resolve(import.meta.dirname, '..');
const server = await createServer({ root: resolve(root, 'apps/web'), logLevel: 'error', server: { host: '127.0.0.1', port: 0, hmr: false } });
await server.listen();
const origin = `http://localhost:${server.httpServer.address().port}`;
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined, args: ['--host-resolver-rules=MAP localhost 127.0.0.1'] });
const errors = [];

async function phase(page, expected) {
  const until = Date.now() + 60_000;
  let state;
  while (Date.now() < until) {
    try {
      state = await page.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.phase);
      if (state === expected) return;
    } catch (error) {
      if (!/Execution context was destroyed|Cannot find context/.test(error.message)) throw error;
    }
    await new Promise((done) => setTimeout(done, 50));
  }
  throw new Error(`Expected ${expected}, got ${state}`);
}

try {
  const context = await browser.newContext({ reducedMotion: 'reduce' });
  await consentAccepted(context);
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  const cdp = await context.newCDPSession(page);
  await cdp.send('WebAuthn.enable');
  const { authenticatorId } = await cdp.send('WebAuthn.addVirtualAuthenticator', {
    options: { protocol: 'ctap2', ctap2Version: 'ctap2_1', transport: 'internal', hasResidentKey: true, hasUserVerification: true, isUserVerified: true, hasPrf: true, automaticPresenceSimulation: true },
  });
  await page.goto(origin);
  await unlockBrowser(page);
  await phase(page, 'ready');
  assert.deepEqual(await page.evaluate(async () => (await import('/src/lib/core/index.ts')).vaultPasskeys()), [], 'no passkey yet');

  // the button under You does exactly this: make one, store its answer in a slot
  await page.evaluate(async () => {
    const { makePasskey } = await import('/src/lib/passkey.ts');
    const { addVaultPasskey } = await import('/src/lib/core/index.ts');
    const made = await makePasskey('hoi · test · browser');
    await addVaultPasskey(made.credential, made.salt, made.prf);
  });
  const listed = await page.evaluate(async () => (await import('/src/lib/core/index.ts')).vaultPasskeys());
  assert.equal(listed.length, 1, 'one passkey opens the vault');
  console.log('✓ a passkey was added to the vault beside the passphrase');

  // lock, and the unlock page offers it
  await page.evaluate(async () => (await import('/src/lib/core/index.ts')).lockVault());
  await page.waitForEvent('load').catch(() => undefined);
  await phase(page, 'locked');
  await page.getByRole('button', { name: 'Unlock with a passkey' }).click();
  await phase(page, 'ready');
  console.log('✓ the passkey unlocked hoi, no passphrase typed');

  // another authenticator's credential for the same id can't: the answer differs
  await page.evaluate(async () => (await import('/src/lib/core/index.ts')).lockVault());
  await page.waitForEvent('load').catch(() => undefined);
  await phase(page, 'locked');
  const wrong = await page.evaluate(async (p) => {
    const { unlockWithPasskey } = await import('/src/lib/core/index.ts');
    try { await unlockWithPasskey(p.credential, 'A'.repeat(43)); return 'opened'; } catch (e) { return String(e?.message ?? e); }
  }, listed[0]);
  assert.notEqual(wrong, 'opened', 'a wrong PRF answer opens nothing');
  await page.getByRole('button', { name: 'Unlock with a passkey' }).click();
  await phase(page, 'ready');
  console.log('✓ a wrong answer opened nothing; the right one still does');

  await cdp.send('WebAuthn.removeVirtualAuthenticator', { authenticatorId });
  await context.close();
  assert.deepEqual(errors, [], `page errors: ${errors.join('; ')}`);
  console.log('PASS: passkey unlock (PRF) with real encrypted storage');
} finally {
  await browser.close();
  await server.close();
}
