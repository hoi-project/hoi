// Builds hoi-wasm and generates the JS bindings the web app imports
// (apps/web/src/lib/core/wasm/pkg). Needs: rustup target wasm32-unknown-unknown and
// wasm-bindgen-cli at the same version as the wasm-bindgen crate (see core/Cargo.toml).
import { execFileSync } from 'node:child_process';
import { existsSync, statSync, readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { privateBuildPaths } from './private-build-paths.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const core = resolve(root, 'core');
const out = resolve(root, 'apps/web/src/lib/core/wasm/pkg');
const env = privateBuildPaths(root, process.env);
const run = (cmd, args, opts = {}) => execFileSync(cmd, args, { stdio: 'inherit', cwd: core, env, ...opts });

run('cargo', ['build', '--locked', '-p', 'hoi-wasm', '--target', 'wasm32-unknown-unknown', '--profile', 'wasm']);
const wasm = resolve(env.CARGO_TARGET_DIR ? resolve(core, env.CARGO_TARGET_DIR) : resolve(core, 'target'), 'wasm32-unknown-unknown/wasm/hoi_wasm.wasm');
run('wasm-bindgen', ['--target', 'web', '--out-dir', out, '--out-name', 'hoi_wasm', wasm]);
const opt = resolve(out, 'hoi_wasm_bg.wasm');
// wasm-opt from the binaryen npm package (or a system install): about 40% smaller, same speed.
const local = resolve(root, 'node_modules/.bin/wasm-opt');
const wasmOpt = existsSync(local) ? local : 'wasm-opt';
try {
  execFileSync(wasmOpt, ['--version'], { stdio: 'ignore' });
  const features = ['--enable-bulk-memory', '--enable-nontrapping-float-to-int', '--enable-sign-ext', '--enable-mutable-globals', '--enable-reference-types', '--enable-multivalue'];
  run(wasmOpt, ['-O3', ...features, '-o', opt, opt]);
} catch {
  console.log('wasm-opt not found; skipping (pnpm install brings binaryen)');
}
if (existsSync(opt)) {
  const bytes = readFileSync(opt);
  for (const path of new Set([root, homedir(), root.replaceAll('\\', '/'), homedir().replaceAll('\\', '/')])) {
    if (bytes.includes(Buffer.from(path))) throw new Error('The browser binary contains a private build path. Do not deploy it.');
  }
  console.log(`hoi_wasm_bg.wasm: ${(statSync(opt).size / 1024).toFixed(0)} KB`);
}
