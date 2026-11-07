#!/usr/bin/env bash
# Install hoi-node as an always-on home node, which is a public relay too: a Raspberry Pi, a Mac
# mini, an old laptop, any computer that stays on. This is deploy/install/install.sh with the
# role "home" and, unless you name a release, hoi-node built from this checkout. Run it again to
# update. No registry is needed; --index URL still announces it to one.
#
#   deploy/home/install.sh --name "lena's pi"                        build from this checkout
#   deploy/home/install.sh --binary ~/hoi-node_0.2.0_aarch64-unknown-linux-gnu.tar.gz
#   deploy/home/install.sh --release v0.2.0                          that GitHub release, with gh
#   deploy/home/install.sh --no-public-relay                         a home node for your devices only
#   deploy/home/install.sh --uninstall                               stop and remove the service, keep the data
#
# Every other option is the general installer's (deploy/install/install.sh --help): --name,
# --daily-cap-gb, --no-port-mapping, --rtc-advertise, --extra, --from, --dry-run, …
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
role=home
source=--source
args=()
while [ $# -gt 0 ]; do
  case "$1" in
    --no-public-relay) role=private ;;
    --uninstall) role=uninstall; source="" ;;
    --binary|--release|--from|--manifest)
      [ $# -ge 2 ] || { echo "install.sh: $1 needs a value" >&2; exit 2; }
      source=""
      args+=("$1" "$2")
      shift ;;
    -h|--help) sed -n '2,/^set -euo/p' "$0" | sed -e '$d' -e 's/^# \{0,1\}//'; exit 0 ;;
    *) args+=("$1") ;;
  esac
  shift
done
exec "$here/../install/install.sh" "$role" ${source:+"$source"} ${args[@]+"${args[@]}"}
