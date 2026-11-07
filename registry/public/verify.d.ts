export type RegistryProof = { version: 1; signature: string } | { version: 2; signature: string; protocolVersion: string; indexVersion: number };
export interface VerifiedRegistryEntry {
  name: string;
  community: string;
  proof: RegistryProof;
  protocolVersion?: string | null;
  indexVersion?: number | null;
  compatible?: boolean;
  indexed?: boolean;
}
export interface RegistryPins {
  accept(entries: VerifiedRegistryEntry[]): Promise<void>;
  checkMissing(name: string): Promise<void>;
}
export function readRegistryJson(response: Response): Promise<unknown>;
export function verifyRegistryEntry(value: unknown, release?: { indexVersion: number; supportedProtocols: string[] }): Promise<VerifiedRegistryEntry>;
export function registryPins(scope: string, factory?: IDBFactory): RegistryPins;
export interface LogHead { logKey: string; size: number; root: string }
export interface RegistryLogPins {
  get(): Promise<LogHead | null>;
  advance(head: LogHead): Promise<void>;
}
export function registryLogPins(scope: string, factory?: IDBFactory): RegistryLogPins;
