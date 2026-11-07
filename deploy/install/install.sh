#!/usr/bin/env bash
# The hoi installer for macOS and Linux: the desktop app, the companion, or a node that runs as a
# service (a home node, which is a public relay too; a public node; a public node with a
# registry). Windows: install.ps1. Run it again to update or to switch roles; the data stays.
# Nothing it installs names a node or a registry to start from: nodes find each other.
#
#   curl -fsSL https://hoi.sh/install.sh | bash                          asks what to install
#   curl -fsSL https://hoi.sh/install.sh | bash -s -- desktop
#   curl -fsSL https://hoi.sh/install.sh | bash -s -- home --name "lena's pi"
#   curl -fsSL https://hoi.sh/install.sh | bash -s -- registry --registry-url https://names.example.org
#   deploy/install/install.sh home --from dist/release/v0.2.0              a release built on a computer
#   deploy/install/install.sh config registry --domain example.org         a file for a server, then there:
#   curl -fsSL https://hoi.sh/install.sh | bash -s -- check --config hoi-sleepy-walrus-48213.conf
#   curl -fsSL https://hoi.sh/install.sh | bash -s -- --config hoi-sleepy-walrus-48213.conf
#
# What:
#   desktop      the hoi app: it opens as the companion in the tray, the full app a click away
#   companion    the same app, started in the tray whenever you log in
#   home         a home node for your phone and browser that is a public relay too, as a service
#                (a Raspberry Pi, a Mac mini): it carries open communities for whoever asks, tells
#                whoever connects about the other public nodes it reached, and has a link to share
#   relay        the same as home (its name from before every home node was a public relay)
#   private      a home node for your own devices and your communities' members only
#   public       a public node: no key, carries open communities, passes on other public nodes
#   registry     a public node with a registry (the public-name API), behind your TLS proxy
#   uninstall    stop and remove the node service this script installed (the data stays)
#   config       install nothing: write a file describing a public node (config public, the
#                default) or a registry (config registry) for another computer, a VPS, under a
#                random name that is also the server's host name; install it there with --config
#   check        install nothing: look at this computer first and say whether the node can be
#                installed, and what would stop it (check public, check --config FILE, …). Every
#                node install runs the same check and stops before changing anything on a BLOCK
#
# A node for another computer:
#   --domain DOMAIN       config registry: the registry's address becomes https://NAME.DOMAIN
#   --out FILE            config: where to write it (default: hoi-NAME.conf here; never overwritten)
#   --config FILE         install the node that file describes and give this computer its host
#                         name; flags given here win over the file's lines
#   --hostname NAME       give this computer that host name (Linux; --config files carry one)
#   --tls caddy           registry: serve its https:// address with Caddy, from this system's
#                         packages (Debian 12, Ubuntu 24.04, Fedora), certificate included
#
# A release folder (GitHub's release files, or scripts/release-local.mjs's) with this installer in
# it is used as it is, without the internet: put manifest.json and the hoi-node archive for the
# server beside install.sh, copy the folder over and run it there.
#
# From where (default: the latest release in https://hoi.sh/releases/manifest.json):
#   --manifest URL        another release manifest
#   --release TAG         that GitHub release, with the GitHub CLI (works for a private repository)
#   --repo OWNER/NAME     the repository --release reads (default: hoi-project/hoi)
#   --from DIR            a release folder, such as dist/release/v0.2.0 from scripts/release-local.mjs
#   --source              build hoi-node from this checkout (node roles)
#   --binary PATH         this hoi-node binary, or the release archive holding it (node roles)
#   --pubkey KEY          the minisign key the manifest must be signed with (default: the project's)
#   --allow-unsigned-local  development only: trust an unsigned --from folder you built yourself
#
# Nodes:
#   --node LINK           a public node to look at first, by its link (repeatable; optional)
#   --notice-file FILE    the operator's notice (JSON: what the node keeps and, optional but
#                         recommended, who runs it, how to reach them and where the server is,
#                         shown to everyone who connects; the app's "A node on a server" page and the setup
#                         app write it, hoi-node admin notice check reads it): copied next to
#                         the service and carried by the node, so people read it before it
#                         serves them. A public node or registry without one says so
#   --index URL           a registry to announce to and carry from (repeatable; optional, none is
#                         needed)
#   --name NAME           what your devices call it (default: this computer's name)
#   --daily-cap-gb N      gigabytes a day it may relay for others (default: 5)
#   --hold-pieces-gb N    public nodes: keep up to N gigabytes of encrypted pieces and private
#                         communities' mailboxes for other people's devices (0.1 to 10000; 2
#                         when not given, 0 for none). The node can read none of it, and its
#                         notice says so
#   --rtc-advertise ADDR  public IP:port browsers reach it at, when you forward the port yourself
#   --no-port-mapping     don't ask the router to forward the node's ports
#   --registry-url URL    registry: its public https:// address, as apps and nodes are given it
#   --registry-origin O   registry: a browser origin allowed to call it (repeatable; default:
#                         https://hoi.sh and the desktop app's)
#   --registry-listen A   registry: where it listens for your TLS proxy (default: 127.0.0.1:7845)
#   --extra "ARGS"        more hoi-node flags, as one quoted string (hoi-node --help)
#
#   --no-firewall         don't open the node's ports in an active ufw or firewalld
#   --reset-data          Linux nodes: erase the selected role's stored data before starting
#   --confirm-reset NAME  confirm hoi-public (public/registry) or hoi-node (home/private);
#                         otherwise asks you to type it; --yes never confirms deletion
#
#   --yes                 ask nothing (give the role)
#   --no-launch           desktop: don't open the app when it is installed
#   --skip-checks         install even when the check finds something that stops it
#   --dry-run             fetch and check the files, print what would be done, change nothing
set -euo pipefail

LABEL=sh.hoi.node
DEFAULT_MANIFEST=https://hoi.sh/releases/manifest.json
DEFAULT_REPO=hoi-project/hoi
# These defaults are filled from publication.json when preparing release files / Node Setup.
RELEASE_PUBKEY=""

# A checkout, when this file is one (not when it arrives through a pipe).
ROOT=""
here=""
if [ -f "${BASH_SOURCE[0]:-}" ]; then
  here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  [ -f "$here/../../core/Cargo.toml" ] && ROOT="$(cd "$here/../.." && pwd)"
fi

role=""
indexes=()
nodes=()
origins=()
notice_file=""
notice_installed=""
name=""
cap=""
pieces=""
advertise=""
registry_url=""
registry_listen="127.0.0.1:7845"
extra=""
mapping=1
manifest_url=""
release=""
repo=""
from=""
source=0
binary=""
pubkey="$RELEASE_PUBKEY"
unsigned_local=0
yes=0
launch=1
dry=0
making=0
checking=0
skip_checks=0
firewall=1
tls=""
domain=""
conf_out=""
config=""
reset_data=0
reset_confirmation=""
host=""
work=""
trap '[ -z "$work" ] || rm -rf "$work"' EXIT

die() { echo "install.sh: $*" >&2; exit 2; }
say() { echo "$*"; }
note() { echo "note: $*" >&2; }
# Changes this computer: printed instead of run with --dry-run.
run() {
  if [ "$dry" = 1 ]; then echo "+ $*"; else "$@"; fi
}
as_root() {
  if [ "$(id -u)" = 0 ]; then run "$@"; else run sudo "$@"; fi
}
# Writes a file only root may change (put -a appends); printed instead with --dry-run.
put() {
  local tee=(tee) verb=write
  if [ "$1" = -a ]; then tee=(tee -a); verb="append to"; shift; fi
  if [ "$dry" = 1 ]; then
    printf '+ %s %s:\n%s\n' "$verb" "$1" "$2"
  elif [ "$(id -u)" = 0 ]; then
    printf '%s\n' "$2" | "${tee[@]}" "$1" >/dev/null
  else
    printf '%s\n' "$2" | sudo "${tee[@]}" "$1" >/dev/null
  fi
}
usage() {
  # Piped from curl there is no file to read the help from: point at the copy that is one.
  if [ -f "${BASH_SOURCE[0]:-}" ]; then
    sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed -e '$d' -e 's/^# \{0,1\}//'
  else
    echo "The options are in the comments at the top of https://hoi.sh/install.sh (curl -fsSL https://hoi.sh/install.sh | less)."
  fi
  exit 0
}

while [ $# -gt 0 ]; do
  case "$1" in
    desktop|companion|relay|home|private|public|registry|uninstall)
      [ -z "$role" ] || die "one role at a time ($role, $1)"
      role="$1"; shift ;;
    config) making=1; shift ;;
    check) checking=1; shift ;;
    --domain) [ $# -ge 2 ] || die "--domain needs a domain"; domain="$2"; shift 2 ;;
    --out) [ $# -ge 2 ] || die "--out needs a file"; conf_out="$2"; shift 2 ;;
    --config) [ $# -ge 2 ] || die "--config needs a file"; config="$2"; shift 2 ;;
    --hostname) [ $# -ge 2 ] || die "--hostname needs a name"; host="$2"; shift 2 ;;
    --tls) [ $# -ge 2 ] || die "--tls needs caddy"; tls="$2"; shift 2 ;;
    --no-firewall) firewall=0; shift ;;
    --reset-data) reset_data=1; shift ;;
    --confirm-reset) [ $# -ge 2 ] || die "--confirm-reset needs hoi-public or hoi-node"; reset_confirmation="$2"; shift 2 ;;
    --skip-checks) skip_checks=1; shift ;;
    --index) [ $# -ge 2 ] || die "--index needs a URL"; indexes+=("$2"); shift 2 ;;
    --node) [ $# -ge 2 ] || die "--node needs a node link"; nodes+=("$2"); shift 2 ;;
    --notice-file) [ $# -ge 2 ] || die "--notice-file needs a file"; notice_file="$2"; shift 2 ;;
    --name) [ $# -ge 2 ] || die "--name needs a name"; name="$2"; shift 2 ;;
    --daily-cap-gb) [ $# -ge 2 ] || die "--daily-cap-gb needs a number"; cap="$2"; shift 2 ;;
    --hold-pieces-gb) [ $# -ge 2 ] || die "--hold-pieces-gb needs a number"; pieces="$2"; shift 2 ;;
    --rtc-advertise) [ $# -ge 2 ] || die "--rtc-advertise needs IP:PORT"; advertise="$2"; shift 2 ;;
    --no-port-mapping) mapping=0; shift ;;
    --registry-url) [ $# -ge 2 ] || die "--registry-url needs a URL"; registry_url="$2"; shift 2 ;;
    --registry-origin) [ $# -ge 2 ] || die "--registry-origin needs an origin"; origins+=("$2"); shift 2 ;;
    --registry-listen) [ $# -ge 2 ] || die "--registry-listen needs ADDR:PORT"; registry_listen="$2"; shift 2 ;;
    --extra) [ $# -ge 2 ] || die "--extra needs flags"; extra="$2"; shift 2 ;;
    --manifest) [ $# -ge 2 ] || die "--manifest needs a URL"; manifest_url="$2"; shift 2 ;;
    --release) [ $# -ge 2 ] || die "--release needs a tag"; release="$2"; shift 2 ;;
    --repo) [ $# -ge 2 ] || die "--repo needs OWNER/NAME"; repo="$2"; shift 2 ;;
    --from) [ $# -ge 2 ] || die "--from needs a folder"; from="$2"; shift 2 ;;
    --source) source=1; shift ;;
    --binary) [ $# -ge 2 ] || die "--binary needs a path"; binary="$2"; shift 2 ;;
    --pubkey) [ $# -ge 2 ] || die "--pubkey needs a minisign public key"; pubkey="$2"; shift 2 ;;
    --allow-unsigned-local) unsigned_local=1; shift ;;
    --yes|-y) yes=1; shift ;;
    --no-launch) launch=0; shift ;;
    --dry-run) dry=1; shift ;;
    -h|--help) usage ;;
    *) die "unknown option $1 (see --help)" ;;
  esac
