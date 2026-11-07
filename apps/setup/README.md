# hoi Node Setup

A small desktop app that installs and manages a **public node** on a server over SSH, or on
this computer, with a wizard. It is the "small application with a wizard" for operators who
would rather not type the installer's commands themselves; it runs the same
[`deploy/install/install.sh`](../../deploy/install/README.md) the download page offers, and
nothing else. It ships beside the hoi app for macOS, Windows, Linux x64 and Linux ARM64
(a Raspberry Pi 5 with a desktop), and is offered on the download page under
*Before you run a public node*.

It needs no installing: `pnpm build:all --only server,node-setup` puts a portable app in
`builds/hoi-<version>/node-setup/` (`…-portable.exe` on Windows, `…-portable.zip` with the
`.app` on macOS, the AppImage on Linux) beside the installer, and the hoi-node for the server in
`../server/`. Run it from there: it finds that hoi-node itself, so a node installs without a
published release.

## What it does, step by step

1. **Responsibility.** The *Before you run a public node* text, and a box to tick: the person
   running the node is responsible for the data it processes.
2. **Server.** A server over SSH (host, port, user; login through the ssh-agent, a key file with
   its passphrase, or a password for this session only; Ed25519 and ECDSA keys, not RSA, whose
   implementation leaks timing), or *This computer* (a Raspberry Pi or a
   computer that stays on; Linux installs as a system service through sudo, macOS in the account,
   Windows for the account through `install.ps1`).
3. **Host key.** The server's host key fingerprint is shown before any login and pinned with the
   server; a server whose key changed later stops everything.
