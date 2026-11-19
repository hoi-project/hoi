// Exercise the real consent dialog with simultaneous and revised requests.
import assert from 'node:assert/strict';
import { chromium } from 'playwright';
import { startComponentServer } from './e2e-component-server.mjs';

process.env.VITE_HOI_BACKEND = 'mock';
const { server, origin } = await startComponentServer('/operator-notice-test', `<!doctype html><html lang="en">
  <title>Operator notice review</title><main></main><script type="module">
    import { mount } from 'svelte';
    import OperatorNotice from '/src/lib/ui/OperatorNotice.svelte';
    import { app } from '/src/lib/app.svelte.ts';
    await app.start();
    app.refresh = async () => {};
    const fixture = window.notices = { decisions: [], pending: [],
      set(pending) { this.pending = pending; app.net = { ...app.net, pendingNotices: pending }; }
    };
    app.core.acceptNotice = async (node, hash) => {
      fixture.decisions.push([node, hash]);
      fixture.set(fixture.pending.filter(n => n.node !== node));
    };
    app.core.declineNotice = async node => {
      fixture.decisions.push([node, 'declined']);
      fixture.set(fixture.pending.filter(n => n.node !== node));
    };
    app.core.net = async () => app.net;
    mount(OperatorNotice, { target:document.querySelector('main') });
  </script></html>`);

const notice = (node, revision) => ({
  node, hash: revision, title: `Notice from ${node}`,
  sections: [{ title: 'Configuration', text: `Details for ${revision}` }], footer: revision,
});
const first = notice('first holder', 'first revision');
const replacement = notice(first.node, 'updated revision');
const second = notice('second holder', 'second revision');
let browser;
try {
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  const page = await browser.newPage();
  await page.goto(origin + '/operator-notice-test', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => !!window.notices);
  const set = notices => page.evaluate(list => window.notices.set(list), notices);
  const dialog = page.getByRole('dialog', { name: 'Before this node serves you' });
  const accept = dialog.getByRole('button', { name: 'Accept and join' });
  const decline = dialog.getByRole('button', { name: 'Decline' });
  const decisions = () => page.evaluate(() => window.notices.decisions);

  await set([first]);
  await dialog.getByRole('heading', { name: first.title }).waitFor();
  await accept.focus();
  await set([second, first]);
  assert.equal(await dialog.getByRole('article').getAttribute('aria-label'), first.title);
  await page.keyboard.press('Enter');
  await dialog.getByRole('heading', { name: second.title }).waitFor();
  assert.deepEqual(await decisions(), [[first.node, first.hash]]);
  await decline.click();
  await dialog.waitFor({ state: 'hidden' });

  await set([first, second]);
  await dialog.getByRole('heading', { name: first.title }).waitFor();
  await set([second, replacement]);
  await dialog.getByRole('alert').waitFor();
  assert.ok((await dialog.getByRole('article').innerText()).includes(first.hash));
  assert.ok(await accept.isDisabled());
  assert.ok(await decline.isDisabled());
  await accept.focus();
  await page.keyboard.press('Enter');
  assert.equal((await decisions()).length, 2, 'A stale declaration cannot be accepted with the keyboard');
  await dialog.getByRole('button', { name: 'Review updated notice' }).click();
  await dialog.getByText('Details for updated revision', { exact: false }).waitFor();
  assert.ok(await dialog.getByRole('heading', { name: first.title }).evaluate(el => el === document.activeElement));
  await accept.click();
  await dialog.getByRole('heading', { name: second.title }).waitFor();
  assert.deepEqual(await decisions(), [[first.node, first.hash], [second.node, 'declined'], [first.node, replacement.hash]]);

  await set([first]); // The authenticated core withdrew the displayed holder.
  await dialog.getByRole('heading', { name: first.title }).waitFor();
  await set([]);
  await dialog.waitFor({ state: 'hidden' });
  assert.equal((await decisions()).length, 3, 'Withdrawing a notice cannot accept or decline it');
  console.log('PASS: reordered arrivals, revised declarations, withdrawal and revision-bound keyboard acceptance.');
} finally {
  await browser?.close();
  await server.close();
}
