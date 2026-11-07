import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { BATCH_BYTES, options, purgeValue } from './operator.mjs';

const room = 'ab'.repeat(32);
const id = 'cd'.repeat(32);

test('removal batches are validated identifiers within one secret', () => {
  assert.equal(purgeValue([room, `${room}/${id}`, room, 'v1']), `${room},${room}/${id},v1`);
  for (const bad of [[], ['AB'.repeat(32)], [`${room}/`], ['../objects'], [`${room};DROP`]]) assert.throws(() => purgeValue(bad));
  const many = Array.from({ length: 64 }, (_, i) => i.toString(16).padStart(2, '0').repeat(32));
  assert.throws(() => purgeValue(many), new RegExp(String(BATCH_BYTES)));
  assert.ok(purgeValue(many.slice(0, 60)).length <= BATCH_BYTES);
});

test('options name one command, a config, an optional HTTPS check and nothing else', () => {
  assert.deepEqual(options(['remove', room, '--config', 'x.toml', '--check', 'https://relay.example/relay/']),
    { command: 'remove', config: 'x.toml', check: 'https://relay.example/relay', dryRun: false, entries: [room] });
  assert.equal(options(['remove', '--check', 'http://127.0.0.1:8787/relay', room]).check, 'http://127.0.0.1:8787/relay');
  for (const argv of [['delete', room], ['remove', '--check', 'http://relay.example/relay', room], ['remove', '--force', room], ['remove', '--config']]) assert.throws(() => options(argv));
});

test('a dry run prints the Wrangler command and never runs it', () => {
  const output = execFileSync(process.execPath, [fileURLToPath(new URL('./operator.mjs', import.meta.url)), 'remove', '--dry-run', `${room}/${id}`], { encoding: 'utf8' });
  assert.match(output, /pnpm dlx wrangler@4 secret put HOI_RELAY_PURGE --config relay\/wrangler\.toml/);
  assert.match(output, new RegExp(`${room}/${id}`));
});