done

# A configuration from `config`: key=value lines, read and checked like flags, never run. Flags
# on the command line win; the file's index lines count only when none was given there.
load_config() {
  local file="$1" line key value given=${#indexes[@]}
  [ -f "$file" ] || die "no such file: $file"
  while IFS= read -r line || [ -n "$line" ]; do
    line="${line%$'\r'}"
    case "$line" in ''|'#'*) continue ;; esac
    [[ "$line" =~ ^([a-z_]+)=(.*)$ ]] || die "$file: not a key=value line: $line"
    key="${BASH_REMATCH[1]}"
    value="${BASH_REMATCH[2]}"
    case "$key" in
      role)
        [[ "$value" =~ ^(public|registry)$ ]] || die "$file: role is public or registry, not $value"
        [ -n "$role" ] || role="$value" ;;
      name) [ -n "$name" ] || name="$value" ;;
      hostname) [ -n "$host" ] || host="$value" ;;
      registry_url) [ -n "$registry_url" ] || registry_url="$value" ;;
      index) [ "$given" -gt 0 ] || indexes+=("$value") ;;
      notice_file)
        # Next to the configuration file unless the line gives a full path.
        case "$value" in /*) notice_file="${notice_file:-$value}" ;; *) notice_file="${notice_file:-$(dirname "$file")/$value}" ;; esac ;;
      daily_cap_gb) [ -n "$cap" ] || cap="$value" ;;
      hold_pieces_gb) [ -n "$pieces" ] || pieces="$value" ;;
      tls) [ -n "$tls" ] || tls="$value" ;;
      *) die "$file: no such setting: $key" ;;
    esac
  done < "$file"
}
[ "$making" = 0 ] || [ "$checking" = 0 ] || die "config writes a configuration and check looks at this computer: one at a time"
if [ -n "$config" ]; then
  [ "$making" = 0 ] || die "config writes a configuration and --config installs one: one at a time"
  load_config "$config"
fi

# Writing a configuration needs nothing of this computer (Git Bash on Windows does); installing
# does. A computer hoi has no build for stops the install here, and is a BLOCK in the check.
unsupported=""
if [ "$making" = 0 ]; then
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$arch" in
    x86_64|amd64) arch=x86_64 ;;
    arm64|aarch64) arch=aarch64 ;;
    armv7l|armv6l) unsupported="a 32-bit system: install a 64-bit one (Raspberry Pi OS 64-bit on a Pi 3, 4 or 5)" ;;
    *) unsupported="no hoi build for $arch" ;;
  esac
  case "$os" in
    Linux) platform=linux; target="$arch-unknown-linux-gnu" ;;
    Darwin) platform=macos; target="$arch-apple-darwin" ;;
    *) die "this installer knows macOS and Linux; on Windows use install.ps1" ;;
  esac
  [ -z "$unsupported" ] || [ "$checking" = 1 ] || die "$unsupported"
fi

# A release folder, or a server kit with the release files put beside the installer: install
# from those files, as an offline server needs (k3s and others read theirs from beside the script).
if [ -n "$here" ] && [ "$making" = 0 ] && [ "$source" = 0 ] && [ -z "$from$release$manifest_url$binary" ] && [ -f "$here/manifest.json" ]; then
  from="$here"
  note "installing from the release files beside the installer ($here)"
fi

# Asks on the terminal, also when the script itself arrives through a pipe.
tty_ok() { [ "$yes" = 0 ] && { : < /dev/tty; } 2>/dev/null; }
ask() {
  local reply
  printf '%s' "$1" > /dev/tty
  IFS= read -r reply < /dev/tty || reply=""
  printf '%s' "$reply"
}

# Names nobody has to think up for `config`: 64 words of each kind, so every word is as likely,
# and a number, about 368 million names in all. A label, not a secret: the node's key is made
# where it runs and is what tells nodes apart.
ADJECTIVES=(bashful bouncy breezy bubbly cheeky chirpy comfy cosmic cozy crispy crumbly curious
  dapper dizzy dozy dreamy fizzy fluffy frosty fuzzy gentle giddy gleeful goofy groovy grumpy
  hasty hazy humble hungry jazzy jolly jumpy lanky lucky mellow mighty misty moody nimble nosy
  peppy perky plucky polite puzzled quiet quirky rowdy rusty salty sleepy sneaky snoozy snug
  soggy spicy sturdy sunny tiny toasty wobbly wiggly zesty)
NOUNS=(alpaca badger bagel banjo beaver biscuit capybara crumpet dumpling ferret gecko gnome goose
  hedgehog kazoo kettle koala lantern lemur llama lobster manatee marmot mitten mongoose moose
  muffin narwhal newt noodle octopus otter owl pancake panda parsnip pelican penguin pickle
  pigeon platypus potato pretzel puffin quokka raccoon radish sardine sloth sock spatula squid
  teacup teapot toaster trombone turnip ukulele waffle walrus wombat yak yeti zucchini)

# A number below $1 from the system's random source, without favouring any.
random_below() {
  local n limit=$(( 4294967296 - 4294967296 % $1 ))
  while :; do
    n="$(od -An -N4 -tu4 /dev/urandom 2>/dev/null | tr -d ' ')"
    [[ "$n" =~ ^[0-9]+$ ]] || return 1
    [ "$n" -ge "$limit" ] || { echo $(( n % $1 )); return; }
  done
}
random_name() {
  echo "${ADJECTIVES[$(random_below ${#ADJECTIVES[@]})]}-${NOUNS[$(random_below ${#NOUNS[@]})]}-$(( 10000 + $(random_below 90000) ))"
}
# The host name a node's name makes: lowercase letters, digits and dashes, at most 63.
host_of() {
  printf '%s' "$1" | tr '[:upper:]' '[:lower:]' | sed -E -e "s/'//g" -e 's/[^a-z0-9]+/-/g' -e 's/^-+//' | cut -c1-63 | sed -E 's/-+$//'
}

if [ "$making" = 1 ]; then
  [ -n "$role" ] || role=public
  [[ "$role" =~ ^(public|registry)$ ]] || die "config is for a public node or a registry: config public, config registry"
  [ -z "$domain" ] || [ "$role" = registry ] || die "--domain is for a registry: config registry --domain $domain"
  # What the file cannot carry is given on the server, where flags win over it anyway.
  [ ${#nodes[@]} -eq 0 ] && [ ${#origins[@]} -eq 0 ] && [ -z "$advertise$extra" ] && [ "$mapping" = 1 ] && [ "$firewall" = 1 ] && [ "$skip_checks" = 0 ] && [ "$registry_listen" = 127.0.0.1:7845 ] ||
    die "the file carries the role, the name, the host name, the registry's address, --index, --daily-cap-gb, --hold-pieces-gb and --tls: give other flags on the server"
  if [ -z "$name" ]; then
    name="$(random_name)"
    [[ "$name" =~ ^[a-z]+-[a-z]+-[0-9]{5}$ ]] || die "no random numbers on this computer (/dev/urandom): give a name with --name"
  fi
fi

if [ -z "$role" ]; then
  tty_ok || die "say what to install: desktop, companion, home, private, public or registry (--help)"
  cat > /dev/tty <<'MENU'
What should this computer run?
  1  desktop    the hoi app: opens as the companion in the tray, the full app a click away
  2  companion  the same app, started in the tray whenever you log in
  3  home       a home node for your phone and browser, a public relay too, as a service
  4  private    a home node for your own devices only, as a service
  5  public     a public node (no key), as a service
  6  registry   a public node with a registry, as a service
MENU
  case "$(ask 'Choose 1-6: ')" in
    1|desktop) role=desktop ;;
    2|companion) role=companion ;;
    3|home|relay) role=home ;;
    4|private) role=private ;;
    5|public) role=public ;;
    6|registry) role=registry ;;
    *) die "nothing chosen" ;;
  esac
fi
# Every home node is a public relay now; "relay" is the name that role had before.
[ "$role" != relay ] || role=home
case "$role" in
  desktop|companion) kind=app ;;
  uninstall) kind=uninstall ;;
  *) kind=node ;;
esac
[ "$making" = 0 ] || kind=config
[ -z "$config" ] || [ "$kind" = node ] || die "--config describes a node: give it no role, or public or registry"
[ "$checking" = 0 ] || [ "$kind" = node ] || die "check looks at this computer for a node: check home, private, public or registry"
[ -z "$reset_confirmation" ] || [ "$reset_data" = 1 ] || die "--confirm-reset requires --reset-data"
if [ "$reset_data" = 1 ]; then
  [ "$kind" = node ] && [ "$platform" = linux ] && [ "$checking" = 0 ] || die "--reset-data is only for installing a Linux node, not check or config"
fi

# Interactive runs ask for what a role cannot do without.
if [ "$role" = registry ] && [ -z "$registry_url" ] && tty_ok; then
  if [ "$kind" = config ]; then
    [ -n "$domain" ] || domain="$(ask "The domain the registry goes under (its address becomes https://NAME.DOMAIN): ")"
  else
    registry_url="$(ask "The registry's public https:// address (your TLS proxy in front of it): ")"
  fi
fi

# Whoever runs a public node answers for other people's data: said once, on the
# terminal, before anything is installed; --yes (a kit, a script) means it was read elsewhere.
if [[ "$role" =~ ^(public|registry)$ ]] && [ "$kind" = node ] && [ "$checking" = 0 ] && [ "$dry" = 0 ] && tty_ok; then
  say "Before you run a public node"
  say "  You become responsible: a public node processes other people's IP addresses and connection"
  say "  data. Under the data protection rules that apply where you, your server and those people are,"
  say "  whoever runs the node is usually the one responsible for that data. The developers are not."
  say "  A node only your family and close friends use may count as personal use, which many such rules"
  say "  exempt; a community open to people you do not know personally generally does not."
  say "  Write a join notice (what the node keeps, for how long): this installer takes it with"
  say "  --notice-file, and everyone reads it before the node serves them. Your name, your contact and"
  say "  the server's country are optional in it, but you should give them; whatever you fill in is"
  say "  shown to everyone who connects. Keep the server secure and answer people's requests about"
  say "  their data. This is not legal advice. (hoi.sh/download/#operator has the whole text.)"
  reply="$(ask "I understand that I am responsible for the data processed on my node [y/N]: ")"
  case "$reply" in y|Y|yes|YES) ;; *) die "nothing was installed" ;; esac
fi

# Everything below ends up in a service file or a command line: plain characters only.
https_url='^https://[A-Za-z0-9.-]+(:[0-9]+)?(/[A-Za-z0-9._~/-]*)?$'
[ -n "$name" ] || name="${host:-$(hostname -s 2>/dev/null || hostname)}"
name="$(printf '%s' "$name" | tr -cd "A-Za-z0-9 ._'-" | cut -c1-40)"
[ -n "$name" ] || name="home node"
if [ "$kind" = config ]; then
  [ -n "$host" ] || host="$(host_of "$name")"
  [ -n "$host" ] || die "that name makes no host name: give one with letters or digits (--name)"
  if [ -n "$domain" ] && [ -z "$registry_url" ]; then
    domain="$(printf '%s' "$domain" | tr '[:upper:]' '[:lower:]')"
    [[ "$domain" =~ ^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)+$ ]] || die "--domain takes a domain such as example.org: $domain"
    registry_url="https://$host.$domain"
  fi
fi
if [ -n "$host" ]; then
  [[ "$host" =~ ^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$ ]] || die "a host name takes lowercase letters, digits and dashes, at most 63: $host"
fi
for url in ${indexes[@]+"${indexes[@]}"}; do
  [[ "$url" =~ $https_url ]] || die "--index needs an https:// registry URL without credentials, query or fragment: $url"
done
node_link='^(hoi://node\?|https://[A-Za-z0-9.-]+(:[0-9]+)?(/[A-Za-z0-9._~/-]*)?#/node\?)[A-Za-z0-9._~:@,=&%-]+$'
for link in ${nodes[@]+"${nodes[@]}"}; do
  [[ "$link" =~ $node_link ]] || die "--node needs a node link (hoi://node?… or https://…#/node?…): $link"
done
if [ -n "$registry_url" ]; then
  [[ "$registry_url" =~ $https_url ]] || die "--registry-url needs the registry's https:// address, without credentials, query or fragment"
fi
if [ "$role" = registry ] && [ -z "$registry_url" ]; then
  [ "$kind" != config ] || die "a registry needs --domain (its address becomes https://$host.DOMAIN) or --registry-url"
  die "a registry needs --registry-url, its public https:// address"
fi
# The host name in the registry's address: what DNS must name, and Caddy's site.
registry_host="${registry_url#https://}"
registry_host="${registry_host%%/*}"
registry_host="${registry_host%%:*}"
if [ -n "$tls" ]; then
  [ "$tls" = caddy ] || die "--tls takes caddy (Caddy serves the registry's https:// address), not $tls"
  [ "$role" = registry ] || die "--tls caddy serves a registry's https:// address: it goes with registry"
  [[ ! "$registry_host" =~ ^[0-9.]+$ ]] || die "--tls caddy needs a domain name in the registry's address, not an IP address"
  [[ "$registry_url" =~ ^https://[^/]+/?$ ]] || die "--tls caddy serves the registry at the root of its own host name: give --registry-url https://NAME.example.org"
fi
for o in ${origins[@]+"${origins[@]}"}; do
  [[ "$o" =~ ^(https://[A-Za-z0-9.-]+(:[0-9]+)?|tauri://localhost|http://tauri\.localhost)$ ]] || die "--registry-origin takes an exact https:// origin: $o"
done
[ ${#origins[@]} -gt 0 ] || origins=(https://hoi.sh tauri://localhost http://tauri.localhost)
[[ "$registry_listen" =~ ^([0-9.]+|\[[0-9a-fA-F:]+\]):[0-9]+$ ]] || die "--registry-listen takes IP:PORT"
if [ -n "$advertise" ]; then
  [[ "$advertise" =~ ^([0-9.]+|\[[0-9a-fA-F:]+\]):[0-9]+$ ]] || die "--rtc-advertise takes the public IP:PORT"
fi
if [ -n "$cap" ]; then
  [[ "$cap" =~ ^[0-9]+(\.[0-9]+)?$ ]] || die "--daily-cap-gb needs a number of gigabytes"
fi
# A public node or registry holds 2 GB of pieces unless told otherwise; 0 means off, and is
# passed on, since hoi-node holds pieces by default on any node that serves strangers.
if [ -z "$pieces" ] && [[ "$role" =~ ^(public|registry)$ ]]; then pieces=2; fi
pieces_off=""
if [ "$pieces" = 0 ]; then pieces=""; pieces_off=1; fi
if [ -n "$pieces" ]; then
  [[ "$pieces" =~ ^[0-9]+(\.[0-9]+)?$ ]] && awk -v n="$pieces" 'BEGIN { exit !(n >= 0.1 && n <= 10000) }' ||
    die "--hold-pieces-gb needs a number of gigabytes from 0.1 to 10000"
  [[ "$role" =~ ^(public|registry|)$ ]] || die "--hold-pieces-gb is for a public node or a registry: other people's devices keep pieces there"
fi
plain='^[][A-Za-z0-9 ._:/=,-]+$'
if [ -n "$extra" ]; then
  [[ "$extra" =~ $plain ]] || die "--extra takes plain flags and values (letters, digits, spaces and . _ : / = , [ ] -)"
fi
if [ -n "$notice_file" ]; then
  [ "$kind" = node ] || [ "$kind" = config ] || die "--notice-file goes with a node: it is the operator's notice the node carries"
  if [ "$kind" = node ]; then
    [ -f "$notice_file" ] || die "no such notice file: $notice_file"
    [ "$(wc -c < "$notice_file")" -le 16384 ] || die "$notice_file is too big for a notice (16 KB at most)"
    # Only these are required; who and where (operator, contact, country) may be left out.
    for key in processes updated; do
      grep -q "\"$key\"" "$notice_file" || die "$notice_file is no notice: no \"$key\" in it (the app's A node on a server page and the setup app write one)"
    done
  fi
fi
if [ -n "$release" ]; then
  [[ "$release" =~ ^v[0-9A-Za-z.-]+$ ]] || die "--release takes a tag such as v0.2.0"
  [ -n "$repo" ] || repo="$DEFAULT_REPO"
  [[ "$repo" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || die "--repo takes OWNER/NAME"
  command -v gh >/dev/null || die "--release needs the GitHub CLI (gh), signed in"
fi
[ -z "$from" ] || [ -f "$from/manifest.json" ] || die "$from has no manifest.json (a release folder from scripts/release-local.mjs has one)"
[ "$unsigned_local" = 0 ] || [ -n "$from" ] || die "--allow-unsigned-local requires a local --from folder; it never permits unsigned downloads"
if [ "$source" = 1 ]; then
  [ "$kind" = node ] || die "--source builds hoi-node; for the app, build a release folder (scripts/release-local.mjs) and use --from"
  [ -n "$ROOT" ] || die "--source needs this script inside a hoi checkout"
fi
[ -z "$binary" ] || [ "$kind" = node ] || die "--binary is a hoi-node; for the app, use --from with a release folder"

work="$(mktemp -d)"

# ── Release files ────────────────────────────────────────────────────────────────────────────

fetch() {
  case "$1" in https://*) ;; *) die "refusing to download over anything but https: $1" ;; esac
  if command -v curl >/dev/null; then
    curl -fsSL --proto '=https' --tlsv1.2 --retry 3 -o "$2" "$1"
  elif command -v wget >/dev/null; then
    wget -q --https-only -O "$2" "$1"
  else
    die "needs curl or wget"
  fi
}

sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

manifest_ready=0
manifest_version=""
manifest_signature=""
manifest_error=""
# Last successful installation, separate from identity/storage. Never changed by a failed check.
release_stamp="$HOME/.local/share/hoi/release-version"
if [ "$kind" = node ]; then
  if [ "$platform" = macos ]; then release_stamp="$HOME/Library/Application Support/hoi-node/release-version"; else release_stamp=/etc/hoi-node/release-version; fi
fi
release_at_least() {
  awk -v a="$1" -v b="$2" 'function num(a,b) { return length(a)!=length(b) ? (length(a)>length(b)?1:-1) : ("x"a=="x"b?0:("x"a>"x"b?1:-1)) }
  BEGIN {
    sub(/\+.*/,"",a); sub(/\+.*/,"",b); ai=index(a,"-"); bi=index(b,"-"); ap=ai?substr(a,ai+1):""; bp=bi?substr(b,bi+1):"";
    sub(/-.*/,"",a); sub(/-.*/,"",b); split(a,x,"."); split(b,y,".");
    for(i=1;i<=3;i++){ c=num(x[i],y[i]); if(c) exit(c<0) }
    if(ap==bp) exit 0; if(ap=="") exit 0; if(bp=="") exit 1;
    an=split(ap,x,"."); bn=split(bp,y,".");
    for(i=1;i<=an || i<=bn;i++) { if(i>an) exit 1; if(i>bn) exit 0; if(x[i]==y[i]) continue;
      xn=x[i]~/^[0-9]+$/; yn=y[i]~/^[0-9]+$/; if(xn && yn) exit(num(x[i],y[i])<0); if(xn!=yn) exit xn; exit("x"x[i]<"x"y[i]); }
  }'
}
check_release_version() {
  local current=""
  if [ -f "$release_stamp" ]; then current="$(cat "$release_stamp")"; fi
  if [ -z "$current" ] && [ "$kind" = node ]; then
    local installed=/usr/local/bin/hoi-node
    [ "$platform" != macos ] || installed="$HOME/Library/Application Support/hoi-node/hoi-node"
    if [ -x "$installed" ]; then current="$("$installed" --version 2>/dev/null | awk '{print $2; exit}')"; fi
  fi
  [ -z "$current" ] || release_at_least "$manifest_version" "$current" || die "refusing to downgrade hoi from $current to $manifest_version; nothing was installed"
}
record_release_version() {
  [ -n "$manifest_version" ] || return 0
  if [ "$kind" = node ] && [ "$platform" = linux ]; then
    as_root mkdir -p "$(dirname "$release_stamp")"
    put "$release_stamp" "$manifest_version"
  else
    run mkdir -p "$(dirname "$release_stamp")"
    if [ "$dry" = 0 ]; then printf '%s\n' "$manifest_version" > "$release_stamp"; fi
  fi
}
# Fetches the release list into the work folder and checks its signature. On a problem it says
# what in $manifest_error and returns 1: the check reports that, an install stops on it.
load_manifest() {
  [ "$manifest_ready" = 0 ] || return 0
  if [ -n "$from" ]; then
    cp "$from/manifest.json" "$work/manifest.json"
    [ ! -f "$from/manifest.json.minisig" ] || cp "$from/manifest.json.minisig" "$work/"
  elif [ -n "$release" ]; then
    gh release download "$release" --repo "$repo" --pattern 'manifest.json*' --dir "$work" --clobber >&2 ||
      { manifest_error="release $release in $repo has no manifest.json (gh signed in?)"; return 1; }
  else
    local url="${manifest_url:-$DEFAULT_MANIFEST}"
    fetch "$url" "$work/manifest.json" || { manifest_error="could not fetch the release list $url (no internet here, or a proxy in the way?)"; return 1; }
    fetch "$url.minisig" "$work/manifest.json.minisig" 2>/dev/null || rm -f "$work/manifest.json.minisig"
  fi
  [ "$(wc -c < "$work/manifest.json")" -le 1048576 ] || { manifest_error="release list exceeds 1 MiB"; return 1; }
  if [ "$unsigned_local" = 1 ] && [ -n "$from" ] && [ -z "$pubkey" ]; then
    manifest_signature="local development folder explicitly trusted: SHA-256 only; publisher not verified"
  elif [ -z "$pubkey" ]; then
    manifest_error="release signing key is not configured; use a published installer with a trusted key, --pubkey, or your own local binary"; return 1
  elif [ ! -f "$work/manifest.json.minisig" ]; then
    manifest_error="the release list has no signature (manifest.json.minisig)"; return 1
  elif command -v minisign >/dev/null; then
    minisign -Vqm "$work/manifest.json" -x "$work/manifest.json.minisig" -P "$pubkey" || { manifest_error="the release list's signature does not verify"; return 1; }
    manifest_signature="release list: signature good"
  else
    manifest_error="minisign is required to verify this release; install it and retry. Nothing was installed"; return 1
  fi
  manifest_version="$(awk -F'"' '/"latest":/ { print $4; exit }' "$work/manifest.json")"
  [[ "$manifest_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.-]+)?(\+[A-Za-z0-9.-]+)?$ ]] || { manifest_error="the release list has no valid version"; return 1; }
  if [ -n "$release" ] && [ "$release" != "v$manifest_version" ]; then manifest_error="release tag does not match the signed list"; return 1; fi
  check_release_version
  manifest_ready=1
}
get_manifest() {
  [ "$manifest_ready" = 0 ] || return 0
  load_manifest || die "$manifest_error"
  say "hoi $manifest_version"
  case "$manifest_signature" in *good) say "$manifest_signature" ;; *) note "$manifest_signature" ;; esac
}

