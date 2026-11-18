# RFC-0010: Advertise all active services in the edition 0.4 hello

Issue: #144

Status: proposed

## Summary

A sync hello may carry up to 32 feature names in `fx`. Each name remains limited to 16 bytes. A sender advertises every service it currently offers on that connection.

## Motivation

A node can participate in the DHT, serve blobs, relay browser signalling and hold pieces durably while offering paired-home placement. These services together can exceed the current sixteen-name budget. Dropping a name makes a usable service invisible to the peer. In particular, the set of unrelated services a node offers must not determine which paired-home protocol another device can discover.

## Proposal

A receiver accepts up to 32 names and rejects a hello with more. The names retain their existing meanings, permission checks and capacity checks. Peers can ignore individual names they do not implement. A peer must advertise `home-scatter-v2` for recoverable placement. A `home-scatter` advertisement alone does not establish v2 support.

An edition 0.4 preview with a sixteen-name decoder can reject a newer hello. Updating both devices restores the connection. There is no change to stored data or identity keys. Sending additional active names discloses which services the connected peer can use.

## Verification

Exercise a fully configured node's encoded hello and verify that its peer receives every eligible service. Publish conformance examples for lists of 32 and 33 names. Retain the per-name byte bound and the v2-only negotiation regression.
