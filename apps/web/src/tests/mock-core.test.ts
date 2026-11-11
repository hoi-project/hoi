import { describe, expect, it } from 'vitest';
import { HoiError, type CommunityView, type HoiCore } from '@hoi/core-api';
import { createMockCore } from '../lib/core/mock-core';

async function seeded(): Promise<{ core: HoiCore; byName: (n: string) => Promise<CommunityView> }> {
  const core = createMockCore();
  await core.createIdentity('lena');
  const list = await core.communities();
  return { core, byName: async (n) => core.community(list.find((c) => c.name === n)!.id) };
}

const code = async (p: Promise<unknown>) => {
  try {
    await p;
  } catch (e) {
    return (e as HoiError).code;
  }
  return 'ok';
};

describe('mock core', () => {
  it('reads warmth for joined communities, coldest first, and takes carry requests for open ones', async () => {
    const { core, byName } = await seeded();
    const view = await core.warmth();
    const joined = (await core.communities()).filter((c) => c.joined);
    expect(view.communities.map((w) => w.community).sort()).toEqual(joined.map((c) => c.id).sort());
    expect(view.ownDurable).toBe(false);
    expect(view.directory).toEqual({ catalogNodes: 0, discoverable: 0, carryNodes: 0 });
    for (let i = 1; i < view.communities.length; i++) expect(view.communities[i - 1].score).toBeLessThanOrEqual(view.communities[i].score);
    for (const w of view.communities) {
      expect(w.level).toBe('cold');
      expect(w.copies).toBe(1);
      expect(w.durableCopies).toBe(0);
      expect(w.complete).toBe(1);
      expect(w.live).toBe(false);
      expect(w.records).toBeGreaterThan(0);
    }
    const privacy = await byName('privacy');
    expect((await core.communityWarmth(privacy.id)).community).toBe(privacy.id);
    expect(await core.requestCarry(privacy.id)).toEqual({ community: privacy.id, asked: 0, carrying: 0 });
    await core.pair('hoi://pair?u=ws%3A%2F%2F127.0.0.1%3A7842&t=mock-token-4f2a&n=lena');
    const warmer = await core.warmth();
    expect(warmer.communities.every((w) => w.live && w.fresh)).toBe(true);
    expect(warmer.communities.some((w) => w.level !== 'cold')).toBe(true);
    expect(warmer.directory.carryNodes).toBe(1);
    expect((await core.requestCarry(privacy.id)).asked).toBe(1);
  });

  it('starts with no identity, then seeds the prototype world', async () => {
    const core = createMockCore();
    expect((await core.boot()).hasIdentity).toBe(false);
    expect(await core.me()).toBeNull();
    const me = await core.createIdentity('lena');
    expect(me.root.startsWith('did:key:z6Mk')).toBe(true);
    const names = (await core.communities()).map((c) => c.name);
    expect(names).toEqual(expect.arrayContaining(['privacy', 'selfhosting', 'cryptography', 'hackspace', 'opsec']));
    expect((await core.net()).label).toBe('only in this browser');
  });

  it('keeps owner changes to owners and asks for cosigners above threshold 1', async () => {
    const { core, byName } = await seeded();
    const hackspace = await byName('hackspace'); // you are a mod there
    expect(hackspace.role).toBe('mod');
    expect(await code(core.setLimits(hackspace.id, hackspace.limits))).toBe('not-allowed');
    expect(hackspace.flags.length).toBeGreaterThan(0);
    expect(hackspace.pending.length).toBeGreaterThan(0);

    const privacy = await byName('privacy'); // owner, 3 of 4
    const r = await core.setLimits(privacy.id, { ...privacy.limits, newcomerPostsPerDay: 5 });
    expect(r).toMatchObject({ status: 'needs-cosign', have: 1, need: 3 });

    const opsec = await byName('opsec'); // sole owner, rav moderates
    const p = await core.setRules(opsec.id, '1. Be careful.');
    expect(p.status).toBe('published');
    // published as a proposal: the people established there decide
    const after = await core.community(opsec.id);
    expect(after.rules?.version).toBe(1);
    expect(after.proposals[0]).toMatchObject({ kind: 'rules', status: 'open', myAnswer: 'agree', canAnswer: false });
  });

  it('publishes the proposal when the last cosigner signs', async () => {
    const { core, byName } = await seeded();
    const r = await core.cosign('hoi://cosign?p=7a1e44c0b9d2f3a8');
    expect(r.status).toBe('published');
    const privacy = await byName('privacy');
    expect(privacy.limits.newcomerPostsPerDay).toBe(2);
    expect(privacy.proposals[0].changes).toEqual([{ field: 'newcomer_posts_per_day', from: '2', to: '3' }]);
  });

  it('applies a change once everyone who decides agrees', async () => {
    const { core, byName } = await seeded();
    const sh = await byName('selfhosting');
    const p = sh.proposals[0];
    expect(p).toMatchObject({ status: 'open', canAnswer: true });
    expect(await code(core.review(p.id, 'veto'))).toBe('invalid');
    await core.review(p.id, 'veto', 'too short for slow threads');
    expect((await byName('selfhosting')).proposals[0].objecting[0].reason).toBe('too short for slow threads');
    await core.review(p.id, 'ok');
    const now = await byName('selfhosting');
    const q = now.proposals[0];
    expect(q.myAnswer).toBe('agree');
    if (q.status === 'passed' || q.status === 'in-effect') expect(now.limits.maxRetention).toBe(60 * 86_400);
    else expect(now.limits.maxRetention).toBe(90 * 86_400);
  });

  it('lets owners retire someone who left for good, never an owner', async () => {
    const { core, byName } = await seeded();
    const opsec = await byName('opsec'); // sole owner
    expect(await code(core.retire(opsec.id, opsec.owners[0].key))).toBe('not-allowed');
    const someone = 'cd'.repeat(32);
    const r = await core.retire(opsec.id, someone);
    expect(r.status).toBe('published');
    if (r.status === 'published') expect(r.entry.action).toBe('retire');
  });

  it('creates communities with co-owners and a threshold', async () => {
    const { core } = await seeded();
    const other = 'ab'.repeat(32);
    const c = await core.createCommunity({ name: 'bikes', description: '', color: 'mint', coOwners: [other], threshold: 2 });
    expect(c.owners.length).toBe(2);
    expect(c.threshold).toBe(2);
    expect((await core.addMod(c.id, 'cd'.repeat(32))).status).toBe('needs-cosign');
    expect(await code(core.createCommunity({ name: 'x', description: '', color: 'mint', threshold: 3 }))).toBe('invalid');
  });

  it('round-trips invites with signed details', async () => {
    const { core, byName } = await seeded();
    const opsec = await byName('opsec');
    const inv = await core.invite(opsec.id);
    const other = createMockCore();
    const pv = await other.previewInvite(inv.uri);
    expect(pv).toMatchObject({ verified: true, alreadyJoined: false, community: { name: 'opsec', color: 'sun' } });
    await other.createIdentity('tobi');
    expect((await other.join(inv.uri)).name).toBe('opsec');
    expect(await code(other.previewInvite('https://example.com'))).toBe('bad-invite');
  });

  it('pairs with a home node', async () => {
    const { core } = await seeded();
    expect(await code(core.pair('hoi://nope'))).toBe('invalid');
    const net = await core.pair('hoi://pair?u=wss://home.example:7842&t=abc');
    expect(net).toMatchObject({ mode: 'home-node', label: 'via your home node' });
    expect((await core.unpair()).mode).toBe('local');
    // the web form a phone's camera opens, with a WebRTC route and no WebSocket
    const web = await core.pair('https://hoi.sh/app/#/pair?t=abc&n=mira%27s%20computer&w=fp@192.168.1.2:7844');
    expect(web).toMatchObject({ mode: 'home-node', label: "via mira's computer", homeNode: { via: 'webrtc' } });
    expect(web.homeNode?.pairingWeb).toContain('#/pair?t=abc');
  });

  it('links a new device through the home node and counts traffic without content', async () => {
    const core = createMockCore();
    await core.boot();
    expect(await code(core.linkHome('Phone'))).toBe('not-allowed');
    await core.pair('hoi://pair?u=ws://127.0.0.1:7842&t=abc');
    const checks = new Promise<string[]>((resolve) => core.on((e) => e.type === 'linkCheck' && resolve(e.checks)));
    const linked = new Promise<string>((resolve) => core.on((e) => e.type === 'linked' && resolve(e.me.name)));
    expect((await core.linkHome('Phone')).sent).toBe(1);
    expect(await checks).toEqual([expect.stringMatching(/^\d{4}$/)]);
    expect(await linked).toBe('lena');
    expect((await core.me())?.hasRoot).toBe(false);
    const t = await core.traffic();
    expect(t.minutes).toHaveLength(60);
    expect(t.relay.possible).toBe(false);
    expect(JSON.stringify(t)).not.toMatch(/text|title/);
  }, 10_000);

  it('emits stamp events around a post and lets mods remove it', async () => {
    const { core, byName } = await seeded();
    const opsec = await byName('opsec');
    const events: string[] = [];
    core.on((e) => {
      if (e.type === 'stamp') events.push(e.phase);
    });
    const post = await core.createPost({ community: opsec.id, title: 'hello', text: 'there' });
    expect(events).toEqual(['start', 'done']);
    const entry = await core.remove(post.id, 'test');
    expect(entry.action).toBe('remove');
    expect((await core.post(post.id)).removed?.reason).toBe('test');
  }, 15_000);

  it('shares the holders it sees only from a desktop', async () => {
    const core = createMockCore();
    expect((await core.settings()).shareHolders).toBe(false);
    expect((await core.setSettings({ shareHolders: true })).shareHolders).toBe(false);
  });

  it('offers "Use less data" to browsers, off at first', async () => {
    const core = createMockCore();
    expect((await core.settings()).lessData).toBe(false);
    expect((await core.setSettings({ lessData: true })).lessData).toBe(true);
    expect((await core.setSettings({ lessData: false })).lessData).toBe(false);
  });

  it('says the node you connect through cannot tell a post is yours only on your own posts in open communities', async () => {
    const { core, byName } = await seeded();
    const opsec = await byName('opsec');
    const mine = await core.createPost({ community: opsec.id, title: 'mine', text: 'whose is it?' });
    const open = opsec.limits.admission === 'open';
    expect(Boolean((await core.post(mine.id)).checks.addressHidden)).toBe(open);
    const theirs = (await core.feed({ community: opsec.id, sort: 'fresh' })).find((p) => !p.mine);
    if (theirs) expect((await core.post(theirs.id)).checks.addressHidden).toBeUndefined();
  }, 15_000);

  it('refuses the combined clear while deletions need this device, and a stale look after', async () => {
    const { core, byName } = await seeded();
    const before = await core.cleanupReadiness();
    expect(before.ready).toBe(true);
    const mine = await core.createPost({ community: (await byName('opsec')).id, title: 'mine', text: 'to be deleted' });
    const listed = await core.cleanupReadiness();
    expect(listed.unlisted).toBe(before.unlisted + 1);
    expect(await code(core.clearAfterDeletion(before.inventory, before.deletions))).toBe('cleanup-changed');
    const preview = await core.previewBulkDeletion();
    expect(preview.communities.flatMap((c) => c.posts)).toContain(mine.id);
    await core.confirmBulkDeletion(preview.snapshot);
    const blocked = await core.cleanupReadiness();
    expect(blocked).toMatchObject({ ready: false, unlisted: 0 });
    expect(blocked.blocked).toEqual([{ reason: 'in-effect', count: preview.count }]);
    expect(await code(core.clearAfterDeletion(blocked.inventory, blocked.deletions))).toBe('cleanup-blocked');
    expect((await core.me())?.name).toBe('lena');
  }, 15_000);
});
