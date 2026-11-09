# RFC-0004: Validate the storage-holder privacy boundary directly

Issue: [#37](https://github.com/hoi-project/hoi/issues/37)

Status: proposed

## Summary

Test the actual requests and persisted data visible to a storage-only holder. Use existing
private-community encryption and membership rules; this RFC agrees on validation coverage,
not a new protocol or a claim of complete anonymity.

## Motivation

Private MLS delivery, membership removal and public/private isolation already have tests.
`scripts/e2e-mailboxes.mjs` checks holder counts and the description “none readable here,”
which does not verify the stored bytes. A regression exposing metadata could pass that check.

## Proposal

Extend the core network harness with test-only request capture and an inspectable piece store.
Exercise both private mailbox messages and scattered posts with attachments. Keep observation
inside tests; do not add production logging or an inspection endpoint.

Use three participant roles: sender, authorized reader and holder, with enough holders and
forwarders for the existing placement rules. The holder has its own transport/envelope keys,
but no MLS membership, community secrets or object-decryption keys.
The reader must recover the original post and attachment, establishing that the tested path
actually delivered usable content.

Check structure as well as bytes:

- Decode holder requests with existing parsers and assert the permitted fields for each
  operation. Inspect persisted rows, including after reopening SQLite: locator, expiry/touched
  days, drop hash, drop-key identifier and opaque bytes. No plaintext community/author mapping
  or content-decryption key belongs there.
- Use distinctive synthetic content and identifiers as leakage sentinels. Their absence is
  supplementary evidence, not a cryptographic proof or a substitute for checking fields,
  key ownership and access paths.
- Exercise known and unknown locators. `has` returns presence/length and `get` the permitted
  object result; neither exposes a community/post directory. Existing private-record and
  media-fetch denial tests remain required.
- Retain invitation-binding and removal tests. Document current epoch rotation and history
  sharing; leaving a group cannot erase keys or copies already received.

Documentation must list sizes, timing, network addresses, expiry and deletion metadata as
visible or correlatable. A shared drop-key identifier is a correlation handle. Public
communities remain publicly readable, and a holder operator who also joins a private group
can recognize content available to that membership.

## Alternatives

| Option | Assessment |
| --- | --- |
| CLI wording and counts only | Cheap, but does not inspect the privacy boundary |
| Byte searches only | Catch obvious plaintext, but miss structured or encoded leakage |
| Request/schema inspection plus delivery/access tests | Recommended: tests both retained data and the relevant access paths |

## Consequences and validation

No wire, storage, key-management or public API change is proposed. Test fixtures expose
synthetic data only. These regressions complement the independent review in #7.

Implementation is complete when both delivery paths pass the checks above, existing MLS and
isolation suites remain green, and applicable SQLite, end-to-end and wiki checks pass. Report
which suites ran; do not infer general security from a passing fixture.

Start with `core/crates/hoi-core/src/tests/forward.rs`, `private_mls.rs` and
`private_isolation.rs` in the same directory, `core/crates/hoi-store/src/pieces.rs`,
`core/crates/hoi-crypto/src/envelope.rs` and `scripts/e2e-mailboxes.mjs`.
