# One installer for everything

`install.sh` (macOS, Linux) and `install.ps1` (Windows) install whatever a computer should run,
from the same release files the download page lists:

| Role | What it installs | Runs |
|---|---|---|
| `desktop` | the hoi app; it opens as the companion in the tray, with the full app a click away | when you open it |
| `companion` | the same app, registered to start in the tray at login (`--start-at-login`, the companion's own switch) | whenever you log in |
| `home` | `hoi-node`: a home node for your phone and browser that is a public relay too: it carries open communities for whoever asks, asks the router to forward its ports, passes on the public nodes it reached and has a link to share | as a service |
| `relay` | the same as `home`: the name that role had before every home node was a public relay | as a service |
| `private` | `hoi-node --no-public-relay`: a home node for your own devices and your communities' members only | as a service |
| `public` | `hoi-node --public`: a light node with no key that carries open communities and passes on other public nodes | as a service |
| `registry` | `hoi-node --public --registry`: a public node with the public-name API, behind your TLS proxy | as a service |
| `uninstall` | removes the node service; the data stays | |
| `config` | nothing here: a file describing a `public` node or a `registry` for another computer, under a random name the server also takes as its host name ([below](#a-node-for-another-computer)) | on the server, with `--config` |
| `check` | nothing: looks at this computer and says whether the node can be installed, and what would stop it ([below](#before-it-installs-the-check)) | |

The **node setup app** ([apps/setup](../../apps/setup/README.md), on the download page under
*Before you run a public node*) runs this same `install.sh` on a server over SSH, or on this
computer, with a wizard: `config`, `check` and the install, every command shown as it runs.

The site serves both files at its root (`scripts/assemble-static.mjs` copies them into `dist/`):

```sh
curl -fsSL https://hoi.sh/install.sh | bash                                    # asks what to install
curl -fsSL https://hoi.sh/install.sh | bash -s -- home --name "lena's pi"
curl -fsSL https://hoi.sh/install.sh | bash -s -- registry --registry-url https://names.example.org --tls caddy
curl -fsSL https://hoi.sh/install.sh | bash -s -- check public                  # is this computer ready?
```

```powershell
irm https://hoi.sh/install.ps1 | iex
& ([scriptblock]::Create((irm https://hoi.sh/install.ps1))) -Role companion
```

From a checkout, run `deploy/install/install.sh` (or `.\deploy\install\install.ps1`) with the same
arguments; `--help` lists every option.

**Running a public node means answering for other people's data**: an interactive
`public` or `registry` install says so first and asks you to acknowledge it (`--yes`, as a kit
or a script runs it, means you read it elsewhere: `hoi.sh/download/#operator`). Your **join
notice** (what the node keeps, for how long and, optional but recommended, who runs it, where and how
to reach you; whatever you fill in is shown to everyone who connects) goes with
`--notice-file FILE` (`-NoticeFile` on Windows): a JSON file the app's *A node on a server* page
and the node setup app write, checked with `hoi-node admin notice check` before anything
changes, copied to `/etc/hoi-node/notice.json` (`~/Library/Application Support/hoi-node/notice.json`
on a Mac, `%LOCALAPPDATA%\hoi-node\notice.json` on Windows) and given to the node with
`--notice`. The node carries it in its link and in its first word to every stranger, so people
read it before it serves them; edit the file on the server and the node reads it again within half
a minute. A configuration file names it with `notice_file=` (next to the file unless a full
path). Without one, a public node or registry installs with a **warn** row and says so at start. `deploy/home/install.sh` is this installer with the role
`home` and a build from the checkout. Nothing it installs names a node or a registry: `--node
LINK` gives a node a public node to look at first and `--index URL` a registry to announce to,
both optional.

## A node for another computer

To build the server from your checkout and include its binary, use `pnpm build:server`,
then `pnpm deploy:server user@host` to upload and install over SSH. You can also upload
the generated archive yourself. See the [server bundle guide](../server/README.md#build-a-bundle-and-install-over-ssh).

The app makes it for you: **You → A node on a server** (`#/server`) picks a public node or a
registry, rolls a random name such as `sleepy-walrus-48213`, and hands you one file,
`hoi-sleepy-walrus-48213.tar.gz`, made on your device and sent nowhere. Inside:

| File | |
|---|---|
| `hoi-NAME.conf` | the configuration below |
| `hoi-NAME.notice.json` | your join notice, written on the page with a preview of how people read it |
| `install.sh` | this installer, as the app was built with it |
| `README.md` | the steps, the server's needs and what the check looks at |
| `cloud-init.yaml` | the same install as user data for a server not ordered yet: it writes the configuration and runs the installer on the first boot. A few hundred bytes, under every provider's limit (AWS: 16 KB), so it fetches the installer from the site that served the app rather than carrying it |
| `Caddyfile` | a registry behind a Caddy you run yourself (without `--tls caddy`) |

Then, on the server, one of three ways: unpack the kit and run `./install.sh --config
hoi-NAME.conf` (after `./install.sh check --config hoi-NAME.conf` if you like); paste
`cloud-init.yaml` into the provider's *user data* field when you order the server; or run the
one-line `curl … | bash -s -- registry --name … --hostname … --registry-url … --tls caddy` the page
shows, which installs the same node with no file at all.

`config` writes the same configuration from a terminal, on your home node or any computer:

```sh
deploy/install/install.sh config registry --domain example.org --tls caddy   # here: writes hoi-sleepy-walrus-48213.conf
scp hoi-sleepy-walrus-48213.conf root@203.0.113.10:
curl -fsSL https://hoi.sh/install.sh | bash -s -- check --config hoi-sleepy-walrus-48213.conf   # there: ready?
curl -fsSL https://hoi.sh/install.sh | bash -s -- --config hoi-sleepy-walrus-48213.conf         # there
```

The name is the node's label and the server's host name; with `--domain` the registry's address
is `https://NAME.DOMAIN`, so point DNS for it at the server first. Run it again for another name,
or pick one with `--name`. It is a label, not a secret: about 368 million names, and the node's
key, made on the server, is what tells nodes apart. Nothing of the computer that wrote the file
goes in it (no name, address or node link), and nothing on that computer changes; on Windows,
run it in Git Bash or WSL.

The file is `key=value` lines (`role`, `name`, `hostname`, `notice_file`, `registry_url`, `index`,
`daily_cap_gb`, `hold_pieces_gb`, `tls`), read and checked like flags, never run. Flags given on the server win over
it; `--hostname NAME` gives a server its name without a file.
`--hold-pieces-gb N` (`hold_pieces_gb=N`, `-HoldPiecesGb N` on Windows) lets a public node keep up
to N gigabytes of encrypted pieces and private communities' mailboxes for other people's devices
(`hoi-node --hold-pieces`): private communities sync through such nodes without any relay, their
devices find them by themselves, and the node can read none of it. Its notice then carries a fixed
sentence saying so, which the node adds itself; the check warns when the disk is short. On (2 GB)
for a public node or registry unless given here (`--hold-pieces-gb 0` for none), as `hoi-node`
does for every node that serves strangers.
`--config` sets the host name with `hostnamectl`, puts it in `/etc/hosts` and, on cloud images,
tells cloud-init to keep it (`/etc/cloud/cloud.cfg.d/99-hoi-hostname.cfg`); delete the `hostname`
line to leave the server's name alone. A Mac keeps its own name.

## Before it installs: the check

Every node install starts by looking at the computer, the way kubeadm's preflight does, and prints
one line per check: **ok**, **info**, **warn** (it installs, but something won't work as well) or
**BLOCK** (installing would fail, or leave a node nobody can reach). On a BLOCK it stops before
anything on the computer changes and says so; `--skip-checks` installs anyway, and a `--dry-run`
goes on to show what it would do. `install.sh check` (with a role, or `--config FILE`, and the
same flags) runs only the check and exits 1 on a BLOCK, 0 otherwise.

```text
Checking this computer for a hoi public node and registry called sleepy-walrus-48213:
  ok     system      Linux x86_64 (Debian GNU/Linux 12 (bookworm)), glibc 2.36
  ok     services    systemd 252
  ok     rights      root
  ok     source      hoi 0.2.0, hoi-node_0.2.0_x86_64-unknown-linux-gnu.tar.gz
  ok     disk        23.4 GB free in /var/lib
  ok     memory      1.9 GB
  ok     clock       synchronized
  ok     ports       UDP 7843 and 7844, TCP 7845 free
  info   firewall    ufw is on: the install opens UDP 7843, 7844 and TCP 80, 443 in it
  info   firewall    a firewall in front of the server (your provider's, a security group) has to let UDP 7843, 7844 and TCP 80, 443 in too
  ok     address     public: 203.0.113.10
  ok     pieces      keeps up to 2 GB of encrypted pieces and private communities' mailboxes for others; it can read none of them, and its notice says so
  warn   dns         sleepy-walrus-48213.example.org has no DNS record yet: add an A record (and AAAA for IPv6) pointing at this server; Caddy keeps asking for its certificate until it does
  ok     https       Caddy comes from this system's packages and serves https://sleepy-walrus-48213.example.org with a certificate of its own
  info   host        this computer becomes sleepy-walrus-48213 (now: debian-s-1vcpu-1gb-fra1-01)
Ready to install, with 1 warning above.
```

| Check | BLOCK when | warn when |
|---|---|---|
| system | a 32-bit or unknown processor; musl (Alpine) or a glibc older than 2.35 for the release's hoi-node (Ubuntu 22.04 builds it; `--source` builds it here) | |
| services, rights | no systemd running (a container: use `deploy/server`); neither root nor sudo; on a Mac, running it with sudo | |
| source | the release list can't be fetched or its signature doesn't verify; no hoi-node for this computer in it; the archive isn't in `--from`; no curl or wget, tar, sha256sum | |
| disk, memory, clock | under 512 MB free in `/var/lib` | under 2 GB free; under 512 MB of memory; the clock not synchronized |
| ports | UDP 7843 or 7844, or the registry's TCP port, taken by another program (a hoi-node installed here is replaced) | |
| firewall | | ufw or firewalld is on and `--no-firewall` keeps the node's ports shut; otherwise the install opens them (`ufw allow`, `firewall-cmd --permanent --add-port`) |
| address | | a public or registry node with only private addresses and no `--rtc-advertise` (behind NAT, or a cloud server whose public address is mapped to it) |
| dns | | a registry's host name has no record, or one that isn't this computer's address |
| https | `--tls caddy` on a Mac, with Caddy missing from the system's packages, or with TCP 80 or 443 taken by another web server | Caddy may be missing from the packages (an apt system that isn't Debian or Ubuntu) |
| notice | | a public node or registry without `--notice-file`: people who connect are not told who runs it (the file itself is checked with `hoi-node admin notice check` once the binary is there, and a bad one stops the install); a notice that names no operator is fine, and the row says what people read then |

Nothing in the check changes the computer or asks for a password; firewall rules that only root
can read are read through sudo when it needs none, and otherwise reported as unknown. The only
outside lookups are DNS for the registry's name and the release list the install fetches anyway:
no service is asked for this computer's public address.

## HTTPS for a registry: `--tls caddy`

Browsers reach a registry only over https. With `--tls caddy` (`tls=caddy` in a configuration)
the installer installs Caddy from the system's packages (Debian 12, Ubuntu 24.04, Fedora; it waits
for apt's lock, which a server's own updates hold on its first boot), writes the registry's site
to `/etc/caddy/hoi-registry.caddy` and imports it from `/etc/caddy/Caddyfile` (the packaged
example is set aside as `Caddyfile.before-hoi`; a Caddyfile of your own gets one import line, and
is put back if Caddy refuses the result), then reloads Caddy. Caddy gets and renews the
certificate by itself once DNS for the name leads to the server, and keeps no access log. It needs
the registry at the root of a host name of its own (`https://names.example.org`), and TCP 80 and
443, which an active ufw or firewalld is opened for. Without it, bring your own TLS proxy to
`127.0.0.1:7845`.

## Where the files come from

By default the installer reads the release list at `https://hoi.sh/releases/manifest.json`, the
signed manifest the release workflow or `scripts/release-local.mjs` makes, picks the file for this
computer (the universal `.dmg` on a Mac; a `.deb`, `.rpm` or AppImage on Linux, by what the system
installs; the setup `.exe` on Windows; the `hoi-node` archive for this CPU and system) and refuses
it unless its SHA-256 matches the list. When the project's minisign key is set in the installers
(`RELEASE_PUBKEY`, `-PubKey`) and `minisign` is installed, the list's signature is checked too;
until the key is published the installer says that it checked SHA-256 only. Fetched over a pipe
from hoi.sh, the installer and the list come from the same place, so the checksums are what
binds the downloaded files to the list hoi.sh published; the signature matters most when the
installer comes from a checkout or another copy.

Other sources: `--manifest URL` (another list), `--release TAG` (a GitHub release, fetched with a
signed-in `gh`, which is what a private repository needs), `--from DIR` (a release folder such as
`dist/release/v0.2.0` from `scripts/release-local.mjs`, copied to a computer with no internet if
need be), and for the node roles `--source` (build hoi-node from the checkout) or `--binary PATH`.
Tag downloads default to `hoi-project/hoi`, including inside a private checkout. Use `--repo OWNER/NAME`
(PowerShell: `-Repo OWNER/NAME`) to select a fork or private release explicitly.

A release carries the installers beside its files, so a folder of them installs as it is, the way
k3s and others take their files from beside the script: download `install.sh`, `manifest.json`,
`manifest.json.minisig` and the hoi-node archive for the server from the GitHub release (or put
them into an unpacked server kit), copy the folder over and run `./install.sh` there. With
`manifest.json` beside it and no source named, the installer uses that folder, offline, checking
each file against the list as always.
Release files are fetched from the addresses in the list: GitHub's by default, or wherever
`HOI_RELEASE_BASE_URL` put them when the list was made (`docs/RELEASES.md`).

## What each system gets

- **macOS.** The app goes to `/Applications` (or `~/Applications`), replacing a running hoi after
  asking it to quit. Nodes run as a launchd agent in your account from
  `~/Library/Application Support/hoi-node`, logging to `~/Library/Logs/hoi-node.log`.
- **Linux.** The app comes from the `.deb` (apt) or `.rpm` (dnf, zypper), asking for sudo, or as an
  AppImage in `~/.local/bin` with a menu entry. Nodes run as the systemd service `hoi-node`
  (sandboxed, `DynamicUser`), data in `/var/lib/hoi-node`, or `/var/lib/hoi-public` for a public
  node or registry, so a person's key and a public node's data never share a directory.
- **Windows.** The app installs for your account with its setup (`/S`, no administrator). Nodes run
  from `%LOCALAPPDATA%\hoi-node` as the scheduled task `hoi-node` at logon, logging to
  `hoi-node.log` there. Windows asks once whether `hoi-node` may use the network.

Nodes listen on UDP 7843 (native peers) and 7844 (browsers and phones), and public nodes on UDP
7846 and 7847 for the DHT among public nodes and its answers. `home`, `public` and
`registry` ask the router to forward them (`--no-port-mapping` to skip; a computer with a public
address of its own is not asked). On Linux an active ufw or firewalld is opened for them (and for
TCP 80 and 443 with `--tls caddy`); `--no-firewall` leaves it as it is. A registry listens on `127.0.0.1:7845` for your TLS proxy, allows
the hoi.sh and desktop origins unless you name others, and announces itself to its own registry.
Every role but `private` writes its node link to `node-link` in its data directory and says how to
read it. See [deploy/home](../home/README.md) for what running a public relay at home means.

Linux node installs preserve existing data unless `--reset-data` is requested and confirmed.
Use `--confirm-reset hoi-public` for a public node or registry, or `--confirm-reset hoi-node`
for a home/private node; without the matching flag, an interactive install asks you to type
that name. `--yes` does not confirm deletion. Reset clears only the selected role's data,
including keys and any registry database, after stopping the service; Caddy and other data
directories are preserved. Check mode refuses reset. The
[server bundle](../server/bundle/README.md#start-fresh) offers this choice in its deployment menu.

`install.sh` is tested by `scripts/install.test.mjs` (`pnpm test`); `install.ps1` by the
`installers` suite of the manual **long tests** workflow on Windows.
