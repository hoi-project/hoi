import type { Publication } from './publication.mjs';
import type { ReleaseConfig } from './release-schema.mjs';
export interface ReleaseFile { platform: string; kind: string; arch: string; product?: string; name: string; url: string; size: number; sha256: string }
export interface VerifiedManifest { schema: 1; latest: string; signature: string; release: ReleaseConfig; releases: { version: string; date: string; notes?: string; files: ReleaseFile[] }[]; updater?: unknown }
export function verifyMinisign(bytes: Uint8Array, signature: string, key: string): void;
export function compareVersions(a: string, b: string): number;
export function httpsUrl(raw: string): string;
export function parseManifest(bytes: Uint8Array, minimumVersion?: string | null): VerifiedManifest;
export function fetchBounded(url: string, max: number, fetcher?: typeof fetch): Promise<Uint8Array>;
export function fetchVerifiedManifest(config: Publication, minimumVersion?: string | null, fetcher?: typeof fetch): Promise<VerifiedManifest>;
