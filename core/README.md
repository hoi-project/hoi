# core

The Rust workspace: one core that does all the thinking. UIs reach it through `hoi-core`'s API: JSON dispatch (`Node::call`) from the web worker and the desktop shell, typed UniFFI bindings from iOS and Android.

| Crate | What it does |
|---|---|
| `hoi-types` | records (incl. identity, vouch, review, media refs), ids, canonical DAG-CBOR (strict decoder), limits and the ladder, delegation statements, did:key, `hoi://` links, scattered delivery's size classes and manifest (`scatter.rs`) |
| `hoi-crypto` | strict Ed25519 with weak-key refusal, BLAKE3 stamps (prefix-cached, multi-core miner), Argon2id identity work, Argon2id + XChaCha20-Poly1305 keystore, fingerprints; scattered delivery's pieces (`scatter/`: object seal, all-or-nothing transform, Reed–Solomon over GF(2⁸), locators, drops, exact sizing) and HPKE envelopes (`envelope.rs`) |
| `hoi-validate` | the pure pipeline stages: decode, stamp floor, signatures and consent, identity, time, rate windows, content (`text.rs`: Unicode and link rules; `media.rs`: sniffing, sanitizing, checking); property tests; the vectors generator |
| `hoi-store` | in-memory index (identities, delegations, revocations, burns, media blobs), expiry, `Persist` (SQLite natively, a journal for IndexedDB) |
| `hoi-moderation` | community state replayed from the signed ledger (thresholds, mods, bans, removals with contest windows, labels, pins, admissions) and `ladder.rs`: standing, holds, hides, blocks, vetoes, strikes, suspensions, settled as a fixpoint |
| `hoi-trust` | trust levels, weights, hot/fresh/loudest ranking (local only) |
| `hoi-spam` | decaying strikes, peer scores |
| `hoi-sync` | `hello` / `have` / `want` / `recs` / `bwant` / `blob` / `bye` messages with caps |
| `hoi-core` | `Node`: ingest pipeline, effects, the API, DTOs, JSON dispatch, sync conversation |
| `hoi-net` | WebSocket home-node link (serve and dial), sweeper; iroh QUIC and WebRTC transports |
| `hoi-wasm` | the browser build (wasm-bindgen) and the stamp miner for worker pools |
| `hoi-ffi` | UniFFI bindings for Swift and Kotlin |
| `hoi-node` | headless home node and pinner |

```sh
cargo test --workspace                                  # everything
cargo run -p hoi-validate --example vectors             # regenerate core/vectors/
node ../scripts/test-core-wasi.mjs                      # crypto, types and vectors in wasm (wasm32-wasip1)
(cd fuzz && cargo +nightly fuzz run manifest)           # fuzz a decoder: `cargo fuzz list` names them
cargo run -p hoi-node -- --listen 127.0.0.1:7842        # a home node
node ../scripts/build-wasm.mjs                          # browser build into apps/web
```
