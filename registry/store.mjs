// Storage operations shared by Node SQLite and Cloudflare Durable Object SQLite.
import { randomBytes } from 'node:crypto';
import { MAX_ANNOUNCED_NODES, NODE_TTL_SECONDS, WITHHELD, normalizeCommunityKey, normalizeName } from './protocol.mjs';
import { logEntryText } from './public/log.js';

class ReservationConflict extends Error {
  constructor(result) { super(result.body.error); this.result = result; }
}

/** The database file's version. 4 added the operator's two lists, 5 the public log of names;
 * a registry older than that refuses the file. The API's `schemaVersion` (response
 * shapes) stays 3. */
export const DATABASE_VERSION = 5;

/** Every table the registry keeps. Public node announcements are the only rows with a time in
 * them: a node's own routes, kept for half an hour, never a person's address. `withheld` and
 * `refused_nodes` are the operator's, written with `registry/admin.mjs`, `hoi-node admin
 * registry` or the Worker's configuration: listings withheld by name or key (still reserved, never
 * served) and nodes whose announcements are refused. `name_log` is the public log: every
 * owner-signed claim retained, in the order it was, never changed or removed; `log_secret` the
 * registry's log key. */
export const SCHEMA = [
  'CREATE TABLE IF NOT EXISTS communities (name TEXT PRIMARY KEY, community TEXT NOT NULL UNIQUE) STRICT',
  'CREATE TABLE IF NOT EXISTS community_index (community TEXT PRIMARY KEY REFERENCES communities(community), protocol_version TEXT NOT NULL, index_version INTEGER NOT NULL CHECK(index_version > 0)) STRICT',
  'CREATE TABLE IF NOT EXISTS community_proofs (community TEXT NOT NULL REFERENCES communities(community), claim_version INTEGER NOT NULL CHECK(claim_version IN (1, 2)), signature TEXT NOT NULL, PRIMARY KEY(community, claim_version)) STRICT',
  'CREATE TABLE IF NOT EXISTS public_nodes (endpoint TEXT PRIMARY KEY, routes TEXT NOT NULL, communities TEXT NOT NULL, issued_at INTEGER NOT NULL CHECK(issued_at >= 0), first_seen INTEGER NOT NULL DEFAULT 0) STRICT',
  'CREATE TABLE IF NOT EXISTS withheld (key TEXT PRIMARY KEY, withheld_at INTEGER NOT NULL) STRICT',
  'CREATE TABLE IF NOT EXISTS refused_nodes (endpoint TEXT PRIMARY KEY, refused_at INTEGER NOT NULL) STRICT',
  'CREATE TABLE IF NOT EXISTS name_log (idx INTEGER PRIMARY KEY CHECK(idx >= 0), name TEXT NOT NULL, community TEXT NOT NULL, entry TEXT NOT NULL UNIQUE) STRICT',
  'CREATE TABLE IF NOT EXISTS log_secret (id INTEGER PRIMARY KEY CHECK(id = 1), secret TEXT NOT NULL) STRICT',
];

/** Adds `public_nodes.first_seen` to a schema-3 database made before it existed. Idempotent. */
export function upgradeSchema(db) {
  const columns = db.prepare('PRAGMA table_info(public_nodes)').all().map((row) => row.name);
  if (!columns.includes('first_seen')) db.prepare('ALTER TABLE public_nodes ADD COLUMN first_seen INTEGER NOT NULL DEFAULT 0').run();
}

/** A name or community key as the operator's withheld list stores it, or null. */
export function withheldKey(value) {
  return normalizeCommunityKey(value) ?? normalizeName(value);
}

