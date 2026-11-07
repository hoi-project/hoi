#!/usr/bin/env bash
# Paths are relative to this file, so the bundle works from any working directory.
set -euo pipefail
here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
role=""
args=()
case "${1:-}" in public|registry|home|private) role="$1"; shift ;; esac
while [ $# -gt 0 ]; do
  case "$1" in
    --role)
      [ $# -ge 2 ] || { echo '--role needs public, registry, home, private or config' >&2; exit 2; }
      role="$2"; shift 2 ;;
    --help|-h)
      cat <<'HELP'
Usage: bash install.sh [--role public|registry|home|private|config] [INSTALL_OPTIONS]
Without --role, asks what to install. An explicit role ignores server.conf;
--role config uses its saved settings. Registry includes the public node.

  bash install.sh --role registry --registry-url https://names.example.org --tls caddy
  bash install.sh --role public
  bash install.sh --role public check
  bash install.sh --role public --reset-data --confirm-reset hoi-public

Data stays by default. --reset-data permanently erases keys, records, media and any
registry database for the selected role. It requires confirmation, even with --yes.
Use hoi-public for public/registry, or hoi-node for home/private.

Other flags: --name NAME, --rtc-advertise IP:7844, --index URL, --node LINK,
--registry-origin ORIGIN, --no-firewall, --dry-run. See node-installer.sh --help for all.
Remote service controls are in deploy.mjs (run on your own computer).
HELP
      exit 0 ;;
    *) args+=("$1"); shift ;;
  esac
done
if [ -z "$role" ]; then
  # A legacy check or explicit configuration is already a deliberate choice.
  for arg in "${args[@]}"; do
    case "$arg" in check|--config) role=config ;; esac
  done
fi
if [ -z "$role" ]; then
  if ! { : < /dev/tty; } 2>/dev/null; then
    echo 'Choose --role public|registry|home|private|config (a terminal opens the menu).' >&2
    exit 2
  fi
  printf '%s\n' 'What should be installed?' '  1) Public node' '  2) Public node and registry' '  3) Home node and public relay' '  4) Private home node' '  5) Saved server.conf settings' > /dev/tty
  read -r -p 'Choice: ' choice < /dev/tty
  case "$choice" in
    1|public) role=public ;;
    2|registry) role=registry ;;
    3|home) role=home ;;
    4|private) role=private ;;
    5|config) role=config ;;
    *) echo 'No valid role selected; nothing was changed.' >&2; exit 2 ;;
  esac
  if [ "$role" = registry ]; then
    read -r -p 'Registry HTTPS URL: ' url < /dev/tty
    [ -n "$url" ] || { echo 'A registry URL is required.' >&2; exit 2; }
    args+=(--registry-url "$url")
    read -r -p 'Configure HTTPS with Caddy? [Y/n]: ' https < /dev/tty
    case "$https" in ''|y|Y|yes) args+=(--tls caddy) ;; n|N|no) ;; *) echo 'Choose y or n.' >&2; exit 2 ;; esac
  fi
  read -r -p 'Existing data: keep or reset? [keep]: ' data < /dev/tty
  case "$data" in ''|keep) ;; reset) args+=(--reset-data) ;; *) echo 'Choose keep or reset.' >&2; exit 2 ;; esac
fi
case "$role" in
  public|registry|home|private) selection=("$role") ;;
  config) selection=(--config "$here/server.conf") ;;
  *) echo '--role needs public, registry, home, private or config' >&2; exit 2 ;;
esac
if [ "$(uname -s)" != Linux ] || [ "$(uname -m)" != '@ARCH@' ]; then
  echo 'This bundle needs Linux @ARCH@. Rebuild with the matching --arch option.' >&2
  exit 2
fi
(cd "$here" && sha256sum --check SHA256SUMS)
# Archives packed on Windows do not always retain Unix executable permissions.
chmod +x "$here/hoi-node"
if ! "$here/hoi-node" --version; then
  echo 'The bundled binary cannot run here. Docker builds need glibc 2.36+ (Debian 12 / Ubuntu 24.04 or newer).' >&2
  exit 2
fi
exec bash "$here/node-installer.sh" "${selection[@]}" --binary "$here/hoi-node" --yes "${args[@]}"
