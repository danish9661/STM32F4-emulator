#!/usr/bin/env bash
# Regression check for all three ethernet firmwares.
# Usage: ./verify_ethernet.sh [max_instructions_per_firmware]
# Exit 0 if all pass; prints a summary.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MAX_INST="${1:-5000000}"
PKG="$REPO_ROOT/stm32-periph-wasm/pkg"
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
  local name="$1" cfg="$2" log="$3" marker="$4"
  (cd "$PKG" && node cli.mjs "../$name/$name.bin" "$MAX_INST" --gateway --config="../../$name/$cfg") >"$log" 2>&1
  if grep -a -q "$marker" "$log"; then
    echo "PASS: $name (marker '$marker')"
  else
    echo "FAIL: $name (missing marker '$marker')"
    tail -25 "$log"
    FAIL=1
  fi
}

run_check eth_http  config.yaml /tmp/opencode/verify_eth_http.log  "TCP connected"
run_check eth_dhcp  config.yaml /tmp/opencode/verify_eth_dhcp.log  "=== DHCP SUCCESS ==="
run_check eth_test  config.yaml /tmp/opencode/verify_eth_test.log  "ETH Test: done"

# eth_http must also show zero TCP failures
if grep -a -q "TCP fail" /tmp/opencode/verify_eth_http.log; then
  echo "FAIL: eth_http reported TCP fail"
  FAIL=1
fi

exit $FAIL
