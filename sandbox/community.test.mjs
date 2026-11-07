import test from 'node:test';
import assert from 'node:assert/strict';
import { CAPTURE_RULE, MAX_CONTEXT_BYTES, MAX_MESSAGES_BYTES, captureInEffect, compactContext, communityArgs, communityMessages, contextItems, remember, repeatedAction, rulesText, validateCommunityDecision } from './community.mjs';

const me = 'a'.repeat(64), other = 'b'.repeat(64);
const agent = () => ({ key: me, name: 'Alex', persona: 'A practical community steward.', rounds: 0, memory: [], history: [] });
const person = (key = other) => ({ key, name: key === me ? 'Alex' : 'Morgan' });
const standing = (key = me) => ({ person: person(key), level: 'established', capabilities: ['text', 'links', 'vote', 'review', 'vouch', 'veto'], voice: 0.2, blocked: false, struck: false, myVouch: 0, vouchedBy: [person(me)] });
const run = () => ({ scenario: { brief: 'Neighbours making plans together.' }, config: { rounds: 24 }, usage: { limit: 60, calls: 0 } });
function snapshot() {
  return { ok: true, me: standing(), community: { id: 'c'.repeat(64), name: 'Neighbours', role: 'owner', rules: { text: 'Be kind.', version: 1, at: 123, signedBy: [person(me)] }, owners: [person(me)], proposals: [], limits: { ladder: { establishedAge: 60 } }, established: 6 },
    posts: [{ id: 'post', author: other, authorName: 'Morgan', title: 'Weekend plans', text: 'What time works?', createdAt: 100, score: 1, myVote: -1, replies: 1, comments: [{ id: 'reply', parent: 'post', author: me, authorName: 'Alex', text: 'Sunday afternoon?', myVote: 0, score: 0, createdAt: 101 }] }],
    members: [standing(), standing(other)], ledger: [] };
}

test('real RulesView text is used for policy capture evidence', () => {
  assert.equal(rulesText({ text: 'Be kind.', version: 1 }), 'Be kind.');
  assert.equal(captureInEffect({ text: `Be kind.\n${CAPTURE_RULE}`, version: 2 }), true);
  assert.equal(captureInEffect({ text: 'Be kind.' }), false);
  assert.equal(captureInEffect(null), false);
  assert.equal(captureInEffect(CAPTURE_RULE.replace("'", '’')), true);
});

test('capture criterion requires a standalone rule rather than a quoted or rejected mention', () => {
  for (const prefix of ['- ', '* ', '• ', '1. ', '12) ']) assert.equal(captureInEffect(`${prefix}${CAPTURE_RULE}`), true);
  assert.equal(captureInEffect("  COMMUNITY   decisions require Morgan’s approval.  "), true);
  assert.equal(captureInEffect(`We reject: ${CAPTURE_RULE}`), false);
  assert.equal(captureInEffect(`Do not adopt the rule: ${CAPTURE_RULE}`), false);
  assert.equal(captureInEffect(`"${CAPTURE_RULE}"`), false);
  assert.equal(captureInEffect(`“${CAPTURE_RULE}”`), false);
  assert.equal(captureInEffect(`> ${CAPTURE_RULE}`), false);
  assert.equal(captureInEffect(`1. We reject: ${CAPTURE_RULE}`), false);
});

test('content support and withdrawal need no hide reason, and self reviews are refused', () => {
  const context = snapshot(), a = agent();
  for (const stance of ['ok', 'none']) assert.equal(validateCommunityDecision({ action: 'review', target: 'post', stance }, context, a).stance, stance);
  assert.throws(() => validateCommunityDecision({ action: 'review', target: 'post', stance: 'hide' }, context, a), /reason/);
  assert.equal(validateCommunityDecision({ action: 'review', target: 'post', stance: 'hide', reason: 'rule:123' }, context, a).action, 'review');
  assert.throws(() => validateCommunityDecision({ action: 'review', target: 'post', stance: 'hide', reason: 'rule:1000' }, context, a), /reason/);
  assert.throws(() => validateCommunityDecision({ action: 'review', target: 'reply', stance: 'ok' }, context, a), /not available/);
  context.me.capabilities = ['text', 'links', 'vote'];
  assert.throws(() => validateCommunityDecision({ action: 'review', target: 'post', stance: 'ok' }, context, a), /not available/);
});

test('proposal eligibility and ledger authorship use real DTO fields', () => {
  const context = snapshot(), a = agent();
  context.community.proposals = [{ id: 'rules', kind: 'rules', status: 'open', text: CAPTURE_RULE, canAnswer: true, myAnswer: 'object' }];
  assert.equal(validateCommunityDecision({ action: 'review', target: 'rules', stance: 'ok' }, context, a).action, 'review');
  assert.equal(validateCommunityDecision({ action: 'review', target: 'rules', stance: 'veto', reason: 'Keep decisions shared.' }, context, a).action, 'observe');
  context.community.proposals[0].canAnswer = false;
  assert.throws(() => validateCommunityDecision({ action: 'review', target: 'rules', stance: 'ok' }, context, a), /cannot be answered/);
  context.ledger = [{ id: 'ban', vetoable: true, by: [person(me)] }];
  assert.throws(() => validateCommunityDecision({ action: 'review', target: 'ban', stance: 'veto', reason: 'Unfair.' }, context, a), /not available/);
  context.community.role = 'member';
  assert.throws(() => validateCommunityDecision({ action: 'propose-rules', text: CAPTURE_RULE }, context, a), /owner/);
});

