# Governance

hoi is maintained by the members of the `hoi-project` GitHub organization. Today that is one
maintainer, who decides what is merged and released.

## How changes are decided

- **Bugs and small improvements:** an issue, then a pull request (see [CONTRIBUTING.md](CONTRIBUTING.md)).
- **Protocol, storage, cryptography, architecture or governance changes:** an RFC, a pull request
  that adds `rfcs/NNNN-short-title.md` ([rfcs/](rfcs/README.md)). The process, comment periods
  and approval are [RFC-0000](rfcs/0000-decision-process.md). Implementation follows in ordinary
  pull requests and updates [docs/spec/PROTOCOL.md](docs/spec/PROTOCOL.md) where relevant.
- Accepted RFCs are not edited; a later RFC supersedes them.
- Releases follow [docs/RELEASES.md](docs/RELEASES.md). Only maintainers start release workflows.

## Becoming a maintainer

A contributor with a record of reviewed, merged work may be invited by the existing maintainers.
Maintainers who stop participating for a year may be moved to emeritus.

This document changes through a pull request that is open for comment for at least 7 days.
