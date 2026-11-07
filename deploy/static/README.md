# Static website and browser app

`pnpm build` writes `dist/`: the website at `/` and the browser app at `/app/`. It is files
only. There is no server code, server rendering, registry API, encrypted relay, content proxy
or public peer in it, and no address of one: nothing in the site or the app names a node, a
registry or a relay to start from. Application JavaScript and WASM run on the visitor's device.
The website demos stay in the browser and submit nothing. Assets and fonts are bundled.

This is the only thing the project deploys. Everything else is run by the people who use hoi:
whoever wants to post gets the home node (the desktop app, or `hoi-node` on a computer that stays
on), which is a public node too if its person chooses. The app finds communities through the nodes it
reaches: a node link someone shared, an invite, its own home node, and the other public nodes
those pass on. See the [home-node guide](../home/README.md).

## Build

```sh
pnpm install --frozen-lockfile
pnpm build
pnpm check
pnpm test
```

The build needs the repository's Rust/WASM toolchain (see the main README). `release.json`
controls preview notices and protocol compatibility.

## Host it

Any static host can serve `dist/`. Every page carries its own Content-Security-Policy in a
`<meta>` tag, so a host that sets no headers still enforces it. `dist/_headers` repeats the
policy as HTTP headers, plus the ones a `<meta>` cannot carry (`frame-ancestors`, `nosniff`,
COOP), for hosts that read that file (Netlify and similar).

**GitHub Pages.** `.github/workflows/website.yml` builds, checks and publishes `dist/`:

