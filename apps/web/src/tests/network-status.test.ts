import { describe, expect, it } from 'vitest';
import type { NetStatus } from '@hoi/core-api';
import { canPublishPublic, carriersOf, postsThroughOthers, savedForSync, syncStatus } from '../lib/network-status';

const local: NetStatus = { mode: 'local', label: 'only on this device', peers: 0, rtc: [], relaying: false };

describe('sync status', () => {
  it('keeps offline saves distinct from delivery', () => {
    expect(syncStatus(null).state).toBe('checking');
    expect(syncStatus(local).state).toBe('offline');
    expect(syncStatus(local).detail).toContain('cannot be sent or received');
    expect(savedForSync(local)).toContain('Waiting for a connection');
  });

  it('shows the browser failure and a way to fix a disconnected home node', () => {
    const net = { ...local, homeNode: { url: 'ws://localhost:7777', connected: false, pairingWeb: '', error: 'Local network permission is blocked.' } };
    const status = syncStatus(net);
    expect(status.state).toBe('offline');
    expect(status.detail).toContain('Local network permission is blocked.');
    expect(status.detail).toContain('browser permissions');
    expect(status.action).toBe('Fix connection');
  });

  it('does not call other routes offline when only the home node failed', () => {
    const net = { ...local, peers: 2, homeNode: { url: 'wss://home.example', connected: false, pairingWeb: '' } };
    expect(syncStatus(net).state).toBe('degraded');
    expect(syncStatus(net).detail).toContain('2 other peers are connected');
    expect(savedForSync(net)).toContain('delivery is not confirmed');
  });

  it('never treats an open connection or earlier catch-up as a post receipt', () => {
    const net = { ...local, peers: 3, lastSync: 123456 };
    expect(syncStatus(net).state).toBe('connected');
    expect(syncStatus(net).detail).toContain('matching community IDs');
    expect(savedForSync(net)).toContain('delivery is not confirmed');
    expect(savedForSync(net)).not.toContain('Sent');
  });

  it('waits for this exact community even when unrelated peers are connected', () => {
    const net = { ...local, peers: 2, communities: [
      { community: 'mine', connectedPeers: 0 },
      { community: 'other', connectedPeers: 2, lastSync: 123456 },
    ] };
    const status = syncStatus(net, 'mine');
    expect(status.state).toBe('degraded');
    expect(status.title).toBe('Connected, but not to this community');
    expect(status.detail).toContain('same community invite');
    expect(savedForSync(net, 'mine')).toContain('Waiting for a peer that carries this community');
    expect(savedForSync(net, 'unknown')).toContain('Waiting for a peer');
  });

  it('reports only matching peers and still does not invent a receipt after catch-up', () => {
    const net = { ...local, peers: 5, communities: [{ community: 'mine', connectedPeers: 1, lastSync: 123456 }] };
    expect(syncStatus(net, 'mine').title).toBe('Connected to this community');
    expect(syncStatus(net, 'mine').detail).toContain('1 peer has');
    expect(savedForSync(net, 'mine')).toContain('delivery is not confirmed');
  });

  it('does not infer a community connection when an older node only reports global peers', () => {
    const net = { ...local, peers: 4 };
    expect(syncStatus(net, 'mine').state).toBe('degraded');
    expect(syncStatus(net, 'mine').title).toBe('Community sync not confirmed');
    expect(savedForSync(net, 'mine')).toContain('sync connection is not confirmed');
  });

  it('opens public posting only for the exact community with a live peer', () => {
    expect(canPublishPublic(null, 'mine')).toBe(false);
    expect(canPublishPublic({ ...local, peers: 3 }, 'mine')).toBe(false);
    const net = { ...local, peers: 2, communities: [{ community: 'other', connectedPeers: 2 }] };
    expect(canPublishPublic(net, 'mine')).toBe(false);
    expect(canPublishPublic(net)).toBe(false);
    expect(canPublishPublic(net, 'other')).toBe(true);
    expect(canPublishPublic({ ...net, peers: 0 }, 'other')).toBe(false);
    expect(canPublishPublic({ ...net, communities: [{ community: 'other', connectedPeers: 0, lastSync: 123 }] }, 'other')).toBe(false);
  });

  it('says plainly that a browser without a home node is read only through the nodes that carry it', () => {
    const net = { ...local, peers: 2, communities: [{ community: 'mine', connectedPeers: 2 }] };
    expect(postsThroughOthers(net, 'web')).toBe(true);
    expect(postsThroughOthers(net, 'mobile')).toBe(true);
    expect(postsThroughOthers(net, 'desktop')).toBe(false);
    expect(postsThroughOthers({ ...net, homeNode: { url: 'ws://127.0.0.1:7842', connected: true, pairingWeb: '' } }, 'web')).toBe(false);
    expect(carriersOf(net, 'mine')).toBe(2);
    expect(carriersOf(net, 'other')).toBe(0);
    expect(savedForSync(net, 'mine', 'web')).toContain('Sent to the home nodes carrying this community');
    expect(savedForSync(net, 'mine', 'web')).toContain('Delivery is not confirmed');
    expect(savedForSync(local, 'mine', 'web')).toContain('Nobody else can read it until a home node carrying this community has it');
    expect(savedForSync(net, 'mine')).not.toContain('Nobody else');
    for (const status of [syncStatus(null), syncStatus(local), syncStatus(net, 'other')]) expect(status.detail).not.toContain('optional');
  });

  it('does not block a live community because an unrelated home node failed', () => {
    const net = { ...local, peers: 1, communities: [{ community: 'mine', connectedPeers: 1 }], homeNode: { url: 'wss://home.example', connected: false, pairingWeb: '', error: 'Unavailable' } };
    expect(canPublishPublic(net, 'mine')).toBe(true);
    expect(syncStatus(net, 'mine').detail).toContain('Delivery is not confirmed');
  });
});
