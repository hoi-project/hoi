# RFC-0003: Separate local cleanup from resumable network deletion

Issue: [#36](https://github.com/hoi-project/hoi/issues/36)

Status: proposed

## Summary

Keep local cleanup available as an explicit choice. Add a separate network-deletion flow
that reports the core's durable progress and permits combined cleanup only when doing so
cannot strand its remaining work. Do not use identity burning as a deletion shortcut.

This depends on the proposed contracts for [#34](https://github.com/hoi-project/hoi/issues/34)
and [#35](https://github.com/hoi-project/hoi/issues/35), provisionally RFC-0001 and RFC-0002.
Neither proposal is accepted or implemented yet.

## Motivation

The account screen already calls `wipe`; locked-vault recovery separately calls `resetVault`.
Ordinary wipe retains vault encryption, while recovery reset removes the local vault and
requires setup again. Neither action coordinates with a durable network-deletion job.
Clearing credentials after merely sending a request can prevent retries.

## Proposal

### Actions and disclosure

Offer two clearly named actions: **Clear local data** and **Delete known posts and comments**
from the network, matching RFC-0002's bounded scope. Before confirming network deletion,
show the core's selected inventory and coverage gaps; do not call it “all my posts.”
Explain which downloads, drafts, credentials, private-group
keys and locally held recovery material will be lost. Exported backups and other devices
remain outside local cleanup. Identity recovery does not promise restoration of private state
or deletion capabilities.

Network deletion leaves local credentials intact. After starting it, show durable job IDs and
resume those jobs after reload; never launch a second batch merely because the screen reopened.
Keep progress text specific:

| Core evidence | User-facing meaning |
| --- | --- |
| Saved / submitted | Requests are saved / have been sent |
| Outstanding or refused targets | Deletion still needs work; show the reason |
| Acknowledged targets | Named targets acknowledged removal; other copies are not proven erased |
| Expired targets | Storage validity ended; erasure was not observed |

Closing the screen does not cancel authorized work. Failure to persist or submit a job leaves
credentials and pending state available for retry. No raw keys enter UI state.

### Cleanup readiness belongs to the core

The core reports readiness for the whole installation, with reasons when blocked and both
the confirmed inventory revision and a global deletion-state revision. It remains blocked
while any local deletion job needs credentials, retries, retained suppression or
further dissemination. Unknown capabilities are not success. Even all known holder
acknowledgements may leave record-dissemination obligations under RFC-0001.

The UI offers combined cleanup only when ready and asks for fresh confirmation; background
completion never silently wipes the installation. The cleanup command rechecks readiness and
both revisions under the same serialization boundary as job updates. A new obligation
between preview and confirmation invalidates the operation before credentials are removed.
An unavailable or older core cannot authorize combined cleanup.

Local-only cleanup remains possible, including while locked. Warn explicitly that it abandons
this installation's ability to finish pending network deletion. Do not report that work as
completed. Recovery reset must not require decrypting the vault to show this limitation.

## Alternatives

- **Wipe immediately after submission:** simpler, but can destroy the only retry authority.
- **Core-gated cleanup — recommended:** preserves resumability and truthful progress, but may
  require keeping the installation until outstanding obligations expire or finish.
- **Hand work to another device before wiping:** potentially faster; requires a separately
  specified durable handoff protocol and is outside this RFC.

## Consequences and compatibility

Reuse existing browser/desktop storage boundaries and confirmations. Add status and guarded
cleanup APIs consistently across Rust, bindings and `packages/core-api`; the UI owns no retry
queue. Any persisted workflow metadata needs a versioned migration. This adds no network
protocol, cryptographic primitive or standalone mobile app. Local deletion is not a guarantee
of physical storage erasure.

## Validation

Test mixed-author local cleanup emitting no deletion requests; failure before persistence;
offline submission and restart; incomplete coverage; stale readiness revisions; new jobs racing
cleanup; and locked reset without network-success claims. Verify browser and desktop managed
storage boundaries and retained encryption after ordinary wipe. Use deterministic status
fixtures, then real-core restart/resume integration once the dependencies exist.

Start: `apps/web/src/routes/You.svelte`, `apps/web/src/lib/ui/VaultRecovery.svelte`,
`apps/web/src/lib/core/durable-settle.ts`, `core/crates/hoi-core/src/node.rs`,
`apps/web/src/tests/vault-storage.test.ts` and `apps/desktop/src-tauri/src/lib.rs`.
