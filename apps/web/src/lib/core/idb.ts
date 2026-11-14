// The worker holds the single-writer lock. Core values are encrypted; only the vault header
// and an old-client guard remain public. Encryption finishes before atomic database writes.
// Where the browser has sync access handles, the cells live in OPFS log files (opfs.ts);
// elsewhere, and for a store that already lives there, in IndexedDB.

import { LogStore, handleClosed, opfsHolds, opfsRoot, openOpfs, removeOpfs } from './opfs';

const DB = 'hoi';
const VERSION = 1;
const GUARD = new TextEncoder().encode('{"version":4294967295}');

export interface VaultCipher {
  header(): Uint8Array;
  seal(context: string, value: Uint8Array): Uint8Array;
  unseal(context: string, value: Uint8Array): Uint8Array;
  fingerprint(value: Uint8Array): string;
}

// A protected manifest detects deleted cells and replay of a single older valid ciphertext,
// not only modifications to cells that happen to remain. Published builds never had a vault
// without this manifest; missing manifests are corruption, not a migration opportunity.
const manifests = new WeakMap<VaultCipher, Map<string, string>>();

/** A store: an IndexedDB database, or an OPFS log. */
export type Db = IDBDatabase | LogStore;

export interface Snapshot {
  records: [string, StoredRecord][];
  blobs: [string, Uint8Array][];
}

export interface StoredRecord {
  wire: Uint8Array;
  community: string | null;
  expiresAt: number;
}

export type Change =
  | { op: 'put'; id: string; community: string | null; expiresAt: number; wire: Uint8Array }
  | { op: 'delete'; id: string }
  | { op: 'blob'; key: string; value: Uint8Array | null }
  | { op: 'wipe' };

function req<T>(r: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}

function done(tx: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    tx.oncomplete = () => resolve();
    tx.onabort = () => reject(tx.error ?? new Error('transaction aborted'));
    tx.onerror = () => reject(tx.error);
  });
}

/** OPFS where this browser has it, unless this store already lives in IndexedDB. */
export async function openStore(name = DB): Promise<Db> {
  const root = await opfsRoot();
  if (root && ((await opfsHolds(root, name)) || !(await idbHolds(name)))) return openOpfs(root, name);
  return openIdb(name);
}

/** Whether an IndexedDB database of this name exists, without making one. */
export async function idbHolds(name: string): Promise<boolean> {
  return new Promise((resolve, reject) => {
    const r = indexedDB.open(name);
    let made = false;
    r.onupgradeneeded = (e) => {
      if (e.oldVersion === 0) {
        made = true;
        r.transaction?.abort();
      }
    };
    r.onsuccess = () => {
      r.result.close();
      resolve(true);
    };
    r.onerror = () => (made ? resolve(false) : reject(r.error));
  });
}

export async function openIdb(name = DB): Promise<IDBDatabase> {
  const r = indexedDB.open(name, VERSION);
  r.onupgradeneeded = () => {
    const db = r.result;
    if (!db.objectStoreNames.contains('records')) db.createObjectStore('records');
    if (!db.objectStoreNames.contains('blobs')) db.createObjectStore('blobs');
  };
  const db = await req(r);
  db.onversionchange = () => db.close();
  return db;
}

/** Caller must hold this store's writer lock and stop its runtime first. */
export async function resetStore(name: string, confirmation: string): Promise<void> {
  if (confirmation !== 'RESET') throw new Error('Type RESET to confirm clearing local data.');
  // Delete this exact database without opening its version or schema: incompatible or
  // damaged metadata must not trap a person in recovery. Other connections receive
  // versionchange and close; a foreign connection must close before deletion completes.
  await req(indexedDB.deleteDatabase(name));
  await removeOpfs(name);
}

export async function snapshot(db: Db): Promise<Snapshot> {
  if (db instanceof LogStore) return db.snapshot();
  const tx = db.transaction(['records', 'blobs'], 'readonly');
  const records = tx.objectStore('records');
  const blobs = tx.objectStore('blobs');
  const finished = done(tx);
  const [ids, recs, keys, vals] = await Promise.all([
    req(records.getAllKeys()), req(records.getAll()), req(blobs.getAllKeys()), req(blobs.getAll()),
  ]);
  await finished;
  if ([...ids, ...keys].some(k => typeof k !== 'string')) throw new Error('Saved data could not be opened.');
  return {
    records: ids.map((id, i) => [id as string, recs[i] as StoredRecord]),
    blobs: keys.map((key, i) => [key as string, vals[i] as Uint8Array]),
  };
}

function equal(a: Uint8Array, b: Uint8Array): boolean {
  return a.length === b.length && a.every((v, i) => v === b[i]);
}

export function vaultHeader(saved: Snapshot): Uint8Array | null {
  const header = saved.blobs.find(([k]) => k === 'vault')?.[1];
  const guard = saved.blobs.find(([k]) => k === 'format')?.[1];
  if (header) {
    if (!(header instanceof Uint8Array) || !(guard instanceof Uint8Array) || !equal(guard, GUARD)
      || !saved.blobs.some(([k]) => k === 'vault-manifest')
      || saved.blobs.some(([k]) => k !== 'vault' && k !== 'format' && k !== 'vault-manifest' && !k.startsWith('v:'))) {
      throw new Error('Encrypted storage is damaged. Your saved data has not been changed.');
    }
    return header;
  }
  if (saved.blobs.some(([k]) => k.startsWith('v:') || k === 'vault-manifest') || (guard && equal(guard, GUARD))) {
    throw new Error('The key for this encrypted storage is missing. Your saved data has not been changed.');
  }
  return null;
}

