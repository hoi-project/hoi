# RFC-0005: Validate availability against explicit holder-failure conditions

Issue: [#38](https://github.com/hoi-project/hoi/issues/38)

Status: proposed

## Summary

Extend deterministic network tests with a fault matrix connecting retrieval, repair and
deletion. Document the conditions under which each result holds. Existing erasure coding,
routing and repair remain unchanged.

## Motivation

The core already verifies pieces, tries alternatives and repairs generations. Separate
tests do not establish which combinations of failures remain survivable.

## Proposal

Use `Mesh` in `core/crates/hoi-core/src/tests/forward.rs`, with test-only faults at holders
and forwarding paths. Exercise production validation and scheduling; do not replace them
with a mock algorithm.

For every case, record the manifest's reconstruction threshold `k`, initial piece count `n`,
valid reachable indexes, usable paths, placement caps and simulated deadline. Distinct
endpoints do not establish independent operators. Pieces must belong to a compatible
generation; counting arbitrary pieces across generations is insufficient.

| Fault or state | Required observation |
| --- | --- |
| Holder timeout or false absence | Another permitted piece/path is tried; retrieval succeeds when enough valid pieces remain reachable within the fixture's budgets |
| Corrupt returned bytes | Hash validation rejects them; verified alternatives recover the original object |
| Fewer than `k` valid pieces, no matching local copy | Unavailable/retryable result, never fabricated content or deletion |
| Matching local copy available | Existing recovery succeeds; a mismatching copy is refused |
| Lost redundancy with eligible repairer and replacement places | Fresh indexes, then a new generation when appropriate, restore the required redundancy within existing budgets |
| Author deletion or moderator removal validated by the participating nodes | No subsequent visible restoration on those nodes; delayed repair results cannot republish deleted content |

Advance the simulated clock through current rules. Silence requires
failures through different places at least twelve hours apart; repair adds a randomized wait
up to six hours. Use current constants and bounded eventual assertions, not wall-clock sleeps.
Preserve forwarding/place caps and generation limits.

Assert that faults never produce deletion/removal records. Capture operations to verify
fallback and repair occurred, rather than accepting a successful read alone.

The in-flight deletion case depends on #34 and its proposed RFC-0001 contract, which is not
yet accepted. Implement that case after the contract is settled; other cases can proceed
independently. This RFC does not resolve deletion propagation or offline freshness.

## Alternatives

| Option | Assessment |
| --- | --- |
| Simulation alone | Useful at scale, but cannot replace exercising production paths |
| Live-network tests only | Useful operational evidence, but difficult to reproduce failures |
| Deterministic core fault matrix | Recommended for repeatable regression checks; retain separate field testing |

## Consequences and validation

No protocol/storage change is proposed. Runtime defects need focused fixes and failing
regressions; never weaken or skip a case to claim success.
Run focused core tests and preserve existing fallback, regeneration and local-copy coverage.
Document results as fixture guarantees, not a universal failure-tolerance percentage.

Start with `core/crates/hoi-core/src/tests/forward.rs`, `core/crates/hoi-core/src/tests/open.rs`,
`core/crates/hoi-core/src/scatter.rs` and `core/crates/hoi-core/src/keep.rs`.
Long-running evidence remains #8; sandbox visualization remains #33.
