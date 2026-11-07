# Contributing

Bug reports and focused pull requests are welcome. Describe the problem, reproduction
steps and version tested. Discuss substantial behavior or API changes in an issue first.
Report vulnerabilities privately using [SECURITY.md](SECURITY.md). Everyone taking part
follows the [code of conduct](CODE_OF_CONDUCT.md).

## How to contribute

1. **Find or open an issue.** Search issues and pull requests first. Issues labelled
   `good first issue` or `help wanted` are ready to pick up; `needs-decision` ones are not.
2. **Claim it.** Comment that you are working on it, and release the claim if you stop.
3. **Branch and change.** One issue per pull request, named `<type>/<issue>-<slug>`. Keep the
   change narrow and add a test that fails without it.
4. **Check.** Run `pnpm check:quick` and the tests for the code you changed.
5. **Open the pull request.** Use [Conventional Commits](https://www.conventionalcommits.org/)
   for the title (`fix(core): …`, `feat(web): …`), link the issue with `Fixes #N`, and fill in
   the template truthfully, including the checks you could not run.

Reviews aim to answer within a week. Please answer review feedback within 7 days; a pull request
waiting on its author for 30 days may be closed and can be reopened at any time.

**Using a coding agent?** You are accountable for what it opens, and you are its author:
commits and pull requests carry no agent attribution. It must follow [AGENTS.md](AGENTS.md);
the topic rules are in [.claude/rules/](.claude/rules/).

## Development

Follow [development setup](docs/DEVELOPMENT.md). Keep changes focused, explain behavior
changes and include relevant validation in the pull request.

- Run `pnpm check:quick` and the tests for the code you changed.
- Format Rust with `cargo fmt --all` from `core/`; use `cargo clippy` for touched crates.
- Cover protocol changes with core tests and conformance vectors, and update the
  [specification](docs/spec/README.md). Protocol, storage, cryptography and architecture changes
  start as an [RFC](rfcs/README.md); see [GOVERNANCE.md](GOVERNANCE.md).
- Keep the TypeScript API in `packages/core-api` aligned with Rust DTOs.
- Edit `design/tokens/tokens.json` and run `pnpm tokens` for shared styling changes.
- Explain new dependencies and preserve their licenses and supply-chain records. Ask in the
  issue before adding one.

## Project conventions

The website and app remain static; nodes, registries and relays are optional services.
Keep keys and validation in the core. Do not log keys, record bodies or peer addresses.
Bundle assets locally, support reduced motion, and use short, clear interface copy.

Every pull request runs **CI** and **Security** for what it changes: quick checks always; web
types and tests for app changes; Rust fmt, clippy, tests and SonarCloud for core changes; CodeQL,
dependency audits and workflow linting where they apply. A documentation change finishes in about
a minute. Runs use a read-only token; a maintainer approves first-time contributors' runs. CodeRabbit comments on pull
requests; treat its findings like any review suggestion. The full platform suites run weekly
and before major releases; run the relevant ones locally when you touch that area. See
[CI](docs/CI.md).

## License

Contributions are accepted under the project's licenses: code under
[AGPL-3.0-only](LICENSE), documentation under [CC BY-SA 4.0](LICENSE-DOCS.md).
