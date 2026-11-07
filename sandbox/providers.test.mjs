import test from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { ProviderBroker, decisionContent } from './providers.mjs';

const providers = { codex: { available: true }, claude: { available: true } };
const request = { driver: 'codex', model: '', messages: [{ role: 'system', content: 'Choose an action.' }, { role: 'user', content: 'A small community.' }] };

test('CLI mailbox limits global concurrency, binds completions to workers, and validates JSON', async t => {
  const broker = new ProviderBroker(); t.after(() => broker.close());
  const workerId = randomUUID(); broker.take({ workerId, providers, capacity: 2 });
  const first = broker.request(request), second = broker.request(request), third = broker.request(request);
  const a = broker.take({ workerId, providers, capacity: 2 }).job;
  const b = broker.take({ workerId, providers, capacity: 1 }).job;
  assert.equal(broker.take({ workerId: randomUUID(), providers, capacity: 2 }).job, null);
  assert.throws(() => broker.complete({ workerId: randomUUID(), id: a.id, content: '{"action":"observe"}' }), /different worker/);
  broker.complete({ workerId, id: a.id, content: '{"action":"observe"}' });
  assert.equal(await first, '{"action":"observe"}');
  const c = broker.take({ workerId, providers, capacity: 1 }).job;
  broker.complete({ workerId, id: b.id, content: '{"action":"comment","text":"My opinion"}' });
  assert.equal(JSON.parse(await second).text, 'My opinion');
  const invalid = assert.rejects(third, /JSON action/);
  broker.complete({ workerId, id: c.id, content: '{"command":"execute"}' });
  await invalid;
  assert.equal(broker.status().running, 0);
});

test('cancelled and timed out jobs cannot publish late decisions', async t => {
  const broker = new ProviderBroker({ timeoutMs: 20 }); t.after(() => broker.close());
  const workerId = randomUUID(); broker.take({ workerId, providers, capacity: 2 });
  const controller = new AbortController();
  const pending = broker.request(request, controller.signal);
  const job = broker.take({ workerId, providers, capacity: 2 }).job;
  const cancelled = assert.rejects(pending, { name: 'AbortError' }); controller.abort(); await cancelled;
  assert.deepEqual(broker.take({ workerId, providers, capacity: 0 }).activeIds, []);
  assert.equal(broker.complete({ workerId, id: job.id, content: '{"action":"post"}' }).discarded, true);
  await assert.rejects(broker.request(request), /timed out/);
  assert.equal(broker.jobs.size, 0);
});

test('offline or missing CLIs fail visibly, without fallback; stale running jobs are rejected', async t => {
  let now = 0;
  const broker = new ProviderBroker({ now: () => now }); t.after(() => broker.close());
  await assert.rejects(broker.request(request), /offline/);
  const workerId = randomUUID(); broker.take({ workerId, providers, capacity: 2 });
  const pending = broker.request(request); broker.take({ workerId, providers, capacity: 2 });
  now = 16000;
  assert.equal(broker.status().online, false);
  const disconnected = assert.rejects(pending, /disconnected/);
  broker.take({ workerId: randomUUID(), providers, capacity: 2 }); await disconnected;
});

test('mailbox rejects oversized context, unsafe models and malformed decisions', async t => {
  const broker = new ProviderBroker(); t.after(() => broker.close());
  broker.take({ workerId: randomUUID(), providers, capacity: 2 });
  await assert.rejects(broker.request({ ...request, model: 'bad\nmodel' }), /model name/);
  await assert.rejects(broker.request({ ...request, messages: [{ role: 'user', content: 'x'.repeat(100001) }] }), /context/);
  assert.throws(() => decisionContent('x'.repeat(20000)), /bounded/);
  assert.throws(() => decisionContent('[]'), /object/);
  assert.equal(decisionContent('```json\n{"action":"observe"}\n```'), '{"action":"observe"}');
});
