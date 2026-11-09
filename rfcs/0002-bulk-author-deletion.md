# RFC-0002: Delete an explicit inventory of the author's posts

Issue: [#35](https://github.com/hoi-project/hoi/issues/35)

Status: accepted

## Summary

Add bulk deletion of known posts and comments, using existing author authorization. Show the
target inventory and gaps before starting; a local scan cannot prove complete publication
history. Propagation depends on #34. Its proposed RFC-0001 lifecycle
is not yet accepted.

## Motivation

`delete_own` handles one post or comment. Identity backups restore identity material, not a
complete publication history, private-group state or every generation's piece-deletion
capabilities. A bulk loop over cached records would otherwise overstate what was deleted.

## Proposal

### Inventory and consent

Preview validated local records after ordinary synchronization. Select live posts/comments whose resolved author is
the current person. Use existing identity/delegation/rotation validation; matching a display
name or merely possessing an old signing key is insufficient.

Return a snapshot ID, target IDs by community, count, available piece-drop
capabilities and gaps: unavailable private-group state, absent
records, missing generation capabilities and sources that have not synchronized. Unknown
history is a separate flag, not a guessed count. Pointer-only entries remain unresolved until
the core has the record information required by ordinary author deletion.

Confirmation freezes the list. New or newly discovered posts require another preview
and confirmation; a running batch never silently expands. Call the action “Delete known posts
and comments.” Whether to support a verifiably complete publication inventory later remains
a separate decision; this RFC does not establish that guarantee.

### Authorization and execution

Persist the snapshot, per-target job references and unresolved reasons in encrypted
client storage. Reusing its snapshot ID resumes the batch without signing duplicate requests.
Recheck authorization and expiry before creating each deletion through the existing core
validation path. Changed permissions or missing data produce a per-target failure.

Each post/comment receives its own signed deletion. Edits are not standalone deletion targets:
their displayed content follows the parent. Related edit records and media generations must
be accounted for by the deletion lifecycle; unresolved content cleanup remains a coverage gap.
Other authors' replies, posts and independently referenced media are not batch targets.

Respect existing rate windows and exemptions. Sign queued deletions when eligible, with the
current time; never bypass limits by backdating or treating a bulk request as one record.
Persist progress before reporting success. Report selected, queued, failed and already-expired
targets separately from #34's submission, acknowledgement and expiry outcomes. Cancellation
stops targets not yet accepted as signed deletions; accepted deletions and their retry jobs
continue even if nothing has been sent yet.

### Recovery contract

| Situation | Promise |
| --- | --- |
| Restart with intact encrypted storage | Resume the same inventory and jobs |
| Another authorized device | Use records and valid capabilities actually received through existing mechanisms |
| Identity backup or recovery-key restore alone | Re-establish signing authority through existing validation; rebuild available inventory and report missing data/capabilities |
| Local wipe | Inventory and jobs are lost unless preserved by an explicitly supported transfer; do not claim a network wipe completed |

A validated root rotation can preserve the person's author identity without restoring old
storage capabilities. Do not distribute a device-wide drop secret or add capabilities to
identity backups under this RFC. Safe combined cleanup belongs to #36.

## Alternatives

- **Cache-wide “delete all”:** simplest, but conceals missing history and recovery gaps.
- **Explicit snapshot — recommended:** bounded, reviewable and testable using existing authority.
- **Complete synchronized publication archive:** could improve coverage, but requires another
  retention, recovery and privacy design; defer it.

## Consequences

No new signed record kind or cryptography is introduced. Add versioned encrypted batch state
and aligned Rust/TypeScript/binding DTOs; old installations start with no historical batches.
The local inventory links authors, communities and objects, so it must not enter logs or holder
requests. Holders continue receiving opaque locators and existing authorization only. Moderator
removal remains separate; malicious operators and recipients may retain copies.

## Validation

Test mixed authors/communities, comments/edits, linked devices, root rotation, revoked
devices, identity-only recovery and unavailable private state. Test batches exceeding rate
windows, repeated confirmation, restart, cancellation, expiry and partial storage failure.
No gap may become completion, and no other author's content may be targeted.

Start in `core/crates/hoi-core/src/api.rs`, `core/crates/hoi-core/src/drop_share.rs` and
`packages/core-api/src/types.ts`; extend core tests in `tests/address.rs`, `tests/recover.rs`
and `tests/scatter.rs`. Add batch-state migration tests and retain existing deletion vectors.
