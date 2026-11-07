import type { CommunityRegistration, CommunityPublication } from '@hoi/core-api';
import { validateRelease, type ReleaseConfig } from '../../../../scripts/release-schema.mjs';
import { RELEASE } from './preview';
import { readRegistryJson, verifyRegistryEntry, registryPins, registryLogPins, type RegistryLogPins, type RegistryPins, type RegistryProof, type VerifiedRegistryEntry } from '../../../../registry/public/verify.js';
import { fromHex, logEntryText, logLeaf, toHex, verifiedLogHead, verifyConsistency, verifyInclusion } from '../../../../registry/public/log.js';

export interface RegistryEntry { name: string; community: string; proof?: RegistryProof; protocolVersion?: string | null; indexVersion?: number | null; compatible?: boolean; indexed?: boolean }
export interface RegistryMetadata { release: ReleaseConfig; publicNodes: string[]; appUrl: string; registry: { apiVersion: number; schemaVersion: number; indexVersion: number } }
export const canJoinPublic = (value: RegistryEntry): boolean => value.indexed === true && value.compatible === true && value.indexVersion === RELEASE.indexVersion && typeof value.protocolVersion === 'string' && RELEASE.supportedProtocols.includes(value.protocolVersion);
export const publicNameValid = (name: string): boolean => /^[a-z0-9][a-z0-9._-]{0,31}$/.test(name);
/** Search accepts the familiar c/name spelling without changing signed names. */
export const registryQuery = (query: string): string => query.trim().toLowerCase().replace(/^c\//, '');

export function registryBase(value: string): string {
  if (value.replace(/\/+$/, '') === '/registry') return '/registry';
  if (!value.trim()) return '';
  const url = new URL(value);
  const loopback = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
  if (url.protocol !== 'https:' && !(url.protocol === 'http:' && loopback)) throw new Error('The public registry needs HTTPS (HTTP is allowed only on localhost).');
  if (url.username || url.password || url.search || url.hash) throw new Error('The registry URL cannot contain credentials, a query or a fragment.');
  return url.href.replace(/\/$/, '');
}

export const REGISTRY_URL = registryBase(import.meta.env.VITE_HOI_REGISTRY_URL ?? '');

export class RegistryConflict extends Error {
  constructor(public readonly entry: RegistryEntry) {
    super(`c/${entry.name} is already registered to another community. Your community remains local and unregistered.`);
  }
}

const unlogged = () => new Error('This registry did not show this name in its public log. Joining has been stopped. Use an invite from someone you trust.');
const rewritten = () => new Error('This registry\'s public log no longer matches what this device saw before. Public discovery has been stopped. Use an invite from someone you trust.');

/** Checks a verified entry against the registry's public log. */
export interface RegistryLogCheck { check(entry: VerifiedRegistryEntry): Promise<void> }

/** The log check: the registry's signed head extends the last one this device saw (a consistency
 * proof), and the entry is in it (an audit path). Then that head is kept. A registry that gives
 * two people two keys for one name must show both entries to anyone who reads its log. */
export function registryLog(get: (path: string) => Promise<Response>, heads: () => RegistryLogPins): RegistryLogCheck {
  const json = async (response: Response) => response.ok ? await readRegistryJson(response) as Record<string, unknown> : null;
  return {
    async check(entry) {
      const head = await verifiedLogHead(await json(await get('/v2/log/head')));
      if (!head) throw unlogged();
      const saved = await heads().get();
      if (saved) {
        if (saved.logKey !== head.logKey || head.size < saved.size || (head.size === saved.size && head.root !== saved.root)) throw rewritten();
        if (head.size > saved.size && saved.size > 0) {
          const proof = await json(await get(`/v2/log/consistency?first=${saved.size}&second=${head.size}`));
          if (!Array.isArray(proof?.path) || !await verifyConsistency(saved.size, head.size, fromHex(saved.root), fromHex(head.root), proof.path as string[])) throw rewritten();
        }
      }
      const leaf = await logLeaf(logEntryText(entry));
      const proof = await json(await get(`/v2/log/proof?leaf=${toHex(leaf)}&size=${head.size}`));
      if (typeof proof?.index !== 'number' || !Array.isArray(proof.path) || !await verifyInclusion(leaf, proof.index, head.size, proof.path as string[], fromHex(head.root))) throw unlogged();
      await heads().advance(head);
    },
  };
}

/** A connection timeout or failed request never becomes a claim that a name is available.
 * Exact lookups and writes are checked against the registry's public log; directory pages are
 * not, one request each, and a name is checked when it is opened. */
export function registryClient(base: string, fetcher: typeof fetch = fetch, savedPins?: RegistryPins, options: { log?: RegistryLogCheck } = {}) {
  const root = registryBase(base);
  const pins = () => savedPins ?? registryPins(root);
  const log = options.log ?? registryLog((path) => request('', undefined, path), () => registryLogPins(root));
  const remember = async (result: VerifiedRegistryEntry) => { await pins().accept([result]); await log.check(result); return result; };
  const checked = (response: Response, indexed = false) => readRegistryJson(response).then(value => verifyRegistryEntry(value, indexed ? RELEASE : undefined));
  async function directory(response: Response, prefix: string, indexed = false) {
    const values = await readRegistryJson(response);
    if (!Array.isArray(values) || values.length > 50) throw new Error('The registry returned an invalid directory.');
    const results = await Promise.all(values.map(value => verifyRegistryEntry(value, indexed ? RELEASE : undefined)));
    if (results.some(value => !value.name.startsWith(prefix))) throw new Error('The registry returned a different name.');
    await pins().accept(results);
    return results;
  }
  async function request(query: string, init?: RequestInit, path = '/v1/communities'): Promise<Response> {
    if (!root) throw new Error('Public names are unavailable right now. You can still use a community invite.');
    try {
      return await fetcher(`${root}${path}${query}`, {
        ...init, credentials: 'omit', referrerPolicy: 'no-referrer', cache: 'no-store', redirect: 'error', signal: AbortSignal.timeout(10_000),
      });
    } catch {
      throw new Error('The public directory is unreachable. Check your connection and try again. Your public name has not been confirmed.');
    }
  }
  return {
    async metadata(): Promise<RegistryMetadata> {
      const response = await request('', undefined, '/v1/meta');
      if (!response.ok) throw new Error(`Public node information is unavailable (${response.status}).`);
      const value = await readRegistryJson(response) as RegistryMetadata;
      if (!value || !Array.isArray(value.publicNodes) || value.publicNodes.length > 16 || value.publicNodes.some((v: unknown) => typeof v !== 'string' || v.length > 128) ||
          typeof value.appUrl !== 'string' || value.registry?.apiVersion !== 2 || value.registry?.schemaVersion !== 3 || !Number.isSafeInteger(value.registry?.indexVersion)) throw new Error('The registry returned invalid service information.');
      return { release: validateRelease(value.release), publicNodes: value.publicNodes, appUrl: value.appUrl, registry: value.registry };
    },
    /** Routes to try for one community: nodes that announced they carry it first, then the
     * operator's configured nodes, then other live nodes. Never more than the metadata limit. */
    async nodesFor(community: string): Promise<string[]> {
      if (!/^[0-9a-f]{64}$/.test(community)) throw new Error('Invalid community key.');
      const response = await request(`?community=${community}`, undefined, '/v2/nodes');
      if (!response.ok) throw new Error(`Public node information is unavailable (${response.status}).`);
      const value = await readRegistryJson(response) as { publicNodes?: unknown };
      const nodes = value?.publicNodes;
      if (!Array.isArray(nodes) || nodes.length > 16 || nodes.some((v: unknown) => typeof v !== 'string' || v.length > 128)) throw new Error('The registry returned invalid public node information.');
      return nodes as string[];
    },
    async lookupIndexed(name: string): Promise<RegistryEntry | null> {
      if (!publicNameValid(name)) throw new Error('Invalid public name.');
      const response = await request(`?name=${encodeURIComponent(name)}`, undefined, '/v2/communities');
      if (response.status === 404) { await pins().checkMissing(name); return null; }
      if (!response.ok) throw new Error(`The public-name lookup failed (${response.status}). Availability is unknown; retry later.`);
      const result = await checked(response, true);
      if (result.name !== name) throw new Error('The registry returned a different name.');
      return remember(result);
    },
    async searchIndexed(prefix: string): Promise<RegistryEntry[]> {
      if (prefix && !publicNameValid(prefix)) return [];
      const response = await request(`?prefix=${encodeURIComponent(prefix)}`, undefined, '/v2/communities');
      if (!response.ok) throw new Error(`The public directory is unavailable (${response.status}). Retry later.`);
      return directory(response, prefix, true);
    },
    async publish(proof: CommunityPublication): Promise<RegistryEntry> {
      if (!publicNameValid(proof.name)) throw new Error('Invalid public name.');
      const response = await request('', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(proof) }, '/v2/communities');
      if (response.status === 409) {
        const result = await checked(response, true);
        if (result.name !== proof.name) throw new Error('This community key already has a different public name.');
        if (result.community === proof.community) throw new Error('This key is already reserved under different, immutable protocol or index metadata. Use its original compatible app and invite.');
        throw new RegistryConflict(await remember(result));
      }
      if (!response.ok) throw new Error(`The public name was not published (${response.status}). Your local community is saved; retry later with a compatible app.`);
      const result = await checked(response, true);
      if (result.name !== proof.name || result.community !== proof.community || result.protocolVersion !== proof.protocolVersion || result.indexVersion !== proof.indexVersion || !canJoinPublic(result)) throw new Error('The registry did not confirm a compatible public listing. Your local community is saved.');
      return remember(result);
    },
    async lookup(name: string): Promise<RegistryEntry | null> {
      if (!publicNameValid(name)) return null;
      const response = await request(`?name=${encodeURIComponent(name)}`);
      if (response.status === 404) { await pins().checkMissing(name); return null; }
      if (!response.ok) throw new Error(`The public-name lookup failed (${response.status}). Availability is unknown; retry later.`);
      const result = await checked(response);
      if (result.name !== name) throw new Error('The registry returned a different name.');
      return remember(result);
    },
    async search(prefix: string): Promise<RegistryEntry[]> {
      if (prefix && !publicNameValid(prefix)) return [];
      const response = await request(`?prefix=${encodeURIComponent(prefix)}`);
      if (!response.ok) throw new Error(`The public directory is unavailable (${response.status}). Retry later.`);
      return directory(response, prefix);
    },
    async reserve(proof: CommunityRegistration): Promise<RegistryEntry> {
      if (!publicNameValid(proof.name)) throw new Error('Public names use 1–32 lowercase letters, digits, dots, dashes or underscores, starting with a letter or digit.');
      const response = await request('', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(proof) });
      if (response.status === 409) {
        const conflict = await checked(response);
        if (conflict.name !== proof.name) throw new Error('This community key already has a different public name, or the registry returned an inconsistent conflict.');
        throw new RegistryConflict(await remember(conflict));
      }
      if (!response.ok) throw new Error(`The public name was not registered (${response.status}). Your local community is saved; retry registration later.`);
      const result = await checked(response);
      if (result.name !== proof.name || result.community !== proof.community) throw new Error('The registry did not confirm this community. Retry to verify its reservation.');
      return remember(result);
    },
  };
}

export const lookupName = (name: string) => registryClient(REGISTRY_URL).lookupIndexed(name);
export const searchNames = (prefix: string) => registryClient(REGISTRY_URL).searchIndexed(prefix);
export const publishName = (proof: CommunityPublication) => registryClient(REGISTRY_URL).publish(proof);
export const reserveName = (proof: CommunityRegistration) => registryClient(REGISTRY_URL).reserve(proof);
