# Field testing on real networks

The automated suites run on one machine or on isolated test networks. They cannot show how hoi
behaves on real home connections, mobile networks, VPNs and servers, over days. This guide is for
volunteers who run hoi across real networks and report what happened. Post results on
[#8](https://github.com/hoi-project/hoi/issues/8) and file each failure as its own `bug`.

A run is useful when someone else can tell what was tested and what was seen. A failed run,
written down, is as useful as a passing one. A run on one computer or with browser mocks
does not count as a field result: say so if that is what you did.

## Never record or post

Your report is public. Leave these out of reports, screenshots and attached logs:

- **IP addresses**, yours or anyone's. This includes the app's **Connection IP** line, **Your
  advertised addresses** and **Connection details**. Write "public IPv6", "private IPv4" or
  "matched", never the value.
- **Links**: invites and node links pack addresses, and pairing links are credentials.
- **Backups, passwords, recovery codes, keys** and the node's data directory.
- **Names and places**: people, providers, cities. Describe a network by its kind: "home
  connection behind a router", "mobile data", "commercial VPN", "rented server".

hoi-node never logs peer addresses, but your system, router, proxy or VPN may. Read a log
before you attach it.

## Before you start

- **Use a build you can name.** Note `hoi-node --version`, the app's version, and the SHA-256 of
  each download you installed. A build from source: note the commit. Use one version for the
  whole run. If you update during it, note when.
- **Use test identities and a test community only.** Preview releases can lose data.
- **Give each device a role name**: "node A", "node B", "browser C on a phone".
- **Keep the clock right** on every device, and write times in UTC.

## Setup

You need at least two native nodes on **different networks**: not behind the same router, and
not on the same LAN. A native node is `hoi-node` or the desktop app. At least one of them should
be reachable from the internet: a home node whose router forwards its ports, or a rented server.
Add a browser or phone on a third network, for example mobile data.

1. Install node A. Use [a home node](../deploy/home/README.md) or
   [a node on a server](../deploy/server/README.md), and check its log for the node link and
   the port-mapping result.
2. Pair a browser with node A, create an open test community and make an invite.
3. Have node B join: open the invite in the desktop app, or start `hoi-node --join INVITE`
   ([flags](../core/crates/hoi-node/README.md)).
4. Pair a second browser or phone with node B.
5. On every device, check **You → Warmth** for the test community, and **Connection details**
   for the peers each device sees.

## Scenarios

For each scenario, note what you did, what you expected, what you saw, and the time in UTC.

| # | Do | Expect |
| --- | --- | --- |
| 1 | Post and reply in both directions, one post with a picture. | Each arrives on the other side and renders. Note roughly how long it took. |
| 2 | Stop node B. Post on node A. Start node B again. | B catches up, with no duplicate posts. |
| 3 | Move a device to another network: Wi-Fi to mobile data, VPN on and off, IPv4-only to IPv6. | It reconnects. The **Connection IP** line shows what the new connections report: public or private, IPv4 or IPv6. |
| 4 | Compare what the **Connection IP** line says a node sees with the exit address you expect: your VPN app's exit address, or **Browser public IP** for a browser. | They match, unless that connection takes another route (a native connection can bypass the browser's path). Record "matched" or "did not match" and the route, never the addresses. |
| 5 | Block direct paths between two nodes, and give both an explicit relay (`--relay`; [packet relay](../core/crates/hoi-node/README.md#run-a-packet-relay-optional)). | They connect, and **Connection details** shows the route as `relay`, not `direct`. |
| 6 | Close the inviter's browser and node. Open the invite in a fresh browser. | The browser joins and catches up from another member's node. |
| 7 | Restart node A with `--no-relay` and reconnect a browser that isn't paired with it. Then restart it with `--daily-cap-gb 0.1` and use more than 100 MB through it, for example by opening pictures. | With relaying off the browser is turned away, while A's own paired devices still connect. With the cap it is served until the cap is used, then turned away until the next UTC day. |
| 8 | Delete a post (post menu → **Delete**) and follow its deletion status. | The status completes, and the post disappears on the other nodes after they catch up. |
| 9 | In a fresh browser, open only node A's node link. | It lists the open communities node A carries. If node B is also a public node and A reached it, the browser can hear of B through A. |

After scenario 7, restart node A with its usual flags. The daily cap is kept in the node's
settings, so set your usual `--daily-cap-gb` again.

## The seven-day run

This is the run [#8](https://github.com/hoi-project/hoi/issues/8) asks for.

- **Nodes:** at least three native nodes on three different networks, running for seven
  consecutive days on one software version.
- **Every day:**
  - Post from a different device.
  - Check the post arrived on every node and on one browser.
  - Note the **Warmth** reading ([what it means](spec/PROTOCOL.md#20-warmth-how-many-copies-a-node-can-see)).
- **At least once during the week:**
  - Restart each node.
  - Move one device to another network.
  - Delete one post and confirm the deletion reached the other nodes.
- **Whenever something goes wrong**, note what and when: a reconnect, a post that did not
  arrive, a node that stopped. Each failure is worth its own `bug`.

## Report

Post one comment per run on [#8](https://github.com/hoi-project/hoi/issues/8):

```markdown
**Software:** hoi-node 0.1.0 (archive SHA-256 …), desktop app 0.1.0, Firefox 153 on Linux
**Dates (UTC):** 2026-12-01 09:00 to 2026-12-08 09:00
**Devices:**
- node A: hoi-node on a home connection behind a router, ports forwarded
- node B: desktop app on a home connection, no forwarding
- node C: hoi-node on a rented server
- browser D: phone on mobile data, paired with A

| # | Expected | Observed | Result | Issue |
| --- | --- | --- | --- | --- |
| 1 | Post and reply arrive both ways | Arrived within seconds; picture rendered | pass | |
| 3 | Reconnects after Wi-Fi → mobile data | Reconnected after about a minute | pass | |

**Seven days:** what was posted, what arrived, the Warmth readings, every reconnect and
failure with its time, and how each ended.
```

Say what you could not run, and why.
