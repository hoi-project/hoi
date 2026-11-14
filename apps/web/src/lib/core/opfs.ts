// Records and media in the origin-private file system, through sync access handles in
// the core worker: one append and one flush a write, instead of an IndexedDB transaction.
//
// A store is a directory with two log files. Each file is a run of frames: `HOIF`, the file's
// generation (u32), the body's length (u32), the body (a batch of changes) and a CRC-32 of the
// body. A frame is there whole or not at all: on opening, the first frame that is cut short or
// doesn't check ends the log, and the file is truncated there. The file whose first frame is
// valid and has the higher generation is the store. Compaction writes everything that is live
// as one frame into the other file under the next generation, then empties the old one; a crash
// in between leaves one of them whole. What goes in is already sealed by the vault (idb.ts): the
// files hold nothing a reader could use, and the vault's manifest catches anything removed.

import type { Change, Snapshot, StoredRecord } from './idb';

/** The part of `FileSystemSyncAccessHandle` a store uses. */
export interface SyncHandle {
  read(buffer: Uint8Array, options: { at: number }): number;
  write(buffer: Uint8Array, options: { at: number }): number;
  truncate(size: number): void;
  flush(): void;
  getSize(): number;
  close(): void;
}

/** Opens (creating) one file of a store. */
export type OpenFile = (name: string) => Promise<SyncHandle>;

/** A handle the browser closed: from then on every call on it throws this. */
export const handleClosed = (e: unknown) => (e as DOMException)?.name === 'InvalidStateError';

/** The browser closed a store's files, and they could not be opened again as the store left them. */
export class StoreLost extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = 'StoreLost';
  }
}

const MAGIC = 0x464f4948; // "HOIF", little-endian
const HEAD = 12;
const FILES = ['log-a', 'log-b'] as const;
/** Compact once the log passes this and three times what is live. */
const COMPACT_AT = 4 << 20;

const CRC = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

