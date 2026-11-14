import test from 'node:test';
import assert from 'node:assert/strict';
import { publicIp } from './protocol.mjs';

test('IPv6 forms that carry a private or loopback IPv4 address are not public', () => {
  for (const hidden of [
    '::ffff:10.0.0.1', '::10.0.0.1', '::127.0.0.1', '::c0a8:101',
    '64:ff9b::10.0.0.1', '64:ff9b::7f00:1', '64:ff9b::c0a8:101', '64:ff9b:1::1',
    '2002:a00:1::1', '2002:7f00:1::', '2002:c0a8:101::1', 'fec0::1',
  ]) {
    assert.equal(publicIp(hidden), false, hidden);
  }
});

test('the same forms around a global IPv4 address, and plain global addresses, stay public', () => {
  for (const global of ['93.184.216.34', '::ffff:93.184.216.34', '::93.184.216.34', '64:ff9b::93.184.216.34', '2002:5db8:d822::1', '2606:2800:220:1:248:1893:25c8:1946', '2001:db8::1']) {
    assert.equal(publicIp(global), true, global);
  }
});
