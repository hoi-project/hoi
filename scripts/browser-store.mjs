// What a browser's store holds, read and changed from the page the way a person with access to
// the profile could: OPFS log files where the app keeps them there, else IndexedDB.
// The page reads OPFS files while the core worker holds them; it writes only while the device is
// locked, when the worker holds nothing open. Never creates a store that isn't there.

/** The store's raw cells, as `snapshot()` lists them, with bytes as arrays. */
export async function rawStore(page, name = 'hoi') {
  return page.evaluate(async (name) => {
    const { LogStore } = await import('/src/lib/core/opfs.ts');
    const { idbHolds, openIdb, snapshot } = await import('/src/lib/core/idb.ts');
    const plain = (saved) => ({
      records: saved.records.map(([key, r]) => [key, { ...r, wire: [...r.wire] }]),
      blobs: saved.blobs.map(([key, value]) => [key, [...value]]),
    });
    const files = await window.__hoiStoreFiles(name);
    if (files) {
      const store = await LogStore.open(async (f) => files[f]);
      return plain(store.snapshot());
    }
    if (!(await idbHolds(name))) return { records: [], blobs: [] };
    const db = await openIdb(name);
    try {
      return plain(await snapshot(db));
    } finally {
      db.close();
    }
  }, name);
}

/** Writes raw changes (`{ op: 'blob', key, value }` with value an array or null) to a locked store. */
export async function editStore(page, changes, name = 'hoi') {
  await page.evaluate(async ({ name, changes }) => {
    const { LogStore } = await import('/src/lib/core/opfs.ts');
    const { idbHolds, openIdb } = await import('/src/lib/core/idb.ts');
    const decoded = changes.map((c) => (c.op === 'blob' ? { ...c, value: c.value && new Uint8Array(c.value) } : c));
    const files = await window.__hoiStoreFiles(name);
    if (files) {
      const store = await LogStore.open(async (f) => files[f]);
      store.write(decoded);
      await window.__hoiSaveStoreFiles(name, files);
      return;
    }
    if (!(await idbHolds(name))) throw new Error('no store to change');
    const db = await openIdb(name);
    await new Promise((resolve, reject) => {
      const tx = db.transaction(['records', 'blobs'], 'readwrite');
      for (const c of decoded) {
        if (c.op === 'blob') c.value ? tx.objectStore('blobs').put(c.value, c.key) : tx.objectStore('blobs').delete(c.key);
        else if (c.op === 'delete') tx.objectStore('records').delete(c.id);
      }
      tx.oncomplete = resolve;
      tx.onerror = () => reject(tx.error);
    });
    db.close();
  }, { name, changes });
}

/** Installs the page helpers `rawStore` and `editStore` use; call once per context. */
export async function storeHelpers(context) {
  await context.addInitScript(() => {
    // An in-memory copy of a store's files, with the sync access handle methods LogStore uses.
    class Copy {
      constructor(data) { this.data = data; }
      read(buffer, { at }) { const n = Math.max(0, Math.min(buffer.length, this.data.length - at)); buffer.set(this.data.subarray(at, at + n)); return n; }
      write(buffer, { at }) {
        if (at + buffer.length > this.data.length) { const grown = new Uint8Array(at + buffer.length); grown.set(this.data); this.data = grown; }
        this.data.set(buffer, at);
        return buffer.length;
      }
      truncate(size) { const cut = new Uint8Array(size); cut.set(this.data.subarray(0, Math.min(size, this.data.length))); this.data = cut; }
      flush() {}
      getSize() { return this.data.length; }
      close() {}
    }
    window.__hoiStoreFiles = async (name) => {
      let dir;
      try {
        dir = await (await navigator.storage.getDirectory()).getDirectoryHandle(`store-${name}`);
      } catch {
        return null;
      }
      const files = {};
      let any = false;
      for (const f of ['log-a', 'log-b']) {
        let data = new Uint8Array(0);
        try {
          data = new Uint8Array(await (await (await dir.getFileHandle(f)).getFile()).arrayBuffer());
        } catch {
          // not there
        }
        any ||= data.length > 0;
        files[f] = new Copy(data);
      }
      return any ? files : null;
    };
    window.__hoiSaveStoreFiles = async (name, files) => {
      const dir = await (await navigator.storage.getDirectory()).getDirectoryHandle(`store-${name}`);
      for (const [f, copy] of Object.entries(files)) {
        const w = await (await dir.getFileHandle(f, { create: true })).createWritable();
        await w.write(copy.data);
        await w.close();
      }
    };
  });
}
