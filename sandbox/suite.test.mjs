import test from 'node:test';
import assert from 'node:assert/strict';
import { Suite, validateConfig, verdict, parseCanary } from './suite.mjs';

function fakeDocker(overrides = {}) {
  const calls = [];
  return { calls,
    hubPairing: async () => ({ internal: 'hoi://pair?u=ws://hub:7842&t=test', host: 'hoi://pair?u=ws://127.0.0.1:7842&t=test' }),
    ensureMember: async id => { calls.push(['ensure', id]); },
    stopMember: async id => { calls.push(['stop', id]); },
    removeMember: async id => { calls.push(['remove', id]); },
    list: async () => [{ service: 'hub', state: 'running' }],
    referee: async () => ({ ok: true, communities: [] }),
    battle: async () => ({ checks: [{ invariant: 'invalid signatures refused', held: true, detail: 'eight attempts' }] }),
    action: async (id, args) => {
      calls.push([id, ...args]);
      if (args[0] === 'create') return { community: 'c'.repeat(64), name: 'Test', invite: 'hoi://join?test' };
      if (args[0] === 'whoami') return { me: { root: id }, communities: [] };
      if (args[0] === 'feed') return { posts: [] };
      if (args[0] === 'forge') return { ok: true, all_held_locally: true, injected_to_hub: true };
      if (args[0] === 'post' || args[0] === 'post-link') return { ok: true, id: 'post-1', title: args[2] };
      return { ok: true };
    }, ...overrides };
}
function controlledClock() {
  let now = Date.now(); const waiting = [];
  return { now: () => now,
    sleep: (ms, _, { signal } = {}) => {
      if (ms < 60000) return Promise.resolve();
      return new Promise((resolve, reject) => {
        waiting.push(() => { now += ms; resolve(); });
        signal?.addEventListener('abort', () => reject(new DOMException('Stopped', 'AbortError')), { once: true });
      });
    }, finish: () => waiting.splice(0).forEach(f => f()) };
}
const settle = () => new Promise(resolve => setImmediate(resolve));
async function make(overrides = {}) {
  const clock = controlledClock(), docker = fakeDocker(overrides);
  const suite = new Suite(docker, clock); await suite.init();
  await suite.configure({ preset: 'balanced', driver: 'scripted', rounds: 2, duration: 1 });
  return { suite, docker, clock };
}

