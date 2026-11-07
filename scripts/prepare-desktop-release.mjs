// Only writes the build configuration; never signs, publishes or installs a release.
import { readFileSync, writeFileSync } from 'node:fs';
import { loadPublication } from './publication.mjs';
const publication = loadPublication(true);
if (!process.env.TAURI_SIGNING_PRIVATE_KEY) throw new Error('Missing TAURI_SIGNING_PRIVATE_KEY for updater artifacts.');
const out = process.argv[2];
if (!out) throw new Error('Pass an output filename.');
const previous = process.env.TAURI_SIGNING_ARGS?.replace(/^--config /, '');
const config = previous ? JSON.parse(readFileSync(previous, 'utf8')) : {};
config.bundle = { ...config.bundle, createUpdaterArtifacts: true };
config.plugins = { ...config.plugins, updater: { pubkey: Buffer.from(`untrusted comment: hoi updater public key\n${publication.updaterKey}\n`).toString('base64') } };
writeFileSync(out, JSON.stringify(config));
