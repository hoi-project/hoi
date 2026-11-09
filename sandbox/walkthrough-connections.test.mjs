import test from 'node:test';
import assert from 'node:assert/strict';
import { describeConnections } from './walkthrough-connections.mjs';

const communities = { public: { id: 'a'.repeat(64), name: 'public-garden' }, private: { id: 'b'.repeat(64), name: 'private-circle' } };
const fixture = () => ({ communities, nativeConfigured: true, capturedAt: 123,
  invitation: { requestCreated: true, inviteCreated: true, joined: true, request: 'hidden-request', welcome: 'hidden-welcome' },
  observations: ['alice', 'bob'].map(id => ({ id,
    public: { joined: true, communityId: communities.public.id, key: 'hidden-public-key' },
    private: { active: true, communityId: communities.private.id, epoch: 3, memberCount: 2, room: 'c'.repeat(64),
      members: [{ credential: 'hidden-member' }], invitations: ['hidden-invitation'], secret: 'hidden-secret' },
  })),
});

test('connection evidence links the same community through distinct public nodes and matching private membership', () => {
  const view = describeConnections(fixture());
  assert.equal(view.public.sameCommunity, true);
  assert.deepEqual(view.public.clients.map(client => [client.id, client.node, client.joined]), [['alice', 'node-a', true], ['bob', 'node-b', true]]);
  assert.deepEqual(view.public.nativeLink, { from: 'node-b', to: 'node-a', configured: true });
  assert.equal(view.private.sameCommunity, true);
  assert.equal(view.private.sameEpoch, true);
  assert.equal(view.private.sameRoom, true);
  assert.deepEqual(view.private.clients.map(client => [client.id, client.active, client.epoch, client.memberCount]), [['alice', true, 3, 2], ['bob', true, 3, 2]]);
  assert.deepEqual(view.private.invitation, { method: 'local-handoff', requestCreated: true, inviteCreated: true, joined: true, completed: true });
  assert.equal(view.private.mode, 'captured-setup');
  assert.equal(view.private.capturedAt, 123);
});

test('connection evidence exports no invitation, credential, key or relay room value', () => {
  const view = describeConnections(fixture());
  assert.doesNotMatch(JSON.stringify(view), /hidden-|cccccccc/);
  assert.deepEqual(Object.keys(view.private.clients[0]).sort(), ['active', 'communityId', 'epoch', 'id', 'memberCount']);
  assert.deepEqual(Object.keys(view.public.clients[0]).sort(), ['communityId', 'id', 'joined', 'node']);
});

test('missing, inactive or mismatched observations do not claim shared access', () => {
  for (const patch of [{ active: false }, { communityId: 'd'.repeat(64) }]) {
    const input = fixture(); Object.assign(input.observations[1].private, patch);
    const view = describeConnections(input);
    assert.equal(view.private.sameCommunity, false);
    assert.equal(view.private.sameRoom, false);
    assert.equal(view.private.sameEpoch, false);
    assert.equal(view.private.invitation.completed, false);
  }
  const input = fixture(); input.observations.pop();
  const missing = describeConnections(input);
  assert.equal(missing.public.sameCommunity, false);
  assert.equal(missing.private.sameCommunity, false);
  const different = fixture(); different.observations[1].private.epoch = 4; different.observations[1].private.room = 'd'.repeat(64);
  assert.equal(describeConnections(different).private.sameEpoch, false);
  assert.equal(describeConnections(different).private.sameRoom, false);
  const empty = fixture(); empty.observations.forEach(item => { item.private.room = ''; item.private.epoch = -1; });
  assert.equal(describeConnections(empty).private.sameRoom, false);
  assert.equal(describeConnections(empty).private.sameEpoch, false);
  const publicMismatch = fixture(); publicMismatch.observations[1].public.communityId = 'd'.repeat(64);
  assert.equal(describeConnections(publicMismatch).public.sameCommunity, false);
});
