# hoi web app

The app as a static PWA (Svelte 5 + Vite). The same build, with `--mode desktop`, is the UI of the desktop companion (`apps/desktop`). People create and run their communities here: start one, invite people, set limits and rules, add moderators, admit newcomers, remove posts with a public reason, ban, read the ledger, link devices and pair with a home node.

## Sign-in and recovery

The local vault password unlocks storage in this browser or desktop app. It is separate from the recovery key (`hoi-key-1-…`) and from an exported `.hoikey` file's password. **You → Back up my identity** offers both backup methods, one form at a time. New-device setup protects local storage first, then offers **Restore my identity** using a recovery key, a file and its password, or the existing authenticated device-linking flow. Recovery keys preserve the person's standing by rotating the signing root; they still need community history to sync. Files restore the exported identity, profile and settings, not private-group history. A public key is not a sign-in secret.

The unlock and storage-error screens offer **Recover or start over**. Clearing data requires a checkbox and the exact text `RESET`; the browser must own the writer lock, and an unlocked runtime refuses recovery. Clearing destroys only this installation's local data and cannot recover an old key without a backup or another usable device. It does not revoke the old identity or erase peers' copies. No reset runs in response to an incorrect password or failed import.

Password forms expose editable `username` fields, `current-password` on use and `new-password` on creation. The submit handlers read the DOM, including autofill that sends no input events. New vaults get distinct, remembered non-secret entry labels; recovery entries use `hoi key · <name> · <short key>`; each file export gets a unique filename and matching `hoi key file · <filename>` label. Restore accepts a selected manager entry instead of fixing the username in advance. Existing vault and file passwords still work.

The browser's save API is optional and is never treated as proof of a saved backup. **Forget the recovery key here** is offered only after the user retrieves a matching key in a separate form. The check stays local and does not rotate the identity. Manual copy/paste remains available for managers without autofill, including native Tauri windows. Desktop also offers optional OS-keystore unlock, described in [its README](../desktop/README.md#local-storage-protection). None of these flows needs an account service or new local server. Real Dashlane, 1Password and each browser/OS combination require separate compatibility testing; standard HTML fields alone do not prove support.

## How it's wired

```
UI (src/routes, src/lib/ui) ──▶ HoiCore (packages/core-api) ──▶ src/lib/core/index.ts picks one:
   ├─ worker-core.ts   browser: core.worker.ts runs hoi-wasm; OPFS log files (opfs.ts) or IndexedDB (idb.ts); stamp miners (miner-pool.ts);
   │                   one tab owns the store (Web Locks); your home node over WebSocket or WebRTC, members'
   │                   nodes over WebRTC without one (the page hosts WebRTC for the worker: rtc-link.ts)
   ├─ tauri-core.ts    desktop: invoke("hoi_call") into the native core
   └─ mock-core.ts     development only (VITE_HOI_BACKEND=mock), seeded, never in production bundles
```

The UI never touches keys, storage or sockets. Posts, replies and edits go draft → mine → publish (`rpc-core.ts`) so the stamp is mined on every core and the composer's work bar can show it. A new key does the same with its identity work: the first run, "say hoi" (`lib/ui/SayHoi.svelte`, `lib/sayhoi/`), shows every Argon2id try the miners make, and hands the core a digest of your gestures to stir into the key's randomness.

State lives in `src/lib/app.svelte.ts` (phase, me, communities, net, settings, a `rev` counter bumped by `changed` events). Screens load with `resource(() => { void app.rev; return app.core.x() })`, so new records from peers re-render what's on screen. Routing is hash-based (`src/lib/router.ts`), so the build works under `/app/`, on its own origin, and inside Tauri.

## Run it

```sh
pnpm core:wasm                       # once, and after core changes (from the repo root)
pnpm --filter @hoi/web dev           # real core in a worker
pnpm --filter @hoi/web dev:mock      # seeded mock, no core needed
pnpm --filter @hoi/web build         # dist/ with CSP meta, SRI, service worker, _headers
pnpm --filter @hoi/web build:desktop # dist-desktop/ for Tauri (no CSP meta, no service worker)
pnpm --filter @hoi/web dev:setup     # the node setup app's window (setup/, src/setup/) with a pretend server
pnpm --filter @hoi/web build:setup   # dist-setup/ for apps/setup
pnpm --filter @hoi/web check && pnpm --filter @hoi/web test
node scripts/e2e-mock.mjs --out <dir>          # UI flows on the mock, both widths and themes
node scripts/e2e-qol.mjs                         # settings layout and photo crop at four widths
node scripts/e2e-consent.mjs                     # Network & Safety before anything, the operator notice, accept and decline
node scripts/e2e-join-warning.mjs                # every join waits for the VPN warning
node scripts/e2e-setup.mjs                       # the node setup wizard and Manage against the pretend server
node ../../scripts/e2e-profile.mjs               # real WASM profile publish, persistence and removal
node ../../scripts/e2e-vault.mjs                 # real WASM unlock, recovery, corruption and backup restore
node ../../scripts/e2e-os-unlock.mjs             # OS-unlock UI with a stubbed native boundary; real keystore checks remain separate
node ../../scripts/e2e-discovery.mjs             # registry navigation and search privacy
node ../../scripts/e2e-network.mjs --shots <dir>   # two browsers through hoi-node, real core
node ../../scripts/e2e-rtc.mjs --shots <dir>       # a phone pairs and links over WebRTC; a member joins while the inviter is away
# mock switches: ?platform=desktop (serving, relaying, traffic; opens as the companion) · ?full · ?ask (a link request) · ?paired
```

## Rules this app keeps

- No third-party requests: fonts and Anime.js are bundled; `connect-src` allows `ws:`/`wss:` only for home nodes and relays you choose.
- Honest about being a browser: the relay pill says "only in this browser" until you pair with your home node, then "via <its name>".
- Every signed action gets a toast; removed things show "Poof." with the reason and the ledger; hidden things can be peeked.
- Reduced motion and the motion setting are respected; every view is complete at rest; no horizontal scroll at 320px.
- Use short sentences and clear labels.

## Known gaps

The first-run friend-recovery demonstration uses only the public key, not recovery shares; real recovery is configured under **You → Friends**. Social recovery and labeler subscriptions are implemented. A browser cannot act as a listening public relay. Tor routing is not implemented in either build. Owners collect cosignatures by passing proposal codes; a proposal inbox and an appeals protocol are outside the completion scope.