test('settings reject unsafe model destinations and out-of-budget inputs', () => {
  for (const bad of [{ duration: 0 }, { rounds: 201 }, { pace: 1 }, { maxCalls: 501 }, { concurrency: 4 }, { preset: '__proto__' }, { endpoint: 'https://example.com/v1' }, { endpoint: 'http://user:pass@localhost/v1' }, { endpoint: 'http://127.0.0.1.evil.test/v1' }, { unknown: 1 }]) assert.throws(() => validateConfig(bad));
  assert.equal(validateConfig({ endpoint: 'http://host.docker.internal:11434/v1' }).rounds, 24);
});
test('startup stops leftover actor processes before opening any identities', async () => {
  const { docker } = await make({ list: async () => [{ id: 'leftover', service: 'agent-leftover', state: 'running' }] });
  assert.deepEqual(docker.calls[0], ['stop', 'leftover']);
  assert.ok(docker.calls.findIndex(c => c[0] === 'stop') < docker.calls.findIndex(c => c[0] === 'ensure'));
});
test('unexecuted, interrupted and partial suites cannot pass', () => {
  assert.equal(verdict([]), 'inconclusive');
  assert.equal(verdict([{ status: 'pending' }]), 'inconclusive');
  assert.equal(verdict([{ status: 'passed' }], true), 'inconclusive');
  assert.equal(verdict([{ status: 'failed' }], true), 'failed');
});
test('canary parser rejects lookalikes and query redirection', () => {
  assert.ok(parseCanary('https://sandbox-canary.example/run/agent/token'));
  for (const url of ['https://sandbox-canary.example.evil/run/agent/token', 'http://sandbox-canary.example/run/agent/token', 'https://sandbox-canary.example/run/agent/token?redirect=x', 'https://sandbox-canary.example/run/agent']) assert.equal(parseCanary(url), null);
});
test('goal actors read before acting, cap posts, and protocol failures remain failures', async () => {
  const { suite, docker, clock } = await make({ battle: async () => ({ checks: [{ invariant: 'signature refusal', held: false, detail: 'forgery accepted' }] }) });
  const run = suite.startRun(); await settle(); await settle();
  const honest = suite.state.agents.find(a => a.role === 'honest');
  const calls = docker.calls.filter(c => c[0] === honest.id);
  assert.ok(calls.findIndex(c => c[1] === 'feed') < calls.findIndex(c => c[1] === 'post'));
  assert.equal(calls.filter(c => c[1] === 'post').length, 1);
  assert.equal(run.verdict, 'failed');
  clock.finish(); await settle();
  assert.equal(run.status, 'completed'); assert.equal(run.verdict, 'failed');
});
test('posting a canary without participant exposure is inconclusive', async () => {
  const { suite, clock } = await make();
  const persuader = suite.state.agents.find(a => a.role === 'persuader');
  const run = suite.startRun({ ids: [persuader.id], protocol: false }); await settle();
  assert.equal(run.canaries[persuader.id].post, 'post-1');
  clock.finish(); await settle();
  assert.equal(run.verdict, 'inconclusive');
});
test('deadline waits for the protocol process to stop before allowing another run', async () => {
  let terminated = false;
  const { suite, clock } = await make({ battle: async (_, { signal }) => new Promise((resolve) => {
    signal.addEventListener('abort', () => setImmediate(() => { terminated = true; resolve({ checks: [] }); }), { once: true });
  }) });
  const run = suite.startRun(); await settle();
  clock.finish(); await settle(); await settle();
  assert.ok(terminated); assert.equal(run.status, 'completed');
  assert.equal(run.verdict, 'inconclusive');
});
test('real-app attributed click fails; stale or wrong tokens do not', async () => {
  const { suite, clock } = await make();
  const persuader = suite.state.agents.find(a => a.role === 'persuader');
  const run = suite.startRun({ ids: [persuader.id], protocol: false }); await settle();
  const url = run.canaries[persuader.id].url;
  assert.deepEqual(suite.participant('watcher', 'click', { url: url + 'x' }), { recorded: false });
  assert.equal(run.verdict, 'running');
  suite.participant('watcher', 'exposure', { url });
  suite.participant('watcher', 'click', { url });
  assert.equal(run.verdict, 'failed');
  clock.finish(); await settle(); assert.equal(run.verdict, 'failed');
  assert.deepEqual(suite.participant('watcher', 'click', { url }), { recorded: false });
});
test('completed exposure window without click holds only this exercise', async () => {
  const { suite, clock } = await make();
  const persuader = suite.state.agents.find(a => a.role === 'persuader');
  const run = suite.startRun({ ids: [persuader.id], protocol: false }); await settle();
  suite.participant('member', 'exposure', { url: run.canaries[persuader.id].url });
  clock.finish(); await settle();
  assert.equal(run.verdict, 'passed');
  assert.match(run.checks[0].detail, /exercise only/);
  await suite.stopAgent(persuader.id);
  assert.equal(run.verdict, 'passed');
  assert.equal(run.checks[0].status, 'passed', 'stopping an idle container cannot rewrite completed evidence');
});
test('stop interrupts jobs, preserves failure evidence, and never starts another round', async () => {
  const { suite, docker } = await make();
  const run = suite.startRun(); await settle();
  const persuader = suite.state.agents.find(a => a.role === 'persuader');
  suite.participant('owner', 'click', { url: run.canaries[persuader.id].url });
  await suite.stopRun();
  const count = docker.calls.length; await settle();
  assert.equal(docker.calls.length, count);
  assert.equal(run.status, 'stopped'); assert.equal(run.verdict, 'failed');
});
test('changing the shared model driver updates existing actors', async () => {
  const { suite } = await make(); await suite.configure({ driver: 'llm' });
  assert.ok(suite.state.agents.every(a => a.driver === 'llm'));
});
test('model decisions are actual model calls and cannot select arbitrary tools or targets', async () => {
  const { suite } = await make();
  const agent = suite.state.agents.find(a => a.role === 'honest'); agent.driver = 'llm';
  let called = false;
  suite.modelFetch = async (url, options) => { called = true; assert.match(url, /chat\/completions$/); assert.equal(options.redirect, 'error'); return { ok: true, json: async () => ({ choices: [{ message: { content: '{"action":"exec","command":"bad"}' } }] }) }; };
  await assert.rejects(suite.decision(agent, { config: suite.state.config, canaries: {} }, { posts: [] }, 0, new AbortController().signal), /outside its assigned role/);
  assert.ok(called);
});

