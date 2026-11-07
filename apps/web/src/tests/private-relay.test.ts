import { describe, it, expect, vi } from 'vitest';
import type { HoiCore, PrivateGroup, PrivateRelayObject } from '@hoi/core-api';
import { syncPrivateGroup } from '../lib/private-relay';
import { toBase64Url } from '../lib/core/rpc-core';

const room = 'a'.repeat(64);
const next = 'e'.repeat(64);
const community = 'c'.repeat(64);
const group: PrivateGroup = { community, room, epoch: 1, active: true, isOwner: true, members: [], pending: 0, received: [], transport: 'waiting', mailboxes: [], pieces: 'whole', followsRecords: false, invitations: [], committers: 1 };
const json = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status });
const hash = async (bytes: Uint8Array) => [...new Uint8Array(await crypto.subtle.digest('SHA-256', new Uint8Array(bytes)))].map(b => b.toString(16).padStart(2, '0')).join('');
function sealed(fill = 7): Uint8Array<ArrayBuffer> {
  const bytes = new Uint8Array(4096).fill(fill);
  bytes[0] = 0; bytes[1] = 247;
  return bytes;
}
async function object(bytes = sealed(), inRoom = room): Promise<PrivateRelayObject> {
  return { id: await hash(bytes), room: inRoom, body: toBase64Url(bytes), size: bytes.length, expires: 1_900_000_000, signature: 'b'.repeat(128) };
}
function core(outbox: PrivateRelayObject[] = [], received: string[] = []) {
  let current = { ...group, received };
  return {
    get room() { return current.room; },
    set room(value: string) { current = { ...current, room: value }; },
    privateGroup: vi.fn(async () => current),
    privateOutbox: vi.fn().mockResolvedValue(outbox),
    privateReceive: vi.fn(async (_community: string, _body: string) => current),
    privateAck: vi.fn(async () => current),
    privateRead: vi.fn(async (_community: string, targets: string[]) => ({
      room: current.room, reads: targets.map((_, index) => ({ time: 1_800_000_000, nonce: index.toString(16).padStart(32, '0'), signature: 'f'.repeat(128) })),
    })),
  };
}
const headers = (init?: RequestInit) => new Headers(init?.headers);
const receipt = (item: PrivateRelayObject, status = 200) => json({ id: item.id, sequence: 1, size: item.size, expires: item.expires }, status);

