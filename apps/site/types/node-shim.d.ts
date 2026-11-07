// The build plugins use two Node APIs. @types/node isn't a dependency of this package,
// so declare just those (these merge cleanly if @types/node is ever added).
declare module 'node:fs' {
  export function readFileSync(path: string, encoding: 'utf8'): string;
  export function readFileSync(path: string): Uint8Array;
  export function existsSync(path: string): boolean;
  export function readdirSync(path: string): string[];
}
declare module 'node:crypto' {
  export function createHash(algorithm: string): { update(data: string): { digest(encoding: 'hex'): string } };
}
declare const Buffer: { from(data: string, encoding: 'base64'): { toString(encoding: 'hex'): string } };
declare module 'node:url' {
  export function fileURLToPath(url: string | URL): string;
}