# Every file of the release: platform kind arch name url sha256, one per line. The node setup
# app's files (product "setup") are left out: they are not the app and not hoi-node.
entries() {
  awk -F'"' '
    /"platform":/ { p = $4 } /"kind":/ { k = $4 } /"arch":/ { a = $4 } /"product":/ { pr = $4 } /"name":/ { n = $4 } /"url":/ { u = $4 }
    /"sha256":/ { if (pr != "setup") print p, k, a, n, u, $4; pr = "" }' "$work/manifest.json"
}

# The release list's hoi-node archive for this computer, or nothing.
node_entry() {
  entries | awk -v t="_$target.tar.gz" '$1 == "node" && index($4, t) == length($4) - length(t) + 1 { print; exit }'
}

# The first entry for this platform among `kinds`, preferring `arches` in order.
pick() {
  local plat="$1" kinds="$2" arches="$3" k a
  for k in $kinds; do
    for a in $arches; do
      entries | awk -v p="$plat" -v k="$k" -v a="$a" '$1 == p && $2 == k && $3 == a { print; exit }' | grep . && return 0
    done
  done
  return 1
}

# Fetches one entry into the work folder and checks it; prints its path.
download() {
  local name url sum dest
  read -r _ _ _ name url sum <<< "$1"
  [[ "$name" =~ ^[A-Za-z0-9._+-]+$ ]] || die "an odd file name in the release list: $name"
  [[ "$sum" =~ ^[0-9a-f]{64}$ ]] || die "no SHA-256 for $name in the release list"
  dest="$work/$name"
  echo "fetching $name" >&2
  if [ -n "$from" ]; then
    cp "$from/$name" "$dest" || die "$from has no $name"
  elif [ -n "$release" ]; then
    gh release download "$release" --repo "$repo" --pattern "$name" --dir "$work" --clobber >&2 || die "could not fetch $name"
  elif ! fetch "$url" "$dest" 2>/dev/null; then
    # A private repository's release files need a signed-in GitHub CLI.
    if [[ "$url" =~ ^https://github\.com/([^/]+/[^/]+)/releases/download/([^/]+)/ ]] && command -v gh >/dev/null; then
      gh release download "${BASH_REMATCH[2]}" --repo "${BASH_REMATCH[1]}" --pattern "$name" --dir "$work" --clobber >&2 || die "could not fetch $name"
    else
      die "could not fetch $url"
    fi
  fi
  [ "$(sha256 "$dest")" = "$sum" ] || die "$name does not match the release list's SHA-256: not installing it"
  echo "$dest"
}

# ── The app ──────────────────────────────────────────────────────────────────────────────────

quit_app_macos() {
  if pgrep -xq hoi-desktop 2>/dev/null; then
    say "quitting the running hoi to replace it"
    run osascript -e 'tell application id "sh.hoi.desktop" to quit' || true
    [ "$dry" = 1 ] || for _ in 1 2 3 4 5 6 7 8 9 10; do pgrep -xq hoi-desktop || break; sleep 1; done
  fi
}

install_app_macos() {
  local line dmg mnt src dest app
  line="$(pick macos dmg "universal $arch")" || die "this release has no app for macOS"
  dmg="$(download "$line")"
  mnt="$work/mnt"
  mkdir -p "$mnt"
  hdiutil attach -nobrowse -readonly -quiet -mountpoint "$mnt" "$dmg"
  src="$(ls -d "$mnt"/*.app | head -n 1)"
  [ -n "$src" ] || die "no app in $(basename "$dmg")"
  dest=/Applications
  [ -w "$dest" ] || { dest="$HOME/Applications"; run mkdir -p "$dest"; }
  app="$dest/$(basename "$src")"
  quit_app_macos
  run rm -rf "$app"
  run ditto "$src" "$app"
  hdiutil detach -quiet "$mnt" || true
  say "installed $app"
  if [ "$role" = companion ]; then
    run open -a "$app" --args --hidden --start-at-login
    say "hoi starts in the menu bar now and whenever you log in (the companion's switch turns it off)"
  elif [ "$launch" = 1 ]; then
    run open -a "$app"
  fi
}

install_app_linux() {
  local line file pkg exe=""
  if command -v apt-get >/dev/null && line="$(pick linux deb "$arch")"; then
    file="$(download "$line")"
    pkg="$(dpkg-deb -f "$file" Package)"
    as_root apt-get install -y "$file"
    [ "$dry" = 1 ] || exe="$(dpkg -L "$pkg" | grep -m1 '^/usr/bin/' || true)"
  elif { command -v dnf >/dev/null || command -v zypper >/dev/null; } && line="$(pick linux rpm "$arch")"; then
    file="$(download "$line")"
    pkg="$(rpm -qp --qf '%{NAME}' "$file" 2>/dev/null)"
    if command -v dnf >/dev/null; then as_root dnf install -y "$file"; else as_root zypper --non-interactive install --allow-unsigned-rpm "$file"; fi
    [ "$dry" = 1 ] || exe="$(rpm -ql "$pkg" | grep -m1 '^/usr/bin/' || true)"
  elif line="$(pick linux AppImage "$arch")"; then
    file="$(download "$line")"
    exe="$HOME/.local/bin/hoi.AppImage"
    run mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications"
    run install -m 0755 "$file" "$exe"
    if [ "$dry" = 0 ]; then
      printf '[Desktop Entry]\nType=Application\nName=hoi\nComment=A forum with community-held keys\nExec=%s %%u\nTerminal=false\nCategories=Network;\nMimeType=x-scheme-handler/hoi;\n' "$exe" > "$HOME/.local/share/applications/hoi.desktop"
    fi
    command -v fusermount >/dev/null || note "AppImages need FUSE 2 (Debian and Ubuntu: sudo apt install libfuse2)"
  else
    die "this release has no app for Linux on $arch"
  fi
  say "installed hoi${exe:+ ($exe)}"
  [ -n "$exe" ] || exe=hoi-desktop
  local desk=""
  [ -z "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ] || desk=1
  if [ "$role" = companion ]; then
    if [ -n "$desk" ]; then
      [ "$dry" = 1 ] && echo "+ $exe --hidden --start-at-login &" || { nohup "$exe" --hidden --start-at-login >/dev/null 2>&1 & }
      say "hoi starts in the tray now and whenever you log in (the companion's switch turns it off)"
    else
      say "no desktop session here: log in to your desktop and run '$exe --hidden --start-at-login' once"
    fi
  elif [ "$launch" = 1 ] && [ -n "$desk" ]; then
    [ "$dry" = 1 ] && echo "+ $exe &" || { nohup "$exe" >/dev/null 2>&1 & }
  fi
}

# ── Nodes ────────────────────────────────────────────────────────────────────────────────────

# What the service runs, one argument per line; the data directory comes first.
node_args() {
  local data="$1" args
  args=(--data "$data" --name "$name" --peer-bind 0.0.0.0:7843 --rtc-port 7844)
  case "$role" in
    home)
      # A home node is a public relay unless told otherwise; nothing to add.
      [ "$mapping" = 1 ] || args+=(--no-port-mapping) ;;
    private)
      args+=(--no-public-relay) ;;
    public)
      args+=(--public)
      [ "$mapping" = 0 ] || args+=(--port-mapping) ;;
    registry)
      args+=(--public --registry "$registry_listen" --registry-url "$registry_url")
      for o in "${origins[@]}"; do args+=(--registry-origin "$o"); done
      # It announces itself to its own registry and carries what that lists.
      args+=(--index "$registry_url")
      [ "$mapping" = 0 ] || args+=(--port-mapping) ;;
  esac
  for url in ${indexes[@]+"${indexes[@]}"}; do
    [ "$url" = "$registry_url" ] || args+=(--index "$url")
  done
  for link in ${nodes[@]+"${nodes[@]}"}; do args+=(--node "$link"); done
  [ -z "$notice_installed" ] || args+=(--notice "$notice_installed")
  [ -z "$cap" ] || args+=(--daily-cap-gb "$cap")
  if [ -n "$pieces_off" ]; then args+=(--hold-pieces 0); elif [ -n "$pieces" ]; then args+=(--hold-pieces "$pieces"); fi
  [ -z "$advertise" ] || args+=(--rtc-advertise "$advertise")
  if [ -n "$extra" ]; then
    # shellcheck disable=SC2206 # split on spaces on purpose; the characters were checked above
    local more=($extra)
    args+=("${more[@]}")
  fi
  printf '%s\n' "${args[@]}"
}

