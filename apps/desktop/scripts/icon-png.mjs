// Renders an icons folder's icon.svg to a 1024px source.png; `tauri icon` then makes every platform
// size. The folder is this app's src-tauri/icons, or the one given (the node setup app passes its own).
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const dir = process.argv[2] ? resolve(process.argv[2]) : resolve(dirname(fileURLToPath(import.meta.url)), '../src-tauri/icons');
const svg = readFileSync(resolve(dir, 'icon.svg'), 'utf8');
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
const page = await browser.newPage({ viewport: { width: 1024, height: 1024 } });
await page.setContent(`<html><body style="margin:0;background:transparent">${svg}</body></html>`);
writeFileSync(resolve(dir, 'source.png'), await page.locator('svg').screenshot({ omitBackground: true }));
await browser.close();
console.log(`wrote ${relative(process.cwd(), resolve(dir, 'source.png'))}`);