export function crc32(data: Uint8Array): number {
  let c = 0xffffffff;
  for (const b of data) c = CRC[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

class Writer {
  private buf = new Uint8Array(256);
  private view = new DataView(this.buf.buffer);
  len = 0;
  private room(n: number) {
    if (this.len + n <= this.buf.length) return;
    let size = this.buf.length * 2;
    while (size < this.len + n) size *= 2;
    const next = new Uint8Array(size);
    next.set(this.buf.subarray(0, this.len));
    this.buf = next;
    this.view = new DataView(next.buffer);
  }
  u8(v: number) { this.room(1); this.buf[this.len++] = v; }
  u16(v: number) { this.room(2); this.view.setUint16(this.len, v, true); this.len += 2; }
  u32(v: number) { this.room(4); this.view.setUint32(this.len, v, true); this.len += 4; }
  f64(v: number) { this.room(8); this.view.setFloat64(this.len, v, true); this.len += 8; }
  bytes(v: Uint8Array) { this.room(v.length); this.buf.set(v, this.len); this.len += v.length; }
  str(v: string | null) {
    if (v === null) return this.u16(0xffff);
    const b = new TextEncoder().encode(v);
    if (b.length >= 0xffff) throw new Error('A storage key is too long.');
    this.u16(b.length);
    this.bytes(b);
  }
  blob(v: Uint8Array | null) {
    if (v === null) return this.u32(0xffffffff);
    this.u32(v.length);
    this.bytes(v);
  }
  done(): Uint8Array { return this.buf.subarray(0, this.len); }
}

class Reader {
  private view: DataView;
  at = 0;
  constructor(private buf: Uint8Array) { this.view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength); }
  private need(n: number) { if (this.at + n > this.buf.length) throw new Error('short'); }
  get left() { return this.buf.length - this.at; }
  u8() { this.need(1); return this.buf[this.at++]; }
  u16() { this.need(2); const v = this.view.getUint16(this.at, true); this.at += 2; return v; }
  u32() { this.need(4); const v = this.view.getUint32(this.at, true); this.at += 4; return v; }
  f64() { this.need(8); const v = this.view.getFloat64(this.at, true); this.at += 8; return v; }
  bytes(n: number) { this.need(n); const v = this.buf.slice(this.at, this.at + n); this.at += n; return v; }
  str(): string | null {
    const n = this.u16();
    return n === 0xffff ? null : new TextDecoder('utf-8', { fatal: true }).decode(this.bytes(n));
  }
  blob(): Uint8Array | null {
    const n = this.u32();
    return n === 0xffffffff ? null : this.bytes(n);
  }
}

function encodeBody(changes: Change[]): Uint8Array {
  const w = new Writer();
  for (const c of changes) {
    switch (c.op) {
      case 'put': w.u8(1); w.str(c.id); w.str(c.community); w.f64(c.expiresAt); w.blob(c.wire); break;
      case 'delete': w.u8(2); w.str(c.id); break;
      case 'blob': w.u8(3); w.str(c.key); w.blob(c.value); break;
      case 'wipe': w.u8(4); break;
    }
  }
  return w.done();
}

function decodeBody(body: Uint8Array): Change[] {
  const r = new Reader(body);
  const out: Change[] = [];
  while (r.left) {
    const op = r.u8();
    if (op === 1) {
      const id = r.str(), community = r.str(), expiresAt = r.f64(), wire = r.blob();
      if (id === null || wire === null) throw new Error('bad put');
      out.push({ op: 'put', id, community, expiresAt, wire });
    } else if (op === 2) {
      const id = r.str();
      if (id === null) throw new Error('bad delete');
      out.push({ op: 'delete', id });
    } else if (op === 3) {
      const key = r.str();
      if (key === null) throw new Error('bad blob');
      out.push({ op: 'blob', key, value: r.blob() });
    } else if (op === 4) {
      out.push({ op: 'wipe' });
    } else {
      throw new Error('unknown change');
    }
  }
  return out;
}

function frame(generation: number, body: Uint8Array): Uint8Array {
  const out = new Uint8Array(HEAD + body.length + 4);
  const v = new DataView(out.buffer);
  v.setUint32(0, MAGIC, true);
  v.setUint32(4, generation, true);
  v.setUint32(8, body.length, true);
  out.set(body, HEAD);
  v.setUint32(HEAD + body.length, crc32(body), true);
  return out;
}

interface Log {
  generation: number;
  frames: Change[][];
  /** Where the valid frames end. */
  end: number;
}

/** The valid frames of one file, from its start up to the first that is cut short or damaged. */
function readLog(h: SyncHandle): Log | null {
  const size = h.getSize();
  if (size < HEAD + 4) return null;
  const all = new Uint8Array(size);
  h.read(all, { at: 0 });
  const v = new DataView(all.buffer);
  let at = 0;
  let generation = -1;
  const frames: Change[][] = [];
  while (at + HEAD + 4 <= size) {
    if (v.getUint32(at, true) !== MAGIC) break;
    const gen = v.getUint32(at + 4, true);
    const len = v.getUint32(at + 8, true);
    if (generation >= 0 && gen !== generation) break;
    if (at + HEAD + len + 4 > size) break;
    const body = all.subarray(at + HEAD, at + HEAD + len);
    if (crc32(body) !== v.getUint32(at + HEAD + len, true)) break;
    let changes: Change[];
    try {
      changes = decodeBody(body);
    } catch {
      break;
    }
    // A file's first frame starts from nothing: it was written by compaction or onto an empty file.
    if (generation < 0 && changes[0]?.op !== 'wipe') return null;
    generation = gen;
    frames.push(changes);
    at += HEAD + len + 4;
  }
  return generation < 0 ? null : { generation, frames, end: at };
}

/** The file that holds the store: the one whose first frame is valid and has the higher generation. */
const newest = (logs: (Log | null)[]): 0 | 1 => (!logs[1] ? 0 : !logs[0] ? 1 : logs[1].generation > logs[0].generation ? 1 : 0);

/** Empty, or a first frame that runs past the end of the file: a first write that never finished. */
function unfinished(h: SyncHandle): boolean {
  const size = h.getSize();
  if (size === 0) return true;
  if (size < HEAD) return true;
  const head = new Uint8Array(HEAD);
  h.read(head, { at: 0 });
  const v = new DataView(head.buffer);
  return v.getUint32(0, true) === MAGIC && HEAD + v.getUint32(8, true) + 4 > size;
}

interface State {
  records: Map<string, StoredRecord>;
  blobs: Map<string, Uint8Array>;
}

function replay(frames: Change[][], into: State = { records: new Map(), blobs: new Map() }): State {
  for (const changes of frames) {
    for (const c of changes) {
      switch (c.op) {
        case 'put': into.records.set(c.id, { wire: c.wire, community: c.community, expiresAt: c.expiresAt }); break;
        case 'delete': into.records.delete(c.id); break;
        case 'blob':
          if (c.value === null) into.blobs.delete(c.key);
          else into.blobs.set(c.key, c.value);
          break;
        case 'wipe': into.records.clear(); into.blobs.clear(); break;
      }
    }
  }
  return into;
}

const recordSize = (r: StoredRecord) => r.wire.length + 64;

/** Closes handles that may be closed already. */
function release(handles: SyncHandle[]) {
  for (const h of handles) {
    try {
      h.close();
    } catch {
      // already closed
    }
  }
}

/** One store's log. Only the worker that holds the store's lock opens it (core.worker.ts). */
export class LogStore {
  readonly kind = 'opfs';
  private state: State;
  /** Bytes of what is live, roughly: compaction starts once the log is three times as big. */
  private live = 0;
  /** Set by `close()`: files that finish opening again after it are not kept (`reacquire`). */
  private closed = false;

  private constructor(
    private readonly openFile: OpenFile,
    private handles: [SyncHandle, SyncHandle],
    private active: 0 | 1,
    private generation: number,
    private size: number,
    state: State,
  ) {
    this.state = state;
    for (const r of state.records.values()) this.live += recordSize(r);
    for (const [k, b] of state.blobs) this.live += k.length + b.length;
  }

  static async open(openFile: OpenFile): Promise<LogStore> {
    const handles = [await openFile(FILES[0]), await openFile(FILES[1])] as [SyncHandle, SyncHandle];
    try {
      const logs = handles.map(readLog);
      const pick = newest(logs);
      const log = logs[pick];
      if (!log) {
        // Nothing whole in either: a store never written, or whose first write was cut short.
        // Anything else is damage, and is left as it is for the person to decide (reset).
        if (!handles.every(unfinished)) throw new Error('Saved data is damaged. Your saved data has not been changed.');
        for (const h of handles) {
          h.truncate(0);
          h.flush();
        }
        return new LogStore(openFile, handles, 0, 1, 0, replay([]));
      }
      if (handles[pick].getSize() !== log.end) {
        // a write that was cut short
        handles[pick].truncate(log.end);
        handles[pick].flush();
      }
      return new LogStore(openFile, handles, pick, log.generation, log.end, replay(log.frames));
    } catch (e) {
      for (const h of handles) h.close();
      throw e;
    }
  }

  /**
   * Opens both files again after the browser closed their handles behind the store's back
   * (Firefox closes all of a site's handles when its storage is cleared or reset). The new
   * handles are kept only if the files still end where this store left them: the same file,
   * generation and size. Otherwise a write would land on data this store does not know, so it
   * lets them go and throws `StoreLost`; only reloading can open what is there now.
   */
  async reacquire(): Promise<void> {
    const handles: SyncHandle[] = [];
    let kept = false;
    try {
      for (const name of FILES) handles.push(await this.openFile(name));
      // Locked, or taken over by another tab, while the files were opening: they are not ours.
      if (this.closed) throw new StoreLost('It was closed.');
      if (!this.leftAsItWas(handles)) throw new StoreLost('Its files changed while they were closed.');
      kept = true;
    } catch (e) {
      throw e instanceof StoreLost ? e : new StoreLost('Its files could not be opened again.', { cause: e });
    } finally {
      if (!kept) release(handles);
    }
    release(this.handles);
    this.handles = handles as [SyncHandle, SyncHandle];
  }

  /** Whether opening `handles` as a store would give this one. */
  private leftAsItWas(handles: SyncHandle[]): boolean {
    const logs = handles.map(readLog);
    const at = newest(logs);
    const log = logs[at];
    if (!log) return this.size === 0 && handles.every((h) => h.getSize() === 0);
    return at === this.active && log.generation === this.generation && log.end === this.size && handles[at].getSize() === this.size;
  }

  /** Everything stored, keys in order, as IndexedDB lists them. */
  snapshot(): Snapshot {
    const byKey = <T>(m: Map<string, T>): [string, T][] => [...m].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
    return { records: byKey(this.state.records), blobs: byKey(this.state.blobs) };
  }

  /** Writes `changes` as one frame: all of them or, after a crash, none. */
  write(changes: Change[]): void {
    if (!changes.length) return;
    const batch: Change[] = this.size === 0 && changes[0].op !== 'wipe' ? [{ op: 'wipe' }, ...changes] : changes;
    const bytes = frame(this.generation, encodeBody(batch));
    const h = this.handles[this.active];
    const wrote = h.write(bytes, { at: this.size });
    if (wrote !== bytes.length) {
      h.truncate(this.size);
      throw new Error('Saving to this browser failed: it may be out of space.');
    }
    h.flush();
    this.size += bytes.length;
    for (const c of batch) {
      if (c.op === 'put') {
        const old = this.state.records.get(c.id);
        if (old) this.live -= recordSize(old);
        this.live += recordSize({ wire: c.wire, community: c.community, expiresAt: c.expiresAt });
      } else if (c.op === 'delete') {
        const old = this.state.records.get(c.id);
        if (old) this.live -= recordSize(old);
      } else if (c.op === 'blob') {
        const old = this.state.blobs.get(c.key);
        if (old) this.live -= c.key.length + old.length;
        if (c.value) this.live += c.key.length + c.value.length;
      } else {
        this.live = 0;
      }
    }
    replay([batch], this.state);
    if (this.size > COMPACT_AT && this.size > 3 * this.live) this.compact();
  }

  /** Rewrites what is live into the other file under the next generation; then empties this one. */
  compact(): void {
    const next = (1 - this.active) as 0 | 1;
    const generation = (this.generation + 1) >>> 0;
    const all: Change[] = [{ op: 'wipe' }];
    for (const [id, r] of this.state.records) all.push({ op: 'put', id, community: r.community, expiresAt: r.expiresAt, wire: r.wire });
    for (const [key, value] of this.state.blobs) all.push({ op: 'blob', key, value });
    const bytes = frame(generation, encodeBody(all));
    const h = this.handles[next];
    h.truncate(0);
    if (h.write(bytes, { at: 0 }) !== bytes.length) {
      h.truncate(0);
      h.flush();
      return; // out of space: keep appending to the old log, which is still whole
    }
    h.flush();
    this.handles[this.active].truncate(0);
    this.handles[this.active].flush();
    this.active = next;
    this.generation = generation;
    this.size = bytes.length;
  }

  close(): void {
    this.closed = true;
    release(this.handles);
  }
}

const dirName = (name: string) => `store-${name}`;

/** Whether this context has sync access handles (a dedicated worker in a current browser). */
export function opfsPossible(): boolean {
  const g = globalThis as { FileSystemFileHandle?: { prototype: object } };
  return typeof navigator !== 'undefined' && !!navigator.storage?.getDirectory && !!g.FileSystemFileHandle && 'createSyncAccessHandle' in g.FileSystemFileHandle.prototype;
}

/** The store's directory, or null when the browser refuses OPFS here (a private window). */
export async function opfsRoot(): Promise<FileSystemDirectoryHandle | null> {
  if (!opfsPossible()) return null;
  try {
    return await navigator.storage.getDirectory();
  } catch {
    return null;
  }
}

/** Whether a store of this name has anything in OPFS. */
export async function opfsHolds(root: FileSystemDirectoryHandle, name: string): Promise<boolean> {
  let dir: FileSystemDirectoryHandle;
  try {
    dir = await root.getDirectoryHandle(dirName(name));
  } catch {
    return false;
  }
  for (const f of FILES) {
    try {
      if ((await (await dir.getFileHandle(f)).getFile()).size > 0) return true;
    } catch {
      // not there
    }
  }
  return false;
}

export async function openOpfs(root: FileSystemDirectoryHandle, name: string): Promise<LogStore> {
  const dir = await root.getDirectoryHandle(dirName(name), { create: true });
  type WithSync = FileSystemFileHandle & { createSyncAccessHandle(): Promise<SyncHandle> };
  return LogStore.open(async (file) => {
    const handle = (await dir.getFileHandle(file, { create: true })) as WithSync;
    // A tab that just took over: the one it took over from may still be letting go.
    for (let tries = 0; ; tries++) {
      try {
        return await handle.createSyncAccessHandle();
      } catch (e) {
        if ((e as DOMException)?.name !== 'NoModificationAllowedError' || tries >= 20) throw e;
        await new Promise((r) => setTimeout(r, 100));
      }
    }
  });
}

/** Removes a store's files. The caller holds its lock and closed it first. */
export async function removeOpfs(name: string): Promise<void> {
  const root = await opfsRoot();
  if (!root) return;
  try {
    await root.removeEntry(dirName(name), { recursive: true });
  } catch (e) {
    if ((e as DOMException)?.name !== 'NotFoundError') throw e;
  }
}