# Puts the hoi-node to install at $work/hoi-node.
node_binary() {
  if [ -n "$binary" ]; then
    [ -f "$binary" ] || die "no such file: $binary"
    case "$binary" in
      *.tar.gz|*.tgz) tar -xzf "$binary" -C "$work" hoi-node ;;
      *) cp "$binary" "$work/hoi-node" ;;
    esac
  elif [ "$source" = 1 ]; then
    command -v cargo >/dev/null || die "building needs Rust (https://rustup.rs)"
    say "building hoi-node (the first build takes a while; on a Raspberry Pi up to an hour)"
    cargo build --release --locked -p hoi-node --manifest-path "$ROOT/core/Cargo.toml"
    cp "${CARGO_TARGET_DIR:-$ROOT/core/target}/release/hoi-node" "$work/hoi-node"
  else
    get_manifest
    local line archive
    line="$(node_entry)"
    [ -n "$line" ] || die "this release has no hoi-node for $target; build it with --source"
    archive="$(download "$line")"
    tar -xzf "$archive" -C "$work" hoi-node
  fi
  # An archive packed on Windows, or a file uploaded without its mode, may have lost the x bit.
  chmod 0755 "$work/hoi-node"
  local actual_version
  actual_version="$("$work/hoi-node" --version | awk '{print $2; exit}')" || die "that hoi-node does not run on this computer"
  [[ "$actual_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.-]+)?(\+[A-Za-z0-9.-]+)?$ ]] || die "that hoi-node has no valid version"
  [ -z "$manifest_version" ] || [ "$actual_version" = "$manifest_version" ] || die "hoi-node version does not match the signed list"
  manifest_version="$actual_version"
  check_release_version
  if [ -n "$notice_file" ]; then
    local said
    said="$("$work/hoi-node" admin notice check "$notice_file" 2>&1)" || die "the notice in $notice_file cannot be used: ${said#hoi-node admin: }"
  fi
}

# Where the node reads the operator's notice from: next to the service, root's to write.
NOTICE_PATH=/etc/hoi-node/notice.json

# systemd: double-quoted words; % and $ doubled, as unit files want.
systemd_quote() {
  local out=() a
  for a in "$@"; do
    a="${a//%/%%}"
    a="${a//\$/\$\$}"
    out+=("\"$a\"")
  done
  echo "${out[*]}"
}

xml_escape() {
  printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'
}

# A key holder's data and a public node's are kept apart, so switching roles never hands a
# person's key to a public node.
state_name() {
  case "$role" in public|registry) echo hoi-public ;; *) echo hoi-node ;; esac
}

