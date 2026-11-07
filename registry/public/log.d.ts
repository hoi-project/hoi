import type { VerifiedRegistryEntry } from './verify.js';
export interface SignedLogHead { logKey: string; size: number; root: string; timestamp: number; signature: string }
export function logEntryText(entry: Pick<VerifiedRegistryEntry, 'name' | 'community' | 'proof'>): string;
export function logHeadStatement(logKey: string, size: number, root: string, timestamp: number): string;
export function toHex(bytes: Uint8Array): string;
export function fromHex(hex: string): Uint8Array;
export function logLeaf(entryText: string): Promise<Uint8Array>;
export function verifyInclusion(leaf: Uint8Array, index: number, size: number, path: string[], root: Uint8Array): Promise<boolean>;
export function verifyConsistency(first: number, second: number, firstRoot: Uint8Array, secondRoot: Uint8Array, path: string[]): Promise<boolean>;
export function verifiedLogHead(value: unknown): Promise<SignedLogHead | null>;
