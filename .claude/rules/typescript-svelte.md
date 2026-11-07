# TypeScript and Svelte (apps/, packages/)

- pnpm workspace (`pnpm-workspace.yaml`). Use `pnpm --filter <package>`; never npm or yarn, and
  always `pnpm install --frozen-lockfile` unless a maintainer agreed to a dependency change.
- The web app uses Svelte 5 runes. Shared reactive state lives in `*.svelte.ts` modules in
  `apps/web/src/lib/`.
- The app and website are static: no server code, no telemetry, no third-party requests, no CDN.
  Bundle fonts, images and scripts locally.
- The Rust core runs as WASM in a Web Worker. Talk to it through the API in `packages/core-api`;
  keep those types aligned with the Rust DTOs. Do not reimplement validation or crypto in
  TypeScript.
- Styling uses the generated tokens. Change `design/tokens/tokens.json`, then `pnpm tokens`; no
  hard-coded colours or spacing.
- Accessibility: every control is reachable by keyboard and has an accessible name, touch targets
  are at least 24 px, motion respects `prefers-reduced-motion`, and layouts work at 320 px wide.
- Interface copy is short and plain. No email addresses or deployment hints in pages
  (`scripts/public-copy.test.mjs`).
- `pnpm check` must report zero errors and zero warnings. UI work without a WASM build:
  `pnpm --filter @hoi/web dev:mock`.