export function createRegistryStore(db, transaction) {
  const byName = db.prepare('SELECT name, community FROM communities WHERE name = ?');
  const byCommunity = db.prepare('SELECT name, community FROM communities WHERE community = ?');
  const insert = db.prepare('INSERT INTO communities(name, community) VALUES (?, ?) ON CONFLICT DO NOTHING RETURNING name, community');
  const list = db.prepare(`SELECT name, community FROM communities WHERE name >= ? AND name < ?
    AND name NOT IN (SELECT key FROM withheld) AND community NOT IN (SELECT key FROM withheld) ORDER BY name LIMIT 50`);
  const indexByCommunity = db.prepare('SELECT protocol_version AS protocolVersion, index_version AS indexVersion FROM community_index WHERE community = ?');
  const insertIndex = db.prepare('INSERT INTO community_index(community, protocol_version, index_version) VALUES (?, ?, ?)');
  const proofs = db.prepare('SELECT claim_version AS version, signature FROM community_proofs WHERE community = ? ORDER BY claim_version DESC');
  const insertProof = db.prepare('INSERT INTO community_proofs(community, claim_version, signature) VALUES (?, ?, ?) ON CONFLICT DO NOTHING');
  // A newer announcement replaces the node's earlier one; a replayed older one changes nothing.
  // `first_seen` stays with the node's first live announcement: when the list is full, the
  // nodes that were there first stay and newcomers wait, so a flood of fresh keys cannot push
  // the carriers people rely on out of discovery.
  const upsertNode = db.prepare(`INSERT INTO public_nodes(endpoint, routes, communities, issued_at, first_seen) VALUES (?, ?, ?, ?, ?)
    ON CONFLICT(endpoint) DO UPDATE SET routes = excluded.routes, communities = excluded.communities, issued_at = excluded.issued_at
    WHERE excluded.issued_at >= public_nodes.issued_at`);
  const expireNodes = db.prepare('DELETE FROM public_nodes WHERE issued_at < ?');
  const trimNodes = db.prepare(`DELETE FROM public_nodes WHERE endpoint IN (SELECT endpoint FROM public_nodes ORDER BY first_seen ASC, issued_at ASC, endpoint LIMIT -1 OFFSET ${MAX_ANNOUNCED_NODES})`);
  const nodeByEndpoint = db.prepare('SELECT endpoint, routes, communities, issued_at AS issuedAt FROM public_nodes WHERE endpoint = ?');
  const liveNodeRows = db.prepare(`SELECT endpoint, routes, communities, issued_at AS issuedAt FROM public_nodes
    WHERE issued_at >= ? AND endpoint NOT IN (SELECT endpoint FROM refused_nodes) ORDER BY issued_at DESC, endpoint LIMIT ${MAX_ANNOUNCED_NODES}`);
  const withheldRow = db.prepare('SELECT 1 AS hit FROM withheld WHERE key = ? OR key = ?');
  const withheldCommunity = db.prepare('SELECT 1 AS hit FROM withheld WHERE key = ? OR key IN (SELECT name FROM communities WHERE community = ?)');
  const refusedNode = db.prepare('SELECT 1 AS hit FROM refused_nodes WHERE endpoint = ?');
  const node = (row) => row && { endpoint: row.endpoint, routes: row.routes.split(','), communities: row.communities ? row.communities.split(',') : [], issuedAt: row.issuedAt };
  const withheld = (row) => !!row && !!withheldRow.get(row.name, row.community);
  // A claim touching a withheld name or key changes nothing and learns nothing about the row.
  const blocked = (name, community) => !!withheldRow.get(name, community) || withheld(byName.get(name)) || withheld(byCommunity.get(community));
  // The public log takes each claim the moment it is retained, in the same transaction, once.
  const appendLog = db.prepare(`INSERT INTO name_log(idx, name, community, entry) SELECT (SELECT COUNT(*) FROM name_log), ?, ?, ?
    WHERE NOT EXISTS (SELECT 1 FROM name_log WHERE entry = ?)`);
  const storedProof = db.prepare('SELECT signature FROM community_proofs WHERE community = ? AND claim_version = ?');
  const logProof = (name, community, version) => {
    const saved = storedProof.get(community, version);
    if (!saved) return;
    const index = version === 2 ? indexByCommunity.get(community) : undefined;
    const proof = { version, signature: saved.signature, ...(index ? { protocolVersion: index.protocolVersion, indexVersion: index.indexVersion } : {}) };
    const entry = logEntryText({ name, community, proof });
    appendLog.run(name, community, entry, entry);
  };
  const logRows = db.prepare(`SELECT entry, (name IN (SELECT key FROM withheld) OR community IN (SELECT key FROM withheld)) AS withheld
    FROM name_log WHERE idx >= ? AND idx < ? ORDER BY idx`);
  const withProof = (row) => {
    if (!row) return undefined;
    const saved = proofs.all(row.community)[0];
    const index = saved?.version === 2 ? indexByCommunity.get(row.community) : undefined;
    return { ...row, proof: saved ? { ...saved, ...(saved.version === 2 ? index : {}) } : null };
  };
  const describe = (row, release) => {
    if (!row) return undefined;
    const index = indexByCommunity.get(row.community);
    const signed = withProof(row);
    const compatible = signed.proof?.version === 2 && !!index && index.indexVersion === release.indexVersion && release.supportedProtocols.includes(index.protocolVersion);
    return { ...signed, protocolVersion: index?.protocolVersion ?? null, indexVersion: index?.indexVersion ?? null, compatible, indexed: compatible };
  };
  /** An exact lookup: the row, `{ withheld: true }` for a withheld one, or undefined. */
  const lookup = (row, shape) => withheld(row) ? { withheld: true } : shape(row);
  const reserve = (name, community) => {
    // Both uniqueness constraints are arbitrated by SQLite, including across processes.
    const created = insert.get(name, community);
    if (created) return { status: 200, body: created };
    const named = byName.get(name);
    if (named?.community === community) return { status: 200, body: named };
    if (named) return { status: 409, body: { error: 'name-taken', message: 'This public name belongs to another community.', ...named } };
    return { status: 409, body: { error: 'community-registered', message: 'This community already has a public name.', ...byCommunity.get(community) } };
  };
  return {
    find(name) { return lookup(byName.get(name), withProof); },
    list(prefix) { return list.all(prefix, `${prefix}\x7f`).map(withProof); },
    reserve(claim) {
      return transaction(() => {
        if (blocked(claim.name, claim.community)) return { status: 409, body: { ...WITHHELD } };
        const result = reserve(claim.name, claim.community);
        if (result.status === 200) {
          insertProof.run(claim.community, 1, claim.signature);
          logProof(result.body.name, claim.community, 1);
        }
        return { ...result, body: { ...result.body, ...withProof({ name: result.body.name, community: result.body.community }) } };
      });
    },
    findPublication(name, release) { return lookup(byName.get(name), (row) => describe(row, release)); },
    findPublicationByKey(community, release) { return lookup(byCommunity.get(community), (row) => describe(row, release)); },
    listPublications(prefix, after, release) {
      const protocols = release.supportedProtocols.map(() => '?').join(',');
      const rows = db.prepare(`SELECT c.name, c.community FROM communities c JOIN community_index i USING(community)
        JOIN community_proofs p ON p.community = c.community AND p.claim_version = 2
        WHERE c.name >= ? AND c.name < ? AND c.name > ? AND i.index_version = ? AND i.protocol_version IN (${protocols})
        AND c.name NOT IN (SELECT key FROM withheld) AND c.community NOT IN (SELECT key FROM withheld)
        ORDER BY c.name LIMIT 50`).all(prefix, `${prefix}\x7f`, after, release.indexVersion, ...release.supportedProtocols);
      return rows.map((row) => describe(row, release));
    },
    publish(claim, release) {
      try {
        return transaction(() => {
          if (blocked(claim.name, claim.community)) throw new ReservationConflict({ status: 409, body: { ...WITHHELD } });
          const result = reserve(claim.name, claim.community);
          if (result.status !== 200) throw new ReservationConflict({ status: result.status, body: {
            ...result.body, ...describe({ name: result.body.name, community: result.body.community }, release),
          } });
          const previous = indexByCommunity.get(claim.community);
          if (previous && (previous.protocolVersion !== claim.protocolVersion || previous.indexVersion !== claim.indexVersion)) {
            throw new ReservationConflict({ status: 409, body: { error: 'index-version-conflict', message: 'This community already has immutable index metadata.', ...describe(result.body, release) } });
          }
          if (!previous) insertIndex.run(claim.community, claim.protocolVersion, claim.indexVersion);
          insertProof.run(claim.community, 2, claim.signature);
          logProof(result.body.name, claim.community, 2);
          return { status: 200, body: describe(result.body, release) };
        });
      } catch (error) {
        if (error instanceof ReservationConflict) return error.result;
        throw error;
      }
    },
    /** Records a verified announcement and returns what discovery now holds for that node, or
     * null when the operator refused the node (nothing is stored). */
    announce(announcement, now) {
      return transaction(() => {
        if (refusedNode.get(announcement.endpoint)) return null;
        expireNodes.run(now - NODE_TTL_SECONDS);
        upsertNode.run(announcement.endpoint, announcement.routes.join(','), announcement.communities.join(','), announcement.issuedAt, now);
        trimNodes.run();
        return node(nodeByEndpoint.get(announcement.endpoint)) ?? { ...announcement, retained: false };
      });
    },
    /** How many entries the public log holds. */
    logSize() { return db.prepare('SELECT COUNT(*) AS n FROM name_log').get().n; },
    /** Log entries `start` to `end` (exclusive), each with whether the operator withholds it. */
    logEntries(start, end) { return logRows.all(start, end).map((row) => ({ entry: row.entry, withheld: !!row.withheld })); },
    /** The log key's secret, made on first use and kept with the names it signs for. */
    logSecret() {
      db.prepare('INSERT INTO log_secret(id, secret) VALUES (1, ?) ON CONFLICT DO NOTHING').run(randomBytes(32).toString('hex'));
      return db.prepare('SELECT secret FROM log_secret WHERE id = 1').get().secret;
    },
    /** Nodes announced within the last half hour, newest first, refused ones left out. */
    liveNodes(now) { return liveNodeRows.all(now - NODE_TTL_SECONDS).map(node); },
    /** Whether discovery should keep quiet about who carries `community`. */
    isWithheld(community) { return !!withheldCommunity.get(community, community); },

    // The operator's tools (registry/admin.mjs). Nothing here is reachable over HTTP.

    /** Withholds a listing by canonical name or key. Returns the stored key, or null if invalid. */
    hide(value, now) {
      const key = withheldKey(value);
      if (key) db.prepare('INSERT INTO withheld(key, withheld_at) VALUES (?, ?) ON CONFLICT DO NOTHING').run(key, now);
      return key;
    },
    unhide(value) {
      const key = withheldKey(value);
      if (!key || !db.prepare('SELECT 1 AS hit FROM withheld WHERE key = ?').get(key)) return false;
      db.prepare('DELETE FROM withheld WHERE key = ?').run(key);
      return true;
    },
    /** Drops a node's announcements and refuses its next ones. */
    dropNode(endpoint, now) {
      const key = normalizeCommunityKey(endpoint);
      if (!key) return false;
      transaction(() => {
        db.prepare('DELETE FROM public_nodes WHERE endpoint = ?').run(key);
        db.prepare('INSERT INTO refused_nodes(endpoint, refused_at) VALUES (?, ?) ON CONFLICT DO NOTHING').run(key, now);
      });
      return true;
    },
    allowNode(endpoint) {
      const key = normalizeCommunityKey(endpoint);
      if (!key || !refusedNode.get(key)) return false;
      db.prepare('DELETE FROM refused_nodes WHERE endpoint = ?').run(key);
      return true;
    },
    /** Replaces both lists with the operator's configuration (the Worker, whose database no
     * shell reaches). */
    syncOperatorLists(hidden, refused, now) {
      transaction(() => {
        db.prepare('DELETE FROM withheld').run();
        db.prepare('DELETE FROM refused_nodes').run();
        for (const key of hidden) db.prepare('INSERT INTO withheld(key, withheld_at) VALUES (?, ?) ON CONFLICT DO NOTHING').run(key, now);
        for (const key of refused) {
          db.prepare('DELETE FROM public_nodes WHERE endpoint = ?').run(key);
          db.prepare('INSERT INTO refused_nodes(endpoint, refused_at) VALUES (?, ?) ON CONFLICT DO NOTHING').run(key, now);
        }
      });
    },
    /** What the operator withheld and refused, and which nodes are announced now (to build an allowlist from). */
    operatorView(now) {
      return {
        withheld: db.prepare(`SELECT w.key, w.withheld_at AS withheldAt, c.name, c.community FROM withheld w
          LEFT JOIN communities c ON c.name = w.key OR c.community = w.key ORDER BY w.key`).all().map((row) => ({ ...row })),
        refusedNodes: db.prepare('SELECT endpoint, refused_at AS refusedAt FROM refused_nodes ORDER BY endpoint').all().map((row) => ({ ...row })),
        announcedNodes: liveNodeRows.all(now - NODE_TTL_SECONDS).map(node).map((n) => ({ endpoint: n.endpoint, routes: n.routes.length, communities: n.communities.length, issuedAt: n.issuedAt })),
      };
    },
  };
}