describe_role() {
  case "$role" in
    home) echo "hoi home node and public relay" ;;
    private) echo "hoi home node (not public)" ;;
    public) echo "hoi public node" ;;
    registry) echo "hoi public node and registry" ;;
  esac
}

# The host name a configuration gave this server, kept across reboots: cloud images set the
# provider's name again at every boot unless cloud-init is told to preserve this one.
set_hostname() {
  if [ "$(hostname 2>/dev/null || true)" != "$host" ]; then
    as_root hostnamectl set-hostname "$host"
    say "this computer is now called $host"
  fi
  # sudo looks the computer's own name up and complains while nothing answers for it.
  if ! grep -qE "[[:space:]]$host([[:space:]]|\$)" /etc/hosts 2>/dev/null; then
    if grep -q '^127\.0\.1\.1[[:space:]]' /etc/hosts 2>/dev/null; then
      as_root sed -i "s/^127\.0\.1\.1[[:space:]].*/127.0.1.1	$host/" /etc/hosts
    else
      put -a /etc/hosts "127.0.1.1	$host"
    fi
  fi
  [ ! -d /etc/cloud/cloud.cfg.d ] || put /etc/cloud/cloud.cfg.d/99-hoi-hostname.cfg "# Written by deploy/install/install.sh --config: keep the host name it gave this server.
preserve_hostname: true"
}

# The registry's https:// address, served by Caddy from this system's packages: Caddy gets and
# renews the certificate by itself and keeps no access log unless told to. The site goes in a file
# of its own; the main Caddyfile only imports it (the packaged example is set aside as
# Caddyfile.before-hoi, a Caddyfile of your own gets one import line at its end).
CADDY_SITE=/etc/caddy/hoi-registry.caddy
setup_caddy() {
  local main=/etc/caddy/Caddyfile
  if ! command -v caddy >/dev/null; then
    say "installing Caddy from this system's packages, for the registry's HTTPS"
    case "$(caddy_source)" in
      # On a server's first boot (cloud-init) the system's own updates may hold apt's lock: wait.
      apt*) as_root apt-get -o DPkg::Lock::Timeout=300 update -qq || true
            as_root env DEBIAN_FRONTEND=noninteractive apt-get -o DPkg::Lock::Timeout=300 install -y -qq caddy ||
              die "Caddy isn't in this system's packages: install it from https://caddyserver.com/docs/install and run this again" ;;
      dnf) as_root dnf install -y -q caddy || die "dnf couldn't install Caddy (RHEL and its kin need EPEL): https://caddyserver.com/docs/install" ;;
      *) die "Caddy isn't in this system's packages: install it from https://caddyserver.com/docs/install and run this again" ;;
    esac
  fi
  as_root mkdir -p /etc/caddy
  put "$CADDY_SITE" "# Written by deploy/install/install.sh (registry): the hoi registry's HTTPS; run it again to change this.
$registry_host {
	reverse_proxy $registry_listen
}"
  if [ ! -f "$main" ] || grep -q 'The Caddyfile is an easy way to configure your Caddy web server' "$main"; then
    [ ! -f "$main" ] || as_root cp "$main" "$main.before-hoi"
    put "$main" "# Written by deploy/install/install.sh: the hoi registry's site is in $CADDY_SITE.
import $CADDY_SITE"
  elif ! grep -qxF "import $CADDY_SITE" "$main"; then
    as_root cp "$main" "$main.before-hoi"
    put -a "$main" "import $CADDY_SITE"
  fi
  if ! as_root caddy validate --config "$main" --adapter caddyfile >/dev/null 2>&1; then
    [ ! -f "$main.before-hoi" ] || as_root cp "$main.before-hoi" "$main"
    die "Caddy doesn't accept $main with the registry in it (put back as it was): sudo caddy validate --config $main"
  fi
  as_root systemctl enable caddy
  as_root systemctl reload-or-restart caddy
  say "Caddy serves $registry_url, with a certificate as soon as DNS for $registry_host leads here"
}

# Lets the node's ports in through the host firewall that the check found on.
open_firewall() {
  [ "$firewall" = 1 ] && [ -n "$fw_missing" ] && [ "$fw" != "?" ] || return 0
  local p
  for p in $fw_missing; do
    case "$fw" in
      ufw) as_root ufw allow "$p" comment hoi-node ;;
      firewalld) as_root firewall-cmd --quiet --permanent --add-port="$p" ;;
    esac
  done
  [ "$fw" != firewalld ] || as_root firewall-cmd --quiet --reload
  # shellcheck disable=SC2086
  say "opened $(ports_words $fw_missing) in $fw"
}

node_linux() {
  command -v systemctl >/dev/null || die "no systemd here; run hoi-node in Docker instead (deploy/home/compose.yml)"
  local unit=/etc/systemd/system/hoi-node.service
  if [ "$role" = uninstall ]; then
    as_root systemctl disable --now hoi-node || true
    as_root rm -f "$unit" /usr/local/bin/hoi-node "$NOTICE_PATH"
    as_root systemctl daemon-reload
    say "removed the hoi-node service; its data stays in /var/lib/hoi-node and /var/lib/hoi-public"
    [ ! -f "$CADDY_SITE" ] || say "Caddy still serves the registry's address: delete $CADDY_SITE and its import line in /etc/caddy/Caddyfile, then: sudo systemctl reload caddy"
    return
  fi
  node_binary
  local state
  state="$(state_name)"
  if [ "$reset_data" = 1 ]; then
    say "Starting fresh erases /var/lib/$state: keys, records, media, settings and any registry database."
    if [ "$dry" = 0 ]; then
      if [ -z "$reset_confirmation" ] && { : < /dev/tty; } 2>/dev/null; then
        reset_confirmation="$(ask "Type $state to permanently erase this data: ")"
      fi
      [ "$reset_confirmation" = "$state" ] || die "data was not erased: confirm with --confirm-reset $state"
    elif [ -n "$reset_confirmation" ] && [ "$reset_confirmation" != "$state" ]; then
      die "--confirm-reset must be $state for this role"
    fi
  fi
  [ "$tls" != caddy ] || setup_caddy
  [ -z "$host" ] || set_hostname
  as_root install -m 0755 "$work/hoi-node" /usr/local/bin/hoi-node
  if [ -n "$notice_file" ]; then
    as_root mkdir -p "$(dirname "$NOTICE_PATH")"
    as_root install -m 0644 "$notice_file" "$NOTICE_PATH"
    notice_installed="$NOTICE_PATH"
  elif [ "$dry" = 0 ] && [ -f "$NOTICE_PATH" ]; then
    # An install without a notice keeps the one a previous install gave: edit it, or remove it.
    notice_installed="$NOTICE_PATH"
    note "keeping the operator notice from the last install ($NOTICE_PATH)"
  fi
  local args=() line content
  while IFS= read -r line; do args+=("$line"); done < <(node_args "/var/lib/$state")
  content="$(cat <<UNIT
# Written by deploy/install/install.sh ($role); run it again to change this.
[Unit]
Description=$(describe_role)
After=network-online.target
Wants=network-online.target

[Service]
DynamicUser=yes
StateDirectory=$state
ExecStart=/usr/local/bin/hoi-node $(systemd_quote "${args[@]}")
Restart=on-failure
RestartSec=5
TimeoutStopSec=30
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
# Netlink is how the node lists its network addresses and finds the router to ask.
RestrictAddressFamilies=AF_INET AF_INET6 AF_NETLINK
LockPersonality=yes
MemoryDenyWriteExecute=yes

[Install]
WantedBy=multi-user.target
UNIT
)"
  put "$unit" "$content"
  open_firewall
  as_root systemctl daemon-reload
  if [ "$reset_data" = 1 ]; then
    # systemd resolves DynamicUser's /var/lib/private directory and its public symlink.
    # Refuse a local drop-in that adds or changes the data directories to be cleaned.
    if [ "$dry" = 0 ]; then
      [ "$(as_root systemctl show --property=StateDirectory --value hoi-node)" = "$state" ] ||
        die "the service's StateDirectory differs from $state: no data was erased"
    fi
    as_root systemctl stop hoi-node
    as_root systemctl clean --what=state hoi-node
  fi
  as_root systemctl enable hoi-node
  as_root systemctl restart hoi-node
  say ""
  say "$(describe_role) runs as a service and starts with the computer."
  say "  status:          journalctl -u hoi-node -f"
  [ "$state" != hoi-node ] || say "  pair your phone: sudo cat /var/lib/hoi-node/pairing-link   (open the https:// link on it)"
  [ "$role" = private ] || say "  share the node:  sudo cat /var/lib/$state/node-link   (anyone who opens it finds what it carries)"
  [ -z "$notice_installed" ] || say "  your notice:     $notice_installed   (edit it; the node reads it again within half a minute; its hash: sudo cat /var/lib/$state/notice-hash)"
  say "  stop, start:     sudo systemctl stop hoi-node / sudo systemctl start hoi-node"
}

