//! `hoi-node`: the hoi core without a window, in one of three roles that compose:
//!
//! - a **home node** (the default): runs on a spare computer, a Raspberry Pi or a small VPS you
//!   rent yourself, keeps your communities warm, and serves your phone and browser. It is a
//!   **public relay** as well, unless started with `--no-public-relay`: it carries open
//!   communities for anyone who asks, has the router forward its ports (`portmap.rs`), tells
//!   whoever connects about the other public nodes it reached (hoi-core `nodes`) and has a link
//!   anyone can open to find what it carries (`DIR/node-link`);
//! - a **public node** (`--public`): the same, with no person behind it and no pairing link;
//! - a **registry node** (`--registry ADDR`): the public-name API, one namespace, the same API
//!   and database shape as the Node service and the Worker, so anyone can run one locally.
//!
//! Nothing here names a node, a registry or a relay to start from: a node finds others through
//! the invites it joins, the node links it is given (`--node`) and the nodes that reach it.
//!
//! It keeps no logs of who connects: it prints its own status and nothing about peers. Its
//! operator's tools (`hoi-node admin`) work on the data directory from a shell.
//!
//! ```text
//! hoi-node --data ~/.local/share/hoi-node --listen 127.0.0.1:7842 --name "lena's pi"
//! hoi-node --data ~/.local/share/hoi-node --name "lena's pi" --node 'https://hoi.sh/app/#/node?code=…'
//! hoi-node --data ~/.local/share/hoi-node --no-public-relay
//! hoi-node --data ~/hoi-public --public --rtc-advertise 203.0.113.10:7844 --index https://registry.example
//! hoi-node --data ~/hoi-registry --public --registry 0.0.0.0:7845 --registry-url https://registry.example --registry-origin https://hoi.sh
//! hoi-node admin --data ~/hoi-public suppress community KEY --evidence ~/cases
//! ```
//!
//! Phones and browsers on the same network pair with a home node over WebRTC (UDP 7844 by
//! default): no certificate, no domain. Members' browsers can catch up from it too while
//! relaying is on. With `--index` a node is also a carrier for that registry: it tells it where
//! it can be reached, carries every open community it lists, and takes a newly published
//! community straight from its founder's browser (see `index.rs`). No registry is needed.

use std::net::SocketAddr;
use std::path::PathBuf;

use hoi_core::Node;
use hoi_net::Shared;

mod admin;
mod http;
mod index;
mod packet_relay;
mod registry;
mod routes;
mod trackers;

/// Where a home node keeps its pairing link: the link is a credential, so it goes to a file only
/// this user can read rather than to a log (`--show-pairing` prints it).
const PAIRING_FILE: &str = "pairing-link";
/// Where a public node keeps the link anyone may open to reach it. Not a credential: it is
/// printed too, and meant to be shared.
const NODE_LINK_FILE: &str = "node-link";
/// The hash of the operator's notice as the node carries it now (`--notice`), for installers.
const NOTICE_HASH_FILE: &str = "notice-hash";
/// How often `--notice FILE` is looked at for a change.
const NOTICE_POLL: std::time::Duration = std::time::Duration::from_secs(30);

/// What a node carries beyond its own person's communities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CarryMode {
    /// Nothing but what `--join` gave it.
    Off,
    /// What each `--index` lists, and what browsers offer under a listed key.
    Index,
    /// The above, and carry requests from connected peers for open communities.
    Any,
}

impl CarryMode {
    fn parse(value: &str) -> Option<CarryMode> {
        match value {
            "off" => Some(CarryMode::Off),
            "index" => Some(CarryMode::Index),
            "any" => Some(CarryMode::Any),
            _ => None,
        }
    }

    fn describe(self) -> &'static str {
        match self {
            CarryMode::Off => "off (only communities joined by invite)",
            CarryMode::Index => "index (what each --index registry lists)",
            CarryMode::Any => "any (carry requests from connected peers, and what each --index registry lists)",
        }
    }
}

struct RegistryArgs {
    listen: SocketAddr,
    origins: Vec<String>,
    public_nodes: Vec<String>,
    announcements: bool,
    app_url: String,
    release: Option<PathBuf>,
    limits: (u32, u32, u32),
    mirrors: Vec<String>,
    url: Option<String>,
    announcers: Vec<String>,
}

struct Args {
    data: PathBuf,
    listen: SocketAddr,
    name: String,
    new_pairing: bool,
    show_pairing: bool,
    peer_only: bool,
    public: bool,
    /// A home node is a public relay unless `--no-public-relay` says otherwise.
    public_relay: bool,
    port_mapping: Option<bool>,
    daily_cap_gb: Option<f64>,
    peer_bind: Option<SocketAddr>,
    relays: Vec<String>,
    public_url: Option<String>,
    joins: Vec<String>,
    rtc_port: Option<u16>,
    /// The embedded packet relay: where it listens, the address others reach it at,
    /// its certificate and key when it does TLS itself, and whether only members may use it.
    packet_relay: Option<SocketAddr>,
    packet_relay_url: Option<String>,
    packet_relay_cert: Option<PathBuf>,
    packet_relay_key: Option<PathBuf>,
    packet_relay_members: bool,
    /// The DHT among public nodes: on for public nodes unless `--no-dht`.
    dht: bool,
    /// LAN beacons: on unless `--no-lan`, and never with `--vpn-only`.
    lan: bool,
    dht_port: Option<u16>,
    dht_answer_port: Option<u16>,
    dht_bootstrap: Vec<String>,
    rtc_advertise: Vec<std::net::SocketAddr>,
    rtc_bind: Vec<std::net::IpAddr>,
    /// Only through this VPN interface.
    vpn_only: Option<String>,
    /// With `vpn_only`: the home network may still be used directly.
    home_network: bool,
    relay: Option<bool>,
    indexes: Vec<String>,
    carry: Option<CarryMode>,
    carry_limit: usize,
    carry_bytes: u64,
    refuse: Vec<String>,
    nodes: Vec<String>,
    /// The operator's notice, read again whenever the file changes.
    notice: Option<PathBuf>,
    /// Gigabytes of encrypted pieces to hold for scattered delivery, or none.
    hold_pieces_gb: Option<f64>,
    registry: Option<RegistryArgs>,
}

/// Communities a carrier takes on by default, those still waiting for a member included.
const DEFAULT_CARRY_LIMIT: usize = 256;
/// Megabytes the communities carried on peers' requests may hold together, by default.
const DEFAULT_CARRY_MB: u64 = hoi_core::carry::DEFAULT_CARRY_BYTES >> 20;

/// Gigabytes of pieces a node that serves strangers holds unless told otherwise.
const DEFAULT_HOLD_GB: f64 = 2.0;

