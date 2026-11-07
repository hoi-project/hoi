// Rasterise public/icon.svg into the PWA's PNG icons with Playwright's Chromium.
// Run: node scripts/icons.mjs   (needs the root devDependency `playwright`; never runs `playwright install`)
import { chromium } from 'playwright';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const pub = join(here, '..', 'public');
const svg = readFileSync(join(pub, 'icon.svg'), 'utf8');
// Maskable: full-bleed ground, mark already inside the 80% safe circle.
const maskable = svg.replace(/<rect width="512" height="512" rx="112"/, '<rect width="512" height="512"');

const jobs = [
  { file: 'icon-192.png', size: 192, src: svg },
  { file: 'icon-512.png', size: 512, src: svg },
  { file: 'icon-maskable-512.png', size: 512, src: maskable },
];

const browser = await chromium.launch();
const page = await browser.newPage();
for (const j of jobs) {
  await page.setViewportSize({ width: j.size, height: j.size });
  const data = 'data:image/svg+xml;base64,' + Buffer.from(j.src).toString('base64');
  await page.setContent(`<html><body style="margin:0;background:transparent"><img src="${data}" width="${j.size}" height="${j.size}" style="display:block"></body></html>`);
  await page.waitForFunction(() => document.images[0]?.complete);
  writeFileSync(join(pub, j.file), await page.screenshot({ omitBackground: true, clip: { x: 0, y: 0, width: j.size, height: j.size } }));
  console.log('wrote', j.file);
}
await browser.close();