export function decodeSnapshot(saved: Snapshot, vault: VaultCipher | null): { records: Uint8Array[]; blobs: [string, Uint8Array][] } {
  const encrypted = vaultHeader(saved);
  if (encrypted && (!vault || !equal(encrypted, vault.header()))) throw new Error('Unlock this device first.');
  if (encrypted) {
    const payload = vault!.unseal('manifest', saved.blobs.find(([key]) => key === 'vault-manifest')![1]);
    const manifest = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(payload));
    if (manifest?.version !== 1 || !manifest.cells || typeof manifest.cells !== 'object' || Array.isArray(manifest.cells)) throw new Error('Saved data is damaged.');
    const expected = new Map<string, string>(Object.entries(manifest.cells));
    const actual = new Map<string, string>([
      ...saved.records.map(([id, r]): [string, string] => [`record:${id}`, vault!.fingerprint(r.wire)]),
      ...saved.blobs.filter(([key]) => key.startsWith('v:')).map(([key, value]): [string, string] => [`blob:${key.slice(2)}`, vault!.fingerprint(value)]),
    ]);
    if (expected.size !== actual.size || [...actual].some(([key, hash]) => expected.get(key) !== hash)) throw new Error('Saved data is incomplete or damaged. No data was replaced.');
    manifests.set(vault!, expected);
  }
  return {
    records: saved.records.map(([id, r]) => encrypted ? vault!.unseal(`record:${id}`, r.wire) : r.wire),
    blobs: encrypted
      ? saved.blobs.filter(([k]) => k.startsWith('v:')).map(([k, value]) => [k.slice(2), vault!.unseal(`blob:${k.slice(2)}`, value)])
      : saved.blobs,
  };
}

/** Encrypt all legacy data before one atomic replacement. No writes on crypto failure. */
export async function migrate(db: Db, saved: Snapshot, vault: VaultCipher): Promise<void> {
  if (vaultHeader(saved)) throw new Error('This storage is already encrypted.');
  manifests.set(vault, new Map());
  const changes: Change[] = [
    { op: 'wipe' },
    ...saved.records.map(([id, r]): Change => ({ op: 'put', id, wire: r.wire, community: r.community, expiresAt: r.expiresAt })),
    ...saved.blobs.map(([key, value]): Change => ({ op: 'blob', key, value })),
  ];
  await apply(db, changes, vault);
}

/** Saves a vault header that changed (a passkey added or removed); the cells stay. */
export async function saveHeader(db: Db, vault: VaultCipher): Promise<void> {
  if (db instanceof LogStore) return db.write([{ op: 'blob', key: 'vault', value: vault.header() }]);
  const tx = db.transaction(['blobs'], 'readwrite');
  const finished = done(tx);
  tx.objectStore('blobs').put(vault.header(), 'vault');
  await finished;
}

export async function apply(db: Db, changes: Change[], vault: VaultCipher): Promise<void> {
  if (!changes.length) return;
  const prior = manifests.get(vault);
  if (!prior && changes[0]?.op !== 'wipe') throw new Error('Unlock this device first.');
  const manifest = new Map(prior);
  const sealed = changes.map((c): Change => {
    if (c.op === 'put') {
      const wire = vault.seal(`record:${c.id}`, c.wire);
      manifest.set(`record:${c.id}`, vault.fingerprint(wire));
      return { ...c, wire };
    }
    if (c.op === 'blob') {
      const value = c.value === null ? null : vault.seal(`blob:${c.key}`, c.value);
      if (value === null) manifest.delete(`blob:${c.key}`);
      else manifest.set(`blob:${c.key}`, vault.fingerprint(value));
      return { ...c, key: `v:${c.key}`, value };
    }
    if (c.op === 'delete') manifest.delete(`record:${c.id}`);
    if (c.op === 'wipe') manifest.clear();
    return c;
  });
  const sealedManifest = vault.seal('manifest', new TextEncoder().encode(JSON.stringify({ version: 1, cells: Object.fromEntries(manifest) })));
  if (db instanceof LogStore) {
    // one frame: the cells, then the header, guard and manifest that vouch for them
    const batch: Change[] = [
      ...sealed,
      { op: 'blob', key: 'vault', value: vault.header() },
      { op: 'blob', key: 'format', value: GUARD },
      { op: 'blob', key: 'vault-manifest', value: sealedManifest },
    ];
    try {
      db.write(batch);
    } catch (e) {
      // The browser closed the files behind the store's back. `write` moves the store's size and
      // state only after its write and flush succeed, so the same frame can go again once the
      // files are open again as the store left them; if they are not, `reacquire` throws StoreLost.
      if (!handleClosed(e)) throw e;
      await db.reacquire();
      db.write(batch);
    }
    manifests.set(vault, manifest);
    return;
  }
  const tx = db.transaction(['records', 'blobs'], 'readwrite');
  const finished = done(tx);
  const records = tx.objectStore('records');
  const blobs = tx.objectStore('blobs');
  for (const c of sealed) {
    switch (c.op) {
      case 'put':
        records.put({ wire: c.wire, community: c.community, expiresAt: c.expiresAt } satisfies StoredRecord, c.id);
        break;
      case 'delete':
        records.delete(c.id);
        break;
      case 'blob':
        if (c.value !== null) blobs.put(c.value, c.key);
        else blobs.delete(c.key);
        break;
      case 'wipe':
        records.clear();
        blobs.clear();
        break;
    }
  }
  // Wiping account data retains encryption; a new identity cannot fall back to plaintext.
  blobs.put(vault.header(), 'vault');
  blobs.put(GUARD, 'format');
  blobs.put(sealedManifest, 'vault-manifest');
  await finished;
  manifests.set(vault, manifest);
}
