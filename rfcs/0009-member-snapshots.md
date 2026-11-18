# RFC-0009 — Encrypted member snapshots and authorized private rejoin

Issue: [#116](https://github.com/hoi-project/hoi/issues/116)

Status: proposed

## Summary

A member can move an existing installation to an empty vault by exporting and importing
a passphrase-protected file. The receiving installation applies the deletion safeguards of
RFC-0001 and obtains new MLS membership before serving private content. The file resumes the
source device identity. The two copies should not run concurrently.
The archive requires a local device signing key and its matching root or signed device
blessing. A remotely signed browser establishes a fresh link with its signing computer.

## Archive

The established HOIK v1 envelope protects the file with Argon2id and XChaCha20-Poly1305.
The authenticated plaintext starts with the ASCII bytes `hoi-member-snapshot`, a zero byte
and version byte 1. Two little-endian u32 counts introduce first the wire records, then
the named blobs. Each record is length-prefixed. Each blob consists of a length-prefixed
UTF-8 name and length-prefixed bytes. All lengths are u32 little-endian. Readers reject
duplicate record IDs and blob names, unknown archive versions, oversized names, missing
identity storage, and trailing bytes. The envelope is at most 32 MiB and each collection
is bounded to 131072 entries. Export fails explicitly when the file would exceed the limit.

The archive contains the logical client database, including signing material and deletion
retry authority. Vault metadata, OS unlock entries and the separate blind-holder database
remain local. Verify the archive and replay it against isolated staging storage before any
replacement. After cleanup write the prepared records, state and deletion inventory in
one vault transaction. A failed write leaves the previous persisted contents intact.
Incoming public content needs peer synchronization. Cached private payloads and old MLS
epoch secrets are discarded. Local pairing and serving settings need fresh authorization.

The restored member retains known holder-removal tokens and pending author deletion jobs.
For an unfinished placement delegated to a paired home (RFC-0008), the previous home may
have written pieces before the member received the cleanup tokens. An author deletion
records the missing coverage and still removes the available copies. Import discards
work the source installation performed as an operator for other clients. Opening a new
pairing does not restart a placement sent under the former credentials.

## MLS rejoin

Before erasing MLS state, retain the fingerprint of this device’s old public leaf encryption
key per group. Restoring does not grant new private membership. The user chooses a group to
request rejoin. Its fresh MLS KeyPackage's signed BasicCredential includes an optional
`replaces` array of two 32-byte lowercase hex strings: the group ID and BLAKE3 of the old
leaf encryption key. Ordinary join requests omit this field.

A current authorized committer checks the ordinary device blessing, root and ban rules.
It must match the existing leaf's group, root, device signature key and encryption-key
fingerprint. It issues one MLS commit which removes that exact leaf and adds the signed new
KeyPackage, and sends the fresh Welcome and authenticated bootstrap. In record-governed
groups other members require the matching addition and removal in that commit. A replay
cannot replace a newer leaf. Old Welcome packages cannot be opened with the erased keys.
An authorized committer on another active device must be available. When the prior leaf is
already absent, normal membership admission can be requested again.

## Compatibility and metadata

This changes the credential JSON seen by the MLS group during an explicit rejoin. All active
members should use the preview's matching protocol implementation. The receiving committer
and group can see the replaced leaf and group; the request should be shared privately
with the committer. Rejoin makes no new global registration or service necessary.

The smaller .hoikey format and ordinary restarts are independent. Restoring active private
epochs directly could re-enable stale or deleted content. Starting a completely separate
identity would lose the signed ownership and deletion work that the member is restoring.