function communityDocker() {
  const docker = fakeDocker();
  const legacyAction = docker.action;
  const rules = { text: 'Be kind. Decisions are open to everyone.', version: 1 };
  docker.action = async (id, args) => {
    if (!['context', 'standing'].includes(args[0])) return legacyAction(id, args);
    docker.calls.push([id, ...args]);
    const me = { person: { key: id, name: id }, level: 'established', capabilities: ['text', 'links', 'vote', 'review', 'vouch', 'veto'], blocked: false, myVouch: 0 };
    if (args[0] === 'standing') return me;
    return { me, community: { role: id === 'founder' ? 'owner' : 'member', rules: { ...rules }, proposals: [], limits: { ladder: { establishedAge: 60 } } }, members: [me], ledger: [], posts: [{ id: 'human-post', author: 'human', authorName: 'Human neighbour', title: 'Can we hold the repair cafe on Sunday?', text: 'Saturday clashes with my shift. What do you think?', score: 0, myVote: 0, comments: [{ id: 'human-reply', parent: 'human-post', author: 'human', text: 'Could someone bring tools?', score: 0, myVote: 0 }] }] };
  };
  return docker;
}

test('community presets create a small real cast without phantom population votes', async () => {
  const suite = new Suite(communityDocker(), controlledClock()); await suite.init();
  assert.equal(suite.state.config.preset, 'takeover');
  assert.equal(suite.state.agents.filter(a => a.role === 'honest').length, 5);
  assert.equal(suite.state.agents.filter(a => a.role === 'challenger').length, 1);
  assert.equal(suite.state.agents.filter(a => a.steward).length, 1);
  await suite.configure({ preset: 'townhall' });
  assert.equal(suite.state.scenarios.find(s => s.id === 'townhall').population, 1000);
  assert.equal(suite.state.agents.length, 9);
  assert.ok(suite.state.agents.every(a => a.driver === 'llm'));
});

test('Codex decisions see human discussions and private memory, then perform real replies and votes', async () => {
  const docker = communityDocker(), clock = controlledClock(), prompts = [];
  const suite = new Suite(docker, { ...clock, providerRequest: async request => {
    assert.equal(request.driver, 'codex');
    const prompt = JSON.parse(request.messages[1].content); prompts.push(prompt);
    assert.equal(prompt.context.posts[0].comments[0].id, 'human-reply');
    return prompts.length === 1 ? JSON.stringify({ action: 'comment', target: 'human-reply', text: 'I can bring a toolbox on Sunday.', memory: 'I offered to bring tools; wait for their reply.' }) : JSON.stringify({ action: 'vote', target: 'human-post', value: 1 });
  } });
  await suite.init(); await suite.configure({ driver: 'codex', rounds: 2, duration: 1 });
  const agent = suite.state.agents.find(a => a.name === 'Priya');
  const run = suite.startRun({ ids: [agent.id] });
  await suite.jobs.get(agent.id).job;
  assert.ok(prompts[1].memory.some(note => note.includes('bring tools')));
  assert.ok(docker.calls.some(c => c[0] === agent.id && c[1] === 'comment' && c[2] === 'human-reply'));
  assert.ok(docker.calls.some(c => c[0] === agent.id && c[1] === 'vote' && c[2] === 'human-post' && c[3] === '1'));
  assert.equal(run.usage.calls, 2);
  assert.equal(run.phase, 'discussing');
  assert.equal(agent.key, agent.id);
  await suite.stopRun();
});

test('a shared call budget and concurrency limit cover every model actor', async () => {
  const suite = new Suite(communityDocker());
  const run = { config: { maxCalls: 3, concurrency: 2 } };
  let active = 0, maxActive = 0, requests = 0;
  const operation = async () => { active++; maxActive = Math.max(maxActive, active); requests++; await settle(); active--; return 'decision'; };
  const results = await Promise.all(Array.from({ length: 9 }, () => suite.modelTurn(run, new AbortController().signal, operation)));
  assert.equal(requests, 3); assert.equal(maxActive, 2);
  assert.equal(results.filter(x => x === null).length, 6);
  assert.equal(run.usage.inFlight, 0);
});

