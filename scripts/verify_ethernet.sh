#!/usr/bin/env bash
# Regression check for all three ethernet firmwares.
# Usage: ./verify_ethernet.sh [max_instructions_per_firmware]
# Exit 0 if all pass; prints a summary.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MAX_INST="${1:-5000000}"
PKG="$REPO_ROOT/stm32-periph-wasm/pkg"
LOGDIR="$REPO_ROOT/.pw-scratch"
mkdir -p "$LOGDIR"
FAIL=0

command -v node >/dev/null || { echo "node not found"; exit 2; }

# Ensure the NAT target HTTP server is up (needed by eth_http; harmless otherwise)
if ! (exec 3<>/dev/tcp/127.0.0.1/8092) 2>/dev/null; then
  node -e '
    const http = require("http");
    const srv = http.createServer((req, res) => { res.writeHead(200, {"Content-Type":"text/plain"}); res.end("Hello from openhw HTTP server"); });
    srv.listen(8092, "127.0.0.1");
  ' >/dev/null 2>&1 &
  for i in $(seq 1 20); do
    (exec 3<>/dev/tcp/127.0.0.1/8092) 2>/dev/null && break
    sleep 0.25
  done
fi

run_check() {
  local name="$1" cfg="$2" log="$3" marker="$4" dir="${5:-$1}" bin="${6:-$1.bin}"
  # NOTE: firmware/config paths are repo-root relative (../../ from $PKG);
  # the old "../$name" form resolved inside stm32-periph-wasm/ and failed.
  (cd "$PKG" && node cli.mjs "../../$dir/$bin" "$MAX_INST" --gateway --config="../../$dir/$cfg") >"$log" 2>&1
  if grep -a -q "$marker" "$log"; then
    echo "PASS: $name (marker '$marker')"
  else
    echo "FAIL: $name (missing marker '$marker')"
    tail -25 "$log"
    FAIL=1
  fi
}

run_check eth_http  config.yaml "$LOGDIR/verify_eth_http.log"  "TCP connected"
run_check eth_dhcp  config.yaml "$LOGDIR/verify_eth_dhcp.log"  "=== DHCP SUCCESS ==="
run_check eth_test  config.yaml "$LOGDIR/verify_eth_test.log"  "ETH Test: done"
run_check eth_http_f429 config_f429.yaml "$LOGDIR/verify_eth_http_f429.log" "TCP connected" eth_http eth_http_f429.bin
run_check eth_dhcp_f429 config_f429.yaml "$LOGDIR/verify_eth_dhcp_f429.log" "=== DHCP SUCCESS ===" eth_dhcp eth_dhcp_f429.bin
run_check eth_test_f429 config_f429.yaml "$LOGDIR/verify_eth_test_f429.log" "ETH Test: done" eth_test eth_test_f429.bin

# eth_http must also show zero TCP failures (both maps)
if grep -a -q "TCP fail" "$LOGDIR/verify_eth_http.log"; then
  echo "FAIL: eth_http reported TCP fail"
  FAIL=1
fi
if grep -a -q "TCP fail" "$LOGDIR/verify_eth_http_f429.log"; then
  echo "FAIL: eth_http_f429 reported TCP fail"
  FAIL=1
fi

exit $FAIL
