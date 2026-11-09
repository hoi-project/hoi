# RFC-0001: Persist deletion work and prevent replay after acknowledgement

Issue: [#34](https://github.com/hoi-project/hoi/issues/34)

Status: accepted

## Summary

Keep an encrypted, durable deletion journal on the requesting device. Retry unfinished
piece drops, preserve their individual outcomes, and make storage holders retain a small
marker after an authorized drop. Deletion must also cancel queued and in-flight repair.

This proposes **bounded, eventual deletion**, not proof of network-wide erasure. The strict
reconnect requirement in #34 needs a decision: an uninformed holder cannot discover a deletion
by itself. The limitation and approval choices are stated below; this RFC must not be treated
as resolving that requirement silently.

## Motivation

`delete_own` already publishes a signed author deletion. Members remove the content and
retain a skeleton through its expiry. Piece drops already require the creator's capability
or the separate moderator key. Those authorization rules remain unchanged.

Today, `keep.rs` submits author drops without retaining a complete retry job; `Job::Dropped`
discards the result. `scatter.rs` merges dropped and unknown responses into a presence result.
The piece store removes bytes without a retained deletion marker. A request submitted once
therefore cannot support reliable progress or recovery after interruption.

## Proposal

### 1. Persist before dispatch or irreversible cleanup

Create one job per accepted author-deletion record. Repeated attempts to delete the same
target reuse its active job. Persist the following with the existing encrypted client state:

| Data | Purpose |
| --- | --- |
| Deletion record, target ID, community and target expiry | Retry publication and keep the target suppressed |
| Known object generations, holders, locators and expiry days | Enumerate the pieces to remove |
| Required per-locator capabilities | Retry without retaining the deleted post or copying a device-wide secret |
| Per-target outcome, last attempt and next retry | Resume and report observed progress |
| Coverage gaps | Distinguish unavailable capabilities or unknown generations from completed work |

Capture targets before discarding manifests; add late validated generations and completed
placements to the job without reviving the post. Commit the job, signed deletion and local
suppression together before releasing network messages or reporting durable success.
A failed write leaves retryable work; a crash cannot leave a reported-successful deletion
without its job. The browser must wait for its host storage acknowledgement as well.

Use bounded scheduling and the existing routing/privacy rules. Retry transient failures
with capped backoff; reconnect may advance a retry. Queue exhaustion rejects new work
explicitly, never evicts unfinished jobs. A missing capability remains a visible gap.

The journal's signed deletion continues through existing public sync or private MLS delivery
until expiry. Holder results do not prove that other members received that record. Home-node
forwarding is submission only: retain the originating job until actual per-target results
return. Durable transfer of responsibility to another device is outside this RFC.

### 2. Report evidence, not a single “done” flag

Expose job ID, local-removal state, record-submission state, counts by target outcome, coverage
gaps and whether deletion work still requires local state.

| Target outcome | Meaning |
| --- | --- |
| Queued / submitted | Saved locally / a request was sent |
| Acknowledged | A capable holder replied after committing removal and its marker |
| Unknown / refused / unreachable | Unresolved; keep the reason and retry where appropriate |
| Expired | Its permitted storage lifetime ended; erasure was not observed |

Persist results before exposing them as durable. Accept replies only against the matching
outstanding request and its one-time reply key. Duplicate or delayed replies must not inflate
counts or move an expired target back to pending. These are local observations, not portable
signed receipts or proof that a malicious holder erased its disk.

An “all known targets acknowledged” result remains distinct from complete inventory or global
propagation. Keep deletion records and suppression until their validity horizon even if every
known holder answered. Report that local state is still needed while dissemination, retries or
coverage gaps remain; #36 must not interpret submission as permission for a safe combined wipe.

### 3. Holders retain an authorized-drop marker

For a known locator, validate the existing token or moderator signature first. In one durable
transaction, replace its piece with `{loc, expiry_day, drop_hash, drop_key_id}`. Only then send
`Dropped`. Retaining the two authorization fields allows authenticated repeated drops to return
the same result without retaining raw tokens, signatures, content or community/author IDs.

While the marker lives, `get`/`has` report absence and every `put` at that locator is refused.
An invalid drop cannot create a marker. A locator with neither a piece nor a marker returns
`Unknown`, which does not establish authority or durable absence.

Retain the marker through the original piece's inclusive UTC expiry day; remove it only when
`today > expiry_day`. Use the actual stored day, not the record's second-level expiry: current
scatter placement rounds upward. Retrying a drop never extends its marker. Reserve marker
capacity when admitting pieces; never evict unexpired markers to admit more data.

This prevents replay at a holder that processed the drop. It does not block every possible
new locator, malicious re-upload, or another holder that has not received the deletion.
Expired signed records remain invalid even after their markers are collected.

### 4. Deletion wins over repair

Check accepted deletion state before scheduling, placing, applying fetched content and
publishing a repair update. Do not wait for a busy object to become idle before suppressing it.

Cancel unsent work. Requests already in transit may still store bytes: journal their known
targets and drop them rather than publishing their result. The device that generated a repair
uses its existing capabilities. Missing capabilities remain explicit gaps, not invented authority.
Apply the same suppression rule to valid moderator removals without changing who may remove.

Remove associated indexes, caches and unreferenced media. Shared media needed by another live
post stays available through that post's own object; deleting one author's post must not erase
another author's content.

### 5. Reconnect, restoration and the knowledge boundary

On ordinary restart, load and enforce persisted deletion state before serving or repairing.
Resume unfinished jobs. During member catch-up, apply available validated deletions before
exposing their targets. If a deletion arrives before its target, keep a bounded pending record;
validate ownership against the target before applying it. An unauthenticated pending claim
must not permanently suppress another author's post. Private catch-up preserves MLS message
and commit ordering; it does not bypass membership or decrypt unavailable epochs.

A supported holder restore must not reactivate old piece payloads from a snapshot that may
predate deletions: discard those restored payloads before serving, allowing members to repair
live content normally. Preserve available markers. Direct disk rollback that bypasses this
procedure is outside the compliant-node guarantee. Restored member state must be quarantined
from serving/repair while catch-up applies available deletion records.

**Unresolved requirement:** catch-up with an uninformed peer is not proof of freshness. An
offline holder can serve an undeleted local piece before learning its drop. No fixed number
of peers or successful sync proves that all deletions have arrived. This proposal promises
rejection after knowledge, retries while the requester runs, and expiry as the outer validity
bound. It does not promise instantaneous rejection throughout a partition.

Before acceptance, choose either:

- Accept this bounded contract and clarify #34's strict pre-serving wording; or
- Require pre-serving freshness evidence and design its authority, leases/checkpoints and
  partition behavior in a further proposal. That requirement remains unsatisfied here.

### 6. Compatibility and migration

Propose a `drop-journal` Hello capability only for holders with durable marker storage. Keep
existing drop authentication and envelope layouts. Treat an older holder's `Dropped` as
legacy removal observed, not a marker-backed acknowledgement. Never infer the capability
from a release version; feature advertisement is still a compliant peer's claim.

Add a versioned client journal and transactional holder-marker table. Migrate existing data
without claiming that historical drops were acknowledged or that lost capabilities can be
recovered. Refuse downgrade where it would ignore live markers/jobs. Update core DTOs,
bindings, conformance vectors, PROTOCOL §21 and SCATTER §5.11 with implementation.

## Alternatives

| Option | Consequence |
| --- | --- |
| Durable retries only | Smallest change, but a holder can accept replay after acknowledging removal |
| Journal plus holder markers — recommended | Bounded storage, no new crypto or global service; does not establish global freshness |
| Shared deletion ledger or freshness authority | Could gate serving on checkpoints, but adds coordination, metadata and availability costs requiring a separate design |

## Consequences

Clients retain deletion capabilities and routing metadata until no longer needed, protected as
existing key-bearing state. Holders retain opaque locator/authentication metadata until expiry;
the existing drop-key correlation remains. No community directory is added. A small journal
has durability and storage costs; an unavailable requester delays retries, and supported
snapshot restoration sacrifices cached availability. None of this forces recipients or malicious operators
to erase copies, or proves hardware storage erasure.

Bulk discovery/recovery (#35), cleanup UI (#36) and broader availability tests (#38) remain
separate. Accepting this RFC would authorize later implementation, not close #34.

## Validation required in implementation

- Crash before/after each persistence and acknowledgement boundary, including browser storage
  failure and home-node forwarding; restart resumes without lost targets or false success.
- Valid, forged, repeated, unknown and refused drops; stale replies; full journals/marker budgets.
- Re-put at a dropped locator before expiry is refused; test the inclusive day boundary and
  rejection of expired signed content after marker collection.
- Delete with a holder offline, reconnect and deliver its drop; distinguish the pre-knowledge
  limitation from the post-acknowledgement guarantee. Test supported snapshot restoration.
- Delete during fetch, placement and update publication; no visible resurrection, and already
  placed orphan pieces enter cleanup. Preserve other authors and shared media.
- Open previous-version storage, migrate, restart and reject unsafe downgrade. Verify legacy
  holder status never becomes a durable-marker acknowledgement.

Use deterministic core/persistence tests, piece-store tests and protocol vectors. Relevant
starting points: `hoi-core/src/{keep,scatter,home_scatter,node}.rs`,
`hoi-store/src/pieces.rs`, `hoi-core/src/tests/forward.rs` and
`hoi-core/tests/persistence.rs` under `core/crates/`.
