import { describe, expect, it } from 'vitest';
import { LogStore, StoreLost, crc32, type OpenFile, type SyncHandle } from '../lib/core/opfs';
import { apply, decodeSnapshot, migrate, snapshot, vaultHeader, type Change } from '../lib/core/idb';
import { testVault } from './fixtures/vault.mjs';

/** A file in memory. */
class MemFile {
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
  getSize() { return this.data.length; }
}

/** A sync access handle on a file: once closed, by its holder or by the browser, it throws as a browser's does. */
class MemHandle implements SyncHandle {
  closed = false;
  constructor(private file: MemFile) {}
  private get open() {
    if (this.closed) throw new DOMException('SyncAccessHandle is closed', 'InvalidStateError');
    return this.file;
  }
  read(buffer: Uint8Array, options: { at: number }) { return this.open.read(buffer, options); }
  write(buffer: Uint8Array, options: { at: number }) { return this.open.write(buffer, options); }
  truncate(size: number) { this.open.truncate(size); }
  flush() { void this.open; }
  getSize() { return this.open.getSize(); }
  close() { this.closed = true; }
}

function disk() {
  const files = new Map<string, MemFile>();
  const handles: MemHandle[] = [];
  const open = async (name: string) => {
    if (!files.has(name)) files.set(name, new MemFile());
    const h = new MemHandle(files.get(name)!);
    handles.push(h);
    return h;
  };
  /** What Firefox does when a site's storage is cleared: every handle closes, whoever holds it. */
  const closeAll = () => handles.forEach((h) => h.close());
  return { files, handles, open, closeAll };
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

  it('opens its files again when the browser closed them, and saves the change', async () => {
    const d = disk();
    const store = await LogStore.open(d.open);
    const vault = testVault();
    await migrate(store, await snapshot(store), vault);
    await apply(store, [put('before', 'saved before')], vault);
    d.closeAll();
    await apply(store, [put('after', 'saved after')], vault);
    store.close();
    expect(d.handles.every((h) => h.closed)).toBe(true);
    const plain = decodeSnapshot(await snapshot(await LogStore.open(d.open)), vault);
    expect(plain.records).toEqual([bytes('saved after'), bytes('saved before')]);
  });

  it('writes nothing to files that were emptied while closed, and says they are lost', async () => {
    const d = disk();
    const store = await LogStore.open(d.open);
    const vault = testVault();
    await migrate(store, await snapshot(store), vault);
    d.closeAll();
    for (const f of d.files.values()) f.truncate(0); // the site's data cleared
    const saving = apply(store, [put('post', 'not saved')], vault);
    await expect(saving).rejects.toBeInstanceOf(StoreLost);
    await expect(saving).rejects.toThrow('changed while they were closed');
    expect([...d.files.values()].map((f) => f.getSize())).toEqual([0, 0]);
    expect(d.handles.every((h) => h.closed)).toBe(true);
  });

  it('keeps no files it could not open again, or that opened after it was closed', async () => {
    const d = disk();
    let gate: Promise<void> = Promise.resolve();
    let refuse = '';
    const open: OpenFile = async (name) => {
      await gate;
      if (name === refuse) throw new DOMException('gone', 'NotFoundError');
      return d.open(name);
    };
    const store = await LogStore.open(open);
    store.write([put('a', 'kept')]);
    d.closeAll();

    refuse = 'log-b'; // the first file opens again, the second does not
    await expect(store.reacquire()).rejects.toBeInstanceOf(StoreLost);
    expect(d.handles.every((h) => h.closed)).toBe(true);

    refuse = '';
    let opened!: () => void;
    gate = new Promise((r) => (opened = r));
    const reopening = store.reacquire();
    store.close(); // locked, or another tab took over
    opened();
    await expect(reopening).rejects.toThrow('It was closed.');
    expect(d.handles.every((h) => h.closed)).toBe(true);
  });
});
