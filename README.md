# hoi

[![CI](https://github.com/hoi-project/hoi/actions/workflows/ci.yml/badge.svg)](https://github.com/hoi-project/hoi/actions/workflows/ci.yml)
[![Security](https://github.com/hoi-project/hoi/actions/workflows/security.yml/badge.svg)](https://github.com/hoi-project/hoi/actions/workflows/security.yml)
[![Quality gate](https://sonarcloud.io/api/project_badges/measure?project=hoi-project_hoi&metric=alert_status)](https://sonarcloud.io/summary/new_code?id=hoi-project_hoi)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/hoi-project/hoi/badge)](https://scorecard.dev/viewer/?uri=github.com/hoi-project/hoi)

A forum with community-held keys. Public communities are readable by anyone; private
communities encrypt posts and media on members' devices. Clients verify signed records
and community rules. Posts expire according to community settings, although recipients
can keep copies.

**Preview software:** compatibility may change and data may be lost.

## Get started

Needs Node 22, pnpm 10 and the [Rust/WASM prerequisites](docs/DEVELOPMENT.md).

```sh
pnpm install --frozen-lockfile
pnpm core:wasm
pnpm dev:site   # website
pnpm dev:web    # browser app (run in another terminal)
```

`pnpm build` produces the static site and app in `dist/`. For installation and hosting,
see the [documentation](docs/README.md).

`pnpm build:all` builds everything this computer can build into one folder,
`builds/hoi-<version>/` (not in Git), a folder per tool with a README and SHA256SUMS:
`server/` (the Linux server bundle and hoi-node), `node-setup/` (hoi Node Setup, including a
portable app that needs no installing), `app/` (the hoi app) and `website/`. Use
`--only server,node-setup` for some of them; see [development](docs/DEVELOPMENT.md#build-everything).

For a Linux VPS, run `pnpm build:all --only server,node-setup` and start hoi Node Setup from
`builds/hoi-<version>/node-setup/`: it uploads the hoi-node it finds in `../server/`, so no
published release is needed. Without the wizard, `pnpm build:server` builds an installation
bundle and `pnpm deploy:server user@host` uploads and installs it over SSH. See the
[server guide](deploy/server/README.md#build-a-bundle-and-install-over-ssh) for requirements
and manual uploads.

## Repository

- `apps/`: website, web app, desktop companion and mobile libraries.
- `core/`: shared Rust core, headless node and protocol test vectors.
- `packages/` and `design/tokens/`: shared UI API, build tools and design tokens.
- `registry/`, `relay/` and `deploy/`: optional services and deployment guides.
- `scripts/` and `sandbox/`: build, release and test tooling.
- `docs/`: development, CI and release guides; `docs/spec/`: protocol specification;
  `rfcs/`: decisions.

The website and browser app are static files. Nodes, registries and relays are run
independently; builds include no default service endpoints.

## Contribute

Read [CONTRIBUTING.md](CONTRIBUTING.md) and the [code of conduct](CODE_OF_CONDUCT.md). Start checks
with `pnpm check:quick`. Coding agents follow [AGENTS.md](AGENTS.md). The protocol specification,
design notes and decision records are in [docs/spec/](docs/spec/README.md); how decisions are made
is in [GOVERNANCE.md](GOVERNANCE.md).
Pull requests run CI, security scanning and SonarCloud automatically ([CI](docs/CI.md));
releases are started deliberately (**Release: version**, also manually from a pushed branch). Patch releases use quick checks, minor releases use
a limited test set, and major releases run the full suites. Prepare versions with `pnpm release:version`;
see [releases](docs/RELEASES.md) and [CI](docs/CI.md).

Report vulnerabilities privately using [SECURITY.md](SECURITY.md).

## License

Software: [AGPL-3.0-only](LICENSE). Documentation: [CC BY-SA 4.0](LICENSE-DOCS.md).
Third-party code and assets retain their own licenses.
