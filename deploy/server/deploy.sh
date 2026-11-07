#!/usr/bin/env bash
# Start or update the public carrier on this host. Safe to run again at any time: the carrier's
# database, keys and carried communities live in the named volume and survive updates.
#
#   deploy/server/deploy.sh            # pull HOI_CARRIER_IMAGE (or build locally) and (re)start
#   deploy/server/deploy.sh status     # show the container and the node's own status lines
#   deploy/server/deploy.sh logs       # follow the node's status output (no peer addresses in it)
#
# Configure deploy/server/.env on the host first.
set -euo pipefail

cd "$(dirname "$0")"
if [ ! -f .env ]; then
  echo "deploy/server/.env is missing. Copy .env.example and set HOI_RTC_ADVERTISE." >&2
  exit 2
fi
compose=(docker compose --env-file .env -f compose.yml)
if [ -f compose.override.yml ]; then
  compose+=(-f compose.override.yml)
fi

case "${1:-up}" in
  up)
    "${compose[@]}" config --quiet
    # Read the one value this script needs without executing .env as shell code.
    image="$(sed -n 's/^HOI_CARRIER_IMAGE=//p' .env | tail -n 1 | sed -e "s/^['\"]//" -e "s/['\"]$//")"
    if [ -n "$image" ]; then
      "${compose[@]}" pull public-carrier
      "${compose[@]}" up -d --no-build public-carrier
    else
      "${compose[@]}" up -d --build public-carrier
    fi
    echo "Carrier started. Its node link (to share) is in the log, and in /data/node-link."
    "${compose[@]}" logs --tail 20 public-carrier
    ;;
  status)
    "${compose[@]}" ps public-carrier
    "${compose[@]}" logs --tail 30 public-carrier
    ;;
  logs)
    "${compose[@]}" logs --follow --tail 100 public-carrier
    ;;
  *)
    echo "Usage: $0 [up|status|logs]" >&2
    exit 2
    ;;
esac
