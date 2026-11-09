// Run the Tauri CLI for the setup app. Its window talks to nothing but its own IPC
// (tauri.conf.json), so unlike the desktop app no registry origin is merged into the CSP.
// A preview version is built for Windows without the .msi, which takes numeric versions only.
import { readFileSync } from 'node:fs';
import { run } from '@tauri-apps/cli';
import { linuxBundles, windowsBundles } from '../../../scripts/tauri-bundles.mjs';
import { clearEmptyAppleSigningEnv } from '../../../scripts/tauri-signing.mjs';

const { version } = JSON.parse(readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
const windows = windowsBundles(process.argv.slice(2), version);
const linux = linuxBundles(windows.args, version);
for (const note of [windows.note, linux.note]) if (note) console.error(note);
clearEmptyAppleSigningEnv();
await run(linux.args, 'pnpm tauri');
