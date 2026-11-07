import { describe, expect, it, vi } from 'vitest';
import type { HoiCore } from '@hoi/core-api';
import { bootstrapSandbox, canaryUrl, localSandboxLocation, parseSandboxContext, sandboxStorageName, type SandboxContext } from '../lib/sandbox';
import { openIdb } from '../lib/core/idb';
import { indexedDB } from 'fake-indexeddb';

const context: SandboxContext = {
  session: 'session-1', role: 'member', name: 'Sandbox member',
  pairing: 'hoi://pair?u=ws%3A%2F%2F127.0.0.1%3A7842&t=test',
  invite: 'hoi://join?c=test', community: 'a'.repeat(64),
};

describe('sandbox boundaries', () => {
  it('only enables the bridge on the three dedicated local app origins', () => {
    for (const host of ['localhost', '127.0.0.1', '[::1]']) {
      for (const port of [7071, 7072, 7073]) expect(localSandboxLocation(new URL(`http://${host}:${port}/`))).toBe(true);
    }
    for (const url of ['https://hoi.sh/', 'http://localhost:5173/', 'http://evil.localhost:7071/', 'http://localhost.evil.test:7071/']) {
      expect(localSandboxLocation(new URL(url))).toBe(false);
    }
  });

  it('refuses invalid sessions and remote home-node pairing', () => {
    expect(parseSandboxContext(context)).toEqual(context);
    expect(() => parseSandboxContext({ ...context, session: '../normal-profile' })).toThrow();
    expect(() => parseSandboxContext({ ...context, role: 'admin' })).toThrow();
    expect(() => parseSandboxContext({ ...context, pairing: 'hoi://pair?u=wss%3A%2F%2Foutside.example&t=test' })).toThrow();
  });

  it('keeps different sessions, personas and the ordinary identity in separate databases', async () => {
    vi.stubGlobal('indexedDB', indexedDB);
    const names = ['hoi', sandboxStorageName(context), sandboxStorageName({ ...context, role: 'owner' }), sandboxStorageName({ ...context, session: 'session-2' })];
    const databases = await Promise.all(names.map((name) => openIdb(name)));
    expect(new Set(databases.map((db) => db.name)).size).toBe(4);
    const owner = databases[2].transaction('blobs', 'readwrite');
    owner.objectStore('blobs').put('owner-private-key', 'keys');
    await new Promise<void>((resolve) => { owner.oncomplete = () => resolve(); });
    for (const index of [0, 1, 3]) {
      const request = databases[index].transaction('blobs').objectStore('blobs').get('keys');
      expect(await new Promise((resolve) => { request.onsuccess = () => resolve(request.result); })).toBeUndefined();
    }
    databases.forEach((db) => db.close());
    vi.unstubAllGlobals();
  });

  it('recognizes only inert canary URLs with a complete run, agent and token', () => {
    const valid = 'https://sandbox-canary.example/run-1/agent-1/token-1';
    expect(canaryUrl(valid)).toBe(valid);
    for (const value of ['https://example.com/run/agent/token', valid + '?redirect=outside', valid + '#x', valid.replace('https:', 'http:'), valid.replace('.example/', '.example.evil.test/'), 'https://sandbox-canary.example/run/agent', 'https://name@sandbox-canary.example/run/agent/token']) {
      expect(canaryUrl(value)).toBeNull();
    }
  });
});

describe('real sandbox identity bootstrap', () => {
  const make = (existing = false) => ({
    me: vi.fn().mockResolvedValue(existing ? { root: 'existing-key' } : null),
    createIdentity: vi.fn().mockResolvedValue({}), linkRequest: vi.fn().mockResolvedValue('real-link-request'),
    linkComplete: vi.fn().mockResolvedValue({}), pair: vi.fn().mockResolvedValue({}), join: vi.fn().mockResolvedValue({}),
  });

  it('creates a separate ordinary identity for a member or watcher, then pairs and joins', async () => {
    for (const role of ['member', 'watcher'] as const) {
      const core = make();
      const post = vi.fn().mockResolvedValue({ ok: true });
      await bootstrapSandbox(core as unknown as HoiCore, { ...context, role }, post);
      expect(core.createIdentity).toHaveBeenCalledWith(context.name);
      expect(core.linkRequest).not.toHaveBeenCalled();
      expect(core.pair).toHaveBeenCalledWith(context.pairing);
      expect(core.join).toHaveBeenCalledWith(context.invite);
      expect(post).toHaveBeenCalledWith('visit', { community: context.community });
    }
  });

  it('links the owner through a signed device approval without creating a fabricated owner identity', async () => {
    const core = make();
    const post = vi.fn().mockResolvedValue({ approval: 'signed-approval' });
    await bootstrapSandbox(core as unknown as HoiCore, { ...context, role: 'owner' }, post);
    expect(post).toHaveBeenCalledWith('link', { request: 'real-link-request' });
    expect(core.linkComplete).toHaveBeenCalledWith('signed-approval');
    expect(core.createIdentity).not.toHaveBeenCalled();
  });

  it('reuses existing keys and fails setup if owner approval fails', async () => {
    const core = make(true);
    await bootstrapSandbox(core as unknown as HoiCore, context, vi.fn().mockResolvedValue({}));
    expect(core.createIdentity).not.toHaveBeenCalled();
    const fresh = make();
    await expect(bootstrapSandbox(fresh as unknown as HoiCore, { ...context, role: 'owner' }, vi.fn().mockResolvedValue({}))).rejects.toThrow('owner could not link');
    expect(fresh.join).not.toHaveBeenCalled();
  });
});
