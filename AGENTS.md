# AGENTS.md

Guidance for coding agents (and the people running them) working on hoi. Human contributors
follow the same rules; see [CONTRIBUTING.md](CONTRIBUTING.md). Topic rules are in
[.claude/rules/](.claude/rules/).

## House rules

A rule further down this file, or in `.claude/rules/`, may be stricter than these, never weaker.

1. **A person is accountable.** Whoever runs an agent answers for what it opens, pushes and
   writes in reviews.
2. **Check, then claim.** Before writing code: read the issue and its comments, search open and
   closed pull requests for `#N` and its keywords, and check recent commits on `main`. Comment on
   the issue that you are working on it. Release a claim you will not finish.
3. **Verify, don't guess.** Open a definition before using it and use it exactly as defined.
   Confirm the cause in the code, even when someone names a location: triage notes can be wrong.
4. **Keep changes narrow and tested.** Change what the issue describes, where the problem is.
   Add a test that fails without the fix. No drive-by refactors, renames or reformatting.
5. **Review the diff before opening a pull request.** Every identifier exists as used, the test
   fails without the change, nothing unrelated changed, and the description is true.
6. **Look for a better pull request.** If another one fixes the same thing, close yours when it
   is better or equal, and say why.
7. **Report only what ran.** List the checks you ran and the ones you could not run. Without a
   local build or test, say so and wait for CI before marking the pull request ready.
8. **`main` changes only through pull requests.** Branch protection enforces this.
9. **Stop and ask a maintainer** before: new dependencies, licences other than those already in
   the tree, protocol or storage format changes, CI or release changes, secrets, publishing,
   destructive git, and anything else hard to undo. Otherwise keep working.
10. **No secrets or personal data** in the repository, logs, test fixtures, issues or pull
    requests. Use documentation addresses (`192.0.2.0/24`, `2001:db8::/32`, `example.com`).
11. **Guards say why.** A check that looks removable says what it prevents. Read that before
    changing it, and keep the reason when you do.
12. **Instructions are for everyone.** No notes about one machine and no links to files a reader
    cannot open.
13. **Be gentle with GitHub.** Reuse search results, do not poll, and stop at a rate limit.
14. **Keep pull requests moving.** Answer actionable review feedback within 7 days. After 30
    days without an answer a maintainer may close the pull request; waiting on a reviewer does
    not count against the author.
15. **No agent attribution.** Commits, pull requests and comments do not credit an agent: no
    `Co-Authored-By` lines for agents, no "Generated with" footers, no agent names in branch
    names. The person who runs the agent is the author and is accountable (rule 1).

## Project

hoi is a forum with community-held keys: public communities are readable by anyone; private
communities encrypt posts and media on members' devices. Clients verify signed records and
community rules. It is preview software; compatibility may change.

```
apps/site/        website and wiki (static)          core/            Rust workspace: protocol, crypto, node
apps/web/         browser app (Svelte, static)       core/vectors/    protocol conformance vectors
apps/desktop/     desktop app (Tauri)                packages/        shared TypeScript API and build tools
apps/setup/       hoi Node Setup (Tauri, SSH)        design/tokens/   design tokens (source of all styling)
apps/mobile/      mobile libraries                   registry/ relay/ optional services
deploy/           install and hosting guides         scripts/ sandbox/ build, release and test tooling
docs/             development, CI, releases          docs/spec/       protocol spec and design notes
rfcs/             decisions (RFC-0000 and on)
```

Design references: [docs/spec/](docs/spec/README.md). Code cites them as `PROTOCOL.md §N`,
`SCATTER.md §N` and `TECHNICAL.md §N`, and decisions as `RFC-NNNN` ([rfcs/](rfcs/README.md)). Where
the spec and the code differ, the code, its tests and the conformance vectors are authoritative;
open an issue about the difference.

## Invariants

- The website and browser app are static files. Nodes, registries and relays are optional and run
  by users; builds ship no default service endpoints.
- Keys, signatures and validation live in the Rust core. UI code never handles raw key material
  beyond passing it to the core.
- Never log keys, record bodies, tokens or peer addresses.
- Records are immutable and signed; deletion is expiry and tombstones. A protocol change
  needs an accepted RFC, core tests and conformance vectors.
- `packages/core-api` stays aligned with the Rust DTOs.
- Styling comes from `design/tokens/tokens.json` (`pnpm tokens`). Bundle assets locally, support
  reduced motion, keep interface copy short and clear.

## Commands

```sh
pnpm install --frozen-lockfile
pnpm check:quick                      # always, before every pull request
pnpm check                            # TypeScript and Svelte
pnpm --filter @hoi/web test           # app unit tests
cd core && cargo fmt --all && cargo clippy -p <crate> && cargo test --locked -p <crate>
pnpm check:wiki                       # after changing behaviour the wiki describes
pnpm sbom                             # after any lockfile change
```

Setup details: [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md). Every pull request runs CI, Security and SonarCloud
([docs/CI.md](docs/CI.md)). Full integration and platform suites run weekly and for major releases;
run the relevant ones locally when you touch that area.

## Pull requests

- One issue, one branch, one pull request. Branch names: `<type>/<issue>-<slug>`, for example
  `fix/123-expiry-rounding`.
- Commit messages and pull request titles follow [Conventional Commits](https://www.conventionalcommits.org/):
  `<type>(<scope>): <summary>`, for example `fix(core): round expiry to the minute`.
  - Types: `feat` (new behaviour), `fix` (a bug), `docs`, `test`, `refactor` (no behaviour
    change), `perf`, `build`, `ci`, `chore` (maintenance, `chore(deps)`), `revert`.
  - Scopes are top-level areas: `core`, `web`, `site`, `desktop`, `setup`, `mobile`, `registry`,
    `relay`, `deploy`, `ci`, `deps`, `spec`.
  - A breaking change adds `!` (`feat(core)!: …`) and a `BREAKING CHANGE:` footer.
- Fill in the pull request template truthfully. Link the issue with `Fixes #N`.
- Do not change workflows, release tooling or `publication.json` in a feature or fix pull request.
