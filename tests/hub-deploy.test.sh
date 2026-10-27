#!/bin/zsh
# Check the hub VPS install script and unit. No server is contacted and nothing is installed.
set -u
ROOT="${0:A:h:h}"
source "${0:A:h}/lib.sh"
SCRIPT="$ROOT/deploy/hub/install-hub.sh"

echo "-- shellcheck"
if command -v shellcheck >/dev/null 2>&1; then
  shellcheck "$SCRIPT"; assert_exit $? 0 "install-hub.sh passes shellcheck"
else
  echo "  skip: shellcheck not installed"
fi

echo "-- unit"
unit="$("$SCRIPT" --print-unit --domain hub.example.test)"
assert_exit $? 0 "--print-unit renders without root"
assert_contains "$unit" "User=ws-hub" "unit runs as ws-hub"
assert_contains "$unit" "LoadCredential=tls-cert:/etc/letsencrypt/live/hub.example.test/fullchain.pem" "unit reads the certbot chain"
assert_contains "$unit" "LoadCredential=tls-key:/etc/letsencrypt/live/hub.example.test/privkey.pem" "unit reads the certbot key"
assert_contains "$unit" "ExecStart=/usr/local/bin/ws hub serve" "unit runs ws hub serve"
assert_contains "$unit" "--tls-cert %d/tls-cert --tls-key %d/tls-key" "unit passes the TLS files"
assert_contains "$unit" "--ca-cert /etc/ws-hub/root-ca.pem" "unit passes the root CA"
assert_contains "$unit" "--state /var/lib/ws-hub" "unit passes the state directory"
assert_contains "$unit" "ReadWritePaths=/var/lib/ws-hub" "unit allows writes to the state directory only"
[[ "$unit" != *@DOMAIN@* ]]; assert_exit $? 0 "no placeholder is left in the unit"

echo "-- order"
main_body="$(awk '/^main\(\) \{/,/^}/' "$SCRIPT")"
line_of() { print -r -- "$main_body" | grep -n -m1 -F -- "$1" | cut -d: -f1; }
ca_line="$(line_of 'install_ca ')"
tls_line="$(line_of 'get_certificate ')"
start_line="$(line_of 'systemctl restart ws-hub.service')"
[[ -n "$ca_line" && -n "$tls_line" && -n "$start_line" ]]; assert_exit $? 0 "main installs the CA, gets the certificate and starts the service"
(( ca_line < start_line )); assert_exit $? 0 "CA installs before the service starts"
(( tls_line < start_line )); assert_exit $? 0 "TLS certificate is obtained before the service starts"
first_start="$(print -r -- "$main_body" | grep -n -m1 -E 'systemctl (start|restart|reload-or-restart)' | cut -d: -f1)"
assert_eq "$first_start" "$start_line" "no earlier systemctl start in main"
body="$(<"$SCRIPT")"
assert_contains "$body" "certbot certonly --standalone" "certbot runs in standalone mode"
assert_contains "$body" "systemctl restart ws-hub.service" "renew hook restarts the service"
assert_contains "$body" 'CA_PATH="/etc/ws-hub/root-ca.pem"' "CA path"
assert_contains "$body" 'BIN_PATH="/usr/local/bin/ws"' "binary path"
assert_contains "$body" 'STATE_DIR="/var/lib/ws-hub"' "state path"
assert_contains "$body" "cargo build --release" "release build"

echo "-- arguments"
"$SCRIPT" --domain x >/dev/null 2>&1; assert_exit $? 1 "missing arguments fail"
"$SCRIPT" --bogus >/dev/null 2>&1; assert_exit $? 1 "unknown argument fails"

test_summary
