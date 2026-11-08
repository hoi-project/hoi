# Dependency compatibility and security patches

These are temporary, locally maintained patches of existing dependencies, not new crypto
implementations or an independently audited fork. Keep their licenses and upstream attribution.
Replace the patches with reviewed upstream releases when compatible releases are available.
The two application workspaces intentionally use the same `[patch.crates-io]` entries.

## Release pipeline fixes

- `netwatch-0.19.3` is the published crate at commit
  `0030e57fc70981895bb86212ff0f059cc41c8a4e`, with both upstream licenses and the
  normalized manifest preserved. It backports the Rust source changes from
  [net-tools#267](https://github.com/n0-computer/net-tools/pull/267) to define
  Darwin routing constants locally, including the macOS test that compares
  them with libc. libc 0.2.190 and later expose these constants only on macOS.
  The backport retains the existing netdev version and its patch below.

- `netdev-0.46.3` is the published crate at commit
  `4505d238696975a97b029c1b8607487453c0f86e`, with its MIT license and normalized
  manifest preserved. Only `src/os/darwin/ipv6_addr_flags.rs` changes: provide the
  XNU IPv6 interface request layout locally because libc 0.2.191 exports
  `in6_ifreq` only on macOS. iOS uses the same ioctl. The full 288-byte request
  includes space for the union's 34-counter statistics member; compile-time
  assertions check its size, field offset and macOS libc ABI. Both workspaces
  patch this version. The Windows-only transitive netdev 0.45.1 stays upstream.
  See [Apple's request definition](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/netinet6/in6_var.h).
- `deepmerge` contains the published 4.3.1 CommonJS distribution and declarations,
  with its MIT license. Both source and target key copies now reject `__proto__`,
  `constructor` and `prototype`, including nested merges and null-prototype
  targets. This fixes [GHSA-gcqx-w48m-w2h4](https://github.com/advisories/GHSA-gcqx-w48m-w2h4),
  which has no patched upstream release as of 2026-11-02. The pnpm override is
  scoped to the Svelte Vite plugin's existing build dependency; the published
  version is preserved and no audit advisory is ignored. The minimal package
  manifest removes upstream development tooling. `scripts/deepmerge.test.mjs`
  verifies the plugin resolves these exact patched bytes and exercises malicious
  keys as well as Svelte's array replacement and function options.

Replace these patches with compatible upstream fixes when available. Re-run
`pnpm audit --audit-level low`, both Cargo policy checks, the desktop frontend
build, and the iOS device/simulator builds when updating them.

## Sources and changes

| Directory | Published baseline and upstream source | Local change |
| --- | --- | --- |
| `openmls_rust_crypto` | 0.6.0, OpenMLS commit `3a3e35de3feeca8f6605143c464d5452ae584d43` | Apply upstream crypto API migration `055a2b4b06b595585783ee488e28eebd62212e2a`; upgrade HPKE dependencies to 0.8 as in `e0b21a70a55a9014b8de2772b36bf3d638adad51`; make unused PQ dependencies feature-gated as upstream did in `27ddc74e9ebf5c154335919ec97643b1792af6f9`. |
| `openmls_basic_credential` | 0.6.0, same published OpenMLS commit | Apply the same upstream crypto API migration and feature-gate optional ML-DSA. Preserve the released serialization and storage APIs. |
| `dimpl` | 0.7.6, commit `0c196c24438dfa1a14eace130c27f54f1e482b66` | Upgrade P-256/P-384 to 0.14 and ECDSA to 0.17. Use their `Generate` trait for ephemeral P-curve keys and the matching PKCS#8 decoding trait. The DTLS state machine, wire protocol and cipher algorithms are unchanged. |
| `core-models` | 0.0.7, libcrux commit `c5fb80f37530ee9b2df9501ae5ff8cb4a973a4bd` | Only change the `hax-lib` dependency from 0.3.7 to 0.4.1, matching the upstream libcrux workspace. This crate models Rust operations for proof tooling under `cfg(hax)`; it is not active in hoi's production runtime. |

Upstream references:

- [OpenMLS crypto migration](https://github.com/openmls/openmls/commit/055a2b4b06b595585783ee488e28eebd62212e2a)
- [OpenMLS HPKE migration](https://github.com/openmls/openmls/commit/e0b21a70a55a9014b8de2772b36bf3d638adad51)
- [OpenMLS optional dependency cleanup](https://github.com/openmls/openmls/commit/27ddc74e9ebf5c154335919ec97643b1792af6f9)
- [dimpl baseline](https://github.com/algesten/dimpl/tree/0c196c24438dfa1a14eace130c27f54f1e482b66)
- [libcrux workspace dependencies](https://github.com/celabshq/libcrux/blob/9d5290ec8e432caad9172e4e02c9062e8fcf4b7f/Cargo.toml)

`openmls-crypto-migration.patch` is the exact upstream source migration with paths adjusted
for this directory. It was applied to published 0.6.0 source, instead of copying all of
OpenMLS main: this preserves the newer 0.9.2 release's targeted-message security interface.
Manifest version changes follow those upstream migrations; normalized Cargo manifests are
retained so the patches do not depend on the upstream workspace.

The OpenMLS provider migration raises its minimum Rust version to 1.94; hoi already pins
Rust 1.97. Both application lockfiles now resolve `crypto-bigint` 0.7.8, above the patched
minimum 0.7.6 for [RUSTSEC-2026-0387](https://rustsec.org/advisories/RUSTSEC-2026-0387.html).
The old 0.5.5 version and `proc-macro-error2` are absent. Advisory policy ignores were not
expanded to achieve this result.

## Validation and maintenance

- Run `node scripts/check-dependency-security.mjs`; it rejects vulnerable duplicate versions,
  the unmaintained macro, or removal of the reviewed patches in either lockfile.
- Run `cargo deny check` in both application workspaces for current advisories, licenses,
  sources and dependency policy. The fast regression guard does not replace current advisories.
- Run `cargo test --locked -p hoi-core -p hoi-net --test crypto_backend` in `core/` for all
  supported MLS suites, AEAD authentication, signing, HPKE exporters, DTLS key exchange and
  interoperability with rcgen/ring certificates, including invalid inputs and tampering.
- Run the complete Rust tests, native/desktop checks, WASM build and browser private-community
  tests. The web and Rust CI filters include changes under `vendor/`.

Proof-tool source was separately compiled against hax 0.4.2. This is a compatibility check,
not a rerun of the upstream formal proofs. New crypto advisories or upstream releases require
reassessment even when these functional tests pass.
