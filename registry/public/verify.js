// Shared by the directory and app. Authority comes from the community key, not
// from a successful HTTPS response. Pins detect changes after the first lookup;
// they are not an independent witness of who first registered a public name.
const NAME = /^[a-z0-9][a-z0-9._-]{0,31}$/;
const KEY = /^[0-9a-f]{64}$/;
const SIGNATURE = /^[0-9a-f]{128}$/;
const PROTOCOL = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;
const invalid = () => new Error('The directory entry could not be verified. Use a trusted community invite.');
const changed = () => new Error('This public name has changed since it was verified on this device. Joining has been stopped. Use an invite from someone you trust.');
const unavailable = () => new Error('Saved community checks are unavailable. Public discovery has been stopped to protect previously verified names.');
const bytes = (hex) => Uint8Array.from(hex.match(/../g), (pair) => Number.parseInt(pair, 16));
const versionValid = (value) => Number.isSafeInteger(value) && value > 0 && value <= 2_147_483_647;

export async function readRegistryJson(response) {
  const maximum = 128 * 1024;
  if (Number(response.headers.get('content-length')) > maximum || !response.body) throw invalid();
  const reader = response.body.getReader();
  const parts = [];
  let size = 0;
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > maximum) { await reader.cancel(); throw invalid(); }
      parts.push(value);
    }
  } finally { reader.releaseLock(); }
  const body = new Uint8Array(size);
  let offset = 0;
  for (const part of parts) { body.set(part, offset); offset += part.byteLength; }
  try { return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(body)); }
  catch { throw invalid(); }
}

export async function verifyRegistryEntry(value, release) {
  if (!value || typeof value !== 'object' || Array.isArray(value)
    || typeof value.name !== 'string' || !NAME.test(value.name)
    || typeof value.community !== 'string' || !KEY.test(value.community)) throw invalid();
  const proof = value.proof;
  if (!proof || ![1, 2].includes(proof.version) || typeof proof.signature !== 'string' || !SIGNATURE.test(proof.signature)) throw invalid();
  const indexedProof = proof.version === 2;
  if (Object.keys(proof).sort().join(',') !== (indexedProof ? 'indexVersion,protocolVersion,signature,version' : 'signature,version')) throw invalid();
  if (indexedProof && (typeof proof.protocolVersion !== 'string' || !PROTOCOL.test(proof.protocolVersion) || !versionValid(proof.indexVersion))) throw invalid();
  const statement = indexedProof
    ? `hoi community index v2\n${value.name}\n${value.community}\n${proof.protocolVersion}\n${proof.indexVersion}`
    : `hoi community name v1\n${value.name}\n${value.community}`;
  try {
    const key = await crypto.subtle.importKey('raw', bytes(value.community), { name: 'Ed25519' }, false, ['verify']);
    if (!await crypto.subtle.verify('Ed25519', key, bytes(proof.signature), new TextEncoder().encode(statement))) throw invalid();
  } catch { throw invalid(); }
  const result = { name: value.name, community: value.community, proof: { ...proof } };
  if (release) {
    if (typeof value.compatible !== 'boolean' || typeof value.indexed !== 'boolean'
      || (indexedProof ? value.protocolVersion !== proof.protocolVersion || value.indexVersion !== proof.indexVersion
        : value.protocolVersion !== null || value.indexVersion !== null)) throw invalid();
    const compatible = indexedProof && proof.indexVersion === release.indexVersion && release.supportedProtocols.includes(proof.protocolVersion);
    return { ...result, protocolVersion: indexedProof ? proof.protocolVersion : null,
      indexVersion: indexedProof ? proof.indexVersion : null, compatible, indexed: compatible && value.indexed };
  }
  return result;
}

/** One read/write transaction compares and saves a whole verified result set.
 * IndexedDB serializes these transactions across tabs. Never fall back to
 * memory/localStorage when persistence is blocked, full, corrupt or unavailable.
 */
