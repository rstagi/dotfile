#!/usr/bin/env bash
# Install or update the ws hub on an Ubuntu VPS. Run as root.
# Usage: install-hub.sh --domain <name> --email <address> --ref <git ref> --ca-cert <file>
#        install-hub.sh --print-unit --domain <name>   (print the systemd unit, change nothing)
set -euo pipefail

REPO_URL="https://github.com/rstagi/dotfile.git"
SRC_DIR="/opt/ws-hub/src"
CA_PATH="/etc/ws-hub/root-ca.pem"
STATE_DIR="/var/lib/ws-hub"
BIN_PATH="/usr/local/bin/ws"
UNIT_PATH="/etc/systemd/system/ws-hub.service"
RENEW_HOOK="/etc/letsencrypt/renewal-hooks/deploy/ws-hub-restart.sh"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

main() {
  local domain="" email="" ref="" ca_cert="" print_unit=0
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --domain) domain="${2:?--domain needs a value}"; shift 2 ;;
      --email) email="${2:?--email needs a value}"; shift 2 ;;
      --ref) ref="${2:?--ref needs a value}"; shift 2 ;;
      --ca-cert) ca_cert="${2:?--ca-cert needs a value}"; shift 2 ;;
      --print-unit) print_unit=1; shift ;;
      *) die "Unknown argument: $1" ;;
    esac
  done
  [[ -n "$domain" ]] || die "--domain is required"
  if [[ $print_unit -eq 1 ]]; then
    render_unit "$domain"
    return
  fi
  [[ -n "$email" && -n "$ref" && -n "$ca_cert" ]] || die "--email, --ref and --ca-cert are required"
  [[ $EUID -eq 0 ]] || die "Run as root"
  [[ -f "$ca_cert" ]] || die "CA certificate not found: $ca_cert"
  openssl x509 -in "$ca_cert" -noout || die "Not a PEM certificate: $ca_cert"

  install_packages
  create_user
  install_ca "$ca_cert"
  get_certificate "$domain" "$email"
  install_rust
  build_ws "$ref"
  install_unit "$domain"
  # The CA and the TLS files exist at this point.
  systemctl enable ws-hub.service
  systemctl restart ws-hub.service
  echo "ws hub is running on https://$domain"
}

die() {
  echo "install-hub: $*" >&2
  exit 1
}

render_unit() {
  sed "s|@DOMAIN@|$1|g" "$HERE/ws-hub.service.in"
}

install_packages() {
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -y
  apt-get install -y build-essential pkg-config git curl ca-certificates openssl certbot
}

create_user() {
  id ws-hub >/dev/null 2>&1 || useradd --system --home-dir "$STATE_DIR" --shell /usr/sbin/nologin ws-hub
  install -d -o ws-hub -g ws-hub -m 0700 "$STATE_DIR"
}

install_ca() {
  install -d -m 0755 /etc/ws-hub
  install -m 0644 "$1" "$CA_PATH"
}

get_certificate() {
  local domain="$1" email="$2"
  install -d -m 0755 "$(dirname "$RENEW_HOOK")"
  cat >"$RENEW_HOOK" <<'HOOK'
#!/bin/sh
systemctl restart ws-hub.service
HOOK
  chmod 0755 "$RENEW_HOOK"
  # Port 80 must be free and reachable. The hub is not running yet on a first install.
  certbot certonly --standalone --non-interactive --agree-tos \
    --email "$email" --domain "$domain" --keep-until-expiring
}

install_rust() {
  if [[ ! -x /root/.cargo/bin/rustup ]]; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  fi
}

build_ws() {
  local ref="$1"
  if [[ ! -d "$SRC_DIR/.git" ]]; then
    mkdir -p "$(dirname "$SRC_DIR")"
    git clone "$REPO_URL" "$SRC_DIR"
  fi
  git -C "$SRC_DIR" fetch --tags --force origin
  git -C "$SRC_DIR" checkout --force --detach "$(git -C "$SRC_DIR" rev-parse --verify "origin/$ref" 2>/dev/null || git -C "$SRC_DIR" rev-parse --verify "$ref^{commit}")"
  # rust-toolchain.toml in wsd/ selects the pinned toolchain.
  (cd "$SRC_DIR/wsd" && PATH="/root/.cargo/bin:$PATH" cargo build --release -p ws)
  install -m 0755 "$SRC_DIR/wsd/target/release/ws" "$BIN_PATH"
}

install_unit() {
  render_unit "$1" >"$UNIT_PATH"
  systemctl daemon-reload
}

main "$@"
