// Types for the shared JS plugin in packages/vite-plugin-hardening (it ships JSDoc only).
declare module '@hoi/vite-plugin-hardening' {
  import type { Plugin } from 'vite';
  export type Directives = Record<string, string[]>;
  export function cspString(directives: Directives): string;
  export default function hardening(opts?: { csp?: Directives | false; sri?: boolean }): Plugin;
  export function headers(directives: Directives, extra?: Record<string, string>): string;
}