1. Settings → Pages → Source: **GitHub Actions**. Add a custom domain there if you have one.
2. Set the repository variable `HOI_PAGES_ENABLED=true`.
3. Set `repository` and `site` in `publication.json` (`node scripts/publication.mjs --site`
   checks them; Pages runs the same check before compiling). That is enough for a preview:
   the Download page and the app then say no signed downloads or updates are offered. Add
   `downloads` together with `manifestKey` once a signed release exists; see
   [Releases](../../docs/RELEASES.md#configure-publication). Private signing keys belong in
   release secrets, never in this file.
4. Actions → **Website** → Run workflow on the current `main`. Re-running an
   older failed run keeps its original commit and configuration. This runs quick checks and builds the
   site and app. Long tests belong to major releases; see [CI](../../docs/CI.md).
   The workflow builds with no registry and no relay, whatever other variables say.

GitHub Pages sends no custom headers, so `frame-ancestors`, `nosniff` and COOP are not
enforced there; the CSP and the referrer policy are, from the pages themselves.

**Search engines.** The website is meant to be found, preview or not: every page names its
canonical address, `robots.txt` allows everything and points at `sitemap.xml`, which lists the
pages. All of them use `HOI_SITE_URL` (default `https://hoi.sh`); set it at build time when your
copy lives on another domain. A preview app keeps itself out of search results with a robots
`<meta>` (and `X-Robots-Tag` in `_headers`). The link preview image is `apps/site/public/og.png`;
`node apps/site/scripts/og-image.mjs` redraws it.

**Anywhere else.** Upload `dist/` as it is. Serve `install.sh` and `install.ps1` as plain text
and don't cache them, and don't cache `/release.json` or `/releases/`.

﻿### Signed release metadata (`/releases/`)

The site and the app fetch the signed release list at `/releases/manifest.json` and its detached
signature at `/releases/manifest.json.minisig` (the `downloads` address in `publication.json`;
see [Releases](../../docs/RELEASES.md)). When the site and the metadata are served from different
origins, the metadata needs CORS; when they share an origin, no CORS header is needed. Either
way, serve both files together and never cache them for long: clients must always revalidate, so
a new release is picked up immediately and a manifest is never paired with a stale signature.

Headers for the two files (adjust the origin to your site's):

```
Access-Control-Allow-Origin: https://hoi.sh
Vary: Origin
Content-Type: application/json; charset=utf-8        # manifest.json
Content-Type: text/plain; charset=utf-8               # manifest.json.minisig
Cache-Control: no-cache                                # both: always revalidate
```

`Access-Control-Allow-Origin` may be `*` only while the metadata is public and no credentials are
sent; a named origin is safer. `Vary: Origin` keeps caches from serving one origin's CORS answer
to another. `no-cache` (not `no-store`) lets a conditional request return 304, so revalidation is
cheap. The same `no-cache` rule applies to `/release.json` and to `install.sh`/`install.ps1`
above. Hashed assets under `/assets/` are the only thing that may be cached long
(`public, max-age=31536000, immutable`); `dist/_headers` already sets all of this for hosts that
read that file.

**Replace the pair atomically.** Upload `manifest.json` and `manifest.json.minisig` together, in
one deploy, never one after the other: a client that reads a new manifest with the old signature
(or the reverse) fails verification and must not be left in that state. Per host:

- **Netlify / Cloudflare Pages / any host that deploys a directory atomically.** Put both files in
  the same `dist/releases/` upload; the deploy swaps the directory in one step. Don't edit files
  in place through a web console.
- **GitHub Pages (Actions).** The Website workflow copies both files from the published release
  into `dist/releases/` in the same build, so one workflow run publishes the pair together.
- **S3 / CloudFront.** Upload both to a new `releases/vX.Y.Z/` prefix, then copy
  `manifest.json` and `manifest.json.minisig` into `releases/` and create one invalidation for
  `/releases/*`. Keep the old version's files in place (don't delete them in the same deploy) so
  clients mid-update can still fetch them; delete old versions only after the new manifest has
  been live for a while.
- **nginx / Caddy / any server you control.** Stage both files as `manifest.json.new` and
  `manifest.json.minisig.new`, verify the signature locally
  (`minisign -Vm manifest.json.new -P "$RELEASE_PUBKEY"`), then rename both into place back to
  back in one script (`mv` is atomic per file; do the two renames consecutively and reload only
  after both). Never serve a directory listing of `/releases/`.
- **Any host without atomic deploys.** Serve `/releases/` from a versioned path
  (`releases/vX.Y.Z/manifest.json`) and make `/releases/manifest.json` a copy you flip in one step;
  or accept a short window where verification fails and clients retry on the next check.

Verify after every upload: `curl -sI https://hoi.sh/releases/manifest.json` shows
`Cache-Control: no-cache` (and `Access-Control-Allow-Origin` when origins differ), and
`minisign -Vm manifest.json -P "$RELEASE_PUBKEY"` passes against the served bytes.

## Nothing to configure

The app needs no service address to work: reading, joining and, through a home node, posting all
happen between devices. A copy you build and host yourself works the same way. Building one
that also talks to a registry or to encrypted private relays you run is possible
(`VITE_HOI_REGISTRY_URL`, `VITE_HOI_RELAY_URLS` at build time), but that is your copy's
configuration, never the project's: the Pages workflow pins both empty, and
`scripts/static-boundaries.test.mjs` fails if anything in the repository adds a default node,
registry or relay. The static host never proxies requests, and `/registry` and `/relay` do not
exist on it.

What people run themselves, none of it started by this deployment:

- [A home node](../home/README.md): the desktop app or `hoi-node`, a public node too if you choose.
- [A public node on a server](../server/README.md): the same with no person's key; no website or sign-up.
- [A registry](../../registry/README.md), optional: `hoi-node --registry`, the dependency-free
  Node API, or a Worker (`registry/wrangler.toml`); signed names, keys, versions and peer
  announcements only, for apps built to use it.
- [A private relay](../../relay/README.md), optional: separate ciphertext storage with its own
  CORS policy, for apps built to use it.

An absent registry costs public names only; invites, node links and the communities connected
nodes list all work without one. A registry entry alone cannot supply a community anyway: at
least one compatible node must have its records and be reachable.

## Verification

After building:

```sh
node scripts/e2e-deployment.mjs
node scripts/e2e-registry-browser.mjs
node scripts/e2e-discovery.mjs
```

`e2e-deployment.mjs` serves `dist/` the way a plain static host does
(`scripts/serve-static.mjs`) and checks that every page carries its policy, registry and
relay paths return 404, writes cannot become service data, demo inputs stay local, the website
uses no cookies or browser storage, and the app loads. `node scripts/serve-static.mjs` serves
`dist/` at `http://127.0.0.1:8787` to look at it yourself. The other checks use a separate
local registry. Private relay tests use their own local relay Worker; see its README.
