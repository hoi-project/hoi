# A home node at home, a public relay too

The home node is what you get when you want to post: a Raspberry Pi, a Mac mini, an old laptop
or a NAS that stays on (or the desktop app, which is public only once you turn on **Be a public
node**). It serves your own phone and browser, and, unless started with `--no-public-relay`, it
is a public node too:

- **carries open communities** for anyone who asks (the app's *Ask public nodes to keep it
  available*, and a founder's browser by itself for the communities it makes), and every open
  community a registry lists if you give it one;
- **asks your router to forward its UDP ports** (UPnP, NAT-PMP or PCP), so browsers, phones and
  other nodes can reach it from outside your home, and learns your public address from the router;
- **tells whoever connects about the other public nodes it reached**, and has a **link** to share
  (`node-link` in its data directory): anyone who opens it finds the communities it carries, and
  the next nodes along. No registry is needed.

That is plain `hoi-node`; `--no-public-relay` keeps it to your devices and your communities'
members. The same carrying rules as on a public node apply ([protocol §10](../../docs/spec/PROTOCOL.md#10-discovery),
carry requests): open communities only, bounded by `--carry-limit` and `--carry-mb`, what nobody else
touched for two weeks is let go and deleted, and nothing about who asked is kept.

## Before you start

- **What you share.** Whoever connects, and whoever its node link or other public nodes lead
  there, learns your home's public IP address, the open communities your node carries, and which
  of theirs they share with it; a registry, if you give one with `--index`, lists the address and
  the keys of the open communities your node holds, your own open ones included. Your node stores other people's
  public posts and media and can read them. You decide what it stops serving
  ([operator tools](../../core/crates/hoi-node/README.md#stop-serving-something-operator-tools)).
- **Bandwidth.** Relaying for others stops for the day after `--daily-cap-gb` (5 GB by default).
- **Only through a VPN, if you prefer.** `--extra "--vpn-only wg0"` (your VPN's interface) binds
  every connection to the internet to the VPN and asks only the VPN's own gateway to forward
  ports; the node waits for the VPN at start and stops when the VPN stops carrying the
  computer's traffic, and the service starts it again to wait. Being public through a VPN needs
  a VPN that forwards ports over NAT-PMP or PCP; most don't. `--no-home-network` keeps even the
  home network out. The log says which address goes to whom
  (see [node options](../../core/crates/hoi-node/README.md)).
- **A public IPv4 address.** If your provider puts you behind carrier-grade NAT (common on
  mobile and some fibre or cable lines), the router's own address is not public and nobody
  outside can reach you this way; the node says so in its log. Ask your provider for a public
  IPv4 address, or run the node on a small VPS instead ([deploy/server](../server/README.md)).
- **A 64-bit system.** On a Raspberry Pi 3, 4 or 5 that is Raspberry Pi OS (64-bit).

## Install on Linux or a Raspberry Pi (systemd)

With a published release, one command fetches, checks and installs it ([deploy/install](../install/README.md)):

```sh
curl -fsSL https://hoi.sh/install.sh | bash -s -- home --name "lena's pi"
```

From a checkout of this repository, with Rust installed (`https://rustup.rs`; the first build
on a Raspberry Pi 4 takes up to an hour):

```sh
deploy/home/install.sh --name "lena's pi"
```

Add `--node LINK` to give it a public node to look at first, and `--index URL` to also announce
it to a registry your community uses; neither is needed. Without Rust, use a release build: download
`hoi-node_<version>_aarch64-unknown-linux-gnu.tar.gz` (a Pi) or `…_x86_64-unknown-linux-gnu.tar.gz`
and pass it with `--binary`, or let the script fetch it with the GitHub CLI: `--release v0.2.0`.
A build you made yourself on another computer works the same way (`docs/RELEASES.md`,
*Build a release on your own computer*).

`deploy/home/install.sh` is [the general installer](../install/README.md) with the role `home`
and, unless you name a release, a build from the checkout. It installs `/usr/local/bin/hoi-node` and a systemd service with its data in
`/var/lib/hoi-node`, started now and with the computer. Then:

```sh
journalctl -u hoi-node -f                  # look for "port mapping: the router … forwards" and "node link"
sudo cat /var/lib/hoi-node/pairing-link    # open the https:// link on your phone to pair it
sudo cat /var/lib/hoi-node/node-link       # share this one: it finds the communities your node carries
```

## Install on a Mac (launchd)

```sh
deploy/home/install.sh --name "studio mac mini"
```

Run it as yourself, not with `sudo`. The node runs in your account from
`~/Library/Application Support/hoi-node`, starts when you log in and logs to
`~/Library/Logs/hoi-node.log`. For a Mac that serves around the clock, turn on automatic login
(System Settings → Users & Groups) and keep it from sleeping (System Settings → Energy, or
`sudo pmset -a sleep 0`). If the macOS firewall asks whether `hoi-node` may accept incoming
connections, allow it.

## Run it in Docker (a NAS, or a Pi with Docker)

```sh
cp deploy/home/.env.example deploy/home/.env        # set HOI_NODE_NAME (optional)
docker compose -f deploy/home/compose.yml up -d --build
docker compose -f deploy/home/compose.yml logs -f
docker compose -f deploy/home/compose.yml exec hoi-node cat /data/pairing-link
docker compose -f deploy/home/compose.yml exec hoi-node cat /data/node-link
```

The container uses the host's network, which the router search needs; only UDP 7843 and 7844
face outward, and the paired-device listener stays on the host's loopback.

## When the router doesn't forward

The log says which case it is:

- *the router answered no PCP, NAT-PMP or UPnP request*: turn on UPnP (sometimes called
  "automatic port forwarding" or "allow devices to open ports") or NAT-PMP in the router's
  settings and restart the node; or forward **UDP 7843 and 7844** to this computer by hand and
  tell the node your public address:

  ```sh
  deploy/home/install.sh --no-port-mapping --extra "--rtc-advertise 203.0.113.7:7844"
  ```

  (203.0.113.7 stands for your public IP; a manual forward needs updating when your provider
  changes that address, which the router-asked forward follows by itself.)
- *the router refused*: the router speaks the protocol but said no; check its settings for
  limits on which devices may open ports.
- *the router forwards from 100.64.x.x, which is not a public address*: carrier-grade NAT or a
  second router in front of yours (see *A public IPv4 address* above). With a second router you
  control, forward UDP 7843 and 7844 on the outer one and use `--rtc-advertise`.

IPv6 needs no forward: when your computer has a public IPv6 address and the router lets
inbound UDP through to it, that address is announced too.

## Update, change, remove

Run the script again with the same or new options: it replaces the binary and the service and
keeps the data (`--release`, `--binary` or a fresh `git pull` and build). Remove the service with
`deploy/home/install.sh --uninstall`; the data stays until you delete it. Keep it to your own
devices with `--no-public-relay`.

Every flag is in `hoi-node --help` and the [hoi-node guide](../../core/crates/hoi-node/README.md).
