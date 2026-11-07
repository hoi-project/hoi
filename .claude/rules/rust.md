# Rust (core/)

- The workspace is `core/`. Run cargo from there. The toolchain is pinned in
  `core/rust-toolchain.toml`; the minimum is `rust-version` in `core/Cargo.toml`.
- Format with `cargo fmt --all` (`core/rustfmt.toml`: width 160). Run `cargo clippy -p <crate>` for
  crates you touched and fix new warnings; do not silence them with `allow` unless you say why.
- Use `--locked` for builds and tests. Do not edit `Cargo.lock` by hand. A new dependency or
  feature needs a maintainer's agreement first (see AGENTS.md rule 9), a licence the project
  already accepts, and a `pnpm sbom` run.
- Patched crates live in `vendor/` and are wired through `[patch.crates-io]`; read
  `vendor/README.md` before touching them.
- Crate roles: `hoi-types` (wire types, CBOR), `hoi-crypto`, `hoi-validate` (the validation
  pipeline), `hoi-store`, `hoi-sync`, `hoi-trust`, `hoi-moderation`, `hoi-spam`, `hoi-net`,
  `hoi-core` (the facade), `hoi-node` (headless node), `hoi-wasm` and `hoi-ffi` (bindings),
  `hoi-sim` (simulation). Keep validation in `hoi-validate`; bindings stay thin.
- Never log or `Debug`-print keys, secrets, record bodies, tokens or peer addresses. Types that hold
  secrets do not derive `Debug` or `Serialize` for convenience.
- Errors are typed; do not `unwrap()` or `expect()` on input from the network, storage or users.
  `expect("…")` is fine in tests and for real invariants, with a message that states the invariant.
- Cite the spec where behaviour follows it: `// PROTOCOL.md §15.3`, `// RFC-0001`.
- Unit tests sit in `#[cfg(test)] mod tests` at the bottom of the file; cross-crate and conformance
  tests in `tests/`.
