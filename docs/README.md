# Documentation

- [Specification](spec/README.md): the protocol, scattered delivery and the technical design,
  which code cites as `PROTOCOL.md §N`. Decisions are [RFCs](../rfcs/README.md).
- [Development](DEVELOPMENT.md): prerequisites, local builds and focused checks.
- [CI](CI.md): opt-in workflows and release test tiers.
- [Releases](RELEASES.md): versioning, signed artifacts and release notices.
- [Static hosting](../deploy/static/README.md), [installation](../deploy/install/README.md),
  [home nodes](../deploy/home/README.md) and [server nodes](../deploy/server/README.md).
- [Node setup app](../apps/setup/README.md): installs and manages a public node on a server over SSH, with a wizard.
- Optional [registry](../registry/README.md) and [encrypted relay](../relay/README.md).
- [Security policy](../SECURITY.md), [contribution guide](../CONTRIBUTING.md),
  [code of conduct](../CODE_OF_CONDUCT.md), [governance](../GOVERNANCE.md) and the rules for
  [coding agents](../AGENTS.md).
- The website's [wiki](../apps/site/wiki/README.md): the protocol and every program in detail,
  a network playground and the software bill of materials, published at `/wiki/`.

Component READMEs describe the code that ships. The Rust types and tests in `core/`,
conformance vectors in `core/vectors/`, and TypeScript API in `packages/core-api/` describe
the current implementation. Preview releases may change compatibility.
