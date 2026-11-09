#!/usr/bin/env node
// Draft releases can have an untagged-* REST tag until publication. Use the ID returned by
// draft preparation, and check its source before transferring any bytes.
import { createReadStream, createWriteStream, existsSync, mkdirSync, renameSync, rmSync, statSync } from 'node:fs';
import { basename, join } from 'node:path';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import { fileURLToPath } from 'node:url';

const positiveId = value => /^[1-9][0-9]*$/.test(String(value));
const assetName = value => typeof value === 'string' && value.length > 0 && !/[\\/\x00-\x1f]/.test(value) && !['.', '..'].includes(value);

export async function transferAssets(action, paths, env = process.env, fetcher = fetch) {
  const { GH_REPO: repository, RELEASE_ID: id, TAG: tag, SOURCE_SHA: source, GH_TOKEN: token } = env;
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository ?? '') || !positiveId(id)
    || !/^v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$/.test(tag ?? '')
    || !/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(source ?? '') || !token) {
    throw new Error('Release assets need a repository, draft ID, version tag, source SHA and GitHub token.');
  }
  if (!['upload', 'download'].includes(action) || !paths.length || (action === 'download' && paths.length !== 1)) {
    throw new Error('Usage: release-assets.mjs upload FILE... | download DIRECTORY');
  }
  const base = `https://api.github.com/repos/${repository}`;
  const headers = { Authorization: `Bearer ${token}`, Accept: 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28' };
  const request = async (url, options = {}) => {
    const response = await fetcher(url, { ...options, headers: { ...headers, ...options.headers }, signal: AbortSignal.timeout(600_000) });
    if (!response.ok) throw new Error(`Release asset request failed (HTTP ${response.status}).`);
    return response;
  };
  const release = await (await request(`${base}/releases/${id}`)).json();
  const markers = (release.body ?? '').split(/\r?\n/).filter(line => line.startsWith('<!-- hoi-release-source:'));
  const expectedMarker = `<!-- hoi-release-source: ${source} -->`;
  const matchingTag = release.tag_name === tag || (/^untagged-[0-9a-f]+$/.test(release.tag_name ?? '') && release.name === `hoi ${tag}`);
  if (String(release.id) !== String(id) || release.draft !== true || !matchingTag
    || markers.length !== 1 || markers[0] !== expectedMarker) {
    throw new Error('The draft ID, tag and source must match the validated release.');
  }
  const commit = await (await request(`${base}/commits/${encodeURIComponent(tag)}`)).json();
  if (commit.sha !== source) throw new Error('The release tag no longer points to the validated source.');
  const assets = [];
  for (let page = 1; ; page++) {
    const batch = await (await request(`${base}/releases/${id}/assets?per_page=100&page=${page}`)).json();
    if (!Array.isArray(batch) || batch.some(asset => !positiveId(asset.id) || !assetName(asset.name))) {
      throw new Error('Invalid release asset listing.');
    }
    assets.push(...batch);
    if (batch.length < 100) break;
  }
  if (action === 'upload') {
    const files = paths.map(path => ({ path, name: basename(path), size: statSync(path).size }));
    if (files.some(file => !assetName(file.name) || !statSync(file.path).isFile())
      || new Set(files.map(file => file.name)).size !== files.length) throw new Error('Upload files must have unique, valid asset names.');
    for (const file of files) {
      // Same-name assets are replaced only within this verified draft, as with --clobber.
      for (const asset of assets.filter(asset => asset.name === file.name)) {
        await request(`${base}/releases/assets/${asset.id}`, { method: 'DELETE' });
      }
      await request(`https://uploads.github.com/repos/${repository}/releases/${id}/assets?name=${encodeURIComponent(file.name)}`, {
        method: 'POST', headers: { 'Content-Type': 'application/octet-stream', 'Content-Length': String(file.size) },
        body: createReadStream(file.path), duplex: 'half',
      });
    }
  } else {
    const [dir] = paths;
    mkdirSync(dir, { recursive: true });
    for (const asset of assets) {
      const path = join(dir, asset.name);
      if (existsSync(path)) continue;
      const response = await request(`${base}/releases/assets/${asset.id}`, { headers: { Accept: 'application/octet-stream' } });
      try {
        await pipeline(Readable.fromWeb(response.body), createWriteStream(`${path}.partial`));
        renameSync(`${path}.partial`, path);
      } finally { rmSync(`${path}.partial`, { force: true }); }
    }
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try { await transferAssets(process.argv[2], process.argv.slice(3)); }
  catch (error) { console.error(`release-assets: ${error.message}`); process.exitCode = 1; }
}
