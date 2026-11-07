import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { test } from 'node:test';

function sources(directory, extension) {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    if (['node_modules', 'dist', 'dist-desktop'].includes(entry.name)) return [];
    const path = resolve(directory, entry.name);
    return entry.isDirectory() ? sources(path, extension) : entry.name.endsWith(extension) ? [path] : [];
  });
}

test('public pages contain no email addresses or deployment-configuration hints', () => {
  const root = resolve(import.meta.dirname, '..');
  const paths = [...sources(resolve(root, 'apps/site'), '.html'), ...sources(resolve(root, 'registry/public'), '.html'), ...sources(resolve(root, 'apps/web/src'), '.svelte')];
  assert.ok(paths.length > 20);
  for (const path of paths) {
    // Test visible/template copy, not implementation identifiers inside scripts or comments.
    const source = readFileSync(path, 'utf8').replace(/<script\b[^>]*>[\s\S]*?<\/script>/gi, '').replace(/<!--[\s\S]*?-->/g, '');
    assert.doesNotMatch(source, /[\w.+-]+@[\w-]+(?:\.[\w-]+)*\.[a-z]{2,}\b/i, `${path}: email address`);
    assert.doesNotMatch(source, /VITE_HOI_|HOI_REGISTRY_|No registry is configured|Set\s+\w+_URL|this build creates unregistered/i, `${path}: deployment hint`);
  }
});
