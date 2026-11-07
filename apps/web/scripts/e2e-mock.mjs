// Drive the app against the mock backend with Playwright: first run → say hoi (stir, stamp, meet your key, 3 of 5
// friends) → start a community → post (sign timeline) →
// open a post → reply → vote → Manage (limits, add a mod, ban, admit, needs-cosign, cosign) → ledger → You
// (pair, export key) → blocked tab → join via invite. At 1280×900 and 390×844, light and dark.
// Fails on console errors, page errors, failed steps and horizontal overflow.
//
//   node scripts/e2e-mock.mjs [--out <dir>] [--url http://localhost:5173] [--only desktop-light,phone-dark]
//
// Without --url it starts `vite` with VITE_HOI_BACKEND=mock itself. Uses the root devDependency `playwright`
// and the preinstalled Chromium (never runs `playwright install`).
import { chromium } from 'playwright';
import { spawn } from 'node:child_process';
import { mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { consentAccepted } from '../../../scripts/browser-vault.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const webDir = resolve(here, '..');
const args = process.argv.slice(2);
const arg = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? args[i + 1] : fallback;
};
const out = resolve(arg('out', join(webDir, '.shots')));
const only = arg('only', '')?.split(',').filter(Boolean) ?? [];
mkdirSync(out, { recursive: true });

const COMBOS = [
  { tag: 'desktop-light', viewport: { width: 1280, height: 900 }, scheme: 'light', phone: false },
  { tag: 'desktop-dark', viewport: { width: 1280, height: 900 }, scheme: 'dark', phone: false },
  { tag: 'phone-light', viewport: { width: 390, height: 844 }, scheme: 'light', phone: true },
  { tag: 'phone-dark', viewport: { width: 390, height: 844 }, scheme: 'dark', phone: true },
].filter((c) => only.length === 0 || only.includes(c.tag));

// A real did:key for a made-up moderator, to exercise key parsing.
const B58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
function b58(bytes) {
  const d = [0];
  for (const byte of bytes) {
    let c = byte;
    for (let i = 0; i < d.length; i++) {
      c += d[i] << 8;
      d[i] = c % 58;
      c = (c / 58) | 0;
    }
    while (c) {
      d.push(c % 58);
      c = (c / 58) | 0;
    }
  }
  return d.reverse().map((x) => B58[x]).join('');
}
const MOD_HEX = '5e'.repeat(32);
const MOD_DID = 'did:key:z' + b58(Uint8Array.from([0xed, 0x01, ...Buffer.from(MOD_HEX, 'hex')]));
const BAN_HEX = 'b4'.repeat(32);

