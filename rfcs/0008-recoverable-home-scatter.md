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
retained, but new initiation requires the new feature.

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
Origin and home must retain bounded metadata until its original holder expiry boundary.

Acceptance includes lost replies, restarts on both sides, duplicate/conflicting requests,
failed persistence, partial placement, post uniqueness, deletion after recovery, and a real
browser/holder run over WebRTC.
