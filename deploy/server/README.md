# A public node on a server

## With the setup wizard

```sh
pnpm build:all --only server,node-setup
```

This puts the server bundle and hoi-node in `builds/hoi-<version>/server/` and hoi Node Setup
in `builds/hoi-<version>/node-setup/`, with a portable app that runs without installing. Start
it from that folder and connect to the VPS: it finds `../server/hoi-node_…` for the server's
processor, uploads it with the rest, checks it arrived whole, and installs it. No published
release is needed. The server bundle below works without the wizard
([all of `pnpm build:all`](../../docs/DEVELOPMENT.md#build-everything)).

## Build a bundle and install over SSH

From the repository root, with Node 22+, pnpm, and Docker running with Linux containers:

```sh
pnpm build:server
pnpm deploy:server root@203.0.113.10
```

The first command builds the current checkout's Linux x64 server and creates
`dist/server/hoi-server-linux-x64.tar.gz` and an unpacked folder beside it. The second
opens a menu to choose what to deploy or which existing service to control, using your
normal SSH authentication and sudo prompts: one SSH connection carries the whole deployment
on Linux and macOS, so a password or a key's passphrase is asked once (Windows' OpenSSH
cannot share a connection and asks a password login again for the upload; a key, or the
node setup app, avoids that). Your computer also needs OpenSSH (`ssh`, `scp`) and `tar` for
deployment. Service controls need only Node and SSH locally.
You can use an SSH config alias, or add `--port 2222 --identity "path/to/key"`.

For a public node or a registry the menu first says what running one means (you answer for
other people's data) and asks you to acknowledge it, then for your **join notice**:
a JSON file the app's *You → A node on a server* page or the node setup app writes
(`--notice FILE` in a script). It travels with the bundle, the installer copies it to
`/etc/hoi-node/notice.json`, and the node carries it in its link and its first word to every
stranger, so people read it before it serves them. Without one the node runs and says so. In the
notice your name, contact and the server's country are optional but recommended; whatever you fill
in is shown to everyone who connects, and what you leave out, the notice says you left out.

For a manual upload, extract the archive on the VPS and run `bash install.sh` inside
the extracted folder. The bundle resolves its own paths and carries the binary: the
server needs no source checkout, Docker, Rust, Node, or release download. It installs
and starts the existing systemd `hoi-node` service and enables startup after reboot.
The target needs root or sudo and 64-bit Linux with glibc 2.36+ and systemd, such as
Debian 12 or Ubuntu 24.04 and newer. Alpine and hosts without systemd are not supported.

```sh
# ARM64 VPS (the default deploy command selects x64; override the folder for ARM64).
pnpm build:server --arch arm64
pnpm deploy:server root@203.0.113.10 --bundle dist/server/hoi-server-linux-arm64

# Package a Linux binary or release archive already built, without Docker.
pnpm build:server --binary dist/release/v0.1.0/hoi-node_0.1.0_x86_64-unknown-linux-gnu.tar.gz

# Choose a public node with registry and automatic HTTPS, with your join notice.
pnpm deploy:server user@host --role registry --registry-url https://names.example.org --tls caddy --notice hoi-NAME.notice.json

# Upload and check the host without installing a service.
pnpm deploy:server user@host --action check --role public

# A host whose public IP is mapped through NAT.
pnpm deploy:server user@host --role public -- --rtc-advertise 203.0.113.10:7844

# Control the existing service without rebuilding or uploading.
pnpm deploy:server user@host --action stop
pnpm deploy:server user@host --action start
pnpm deploy:server user@host --action status --service all
pnpm deploy:server user@host --action logs --service caddy

# Explicitly discard public-node/registry data and install a clean public node.
pnpm deploy:server user@host --role public --reset-data --confirm-reset hoi-public
```

Allow UDP 7843/7844 (and 7846/7847 for the DHT among public nodes) in the provider's firewall. The installer handles active ufw/firewalld
on the server. Select public, registry (public node plus registry), home relay, or private
node in the menu or with `--role`. Registry mode can configure Caddy (`--tls caddy`) or
use your proxy (`--tls external`). For saved settings select `--role config`, which reads
the generated folder's `server.conf`; rebuilding preserves that file. An explicit node
role ignores the saved file. The SSH helper packs the folder again, so edits are included. The
archive reflects settings at build time; rebuild it after edits before uploading it manually.
Rebuilding and deploying again updates the service while keeping its data. This replaces
the host's existing `hoi-node` service configuration. Data stays unless reset is explicitly
requested and confirmed: public/registry share `hoi-public`; home/private share `hoi-node`.
Stop preserves data; disable also prevents startup after reboot. `--service all` controls
only installed hoi-node and Caddy services. Caddy controls affect every site it serves.
The bundle's [README](bundle/README.md) covers all choices, service controls and reset behavior.

## Other deployment options

This is the VPS deployment: a public node with no person behind it. For a computer at home
(a Raspberry Pi, a Mac mini) that also serves your own devices, see [deploy/home](../home/README.md).
Without Docker, the installer sets up the same node as a service, a registry too, with Caddy for
its HTTPS if you like. The app makes everything the server needs on your own device (**You → A node
on a server**: a configuration under a random name the server takes as its host name, the
installer, a README and a cloud-init file to paste when you order the server), and the installer
checks the server before it changes anything
([a node for another computer](../install/README.md#a-node-for-another-computer),
[the check](../install/README.md#before-it-installs-the-check)).

The static website and client deployment never launches this node. Anyone can run it on
their own machine; it is a peer that accepts connections and stores/serves public community
records. It has no website, accounts or sign-up.
The supplied Compose configuration uses `--public`: no paired-device control
listener, no pairing links, no person's key; only the native peer and WebRTC transports are
started, relaying is on, and the node takes open communities from connected peers that hand it
a community's signed definition (`--carry any`), founders' browsers included. It tells whoever
connects about the other public nodes it reached and writes a link anyone can open to
`/data/node-link`; no registry is needed. What it carries on request is bounded
(`--carry-limit`, `--carry-mb`), let go and deleted after two weeks without another holder or an
hour without its records, and the operator can keep a key off the node with `--refuse KEY`. To
also mirror and announce to a registry, add `--index URL` to the command in a
`compose.override.yml`.

Public records can include text, profiles, signed actions and media. The node can read
what it carries and persists its database across restarts. It is not a metadata-only
registry or a transient packet relay.
No peer access logs are added by the application. Host/network logging is separate.

Configure `deploy/server/.env` with `HOI_RTC_ADVERTISE` (the node's reachable public IP and UDP
port). No public domain, website or registry is required for the peer. Never put private keys or
personal details in public announcements.

```sh
# From the repository root; Docker build includes core/ and its vendor/ patches.
docker compose -f deploy/server/compose.yml build
deploy/server/deploy.sh up
```

Open UDP 7843, 7844, 7846 and 7847 in the host firewall. `HOI_CARRY_LIMIT` bounds the number of
communities it carries (default 256). `HOI_NODE_NAME` is a local label.
`HOI_CARRIER_IMAGE` can select an image you built and pushed elsewhere instead. Nothing in
the static site's workflow builds or starts this node.

Clients reach it through its node link, the invites of the communities it carries and the other
public nodes that pass it on, connect directly, check signatures and apply community rules
locally. With `--index`, it also signs announcements of its routes for that registry, which
holds no posts or media. Availability depends on reachable peers retaining the requested
records. Independent registries have independent naming namespaces.

Stopping the node stops serving but does not erase its persistent volume; plan its storage and
backups like any other server's. Community moderation decides what a community keeps; what this
node carries is up to you (below).

## Stop serving an identified item

There is no admin endpoint, on the node or on the registry, and the operator never signs
anything on a community's behalf. The tools run from a shell on the host and act on
this deployment's own copy: the item is deleted here and refused from then on, whoever offers,
lists or asks for it; other nodes keep their copies and their own responsibilities.

1. **Identify** the item: a community by its 64-character key, a post, comment, edit, report or
   profile by its record id, a picture or file by its BLAKE3 hash. The app shows ids and keys; a
   registry lookup (`GET <registry>/v2/communities?name=NAME`) gives a community's key.
2. **Stop the carrier**, so nothing changes under the tool (it refuses to run while the node
   holds its lock):

   ```sh
   docker compose --env-file deploy/server/.env -f deploy/server/compose.yml stop public-carrier
   ```

3. **Keep a copy, only if you want one**, and **suppress**, with
   the same image and volume, as the same unprivileged user:

   ```sh
   compose=(docker compose --env-file deploy/server/.env -f deploy/server/compose.yml)
   # Evidence goes outside the node's volume, owned by the image's unprivileged user.
   uid=$("${compose[@]}" run --rm --no-deps --entrypoint id public-carrier -u)
   sudo install -d -m 700 -o "$uid" /srv/hoi-evidence
   "${compose[@]}" run --rm --no-deps -v /srv/hoi-evidence:/evidence public-carrier \
     admin --data /data suppress community KEY --note "case 2026-14" --evidence /evidence
   ```

   Use `suppress record ID` or `suppress media HASH` for a single item; leave out `--evidence`
   when you need no copy (the default). The copy is the signed records exactly as received
   (each verifies on its own) and the media, readable by the container's user only, with a
   `manifest.json`; keep it no longer than you need it, then delete it. `--note` is your
   own reference, never anyone's personal details. The command prints counts and ids, never
   contents. If the evidence cannot be written, nothing is deleted.
4. **Start the carrier** (`deploy/server/deploy.sh up`) and **verify**: `admin --data /data
   suppressions` (run as above) lists it; within five minutes the registry no longer names this
   node as a carrier of a suppressed community (`GET <registry>/v2/nodes?community=KEY`); a
   carry request or index listing for it is refused.
5. **Registry**: if the community's name is listed, ask the registry's operator to withhold the
   listing, or do it yourself if you run it ([registry guide](../../registry/README.md#operator-tools-withhold-a-listing-drop-a-node)),
   so carriers elsewhere stop being pointed at it. A registry can also drop a node's
   announcements by its endpoint key.

`unsuppress KEY|ID|HASH` lifts a suppression; nothing deleted comes back by itself. A
`--refuse KEY` flag in `compose.yml` only lasts as long as the command line keeps it; a
suppression is stored with the node and survives updates of the image.

**Backups and retention.** Deletion happens inside SQLite with `secure_delete` on and the
write-ahead log is flushed afterwards, but volume snapshots, `docker volume` backups and
provider snapshots taken earlier still hold the item. Delete or let them expire according to a
written retention period, and keep that period short. A volume restored from an older backup has
neither the deletion nor the suppression: suppress again before starting it. Evidence copies are
not backups; keep them apart from the node's volume and its backups.

The same binary runs a registry node: add `--registry 0.0.0.0:7845` with `--registry-origin`
for each app origin and `--registry-url` set to its public URL, put TLS in front, and point
`--index` at that public URL so the node announces to and mirrors its own registry; see the
[hoi-node guide](../../core/crates/hoi-node/README.md) and the [registry guide](../../registry/README.md).
Announcements are signed for one registry's origin, so `--registry-url` must be the address
carriers are given. For a private home node with a paired-device control interface, omit
`--public` deliberately: the node then listens on `127.0.0.1:7842`, which with
`network_mode: host` is the host's loopback, and keeps its pairing link in `/data/pairing-link`.
