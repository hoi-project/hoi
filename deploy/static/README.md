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
