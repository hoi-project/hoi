# Testing

- A fix comes with a test that fails without it. A feature comes with tests for its behaviour and
  its error cases. Do not add tests for trivial getters.
- Never skip, disable or loosen a failing test to get a green run. If a test is wrong, fix it in
  the same pull request and say why.
- Where tests live:
  - Rust: `#[cfg(test)] mod tests` in the file, or `core/crates/<crate>/tests/`.
  - Conformance: `core/vectors/` and `core/crates/hoi-validate/tests/vectors.rs`.
  - Web app: `apps/web/src/tests/*.test.ts` (`pnpm --filter @hoi/web test`).
  - Build, release and boundary tooling: `scripts/*.test.mjs` (Node's test runner).
  - End-to-end: `scripts/e2e-*.mjs`, and `sandbox/` for isolated networks.
- Tests are deterministic and offline: no live DNS, public relays or real services. Use fixtures
  and documentation addresses (`192.0.2.0/24`, `2001:db8::/32`, `example.com`).
- Fixtures contain no real keys, personal data or copyrighted media.
- Run `pnpm check:quick` before every pull request, plus the focused suites for what you changed.
  Full integration and platform suites run for major releases (docs/CI.md); if your change touches
  their area, run the relevant one locally or say in the pull request that you could not.
