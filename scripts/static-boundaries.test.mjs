import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const read = path => readFileSync(new URL('../' + path, import.meta.url), 'utf8');

test('static deployment cannot start a registry, relay or peer', () => {
  const workflow = read('.github/workflows/website.yml').replace(/^\s*#.*$/gm, '');
  assert.match(workflow, /upload-pages-artifact@[0-9a-f]{40}[^\n]*\n\s*with:\n\s*path: dist\n/);
  assert.doesNotMatch(workflow, /wrangler|docker|hoi-node|ssh\b|deploy --config (?:registry|relay)\//);
  const assembly = read('scripts/assemble-static.mjs');
  assert.doesNotMatch(assembly, /assembleRegistrySite|registryCsp/);
});

test('the deployed site is built with nothing configured, and nothing names a node or registry', () => {
  // The Pages build pins both service settings empty: no repository variable can add one.
  const workflow = read('.github/workflows/website.yml');
  assert.match(workflow, /VITE_HOI_REGISTRY_URL: ""\n\s*VITE_HOI_RELAY_URLS: "\[\]"\n\s*run: node scripts\/reproduce\.mjs --only static/);
  assert.doesNotMatch(workflow, /vars\.VITE_HOI/);
  // The app's own defaults are empty.
  const vite = read('apps/web/vite.config.ts');
  assert.match(vite, /VITE_HOI_RELAY_URLS'\)\.VITE_HOI_RELAY_URLS,\s*\[\]\)/);
  assert.match(vite, /VITE_HOI_REGISTRY_URL'\)\.VITE_HOI_REGISTRY_URL\s*\?\? ''/);
  assert.match(read('apps/web/src/lib/registry.ts'), /VITE_HOI_REGISTRY_URL \?\? ''\)/);
  // A node starts with no registry, no relay and no node to look at: it finds others by itself.
  const node = read('core/crates/hoi-node/src/main.rs');
  for (const field of ['relays', 'indexes', 'nodes', 'joins']) assert.match(node, new RegExp(`\\n\\s*${field}: Vec::new\\(\\),`), field);
  // iroh with none of its operator's DNS, pkarr or relay services, and only the relays given.
  const p2p = read('core/crates/hoi-net/src/p2p.rs');
  assert.match(p2p, /Endpoint::builder\(presets::Minimal\)/);
  assert.match(p2p, /relay_mode\(RelayMode::Custom\(RelayMap::from_iter\(relays\.clone\(\)\)\)\)/);
  // The deployments a person may run themselves name no registry either.
  for (const path of ['deploy/home/compose.yml', 'deploy/server/compose.yml', 'deploy/home/.env.example', 'deploy/server/.env.example', 'deploy/install/install.sh', 'deploy/install/install.ps1']) {
    assert.doesNotMatch(read(path), /HOI_INDEX_URL|--index"?\s*\n?\s*-?\s*"?https:\/\/(?!names\.example)/, path);
  }
});

test('marketing scripts contain no telemetry, browser persistence or background requests', () => {
  const common = read('apps/site/src/common.ts');
  assert.doesNotMatch(common, /\bfetch\(|sendBeacon|localStorage|sessionStorage|indexedDB|document\.cookie/);
  for (const partial of ['footer', 'nav-home', 'nav-site']) {
    assert.doesNotMatch(read(`apps/site/partials/${partial}.html`), /href="\/registry/);
  }
});
