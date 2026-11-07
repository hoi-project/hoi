// Runs the pure core crates' unit tests (hoi-crypto, hoi-types) and the protocol conformance
// vectors built for wasm32-wasip1 under Node's WASI, so the byte formats and the exact arithmetic
// are checked in a wasm build too, not only natively. Needs: rustup target add wasm32-wasip1.
//
//     node scripts/test-core-wasi.mjs
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { WASI } from 'node:wasi';

const core = fileURLToPath(new URL('../core/', import.meta.url)).replace(/\/$/, '');
const builds = [
  ['-p', 'hoi-crypto', '-p', 'hoi-types', '--lib'],
  ['-p', 'hoi-validate', '--test', 'vectors'],
];

const executables = [];
for (const args of builds) {
  const r = spawnSync('cargo', ['test', '--locked', '--target', 'wasm32-wasip1', '--no-run', '--message-format=json-render-diagnostics', ...args], {
    cwd: core,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'inherit'],
    maxBuffer: 64 << 20,
  });
  if (r.status !== 0) process.exit(r.status ?? 1);
  for (const line of r.stdout.split('\n')) {
    if (!line.startsWith('{')) continue;
    const m = JSON.parse(line);
    if (m.reason === 'compiler-artifact' && m.executable && m.profile?.test) executables.push(m.executable);
  }
}

let failed = 0;
for (const file of executables) {
  console.log(`\n== ${file.slice(core.length + 1)}`);
  const wasi = new WASI({
    version: 'preview1',
    // the generator check spawns cargo, which WASI cannot
    args: ['test', '--skip', 'committed_vectors_are_current'],
    env: { RUST_TEST_THREADS: '1' },
    // tests find core/vectors through absolute paths compiled in
    preopens: { [core]: core },
    returnOnExit: true,
  });
  const instance = await WebAssembly.instantiate(await WebAssembly.compile(readFileSync(file)), wasi.getImportObject());
  if (wasi.start(instance) !== 0) failed++;
}
if (failed) {
  console.error(`\n${failed} wasm test binar${failed === 1 ? 'y' : 'ies'} failed`);
  process.exit(1);
}
