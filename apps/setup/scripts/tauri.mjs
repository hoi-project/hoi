// Run the Tauri CLI for the setup app. Its window talks to nothing but its own IPC
// (tauri.conf.json), so unlike the desktop app no registry origin is merged into the CSP.
// A preview version is built for Windows without the .msi, which takes numeric versions only.
import { readFileSync } from 'node:fs';
import { run } from '@tauri-apps/cli';
import { windowsBundles } from '../../../scripts/tauri-bundles.mjs';

const { version } = JSON.parse(readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
const { args, note } = windowsBundles(process.argv.slice(2), version);
if (note) console.error(note);
await run(args, 'pnpm tauri');
