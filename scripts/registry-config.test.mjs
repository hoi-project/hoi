import test from 'node:test';
import assert from 'node:assert/strict';
import { registryOverride } from '../apps/desktop/scripts/tauri.mjs';

test('desktop allows only the configured registry origin and preserves existing policy', () => {
  const base = { app: { security: { csp: { 'connect-src': "'self' ipc: ws: wss:", 'script-src': "'self'" } } } };
  const result = registryOverride(base, 'https://registry.example/hoi');
  assert.equal(result.app.security.csp['connect-src'], "'self' ipc: ws: wss: https://registry.example");
  assert.equal(result.app.security.csp['script-src'], "'self'");
  assert.equal(base.app.security.csp['connect-src'], "'self' ipc: ws: wss:");
  assert.equal(registryOverride(base, ''), null);
  assert.throws(() => registryOverride(base, 'http://registry.example'), /HTTPS/);
  assert.throws(() => registryOverride(base, 'https://user:secret@registry.example'), /credentials/);
});
