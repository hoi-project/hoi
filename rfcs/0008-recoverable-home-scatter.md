# RFC-0008: Recover home-delegated scatter requests after restart

Issue: #117

Status: proposed. This file records the approved implementation scope; it does not assert
repository acceptance before the RFC pull request is merged.

## Summary

Eligible public-community objects may be placed through an authenticated paired home using
a durable, content-bound request identity. The origin retains correlation to its records;
the home retains every possibly dispatched generation and its deletion capability before
network egress. Retrying a request recovers the same result. Private communities continue
through MLS and mailboxes.

## Motivation

Legacy home placement cannot recover a lost reply after the phone restarts. It is therefore
disabled: enabling it without preserving locator-level deletion authority could strand pieces
and make local cleanup unsafe.

## Proposal

Advertise `home-scatter-v2` separately from legacy `home-scatter`. Add journalled store
and result packets to the existing authenticated home link. A request identity binds the
community, content and scatter level; its first accepted expiry is retained. Origin entries
also name the carried record IDs. Home entries are namespaced by the digest of the durable
pairing credential, never a connection-local peer name or message ID. A duplicate with altered
parameters is rejected. Only public communities shared by the paired device are eligible.

A separate versioned local journal fails closed if unreadable. Every generation manifest,
per-locator deletion token, holder key and request result is persisted before its PUT or
result leaves. A rejected storage transaction releases no network messages. Browsers retain
their existing host-storage barrier before egress. Pending origin entries survive a restart;
reconstructed outgoing content reuses the same request and result, rather than generating
another object. Successful results are verified against the origin's content and parameters
before pointer signing. Results are durable before delivery acknowledgement.

If a home restarts during an uncertain placement it returns a fallback result and the
recorded partial generation capability. It does not initiate another generation under the
same request. The origin retains the capability as a cleanup target even when delivering the
whole record. A lost successful result is returned unchanged on retry. Pending requests
continue to prevent author deletion/local cleanup until the result or bounded expiry makes
authority recoverable; no timeout claims erasure.

Admission is bounded by entry count and aggregate content size. A full journal rejects new
work instead of evicting live capabilities. Entries remain through the original inclusive
holder expiry day, at most the existing 400-day piece lifetime, and are then pruned. The
journal stores no raw pairing credential or global author drop secret. Once a deletion has
atomically captured locator-level retry tokens, clear originating content and decryption
keys. An authenticated `JournalRelease` clears the home result capability and retains its
terminal deduplication tombstone; retry releases while paired. Legacy reception is
retained, but new initiation requires the new feature. Non-author or local-leave cleanup
never initiates a legacy whole-capability Drop to the currently paired operator; it retains
local authority and uses sealed holder requests.

Durable deletion targets retain the original pairing digest alongside each locator/token
known in that home receipt, never alongside newly appended locators from other devices.
After a restart, the origin can request up to 32 current holder keys with `CleanupKeys`;
`CleanupKeyList` returns endpoint-signed X25519 keys and optional signed durable-drop proofs,
without uptime or first-hand sighting claims. The origin independently verifies both signatures.
It then seals normal per-holder Drop envelopes and receives the holder's encrypted replies
through the authenticated home. The home permits this forwarding only for authenticated v2
clients and Drop requests. Home acceptance and key lookup never count as holder acknowledgement.
The original home already knows the delegated generation, so this route may carry all its
cleanup envelopes; ordinary untrusted forwarder place caps remain unchanged. Requests and queued
sealed envelopes are scoped to the original pairing, including after re-pairing. Snapshot restore removes these archived routing scopes while
preserving deletion tokens for ordinary routes. Unknown or
expired keys and unavailable routes retain queued work; they do not invent successful deletion.

A full member snapshot restore never resumes archived requests through a new operator. It
drops serving-placement receipts for other clients and retains known origin capabilities as
deletion inventory. An unanswered archived request becomes explicit unknown coverage, with
its original holder expiry horizon, rather than another placement or a claim of erasure.
Outgoing requests, including already queued packets, are scoped to the current pairing.

## Alternatives

Keeping whole-record delivery is safe but prevents phones from using a home's available
piece network. Persisting only a request ID loses partial-placement authority. Repeating an
uncertain placement with fresh randomness strands earlier locators, so it is rejected.

## Consequences

This changes preview link messages and adds a versioned local authority journal. Older
clients retain the whole-record fallback; client storage format 6 prevents older binaries
from silently ignoring pending authority. Format-5 stores migrate with an empty journal.
The home necessarily sees the sender, community and plaintext of public objects. Holders
continue to receive sealed pieces, locators and expiry rather than authors or communities.
Receipts report trusted-home delivery progress; they do not prove anonymity or global erasure.
Pending delivery polls and replies are scoped to the original pairing. Replacement pairing,
unpairing and archive restore discard queued/in-flight polls and mark unfinished reports
unknown; ordinary reconnect and reopen with the same pairing retain them.
Origin and home must retain bounded metadata until its original holder expiry boundary.

Acceptance includes lost replies, restarts on both sides, duplicate/conflicting requests,
failed persistence, partial placement, post uniqueness, deletion after recovery, and a real
browser/holder run over WebRTC.
