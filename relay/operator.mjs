// Offline removal of identified encrypted objects or whole rooms. The relay has no admin HTTP
// endpoint: this stores HOI_RELAY_PURGE as a Worker secret through Wrangler, which deploys a new
// version; the Durable Object deletes the listed rows when it next starts and keeps refusing them.
//
//   node relay/operator.mjs remove [--config relay/wrangler.toml] [--check https://relay.example/relay] [--dry-run] <room>[/<id>]… | v1
//
// Each run replaces the secret with one batch; earlier batches stay in the relay's `removed` table.
import { spawnSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';
import { removals } from './protocol.mjs';

// Worker secrets hold at most 5 KiB; larger lists go in later batches.
export const BATCH_BYTES = 4096;

export function purgeValue(entries) {
  const list = removals(entries.join(','));
  if (!list.length) throw new Error('Name at least one <room>, <room>/<id> or v1.');
  const value = list.join(',');
  if (Buffer.byteLength(value) > BATCH_BYTES) throw new Error(`One batch holds at most ${BATCH_BYTES} bytes; remove the rest in a later run.`);
  return value;
}

export function options(argv) {
  const [command, ...rest] = argv;
  const result = { command, config: 'relay/wrangler.toml', check: null, dryRun: false, entries: [] };
  for (let i = 0; i < rest.length; i++) {
    if (rest[i] === '--dry-run') result.dryRun = true;
    else if (rest[i] === '--config' || rest[i] === '--check') {
      if (!rest[i + 1]) throw new Error(`${rest[i]} needs a value.`);
      result[rest[i].slice(2)] = rest[++i];
    } else if (rest[i].startsWith('--')) throw new Error(`Unknown option ${rest[i]}.`);
    else result.entries.push(rest[i]);
  }
  if (command !== 'remove') throw new Error('Usage: node relay/operator.mjs remove [--config <wrangler.toml>] [--check <relay base URL>] [--dry-run] <room>[/<id>]… | v1');
  if (result.check) {
    const url = new URL(result.check);
    if (url.protocol !== 'https:' && !['localhost', '127.0.0.1', '[::1]'].includes(url.hostname)) throw new Error('--check needs the HTTPS relay base, for example https://relay.example/relay.');
    result.check = url.href.replace(/\/+$/, '');
  }
  return result;
}

async function removed(base) {
  const response = await fetch(`${base}/v2/meta`, { redirect: 'error', cache: 'no-store', signal: AbortSignal.timeout(10_000) });
  if (!response.ok) throw new Error(`The relay answered ${response.status}.`);
  return (await response.json()).removed;
}

async function main(argv) {
  const { config, check, dryRun, entries } = options(argv);
  const value = purgeValue(entries);
  const command = ['pnpm', 'dlx', 'wrangler@4', 'secret', 'put', 'HOI_RELAY_PURGE', '--config', config];
  if (dryRun) {
    console.log(`Would run: ${command.join(' ')}\nwith the value: ${value}`);
    return;
  }
  const before = check ? await removed(check) : null;
  const result = spawnSync(command[0], command.slice(1), { input: value, stdio: ['pipe', 'inherit', 'inherit'] });
  if (result.status !== 0) throw new Error('Wrangler did not store the removal list; nothing was removed.');
  if (!check) {
    console.log('Stored. The relay removes these on its next start; request /relay/v2/meta to wake it and compare `removed`.');
    return;
  }
  // Requests wake the Durable Object; a new version can take a few seconds to reach it.
  for (let attempt = 0; attempt < 12; attempt++) {
    const after = await removed(check);
    if (after !== before || attempt === 11) {
      console.log(`The relay reports ${after} removed identifiers (${before} before this batch).`);
      return;
    }
    await new Promise(done => setTimeout(done, 5000));
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2)).catch(error => { console.error(error.message); process.exitCode = 1; });
}