fn usage() -> ! {
    eprintln!(
        "hoi-node {}

Usage: hoi-node [--data DIR] [--name NAME] [role flags] [transport flags]
       hoi-node admin [--data DIR] COMMAND   (the operator's tools; hoi-node admin --help)

Roles (they compose):
  (default)                a home node that is a public relay: serves your paired browser and
                           phone, relays (--relaying), carries open communities anyone asks it
                           to (--carry any), has the router forward its ports (--port-mapping),
                           tells whoever connects about the other public nodes it reached, and
                           writes a link anyone can open to DIR/node-link
  --no-public-relay        a home node for your own devices and your communities' members only:
                           carries nothing for strangers and asks the router for nothing
  --public                 a public node: the same as the default with no pairing listener and
                           no person's key
  --public-relay           the default; kept for service files written before it was
  --registry ADDR          a registry node: serve the public-name API on ADDR from DIR/registry.db

Home node:
  --listen ADDR            address for paired devices (default: 127.0.0.1:7842)
  --new-pairing            issue a new pairing link and print it; devices paired before must pair again
  --show-pairing           print the pairing link (otherwise it is only written to DIR/pairing-link)
  --public-url URL         advertise a wss:// TLS proxy address in pairing links
  --peer-only              run only peer transports; no paired-device control listener (--public implies it)

Operator notice (a public node tells people who runs it before it serves them):
  --notice FILE            the operator's notice as JSON (hoi-node admin notice check FILE shows
                           it as people read it, and its hash): carried in every hello to a
                           stranger and, signed, in DIR/node-link, so it is read before anyone
                           connects; read again when the file changes; its hash goes to
                           DIR/notice-hash. A public node without one says so when it starts.

Finding others (no node or registry is built in):
  --node LINK              a public node to look at first: its link (hoi://node?… or …#/node?…),
                           repeatable; invites (--join) and nodes that connect lead to others too
  --index URL              also announce this node's public routes to that registry and carry the
                           open communities it lists (repeatable: one registry per flag; optional)

Carrying:
  --carry MODE             off | index | any (default: any, except with --no-public-relay: index
                           with --index, else off)
  --carry-limit N          communities to carry at most (default: {})
  --carry-mb N             megabytes the communities carried on peers' requests may hold together;
                           over it, the ones nobody touched for longest are let go (default: {})
  --refuse KEY             never carry this community, whoever asks or lists it (repeatable); one
                           carried already is let go and deleted
  --join INVITE            join a community and remember its routes (repeatable)

Scattered delivery:
  --hold-pieces GB         hold up to GB gigabytes of other people's encrypted pieces in
                           DIR/pieces.db and answer sealed requests for them: this node can read
                           none of them, holds at most one piece of any post, and sees only the
                           forwarders requests come through (default: 2 on a node that serves
                           strangers; 0 turns it off)

Registry:
  --registry-url URL       this registry's public URL, as carriers are given it; announcements must
                           be signed for its origin (without it: only for this machine's loopback address)
  --registry-origin ORIGIN exact browser origin allowed to call the API (repeatable; tauri://localhost for the desktop app)
  --registry-public-node ROUTE
                           a route listed before any announced node (repeatable)
  --registry-no-announcements
                           list only those configured routes; refuse node announcements
  --registry-announcer KEY allowlist mode: take announcements only from these endpoint keys (repeatable)
  --registry-mirror URL    import another registry's verified names when they are free here (repeatable)
  --registry-release PATH  serve this release manifest instead of the compiled one (read on each request)
  --registry-app-url URL   the app address the registry points people at (default: https://hoi.sh/app/)
  --registry-limits R,C,A  per-minute budgets for reads, name claims and announcements (default: 600,30,120)

Transport:
  --data DIR               where records live (default: ./hoi-node-data)
  --name NAME              what your devices call this node, or a public node's label (default: home node)
  --peer-bind ADDR         bind the iroh UDP transport (default: automatic)
  --relay URL              explicit HTTPS iroh relay (repeatable; default: none)
  --packet-relay ADDR      run a packet relay (iroh's relay server) on this TCP address, e.g.
                           127.0.0.1:3340 behind a TLS proxy, or 0.0.0.0:443 with a certificate
  --packet-relay-url URL   the address others reach it at, https://… (put into this node's routes,
                           so invites name it; default for a relay on this machine: http://ADDR)
  --packet-relay-cert FILE, --packet-relay-key FILE
                           do TLS itself, with this PEM certificate chain and key
  --packet-relay-members   only for endpoints this node knows as members of what it carries
                           (otherwise anyone but those on the block list: hoi-node admin relay)
  --rtc-port PORT          UDP port for browsers and phones over WebRTC (default: 7844; 0 = off)
  --rtc-advertise ADDR     also advertise this UDP address, e.g. a forwarded port (repeatable)
  --dht-port PORT          UDP port of the DHT among public nodes (default: 7846); a public node
                           runs it and announces the open communities it carries there
  --dht-answer-port PORT   UDP port answering the DHT's hoi queries with signed routes (default: 7847)
  --dht-bootstrap ADDR     also start the DHT from this outside node, e.g. a BitTorrent router;
                           none by default: public nodes find each other's DHT nodes themselves
  --no-dht                 don't run the DHT
  --no-lan                 don't send or listen for beacons on the local network (UDP 7848
                           multicast; members of the same communities find each other with it)
  --rtc-bind IP            serve WebRTC only on this local address (repeatable; default: every
                           interface's), e.g. one address of a machine that runs several nodes
  --no-relay               don't serve members' browsers or accept their nodes' connections
  --relaying               serve them again (the default)
  --daily-cap-gb N         gigabytes a day this node may relay for others; over it, it stops until
                           the next day (default: 5)
  --port-mapping           ask the router (PCP, NAT-PMP or UPnP) to forward the WebRTC and native
                           UDP ports and announce its public address (the default, except with
                           --no-public-relay)
  --no-port-mapping        don't ask the router; forward ports by hand and use --rtc-advertise
  --vpn-only IFACE         reach the internet only through this VPN interface (wg0, tun0, …):
                           sockets are bound and pinned to it, nothing starts until the system
                           routes internet traffic through it, and the node stops when it no
                           longer does (a service manager starts it again, and it waits); port
                           mapping asks only the VPN's own gateway
  --no-home-network        with --vpn-only: not even the home network directly",
        hoi_core::VERSION,
        DEFAULT_CARRY_LIMIT,
        DEFAULT_CARRY_MB
    );
    std::process::exit(2)
}

fn args() -> Args {
    let mut a = Args {
        data: PathBuf::from("hoi-node-data"),
        listen: "127.0.0.1:7842".parse().expect("addr"),
        name: "home node".into(),
        new_pairing: false,
        show_pairing: false,
        peer_only: false,
        public: false,
        public_relay: false,
        port_mapping: None,
        daily_cap_gb: None,
        peer_bind: None,
        relays: Vec::new(),
        public_url: None,
        joins: Vec::new(),
        rtc_port: None,
        packet_relay: None,
        packet_relay_url: None,
        packet_relay_cert: None,
        packet_relay_key: None,
        packet_relay_members: false,
        dht: true,
        lan: true,
        dht_port: None,
        dht_answer_port: None,
        dht_bootstrap: Vec::new(),
        rtc_advertise: Vec::new(),
        rtc_bind: Vec::new(),
        vpn_only: None,
        home_network: true,
        relay: None,
        indexes: Vec::new(),
        carry: None,
        carry_limit: DEFAULT_CARRY_LIMIT,
        carry_bytes: DEFAULT_CARRY_MB << 20,
        refuse: Vec::new(),
        nodes: Vec::new(),
        notice: None,
        hold_pieces_gb: None,
        registry: None,
    };
    let mut registry = RegistryArgs {
        listen: "127.0.0.1:7845".parse().expect("addr"),
        origins: Vec::new(),
        public_nodes: Vec::new(),
        announcements: true,
        app_url: "https://hoi.sh/app/".into(),
        release: None,
        limits: (600, 30, 120),
        mirrors: Vec::new(),
        url: None,
        announcers: Vec::new(),
    };
    let mut registry_on = false;
    let mut public_relay = None;
    let mut it = std::env::args().skip(1);
    while let Some(x) = it.next() {
        match x.as_str() {
            "--data" => a.data = it.next().map(PathBuf::from).unwrap_or_else(|| usage()),
            "--listen" => a.listen = it.next().and_then(|s| s.parse().ok()).unwrap_or_else(|| usage()),
            "--name" => a.name = it.next().unwrap_or_else(|| usage()),
            "--new-pairing" => a.new_pairing = true,
            "--show-pairing" => a.show_pairing = true,
            "--peer-only" => a.peer_only = true,
            "--public" => {
                a.public = true;
                a.peer_only = true;
            }
            "--public-relay" => public_relay = Some(true),
            "--no-public-relay" => public_relay = Some(false),
            "--node" => {
                let value = it.next().unwrap_or_else(|| usage());
                if hoi_types::NodeLink::parse(&value).is_err() {
                    eprintln!("hoi-node: --node needs a node link (hoi://node?… or https://…#/node?…)");
                    usage();
                }
                a.nodes.push(value);
            }
            "--notice" => a.notice = Some(it.next().map(PathBuf::from).unwrap_or_else(|| usage())),
            "--port-mapping" => a.port_mapping = Some(true),
            "--no-port-mapping" => a.port_mapping = Some(false),
            "--daily-cap-gb" => {
                a.daily_cap_gb = Some(it.next().and_then(|s| s.parse::<f64>().ok()).filter(|n| (0.1..=1000.0).contains(n)).unwrap_or_else(|| {
                    eprintln!("hoi-node: --daily-cap-gb needs a number of gigabytes from 0.1 to 1000");
                    usage()
                }))
            }
            "--hold-pieces" => {
                a.hold_pieces_gb =
                    Some(it.next().and_then(|s| s.parse::<f64>().ok()).filter(|n| *n == 0.0 || (0.1..=10_000.0).contains(n)).unwrap_or_else(|| {
                        eprintln!("hoi-node: --hold-pieces needs a number of gigabytes from 0.1 to 10000, or 0 for none");
                        usage()
                    }))
            }
            "--peer-bind" => a.peer_bind = Some(it.next().and_then(|s| s.parse().ok()).unwrap_or_else(|| usage())),
            "--relay" => a.relays.push(it.next().unwrap_or_else(|| usage())),
            "--public-url" => {
                let value = it.next().unwrap_or_else(|| usage());
                if !valid_public_url(&value) {
                    usage();
                }
                a.public_url = Some(value);
            }
            "--join" => a.joins.push(it.next().unwrap_or_else(|| usage())),
            "--rtc-port" => a.rtc_port = Some(it.next().and_then(|s| s.parse().ok()).unwrap_or_else(|| usage())),
            "--no-dht" => a.dht = false,
            "--no-lan" => a.lan = false,
            "--packet-relay" => a.packet_relay = Some(it.next().and_then(|s| s.parse().ok()).unwrap_or_else(|| usage())),
            "--packet-relay-url" => a.packet_relay_url = Some(it.next().filter(|s| s.len() <= 512).unwrap_or_else(|| usage())),
            "--packet-relay-cert" => a.packet_relay_cert = Some(PathBuf::from(it.next().unwrap_or_else(|| usage()))),
            "--packet-relay-key" => a.packet_relay_key = Some(PathBuf::from(it.next().unwrap_or_else(|| usage()))),
            "--packet-relay-members" => a.packet_relay_members = true,
            "--dht-port" => a.dht_port = Some(it.next().and_then(|s| s.parse().ok()).filter(|p| *p != 0).unwrap_or_else(|| usage())),
            "--dht-answer-port" => a.dht_answer_port = Some(it.next().and_then(|s| s.parse().ok()).filter(|p| *p != 0).unwrap_or_else(|| usage())),
            "--dht-bootstrap" => a.dht_bootstrap.push(it.next().filter(|s| !s.is_empty() && s.len() <= 255).unwrap_or_else(|| usage())),
            "--rtc-advertise" => a.rtc_advertise.push(it.next().and_then(|s| s.parse().ok()).unwrap_or_else(|| usage())),
            "--rtc-bind" => a.rtc_bind.push(it.next().and_then(|s| s.parse().ok()).unwrap_or_else(|| usage())),
            "--vpn-only" => a.vpn_only = Some(it.next().filter(|s| !s.is_empty() && s.len() <= 64).unwrap_or_else(|| usage())),
            "--no-home-network" => a.home_network = false,
            "--no-relay" => a.relay = Some(false),
            "--relaying" => a.relay = Some(true),
            "--index" => {
                let value = it.next().unwrap_or_else(|| usage());
                let Some(url) = http::valid_index_url(&value) else {
                    eprintln!("hoi-node: --index needs an https:// registry URL (plain http only on this machine), without credentials, a query or a fragment");
                    usage();
                };
                if !a.indexes.contains(&url) {
                    a.indexes.push(url);
                }
            }
            "--carry" => a.carry = Some(it.next().and_then(|s| CarryMode::parse(&s)).unwrap_or_else(|| usage())),
            "--carry-limit" => a.carry_limit = it.next().and_then(|s| s.parse().ok()).filter(|n| *n >= 1).unwrap_or_else(|| usage()),
            "--carry-mb" => a.carry_bytes = it.next().and_then(|s| s.parse::<u64>().ok()).filter(|n| *n >= 1).unwrap_or_else(|| usage()) << 20,
            "--refuse" => {
                let key = it.next().unwrap_or_else(|| usage());
                if hoi_types::Id::from_hex(&key).is_err() {
                    eprintln!("hoi-node: --refuse needs a community key (64 hex characters)");
                    usage();
                }
                if !a.refuse.contains(&key) {
                    a.refuse.push(key);
                }
            }
            "--registry" => {
                registry.listen = it.next().and_then(|s| s.parse().ok()).unwrap_or_else(|| usage());
                registry_on = true;
            }
            "--registry-origin" => registry.origins.push(it.next().unwrap_or_else(|| usage())),
            "--registry-public-node" => registry.public_nodes.push(it.next().unwrap_or_else(|| usage())),
            "--registry-no-announcements" => registry.announcements = false,
            "--registry-url" => {
                let value = it.next().unwrap_or_else(|| usage());
                if let Err(e) = registry::protocol::registry_origin(&value) {
                    eprintln!("hoi-node: --registry-url: {e}");
                    usage();
                }
                registry.url = Some(value);
            }
            "--registry-announcer" => {
                let key = it.next().unwrap_or_else(|| usage());
                if !registry::protocol::key_is_strong(&key) {
                    eprintln!("hoi-node: --registry-announcer needs an endpoint key (64 lowercase hex characters)");
                    usage();
                }
                if !registry.announcers.contains(&key) {
                    registry.announcers.push(key);
                }
            }
            "--registry-mirror" => {
                let value = it.next().unwrap_or_else(|| usage());
                let Some(url) = http::valid_index_url(&value) else {
                    eprintln!(
                        "hoi-node: --registry-mirror needs an https:// registry URL (plain http only on this machine), without credentials, a query or a fragment"
                    );
                    usage();
                };
                if !registry.mirrors.contains(&url) {
                    registry.mirrors.push(url);
                }
            }
            "--registry-release" => registry.release = Some(it.next().map(PathBuf::from).unwrap_or_else(|| usage())),
            "--registry-app-url" => registry.app_url = it.next().unwrap_or_else(|| usage()),
            "--registry-limits" => {
                let value = it.next().unwrap_or_else(|| usage());
                let parts: Vec<u32> =
                    value.split(',').map(|p| p.trim().parse::<u32>().ok().filter(|n| *n > 0)).collect::<Option<_>>().unwrap_or_else(|| usage());
                let [reads, claims, announcements] = parts[..] else { usage() };
                registry.limits = (reads, claims, announcements);
            }
            "-h" | "--help" => usage(),
            "-V" | "--version" => {
                println!("hoi-node {}", hoi_core::VERSION);
                std::process::exit(0)
            }
            _ => usage(),
        }
    }
    // A public node has no person behind it, so it is no home node's public relay unless asked.
    a.public_relay = public_relay.unwrap_or(!a.public);
    if (a.public || a.public_relay) && a.relay.is_none() {
        a.relay = Some(true);
    }
    // A node that serves strangers holds pieces unless told not to (SCATTER.md S6); 0 is none.
    if a.hold_pieces_gb.is_none() && (a.public || a.public_relay) {
        a.hold_pieces_gb = Some(DEFAULT_HOLD_GB);
    }
    if a.hold_pieces_gb == Some(0.0) {
        a.hold_pieces_gb = None;
    }
    if registry_on {
        a.registry = Some(registry);
    } else if !registry.origins.is_empty()
        || !registry.public_nodes.is_empty()
        || !registry.mirrors.is_empty()
        || registry.release.is_some()
        || registry.url.is_some()
        || !registry.announcers.is_empty()
    {
        eprintln!("hoi-node: --registry-* flags need --registry ADDR");
        usage();
    }
    a
}

fn valid_public_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else { return false };
    url.scheme() == "wss"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
}

#[tokio::main]
async fn main() {
    if std::env::args().nth(1).as_deref() == Some("admin") {
        std::process::exit(admin::run(std::env::args().skip(2).collect()));
    }
    let a = args();
    let carry = a.carry.unwrap_or(if a.public || a.public_relay {
        CarryMode::Any
    } else if a.indexes.is_empty() {
        CarryMode::Off
    } else {
        CarryMode::Index
    });
    if let Err(e) = std::fs::create_dir_all(&a.data) {
        eprintln!("hoi-node: can't create {}: {e}", a.data.display());
        std::process::exit(1);
    }
    // One node per data directory, and `hoi-node admin` waits until it stops.
    let _lock = admin::lock(&a.data).unwrap_or_else(|e| {
        eprintln!("hoi-node: {e}");
        std::process::exit(1);
    });
    let db = match hoi_store::sqlite::Sqlite::open(&a.data.join("hoi.db")) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("hoi-node: can't open storage: {e}");
            std::process::exit(1);
        }
    };
    let mut node = match Node::open(Box::new(db), "node", "https://hoi.sh/app/", hoi_core::system_clock()) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("hoi-node: {e}");
            std::process::exit(1);
        }
    };
    // A home node signs as its person's device and needs a key. A public node has no person:
    // it stores and serves, never signs, so it gets a label and no key. A key already in its
    // data directory (an older carrier) is kept.
    if node.me().is_none() {
        if a.peer_only {
            node.set_node_name(Some(a.name.clone()));
        } else {
            node.create_identity(&a.name).expect("identity");
        }
    } else if node.me().is_some_and(|m| m.name != a.name) {
        let _ = node.set_name(&a.name);
    }
    if a.new_pairing {
        let _ = node.rotate_pairing();
    }
    if let Some(on) = a.relay {
        let _ = node.set_settings(&hoi_core::dto::SettingsPatch { relay: Some(on), relay_notice_seen: Some(true), ..Default::default() });
    }
    if let Some(gb) = a.daily_cap_gb {
        let _ = node.set_settings(&hoi_core::dto::SettingsPatch { daily_cap_gb: Some(gb), ..Default::default() });
    }
    // The core applies the policy on the link: with `any` it advertises `carry` in its hello
    // and takes communities connected peers hand it (hoi_core::carry).
    node.set_carry_policy(
        match carry {
            CarryMode::Off => hoi_core::CarryPolicy::Off,
            CarryMode::Index => hoi_core::CarryPolicy::Index,
            CarryMode::Any => hoi_core::CarryPolicy::Any,
        },
        a.carry_limit,
    );
    node.set_carry_bytes(a.carry_bytes);
    if let Some(gb) = a.hold_pieces_gb {
        match hoi_store::pieces::SqlitePieces::open(&a.data.join("pieces.db"), (gb * 1e9) as u64) {
            Ok(store) => node.set_piece_store(Some(Box::new(store))),
            Err(e) => {
                eprintln!("hoi-node: cannot open {}: {e}", a.data.join("pieces.db").display());
                std::process::exit(1);
            }
        }
    }
    if node.set_refused(&a.refuse).is_err() {
        eprintln!("hoi-node: --refuse needs community keys");
        std::process::exit(1);
    }
    let mut notice_hash = None;
    if let Some(path) = &a.notice {
        match load_notice(path).and_then(|n| node.set_notice(Some(n)).map_err(|e| e.message)) {
            Ok(hash) => notice_hash = hash,
            Err(e) => {
                eprintln!("hoi-node: --notice {}: {e}", path.display());
                std::process::exit(1);
            }
        }
    } else if a.public || a.registry.is_some() {
        eprintln!(
            "hoi-node: WARNING: this public node carries no operator notice (--notice FILE): people who connect are not told who runs it, where, or how to reach you. Write one with the installer or the app, or see https://hoi.sh/download/#operator"
        );
    }
    for invite in &a.joins {
        if node.join(invite).is_err() {
            eprintln!("hoi-node: could not join a community; check its invite");
            std::process::exit(1);
        }
    }
    for link in &a.nodes {
        if let Err(e) = node.open_node_link(link) {
            eprintln!("hoi-node: --node: {}", e.message);
            std::process::exit(1);
        }
    }
    // The registry opens its database before any socket, so a bad configuration stops here.
    let registry = a.registry.as_ref().map(|r| {
        let release = match &r.release {
            Some(path) => registry::release::ReleaseSource::file(path.clone()),
            None => registry::release::ReleaseSource::compiled(),
        };
        let release = release.unwrap_or_else(|e| {
            eprintln!("hoi-node: registry release manifest: {e}");
            std::process::exit(1);
        });
        let mut config = registry::RegistryConfig::new(release);
        config.origins = r.origins.clone();
        config.public_nodes = r.public_nodes.clone();
        config.announcements = r.announcements;
        config.app_url = r.app_url.clone();
        config.limits = r.limits;
        config.url = r.url.clone();
        config.announcers = r.announcers.clone();
        registry::Registry::open(&a.data, &config).unwrap_or_else(|e| {
            eprintln!("hoi-node: registry: {e}");
            std::process::exit(1);
        })
    });
    let shared = Shared::new(node, Box::new(|_| {}));
    // Only through a VPN: nothing starts until the system sends internet traffic through it,
    // and then everything is bound to it (hoi_net::vpn).
    let policy = hoi_net::vpn::NetPolicy { vpn: a.vpn_only.clone(), home_network: a.home_network };
    let mut peer_bind = a.peer_bind;
    if let Some(name) = &a.vpn_only {
        let vpn = wait_for_vpn(&policy, name).await;
        let Some(v4) = vpn.ipv4() else {
            eprintln!("hoi-node: {name} has no IPv4 address; the native transport needs one");
            std::process::exit(1);
        };
        peer_bind = Some(SocketAddr::new(v4.into(), a.peer_bind.map_or(0, |b| b.port())));
        http::bind_to(v4.into(), name);
        shared.with(|n| n.set_vpn_status(vpn.report(name)));
    }
    // the embedded packet relay, started first so this node's own routes name it
    let mut relays = a.relays.clone();
    let _packet_relay = match a.packet_relay {
        None => None,
        Some(listen) => {
            let cert = match (&a.packet_relay_cert, &a.packet_relay_key) {
                (Some(c), Some(k)) => Some((c.clone(), k.clone())),
                (None, None) => None,
                _ => {
                    eprintln!("hoi-node: --packet-relay-cert and --packet-relay-key go together");
                    std::process::exit(2);
                }
            };
            let url = a.packet_relay_url.clone().or_else(|| listen.ip().is_loopback().then(|| format!("http://{listen}")));
            let options = packet_relay::Options { listen, cert, members_only: a.packet_relay_members, data: a.data.clone() };
            match packet_relay::start(shared.clone(), options).await {
                Ok(server) => {
                    match &url {
                        Some(u) => {
                            println!(
                                "packet relay: on {listen}, reached at {u}; {}",
                                if a.packet_relay_members { "members of what this node carries only" } else { "anyone not refused (hoi-node admin relay)" }
                            );
                            relays.insert(0, u.clone());
                        }
                        None => println!("packet relay: on {listen}; give --packet-relay-url so invites can name it"),
                    }
                    Some(server)
                }
                Err(e) => {
                    eprintln!("hoi-node: {e}");
                    std::process::exit(1);
                }
            }
        }
    };
    let p2p = match hoi_net::start_p2p(shared.clone(), hoi_net::P2pConfig { bind: peer_bind, relays }).await {
        Ok(p2p) => p2p,
        Err(e) => {
            eprintln!("hoi-node: peer transport could not start: {e}");
            std::process::exit(1);
        }
    };
    let server = if a.peer_only {
        None
    } else {
        Some(match hoi_net::serve(shared.clone(), a.listen, &hoi_net::origins_from_env()).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("hoi-node: can't listen on {}: {e}", a.listen);
                std::process::exit(1);
            }
        })
    };
    if !a.peer_only
        && let Some(url) = &a.public_url
    {
        shared.with(|n| n.set_serving(Some(url))).expect("public pairing address");
    }
    let rtc = match a.rtc_port {
        Some(0) => None,
        port => match hoi_net::start_rtc(
            shared.clone(),
            hoi_net::RtcConfig { port: port.unwrap_or(0), advertise: a.rtc_advertise.clone(), bind: a.rtc_bind.clone(), policy: policy.clone() },
        )
        .await
        {
            Ok(h) => Some(h),
            Err(e) => {
                eprintln!("hoi-node: WebRTC for browsers and phones could not start: {e}");
                None
            }
        },
    };
    let _sweeper = hoi_net::spawn_sweeper(shared.clone());
    let st = shared.with(|n| n.net_status());
    let communities = shared.with(|n| n.communities().len());
    let role = match (a.public, a.public_relay, a.peer_only) {
        (true, _, _) => "public node",
        (false, true, true) => "public relay, peer only",
        (false, true, false) => "home node and public relay",
        (false, false, true) => "peer only, not public",
        (false, false, false) => "home node, not public",
    };
    println!("hoi-node {} · {} · {role} · {} communities kept warm", hoi_core::VERSION, a.data.display(), communities);
    let pairing_file = a.data.join(PAIRING_FILE);
    let mut pairing_task = None;
    if let Some(links) = pairing_text(&st) {
        let s = st.serving.as_ref().expect("pairing text comes from a serving node");
        println!("serving paired devices at {}", s.url);
        // The link carries the pairing token: it goes to a file only this user can read, and to
        // the terminal (and so to service logs) only when asked for.
        match admin::write_private(&pairing_file, links.as_bytes()) {
            Ok(()) => println!("pairing link: written to {} (readable by this user only)", pairing_file.display()),
            Err(e) => eprintln!("hoi-node: could not write the pairing link to {}: {e}", pairing_file.display()),
        }
        pairing_task = Some(tokio::spawn(refresh_pairing_file(shared.clone(), pairing_file.clone(), links)));
        if a.show_pairing || a.new_pairing {
            println!("pairing link (paste it in hoi → You → Home node):\n\n  {}\n", s.pairing);
            if !st.rtc.is_empty() {
                println!("or open this on your phone (a QR code of it works too):\n\n  {}\n", s.pairing_web);
            }
        } else {
            println!("start with --show-pairing to print it here (it is a credential: keep it out of shared logs)");
        }
    } else if std::fs::remove_file(&pairing_file).is_ok() {
        // No pairing listener: a link left from an earlier run pairs with nothing and goes.
        println!("no pairing listener: removed {}", pairing_file.display());
    }
    if let Some(h) = &rtc {
        let routes = if st.rtc.is_empty() { "no network address found yet".to_string() } else { st.rtc.join(", ") };
        println!("WebRTC on UDP {} for browsers and phones: {routes}", h.port());
    }
    let cap = shared.with(|n| n.settings().daily_cap_gb);
    println!("relaying for members' browsers and nodes: {}", if st.relaying { format!("on, up to {cap} GB a day") } else { "off".into() });
    println!("carry policy: {}", carry.describe());
    if carry == CarryMode::Any {
        println!("carrying on request: up to {} communities and {} MB together", a.carry_limit.div_ceil(2), a.carry_bytes >> 20);
    }
    if !a.refuse.is_empty() {
        println!("refusing {} communit{} whoever asks (--refuse)", a.refuse.len(), if a.refuse.len() == 1 { "y" } else { "ies" });
    }
    if let Some(p) = shared.with(|n| n.piece_stats()) {
        println!(
            "holding encrypted pieces: {} held, {} MB of up to {} MB; none readable here (--hold-pieces)",
            p.count,
            p.bytes / 1_000_000,
            p.budget / 1_000_000
        );
    }
    if !a.peer_only && !a.listen.ip().is_loopback() {
        println!("note: browsers on https pages can only reach this node over wss://; put a TLS proxy in front or use the desktop app");
        println!(
            "note: paired devices may connect on {} from any network that reaches it; in a container publish it on the host's loopback only (-p 127.0.0.1:7842:7842)",
            a.listen
        );
    }
    let mut tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    for url in &a.indexes {
        if carry == CarryMode::Off {
            println!("public index {url}: announcing this node's public routes only (--carry off)");
        } else {
            println!("public index {url}: announcing this node's public routes, carrying up to {} of its communities", a.carry_limit);
        }
        if !st.relaying {
            println!("note: relaying is off, so nobody can reach what this node carries; start it with --relaying");
        }
        tasks.push(index::spawn(shared.clone(), index::IndexConfig { url: url.clone(), carry_limit: a.carry_limit, mirror: carry != CarryMode::Off }));
    }
    // the trackers the communities it carries name; only a public node announces
    if st.relaying {
        tasks.push(trackers::spawn(shared.clone(), a.indexes.clone(), a.carry_limit));
    }
    // the DHT among public nodes: only a public node runs one
    let dht = if st.relaying && carry == CarryMode::Any && a.dht {
        let bind = peer_bind.and_then(|b| match b.ip() {
            std::net::IpAddr::V4(ip) if !ip.is_unspecified() => Some(ip),
            _ => None,
        });
        let config = hoi_net::dht::DhtConfig {
            port: a.dht_port.unwrap_or(hoi_net::dht::DEFAULT_PORT),
            answer_port: a.dht_answer_port.unwrap_or(hoi_net::dht::DEFAULT_ANSWER_PORT),
            bootstrap: a.dht_bootstrap.clone(),
            bind,
        };
        match hoi_net::dht::start_dht(shared.clone(), config).await {
            Ok(h) => {
                let from = if a.dht_bootstrap.is_empty() { "the public nodes it meets".to_string() } else { a.dht_bootstrap.join(", ") };
                println!("dht: listening on UDP {}, answering on UDP {}, starting from {from}", h.ports.dht, h.ports.answer);
                Some(h)
            }
            Err(e) => {
                eprintln!("hoi-node: the DHT could not start: {e}");
                None
            }
        }
    } else {
        None
    };
    if st.relaying && carry == CarryMode::Any {
        println!("a public node: passing on the public nodes it reaches to whoever connects; no registry needed");
    } else if st.relaying {
        println!("not a public node: it relays for its communities' members only, has no node link and passes on no other nodes");
    }
    let node_link_file = a.data.join(NODE_LINK_FILE);
    let node_link_task = tokio::spawn(refresh_node_link(shared.clone(), node_link_file));
    let notice_hash_file = a.data.join(NOTICE_HASH_FILE);
    let notice_task = match (&a.notice, notice_hash) {
        (Some(path), Some(hash)) => {
            let byline = shared.with(|n| n.operator_notice().map(hoi_types::NodeNotice::byline).unwrap_or_default());
            println!("operator notice: {byline} · {} · hash in {} (re-read when {} changes)", &hash[..12], notice_hash_file.display(), path.display());
            Some(tokio::spawn(refresh_notice(shared.clone(), path.clone(), notice_hash_file, hash)))
        }
        _ => {
            let _ = std::fs::remove_file(&notice_hash_file);
            None
        }
    };
    let addresses_task = tokio::spawn(report_addresses(shared.clone()));
    let vpn_task = a.vpn_only.clone().map(|name| tokio::spawn(watch_vpn(policy.clone(), name, shared.clone())));
    // The router forwards the WebRTC port and the native transport's IPv4 ports; what it says
    // its public address is goes into this node's routes, and so into its announcements.
    let rtc_port = rtc.as_ref().map(|h| h.port());
    let peer_ports = p2p.ipv4_ports();
    // members of the same communities on this network; not through a VPN only
    let _lan = match (a.lan && a.vpn_only.is_none(), peer_ports.first()) {
        (true, Some(port)) => match hoi_net::lan::start_lan(shared.clone(), *port, None) {
            Ok(h) => {
                println!("lan: beacons on UDP {} for members of the same communities nearby", hoi_core::lan::LAN_PORT);
                Some(h)
            }
            Err(e) => {
                println!("lan: no beacons ({e})");
                None
            }
        },
        _ => {
            shared.with(|n| n.set_lan(false));
            None
        }
    };
    // link previews for this node's paired browsers, through the VPN where asked
    let _previews = {
        let p = policy.clone();
        hoi_net::preview::start_previews(shared.clone(), std::sync::Arc::new(move || p.clone()))
    };
    // names that are domains, checked against the keys they claim
    let _handles = {
        let p = policy.clone();
        hoi_net::handle::start_handle_checks(shared.clone(), std::sync::Arc::new(move || p.clone()))
    };
    let (forwarded_tx, mut forwarded) = tokio::sync::watch::channel(Vec::<hoi_net::portmap::Forward>::new());
    let dht_ports = dht.as_ref().map(|h| [h.ports.dht, h.ports.answer]).into_iter().flatten();
    let ports: Vec<u16> = rtc_port.into_iter().chain(peer_ports.iter().copied()).chain(dht_ports).collect();
    let mapper = if a.port_mapping.unwrap_or(a.public_relay) && !ports.is_empty() {
        let list = ports.iter().map(u16::to_string).collect::<Vec<_>>().join(" and ");
        println!("port mapping: asking the router to forward UDP {list}");
        let tx = forwarded_tx.clone();
        Some(hoi_net::portmap::PortMapper::spawn(
            ports,
            // Through a VPN, only its own gateway: asking the home router would publish the home.
            a.vpn_only.clone().map_or(hoi_net::portmap::Via::DefaultRoute, hoi_net::portmap::Via::Interface),
            move |f| {
                let _ = tx.send(f.to_vec());
            },
            |s| match s {
                hoi_net::portmap::Status::Failed(_) => eprintln!("hoi-node: {}", s.line()),
                _ => println!("{}", s.line()),
            },
        ))
    } else {
        None
    };
    let mut registry_task = None;
    if let (Some(registry), Some(r)) = (&registry, &a.registry) {
        match registry::serve(registry.clone(), r.listen).await {
            Ok((task, addr)) => {
                println!("registry listening on {addr} · {}", registry.summary());
                if !r.announcements {
                    println!("registry: node announcements are off; only configured routes are listed");
                } else {
                    match registry.announcement_origin() {
                        Some(origin) => println!("registry: takes node announcements signed for {origin}"),
                        None => println!("registry: without --registry-url, node announcements are taken only when signed for this machine's loopback address"),
                    }
                    if let Some(n) = registry.allowlist() {
                        println!("registry: allowlist mode, announcements from {n} endpoint key{} only", if n == 1 { "" } else { "s" });
                    }
                }
                if r.origins.is_empty() {
                    println!("note: no --registry-origin given, so browsers are refused; the hoi-node carrier and other non-browser clients still work");
                }
                if !addr.ip().is_loopback() {
                    println!("note: browsers on https pages reach a registry only over https://; put a TLS proxy in front");
                }
                registry_task = Some(task);
            }
            Err(e) => {
                eprintln!("hoi-node: registry can't listen on {}: {e}", r.listen);
                std::process::exit(1);
            }
        }
        for url in &r.mirrors {
            println!("registry: mirroring names from {url}");
            tasks.push(registry::mirror::spawn(registry.clone(), url.clone()));
        }
    }
    let stop = shutdown_signal();
    tokio::pin!(stop);
    loop {
        tokio::select! {
            _ = &mut stop => break,
            Ok(()) = forwarded.changed() => {
                let forwards = forwarded.borrow_and_update().clone();
                let public = |port: Option<u16>| hoi_net::portmap::PortMapper::external_for(&forwards, port);
                if let Some(h) = &rtc {
                    h.set_advertised(public(rtc_port));
                }
                p2p.set_advertised(peer_ports.iter().flat_map(|p| public(Some(*p))).collect());
            }
        }
    }
    drop(forwarded_tx);
    if let Some(mapper) = mapper {
        mapper.shutdown().await;
    }
    node_link_task.abort();
    addresses_task.abort();
    if let Some(t) = vpn_task {
        t.abort();
    }
    for task in tasks.into_iter().chain(pairing_task).chain(notice_task) {
        task.abort();
    }
    if let Some(task) = registry_task {
        task.abort();
    }
    if let Some(server) = server {
        server.abort();
    }
    shared.close_all("client-");
    if let Some(h) = rtc {
        h.shutdown().await;
    }
    p2p.shutdown().await;
    shared.with(|n| {
        let _ = n.commit();
    });
    println!("bye");
}

