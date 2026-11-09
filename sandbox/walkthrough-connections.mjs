// Only these observations may leave the local clients. Invitations, member credentials and
// relay-room identifiers are intentionally excluded from the walkthrough's connection view.
const PEOPLE = ['alice', 'bob'];
const validCommunity = value => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);

export function describeConnections({ communities, observations, nativeConfigured, invitation, capturedAt = Date.now() }) {
  const publicClients = PEOPLE.map((id, index) => {
    const value = observations.find(item => item.id === id)?.public;
    return { id, joined: value?.joined === true, communityId: validCommunity(value?.communityId) ? value.communityId : null, node: index ? 'node-b' : 'node-a' };
  });
  const privateClients = PEOPLE.map(id => {
    const value = observations.find(item => item.id === id)?.private;
    return { id, active: value?.active === true, communityId: validCommunity(value?.communityId) ? value.communityId : null,
      epoch: Number.isSafeInteger(value?.epoch) && value.epoch >= 0 ? value.epoch : null,
      memberCount: Number.isSafeInteger(value?.memberCount) && value.memberCount >= 0 && value.memberCount <= 128 ? value.memberCount : null };
  });
  const sameCommunity = privateClients.every(client => client.active && client.communityId === communities.private.id);
  const rooms = PEOPLE.map(id => observations.find(item => item.id === id)?.private?.room);
  const requestCreated = invitation?.requestCreated === true;
  const inviteCreated = invitation?.inviteCreated === true;
  const joined = invitation?.joined === true;
  return {
    public: { communityId: communities.public.id, name: communities.public.name, clients: publicClients,
      sameCommunity: publicClients.every(client => client.joined && client.communityId === communities.public.id),
      nativeLink: { from: 'node-b', to: 'node-a', configured: nativeConfigured === true }, capturedAt, mode: 'captured-setup' },
    private: { communityId: communities.private.id, name: communities.private.name, clients: privateClients, sameCommunity,
      sameEpoch: sameCommunity && privateClients.every(client => client.epoch !== null && client.epoch === privateClients[0].epoch),
      sameRoom: sameCommunity && rooms.every(room => validCommunity(room) && room === rooms[0]),
      invitation: { method: 'local-handoff', requestCreated, inviteCreated, joined, completed: requestCreated && inviteCreated && joined && sameCommunity },
      relays: ['relay-a', 'relay-b'], capturedAt, mode: 'captured-setup' },
  };
}