4. **The node.** Public node, or public node and registry (domain and Caddy, or an address of
   your own); the name; **Keep mailboxes for private communities** (on, 2 GB: the node keeps
   sealed messages and pieces it cannot read, private communities' devices find it by themselves,
   and the notice says so); and under *More* the extra registries, a daily cap and the public
   address when NAT maps it. The same choices as the kit page in hoi (`apps/web/src/lib/server-kit.ts`).
   **The program:** after login the wizard reads the server's `uname -m` and offers the matching
   `hoi-node_<version>_<target>.tar.gz` built on this computer (beside the app, in `../server/`,
   or in a development build the checkout's `builds/*/server/`; *Another file…* picks one), or
   the signed release list.
5. **Join notice.** The operator's words in the fields; the preview shows the notice exactly as
   people read it before they connect, and how much of the link's budget it uses. Name, contact,
   country and provider are optional but recommended: left empty, the notice says so to everyone
   who connects, and the step lists what was left out; filled in, they are shown to everyone.
6. **Check.** The configuration, the notice and the installer go to a private folder on the
   server (`umask 077 && mktemp -d`), with the hoi-node from this computer when that was chosen:
   read by the Rust side (never through the window), uploaded in pieces with a progress bar, and
   compared by SHA-256 with what `sha256sum` reads on the server. `install.sh check --config …
   [--binary ./hoi-node_….tar.gz]` prints its rows: what is fine, what is a warning, what blocks.
   Nothing on the server has changed yet.
7. **Install.** `install.sh --config … --yes`, line by line as it happens. A sudo prompt is
   answered with the password the window asked for; the installer installs the uploaded hoi-node,
   or fetches it from the signed release list and checks it. **Manage → Update** makes the same
   choice by itself.
8. **Done.** The node's link, with the notice in it, as text and QR code, and the notice hash.
   Founding a community on it stays in hoi: *Open in hoi* takes the link there.

Every command is printed before it runs, under *What was run*; the person can read the whole
session afterwards.

**Your servers** keeps what was set up here: host, user, login kind, the pinned host key, the
node's name, role, link and the files it got. Never a password. **Manage** connects again and
shows the service's state and version, the logs, the link; restarts, stops, enables, updates
(re-runs the installer), shows how much the mailboxes keep (totals from `hoi-node admin pieces`,
never what is in them), edits the notice on the server (the node reads it again within half a
minute and tells people who accepted the old words), sets how long the server keeps its own logs
(a journald drop-in; hoi-node logs no addresses), deletes the node's logs now, and uninstalls, with
the data erased only when the node's name is typed.

## How it looks

Like hoi.sh (`apps/site/src/styles/site.css`), from the same tokens: the dot logo over an ink rule,
a yellow sticker and a two-tone headline (ink, then cobalt) for every step, yellow numbered badges
for the steps, the responsibility text as ledger rows, tagged check rows (mint ok, sky info, sun
warn, coral BLOCK), ink-framed cards and pill buttons. It is laid out for the 880-pixel window and
works down to phone width, in light and dark.

## How it is built

- `src-tauri/`: the Tauri 2 shell (crate `hoi-setup`, identifier `sh.hoi.setup`, product
  name "hoi Node Setup", program `hoi-node-setup`). It has an icon of its own (the hoi mark on
  an ink tile, with a server; `pnpm --filter @hoi/setup icons` makes every size from
  `icons/icon.svg`), and the installers show `installer/` (header, sidebar, disk-image
  background; `node scripts/installer-art.mjs` draws them). Its own Cargo workspace, like the desktop app, so the core workspace stays
  free of SSH and WebKitGTK and cargo-vet does not cover it; `deny.toml` and the release
  workflow's dependency review do. Plugins: `dialog` (pick a key file, save a file) and `opener`
  (open the link). Capabilities: `core:default` only.
- `src-tauri/engine/`: `hoi-setup-engine`, the part without Tauri, so it builds and tests on
  its own. `ssh.rs` wraps russh: probe the host key, connect with a pinned fingerprint, run a
  command with or without a pty (a password prompt on the pty is answered once), read a command's
  output, upload a file with `cat` (no SFTP; a hoi-node in pieces, checked by SHA-256 on
  arrival), close. `node_files.rs` finds and checks hoi-node archives on this computer.
  `profiles.rs` is `servers.json` in the app's config folder. `local.rs` runs a program on this
  computer.
- The window is a page of the web app: `apps/web/setup/index.html` and `apps/web/src/setup/`
  (`Setup.svelte`, `steps.ts` for the pure parts, `bridge.ts` for the Tauri commands,
  `mock-bridge.ts` for a browser without Tauri). It reuses the kit page's files, the notice
  model and texts, `NoticeEditor`, `NoticeText` and the QR code. `pnpm --filter @hoi/web
  build:setup` writes `apps/web/dist-setup/`, which `tauri.conf.json` bundles; the installer
  scripts are imported as text, so the bundle always carries the installer of its own version.

```sh
pnpm dev:setup                                   # the app, with the window served by Vite on :5175
pnpm build:setup                                 # bundles for this system (apps/setup/src-tauri/target); a preview: no .msi on Windows
pnpm build:all --only server,node-setup          # the same, named and with the portable app, in builds/hoi-<version>/
pnpm --filter @hoi/web dev:setup                 # the window alone in a browser, with a pretend server
```

In the browser the pretend server (`mock-bridge.ts`) pins `SHA256:mock…`, takes the password
`correct horse`, cannot reach `down.example`, finds a hoi-node built for x86_64 and uploads it,
prints the check rows and installs a node whose link carries a notice.

## Tests

```sh
cargo test --manifest-path apps/setup/src-tauri/Cargo.toml --workspace   # quoting, prompts, profiles, the local runner
pnpm --filter @hoi/web test                                              # src/tests/setup-steps.test.ts
node scripts/setup-build.test.mjs                                        # the window builds without the core
node apps/web/scripts/e2e-setup.mjs                                      # the wizard and Manage against the pretend server
```

The release workflow builds it in the `setup` job, on the same switch as the desktop app;
`scripts/release-manifest.mjs` marks its files with `product: "setup"`, so the download page lists
them in their own row and the installers never take them for the app.
