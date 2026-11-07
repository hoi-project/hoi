# Protocol, storage and cryptography

Changes here can lose user data or break compatibility between clients. Treat them as their own
pull request, never as a side effect of a fix or feature.

- Ask in an issue before changing wire formats, record kinds, signatures, hashes, validation
  stages, storage layouts or key handling (AGENTS.md rule 9).
- Every protocol change updates, in the same pull request:
  1. the Rust types and validation in `core/`, with tests;
  2. the conformance vectors: `cargo run -p hoi-validate --example vectors` regenerates
     `core/vectors/`, and `cargo test -p hoi-validate` fails while they are stale;
  3. `packages/core-api`, if a DTO changed;
  4. `docs/spec/PROTOCOL.md` (keep existing section numbers; add new subsections at the end of a
     section), following an accepted RFC in `rfcs/`;
  5. the wiki page that describes it (`pnpm check:wiki`).
- Never edit an accepted RFC. Supersede it with a new one that says what changed and why.
- Do not invent cryptography. Use the primitives already in `hoi-crypto` and the vendored,
  reviewed crates. A new primitive or construction needs a maintainer and an outside review.
- Storage changes need a migration and a test that opens data written by the previous version.
- Hidden or encrypted content must stay unreadable to nodes that only relay or store it. Say in the
  pull request what metadata a change exposes, even when it is "none".