node_macos() {
  local support="$HOME/Library/Application Support/hoi-node"
  local plist="$HOME/Library/LaunchAgents/$LABEL.plist"
  local domain
  domain="gui/$(id -u)"
  if [ "$role" = uninstall ]; then
    run launchctl bootout "$domain/$LABEL" 2>/dev/null || true
    run rm -f "$plist" "$support/hoi-node" "$support/notice.json"
    say "removed the hoi-node agent; its data stays in $support"
    return
  fi
  [ "$(id -u)" != 0 ] || die "on macOS run this as yourself, not with sudo: the node runs in your account"
  [ -z "$host" ] || note "a Mac keeps the name it has (System Settings → General → Sharing); the node is called $name"
  node_binary
  local data="$support/data"
  [ "$(state_name)" = hoi-node ] || data="$support/public"
  run mkdir -p "$data" "$HOME/Library/LaunchAgents" "$HOME/Library/Logs"
  run install -m 0755 "$work/hoi-node" "$support/hoi-node"
  run xattr -d com.apple.quarantine "$support/hoi-node" 2>/dev/null || true
  if [ -n "$notice_file" ]; then
    run install -m 0644 "$notice_file" "$support/notice.json"
    notice_installed="$support/notice.json"
  elif [ "$dry" = 0 ] && [ -f "$support/notice.json" ]; then
    notice_installed="$support/notice.json"
    note "keeping the operator notice from the last install ($support/notice.json)"
  fi
  local items line content
  items="    <string>$(xml_escape "$support/hoi-node")</string>"
  while IFS= read -r line; do
    items="$items
    <string>$(xml_escape "$line")</string>"
  done < <(node_args "$data")
  content="$(cat <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- Written by deploy/install/install.sh ($role); run it again to change this. -->
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
$items
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>ExitTimeOut</key>
  <integer>30</integer>
  <key>ProcessType</key>
  <string>Background</string>
  <key>StandardOutPath</key>
  <string>$(xml_escape "$HOME/Library/Logs/hoi-node.log")</string>
  <key>StandardErrorPath</key>
  <string>$(xml_escape "$HOME/Library/Logs/hoi-node.log")</string>
</dict>
</plist>
PLIST
)"
  if [ "$dry" = 1 ]; then
    printf '+ write %s:\n%s\n' "$plist" "$content"
  else
    printf '%s\n' "$content" > "$plist"
  fi
  run launchctl bootout "$domain/$LABEL" 2>/dev/null || true
  if [ "$dry" = 1 ]; then
    echo "+ launchctl bootstrap $domain $plist"
  else
    # Right after a bootout launchd may still be letting the old one go: one more try.
    launchctl bootstrap "$domain" "$plist" 2>/dev/null || { sleep 2; launchctl bootstrap "$domain" "$plist"; }
  fi
  say ""
  say "$(describe_role) runs in your account and starts when you log in."
  say "  status:          tail -f ~/Library/Logs/hoi-node.log"
  [ "$(state_name)" != hoi-node ] || say "  pair your phone: cat \"$data/pairing-link\"   (open the https:// link on it)"
  [ "$role" = private ] || say "  share the node:  cat \"$data/node-link\"   (anyone who opens it finds what it carries)"
  [ -z "$notice_installed" ] || say "  your notice:     $notice_installed   (edit it; the node reads it again within half a minute)"
  say "  stop, start:     launchctl bootout $domain/$LABEL / launchctl bootstrap $domain $plist"
  say "For a Mac that serves around the clock: turn on automatic login (System Settings → Users & Groups),"
  say "stop it sleeping (System Settings → Energy, or: sudo pmset -a sleep 0), and allow hoi-node in the firewall."
}

node_notes() {
  case "$role" in
    home|public|registry)
      say "No registry is needed: apps and other nodes reach this one through its node link, the invites"
      say "of the communities it carries, and the public nodes that pass it on." ;;
  esac
  case "$role" in
    public|registry)
      # shellcheck disable=SC2046
      say "Other networks reach it on $(ports_words $(node_ports)): a firewall in front of this computer (your provider's) has to let them in, and a router at home forward them (--port-mapping asks it to)." ;;
  esac
  if [ "$role" = registry ]; then
    say ""
    if [ "$tls" = caddy ]; then
      say "The registry listens on $registry_listen and Caddy serves it at $registry_url (sudo journalctl -u caddy -f)."
    else
      say "The registry listens on $registry_listen. Browsers reach a registry only over https, so put a TLS proxy"
      say "in front of it at $registry_url, for example with Caddy (or install again with --tls caddy):"
      say "  caddy reverse-proxy --from ${registry_url#https://} --to $registry_listen"
    fi
    say "The published app uses no registry; only an app built for this one reaches it (registry/README.md)."
  fi
}

# ── Before installing: what this computer has, and what would stop the install ───────────────
#
# One line per check, the way kubeadm's preflight does it: ok; info; warn (it installs, but
# something won't work as well); BLOCK (installing would fail, or leave a node nobody can reach).
# A BLOCK stops the install before anything on this computer changes (--skip-checks goes on
# anyway); `install.sh check` runs only this. Nothing here changes the computer or asks for a
# password, and no outside service is asked about it: DNS and the release list only.

blocks=0
warns=0
row() {
  local level="$1" what="$2"
  shift 2
  case "$level" in
    warn) warns=$((warns + 1)) ;;
    block) blocks=$((blocks + 1)); level=BLOCK ;;
  esac
  printf '  %-5s  %-10s  %s\n' "$level" "$what" "$*"
}

# A value from /etc/os-release, read, never sourced.
os_field() {
  [ -f /etc/os-release ] || return 0
  sed -n "s/^$1=//p" /etc/os-release | head -n 1 | tr -d "\"'"
}
# Whether version $1 is at least $2 (dotted numbers).
ver_ge() {
  [ -n "$1" ] && [ "$(printf '%s\n%s\n' "$2" "$1" | sort -t. -k1,1n -k2,2n -k3,3n | head -n 1)" = "$2" ]
}
# Runs what only root may read, without asking for a password; fails when it can't.
root_read() {
  if [ "$(id -u)" = 0 ]; then
    "$@"
  elif command -v sudo >/dev/null && sudo -n true 2>/dev/null; then
    sudo -n "$@"
  else
    return 1
  fi
}

# Whether something listens on a port here: 0 taken, 1 free, 2 can't tell (no ss, netstat or lsof).
port_taken() {
  local proto="$1" port="$2" list
  if [ "$platform" = macos ] || { ! command -v ss >/dev/null && ! command -v netstat >/dev/null; }; then
    command -v lsof >/dev/null || return 2
    if [ "$proto" = tcp ]; then lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; else lsof -nP -iUDP:"$port" >/dev/null 2>&1; fi
    return
  fi
  if command -v ss >/dev/null; then
    list="$(ss -ln"${proto:0:1}" 2>/dev/null)" || return 2
  else
    list="$(netstat -ln"${proto:0:1}" 2>/dev/null)" || return 2
  fi
  printf '%s\n' "$list" | awk -v p=":$port" '{ for (i = 1; i <= NF; i++) if (substr($i, length($i) - length(p) + 1) == p) { f = 1; exit } } END { exit f ? 0 : 1 }'
}

# The ports other networks reach a node on: UDP 7843 (computers) and 7844 (browsers and phones),
# UDP 7846 and 7847 for the DHT among public nodes, and TCP 80 and 443 for Caddy's
# certificate and the registry's HTTPS.
node_ports() {
  echo 7843/udp 7844/udp 7846/udp 7847/udp
  [ "$tls" != caddy ] || echo 80/tcp 443/tcp
}
# "7843/udp 7844/udp 80/tcp" → "UDP 7843, 7844 and TCP 80"
ports_words() {
  local udp="" tcp="" p
  for p in "$@"; do
    case "$p" in */udp) udp="$udp, ${p%/*}" ;; */tcp) tcp="$tcp, ${p%/*}" ;; esac
  done
  udp="${udp#, }"
  tcp="${tcp#, }"
  printf '%s%s%s' "${udp:+UDP $udp}" "${udp:+${tcp:+ and }}" "${tcp:+TCP $tcp}"
}
# Megabytes in words: "512 MB", "15.7 GB".
size_words() {
  awk -v m="$1" 'BEGIN { if (m < 1024) printf "%d MB", m; else printf "%.1f GB", m / 1024 }'
}

# The host firewall, when one is on: fw is ufw or firewalld ("?" when it can't be read without a
# password), fw_missing the node's ports it doesn't let in yet.
fw=""
fw_missing=""
read_firewall() {
  local status="" p
  fw=""
  fw_missing=""
  [ "$platform" = linux ] || return 0
  if command -v ufw >/dev/null || command -v firewall-cmd >/dev/null; then
    root_read true || { fw="?"; return 0; }
  fi
  if command -v ufw >/dev/null; then
    status="$(root_read ufw status 2>/dev/null || true)"
    case "$status" in *"Status: active"*) fw=ufw ;; esac
  fi
  if [ -z "$fw" ] && command -v firewall-cmd >/dev/null && [ "$(root_read firewall-cmd --state 2>/dev/null || true)" = running ]; then
    fw=firewalld
  fi
  [ -n "$fw" ] || return 0
  for p in $(node_ports); do
    case "$fw" in
      ufw) printf '%s\n' "$status" | grep -qE "^(${p}|${p%/*})[[:space:]]+ALLOW" || fw_missing="$fw_missing $p" ;;
      firewalld) root_read firewall-cmd --query-port="$p" >/dev/null 2>&1 || fw_missing="$fw_missing $p" ;;
    esac
  done
  fw_missing="${fw_missing# }"
}