// ------------------------------------------------------------------ server
let server = null;
async function startServer() {
  const given = arg('url', '');
  if (given) return given.replace(/\/$/, '');
  const port = 5300 + Math.floor(Math.random() * 400);
  server = spawn(process.execPath, [join(webDir, 'node_modules/vite/bin/vite.js'), '--port', String(port), '--strictPort'], {
    cwd: webDir,
    env: { ...process.env, VITE_HOI_BACKEND: 'mock' },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  server.stderr.on('data', (d) => process.stderr.write(d));
  const url = `http://localhost:${port}`;
  for (let i = 0; i < 100; i++) {
    try {
      if ((await fetch(url)).ok) return url;
    } catch {
      /* not yet */
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error('vite did not start');
}

// ------------------------------------------------------------------ run
const problems = [];
const base = await startServer();
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
console.log(`base ${base}, shots → ${out}`);

for (const combo of COMBOS) {
  const t0 = Date.now();
  const ctxOpts = {
    viewport: combo.viewport,
    colorScheme: combo.scheme,
    acceptDownloads: true,
    ...(combo.phone ? { isMobile: true, hasTouch: true, deviceScaleFactor: 2 } : {}),
  };
  const ctx = await browser.newContext(ctxOpts);
  await consentAccepted(ctx);
  const page = await ctx.newPage();
  watch(page, combo.tag);
  let inviteWeb = '';
  let ownId = '';

  const shot = async (name, p = page) => {
    await p.evaluate(() => window.scrollTo(0, 0));
    // Every entrance must end fully visible, even if data or a re-render landed mid-animation.
    const settled = () => [...document.querySelectorAll('.anim')].every((el) => Number(getComputedStyle(el).opacity) > 0.99);
    const ok = await p.waitForFunction(settled, null, { timeout: 2500 }).then(() => true, () => false);
    if (!ok) {
      const n = await p.evaluate(() => [...document.querySelectorAll('.anim')].filter((el) => Number(getComputedStyle(el).opacity) <= 0.99).length);
      problems.push(`${combo.tag} ${name}: ${n} element(s) still not fully visible after 2.5 s`);
    }
    await p.waitForTimeout(150);
    await p.screenshot({ path: join(out, `${combo.tag}-${name}.png`), fullPage: !combo.phone });
    const over = await p.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    if (over > 0) problems.push(`${combo.tag} ${name}: horizontal overflow of ${over}px`);
  };
  const step = async (name, fn) => {
    try {
      await fn();
    } catch (e) {
      problems.push(`${combo.tag} step "${name}" failed: ${e.message.split('\n')[0]}`);
      await page.screenshot({ path: join(out, `${combo.tag}-FAILED-${name.replace(/\W+/g, '-')}.png`), fullPage: true }).catch(() => {});
    }
  };
  const toast = (re) => page.locator('.toast').filter({ hasText: re }).first().waitFor({ timeout: 10_000 });
  const go = async (hash) => {
    await page.evaluate((h) => (location.hash = h), hash);
    await page.waitForTimeout(250);
  };

  await step('first run', async () => {
    await page.goto(`${base}/`);
    await page.getByText('Pick a name, get a key.').waitFor();
    await shot('01-first-run');
  });

  await step('say hoi: stir, stamp, meet your key', async () => {
    await page.locator('input[placeholder="lena"]:not(#fr-restore-name)').fill('lena');
    await page.getByRole('button', { name: 'Start one →' }).click();
    await page.getByRole('heading', { name: 'Stir the swarm.' }).waitFor();
    const box = await page.locator('.swarm').boundingBox();
    const r = Math.min(box.height * 0.4, box.width * 0.42);
    for (let k = 0; k < 4; k++) {
      await page.mouse.move(box.x + box.width / 2 - r, box.y + box.height / 2 - r * 0.6 + k * r * 0.4);
      await page.mouse.move(box.x + box.width / 2 + r, box.y + box.height / 2 - r * 0.5 + k * r * 0.4, { steps: 15 });
    }
    if (Number(await page.locator('.hud').getAttribute('aria-valuenow')) <= 0) throw new Error('stirring filled nothing');
    await shot('01a-stir');
    await page.getByRole('button', { name: 'Make my key →' }).click();
    await page.locator('.strip .bits').waitFor();
    await page.getByRole('button', { name: 'Meet your key →' }).waitFor({ timeout: 20_000 });
    await shot('01b-stamped');
    await page.getByRole('button', { name: 'Meet your key →' }).click();
    await page.getByRole('heading', { name: 'Meet your key, lena.' }).waitFor();
    await shot('01c-meet');
    if (!combo.phone) {
      await page.getByRole('button', { name: 'How do friends get me back in?' }).click();
      await page.getByRole('button', { name: 'Hand them out for me' }).click();
      for (const f of [0, 2, 4]) await page.locator(`[data-friend="${f}"]`).click();
      await page.locator('.verdict.yes').waitFor();
      await shot('01d-friends');
    }
  });

  await step('make key and start a community', async () => {
    await page.getByRole('button', { name: 'Start my community →' }).click();
    await page.getByRole('heading', { name: 'Start a community' }).waitFor();
    await page.locator('#n-name').fill('bikes-repair');
    await page.locator('#n-desc').fill('Fixing bikes, routes, and where to get parts nearby.');
    await page.locator('label.sw-c[title="mint"]').click();
    await page.locator('[aria-labelledby="n-ret"] label', { hasText: '90 d' }).click();
    await page.locator('#n-rules').fill('1. Be kind.\n2. No selling.\n3. Photos of other people only with a yes.');
    if (combo.tag === 'desktop-light') {
      await page.getByText('Share ownership').click();
      await page.locator('.ownersv textarea').fill(MOD_DID);
    }
    await shot('02-new-community');
    await page.getByRole('button', { name: 'Make c/bikes-repair' }).click();
    await page.locator('dialog[open] h2', { hasText: 'exists' }).waitFor();
    inviteWeb = await page.locator('dialog[open] input.mono-in').first().inputValue();
    ownId = (await page.evaluate(() => location.hash)).split('/')[2];
    await shot('03-invite');
    await page.keyboard.press('Escape');
    await page.locator('[aria-label="Community identity"]').getByText('Local name · public reservation unavailable', { exact: true }).waitFor();
    await page.getByText('Community ID and public name', { exact: true }).click();
    if (await page.getByLabel("This community's full ID", { exact: true }).inputValue() !== ownId) throw new Error('community identity does not show its full ID');
    await page.getByText('Community ID and public name', { exact: true }).click();
  });

  await step('post with the sign timeline', async () => {
    // This UI flow exercises sending through a connected community. Offline draft and
    // disconnect-before-submit behavior are covered by e2e-public-reading.mjs.
    await page.evaluate(async () => {
      const { app } = await import('/src/lib/app.svelte.ts');
      const status = { ...(await app.core.net()), peers: 1, communities: app.joined.map(c => ({ community: c.id, connectedPeers: 1 })) };
      app.core.net = async () => status;
      app.net = status;
    });
    if (combo.phone) await page.locator('.fab').click();
    else await page.locator('header .newpost').click();
    // "+ Post" puts the cursor in the title within the same tap (a sheet over the screen on a phone).
    if (!(await page.evaluate(() => document.activeElement?.id === 'ctitle'))) throw new Error('the title field is not focused after + Post');
    if (combo.phone && !(await page.locator('.composer.sheet').isVisible())) throw new Error('the phone composer is not a sheet');
    await page.locator('#ctitle').fill('Anyone have a spare 11-speed chain tool?');
    await page.locator('#ctext').fill('Mine snapped. I can bring cake.\n\nBack room on Thursday works.');
    await page.getByRole('status', { name: 'Sync status' }).getByText('Connected to this community', { exact: true }).waitFor({ state: 'attached' });
    // Sample the actual animation, not only its final resting box. The stamp used to extend
    // above its overflow:hidden composer, especially while scaling in on narrow screens.
    await page.evaluate(() => {
      window.__stampCheck = { samples: 0, clipped: false };
      const sample = () => {
        const stamp = document.querySelector('.composer .stamp');
        const composer = document.querySelector('.composer');
        if (!stamp || !composer) return;
        if ((composer.style.height === 'auto' || composer.classList.contains('sheet')) && Number(getComputedStyle(stamp).opacity) > 0.1) {
          const s = stamp.getBoundingClientRect();
          const c = composer.getBoundingClientRect();
          window.__stampCheck.samples++;
          window.__stampCheck.clipped ||= s.top < c.top - 1 || s.bottom > c.bottom + 1 || s.left < c.left - 1 || s.right > c.right + 1;
        }
        requestAnimationFrame(sample);
      };
      requestAnimationFrame(sample);
    });
    await page.getByRole('button', { name: combo.phone ? 'Send' : 'Sign & send', exact: true }).click();
    // A browser without a home node of its own is told once who can read what it posts.
    const notice = page.getByRole('dialog', { name: 'Nobody can read posts from this browser' });
    await notice.getByText(/reaches others only through a home node that carries this community \(1 node connected now\)/).waitFor();
    await shot('04-posting-notice');
    await notice.getByRole('button', { name: 'Post through them' }).click();
    await page.waitForTimeout(450);
    await page.screenshot({ path: join(out, `${combo.tag}-04-signing.png`), fullPage: false });
    await page.waitForFunction(() => {
      const stamp = document.querySelector('.composer .stamp');
      return stamp && Number(getComputedStyle(stamp).opacity) > 0.9;
    });
    await page.screenshot({ path: join(out, `${combo.tag}-04a-stamped.png`), fullPage: false });
    await toast(/Signed\. Your stamp took/);
    await page.locator('.toast').filter({ hasText: /Sent to the home nodes carrying this community; others read it from there\. Delivery is not confirmed/ }).waitFor();
    const stamp = await page.evaluate(() => window.__stampCheck);
    if (!stamp.samples || stamp.clipped) throw new Error(`signed stamp clipped during animation (${stamp.samples} samples)`);
    await shot('05-posted');
  });

  await step('home feed, peek', async () => {
    await go('#/');
    await page.getByRole('heading', { name: 'Hey lena.' }).waitFor();
    await page.getByRole('button', { name: 'Peek anyway' }).first().click();
    await shot('06-home');
  });

  await step('post: reply, vote, flag', async () => {
    await page.getByRole('link', { name: /Which messengers actually survived/ }).first().click();
    await page.getByRole('heading', { name: /Which messengers/ }).waitFor();
    await shot('07-post');
    await page.locator('form.reply textarea').first().fill('Counting defaults seems right. Thanks for doing this.');
    await page.getByRole('button', { name: 'Sign & send →' }).first().click();
    await toast(/Reply signed/);
    await page.locator('.hvote button[aria-label="Upvote"]').click();
    await toast(/Upvote signed/);
    await page.getByRole('button', { name: 'Flag for the mods' }).click();
    await page.locator('form.flag input[value="illegal"]').check();
    await shot('08-post-after');
    await page.locator('form.flag').getByRole('button', { name: 'Cancel' }).click();
  });

  await step('manage own community: limits, add mod, ban', async () => {
    await go(`#/c/${ownId}/manage`);
    await page.getByRole('heading', { name: 'Limits' }).waitFor();
    await page.locator('#m-pow').evaluate((el) => {
      el.value = '20';
      el.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await page.locator('[aria-labelledby="m-ret"] label', { hasText: '30 d' }).click();
    await page.getByRole('button', { name: 'Propose new limits' }).click();
    await toast(/New limits in effect/);
    await page.getByPlaceholder('their key (64 hex) or did:key:z6Mk…').fill(MOD_DID);
    await page.getByRole('button', { name: 'Sign & add' }).click();
    await toast(/New moderator signed/);
    await page.getByPlaceholder('key (hex) or did:key:z6Mk…').fill(BAN_HEX);
    await page.getByPlaceholder('reason, e.g. rule 2, doxxing').fill('spam, three times');
    await page.getByRole('button', { name: 'Ban & sign' }).click();
    await toast(/Banned\./);
    await shot('09-manage-own');
  });

  const openCommunity = async (name) => {
    await go('#/communities');
    // Joined communities are rows with a link and a favorite star; others are links themselves.
    await page.locator('main .crow a.cbody, main a.crow', { hasText: `c/${name}` }).first().click();
    await page.getByRole('heading', { name, exact: true }).waitFor();
  };

  await step('manage as a mod: admit', async () => {
    await openCommunity('hackspace');
    await page.getByRole('link', { name: /^Manage/ }).click();
    await page.getByRole('heading', { name: 'Waiting at the door' }).waitFor();
    await shot('10-manage-mod');
    await page.getByRole('button', { name: 'Admit', exact: true }).first().click();
    await toast(/is in/);
  });

  await step('owner change needs cosign, cosign a code', async () => {
    await openCommunity('privacy');
    await page.getByRole('link', { name: /^Manage/ }).click();
    await page.locator('#m-npd').selectOption('5');
    await page.getByRole('button', { name: 'Sign & ask the other owners' }).click();
    await page.locator('.proposal').first().waitFor();
    await page.locator('.proposal').first().scrollIntoViewIfNeeded();
    await page.screenshot({ path: join(out, `${combo.tag}-11-needs-cosign.png`) });
    await page.getByPlaceholder('hoi://cosign?p=…').fill('hoi://cosign?p=7a1e44c0b9d2f3a8');
    await page.getByRole('button', { name: 'Cosign', exact: true }).click();
    await toast(/Cosigned/);
  });

  await step('ledger', async () => {
    await page.getByRole('link', { name: 'Ledger', exact: true }).click();
    await page.getByRole('heading', { name: 'The ledger' }).waitFor();
    await shot('12-ledger');
  });

  await step('you: pair, export key', async () => {
    await go('#/you');
    await page.getByRole('heading', { name: /Hey lena\./ }).waitFor();
    await page.locator('#y-pair').fill('hoi://pair?u=wss://home.example:7842&t=7f3c9a&n=lena%27s%20desktop');
    await page.getByRole('button', { name: 'Pair', exact: true }).click();
    await toast(/Paired|Connected to lena's desktop/);
    await page.locator('.relay', { hasText: "via lena's desktop" }).waitFor();
    // The network map draws the links and where a post in each community goes.
    await page.locator('.netmap').getByText(/is connected to \d+ devices? now/).waitFor();
    await page.locator('.netmap .route').first().click();
    await page.locator('.netmap .path').getByText(/goes from here to \d+ links?/).waitFor();
    await shot('13a-network');
    await page.getByRole('button', { name: 'Back up my identity' }).click();
    await page.getByRole('button', { name: 'Show my recovery key' }).click();
    const recovery = await page.locator('#kb-key').inputValue();
    if (!/^hoi-key-1-([a-z0-9]{8}-){6}[a-z0-9]{8}$/.test(recovery)) throw new Error(`recovery key looks wrong: ${recovery}`);
    await page.getByRole('button', { name: 'Encrypted file', exact: true }).click();
    await page.locator('#kb-pass1').fill('correct horse battery staple');
    await page.locator('#kb-pass2').fill('correct horse battery staple');
    const [dl] = await Promise.all([page.waitForEvent('download'), page.getByRole('button', { name: 'Download .hoikey' }).click()]);
    if (!dl.suggestedFilename().endsWith('.hoikey')) throw new Error(`download named ${dl.suggestedFilename()}`);
    await toast(/Backup download started|Backup file saved/);
    await shot('13-you');
    // Friends who keep shares of the recovery line
    await page.getByRole('button', { name: 'Friends', exact: true }).click();
    await page.getByRole('heading', { name: 'Your friends' }).waitFor();
    await page.getByRole('heading', { name: 'Help a friend' }).waitFor();
    await page.keyboard.press('Escape');
  });

  await step('search, saved, person', async () => {
    await go('#/search?q=relay');
    await page.getByText(/matches? for “relay”/).waitFor();
    await page.getByRole('button', { name: 'Save', exact: true }).first().click();
    await toast(/Saved on this browser/);
    await shot('14-search');
    await go('#/saved');
    await page.getByRole('heading', { name: 'Saved' }).waitFor();
    await page.locator('.row').first().waitFor();
    await page.locator('.row a.author').first().click();
    await page.getByText('Short key').waitFor();
    await shot('15-person');
  });

  await step('blocked by another tab', async () => {
    const p2 = await ctx.newPage();
    watch(p2, combo.tag + ' (blocked tab)');
    await p2.goto(`${base}/?blocked`);
    await p2.getByText('hoi is open in another tab.').waitFor();
    await shot('16-blocked', p2);
    await p2.getByRole('button', { name: 'Use it here' }).click();
    await p2.getByText('Pick a name, get a key.').waitFor();
    await p2.close();
  });

  await step('join from an invite during first run', async () => {
    if (!inviteWeb) throw new Error('no invite link captured');
    const ctx2 = await browser.newContext(ctxOpts);
    await consentAccepted(ctx2);
    const p3 = await ctx2.newPage();
    watch(p3, combo.tag + ' (invitee)');
    await p3.goto(inviteWeb);
    await p3.getByText("You're opening an invite").waitFor();
    await p3.getByText('c/bikes-repair').first().waitFor();
    await p3.locator('input[placeholder="lena"]:not(#fr-restore-name)').fill('tobi');
    await shot('17-invite-first-run', p3);
    await p3.getByRole('button', { name: /Make my key & see/ }).click();
    await p3.getByRole('button', { name: 'Skip', exact: true }).click();
    await p3.getByRole('button', { name: 'Meet your key →' }).click({ timeout: 20_000 });
    await p3.getByRole('button', { name: 'See c/bikes-repair →' }).click();
    await p3.getByRole('button', { name: 'Join c/bikes-repair' }).waitFor();
    await shot('18-join-preview', p3);
    await p3.getByRole('button', { name: 'Join c/bikes-repair' }).click();
    await p3.getByRole('dialog', { name: 'Turn on your VPN before joining' }).getByRole('button', { name: 'Continue joining' }).click();
    await p3.getByRole('navigation', { name: 'Community sections' }).waitFor();
    await p3.getByRole('button', { name: '✓ Joined' }).waitFor();
    await shot('19-joined', p3);
    await ctx2.close();
  });

  await ctx.close();
  console.log(`${combo.tag}: done in ${((Date.now() - t0) / 1000).toFixed(1)} s`);
}

// ------------------------------------------------------------------ 320 px: no horizontal scroll anywhere
if (only.length === 0 || only.includes('narrow')) {
  const ctx = await browser.newContext({ viewport: { width: 320, height: 640 }, isMobile: true, hasTouch: true });
  await consentAccepted(ctx);
  const page = await ctx.newPage();
  watch(page, 'narrow-320');
  try {
    await page.goto(`${base}/?me=lena#/communities`);
    await page.locator('main .crow').first().waitFor();
    const privacy = await page.locator('main .crow a.cbody, main a.crow', { hasText: 'c/privacy' }).first().getAttribute('href');
    await page.evaluate(() => (location.hash = '#/'));
    await page.locator('.row a.title').first().waitFor();
    const post = await page.locator('.row a.title').first().getAttribute('href');
    const hashes = ['#/', '#/communities', privacy, `${privacy}/rules`, `${privacy}/manage`, `${privacy}/ledger`, post, '#/you', '#/new', '#/join', '#/saved', '#/search?q=relay'];
    for (const h of hashes) {
      await page.evaluate((x) => (location.hash = x), h);
      await page.waitForTimeout(900);
      const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
      if (over > 0) {
        problems.push(`narrow-320 ${h}: horizontal overflow of ${over}px`);
        await page.screenshot({ path: join(out, `narrow-320-overflow-${h.replace(/\W+/g, '-')}.png`), fullPage: true });
      }
    }
    await page.evaluate(() => (location.hash = '#/'));
    await page.waitForTimeout(600);
    await page.screenshot({ path: join(out, 'narrow-320-home.png') });
  } catch (e) {
    problems.push(`narrow-320 failed: ${e.message.split('\n')[0]}`);
  }
  await ctx.close();
  console.log('narrow-320: done');
}

// ------------------------------------------------------------------ reduced motion: complete at rest, nothing stuck hidden
if (only.length === 0 || only.includes('reduced')) {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 }, reducedMotion: 'reduce' });
  await consentAccepted(ctx);
  const page = await ctx.newPage();
  watch(page, 'reduced-motion');
  try {
    await page.goto(`${base}/?me=lena#/`);
    await page.locator('.row').first().waitFor();
    const hidden = await page.evaluate(() =>
      [...document.querySelectorAll('.anim, .row, .vh')].filter((el) => Number(getComputedStyle(el).opacity) < 0.99).length,
    );
    if (hidden) problems.push(`reduced-motion: ${hidden} elements not fully visible at rest`);
    // Sending needs a connected community, as in the main flow.
    await page.evaluate(async () => {
      const { app } = await import('/src/lib/app.svelte.ts');
      const status = { ...(await app.core.net()), peers: 1, communities: app.joined.map(c => ({ community: c.id, connectedPeers: 1 })) };
      app.core.net = async () => status;
      app.net = status;
    });
    await page.locator('header .newpost').click();
    if (!(await page.evaluate(() => document.activeElement?.id === 'ctitle'))) throw new Error('the title field is not focused after + Post');
    await page.locator('#ctitle').fill('Posted with reduced motion');
    await page.getByRole('button', { name: 'Sign & send' }).click();
    const notice = page.getByRole('dialog', { name: 'Nobody can read posts from this browser' });
    if (await notice.waitFor({ timeout: 5_000 }).then(() => true, () => false)) await notice.getByRole('button', { name: 'Post through them' }).click();
    await page.locator('.toast', { hasText: /Signed/ }).waitFor({ timeout: 10_000 });
    await page.locator('.row', { hasText: 'Posted with reduced motion' }).waitFor();
    await page.screenshot({ path: join(out, 'reduced-motion-home.png') });
  } catch (e) {
    problems.push(`reduced-motion failed: ${e.message.split('\n')[0]}`);
  }
  await ctx.close();
  console.log('reduced-motion: done');
}

await browser.close();
server?.kill();

if (problems.length) {
  console.log(`\n${problems.length} problem(s):`);
  for (const p of problems) console.log(' - ' + p);
  process.exit(1);
}
console.log('\nall flows passed: no console errors, no horizontal overflow');

function watch(page, tag) {
  page.on('console', (m) => {
    if (m.type() === 'error') problems.push(`${tag} console error: ${m.text()}`);
  });
  page.on('pageerror', (e) => problems.push(`${tag} page error: ${e.message}`));
}