/// Ctrl-C, or the SIGTERM a service manager (systemd, launchd, Docker) stops a service with: the
/// node commits its state and removes its port forwards either way.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let Ok(mut term) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) else {
            let _ = tokio::signal::ctrl_c().await;
            return;
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// The pairing links as `DIR/pairing-link` holds them, while the node serves paired devices.
fn pairing_text(st: &hoi_core::dto::NetStatus) -> Option<String> {
    let s = st.serving.as_ref()?;
    let web = if st.rtc.is_empty() { String::new() } else { format!("{}\n", s.pairing_web) };
    Some(format!("{}\n{web}", s.pairing))
}

/// Reads `--notice FILE`: JSON, as the installers and the app write it, checked the way the
/// node checks it.
fn load_notice(path: &std::path::Path) -> Result<hoi_types::NodeNotice, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("can't read it: {e}"))?;
    let notice: hoi_types::NodeNotice = serde_json::from_str(&text).map_err(|e| format!("not a notice file (JSON): {e}"))?;
    let problems = notice.validate();
    if !problems.is_empty() {
        return Err(problems.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "));
    }
    Ok(notice)
}

/// Keeps `DIR/notice-hash` on the hash of the notice this node carries, and takes a changed
/// `--notice FILE` on: connected strangers hear the new words, links made from now on carry
/// them. A file that no longer reads keeps the notice as it was, and says so once.
async fn refresh_notice(shared: std::sync::Arc<Shared>, path: PathBuf, hash_file: PathBuf, hash: String) {
    let mut written = None;
    let mut modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let mut hash = hash;
    let mut complained = false;
    let mut tick = tokio::time::interval(NOTICE_POLL);
    loop {
        if written.as_ref() != Some(&hash) {
            match std::fs::write(
                &hash_file,
                format!(
                    "{hash}
"
                ),
            ) {
                Ok(()) => written = Some(hash.clone()),
                Err(e) => eprintln!("hoi-node: could not write the notice hash to {}: {e}", hash_file.display()),
            }
        }
        tick.tick().await;
        let now = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if now == modified {
            continue;
        }
        modified = now;
        match load_notice(&path) {
            Ok(notice) => match shared.with(|n| n.set_notice(Some(notice))) {
                Ok(Some(new)) => {
                    if new != hash {
                        println!("operator notice changed: hash {} (people who connected before read it again)", &new[..12]);
                        hash = new;
                    }
                    complained = false;
                }
                Ok(None) => {}
                Err(e) => eprintln!("hoi-node: {}: {} (keeping the notice as it was)", path.display(), e.message),
            },
            Err(e) if !complained => {
                eprintln!("hoi-node: {}: {e} (keeping the notice as it was)", path.display());
                complained = true;
            }
            Err(_) => {}
        }
    }
}