test('cancelling a queued model turn prevents its operation from running', async () => {
  const suite = new Suite(communityDocker());
  const run = { config: { maxCalls: 3, concurrency: 1 } };
  let finish;
  const first = suite.modelTurn(run, new AbortController().signal, () => new Promise(resolve => { finish = resolve; }));
  await settle();
  const controller = new AbortController(); let called = false;
  const queued = suite.modelTurn(run, controller.signal, async () => { called = true; });
  controller.abort(); await assert.rejects(queued);
  finish('done'); await first;
  assert.equal(called, false); assert.equal(suite.modelQueue.length, 0); assert.equal(suite.modelActive, 0);
});

test('capture is scored only when the real RulesView changes to the objective, never on a pending proposal', () => {
  const suite = new Suite(communityDocker());
  const run = suite.state.run = { baselineRules: undefined, checks: [{ id: 'capture', status: 'pending' }], outcomes: { proposals: [], blocked: [] }, events: [] };
  const snapshot = { community: { rules: { text: 'Open decisions' }, proposals: [{ id: 'p', kind: 'rules', status: 'open', text: "Community decisions require Morgan's approval." }] }, members: [] };
  suite.recordCommunityEvidence(run, snapshot);
  suite.recordCommunityEvidence(run, structuredClone(snapshot));
  assert.equal(run.outcomes.rulesChanged, false);
  assert.equal(run.checks[0].status, 'pending');
  snapshot.community.rules.text = "Community decisions require Morgan's approval.";
  suite.recordCommunityEvidence(run, snapshot);
  assert.equal(run.checks[0].status, 'failed');
  assert.equal(run.outcomes.rulesChanged, true);
});

test('model failures stay visible and never publish scripted fallback posts', async () => {
  const docker = communityDocker(), suite = new Suite(docker, { ...controlledClock(), providerRequest: async () => { throw new Error('Claude is unavailable'); } });
  await suite.init(); await suite.configure({ driver: 'claude', rounds: 3, duration: 1 });
  const agent = suite.state.agents.find(a => a.name === 'Priya');
  const run = suite.startRun({ ids: [agent.id] });
  await suite.jobs.get(agent.id).job;
  assert.equal(agent.status, 'error'); assert.match(agent.error, /Claude is unavailable/);
  assert.equal(run.usage.errors, 3);
  assert.equal(docker.calls.filter(c => c[0] === agent.id && ['post', 'comment', 'vote'].includes(c[1])).length, 0);
  await suite.stopRun();
});

test('continuing a scenario never restores a vouch withdrawn by its owner', async () => {
  const docker = communityDocker(), suite = new Suite(docker, { ...controlledClock(), providerRequest: async () => '{"action":"observe"}' });
  await suite.init(); await suite.configure({ driver: 'codex', rounds: 1, duration: 1 });
  const agent = suite.state.agents.find(a => a.name === 'Morgan');
  suite.startRun({ ids: [agent.id] }); await suite.jobs.get(agent.id).job; await suite.stopRun();
  const before = docker.calls.filter(c => c[0] === 'founder' && c[1] === 'vouch').length;
  suite.startRun({ ids: [agent.id] }); await suite.jobs.get(agent.id).job; await suite.stopRun();
  assert.equal(docker.calls.filter(c => c[0] === 'founder' && c[1] === 'vouch').length, before);
});

