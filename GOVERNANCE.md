# Governance

hoi is maintained by the members of the `hoi-project` GitHub organization. Today that is one
maintainer, who decides what is merged and released.

## Roles and responsibilities

- **Maintainers** are the members of the `hoi-project/maintainers` GitHub team. They review and
  merge pull requests, decide RFCs ([RFC-0000](rfcs/0000-decision-process.md)), start releases,
  answer security reports within the times in [SECURITY.md](SECURITY.md), and hold access to
  sensitive resources (below).
- **Contributors** are everyone who opens an issue or pull request. They follow
  [CONTRIBUTING.md](CONTRIBUTING.md) and the [code of conduct](CODE_OF_CONDUCT.md), sign off
  every commit, and answer review feedback.
- **Emeritus maintainers** are former maintainers. They hold no access.

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

## Access to sensitive resources

Only maintainers have access to release secrets, signing keys, repository settings and security
reports. Today they are:

- [@pclshm](https://github.com/pclshm)

Access is granted and removed by the existing maintainers, and a maintainer who becomes emeritus
loses it. This list is updated whenever access changes.

This document changes through a pull request that is open for comment for at least 7 days.