/// Keeps `DIR/node-link` on this node's current public link, and says what it is whenever it
/// changes: once the router forwarded its ports, the link names its public address. The link is
/// an address, no credential, so it is printed: it is there to be shared. While the node serves
/// no strangers there is no link and no file.
async fn refresh_node_link(shared: std::sync::Arc<Shared>, path: PathBuf) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(2));
    let mut written: Option<String> = None;
    let mut first = true;
    loop {
        tick.tick().await;
        let link = shared.with(|n| n.net_status().node_link);
        if link == written && !first {
            continue;
        }
        first = false;
        match &link {
            Some(l) => match std::fs::write(&path, format!("{l}\n")) {
                Ok(()) => println!(
                    "node link (anyone who opens it finds the communities this node carries, and the next nodes along):\n\n  {l}\n\n  {}\n",
                    link_holds(l)
                ),
                Err(e) => eprintln!("hoi-node: could not write the node link to {}: {e}", path.display()),
            },
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
        written = link;
    }
}

/// What a node link holds, in words: its addresses are packed into the code, out of sight.
fn link_holds(link: &str) -> String {
    let Ok(l) = hoi_types::NodeLink::parse(link) else { return "it holds nothing this node can read back".into() };
    let routes = |n: usize, what: &str| format!("{n} {what} route{}", if n == 1 { "" } else { "s" });
    let notice = if l.notice.is_some() && l.signature.is_some() { ", and the operator's notice, signed" } else { ", and no operator notice" };
    format!("it holds {} and {}{notice}; the addresses are packed into the code", routes(l.rtc.len(), "WebRTC"), routes(l.peers.len(), "native"))
}