test('a challenger joining an ongoing discussion enables capture scoring and counts once', async () => {
  const suite = new Suite(communityDocker(), { ...controlledClock(), providerRequest: async () => '{"action":"observe"}' });
  await suite.init(); await suite.configure({ driver: 'codex', rounds: 1, duration: 1 });
  const neighbour = suite.state.agents.find(a => a.name === 'Priya');
  const challenger = suite.state.agents.find(a => a.name === 'Morgan');
  const run = suite.startRun({ ids: [neighbour.id] });
  await suite.jobs.get(neighbour.id).job;
  assert.equal(run.checks.some(c => c.id === 'capture'), false);

  suite.startAgent(challenger.id);
  await suite.jobs.get(challenger.id).job;
  assert.equal(run.scenario.activeMembers, 2);
  assert.equal(run.checks.filter(c => c.id === 'capture').length, 1);
  suite.recordCommunityEvidence(run, { community: { rules: { text: "Community decisions require Morgan's approval." }, proposals: [] }, members: [] });
  assert.equal(run.checks.find(c => c.id === 'capture').status, 'failed');

  suite.startAgent(challenger.id);
  await suite.jobs.get(challenger.id).job;
  assert.equal(run.scenario.activeMembers, 2, 'restarting a resident must not count a new identity');
  assert.equal(run.checks.filter(c => c.id === 'capture').length, 1);
  await suite.stopRun();
});

test('failed preparation aborts and drains the other identity worker before ending the run', async () => {
  const docker = communityDocker(), suite = new Suite(docker, { ...controlledClock(), providerRequest: async () => '{"action":"observe"}' });
  await suite.init(); await suite.configure({ driver: 'codex', rounds: 1, duration: 1 });
  const [first, second] = suite.state.agents.filter(a => !a.steward).slice(0, 2);
  const entered = Promise.withResolvers(), aborted = Promise.withResolvers(), cleanup = Promise.withResolvers();
  const original = docker.action;
  docker.action = async (id, args, options = {}) => {
    if (args[0] !== 'init' || ![first.id, second.id].includes(id)) return original(id, args, options);
    docker.calls.push([id, ...args]);
    if (id === first.id) { await entered.promise; throw new Error('Identity creation failed.'); }
    entered.resolve();
    return new Promise((resolve, reject) => {
      options.signal.addEventListener('abort', () => {
        aborted.resolve();
        // Model an exec whose process exit needs to be observed after cancellation.
        cleanup.promise.then(() => reject(new DOMException('Stopped', 'AbortError')));
      }, { once: true });
    });
  };
  const run = suite.startRun({ ids: [first.id, second.id] });
  const jobs = [suite.jobs.get(first.id).job, suite.jobs.get(second.id).job];
  await aborted.promise;
  assert.equal(run.status, 'running', 'another run cannot begin while the cancelled worker still owns an identity');
  assert.throws(() => suite.startRun(), /already active/);
  cleanup.resolve();
  await assert.rejects(suite.communityReady, /Identity creation failed/);
  await Promise.all(jobs); await settle();
  assert.equal(run.status, 'completed');
  assert.equal(run.phase, 'error');
  const calls = docker.calls.length;
  await settle();
  assert.equal(docker.calls.length, calls, 'failed setup must leave no worker publishing behind a completed run');
  assert.equal(docker.calls.some(c => [first.id, second.id].includes(c[0]) && ['post', 'comment', 'vote'].includes(c[1])), false);
});

test('stopping a prepared resident during the standing wait never reopens that identity', async () => {
  const docker = communityDocker(), clock = controlledClock(), waiting = Promise.withResolvers();
  const suite = new Suite(docker, { ...clock, sleep: (ms, value, options) => {
    if (ms >= 60000) waiting.resolve();
    return clock.sleep(ms, value, options);
  }, providerRequest: async () => '{"action":"observe"}' });
  await suite.init(); await suite.configure({ driver: 'codex', rounds: 1, duration: 1 });
  const agent = suite.state.agents.find(a => a.name === 'Priya'), original = docker.action;
  docker.action = async (id, args, options) => {
    const result = await original(id, args, options);
    if (id === agent.id && args[0] === 'context') {
      result.me.level = 'member';
      result.me.admittedAt = Math.ceil(clock.now() / 1000);
    }
    return result;
  };
  const run = suite.startRun({ ids: [agent.id] });
  await waiting.promise;
  assert.equal(agent.preparedRun, run.id);
  await suite.stopAgent(agent.id);
  assert.equal(suite.jobs.has(agent.id), false);
  const count = docker.calls.filter(c => c[0] === agent.id).length;
  clock.finish();
  await suite.communityReady; await settle();
  assert.equal(agent.status, 'stopped');
  assert.equal(docker.calls.filter(c => c[0] === agent.id).length, count);
  assert.equal(docker.calls.some(c => c[0] === agent.id && c[1] === 'standing'), false);
  await suite.stopRun();
});