# This computer's addresses on its network interfaces, one per line.
local_addresses() {
  if command -v ip >/dev/null; then
    ip -o addr show scope global 2>/dev/null | awk '{ split($4, a, "/"); print a[1] }' || true
  elif [ "$platform" = linux ] && hostname -I >/dev/null 2>&1; then
    hostname -I | tr ' ' '\n' | grep . || true
  elif command -v ifconfig >/dev/null; then
    ifconfig 2>/dev/null | awk '$1 == "inet" || $1 == "inet6" { sub(/%.*/, "", $2); print $2 }' | grep -vE '^(127\.|::1$|fe80:)' || true
  fi
}
# Whether an address is reachable from the internet: not private, carrier-grade NAT, link-local or
# loopback (IPv6: only global unicast, 2000::/3).
is_public() {
  case "$1" in
    *:*) case "$1" in [23]*) return 0 ;; *) return 1 ;; esac ;;
    10.*|127.*|169.254.*|192.168.*|0.*) return 1 ;;
    172.1[6-9].*|172.2[0-9].*|172.3[01].*) return 1 ;;
    100.6[4-9].*|100.[7-9][0-9].*|100.1[01][0-9].*|100.12[0-7].*) return 1 ;;
    *) return 0 ;;
  esac
}
# What DNS names a host, one address per line.
resolve() {
  if command -v getent >/dev/null; then
    getent ahosts "$1" 2>/dev/null | awk '{ print $1 }' | sort -u || true
  elif command -v dscacheutil >/dev/null; then
    dscacheutil -q host -a name "$1" 2>/dev/null | awk '/_address:/ { print $2 }' | sort -u || true
  elif command -v host >/dev/null; then
    host "$1" 2>/dev/null | awk '/has (IPv6 )?address/ { print $NF }' | sort -u || true
  fi
}
# How Caddy would come onto this computer: apt or dnf; "apt?" where it may be missing from the
# packages; nothing where it can't. Debian has it from 12, Ubuntu from 23.04.
caddy_source() {
  if command -v apt-get >/dev/null; then
    case "$(os_field ID)" in
      ubuntu) ! ver_ge "$(os_field VERSION_ID)" 23.04 || echo apt ;;
      debian|raspbian) ! ver_ge "$(os_field VERSION_ID)" 12 || echo apt ;;
      *) echo "apt?" ;;
    esac
  elif command -v dnf >/dev/null; then
    echo dnf
  fi
}

check_system() {
  local about="$os $arch" distro libc=""
  if [ "$platform" = linux ]; then
    distro="$(os_field PRETTY_NAME)"
    [ -z "$distro" ] || about="$about ($distro)"
  else
    about="macOS $(sw_vers -productVersion 2>/dev/null || true) $arch"
  fi
  if [ -n "$unsupported" ]; then
    row block system "$about: $unsupported"
    return
  fi
  # Release builds of hoi-node are made on Ubuntu 22.04: they need its glibc (2.35) or newer.
  if [ "$platform" = linux ] && [ "$source" = 0 ]; then
    libc="$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{ print $2 }' || true)"
    if [ -z "$libc" ]; then
      case "$(ldd --version 2>&1 || true)" in
        *musl*) row block system "$about uses musl (Alpine?): the release's hoi-node needs glibc. Run it in Docker (deploy/server) or build it here with --source"; return ;;
      esac
    elif ! ver_ge "$libc" 2.35; then
      row block system "$about has glibc $libc: the release's hoi-node needs 2.35 or newer (Debian 12, Ubuntu 22.04, Fedora 36), or build it here with --source"
      return
    fi
  fi
  row ok system "$about${libc:+, glibc $libc}"
}

check_services() {
  if [ "$platform" = macos ]; then
    if [ "$(id -u)" = 0 ]; then
      row block services "run it as yourself, not with sudo: on a Mac the node runs in your account (launchd)"
    else
      row ok services "launchd, in your account"
    fi
    return
  fi
  if ! command -v systemctl >/dev/null || [ ! -d /run/systemd/system ]; then
    row block services "no systemd running here (a container?): run hoi-node in Docker instead (deploy/server)"
  else
    row ok services "$(systemctl --version 2>/dev/null | head -n 1 | awk '{ print $1, $2 }')"
  fi
  if [ "$(id -u)" = 0 ]; then
    row ok rights "root"
  elif ! command -v sudo >/dev/null; then
    row block rights "neither root nor sudo: run it as root, or install sudo"
  elif sudo -n true 2>/dev/null; then
    row ok rights "sudo"
  elif [ "$checking" = 1 ] || [ "$dry" = 1 ]; then
    row ok rights "sudo (the install asks for your password)"
  elif sudo -v; then
    row ok rights "sudo"
  else
    row block rights "sudo said no: run it as root, or as someone sudo lets in"
  fi
}

# Where the node comes from, and whether that works from here. A release list fetched now is kept
# for the install.
check_source() {
  local line name
  if [ "$source" = 1 ]; then
    if command -v cargo >/dev/null; then row ok source "built here from the checkout ($(cargo --version 2>/dev/null | awk '{ print $1, $2 }'))"; else row block source "--source builds hoi-node with Rust, which isn't here (https://rustup.rs)"; fi
    return
  fi
  if [ -n "$binary" ]; then
    if [ -f "$binary" ]; then row ok source "$binary"; else row block source "no such file: $binary"; fi
    return
  fi
  command -v tar >/dev/null || row block tools "no tar here to unpack hoi-node with"
  command -v sha256sum >/dev/null || command -v shasum >/dev/null || row block tools "no sha256sum or shasum here to check the download with"
  if [ -z "$from$release" ] && ! command -v curl >/dev/null && ! command -v wget >/dev/null; then
    row block source "neither curl nor wget is here to download hoi-node with: install one, or bring the release files (--from)"
    return
  fi
  if [ -n "$release" ] && ! gh auth status >/dev/null 2>&1; then
    row block source "the GitHub CLI isn't signed in, which --release needs: gh auth login"
    return
  fi
  if ! load_manifest; then
    row block source "$manifest_error"
    return
  fi
  line="$(node_entry)"
  if [ -z "$line" ]; then
    row block source "hoi $manifest_version has no hoi-node for $target: build it here with --source"
    return
  fi
  name="$(awk '{ print $4 }' <<< "$line")"
  if [ -n "$from" ] && [ ! -f "$from/$name" ]; then
    row block source "hoi $manifest_version: $name isn't in $from (it is on the release's GitHub page)"
    return
  fi
  row ok source "hoi $manifest_version, $name${from:+ from $from}${release:+ from the GitHub release $release}"
  case "$manifest_signature" in
    *good) row ok signature "$manifest_signature" ;;
    *) row info signature "$manifest_signature" ;;
  esac
}

check_resources() {
  local dir=/var/lib free mem
  [ "$platform" = linux ] || dir="$HOME"
  free="$(df -Pk "$dir" 2>/dev/null | awk 'NR == 2 { print int($4 / 1024) }' || true)"
  if [ -z "$free" ]; then
    row info disk "couldn't read the free space in $dir"
  elif [ "$free" -lt 512 ]; then
    row block disk "$free MB free in $dir: a node needs 512 MB at least (what it carries: 256 MB by default, and its database)"
  elif [ "$free" -lt 2048 ]; then
    row warn disk "$free MB free in $dir: enough to start, little room to carry communities (2 GB or more is comfortable)"
  else
    row ok disk "$(size_words "$free") free in $dir"
  fi
  if [ -n "$pieces" ] && [ -n "$free" ]; then
    local want
    want="$(awk -v n="$pieces" 'BEGIN { printf "%d", n * 1000 }')"
    if [ "$free" -lt $(( want + 512 )) ]; then
      row warn pieces "keeping up to $pieces GB of encrypted pieces and mailboxes needs that much free space beside the node's own: $(size_words "$free") free in $dir"
    else
      row ok pieces "keeps up to $pieces GB of encrypted pieces and private communities' mailboxes for others; it can read none of them, and its notice says so"
    fi
  fi
  if [ -r /proc/meminfo ]; then
    mem="$(awk '/^MemTotal:/ { print int($2 / 1024) }' /proc/meminfo)"
  else
    mem="$(( $(sysctl -n hw.memsize 2>/dev/null || echo 0) / 1048576 ))"
  fi
  if [ "${mem:-0}" -le 0 ]; then
    row info memory "couldn't read how much memory this computer has"
  elif [ "$mem" -lt 512 ]; then
    row warn memory "$mem MB: tight for a node that carries many communities (1 GB is comfortable)"
  else
    row ok memory "$(size_words "$mem")"
  fi
  # Signatures carry times, and HTTPS certificates need the right one.
  if [ "$platform" = linux ] && command -v timedatectl >/dev/null && [ -d /run/systemd/system ]; then
    case "$(timedatectl show -p NTPSynchronized --value 2>/dev/null || true)" in
      yes) row ok clock "synchronized" ;;
      no) row warn clock "not synchronized: records and certificates carry times. Turn it on: sudo timedatectl set-ntp true" ;;
    esac
  fi
}

check_ports() {
  local rc=0 ours="" taken="" wanted="UDP 7843 and 7844"
  [ "$role" != registry ] || wanted="UDP 7843 and 7844, TCP ${registry_listen##*:}"
  # A node this installer put here holds them itself; it is replaced.
  if [ "$platform" = linux ] && systemctl is-active --quiet hoi-node 2>/dev/null; then ours=1; fi
  if [ "$platform" = macos ] && launchctl print "gui/$(id -u)/$LABEL" >/dev/null 2>&1; then ours=1; fi
  port_taken udp 7843 || rc=$?
  if [ "$rc" = 2 ]; then
    row info ports "couldn't see which ports are in use (no ss, netstat or lsof)"
    return
  fi
  [ "$rc" != 0 ] || taken="$taken UDP 7843"
  rc=0; port_taken udp 7844 || rc=$?
  [ "$rc" != 0 ] || taken="$taken UDP 7844"
  if [ "$role" = registry ]; then
    rc=0; port_taken tcp "${registry_listen##*:}" || rc=$?
    [ "$rc" != 0 ] || taken="$taken TCP ${registry_listen##*:}"
  fi
  if [ -z "$taken" ]; then
    row ok ports "$wanted free"
  elif [ -n "$ours" ]; then
    row ok ports "${taken# } held by the hoi-node installed here, which this replaces"
  else
    row block ports "${taken# } taken by another program (see which: sudo ss -lnup; sudo ss -lntp): stop it, or move it"
  fi
}

