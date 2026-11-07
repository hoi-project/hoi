import { homedir } from 'node:os';
import { resolve } from 'node:path';

/** Rust panic locations can otherwise embed the builder's personal directory in public WASM. */
export function privateBuildPaths(root, env = process.env, home = homedir()) {
  const flags = env.CARGO_ENCODED_RUSTFLAGS
    ? env.CARGO_ENCODED_RUSTFLAGS.split('\x1f')
    : (env.RUSTFLAGS || '').trim().split(/\s+/).filter(Boolean);
  const prefixes = new Map([
    [home, '/build-user'], [env.USERPROFILE, '/build-user'],
    [env.CARGO_HOME || resolve(home, '.cargo'), '/cargo'],
    [env.RUSTUP_HOME || resolve(home, '.rustup'), '/rustup'],
    [root, '/hoi'],
  ]);
  for (const [source, target] of prefixes) if (source) {
    for (const path of new Set([source, source.replaceAll('\\', '/')])) flags.push(`--remap-path-prefix=${path}=${target}`);
  }
  return { ...env, CARGO_ENCODED_RUSTFLAGS: flags.join('\x1f') };
}
