import { describe, expect, it } from 'vitest';
import { LogStore, crc32, type SyncHandle } from '../lib/core/opfs';
import { apply, decodeSnapshot, migrate, snapshot, vaultHeader, type Change } from '../lib/core/idb';
import { testVault } from './fixtures/vault.mjs';

/** A file in memory, as a sync access handle sees it. */
class MemFile implements SyncHandle {
  data = new Uint8Array(0);
  read(buffer: Uint8Array, { at }: { at: number }) {
    const n = Math.max(0, Math.min(buffer.length, this.data.length - at));
    buffer.set(this.data.subarray(at, at + n));
    return n;
  }
  write(buffer: Uint8Array, { at }: { at: number }) {
    if (at + buffer.length > this.data.length) {
      const grown = new Uint8Array(at + buffer.length);
      grown.set(this.data);
      this.data = grown;
    }
    this.data.set(buffer, at);
    return buffer.length;
  }
  truncate(size: number) {
    const cut = new Uint8Array(size);
    cut.set(this.data.subarray(0, Math.min(size, this.data.length)));
    this.data = cut;
  }
  flush() {}
  getSize() { return this.data.length; }
  close() {}
}

function disk() {
  const files = new Map<string, MemFile>();
  const open = async (name: string) => {
    if (!files.has(name)) files.set(name, new MemFile());
    return files.get(name)!;
  };
  return { files, open };
}

const bytes = (s: string) => new TextEncoder().encode(s);
const put = (id: string, text: string, community: string | null = 'c'): Change => ({ op: 'put', id, community, expiresAt: 99, wire: bytes(text) });

describe('records in OPFS log files', () => {
  it('keeps what was written across reopening, in key order', async () => {
    const d = disk();
    const s = await LogStore.open(d.open);
    s.write([put('b', 'second'), put('a', 'first', null), { op: 'blob', key: 'keys', value: bytes('k') }]);
    s.write([{ op: 'delete', id: 'b' }, { op: 'blob', key: 'state', value: bytes('s') }]);
    s.close();
    const again = (await LogStore.open(d.open)).snapshot();
    expect(again.records).toEqual([['a', { wire: bytes('first'), community: null, expiresAt: 99 }]]);
    expect(again.blobs.map(([k]) => k)).toEqual(['keys', 'state']);
  });

  it('drops a write that was cut short, and only that one', async () => {
    const d = disk();
    const s = await LogStore.open(d.open);
    s.write([put('a', 'kept')]);
    s.write([put('b', 'lost')]);
    const file = d.files.get('log-a')!;
    file.truncate(file.getSize() - 3);
    const whole = file.getSize();
    const again = await LogStore.open(d.open);
    expect(again.snapshot().records.map(([id]) => id)).toEqual(['a']);
    expect(file.getSize()).toBeLessThan(whole);
    again.write([put('c', 'after')]);
    expect((await LogStore.open(d.open)).snapshot().records.map(([id]) => id)).toEqual(['a', 'c']);
  });

  it('a damaged frame ends the log, and a store that never finished its first write is empty', async () => {
    const d = disk();
    const s = await LogStore.open(d.open);
    s.write([put('a', 'kept')]);
    const end = d.files.get('log-a')!.getSize();
    s.write([put('b', 'flipped')]);
    d.files.get('log-a')!.data[end + 14] ^= 1;
    expect((await LogStore.open(d.open)).snapshot().records.map(([id]) => id)).toEqual(['a']);

    const fresh = disk();
    const first = await LogStore.open(fresh.open);
    first.write([put('a', 'never finished')]);
    fresh.files.get('log-a')!.truncate(20);
    expect((await LogStore.open(fresh.open)).snapshot()).toEqual({ records: [], blobs: [] });
  });

  it('refuses to open a first frame that is whole but damaged, and changes nothing', async () => {
    const d = disk();
    const s = await LogStore.open(d.open);
    s.write([put('a', 'only copy')]);
    const file = d.files.get('log-a')!;
    file.data[file.getSize() - 1] ^= 1;
    const before = file.data.slice();
    await expect(LogStore.open(d.open)).rejects.toThrow('damaged');
    expect(file.data).toEqual(before);
  });

  it('compacts into the other file, and a crash on either side of it loses nothing', async () => {
    const d = disk();
    const s = await LogStore.open(d.open);
    for (let i = 0; i < 50; i++) s.write([put('same', `version ${i}`), { op: 'blob', key: 'state', value: bytes(`state ${i}`) }]);
    const before = s.snapshot();
    const a = d.files.get('log-a')!;
    const b = d.files.get('log-b')!;
    const old = a.data.slice();
    s.compact();
    expect(a.getSize()).toBe(0);
    expect(b.getSize()).toBeGreaterThan(0);
    expect(b.getSize()).toBeLessThan(old.length);
    expect((await LogStore.open(d.open)).snapshot()).toEqual(before);
    // the new file written, the old not yet emptied: the newer generation counts
    a.data = old.slice();
    expect((await LogStore.open(d.open)).snapshot()).toEqual(before);
    // the new file cut short: the old one still holds everything
    b.truncate(30);
    expect((await LogStore.open(d.open)).snapshot()).toEqual(before);
    // and writes go on from there
    const reopened = await LogStore.open(d.open);
    reopened.write([put('next', 'after the crash')]);
    expect((await LogStore.open(d.open)).snapshot().records.map(([id]) => id)).toEqual(['next', 'same']);
  });

  it('compacts by itself once the log is mostly old versions', async () => {
    const d = disk();
    const s = await LogStore.open(d.open);
    const big = new Uint8Array(256 * 1024).fill(7);
    for (let i = 0; i < 24; i++) s.write([{ op: 'blob', key: 'media', value: big.map((v) => v ^ (i & 1)) }]);
    const total = d.files.get('log-a')!.getSize() + (d.files.get('log-b')?.getSize() ?? 0);
    // 6 MB written; the log never holds much more than 4 MiB before it compacts
    expect(total).toBeLessThan(12 * big.length);
    expect((await LogStore.open(d.open)).snapshot().blobs[0][1]).toEqual(big.map((v) => v ^ 1));
  });

  it('checks frames with the usual CRC-32', () => {
    expect(crc32(bytes('123456789'))).toBe(0xcbf43926);
  });

  it('holds the vault and its manifest like IndexedDB does, and catches a record taken out', async () => {
    const d = disk();
    const store = await LogStore.open(d.open);
    const vault = testVault();
    await migrate(store, await snapshot(store), vault);
    await apply(store, [put('post', 'a private post'), { op: 'blob', key: 'keys', value: bytes('identity secret') }], vault);
    store.close();
    const reopened = await LogStore.open(d.open);
    const saved = await snapshot(reopened);
    expect(vaultHeader(saved)).toEqual(vault.header());
    expect(saved.records[0][1].wire).not.toEqual(bytes('a private post'));
    const plain = decodeSnapshot(saved, vault);
    expect(plain.records).toEqual([bytes('a private post')]);
    expect(Object.fromEntries(plain.blobs)).toEqual({ keys: bytes('identity secret') });
    // someone with the files deletes a record behind the vault's back
    reopened.write([{ op: 'delete', id: 'post' }]);
    expect(() => decodeSnapshot(reopened.snapshot(), vault)).toThrow("incomplete or damaged");
  });
});