/// Waits until the system sends internet traffic through the VPN, saying why not now and then.
async fn wait_for_vpn(policy: &hoi_net::vpn::NetPolicy, name: &str) -> hoi_net::vpn::VpnState {
    let mut said = None;
    loop {
        match policy.state() {
            up @ hoi_net::vpn::VpnState::Up(_) => {
                let addrs: Vec<String> = up.addresses().iter().map(ToString::to_string).collect();
                println!("only through the VPN: {name} is up ({}); every connection to the internet goes through it", addrs.join(", "));
                return up;
            }
            hoi_net::vpn::VpnState::Down(why) if said != Some(why) => {
                println!("only through the VPN: {name} {}; waiting, nothing goes to the internet until it does", why.words());
                said = Some(why);
            }
            _ => {}
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

/// The kill switch: while the VPN carries this computer's traffic the node runs; once it
/// doesn't, the node stops, so nothing reaches the internet around it. A service manager
/// starts it again, and it waits for the VPN (`wait_for_vpn`).
async fn watch_vpn(policy: hoi_net::vpn::NetPolicy, name: String, shared: std::sync::Arc<Shared>) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        tick.tick().await;
        let state = policy.state();
        if let hoi_net::vpn::VpnState::Down(why) = state {
            shared.with(|n| {
                n.set_vpn_status(state.report(&name));
                let _ = n.commit();
            });
            eprintln!("hoi-node: {name} {}; stopping so nothing reaches the internet around the VPN", why.words());
            std::process::exit(3);
        }
    }
}

/// Says which addresses this node gives out and who hears each, whenever that changes: the
/// operator should never have to guess what the internet learns.
async fn report_addresses(shared: std::sync::Arc<Shared>) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(2));
    let mut said: Option<String> = None;
    loop {
        tick.tick().await;
        let text = addresses_text(&shared.with(|n| n.net_status().exposure));
        if said.as_deref() != Some(text.as_str()) {
            println!("{text}");
            said = Some(text);
        }
    }
}

