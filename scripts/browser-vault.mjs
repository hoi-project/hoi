// Exercise the public unlock form, including on production builds without module imports.
import { CONSENT_KEY, NETWORK_NOTICE_VERSION } from '../apps/web/src/lib/legal/version.mjs';

export const TEST_PASSPHRASE = 'browser tests use this local passphrase';

/** The Network & Safety notice comes before anything else: both boxes, then Continue.
 * Resolves at once when it was acknowledged in this context already. */
export async function acceptNetworkNotice(page) {
  const stored = await page.evaluate((key) => { try { return localStorage.getItem(key); } catch { return null; } }, CONSENT_KEY).catch(() => null);
  if (stored && JSON.parse(stored)?.version === NETWORK_NOTICE_VERSION) return false;
  const ip = page.locator('#safety-ack-ip');
  try {
    await ip.waitFor({ state: 'visible', timeout: 5000 });
  } catch {
    return false;
  }
  await ip.check();
  await page.locator('#safety-ack-vpn').check();
  await page.getByRole('button', { name: 'Continue', exact: true }).click();
  await ip.waitFor({ state: 'detached', timeout: 10_000 });
  return true;
}

/** For a context that should start past the notice: what a person who read it has in localStorage. */
export const CONSENT_INIT = { version: NETWORK_NOTICE_VERSION, date: '2026-11-03' };
export async function consentAccepted(context) {
  await context.addInitScript(({ key, record }) => {
    try { localStorage.setItem(key, JSON.stringify(record)); } catch { /* storage off */ }
  }, { key: CONSENT_KEY, record: CONSENT_INIT });
}

export async function unlockBrowser(page) {
  await acceptNetworkNotice(page);
  await page.waitForFunction(() => !document.body.innerText.includes('Waking up your'), null, { timeout: 30_000 });
  const password = page.locator('#vault-passphrase');
  if (!await password.count()) return;
  await password.fill(TEST_PASSPHRASE);
  if (await page.locator('#vault-confirm').count()) await page.locator('#vault-confirm').fill(TEST_PASSPHRASE);
  await page.getByRole('button', { name: /^(Protect and continue|Unlock)$/ }).click();
  await password.waitFor({ state: 'detached', timeout: 60_000 });
}
