#!/usr/bin/env bash
# Generate a local self-signed cert for the gateway's WSS listener.
# Usage: ./gen-local-cert.sh [outdir]
# Output: <outdir>/gw-cert.pem + <outdir>/gw-key.pem (SAN: 127.0.0.1, ::1, localhost)
# Browsers will warn on self-signed certs: open https://127.0.0.1:5071/ once
# and accept the exception, then wss:// connections from the page work.
set -u
OUTDIR="${1:-$HOME/.openhw-gw}"
mkdir -p "$OUTDIR"
CERT="$OUTDIR/gw-cert.pem"
KEY="$OUTDIR/gw-key.pem"
if [ -f "$CERT" ] && [ -f "$KEY" ]; then
  echo "[INFO] cert already exists: $CERT"
  exit 0
fi
command -v openssl >/dev/null || { echo "[ERROR] openssl not found"; exit 2; }
openssl req -x509 -newkey rsa:2048 -sha256 -days 825 -nodes \
  -keyout "$KEY" -out "$CERT" \
  -subj "/CN=127.0.0.1" \
  -addext "subjectAltName=IP:127.0.0.1,IP:::1,DNS:localhost" 2>/dev/null
chmod 600 "$KEY"
echo "[SUCCESS] wrote $CERT and $KEY"
echo "Start the gateway with TLS like:"
echo "  ./openhw-gw --tls-port 5071 --tls-cert $CERT --tls-key $KEY"
echo "or:"
echo "  TLS_PORT=5071 TLS_CERT=$CERT TLS_KEY=$KEY ./openhw-gw"
