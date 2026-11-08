# Releases

A release uses a tag matching all package versions, for example `v1.0.0-preview.1`
or `v1.0.0`. `release.json` controls the preview/stable channel; compatibility
changes can cause data loss.

## Configure publication

`publication.json` is the shared build input for repository, site and static-download URLs,
public contact, the minisign manifest key and the separate Tauri updater public key. Leave
it blank for development: update checks then report that signed releases are unavailable.
Only public keys belong in this file. There are two gates:

```sh
node scripts/publication.mjs --site        # website: repository and site
HOI_PUBLICATION=1 node scripts/publication.mjs   # release: every field
```

The Pages workflow uses the website gate and builds with `HOI_PUBLICATION=site`, so a preview
website deploys before any signing key exists. If it sets `downloads`, it must set `manifestKey`
too. The release workflow and local uploads require every field. Desktop publication builds also require
`TAURI_SIGNING_PRIVATE_KEY` (and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` when encrypted).
The private updater key must match `updaterKey`; the manifest signing key must match
`manifestKey`. Keep both in local secret storage or CI secrets. OS signing/notarisation
uses the separate Apple/Windows credentials described below.

The signed `manifest.json` embeds the release notice and updater metadata, lists artifact
SHA-256 hashes, and has a detached `manifest.json.minisig`. Consumers verify the exact
metadata bytes against the bundled key before using download URLs or version instructions.
Desktop updates also verify the updater package signature and its signed version; checking,
downloading and installing are separate user actions. Installed and observed version checks
reject rollbacks. Static hosting is sufficient; no update backend is required.

After producing a candidate folder, verify it without installing or publishing:

```sh
node scripts/release-check.mjs dist/release/v1.0.0-preview.1
```

This checks the manifest signature, every listed file's size/hash, and every updater
artifact's platform and signed version, and requires an updater package for each desktop
platform included in the candidate. Node-only candidates remain supported. Upload workflows
run this check before uploading the signed manifest.
Host the manifest and signature together with CORS for the app/site when origins differ;
serve metadata without a long cache lifetime. Keep old artifacts while clients finish updates.

Shell/PowerShell installers fail closed when the trusted key or minisign verifier is missing.
For a locally built, unsigned **local folder only**, use `--allow-unsigned-local` or
`-AllowUnsignedLocal`; remote unsigned installation has no bypass. An explicitly selected
local hoi-node binary remains available for development and server reconfiguration.
The PWA uses HTTPS plus its existing complete-cache service-worker update path; it does not
claim cryptographic verification of browser assets. A failed download keeps the cached app.

## Prepare a version

```sh
pnpm release:version                              # show and check the current version
pnpm release:version 1.0.0-preview.1 --dry-run      # preview the edits
pnpm release:version 1.0.0-preview.1               # prepare the first preview
pnpm release:version --check 1.0.0-preview.1       # verify all manifests and lockfiles
```

The helper updates the Rust workspace, the desktop and node setup apps' Cargo/Tauri
manifests, versioned first-party JS packages and all three Rust lockfiles. It does not run a build, commit,
tag, push or deploy. Dependency versions stay unchanged; the pnpm lockfile uses
workspace links and needs no product-version edit. Mobile wrappers use the Rust
workspace version and currently have no separate application version.

Use `1.0.0-preview.2`, `1.0.0-preview.3`, optionally `1.0.0-rc.1`, then `1.0.0`.
Prereleases and `0.x` versions default to the preview channel; other versions default
to stable. `--channel preview` can explicitly keep a numeric release in preview.
A prerelease cannot use the stable channel. Channel changes refresh the notice;
customize it with `--notice-title` and `--notice-message`. Any release metadata change
increments `revision` once; repeating an identical command makes no changes.

Protocol and index versions stay unchanged unless explicitly requested:

```sh
pnpm release:version 1.0.0-preview.2 --protocol 0.4 --index 2
# Only if this build really supports both formats:
pnpm release:version 1.0.0-preview.2 --protocol 0.4 --supported-protocols 0.3,0.4
```

A changed protocol defaults to supporting only itself. List older protocols explicitly
only after testing them. `--index` sets a positive registry index edition; it is
independent of the product version. `--help` lists every option.

Review the edits and run `pnpm check:quick`, then commit and push to `main`:

```sh
git add -u
git commit -m "chore(release): v1.0.0-preview.1"
git push origin HEAD
```

Commit only the intended release changes. With automatic releases enabled, a changed
product version pushed or merged into `main` starts **Release: version**. It validates
all versions, creates the matching tag on the pushed commit, then starts **Draft
release** to build and upload the artifacts. There is no separate local tagging step.
A local commit must be pushed before GitHub can see it.

Ordinary pushes with the same product version are ignored. The initial creation of
`main` does not release an imported repository automatically; use a manual run for
that first release. Use a new version for each published release. Existing tags are
reused by **Release: version** only when they point to the exact selected commit.

## GitHub

For a manual release, push your commit, then open **Actions → Release: version → Run
workflow** and select its branch. The version is read from that commit; no tag input
is needed. Manual release runs work even when the automatic switches are unset.
The workflow must be present on the default branch for GitHub to offer the manual
entry point. Follow the subsequent **Release** run for validation/build progress;
completion of **Release: version** only means the build was started.

For automatic releases, set the repository variable `HOI_RELEASES_ENABLED=true` in
hoi-project/hoi, then push a version change to `main`. Without the switch (and in any other
repository), automatic release jobs are skipped. Enabling them does not process old
commits retroactively; use a manual run for an already-pushed version.

The existing tag workflow is also available: create and push `v1.0.0-preview.1`
yourself, or run **Release** with an existing tag to retry that tagged source.
By default **Release** builds the commit the tag points to, even when you select `main`.
For a failed, unpublished release, merge the fix to `main` at the same version. Open
**Actions → Release → Run workflow**, select `main`, enter the existing tag and check
**Retry unpublished**. The selected commit must descend from the lightweight tag. The
workflow validates that commit and its version before advancing the tag and building the draft.
If the failed build left a draft, discard it while keeping the tag before retrying. That keeps
assets from different commits out of the same draft. Published releases require a fresh version.

To enable this retry option, turn on [immutable releases](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases)
and install a dedicated GitHub App in the repository with Contents write and Administration read.
In the `release` environment, set `HOI_RELEASE_TAG_APP_CLIENT_ID` as a variable and
`HOI_RELEASE_TAG_APP_PRIVATE_KEY` as a secret. Put tag update and deletion protection in separate
rulesets and grant the app a bypass for the update rule only. The workflow uses an expiring
installation token for the tag advance. [GitHub ruleset configuration](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/creating-rulesets-for-a-repository)
Automatic tag pushes obey the same switches. The **Release: version** workflow uses
the built-in GitHub token to create its tag and explicitly dispatch the build:
[token-created tags do not trigger push workflows](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow).

Validation is selected automatically from the tag: patch = quick, minor = limited,
major = full. Prereleases inherit their numeric version's tier. See [CI](CI.md).
Validation runs on the pinned source commit and must pass before artifact creation.

The workflow builds hoi-node archives, desktop installers and the node setup app
(`apps/setup`, product name "hoi Node Setup", so its files start `hoi Node Setup_`, which a
GitHub release turns into `hoi.Node.Setup_`, and `hoi-node-setup_` from local builds;
`product: "setup"` in the manifest marks them all). For
manual runs, turn off `desktop` to build only hoi-node; it leaves out the setup app too. Builds and tests are separate: even a patch needs
to compile the requested binaries.

Required secrets, in the `release` environment (deployments from `v*` tags and `main` only):

- `MINISIGN_SECRET_KEY` (base64 secret-key file) and `MINISIGN_PASSWORD`.
- For desktop updates: `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
  A key from `tauri signer generate` is a minisign key, base64-wrapped, so it serves as
  `MINISIGN_SECRET_KEY` too. Before any build, the workflow signs a test file with both keys
  and checks the result against `manifestKey` and `updaterKey`.
- For signed desktop installers: `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`,
  `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`,
  `WINDOWS_CERTIFICATE` (base64 PFX) and `WINDOWS_CERTIFICATE_PASSWORD`.

`HOI_ALLOW_UNSIGNED_INSTALLERS=true` deliberately allows unsigned installers, with
a warning in the draft notes. The minisign manifest remains mandatory.

Artifacts, SHA-256 manifest and signature are uploaded to a **draft** release.
The preview channel sets GitHub's pre-release flag automatically, including numeric
`0.x` previews. Both manual and automatic runs validate the same tagged source and
require the same signing secrets.
Publishing the draft is a manual action. It starts **Website**, which copies the newest
published release's `manifest.json` and `manifest.json.minisig` into the site's `/releases/`
(the `downloads` address in `publication.json`) after checking the signature. The files stay
on the GitHub release; only `HOI_RELEASE_BASE_URL` points them elsewhere. Until a release
is published, the site serves no signed list and the Download page says so.

## Local builds

```sh
node scripts/release-local.mjs --check-version 0.1.0
node scripts/release-local.mjs 0.1.0 --no-desktop --no-setup
```

Local builds use the same patch/minor/major tiers. Major versions also run the local
browser suites; platform-specific checks still require their respective platforms.
`--quick` selects only quick checks for a local build; `--no-audit` omits online audits.
Neither option replaces full major-release CI validation. `--long` is accepted only
for major versions.

Output goes to `dist/release/vX.Y.Z/`, flat, as a GitHub release and `install.sh --from`
take it. To build every part into a folder per tool for trying and posting, without checks
or signing, use `pnpm build:all` ([development](DEVELOPMENT.md#build-everything)). Signing uses `--minisign-key PATH`,
`MINISIGN_KEY` or `~/.minisign/minisign.key`. Without a key a local manifest is unsigned.

**Mutating options:** `--bump` changes versions, updates lockfiles and creates a commit;
`--tag` creates an annotated tag; `--push` pushes it; `--upload` creates or updates a
remote draft. Use those options only when intending those actions.
`--allow-dirty` is for local trial builds and cannot be combined with tagging/uploading.
See the script's `--help` for target, Docker and signing options.
Use `pnpm release:version` instead of `--bump` when preparing a release for GitHub
Actions without a local build or automatic commit.

Set `HOI_RELEASE_BASE_URL` when release files live somewhere other than the repository's
release downloads. Installers can also use a local release folder.

## Release notices and compatibility

The public release destination is `hoi-project/hoi`. Local release tooling uses it even when
building in a private checkout. For a fork, set `publication.json.repository` and
`GITHUB_REPOSITORY=owner/repository` consistently before building or uploading. A Git
remote is not a publication boundary.

The checked-in download manifest is empty until the first signed release. Publish
its minisign public key on the Verify page and alongside the release before offering
downloads. Apps verify signed release metadata using the configured trust root. Desktop updates
also verify the selected artifact and updater signature; browser app replacement still
uses HTTPS and the service worker. Complete native-platform acceptance before publication.

`release.json` controls the preview/stable notice, protocol family and registry index
edition. Increase `revision` when changing notices; rebuild native, WASM and UI artifacts
together when changing compatibility fields. Do not claim support for an untested format.

Changing a notice does not reset device data, migrate storage or re-sign existing
community proofs. Keep backups before any incompatible upgrade. The Rust storage format
guard is separate from this manifest.
