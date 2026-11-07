import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { passageCorpus, loadWiki } from '../apps/site/plugins/wiki.ts';
import { retrieve, parseReply, parseTrail, TRAIL_TTL } from '../apps/site/src/wiki/assistant-context.ts';
import { explain, createSession } from '../apps/site/src/wiki/assistant-model.ts';

const root = resolve(import.meta.dirname, '../apps/site') + '/';
const corpus = passageCorpus(loadWiki(root), (p) => readFileSync(root + p.file, 'utf8'));
const topic = { url: '/wiki/identity/#recovery-line', title: 'The recovery line', level: 2 };

test('explanation corpus retains full sections, row boundaries and safe wiki destinations', () => {
  const page = { href: '/wiki/example/', title: 'Example' };
  const html = '<article><section class="wk-sec" id="a"><h2>Example</h2><p>' + 'Long sentence. '.repeat(400) +
    'THE END</p><table><tr><td>Setting</td><td>Value</td></tr></table>' +
    '<a href="#b">Next</a><a href="https://evil.test">External</a><a href="javascript:alert(1)">Bad</a>' +
    '<h3 id="b">Details</h3><p>The rest.</p></section></article>';
  const built = passageCorpus([page], () => html);
  assert.ok(built.passages.length > 3);
  assert.match(built.passages.map((p) => p.text).join(' '), /THE END/);
  assert.ok(built.passages.every((p) => p.text.length <= 2600));
  assert.ok(built.passages.some((p) => p.text.includes('Setting | Value |')));
  assert.ok(built.passages.every((p) => p.links.every((l) => l === '/wiki/example/#b')));
  assert.equal(built.passages.at(-1).parent, '/wiki/example/#a');
  assert.equal(passageCorpus([page], () => html).version, built.version);
  assert.notEqual(passageCorpus([page], () => html.replace('THE END', 'Changed')).version, built.version);
});

test('natural-language retrieval keeps the active section and finds related documentation', () => {
  const results = retrieve(corpus, topic, 'How does recovery work in the web app vault?');
  assert.ok(results[0].url === topic.url || results[0].parent === topic.url);
  assert.ok(results.some((p) => p.url.startsWith('/wiki/web-app/') || p.url.startsWith('/wiki/storage/')));
  assert.ok(results.length <= 7);
  assert.ok(results.some((p) => p.text.length > 220));
});

test('unknown citations and ungrounded answers cannot be rendered as supported answers', () => {
  const sources = retrieve(corpus, topic, 'What is a recovery line?');
  const response = { status: 'answered', answer: 'An explanation.', sources: [1], next: [] };
  assert.equal(parseReply(JSON.stringify(response), sources, topic).answer, response.answer);
  assert.throws(() => parseReply(JSON.stringify({ ...response, sources: [9999] }), sources, topic), /Unknown citation/);
  assert.throws(() => parseReply(JSON.stringify({ ...response, sources: [] }), sources, topic), /Missing section/);
  const filtered = parseReply(JSON.stringify({ ...response, next: [{ source: 9999, reason: 'bad', question: 'bad' }] }), sources, topic);
  assert.deepEqual(filtered.next, []);
  assert.equal(parseReply(JSON.stringify({ status: 'off-topic', answer: 'Ask about recovery.', sources: [], next: [] }), sources, topic).status, 'off-topic');
});

test('continuations reject expired, oversized, malformed and non-wiki destinations', () => {
  const now = Date.now();
  const trail = { version: corpus.version, original: 'Why?', question: 'How?', answer: 'Earlier context', fromTitle: 'Recovery', from: topic.url, to: '/wiki/web-app/#vault', at: now };
  assert.deepEqual(parseTrail(JSON.stringify(trail), now), trail);
  for (const override of [
    { at: now - TRAIL_TTL - 1 }, { at: now + 2000 }, { to: 'https://evil.test/' },
    { to: '//evil.test/wiki/#a' }, { original: 'x'.repeat(1201) }, { answer: null },
  ]) assert.equal(parseTrail(JSON.stringify({ ...trail, ...override }), now), null);
  assert.equal(parseTrail('{nope'), null);
});

test('context budgeting removes excess passages and history before prompting', async () => {
  const candidates = retrieve(corpus, topic, 'How does recovery work in the web app vault?');
  let measured = 0;
  const session = {
    contextWindow: 4000, contextUsage: 100,
    async measureContextUsage(prompt) {
      measured++;
      const data = JSON.parse(prompt);
      return data.SOURCES.length * 1100 + data.RECENT_CONVERSATION.length * 2000;
    },
    async prompt(prompt, options) {
      const data = JSON.parse(prompt);
      assert.ok(data.SOURCES.length <= 2);
      assert.equal(data.RECENT_CONVERSATION.length, 0);
      assert.ok(options.responseConstraint);
      return JSON.stringify({ status: 'answered', answer: 'Supported.', sources: [1], next: [] });
    },
  };
  const result = await explain(session, topic, 'Why?', candidates, [{ question: 'Earlier', answer: 'Earlier answer' }], undefined, new AbortController().signal);
  assert.ok(measured > 1);
  assert.equal(result.reply.answer, 'Supported.');
  assert.ok(result.passages.length <= 2);
});

test('aborted generation does not prompt and synchronous setup errors become rejections', async () => {
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(explain({ measureContextUsage() { assert.fail('must not measure'); } }, topic, 'Why?', [], [], undefined, controller.signal));
  await assert.rejects(createSession({ create() { throw new Error('Unavailable'); } }, new AbortController().signal, () => {}), /Unavailable/);
});
