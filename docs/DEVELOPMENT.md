# Development

Use Node 22, pnpm 10 (the exact version is in `package.json`), and the Rust toolchain
pinned in `core/rust-toolchain.toml`. Browser builds also need:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked
pnpm install --frozen-lockfile
pnpm core:wasm
```

Run `pnpm dev:site` for the website or `pnpm dev:web` for the browser app. For UI work
without a WASM build, run `pnpm --filter @hoi/web dev:mock` in a POSIX shell; in PowerShell,
set `$env:VITE_HOI_BACKEND='mock'` and run `pnpm dev:web`.

`pnpm build` assembles the static website and app into `dist/`.
`node scripts/serve-static.mjs` serves them at `http://127.0.0.1:8787`.

## Build everything

`pnpm build:all` (`scripts/build-all.mjs`) builds every part this computer can build into
`builds/hoi-<version>/`, which Git ignores. Each part has a folder. File names follow
`<tool>_<version>_<os>-<arch>[-setup|-portable].<ext>`, so each name says what the file is:

| Folder | Tool | Needs |
| --- | --- | --- |
| `server/` | `hoi-server_…_linux-x64.tar.gz`, the server bundle (`install.sh`, `deploy.mjs`); `hoi-node_…_x86_64-unknown-linux-gnu.tar.gz`, hoi-node alone | Docker (Linux containers), or `--node-binary PATH` |
| `node-setup/` | hoi Node Setup: `…-portable.exe` (Windows, runs without installing), `…-setup.exe`, `.dmg` (and `…-portable.zip`), `.AppImage`, `.deb`, `.rpm` | Rust and the [Tauri prerequisites](https://tauri.app/start/prerequisites/) |
| `app/` | hoi, the app: the same installer kinds for this system | the same |
| `website/` | `hoi-website_….tar.gz`, the website and browser app as static files | the WASM setup above |

`README.md` in the folder lists every file with its size and use, and `SHA256SUMS` covers
them. `--only` and `--skip` take part names (`server,node-setup,app,website`), and
`--arch all` builds both server architectures. Rebuilding a part replaces only its folder; a
part that fails is reported, keeps its previous folder, and the rest still build. Desktop
installers come out for the system that builds them. The signed, flat folder for a GitHub
release is still [release-local](RELEASES.md#local-builds)'s.

hoi Node Setup looks for a hoi-node archive beside itself and in `../server/`, so the portable
app run from `builds/hoi-<version>/node-setup/` uploads `server/hoi-node_…` to the VPS. In a
development build (`pnpm dev:setup`) it also looks in the checkout's `builds/*/server/`.

The installers' artwork (the NSIS header and sidebar, the macOS disk-image background) comes
from `node scripts/installer-art.mjs`, and the setup app's icons from
`pnpm --filter @hoi/setup icons`. Both are committed; run them again after changing the brand.

| Area | Guide |
| --- | --- |
| Rust core and headless node | [core](../core/README.md), [hoi-node](../core/crates/hoi-node/README.md) |
| Web app | [apps/web](../apps/web/README.md) |
| Desktop and native dependencies | [apps/desktop](../apps/desktop/README.md) |
| Node setup app (SSH installer with a wizard) | [apps/setup](../apps/setup/README.md) |
| Mobile libraries | [apps/mobile](../apps/mobile/README.md) |
| Website wiki, SBOM and network playground | [apps/site/wiki](../apps/site/wiki/README.md) |
| Isolated test sandbox | [sandbox](../sandbox/README.md) |

Start with `pnpm check:quick`. Run focused tests for code you change, such as
`pnpm --filter @hoi/web test` or `cargo test --locked -p hoi-types` from `core/`.
`pnpm core:test:wasi` runs the pure core crates' tests and the conformance vectors built for
`wasm32-wasip1` under Node (`rustup target add wasm32-wasip1` first). Fuzz targets for the
scattered-delivery decoders are in `core/fuzz` (`cargo +nightly fuzz run manifest`).
`pnpm check` runs TypeScript/Svelte checks; `pnpm test` runs the JavaScript suites.
Rust workspace tests, browser integration and platform builds belong to major-release
validation in CI; see [CI](CI.md). They remain available locally when needed.

Keep UI tokens in `design/tokens/tokens.json`; `pnpm tokens` regenerates CSS, Swift and
Kotlin outputs. Keep protocol changes covered by `core/vectors/` and core tests.

The website's wiki (`apps/site/wiki/`) documents the protocol and every program from the code.
When you change behaviour it describes, update its page; `pnpm check:wiki` checks every page's
structure and links. After any lockfile change run `pnpm sbom` (it needs Cargo and an installed
workspace) so the published SBOM matches; `pnpm check:quick` fails until it does.
