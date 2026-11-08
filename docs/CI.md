# Continuous integration

Checks run by themselves in the public repository, **hoi-project/hoi**, where GitHub Actions are
free: every pull request (forks included) and every push to `main`. No variable turns them on.
Each workflow has one small **Changes** job that checks `github.repository_owner == 'hoi-project'`
and that the repository is not private; every other job needs it. So a private copy (a
maintainer's workspace, or hoi-project/hoi before it went public) and a fork's own pushes start
nothing. **Run workflow** by hand runs anywhere and checks everything: that is how CI is tested in
a private repository. A pull request from a fork runs in this repository with a read-only token and
no secrets; GitHub asks a maintainer to approve first-time contributors' runs
(**Settings -> Actions -> General**).

Two switches stay deliberate, because they publish or sign:

| Variable | Enables |
| --- | --- |
| `HOI_PAGES_ENABLED=true` | **Website** (manual): builds the static site and deploys it to Pages |
| `HOI_RELEASES_ENABLED=true` | **Release: version** on a version push to `main`, and **Release** on a tag push, in hoi-project/hoi only |

Manually running **Release: version** or **Release** is explicit opt-in and works without the
switch, including all called validation suites, signing and uploads.

## Fast, and only what changed

The **Changes** job (`scripts/ci-changes.mjs`, seconds) reads the pull request's own changes, or
the push's, and switches jobs on: Rust for `core/` and `vendor/`; Web for apps, packages,
tokens and the pnpm files; dependency audits for manifests and lockfiles; workflow linting for
`.github/`; CodeQL for the languages touched. A documentation pull request runs the quick checks
only. A job skipped this way counts as passed for required checks, which is why there are no
workflow-level `paths:` filters (they would leave required checks pending). Manual and scheduled
runs, and pushes whose base is unknown, check everything.

- Rust builds once: the tests run with `cargo llvm-cov nextest` and SonarCloud uses that coverage.
  nextest runs every test in its own process. It does not run doctests: the workspace has none,
  and the weekly suites run `cargo test`, which would.
- Tools are prebuilt (`taiki-e/install-action`, pinned versions): seconds instead of minutes of
  `cargo install`.
- The Rust cache is written only by pushes to `main` and kept on failure; pull requests restore
  it and never write, so a pull request cannot plant a cache that `main` would restore. Builds use
  `CARGO_INCREMENTAL=0` and line-table debug info.
- Superseded runs on a branch are cancelled; runs on `main` finish.
- Dependency advisories run on pull requests that change dependencies, on `main` and daily, so a
  new advisory is reported within a day without turning unrelated pull requests red.

## Workflows

| File / name | When | What |
| --- | --- | --- |
| `ci.yml` / CI | every pull request and push to `main`; by hand anywhere | **Changes**; **Quick checks** (`pnpm check:quick`, always); **Web** (site and API types, web unit tests); **Rust** (fmt, clippy `-D warnings`, `cargo llvm-cov nextest` over the workspace including the conformance vectors); **SonarCloud** (scan with the Rust coverage; `sonar-project.properties`; passes with a notice without `SONAR_TOKEN`) |
| `security.yml` / Security | pull requests, `main`, daily; by hand anywhere | **Changes**; CodeQL per changed language (Rust, JavaScript/TypeScript, Actions; `security-extended`; a private repository analyses without uploading); per Cargo graph the dependency regression gate's tests, cargo-deny and cargo-audit; cargo-vet; `pnpm audit`; GitHub dependency review (pull requests); zizmor and actionlint |
| `scorecard.yml` / Scorecard | `main`, weekly, branch-rule changes | OpenSSF Scorecard; results in the Security tab and the public badge (public repository only) |
| `weekly.yml` / Weekly | Saturdays (public repository); by hand anywhere, `linux_only` by default | the full major suites on `main` |
| `release-version.yml` / Release: version | version push to `main` (switch), or manual | validates versions, creates the tag and starts Release |
| `release.yml` / Release | tag push (switch), or manual existing tag | validates, builds, signs and uploads a draft with the channel's pre-release flag |
| `release-checks.yml` / Release: checks | called by Release | quick or minor validation on the exact tagged commit |
| `release-major.yml` / Release: major suites | called by Release for a validated major tag, and by Weekly | full suites on Linux, macOS and Windows (`linux_only` keeps them on Linux) |
| `website.yml` / Website | manual (`HOI_PAGES_ENABLED`) | website publication check (repository and site, no signing keys), quick checks, static build and Pages upload |

Required checks for `main` (**Settings -> Rules**): *CI / Changes*, *CI / Quick checks*, *CI / Web*
and *CI / Rust*. Security, SonarCloud and Scorecard report but do not block.

Pull requests are also reviewed by CodeRabbit (`.coderabbit.yaml`, against `AGENTS.md` and
`.claude/rules/`); its comments are suggestions. Dependabot opens Actions updates and npm/Cargo
security updates (`.github/dependabot.yml`).

## Test tiers

Tags must be semantic versions such as `v1.2.3`. The numeric version selects the
tier; prerelease suffixes use the same rule (`v2.0.0-rc.1` is major).

| Change | Validation |
| --- | --- |
| Patch (`X.Y.Z`, Z > 0) | Metadata, generated tokens, dependency regression checks and focused release/configuration tests (the quick checks) |
| Minor (`X.Y.0`, Y > 0) | Quick checks, site/API type checks, web unit tests and selected pure Rust library tests on Linux |
| Major (`X.0.0`, X > 0) | Minor checks plus full Rust and JavaScript suites, WASM/static builds, browser integration, platform and installer checks, dependency audits |

`v0.1.0` uses the minor tier. `v0.0.0` uses patch checks.
Minor checks do not compile WASM or run network/browser integration suites. Full
Svelte checks need generated WASM bindings and run in the major suite after its build.
Patch checks install the JavaScript dependencies but need no Rust compilation, browser, Docker
or network service.
Artifact compilation and signing still happen for every release; reducing tests does
not remove the work needed to build release binaries.

Every pull request already runs more than the minor tier; the tiers decide what a release
re-runs on its exact tag before signing.

## Major suites

The major suites also fuzz every decoder target for a minute (`core/fuzz`, nightly) and run
the WebRTC browser tests (`e2e-network`, `e2e-rtc`, `e2e-node-link`) in Firefox and WebKit as
well as Chromium. Locally, `HOI_E2E_BROWSER=firefox` or `webkit` picks the browser once
`pnpm exec playwright install firefox webkit` has fetched them.

They also build the wasm package, hoi-node and the static site twice from empty target
directories and fail on any byte that differs (`node scripts/reproduce.mjs --twice`),
and unlock a browser vault with a passkey through Chromium's virtual authenticator
(`scripts/e2e-passkey.mjs`). To check a release yourself, build its tag with
`node scripts/reproduce.mjs --only node --target <triple>` and compare with the
`hoi-node_<version>_<target>.digests.json` published beside the archive.

Required release checks must succeed before any draft or release assets are written.
In Release and Weekly, Android and iOS compile checks run only when the
[platform's build configuration](../apps/mobile/README.md#integrate) declares a valid version.
Major suites only run a commit at or before the triggering commit in its Git history. If you
dispatch Release for a major tag outside the selected branch's history, select the tag itself.
Jobs have timeouts and minimal permissions; branch runs cancel obsolete runs, `main` runs
finish. Only release-writing jobs, code-scanning uploads and Pages deployment get write
permissions; the tag creation job also has `actions: write` to dispatch Release. Every action
is pinned by commit.

For deployment setup see [static hosting](../deploy/static/README.md); for signing and
local release commands see [releases](RELEASES.md).
