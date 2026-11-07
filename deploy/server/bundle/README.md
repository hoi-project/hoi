# hoi server bundle

This folder contains a built Linux hoi-node and its service installer. The server needs
64-bit Linux with systemd, Bash, tar, sha256sum, and root or sudo access. Docker builds
need glibc 2.36 or newer: Debian 12 or Ubuntu 24.04 and newer work. Alpine is not supported.
The architecture must match the bundle (x64 or ARM64).

## Install over SSH from your computer

With Node 22+, OpenSSH (ssh and scp), and tar installed on your computer:

```sh
node deploy.mjs root@203.0.113.10
```

This opens a menu: deploy/check a node, list existing services, or control a service.
For deployment, choose public node, public node with registry, home relay, private home
node, or the saved server.conf settings. The registry choice asks for its HTTPS URL
and whether to configure Caddy. A public node or registry first says what running one means
(you answer for other people's data) and asks you to acknowledge it, then for your join notice,
a JSON file from the hoi app (You → A node on a server) or the node setup app (`--notice FILE`
in a script); it travels with the bundle and the node carries it. Existing data is kept unless
you choose to start fresh.

Run this from the extracted folder, or give the full path to deploy.mjs from anywhere.
SSH config aliases, SSH keys, password login, and sudo prompts work as usual; on Linux and
macOS one connection carries the whole deployment, so a password is asked once (Windows'
OpenSSH cannot share a connection and asks again for the upload). For scripts, give the
choices as flags:

```sh
node deploy.mjs my-vps --port 2222 --identity "path/to/key"
node deploy.mjs user@host --role public -- --rtc-advertise 203.0.113.10:7844
node deploy.mjs user@host --role registry --registry-url https://names.example.org --tls caddy
node deploy.mjs user@host --role public --notice hoi-NAME.notice.json
node deploy.mjs user@host --action check --role registry --registry-url https://names.example.org --tls caddy
```

`--action check` uploads the bundle and checks the host without installing a service. Uploads stay
in the private /tmp/hoi-deploy.* folder printed by the helper; the host may clear it later.
Node, Docker, Rust, and a source checkout are not needed on the server.

## Upload and install yourself

Upload the .tar.gz using your SSH/SFTP client, extract it, then run:

```sh
bash install.sh
```

The installer offers the same role and data choices. Use `bash install.sh --role public`
or `bash install.sh --role registry --registry-url https://names.example.org --tls caddy`
to give the choice directly. An explicit role ignores server.conf, so selecting a public
node never inherits a saved registry configuration. `--role config` uses that file.

The installer resolves its own paths. You can also run `bash /path/to/bundle/install.sh`
from any directory. It checks the binary and installer checksums, checks the server,
installs hoi-node, and starts a systemd service that also starts after reboot.
It installs the binary at /usr/local/bin/hoi-node and keeps public-node data at
/var/lib/hoi-public (public/registry), or /var/lib/hoi-node (home/private). Repeating the
installation updates the existing hoi-node service and keeps its data by default.
A host has one hoi-node service; this replaces its configuration. Registry mode adds
the registry to that same service; stopping it stops both the node and its registry.

The public role carries public communities and needs no domain or registry. This bundle
deploys node roles, the registry, and optional Caddy HTTPS. The binary is included, so a
public-node installation needs no release download or build tools.

Allow inbound UDP 7843, 7844, 7846 and 7847 in your provider's firewall/security group. The installer
opens these ports in active ufw/firewalld on the host. If the public IP is mapped through
NAT, pass `--rtc-advertise PUBLIC_IP:7844` (IPv6: `[ADDRESS]:7844`).

## Stop, start, status and logs

These commands use SSH directly; they work even without a built bundle and upload nothing:

```sh
node deploy.mjs user@host --action services
node deploy.mjs user@host --action status --service all
node deploy.mjs user@host --action stop
node deploy.mjs user@host --action start
node deploy.mjs user@host --action restart --service caddy
node deploy.mjs user@host --action logs --service node
node deploy.mjs user@host --action disable --service node
node deploy.mjs user@host --action enable --service node
```

`stop` preserves data and startup settings. `disable` stops the service and prevents
startup after reboot. `enable` enables startup and starts it. Status and the latest
80 log lines are read-only. `--service node` (the default) and `--service registry`
both select hoi-node; `caddy` selects the HTTPS service, affecting every site it serves.
`all` selects the installed hoi-node and Caddy services, never every service on the VPS.
You can also give an exact existing service name, such as `--service old-registry`.
These controls operate systemd services, not Docker containers.

## Start fresh

The menu offers keep/reset, with keep as the default. A reset requires typing the data
directory's name. The equivalent scripted command is:

```sh
node deploy.mjs user@host --role public --reset-data --confirm-reset hoi-public
node deploy.mjs user@host --role registry --registry-url https://names.example.org --tls caddy --reset-data --confirm-reset hoi-public
node deploy.mjs user@host --role private --reset-data --confirm-reset hoi-node
```

This permanently clears **the selected role's** stored keys, identities, records, media,
settings, suppressions and registry database before starting the new service. Public
and registry roles share `hoi-public`; home and private roles share `hoi-node`. The
other data directory, backups and Caddy's certificates/configuration are preserved.
Old node links and pairings may no longer work after keys are regenerated.

The installer validates the bundle and runs its checks before changing the service.
It verifies that the unit's StateDirectory matches the selected role, stops the node,
then uses [systemd's state cleanup](https://www.freedesktop.org/software/systemd/man/latest/systemctl.html#clean%20PATTERN%E2%80%A6)
before restarting it. A cleanup failure stops the installation. `--yes` never counts
as reset confirmation, and check mode cannot reset data.

## Saved settings

For `--role config`, settings go in server.conf, using plain key=value lines without shell quotes:

```ini
role=public
name=my public node
```

For a registry, use `role=registry`, `registry_url=https://names.example.org` and
`tls=caddy`. Point the domain at the server and allow TCP 80/443 in the provider firewall.
This option installs Caddy from the host's packages and needs internet access.
`--tls external` in deploy.mjs uses a proxy you manage yourself. Switching node roles
leaves any existing Caddy configuration in place; use the service controls as needed.

`bash install.sh --help` describes the bundle; `bash node-installer.sh --help` lists all
underlying installer options. Checksums cover the
binary and underlying installer; server.conf is editable. They detect corruption, not
the authenticity of an untrusted bundle.

```sh
sudo systemctl status hoi-node
sudo journalctl -u hoi-node -f
sudo cat /var/lib/hoi-public/node-link
sudo systemctl stop hoi-node
```
