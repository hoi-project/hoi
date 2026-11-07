import { describe, expect, it, vi } from 'vitest';
import type { CommunitySummary, CommunityView, HoiCore, NetStatus, PrivateGroup } from '@hoi/core-api';
import { PublicConnectionDiscovery, publicRoutes } from '../lib/public-connections';

const rtc = `rtc:${'q'.repeat(43)}@203.0.113.10:7844`;
const native = `${'a'.repeat(64)}@203.0.113.10:7843`;
const local: NetStatus = { mode: 'local', label: 'local', peers: 0, rtc: [], relaying: false };
const community = (id: string, joined = true) => ({ id, joined } as CommunitySummary);
const view = (id: string, joined = true, admission = 'open') => ({ id, joined, limits: { admission } } as CommunityView);
function fixture() {
  let now = 1_000;
  const metadata = vi.fn(async () => ({ publicNodes: [rtc, native] }));
  const core = {
    community: vi.fn(async (id: string) => view(id)),
    publicJoin: vi.fn(async (id: string) => ({ id, status: 'joined' as const })),
  } satisfies Pick<HoiCore, 'community' | 'publicJoin'>;
  return { metadata, core, discovery: new PublicConnectionDiscovery(metadata, () => now), later: () => { now += 300_001; } };
}

describe('public relay discovery', () => {
  it('connects joined open communities directly with routes for this device', async () => {
    const f = fixture();
    expect(publicRoutes([rtc, native], 'web')).toEqual([rtc]);
    expect(publicRoutes([rtc, native], 'desktop')).toEqual([native]);
    const result = await f.discovery.connect(f.core, [community('garden')], [], 'web', local);
    expect(f.core.publicJoin).toHaveBeenCalledExactlyOnceWith('garden', [rtc]);
    expect(result.requested).toBe(1);
    expect(result.message).toContain('Nobody can read what you post from here until one has it');
  });

  it('does nothing at all in an app built without a registry', async () => {
    const f = fixture();
    const discovery = new PublicConnectionDiscovery(null);
    const result = await discovery.connect(f.core, [community('garden')], [], 'web', local);
    expect(result).toMatchObject({ requested: 0, registry: 'unchecked', message: '' });
    expect(f.core.community).not.toHaveBeenCalled();
    expect(f.core.publicJoin).not.toHaveBeenCalled();
  });

  it('asks the trackers a community names, even in an app built without a registry', async () => {
    const f = fixture();
    const other = `rtc:${'w'.repeat(43)}@198.51.100.4:7844`;
    f.core.community.mockImplementation(async id => ({ ...view(id), trackers: ['https://t1.example', 'https://t2.example'] } as CommunityView));
    const asked: string[] = [];
    const trackerNodes = vi.fn(async (tracker: string, c: string) => {
      asked.push(`${tracker} ${c}`);
      if (tracker === 'https://t1.example') throw new Error('down');
      return [other, native];
    });
    const discovery = new PublicConnectionDiscovery(null, Date.now, trackerNodes);
    const result = await discovery.connect(f.core, [community('garden')], [], 'web', local);
    expect(asked).toEqual(['https://t1.example garden', 'https://t2.example garden']);
    expect(f.core.publicJoin).toHaveBeenCalledExactlyOnceWith('garden', [other]);
    expect(result).toMatchObject({ tracked: 1, requested: 1, registry: 'unchecked' });
    // a community that names none asks nobody
    f.core.community.mockImplementation(async id => ({ ...view(id), trackers: [] } as CommunityView));
    const none = await new PublicConnectionDiscovery(null, Date.now, trackerNodes).connect(f.core, [community('quiet')], [], 'web', local);
    expect(none.tracked).toBe(0);
    expect(trackerNodes).toHaveBeenCalledTimes(2);
  });

  it('never connects private, unjoined, restricted or already connected communities', async () => {
    const f = fixture();
    f.core.community.mockImplementation(async id => view(id, true, 'invite'));
    const net = { ...local, peers: 1, communities: [{ community: 'connected', connectedPeers: 1 }] };
    await f.discovery.connect(f.core, [community('private'), community('left', false), community('restricted'), community('connected')],
      [{ community: 'private' } as PrivateGroup], 'web', net);
    expect(f.core.community).toHaveBeenCalledExactlyOnceWith('restricted');
    expect(f.metadata).not.toHaveBeenCalled();
    expect(f.core.publicJoin).not.toHaveBeenCalled();
  });

  it('bounds automatic checks and does not reset transport backoff for unchanged routes', async () => {
    const f = fixture();
    const args = [f.core, [community('garden')], [], 'web', local] as const;
    await f.discovery.connect(...args);
    await f.discovery.connect(...args);
    expect(f.metadata).toHaveBeenCalledTimes(1);
    f.later();
    await f.discovery.connect(...args);
    expect(f.metadata).toHaveBeenCalledTimes(2);
    expect(f.core.publicJoin).toHaveBeenCalledTimes(1);
    await f.discovery.connect(...args, { force: true });
    expect(f.core.publicJoin).toHaveBeenCalledTimes(2);
  });

  it('picks up changed routes without repeatedly trying all joined communities', async () => {
    const f = fixture();
    const communities = Array.from({ length: 20 }, (_, i) => community(String(i)));
    expect((await f.discovery.connect(f.core, communities, [], 'web', local)).checked).toBe(16);
    expect((await f.discovery.connect(f.core, communities, [], 'web', local)).checked).toBe(4);
    expect(f.metadata).toHaveBeenCalledTimes(1);
    f.later();
    const next = rtc.replace('7844', '8844');
    f.metadata.mockResolvedValue({ publicNodes: [next] });
    await f.discovery.connect(f.core, communities, [], 'web', local, { community: '0' });
    expect(f.core.publicJoin).toHaveBeenLastCalledWith('0', [next]);
  });

  it('does not rejoin a community left while the directory request was pending', async () => {
    const f = fixture();
    f.metadata.mockImplementation(async () => { f.core.community.mockResolvedValue(view('garden', false)); return { publicNodes: [rtc] }; });
    await f.discovery.connect(f.core, [community('garden')], [], 'web', local);
    expect(f.core.publicJoin).not.toHaveBeenCalled();
  });

  it('restores routes after leaving and rejoining the same community', async () => {
    const f = fixture();
    await f.discovery.connect(f.core, [community('garden')], [], 'web', local);
    await f.discovery.connect(f.core, [community('garden', false)], [], 'web', local);
    await f.discovery.connect(f.core, [community('garden')], [], 'web', local);
    expect(f.core.publicJoin).toHaveBeenCalledTimes(2);
  });

  it('discards discovery after a lock or wipe even when the directory later answers', async () => {
    const f = fixture();
    f.metadata.mockImplementation(async () => { f.discovery.reset(); return { publicNodes: [rtc] }; });
    const result = await f.discovery.connect(f.core, [community('garden')], [], 'web', local);
    expect(f.core.publicJoin).not.toHaveBeenCalled();
    expect(result.message).toBe('');
  });

  it('keeps missing or unavailable infrastructure separate from a successful connection', async () => {
    const f = fixture();
    f.metadata.mockResolvedValue({ publicNodes: [] });
    const empty = await f.discovery.connect(f.core, [community('garden')], [], 'web', local);
    expect(empty.requested).toBe(0);
    expect(empty.message).toContain('No public relays');
    expect(empty).toMatchObject({ registry: 'reachable', announced: 0 });
    f.metadata.mockRejectedValue(new Error('offline'));
    const failed = await f.discovery.connect(f.core, [community('garden')], [], 'web', local, { force: true });
    expect(failed.message).toContain('Saved posts and existing connections still work');
    expect(failed.registry).toBe('unreachable');
    expect(f.core.publicJoin).not.toHaveBeenCalled();
  });

  it('reports the registry and its announced nodes for the directory reading', async () => {
    const f = fixture();
    const idle = await f.discovery.connect(f.core, [], [], 'web', local);
    expect(idle).toMatchObject({ registry: 'unchecked', announced: 0 });
    const checked = await f.discovery.connect(f.core, [community('garden')], [], 'web', local);
    expect(checked).toMatchObject({ registry: 'reachable', announced: 2 });
    // Nothing to connect: the last answer still counts until it is five minutes old.
    const net = { ...local, peers: 1, communities: [{ community: 'garden', connectedPeers: 1 }] };
    expect(await f.discovery.connect(f.core, [community('garden')], [], 'web', net)).toMatchObject({ registry: 'reachable', announced: 2 });
    f.discovery.reset();
    expect((await f.discovery.connect(f.core, [], [], 'web', local)).registry).toBe('unchecked');
  });
});