check_firewall() {
  local need missing
  # shellcheck disable=SC2046 # one word per port
  need="$(ports_words $(node_ports))"
  read_firewall
  # shellcheck disable=SC2086
  missing="$(ports_words $fw_missing)"
  case "$fw" in
    "") [ "$platform" = macos ] || row ok firewall "no host firewall on (ufw, firewalld)" ;;
    "?") row info firewall "couldn't read the host firewall without a password; the install opens $need in it when it is on" ;;
    *)
      if [ -z "$fw_missing" ]; then
        row ok firewall "$fw lets $need in"
      elif [ "$firewall" = 1 ]; then
        row info firewall "$fw is on: the install opens $missing in it"
      else
        row warn firewall "$fw is on and doesn't let $missing in, and --no-firewall leaves it so: nobody outside reaches the node"
      fi ;;
  esac
  case "$role" in
    public|registry) row info firewall "a firewall in front of the server (your provider's, a security group) has to let $need in too" ;;
  esac
}

check_address() {
  local all public="" private="" a
  all="$(local_addresses)"
  for a in $all; do
    if is_public "$a"; then public="$public $a"; else private="$private $a"; fi
  done
  public="${public# }"
  private="${private# }"
  if [ -n "$advertise" ]; then
    row ok address "browsers and phones are told $advertise (--rtc-advertise)"
  elif [ -n "$public" ]; then
    row ok address "public: ${public// /, }"
  elif [ -z "$all" ]; then
    row info address "couldn't list this computer's addresses"
  elif [ "$role" = home ] || [ "$role" = private ]; then
    row info address "${private// /, } on your network: the node asks your router to forward its ports"
  elif [ "$mapping" = 1 ]; then
    row warn address "only ${private// /, }: behind a router the node asks it to forward its ports; on a cloud server whose public address is mapped to it (AWS, Google Cloud, Azure) give that address: --rtc-advertise PUBLIC-IP:7844"
  else
    row warn address "only ${private// /, } and --no-port-mapping: give the public address browsers reach: --rtc-advertise PUBLIC-IP:7844"
  fi
  # A registry's address has to lead here for browsers, and for Caddy's certificate.
  [ "$role" = registry ] || return 0
  local named here_too=""
  named="$(resolve "$registry_host" | tr '\n' ' ')"
  named="${named% }"
  for a in $named; do
    case " $all " in *" $a "*) here_too=1 ;; esac
  done
  if [ -z "$named" ]; then
    row warn dns "$registry_host has no DNS record yet: add an A record (and AAAA for IPv6) pointing at this server${tls:+; Caddy keeps asking for its certificate until it does}"
  elif [ -n "$here_too" ]; then
    row ok dns "$registry_host → ${named// /, } (this computer)"
  else
    row warn dns "$registry_host → ${named// /, }, which isn't one of this computer's addresses: right if your provider maps it here (AWS, Google Cloud, Azure), else point the record at this server"
  fi
}

check_https() {
  [ "$role" = registry ] || return 0
  if [ "$tls" != caddy ]; then
    row info https "the registry listens on $registry_listen: put your TLS proxy in front of it at $registry_url (or install with --tls caddy)"
    return
  fi
  if [ "$platform" != linux ]; then
    row block https "--tls caddy sets Caddy up on Linux; on a Mac, install it (brew install caddy) and leave out --tls"
    return
  fi
  local rc p taken=""
  for p in 80 443; do
    rc=0; port_taken tcp "$p" || rc=$?
    [ "$rc" != 0 ] || taken="$taken $p"
  done
  if command -v caddy >/dev/null; then
    row ok https "Caddy $(caddy version 2>/dev/null | awk '{ print $1 }') is here: it serves $registry_url with a certificate of its own"
  else
    case "$(caddy_source)" in
      apt|dnf) row ok https "Caddy comes from this system's packages and serves $registry_url with a certificate of its own" ;;
      "apt?") row warn https "Caddy may not be in this system's packages (Ubuntu 22.04 hasn't it): if the install can't find it, install it from https://caddyserver.com/docs/install and run this again" ;;
      *) row block https "Caddy isn't in $(os_field PRETTY_NAME)'s packages: install it from https://caddyserver.com/docs/install and run this again, or leave out --tls and bring your own TLS proxy" ;;
    esac
  fi
  if [ -n "$taken" ] && ! systemctl is-active --quiet caddy 2>/dev/null; then
    row block https "TCP${taken// /, } taken by another web server: leave out --tls caddy and proxy $registry_url to $registry_listen from it"
  fi
}

check_existing() {
  if [ -n "$host" ]; then
    local now
    now="$(hostname 2>/dev/null || true)"
    if [ "$platform" = macos ]; then
      row info host "a Mac keeps the name it has ($now); the node is called $name"
    elif [ "$now" = "$host" ]; then
      row ok host "this computer is called $host"
    elif command -v hostnamectl >/dev/null; then
      row info host "this computer becomes $host (now: $now)"
    else
      row warn host "no hostnamectl here, so this computer keeps the name $now"
    fi
  fi
  local unit=/etc/systemd/system/hoi-node.service was
  if [ "$platform" = linux ] && [ -f "$unit" ]; then
    was="$(sed -n 's/^# Written by deploy\/install\/install\.sh (\([a-z]*\)).*/\1/p' "$unit" | head -n 1)"
    row info existing "a hoi-node service is installed${was:+ ($was)}: it is replaced, its data stays"
  elif [ "$platform" = macos ] && [ -f "$HOME/Library/LaunchAgents/$LABEL.plist" ]; then
    row info existing "a hoi-node agent is installed: it is replaced, its data stays"
  fi
}

# The operator's notice: there, or missing where people would expect one.
check_notice() {
  if [ -n "$notice_file" ]; then
    local by
    by="$(sed -n 's/.*"operator"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$notice_file" | head -n 1)"
    if [ -n "$by" ]; then
      row ok notice "$notice_file, by $by (checked again with hoi-node before the install)"
    else
      row ok notice "$notice_file, which names no operator: people who connect read that you chose not to say who you are (optional, but you should; checked again with hoi-node before the install)"
    fi
  elif [[ "$role" =~ ^(public|registry)$ ]]; then
    row warn notice "no operator notice: people who connect are not told who runs this node, where, or how to reach you. Write one in the app (You → A node on a server) or the setup app and give it with --notice-file"
  fi
}

# The whole check; returns 1 when something blocks the install.
preflight() {
  say "Checking this computer for a $(describe_role) called $name:"
  check_system
  if [ -z "$unsupported" ]; then
    check_services
    check_source
    check_resources
    check_ports
    check_firewall
    check_address
    check_https
    check_existing
    check_notice
  fi
  if [ "$blocks" -gt 0 ]; then
    say "Not ready: $blocks $( [ "$blocks" = 1 ] && echo thing || echo things) above (BLOCK) would stop the install."
    return 1
  fi
  say "Ready to install$( [ "$warns" = 0 ] || echo ", with $warns $( [ "$warns" = 1 ] && echo warning || echo warnings) above")."
}

# ── A node for another computer ──────────────────────────────────────────────────────────────

# Writes what --config reads on the server. Nothing of this computer goes in it: not its name,
# its addresses or its node's link, so the server is not tied to it.
write_config() {
  [ -n "$conf_out" ] || conf_out="hoi-$host.conf"
  [ ! -e "$conf_out" ] && [ ! -L "$conf_out" ] || die "$conf_out is already there: not overwriting it (--out picks another file)"
  local file content
  file="$(basename "$conf_out")"
  content="$(
    echo "# A $(describe_role) for another computer, from deploy/install/install.sh config."
    echo "# Nothing here is secret: the node makes its own key where it runs. Copy this file to the"
    echo "# server and install with it there; flags given there win over these lines, and without"
    echo "# the hostname line the server keeps its name:"
    echo "#   curl -fsSL https://hoi.sh/install.sh | bash -s -- --config $file"
    echo "role=$role"
    echo "name=$name"
    echo "hostname=$host"
    [ -z "$notice_file" ] || echo "notice_file=$(basename "$notice_file")"
    [ -z "$registry_url" ] || echo "registry_url=$registry_url"
    for url in ${indexes[@]+"${indexes[@]}"}; do echo "index=$url"; done
    [ -z "$cap" ] || echo "daily_cap_gb=$cap"
    [ -z "$pieces" ] || echo "hold_pieces_gb=$pieces"
    [ -z "$tls" ] || echo "tls=$tls"
  )"
  if [ "$dry" = 1 ]; then
    printf '+ write %s:\n%s\n' "$conf_out" "$content"
  else
    printf '%s\n' "$content" > "$conf_out"
  fi
  say "wrote $conf_out: a $(describe_role) called $name"
  [ -z "$notice_file" ] || say "  notice:     $(basename "$notice_file")   (copy it next to the configuration on the server)"
  say "  host name:  $host   (name the server this when you order it, or the installer does)"
  [ "$role" != registry ] || say "  registry:   $registry_url   (point DNS for ${registry_url#https://} at the server first)"
  [ "$tls" != caddy ] || say "  https:      Caddy, installed on the server with its certificate (ports 80 and 443 open there)"
  say "Nothing on this computer changed. On to the server:"
  say "  scp $conf_out root@SERVER:"
  say "  curl -fsSL https://hoi.sh/install.sh | bash -s -- check --config $file   (there: is it installable?)"
  say "  curl -fsSL https://hoi.sh/install.sh | bash -s -- --config $file   (there)"
  say "  deploy/install/install.sh --config ~/$file --source           (there, from a copy of the source)"
  say "Not feeling the name? Run it again for another."
}

case "$kind" in
  config) write_config ;;
  app)
    [ "$(id -u)" != 0 ] || die "install the app as yourself, not with sudo: it starts in your account (the installer asks for sudo when a package needs it)"
    get_manifest
    if [ "$platform" = macos ]; then install_app_macos; else install_app_linux; fi ;;
  node|uninstall)
    if [ "$kind" = node ]; then
      if ! preflight; then
        [ "$checking" = 0 ] || exit 1
        if [ "$skip_checks" = 1 ]; then
          note "installing anyway (--skip-checks)"
        elif [ "$dry" = 1 ]; then
          note "a dry run goes on to show what the install would do"
        else
          die "nothing was changed: fix what says BLOCK above and run this again (install.sh check shows it without installing), or add --skip-checks to install anyway"
        fi
      fi
      if [ "$checking" = 1 ]; then
        say "Install it: the same command without check."
        exit 0
      fi
      say ""
    fi
    if [ "$platform" = macos ]; then node_macos; else node_linux; fi
    [ "$kind" = uninstall ] || node_notes ;;
esac
if [ "$checking" = 0 ] && { [ "$kind" = app ] || [ "$kind" = node ]; }; then record_release_version; fi