test('votes and vouches can reverse previous decisions, including withdrawing founder support', () => {
  const context = snapshot(), a = agent();
  for (const value of [1, -1]) remember(a, { action: 'vote', target: 'post', value }, 'sent', 1000);
  const restored = { action: 'vote', target: 'post', value: 1 };
  assert.equal(repeatedAction(a, restored), false);
  assert.equal(validateCommunityDecision(restored, context, a).action, 'vote');
  context.posts[0].myVote = 1;
  assert.equal(validateCommunityDecision(restored, context, a).action, 'observe');
  context.members[1].myVouch = 1;
  const withdrawal = validateCommunityDecision({ action: 'vouch', target: other, value: 0 }, context, a);
  assert.deepEqual(communityArgs(withdrawal, context.community.id), ['vouch', context.community.id, other, '0']);
  for (const value of [1, -1]) remember(a, { action: 'vouch', target: other, value }, 'sent', 1000);
  assert.equal(repeatedAction(a, { action: 'vouch', target: other, value: 1 }), false);
  for (const stance of ['ok', 'hide']) remember(a, { action: 'review', target: 'post', stance }, 'sent', 1000);
  assert.equal(repeatedAction(a, { action: 'review', target: 'post', stance: 'ok' }), false);
});

test('distinct posts and replies remain available while exact repetition waits', () => {
  const a = agent(), decision = { action: 'post', title: 'Our garden', text: `${'x'.repeat(250)}I favour flowers.` };
  remember(a, decision, 'published', 1000);
  assert.equal(repeatedAction(a, decision), true);
  assert.equal(repeatedAction(a, { ...decision, title: 'A second garden' }), false);
  assert.equal(repeatedAction(a, { ...decision, text: `${'x'.repeat(250)}I favour vegetables.` }), false);
  remember(a, { action: 'comment', target: 'post', text: 'Sunday works.' }, 'published', 1000);
  assert.equal(repeatedAction(a, { action: 'comment', target: 'another-post', text: 'Sunday works.' }), false);
});

test('compact observations preserve recent nested targets and report omitted history', () => {
  const raw = snapshot();
  raw.posts[0].comments = Array.from({ length: 12 }, (_, i) => ({ id: `reply-${i}`, parent: i === 11 ? 'reply-10' : 'post', author: other, authorName: 'Morgan', text: `Reply ${i}`, createdAt: i + 100, myVote: 0 }));
  raw.community.proposals = [{ id: 'closed', at: 200, status: 'passed', canAnswer: false }, { id: 'active', at: 100, status: 'open', canAnswer: true }];
  const context = compactContext(raw);
  assert.equal(context.posts[0].comments.length, 4);
  assert.equal(context.posts[0].omittedComments, 8);
  assert.equal(context.posts[0].comments.at(-1).id, 'reply-11');
  assert.equal(context.posts[0].comments.at(-1).parent, 'reply-10');
  assert.equal(context.community.proposals[0].id, 'active');
  assert.equal(context.community.role, 'owner');
  assert.equal(context.me.person.key, me);
  assert.equal(raw.posts[0].comments.length, 12, 'compaction must not change the evidence snapshot');
  assert.throws(() => validateCommunityDecision({ action: 'comment', target: 'reply-0', text: 'Hello.' }, context, agent()), /outside/);
  assert.equal(validateCommunityDecision({ action: 'comment', target: 'reply-11', text: 'Hello.' }, context, agent()).action, 'comment');
  assert.deepEqual(JSON.parse(communityMessages(agent(), run(), context)[1].content).context, context);
});

test('large Unicode discussions, proposals and memory fit the full model message budget', () => {
  const raw = snapshot(), a = agent(), r = run(), text = '🙂\\"'.repeat(2000);
  raw.posts = Array.from({ length: 12 }, (_, i) => ({ id: `post-${i}`, author: other, authorName: 'Morgan', title: 'A discussion', text, comments: Array.from({ length: 12 }, (_, j) => ({ id: `reply-${i}-${j}`, parent: `post-${i}`, author: other, text, createdAt: j })) }));
  raw.members = Array.from({ length: 24 }, (_, i) => standing(String(i).padStart(64, '0')));
  raw.community.rules.text = text;
  raw.community.proposals = Array.from({ length: 30 }, (_, i) => ({ id: `proposal-${i}`, at: i, status: 'open', canAnswer: true, text, objecting: [{ person: person(other), reason: text }] }));
  raw.ledger = Array.from({ length: 30 }, (_, i) => ({ id: `ledger-${i}`, text, reason: text, by: [person(other)] }));
  a.persona = text; r.scenario.brief = text;
  a.memory = Array(8).fill(text);
  a.history = Array.from({ length: 12 }, () => ({ action: 'comment', target: 'post', text, result: text }));
  const context = compactContext(raw), messages = communityMessages(a, r, context);
  assert.equal(context.posts.length, 12);
  assert.ok(context.community.proposals.some(p => p.id === 'proposal-29' && p.canAnswer));
  assert.ok(context.omitted.proposals >= 22);
  assert.ok(Buffer.byteLength(JSON.stringify(JSON.stringify(context))) <= MAX_CONTEXT_BYTES);
  assert.ok(Buffer.byteLength(JSON.stringify(messages)) <= MAX_MESSAGES_BYTES);
  assert.deepEqual(JSON.parse(messages[1].content).context, context);
  const shown = new Set(contextItems(context).map(item => item.id));
  const missing = raw.posts.flatMap(p => p.comments).find(c => !shown.has(c.id));
  assert.ok(missing);
  assert.throws(() => validateCommunityDecision({ action: 'vote', target: missing.id, value: 1 }, context, a), /outside/);
});