describe('encrypted relay replication', () => {
  it('aborts a slow relay immediately when locking and never advances to another replica or upload', async () => {
    const controller = new AbortController();
    const client = core([await object()]);
    const fetcher = vi.fn((_url: string | URL | Request, init?: RequestInit) => new Promise<Response>((_resolve, reject) => {
      init?.signal?.addEventListener('abort', () => reject(init.signal!.reason), { once: true });
    }));
    const job = syncPrivateGroup(client as unknown as HoiCore, community, ['/one', '/two'], fetcher as typeof fetch, controller.signal);
    const rejection = expect(job).rejects.toMatchObject({ name: 'AbortError' });
    await vi.waitFor(() => expect(fetcher).toHaveBeenCalledOnce());
    controller.abort();
    await rejection;
    expect(fetcher).toHaveBeenCalledOnce();
    expect(client.privateOutbox).not.toHaveBeenCalled();
    expect(client.privateReceive).not.toHaveBeenCalled();
  });

  it('cancels an unfinished response body on lock without waiting for relay timeout', async () => {
    const controller = new AbortController();
    const client = core();
    const cancel = vi.fn();
    const fetcher = vi.fn(async () => new Response(new ReadableStream({ cancel })));
    const job = syncPrivateGroup(client as unknown as HoiCore, community, ['/relay'], fetcher, controller.signal);
    const rejection = expect(job).rejects.toMatchObject({ name: 'AbortError' });
    await vi.waitFor(() => expect(fetcher).toHaveBeenCalledOnce());
    controller.abort();
    await rejection;
    expect(cancel).toHaveBeenCalledOnce();
    expect(client.privateOutbox).not.toHaveBeenCalled();
  });

  it('signs every listing and download in the core, once per request, and sends no writer or grant metadata', async () => {
    const stored = await object();
    const client = core();
    const fetcher = vi.fn(async (url: string | URL | Request) => String(url).includes('?after=')
      ? json({ objects: [{ id: stored.id, sequence: 1, size: stored.size }] })
      : new Response(sealed()));
    const result = await syncPrivateGroup(client as unknown as HoiCore, community, ['https://one.test/relay'], fetcher as typeof fetch);
    expect(result.error).toBeUndefined();
    expect(client.privateRead.mock.calls.map(([, targets]) => targets)).toEqual([['objects?after=0'], [`objects/${stored.id}`]]);
    const [[listUrl, listInit], [getUrl, getInit]] = fetcher.mock.calls as unknown as [string, RequestInit][];
    expect(listUrl).toBe(`https://one.test/relay/v2/rooms/${room}/objects?after=0`);
    expect(getUrl).toBe(`https://one.test/relay/v2/rooms/${room}/objects/${stored.id}`);
    expect(headers(listInit).get('x-hoi-read-signature')).toBe('f'.repeat(128));
    expect(headers(listInit).get('x-hoi-read-nonce')).toMatch(/^[0-9a-f]{32}$/);
    expect(headers(getInit).get('x-hoi-read-time')).toBe('1800000000');
    expect(client.privateReceive).toHaveBeenCalledWith(community, stored.body);
    for (const [, init] of fetcher.mock.calls as unknown as [string, RequestInit][]) {
      expect([...headers(init).keys()].some(name => /author|grant/.test(name))).toBe(false);
      expect(init).toMatchObject({ credentials: 'omit', redirect: 'error', referrerPolicy: 'no-referrer', cache: 'no-store' });
    }
  });

  it('follows a commit into the next epoch room and leaves the old room’s remaining objects unflagged', async () => {
    const commit = await object(sealed(1));
    const stale = await object(sealed(2));
    const fresh = await object(sealed(3), next);
    const client = core();
    client.privateReceive.mockImplementation(async (_community: string, body: string) => {
      if (body === commit.body) client.room = next;
      else if (body === stale.body) throw new Error('not for this epoch');
      return client.privateGroup();
    });
    const listings: Record<string, PrivateRelayObject[]> = { [room]: [commit, stale], [next]: [fresh] };
    const bodies = new Map([commit, stale, fresh].map(item => [item.id, Uint8Array.from(atob(item.body.replace(/-/g, '+').replace(/_/g, '/')), c => c.charCodeAt(0))]));
    const fetcher = vi.fn(async (url: string | URL | Request) => {
      const [, inRoom, rest] = /\/v2\/rooms\/([0-9a-f]{64})\/(.*)$/.exec(String(url))!;
      if (rest.startsWith('objects?after=')) return json({ objects: listings[inRoom].map((item, index) => ({ id: item.id, sequence: index + 1, size: item.size })) });
      return new Response(bodies.get(rest.slice('objects/'.length)));
    });
    const result = await syncPrivateGroup(client as unknown as HoiCore, community, ['/relay'], fetcher as typeof fetch);
    expect(result.error).toBeUndefined();
    expect(client.privateReceive.mock.calls.map(([, body]) => body)).toEqual([commit.body, fresh.body]);
    expect(fetcher.mock.calls.map(([url]) => String(url))).toContain(`/relay/v2/rooms/${next}/objects?after=0`);
  });

  it('uploads each object to its own room and acknowledges only after every configured replica accepts it', async () => {
    const commit = await object(sealed(4), room);
    const item = await object(sealed(5), next);
    const client = core([commit, item]);
    const fetcher = vi.fn(async (url: string | URL | Request, init?: RequestInit) => {
      if (init?.method !== 'PUT') return json({ objects: [] });
      return receipt(String(url).includes(commit.id) ? commit : item, 201);
    });
    const result = await syncPrivateGroup(client as unknown as HoiCore, community, ['https://one.test/relay', 'https://two.test/relay'], fetcher as typeof fetch);
    expect(result.replicas).toBe(2);
    expect(client.privateAck).toHaveBeenCalledWith(community, [commit.id, item.id]);
    const uploads = fetcher.mock.calls.filter(([, options]) => options?.method === 'PUT') as unknown as [string, RequestInit][];
    expect(uploads.map(([url]) => url)).toEqual([
      `https://one.test/relay/v2/rooms/${room}/objects/${commit.id}`, `https://two.test/relay/v2/rooms/${room}/objects/${commit.id}`,
      `https://one.test/relay/v2/rooms/${next}/objects/${item.id}`, `https://two.test/relay/v2/rooms/${next}/objects/${item.id}`,
    ]);
    for (const [, options] of uploads) {
      expect(options).toMatchObject({ credentials: 'omit', redirect: 'error', referrerPolicy: 'no-referrer', cache: 'no-store' });
      expect([...headers(options).keys()].sort()).toEqual(['content-type', 'x-hoi-expires', 'x-hoi-signature']);
      expect(JSON.stringify(options?.headers)).not.toContain(community);
    }
  });

  it('retains ciphertext for retry if one replica fails or lies about the stored hash', async () => {
    const item = await object();
    const client = core([item]);
    const fetcher = vi.fn(async (url: string | URL | Request, init?: RequestInit) => {
      if (init?.method !== 'PUT') return json({ objects: [] });
      return String(url).includes('two.test') ? receipt({ ...item, id: 'd'.repeat(64) }) : receipt(item);
    });
    const result = await syncPrivateGroup(client as unknown as HoiCore, community, ['https://one.test/relay', 'https://two.test/relay'], fetcher as typeof fetch);
    expect(result.error).toBeTruthy();
    expect(client.privateAck).not.toHaveBeenCalled();
  });

  it('rejects modified ciphertext before the core sees it and skips already processed object hashes', async () => {
    const item = await object();
    const client = core();
    const fetcher = vi.fn(async (url: string | URL | Request) => String(url).includes('?after=')
      ? json({ objects: [{ id: item.id, sequence: 1, size: item.size }] })
      : new Response(sealed(9)));
    const result = await syncPrivateGroup(client as unknown as HoiCore, community, ['/relay'], fetcher as typeof fetch);
    expect(result.error).toMatch(/verified/);
    expect(client.privateReceive).not.toHaveBeenCalled();
    const remembered = core([], [item.id]);
    fetcher.mockClear();
    await syncPrivateGroup(remembered as unknown as HoiCore, community, ['/relay'], fetcher as typeof fetch);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(remembered.privateRead).toHaveBeenCalledTimes(1);
    expect(remembered.privateReceive).not.toHaveBeenCalled();
  });

  it('refuses a listing whose sizes are not padded relay objects', async () => {
    const client = core();
    const fetcher = vi.fn(async () => json({ objects: [{ id: 'd'.repeat(64), sequence: 1, size: 12 }] }));
    const result = await syncPrivateGroup(client as unknown as HoiCore, community, ['/relay'], fetcher as typeof fetch);
    expect(result.replicas).toBe(0);
    expect(fetcher).toHaveBeenCalledOnce();
  });

  it('does not call a successful upload a completed sync when the relay withheld its listing', async () => {
    const item = await object();
    const client = core([item]);
    const fetcher = vi.fn(async (_url: string | URL | Request, init?: RequestInit) => init?.method === 'PUT' ? receipt(item) : json({}, 503));
    const result = await syncPrivateGroup(client as unknown as HoiCore, community, ['/relay'], fetcher as typeof fetch);
    expect(client.privateAck).toHaveBeenCalledWith(community, [item.id]);
    expect(result.lastSync).toBeUndefined();
    expect(result.error).toBeTruthy();
  });

  it('has no network fallback for a removed device or unavailable private relay', async () => {
    const client = core();
    const fetcher = vi.fn();
    await syncPrivateGroup(client as unknown as HoiCore, community, [], fetcher);
    client.privateGroup.mockResolvedValue({ ...group, active: false, room: '' });
    await syncPrivateGroup(client as unknown as HoiCore, community, ['/relay'], fetcher);
    expect(fetcher).not.toHaveBeenCalled();
    expect(client.privateOutbox).not.toHaveBeenCalled();
    expect(client.privateRead).not.toHaveBeenCalled();
  });

  it('cancels an oversized streaming response from an untrusted relay', async () => {
    const client = core();
    const cancel = vi.fn();
    const fetcher = vi.fn(async () => new Response(new ReadableStream({
      start(controller) {
        controller.enqueue(new Uint8Array(65_536));
        controller.enqueue(new Uint8Array(1));
      }, cancel,
    })));
    const result = await syncPrivateGroup(client as unknown as HoiCore, community, ['/relay'], fetcher);
    expect(cancel).toHaveBeenCalled();
    expect(result.replicas).toBe(0);
    expect(result.error).toBeTruthy();
    expect(client.privateReceive).not.toHaveBeenCalled();
  });
});
