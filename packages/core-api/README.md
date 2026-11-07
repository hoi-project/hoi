# @hoi/core-api

The typed surface every hoi UI calls (`HoiCore` in `src/types.ts`). UIs never touch keys, storage or the network directly; they call this.

| Shell | Implementation | Where the core runs |
|---|---|---|
| Web (PWA) | `apps/web/src/lib/core/worker-core.ts` | `hoi-wasm` in a Web Worker, records in IndexedDB |
| Desktop (Tauri 2) | `apps/web/src/lib/core/tauri-core.ts` | native `hoi-core` behind one Tauri command |
| iOS / Android | generated UniFFI bindings (`apps/mobile`) | native `hoi-core` via `hoi-ffi` |
| Dev | `apps/web/src/lib/core/mock-core.ts` | in memory, seeded, no crypto |

The Rust DTOs in `core/crates/hoi-core/src/dto.rs` mirror these types field for field (serde `camelCase`). Change both in the same commit.
