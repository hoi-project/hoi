import test from 'node:test';
import assert from 'node:assert/strict';
import { relayUrls } from './relay-config.mjs';

test('relay destinations are bounded explicit HTTPS origins, with local development exceptions', () => {
  assert.deepEqual(relayUrls(undefined, ['/relay']), ['/relay']);
  assert.deepEqual(relayUrls('["https://one.example/relay/","https://two.example/relay"]'), ['https://one.example/relay', 'https://two.example/relay']);
  assert.deepEqual(relayUrls('["http://127.0.0.1:8787/relay"]'), ['http://127.0.0.1:8787/relay']);
  for (const values of [['http://public.example'], ['https://user:secret@example.org'], ['https://example.org?token=x'], ['//example.org/relay'], ['/other'], Array(4).fill('https://example.org')]) {
    assert.throws(() => relayUrls(JSON.stringify(values)));
  }
});
