# RFC-0006: Separate node declarations from verified execution evidence

Issue: [#39](https://github.com/hoi-project/hoi/issues/39)

Status: proposed

## Summary

Extend existing node consent with a readable configuration declaration and an explicit
assurance policy. Recommend two clearly separated modes: operator-declared configuration,
and strict verification using protected remote attestation. Strict mode refuses unsupported
or unverifiable paths instead of silently using declarations as execution evidence.

A concrete attestation profile remains an approval prerequisite; no vendor, verifier service
or new cryptography is selected.

## Motivation

hoi already asks users to accept supplied notices, repeats the question when a notice changes,
and avoids declined nodes. Signed node links authenticate the notice. Notice-less peers and
headless nodes currently bypass that interactive gate.

These mechanisms do not prove which software runs. A modified program can sign the expected
configuration. Remote attestation instead evaluates evidence from an isolated, trusted
measurement/signing environment; its assurance depends on that environment, the verifier and
the appraisal policy. See [RFC 9334 §§7, 10, 12](https://www.rfc-editor.org/rfc/rfc9334.html).

## Proposal

### 1. Show what is declared and what is verified

Define a versioned, canonical declaration containing endpoint identity, notice hash, software
digest and effective nonsecret settings: node roles, enabled services, data handled, storage
and retention, and relevant access/forwarding policies. Generate software-controlled fields
from effective configuration, rather than copying operator prose. Exclude secrets and private
membership lists. Host-level logging remains an operator claim unless the selected profile
actually covers it.

Sign the declaration with the endpoint key using a domain-separated statement.
Display its fields and assurance level before application traffic. Initial
transport addresses, handshake traffic and timing remain observable. Approval binds to the
endpoint, declaration hash and assurance policy, not merely a hostname or version string.

### 2. Require a concrete attestation profile

Before strict mode can ship, approve a profile specifying:

- Protected code/configuration measurements, trusted reference values and how releases map
  to them; unmeasured executables or configuration cannot inherit verified status.
- Trust anchors, verifier ownership, certificate/revocation handling and maximum evidence age.
- A fresh client challenge and binding between evidence, declaration hash and the connection's
  cryptographic key; that key must be controlled by the measured environment.
- Revalidation on reconnect, expiry and measured configuration/software changes, plus how the
  protected runtime stops traffic when its approved state changes.
- Bounded evidence sizes, wire negotiation, verification costs and failure behavior.

Attestation can establish a measured state, not prove every instruction or absence of hidden
reads outside its boundary. A boot measurement alone is insufficient for arbitrary later code
changes. See the [confidential-VM attestation overview](https://docs.cloud.google.com/confidential-computing/confidential-vm/docs/attestation-overview) for an existing model.

### 3. Enforce the policy across the selected path

For strict mode, every application-level participant handling the requested operation must
meet its role's profile: entry/home nodes, relays/forwarders, holders and discovery services.
A verified entry node does not verify downstream execution. Evidence for
remote hops must bind to selected endpoints; an unknown or unsupported
hop makes that path ineligible. This does not attest ordinary internet routers.

Users approve each endpoint/configuration before use. Headless nodes use explicit
operator policy instead of prompts. Notice-less peers,
including member devices, are ineligible in strict mode until they support the required
disclosure/evidence. Declared-only use is a separate, explicit choice, never an automatic
fallback. Until a profile and forwarding integration exist, no path is labelled verified.

### 4. Stop, block and select again

On invalid, missing, expired or mismatched required evidence, stop queued application traffic,
disconnect and persist a local block with its reason. Selection excludes that endpoint and
known bound routes. Retry an eligible alternative only after its own approval; otherwise show
an offline/unavailable result. Users can explicitly reconsider a block. A legitimate announced
change suspends traffic and requires renewed consent and verification before resuming.

Blocks survive restart and candidate-cache eviction. They
identify endpoints, not people: a new key is a new, unapproved identity, not proof the operator
was permanently excluded. Failure to verify does not itself prove malicious behavior.

## Alternatives

| Option | Consequence |
| --- | --- |
| Signed declarations only | Broad compatibility; cannot establish running-code integrity |
| Explicit declaration and strict modes — recommended | Honest boundaries and gradual adoption; strict availability may initially be zero |
| Mandatory attestation for all traffic | Stronger admission policy, but excludes unsupported devices and requires a complete profile first |

## Consequences and compatibility

Add negotiated, versioned declaration/evidence messages and stored approval/block policy.
Existing acceptance remains valid only for the exact old notice; a new configuration
declaration needs its own approval and never inherits verified status.
Update Rust validation, DTOs, bindings, protocol vectors and PROTOCOL §14.5 together.

Measurements and configuration fingerprints can correlate machines. Verifiers may learn
connection metadata; evidence exchange must not publish secrets or create a public device
directory. Hardware availability, verifier outages, certificate updates and operating costs
become dependencies. No default external verifier is introduced.

## Validation required

Use deterministic fixtures for accepted, forged, replayed, expired, missing and wrong-session
evidence; changed measured code with unchanged notice; legitimate updates; unsupported peers;
headless policy; and an unverifiable intermediary. Assert zero application traffic before
approval, durable blocking, no downgrade, separately approved fallback and explicit failure
when no route remains. A real profile additionally requires hardware-backed positive/negative
tests and independent review; mocks cannot establish attestation security.

Start: `core/crates/hoi-core/src/notice.rs`, `core/crates/hoi-types/src/notice.rs`,
`core/crates/hoi-core/src/nodes.rs`, `core/crates/hoi-core/src/forward.rs` and
`core/crates/hoi-core/src/tests/notice.rs`.