fn addresses_text(exposure: &[hoi_core::dto::Exposure]) -> String {
    if exposure.is_empty() {
        return "addresses: none yet; nothing is published".into();
    }
    let mut out = String::from("addresses this node gives out (whoever connects also sees the address the connection comes from):");
    for e in exposure {
        let reach = match (e.scope.as_str(), e.from_router) {
            ("internet", true) => "internet, forwarded by the router",
            ("internet", false) => "internet",
            ("network", _) => "home network",
            _ => "this computer",
        };
        let told: Vec<&str> = e
            .told
            .iter()
            .map(|t| match t.as_str() {
                "devices" => "your devices",
                "strangers" => "anyone who connects, invites",
                _ => "the node link",
            })
            .collect();
        out.push_str(&format!("\n  {:<26} {reach} · told to {}", e.address, told.join(", ")));
    }
    out
}

/// Each link pairs one device and is then spent: keeps `DIR/pairing-link`
/// on the node's next link, so the file never offers a spent one. Says that it changed, never
/// the link itself.
async fn refresh_pairing_file(shared: std::sync::Arc<Shared>, path: PathBuf, mut written: String) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(2));
    loop {
        tick.tick().await;
        let Some(links) = pairing_text(&shared.with(|n| n.net_status())) else { continue };
        if links == written {
            continue;
        }
        match admin::write_private(&path, links.as_bytes()) {
            Ok(()) => {
                println!("a device paired; the next pairing link is in {}", path.display());
                written = links;
            }
            Err(e) => eprintln!("hoi-node: could not write the pairing link to {}: {e}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_operator_reads_which_address_goes_to_whom() {
        let e = |address: &str, scope: &str, from_router: bool, told: &[&str]| hoi_core::dto::Exposure {
            address: address.into(),
            scope: scope.into(),
            from_router,
            told: told.iter().map(|t| t.to_string()).collect(),
        };
        let text = addresses_text(&[
            e("203.0.113.7:7844", "internet", true, &["devices", "strangers", "node-link"]),
            e("192.168.1.5:7844", "network", false, &["devices", "strangers", "node-link"]),
        ]);
        assert!(
            text.contains("203.0.113.7:7844")
                && text.contains("internet, forwarded by the router · told to your devices, anyone who connects, invites, the node link"),
            "{text}"
        );
        assert!(text.contains("192.168.1.5:7844") && text.contains("home network"), "{text}");
        assert_eq!(addresses_text(&[]), "addresses: none yet; nothing is published");
    }

    #[test]
    fn public_pairing_address_requires_tls_without_credentials() {
        assert!(valid_public_url("wss://home.example/hoi"));
        for url in ["ws://home.example", "https://home.example", "wss://secret@home.example", "wss://home.example/?token=secret", "wss://home.example/#secret"]
        {
            assert!(!valid_public_url(url));
        }
    }

    #[test]
    fn carry_modes_are_the_three_words() {
        assert_eq!(CarryMode::parse("off"), Some(CarryMode::Off));
        assert_eq!(CarryMode::parse("index"), Some(CarryMode::Index));
        assert_eq!(CarryMode::parse("any"), Some(CarryMode::Any));
        assert_eq!(CarryMode::parse("all"), None);
    }
}
