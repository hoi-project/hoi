// The registry operator's tool: withhold a listing, drop a node's announcements, and
// see both lists, against the Node service's SQLite file. There is no HTTP endpoint for any of
// this; whoever can run this can already read and write the database. Safe while the service
// runs: it reads the database on every request, so a change takes effect at once.
//
//   node registry/admin.mjs [--db FILE] list
//   node registry/admin.mjs [--db FILE] hide NAME|KEY       # still reserved, no longer served
//   node registry/admin.mjs [--db FILE] unhide NAME|KEY
//   node registry/admin.mjs [--db FILE] drop-node ENDPOINT  # forget its announcement, refuse its next
//   node registry/admin.mjs [--db FILE] allow-node ENDPOINT
//
// FILE defaults to HOI_REGISTRY_DB, then hoi-registry.db, and must already exist.
import { existsSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { openRegistry } from './server.mjs';

const USAGE = 'Usage: node registry/admin.mjs [--db FILE] list | hide NAME|KEY | unhide NAME|KEY | drop-node ENDPOINT | allow-node ENDPOINT';

/** Runs one command; returns the lines to print. Throws with a message for the operator. */
export function runAdmin(args, { env = process.env, now = () => Math.floor(Date.now() / 1000) } = {}) {
  const rest = [...args];
  let filename = env.HOI_REGISTRY_DB || 'hoi-registry.db';
  if (rest[0] === '--db') {
    rest.shift();
    filename = rest.shift();
  }
  const [command, target, ...extra] = rest;
  const needsTarget = ['hide', 'unhide', 'drop-node', 'allow-node'].includes(command);
  if (!filename || !command || (command !== 'list' && !needsTarget) || (needsTarget ? !target || extra.length : target !== undefined)) throw new Error(USAGE);
  // A typo must not create an empty namespace next to the real one.
  if (!existsSync(filename)) throw new Error(`No registry database at ${filename}.`);
  const registry = openRegistry(filename);
  try {
    switch (command) {
      case 'list': {
        const view = registry.operatorView(now());
        const lines = [`withheld (${view.withheld.length}):`];
        for (const row of view.withheld) lines.push(`  ${row.key} · ${row.community ? `c/${row.name} ${row.community}` : 'not reserved'} · since ${row.withheldAt}`);
        lines.push(`refused nodes (${view.refusedNodes.length}):`);
        for (const row of view.refusedNodes) lines.push(`  ${row.endpoint} since ${row.refusedAt}`);
        lines.push(`announced now (${view.announcedNodes.length}):`);
        for (const row of view.announcedNodes) lines.push(`  ${row.endpoint} · ${row.routes} route${row.routes === 1 ? '' : 's'} · ${row.communities} communit${row.communities === 1 ? 'y' : 'ies'} · issued ${row.issuedAt}`);
        return lines;
      }
      case 'hide': {
        const key = registry.hide(target, now());
        if (!key) throw new Error('Give a public name or a 64-character community key.');
        return [`withheld ${key}: still reserved, no longer listed, looked up or announced as carried`];
      }
      case 'unhide':
        if (!registry.unhide(target)) throw new Error('That name or key is not withheld.');
        return [`listed again: ${target}`];
      case 'drop-node':
        if (!registry.dropNode(target, now())) throw new Error('Give a 64-character endpoint key.');
        return [`dropped node ${target}: its announcement is gone and its next ones are refused`];
      default:
        if (!registry.allowNode(target)) throw new Error('That endpoint key is not refused.');
        return [`node ${target} may announce itself again`];
    }
  } finally {
    registry.close();
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    for (const line of runAdmin(process.argv.slice(2))) console.log(line);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 2;
  }
}
