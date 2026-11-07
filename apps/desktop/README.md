# hoi desktop

The desktop companion for Linux, Windows and macOS: the same UI as the web app (`apps/web`, built with `--mode desktop`) in a Tauri 2 shell, over the **native** core instead of the browser's wasm one.

Why it exists: the browser runs hoi locally but cannot accept connections or reliably stay awake. The desktop app provides the native transport and keeps communities reachable while it runs. It has two faces in one bundle. It opens as the **companion**: a small window and a tray icon with status, what passes through, a QR code for your phone and *Open hoi in my browser*, no forum and no key needed. Clicking the tray icon shows the companion. The **full app** (read and post right here) opens from the companion or the tray menu and widens the window; hoi still opens as the companion next time unless you turn off *You → Open as companion*. Closing the window keeps hoi in the tray; quit from the tray menu.

| | Browser (PWA) | Desktop |
|---|---|---|
| Core | `hoi-wasm` in a Web Worker | native `hoi-core` |
| Storage | Encrypted payloads in IndexedDB | Encrypted payloads in SQLite in the app data folder |
| Stamps | worker pool | every CPU core |
| Home node for your browser and phone | no | **yes**: your browser over `ws://127.0.0.1:7842`, your phone over WebRTC (UDP 7844), one click or one scan |
| Keeps communities reachable for members | no | **yes**, a switch with a daily cap (on by default, with a notice) |
| Peer across networks (iroh, user-run relays) | through a paired home node | native endpoint with explicit invite hints (see [protocol §10](../../docs/spec/PROTOCOL.md#10-discovery)) |
| `hoi://` links | no | opens invites and pairing links |

## Run it

Prerequisites: Node 22, pnpm 10, Rust (the version in `core/rust-toolchain.toml`) and [Tauri's system dependencies](https://v2.tauri.app/start/prerequisites/) (Linux: `libwebkit2gtk-4.1-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev libdbus-1-dev`; macOS: Xcode command line tools; Windows: WebView2 and the MSVC build tools).

```sh
pnpm install
pnpm --filter @hoi/desktop dev      # starts the web UI on :5174 and opens the app
pnpm --filter @hoi/desktop build    # bundles: .dmg/.app (macOS), .msi/.exe (Windows), .AppImage/.deb/.rpm (Linux)
```

`tauri build` runs `pnpm --filter @hoi/web build:desktop` first (output `apps/web/dist-desktop`). A preview version (such as `1.0.0-preview.1`) is built for Windows without the `.msi`: Windows Installer takes numeric versions only, so `scripts/tauri.mjs` asks for the NSIS `-setup.exe` alone (`scripts/tauri-bundles.mjs`). Signed release builds come from `.github/workflows/release.yml` on a `v*` tag. `pnpm build:all --only app` copies this system's bundles into `builds/hoi-<version>/app/` as `hoi-desktop_<version>_<os>-<arch>…`. The installers show the artwork in `src-tauri/installer/` (`node scripts/installer-art.mjs` draws it).

## How it fits together

```
apps/web (Svelte UI) ──invoke("hoi_call", {method, args})──▶ src-tauri/src/lib.rs
        ▲                                                       │ Shared::with(|node| node.call(...))
        └──────── "hoi://event" (changed, notice, net) ◀────────┤
                                                                ▼
                                     hoi-core (SQLite) ◀──▶ hoi-net: WebSocket server :7842
                                                                ▲
                                     paired browser / phone ────┘  (hoi-sync messages)
```

- **Commands** (`src-tauri/src/lib.rs`): `hoi_call` (the whole API, same JSON as the web worker), `hoi_mine` (stamps on every core, off the node lock), `hoi_benchmark`, `hoi_save_file` (native save dialog for key exports and bundles).
- **Home node**: after local unlock the app serves `127.0.0.1:7842` (or any free port if taken) and WebRTC on UDP 7844 on every local address. *Open hoi in my browser* (companion, tray, *You → Connection*) opens the pairing link in your browser: it connects over the loopback WebSocket. Chrome asks before a website such as hoi.sh may reach apps on this computer: choose **Allow** (it's *Apps on device* in the site settings); if it was blocked, the app says so under *Home node*. A phone scans the QR code: its browser opens hoi, connects over WebRTC on your network (the link pins this computer's certificate) and can ask to be linked to your key; you compare four digits here. Devices without the token are turned away, and nothing about them is logged.
- **Keeping communities reachable**: while the switch is on, members' browsers (WebRTC) and computers (QUIC) may connect and catch up, up to the daily cap. *What passes through this computer* shows amounts per minute and per connection with how each is encrypted, never content.
- **Mailboxes for others**: while *Be a public node* is on, *You → Keep mailboxes for others* lets this computer keep other people's private communities' sealed messages (and, later, encrypted pieces of posts), up to 1 to 20 GB, in `pieces.db` beside the database. It can read none of it, and its notice says so. Off, or no longer a public node, the file is deleted.
- **Native peers**: the app starts an authenticated iroh endpoint with no default relays or discovery servers. Joining an invite retains its peer and relay hints. Generated invites advertise the running endpoint. See **You → Connection details** for endpoint, route information and errors; a completed connection does not prove catch-up.
- **First relay**: there is no relay URL editor in the desktop UI. Existing community invites supply relay hints. For a new community that needs a relay, run `hoi-node --relay https://your-relay.example/`, pair the desktop to it, and let the paired link exchange the community's routes before exporting a fresh invite. Use a compatible relay operated by you or your community; the example domain is a placeholder.
- **Deep links**: `hoi://join?…` opens the join screen, `hoi://pair?…` the pairing screen (single-instance plugin forwards links to the running app on Windows and Linux).
- **Start with the computer**: the companion's switch registers hoi to start at login with `--hidden` (tray only, no window). `--start-at-login` does the same from outside, for an installer: `deploy/install` starts hoi with it once for the role `companion` (a running hoi takes it over the single-instance link and shows nothing).
- **Security**: the window may only call hoi's own commands (`capabilities/default.json`: `core:default`, no shell, no file system, no HTTP from the UI). `hoi_open_url` opens only `https://` and loopback `http://` addresses in the default browser. CSP in `tauri.conf.json`. Automatic update checks are not implemented.

Public names are optional: set `VITE_HOI_REGISTRY_URL` in the shell before `pnpm --filter
@hoi/desktop build` or `dev`. The package's Tauri wrapper allows that exact registry origin
in the native content security policy. Use the package scripts so the native policy and
web bundle agree. See [registry setup](../../registry/README.md). No registry is deployed
by building the desktop app; unregistered local communities still work by invite.

## Local storage protection

The app starts locked, including when launched at login. No core, peer listener or home-node connection starts until the person unlocks their local vault. `hoi_vault_status` reports create/migrate/locked/ready; `hoi_vault_unlock` validates and opens storage. `hoi_vault_lock` flushes pending writes under the node lock and restarts the process so background tasks cannot retain an unlocked core. A failed flush keeps the process open and reports an error. The locked tray can reopen the unlock screen.

A passphrase of at least 15 characters wraps a random data key using Argon2id (64 MiB, three passes). XChaCha20-Poly1305 encrypts record and blob payloads, including identity, transport and MLS keys, with their storage addresses authenticated. An encrypted inventory binds every ciphertext address and hash, rejecting missing, injected and individually replayed cells. Native inventories use the existing canonical CBOR implementation; the browser uses JSON with the same schema. Only encrypted values and non-content row metadata are persisted. Existing data is validated before migration and then rewritten atomically with a format guard that makes older clients refuse the database. Incorrect passphrases and damaged ciphertext never open a replacement identity.

SQLite uses secure deletion and truncates its WAL after migration. Successful reopen retries WAL cleanup before exposing the node, covering a crash after the encryption transaction commits. This does not erase existing backups, filesystem snapshots or copies made by earlier versions. Encryption does not defend an unlocked process, malicious client release, endpoint compromise or rollback of a complete earlier database. There is no passphrase reset that recovers a locked vault; keep recovery material safely.

Locked recovery is available from the unlock and startup-error screens. It requires the exact confirmation `RESET` and is rejected while the native core is open. It removes this installation's `hoi.db` and SQLite sidecars, then lets the user protect fresh storage and import a `.hoikey` backup or link a signed-in device. It cannot recover encrypted local content, revoke the old identity, or remove copies held by other devices. Authentication and later storage failures are reported separately so a valid passphrase is not blamed for a failed database read.

Password forms expose standard autofill fields. Browser password-manager extensions do not automatically run inside Tauri's WebView; native autofill depends on the manager and OS, and pasting remains supported.

**You** and **Companion → Settings** offer optional **Unlock with this OS account**. Enabling requires the current vault password, authenticates stored data, and writes a device-local unlock key to macOS Keychain, Windows Credential Manager or Linux Secret Service. The entry is read back before enabling it. The password slot and encrypted database format remain intact. The local `os-unlock` marker contains only the vault-header fingerprint. The unlock key never reaches the webview; the typed password is cleared after confirmation. Neither secret is stored in preferences.

OS protection depends on the logged-in account and its keystore; a biometric or PIN prompt is not guaranteed. A lost or locked keystore leaves password unlock available. Startup and explicit lock always show the unlock screen; OS unlock requires a button press. Removing OS unlock deletes its credential, and confirmed local reset removes it before deleting storage. If cleanup fails, reset stops and can be retried. This option is not an identity backup and does not sync or restore the vault on another computer. Linux builds need the Secret Service/D-Bus development dependency (`libdbus-1-dev`) and a running Secret Service for this optional feature.

On Windows, the credential write explicitly uses `CRED_PERSIST_LOCAL_MACHINE`; keyring's default enterprise-roaming persistence is not used for this vault key. Lookup and deletion use the same keyring entry.

## Still to do

- Cross-network NAT/relay acceptance, discovery, UPnP/NAT-PMP for the WebRTC port, and an embedded packet relay with a block list. Today a phone reaches the computer on the same network, or across the internet through a forwarded port.
- Code signing and notarisation secrets in CI; the Tauri updater pointed at the signed manifest.
- Verify OS-keystore unlock and manual password-manager fallback in built macOS, Windows and Linux apps; automated mock-keystore tests do not establish platform integration.