/** The device's trust store: pinned names, and the last log head seen from each registry. */
async function openTrust(factory) {
  if (!factory) throw unavailable();
  return new Promise((resolve, reject) => {
    let ended = false;
    const request = factory.open('hoi-registry-trust-v1', 2);
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains('names')) db.createObjectStore('names', { keyPath: ['scope', 'name'] });
      if (!db.objectStoreNames.contains('logs')) db.createObjectStore('logs', { keyPath: 'scope' });
    };
    request.onerror = request.onblocked = () => { ended = true; reject(unavailable()); };
    request.onsuccess = () => {
      if (ended) { request.result.close(); return; }
      request.result.onversionchange = () => request.result.close();
      resolve(request.result);
    };
  }).catch(() => { throw unavailable(); });
}

const scopeOf = (scope) => new URL(scope, globalThis.location?.href).href.replace(/\/$/, '');

/** The last signed log head this device checked for one registry. `advance` keeps
 * the larger of what is saved and `head`, in one transaction, so tabs never step back. */
export function registryLogPins(scope, factory = globalThis.indexedDB) {
  const namespace = scopeOf(scope);
  const run = async (mode, work) => {
    const db = await openTrust(factory);
    try {
      return await new Promise((resolve, reject) => {
        const transaction = db.transaction('logs', mode);
        const logs = transaction.objectStore('logs');
        let result;
        transaction.oncomplete = () => resolve(result);
        transaction.onabort = transaction.onerror = () => reject(unavailable());
        work(logs, (value) => { result = value; });
      });
    } finally { db.close(); }
  };
  return {
    get: () => run('readonly', (logs, done) => {
      const request = logs.get(namespace);
      request.onsuccess = () => done(request.result ? { logKey: request.result.logKey, size: request.result.size, root: request.result.root } : null);
    }),
    advance: (head) => run('readwrite', (logs) => {
      const request = logs.get(namespace);
      request.onsuccess = () => {
        const prior = request.result;
        if (!prior || (prior.logKey === head.logKey && prior.size <= head.size)) logs.put({ scope: namespace, logKey: head.logKey, size: head.size, root: head.root });
      };
    }),
  };
}

export function registryPins(scope, factory = globalThis.indexedDB) {
  const namespace = scopeOf(scope);
  const open = () => openTrust(factory);
  return {
    async accept(entries) {
      if (new Set(entries.map((entry) => entry.name)).size !== entries.length) throw invalid();
      const db = await open();
      try {
        await new Promise((resolve, reject) => {
          const transaction = db.transaction('names', 'readwrite');
          const names = transaction.objectStore('names');
          let failure;
          transaction.oncomplete = () => resolve();
          transaction.onabort = transaction.onerror = () => reject(failure || unavailable());
          for (const entry of entries) {
            const request = names.get([namespace, entry.name]);
            request.onsuccess = () => {
              const prior = request.result;
              const publication = entry.proof.version === 2 ? { protocolVersion: entry.proof.protocolVersion, indexVersion: entry.proof.indexVersion } : null;
              if (prior && (prior.community !== entry.community || !KEY.test(prior.community)
                || !('publication' in prior) || (prior.publication !== null
                  && (!prior.publication || typeof prior.publication !== 'object' || !publication
                    || prior.publication.protocolVersion !== publication.protocolVersion || prior.publication.indexVersion !== publication.indexVersion)))) {
                failure = changed(); transaction.abort(); return;
              }
              names.put({ scope: namespace, name: entry.name, community: entry.community, publication });
            };
          }
        }).catch(error => { throw error?.message === changed().message ? error : unavailable(); });
      } finally { db.close(); }
    },
    async checkMissing(name) {
      const db = await open();
      try {
        await new Promise((resolve, reject) => {
          const transaction = db.transaction('names', 'readonly');
          const request = transaction.objectStore('names').get([namespace, name]);
          transaction.onabort = transaction.onerror = () => reject(unavailable());
          transaction.oncomplete = () => request.result
            ? reject(new Error('This previously verified public name is missing from the directory. Its availability cannot be confirmed. Use a trusted community invite.'))
            : resolve();
        }).catch(error => {
          if (error?.message?.startsWith('This previously verified public name')) throw error;
          throw unavailable();
        });
      } finally { db.close(); }
    },
  };
}
